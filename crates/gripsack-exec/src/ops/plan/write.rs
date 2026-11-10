//! Whole-file permissions, copy/template authority through the kernel's
//! per-unit decision (plan/0056 K1), and explicit migration writes.
use super::*;

/// Ordinary deployment follows source executability without widening
/// acquired read/write permissions. Rollback restores an exact recorded mode.
#[derive(Clone, Copy)]
pub enum WritePermissions {
    Source { executable: bool },
    Preserve,
    Exact(u32),
}

impl WritePermissions {
    pub(super) fn resolve(self, view: &DestView) -> u32 {
        let executable = match self {
            Self::Source { executable } => executable,
            Self::Exact(mode) => return mode,
            Self::Preserve => {
                return match &view.observed {
                    Some(crate::deploy::Observation::File { mode, .. }) => *mode,
                    _ => 0o644,
                };
            }
        };
        let Some(prev) = view.prev_entry().filter(|entry| {
            matches!(
                entry.ownership.policy(),
                Ownership::TrackedCopy | Ownership::Template
            ) && !entry.preserved_drift
        }) else {
            return if executable { 0o755 } else { 0o644 };
        };
        let Some(mode) = prev.file_mode else {
            return if executable { 0o755 } else { 0o644 };
        };
        if prev.source_executable.unwrap_or(mode & 0o111 != 0) == executable {
            mode
        } else if executable {
            // Only classes already allowed to read gain execute access.
            mode | ((mode & 0o444) >> 2)
        } else {
            mode & !0o111
        }
    }
}

pub(super) fn plan_write(
    view: &DestView,
    content: &[u8],
    permissions: WritePermissions,
) -> Result<Op, ExecError> {
    let observed_mode = match &view.observed {
        Some(crate::deploy::Observation::File { mode, .. }) => Some(*mode),
        _ => None,
    };
    let intent_mode = permissions.resolve(view);
    let desired = store::canonical_bytes_identity(content, intent_mode);
    let desired_hash: store::hash::ManifestHash = desired.clone().into();
    let decision = view.decide(store::hash::kernel::file(&desired));
    let mut op = match decision {
        Decision::Satisfied => {
            // satisfied means managed (0029 §2): the record clears any
            // prior preserve mark and holds the desired identity — an
            // observation recorded as drift must not stick forever
            let landed_mode = match &view.observed {
                Some(crate::deploy::Observation::File { mode, .. }) => Some(*mode),
                _ => None,
            };
            view.base(
                OpKind::Satisfied,
                None,
                Intended::Object(view.observed_identity().expect("satisfied implies live")),
            )
            .with_produces(view.produces(desired_hash, landed_mode, false))
        }
        Decision::Create | Decision::ReplaceOwn => {
            let update = matches!(decision, Decision::ReplaceOwn);
            let mode = intent_mode;
            view.base(
                OpKind::Write {
                    content: ContentSource::Bytes(content.to_vec()),
                    mode,
                },
                Some(if update {
                    Authority::Update
                } else {
                    Authority::Fresh
                }),
                Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                    content, mode,
                ))),
            )
            .with_produces(view.produces(desired_hash, Some(mode), false))
        }
        Decision::TakeOver => {
            // adoption keeps the live mode (0033 R1)
            let mode = match &view.observed {
                Some(crate::deploy::Observation::File { mode, .. }) => *mode,
                _ => intent_mode,
            };
            // the prior is captured at execution (0015 §4)
            let written_hash = store::canonical_bytes_identity(content, mode).into();
            view.base(
                OpKind::Write {
                    content: ContentSource::Bytes(content.to_vec()),
                    mode,
                },
                Some(Authority::TakeOver),
                Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                    content, mode,
                ))),
            )
            .with_produces(view.produces(written_hash, Some(mode), false))
        }
        Decision::Refuse(Refusal::Unsupported) => return Err(super::link::unsupported(view)),
        Decision::Refuse(Refusal::Drifted | Refusal::Unowned) => {
            // the record holds the OBSERVED identity, marked preserved
            // (0029 §2) — it authorizes nothing. A foreign link hashes its
            // target in the bytes domain.
            let observed_hash: store::hash::ManifestHash = match &view.observed {
                Some(crate::deploy::Observation::Symlink { target }) => {
                    store::canonical_bytes_hash(target.as_encoded_bytes()).into()
                }
                Some(crate::deploy::Observation::File { bytes, mode }) => {
                    store::canonical_bytes_identity(bytes, *mode).into()
                }
                None => unreachable!("the kernel refuses only a live object"),
            };
            view.base(
                OpKind::Preserved,
                None,
                Intended::Object(view.observed_identity().expect("preserve implies live")),
            )
            .with_produces(view.produces(observed_hash, observed_mode, true))
        }
    };
    if let Some(produced) = op.produces_mut()
        && !produced.preserved_drift
        && let WritePermissions::Source { executable } = permissions
    {
        produced.source_executable = Some(executable);
    }
    Ok(op)
}

