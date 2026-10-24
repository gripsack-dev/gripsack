//! Whole-file permissions, copy/template authority and explicit migration writes.
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
        let Some(prev) = view.prev.filter(|entry| {
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

pub(super) fn plan_write(view: &DestView, content: &[u8], permissions: WritePermissions) -> Op {
    use crate::deploy::CopyPlan;
    let observed_mode = match &view.observed {
        Some(crate::deploy::Observation::File { mode, .. }) => Some(*mode),
        _ => None,
    };
    let intent_mode = permissions.resolve(view);
    let desired_hash: store::hash::ManifestHash =
        store::canonical_bytes_identity(content, intent_mode).into();
    // the manifest-domain live identity, mode-aware for every surface
    // that lands bytes (0043: templates joined copies); a foreign link
    // hashes its target
    let live = match &view.observed {
        None => None,
        Some(crate::deploy::Observation::Symlink { target }) => {
            Some(store::canonical_bytes_hash(target.as_encoded_bytes()).to_string())
        }
        Some(crate::deploy::Observation::File { bytes, mode }) => {
            Some(store::canonical_bytes_identity(bytes, *mode).to_string())
        }
    };
    // Admit old template receipts before policy dispatch. Only an intact,
    // non-preserved bytes-and-mode receipt denotes the live identity; this
    // must hold with take-over too, or a legacy managed file becomes a prior.
    let prev_pair = view
        .prev
        .map(|entry| match (&view.observed, live.as_deref()) {
            (Some(crate::deploy::Observation::File { bytes, mode }), Some(live))
                if entry.ownership.policy() == Ownership::Template
                    && entry.hash.as_str() != live
                    && entry.matches_file(bytes, *mode) =>
            {
                (live, false)
            }
            _ => (entry.hash.as_str(), entry.preserved_drift),
        });
    let plan = crate::deploy::plan_copy(
        desired_hash.as_str(),
        live.as_deref(),
        prev_pair,
        view.take_over,
    );
    let mut op = match plan {
        CopyPlan::Satisfied => {
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
        CopyPlan::Fresh | CopyPlan::Update => {
            let update = matches!(plan, CopyPlan::Update);
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
        CopyPlan::TakeOver => {
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
        CopyPlan::Preserve => {
            // the record holds the OBSERVED identity, marked preserved
            // (0029 §2) — it authorizes nothing
            view.base(
                OpKind::Preserved,
                None,
                Intended::Object(view.observed_identity().expect("preserve implies live")),
            )
            .with_produces(view.produces(
                store::hash::ManifestHash::from_raw(live.expect("Preserve implies a live object")),
                observed_mode,
                true,
            ))
        }
    };
    if let Some(produced) = op.produces_mut()
        && !produced.preserved_drift
        && let WritePermissions::Source { executable } = permissions
    {
        produced.source_executable = Some(executable);
    }
    op
}

pub(super) fn plan_migration_write(
    view: &DestView,
    authorization: &crate::deploy::MigrationEntry,
    content: &[u8],
    permissions: WritePermissions,
) -> Result<Op, ExecError> {
    authorization.reobserve(view)?;
    if !matches!(
        view.entry.mode,
        Ownership::TrackedCopy | Ownership::Template
    ) {
        return Err(ExecError::Step {
            module: view.module.into(),
            step: "migrate".into(),
            detail: "migration write requires a whole-file destination".into(),
        });
    }
    let mode = permissions.resolve(view);
    let identity = store::canonical_bytes_identity(content, mode);
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