pub(super) fn plan_migration_write(
    view: &DestView,
    authorization: &crate::deploy::MigrationEntry,
    content: &[u8],
    permissions: WritePermissions,
) -> Result<Op, ExecError> {
    authorization.reobserve(view)?;
    let mode = permissions.resolve(view);
    let identity = store::canonical_bytes_identity(content, mode);
    // The admitted predecessor link is the recorded identity: the kernel
    // authorizes replacing exactly that link, nothing else.
    let decision = decide_unit(
        &authorization.recorded(),
        &kernel_live(view.observed.as_ref()),
        &KernelIntended::Obj(store::hash::kernel::file(&identity)),
        &take_over_consent(false),
    );
    if decision != Decision::ReplaceOwn {
        return Err(ExecError::Step {
            output: view.output.into(),
            step: "migrate".into(),
            detail: format!(
                "{}: predecessor changed after migration admission",
                view.entry.to
            ),
        });
    }
    if !matches!(
        view.entry.mode,
        Ownership::TrackedCopy | Ownership::Template
    ) {
        return Err(ExecError::Step {
            output: view.output.into(),
            step: "migrate".into(),
            detail: "migration write requires a whole-file destination".into(),
        });
    }
    let mut produced = view.produces(identity.clone().into(), Some(mode), false);
    if let Some(produced) = &mut produced
        && let WritePermissions::Source { executable } = permissions
    {
        produced.source_executable = Some(executable);
    }
    Ok(view
        .base(
            OpKind::Write {
                content: ContentSource::Bytes(content.to_vec()),
                mode,
            },
            Some(Authority::Update),
            Intended::Object(ObjectIdentity::File(identity)),
        )
        .with_produces(produced))
}

#[cfg(test)]
mod tests {
    //! The permission policy (0043; the independent model is FileMode.tla):
    //! a fresh file follows the source's executability, an update keeps the
    //! recorded mode and moves only the execute bits a source change asks
    //! for (never widening read access), preserved drift confers no mode, and
    //! a merge host keeps its own. Whether the chosen mode LANDED is the leaf
    //! protocol's and the apply machine's identity check, not this table's.
    use super::*;

    fn mode(
        prev: Option<(u32, bool, bool)>,
        observed: Option<u32>,
        permissions: WritePermissions,
    ) -> u32 {
        let dir = std::path::Path::new("/nonexistent");
        let entry = DeploymentEntry {
            from: "payload".into(),
            to: "/nonexistent/dest".into(),
            mode: Ownership::TrackedCopy,
            vars: Default::default(),
            marker: None,
            span: None,
        };
        let record = prev.map(|(file_mode, executable, preserved)| store::DeployedEntry {
            from: "payload".into(),
            to: entry.to.clone(),
            key: None,
            ownership: store::StoredOwnership::Legacy(Ownership::TrackedCopy),
            vars: Default::default(),
            hash: store::canonical_bytes_identity(b"x", file_mode).into(),
            file_mode: Some(file_mode),
            source_executable: Some(executable),
            prior: None,
            preserved_drift: preserved,
        });
        let view = DestView {
            output: "m",
            entry: &entry,
            dest: dir.join("dest"),
            home: dir,
            observed: observed.map(|mode| crate::deploy::Observation::File {
                bytes: b"x".to_vec(),
                mode,
            }),
            prev: record.as_ref().map(|entry| PreviousRecord {
                entry,
                store_path: dir,
            }),
            consent: take_over_consent(false),
        };
        permissions.resolve(&view)
    }

    #[test]
    fn the_permission_policy() {
        let source = |executable| WritePermissions::Source { executable };
        // fresh: the source's executability
        assert_eq!(mode(None, None, source(false)), 0o644);
        assert_eq!(mode(None, None, source(true)), 0o755);
        // update: the recorded mode, private data stays private
        assert_eq!(
            mode(Some((0o600, false, false)), None, source(false)),
            0o600
        );
        // the source became executable: execute only where read was granted
        assert_eq!(mode(Some((0o600, false, false)), None, source(true)), 0o700);
        assert_eq!(mode(Some((0o644, false, false)), None, source(true)), 0o755);
        // the source lost its execute bits
        assert_eq!(mode(Some((0o755, true, false)), None, source(false)), 0o644);
        // preserved drift confers no mode
        assert_eq!(mode(Some((0o600, false, true)), None, source(false)), 0o644);
        // a merge host keeps its own mode; rollback restores the record exactly
        assert_eq!(mode(None, Some(0o640), WritePermissions::Preserve), 0o640);
        assert_eq!(mode(None, None, WritePermissions::Preserve), 0o644);
        assert_eq!(
            mode(
                Some((0o644, false, false)),
                None,
                WritePermissions::Exact(0o600)
            ),
            0o600
        );
    }
}
