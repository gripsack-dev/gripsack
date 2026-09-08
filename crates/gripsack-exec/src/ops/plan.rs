//! Codegen (0034): observations + the desired state in, Ops out.
//! Every decision a destination needs is computed HERE — apply,
//! rollback, and plan share this module.

use super::*;
use crate::ctx::ExecError;
use store::journal::{Intended, ObjectIdentity};

/// Everything a planner needs to know about one destination — the
/// ONE observation, the lineage record, the run's take-over scope.
/// (A struct, not nine positional arguments: each field is named at
/// the call site.)
pub struct DestView<'a> {
    pub module: &'a str,
    pub entry: &'a Entry,
    /// The canonical physical destination (0030 §P0-1).
    pub dest: PathBuf,
    /// The home the store lives under (link "ours" is target-under-home).
    pub home: &'a Path,
    pub observed: Option<crate::deploy::Observation>,
    pub prev: Option<&'a store::DeployedEntry>,
    pub take_over: bool,
}

impl DestView<'_> {
    /// The journal-domain identity of the observation.
    pub(crate) fn observed_identity(&self) -> Option<ObjectIdentity> {
        self.observed.as_ref().map(|o| match o {
            crate::deploy::Observation::Symlink { target } => {
                ObjectIdentity::Link(target.to_string_lossy().into_owned())
            }
            crate::deploy::Observation::File { bytes, mode } => {
                ObjectIdentity::File(store::canonical_bytes_identity(bytes, *mode))
            }
        })
    }

    fn base(&self, kind: OpKind, authority: Option<Authority>, intended: Intended) -> Op {
        Op {
            module: self.module.to_string(),
            dest: self.dest.clone(),
            declared_to: self.entry.to.clone(),
            mode: self.entry.mode.clone(),
            kind,
            authority,
            observed: self.observed_identity(),
            intended,
            produces: None,
            note: None,
            removing: None,
        }
    }

    fn produces(
        &self,
        hash: store::hash::ManifestHash,
        file_mode: Option<u32>,
        preserved: bool,
    ) -> Option<ProducedEntry> {
        Some(ProducedEntry {
            from: std::path::PathBuf::from(&self.entry.from),
            mode: self.entry.mode.clone(),
            vars: self.entry.vars.clone(),
            hash,
            file_mode,
            source_executable: None,
            prior: None,
            preserved_drift: preserved,
        })
    }
}

/// What the mode's decision needs beyond the view.
pub enum ModeInput<'a> {
    /// Owned link: the store payload it points at, its content hash,
    /// and whether the live link already points there.
    Link {
        source: &'a Path,
        content_hash: store::hash::ManifestHash,
        already: bool,
    },
    /// Tracked copy / template: content plus source policy or exact restoration.
    Write {
        content: &'a [u8],
        permissions: WritePermissions,
    },
    /// Merge payload and permissions; hosting text is the view's one observation.
    Merge {
        payload: &'a str,
        permissions: WritePermissions,
    },
}

/// The per-destination decision (0034): one Op out, per the mode's
/// rule — plan_copy for copies/templates, plan_link for owned links,
/// the block hash for merge. THE function plan/apply/rollback share.
pub(crate) fn plan_entry_op(view: &DestView, input: ModeInput) -> Result<Op, ExecError> {
    match input {
        ModeInput::Link {
            source,
            content_hash,
            already,
        } => Ok(plan_link(view, source, content_hash, already)),
        ModeInput::Write {
            content,
            permissions,
        } => Ok(plan_write(view, content, permissions)),
        ModeInput::Merge {
            payload,
            permissions,
        } => plan_merge(view, payload, permissions),
    }
}

/// Ordinary deployment follows source executability without widening
/// acquired read/write permissions. Rollback restores an exact recorded mode.
#[derive(Clone, Copy)]
pub enum WritePermissions {
    Source { executable: bool },
    Preserve,
    Exact(u32),
}

impl WritePermissions {
    fn resolve(self, view: &DestView) -> u32 {
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
            matches!(entry.mode, Ownership::TrackedCopy | Ownership::Template)
                && !entry.preserved_drift
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

fn plan_write(view: &DestView, content: &[u8], permissions: WritePermissions) -> Op {
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
    let prev_pair = view.prev.map(|e| (e.hash.as_str(), e.preserved_drift));
    let mut plan = crate::deploy::plan_copy(
        desired_hash.as_str(),
        live.as_deref(),
        prev_pair,
        view.take_over,
    );
    // Read old template receipts without promoting bytes-only or preserved
    // observations into authority. The recorded permissions must also match.
    if plan == CopyPlan::Preserve
        && let Some(prev) = view.prev
        && let Some(crate::deploy::Observation::File { bytes, mode }) = &view.observed
        && prev.matches_file(bytes, *mode)
    {
        plan = CopyPlan::Update;
    }
    let mut op = match plan {
        CopyPlan::Satisfied => {
            // satisfied means managed (0029 §2): the record clears any
            // prior preserve mark and holds the desired identity — an
            // observation recorded as drift must not stick forever
            let mut op = view.base(
                OpKind::Satisfied,
                None,
                Intended::Object(view.observed_identity().expect("satisfied implies live")),
            );
            let landed_mode = match &view.observed {
                Some(crate::deploy::Observation::File { mode, .. }) => Some(*mode),
                _ => None,
            };
            op.produces = view.produces(desired_hash, landed_mode, false);
            op
        }
        CopyPlan::Fresh | CopyPlan::Update => {
            let update = matches!(plan, CopyPlan::Update);
            let mode = intent_mode;
            let mut op = view.base(
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
            );
            op.produces = view.produces(desired_hash, Some(mode), false);
            op
        }
        CopyPlan::TakeOver => {
            // adoption keeps the live mode (0033 R1)
            let mode = match &view.observed {
                Some(crate::deploy::Observation::File { mode, .. }) => *mode,
                _ => intent_mode,
            };
            let mut op = view.base(
                OpKind::Write {
                    content: ContentSource::Bytes(content.to_vec()),
                    mode,
                },
                Some(Authority::TakeOver),
                Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                    content, mode,
                ))),
            );
            // the prior is captured at execution (0015 §4)
            let written_hash = store::canonical_bytes_identity(content, mode).into();
            op.produces = view.produces(written_hash, Some(mode), false);
            op
        }
        CopyPlan::Preserve => {
            let mut op = view.base(
                OpKind::Preserved,
                None,
                Intended::Object(view.observed_identity().expect("preserve implies live")),
            );
            // the record holds the OBSERVED identity, marked preserved
            // (0029 §2) — it authorizes nothing
            op.produces = view.produces(
                store::hash::ManifestHash::from_raw(live.expect("Preserve implies a live object")),
                observed_mode,
                true,
            );
            op
        }
    };
    if let Some(produced) = &mut op.produces
        && !produced.preserved_drift
        && let WritePermissions::Source { executable } = permissions
    {
        produced.source_executable = Some(executable);
    }
    op
}

fn plan_link(
    view: &DestView,
    source: &Path,
    content_hash: store::hash::ManifestHash,
    already: bool,
) -> Op {
    use crate::deploy::LinkPlan;
    let exists = view.observed.is_some();
    let ours = match &view.observed {
        Some(crate::deploy::Observation::Symlink { target }) => {
            Path::new(target.as_os_str()).starts_with(view.home)
        }
        _ => false,
    };
    let recorded = view.prev.is_some_and(|e| !e.preserved_drift);
    let link_intent = Intended::Object(ObjectIdentity::Link(source.to_string_lossy().into_owned()));
    match crate::deploy::plan_link(exists, ours, recorded, view.take_over) {
        LinkPlan::Link if already => {
            let mut op = view.base(OpKind::Satisfied, None, link_intent);
            op.produces = view.produces(content_hash, None, false);
            op
        }
        LinkPlan::Link => {
            let mut op = view.base(
                OpKind::Link {
                    target: source.to_path_buf(),
                },
                Some(Authority::Fresh),
                link_intent,
            );
            op.produces = view.produces(content_hash, None, false);
            op
        }
        LinkPlan::TakeOver => {
            let mut op = view.base(
                OpKind::Link {
                    target: source.to_path_buf(),
                },
                Some(Authority::TakeOver),
                link_intent,
            );
            op.produces = view.produces(content_hash, None, false);
            op
        }
        // rendered "foreign — needs --take-over"; apply refuses
        LinkPlan::Refuse => view.base(
            OpKind::Preserved,
            Some(Authority::Foreign),
            Intended::Object(view.observed_identity().expect("refuse implies an object")),
        ),
    }
}

fn plan_merge(
    view: &DestView,
    payload: &str,
    permissions: WritePermissions,
) -> Result<Op, ExecError> {
    let fail = |detail: String| ExecError::Step {
        module: view.module.to_string(),
        step: "deploy".into(),
        detail: format!("{}: {detail}", view.entry.to),
    };
    let existing = match &view.observed {
        None => "",
        Some(crate::deploy::Observation::File { bytes, .. }) => std::str::from_utf8(bytes)
            .map_err(|e| fail(format!("hosting file is not UTF-8: {e}")))?,
        Some(crate::deploy::Observation::Symlink { .. }) => {
            return Err(fail(
                "hosting file is a symlink; refusing to replace foreign ownership".into(),
            ));
        }
    };
    let blocks = crate::managed_blocks::ManagedBlockSet::parse(existing, view.module)
        .map_err(|e| fail(e.to_string()))?;
    let mode = permissions.resolve(view);
    if let Some(crate::deploy::Observation::File {
        mode: live_mode, ..
    }) = &view.observed
        && !blocks.is_empty()
        && blocks.mode_conflicts(*live_mode, view.prev)
    {
        let mut op = view.base(
            OpKind::Preserved,
            None,
            Intended::Object(view.observed_identity().expect("block implies live")),
        );
        // An explicitly non-authoritative observation of the whole hosting text.
        op.produces = view.produces(
            store::canonical_bytes_hash(existing.as_bytes()).into(),
            Some(*live_mode),
            true,
        );
        return Ok(op);
    }
    let hash = crate::managed_blocks::content_hash(payload);
    if blocks.satisfied(&hash, mode)
        && matches!(&view.observed, Some(crate::deploy::Observation::File { mode: live, .. }) if *live == mode)
    {
        let mut op = view.base(
            OpKind::Satisfied,
            None,
            Intended::Object(view.observed_identity().expect("satisfied implies live")),
        );
        op.produces = view.produces(hash.into(), Some(mode), false);
        return Ok(op);
    }
    let spliced = blocks
        .upsert(
            view.module,
            &view.dest,
            view.entry.marker.as_deref(),
            payload,
            mode,
        )
        .map_err(|e| fail(e.to_string()))?;
    let mut op = view.base(
        OpKind::MergeUpsert {
            payload: payload.as_bytes().to_vec(),
            marker: view.entry.marker.clone(),
            mode,
        },
        Some(Authority::Update),
        Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
            spliced.as_bytes(),
            mode,
        ))),
    );
    op.produces = view.produces(hash.into(), Some(mode), false);
    op.note = blocks.report_note();
    Ok(op)
}

/// A Remove op (prune-on-undeclare, rollback's current-only
/// destinations): removal authority is the manifest entry, the intent
/// is the prior's restoration or REMOVED (0026 §6). None when nothing
/// needs doing; Preserved when the drift guard keeps the destination
/// (a drifted merge block, a modified copy — the user's now).
pub(crate) fn plan_remove_op(
    module: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    home: &Path,
) -> Result<Option<Op>, ExecError> {
    let fail = |detail: String| ExecError::Step {
        module: module.to_string(),
        step: "plan".into(),
        detail,
    };
    let dest = store::canonical_dest(&entry.to)
        .map_err(|e| fail(format!("destination {:?}: {e}", entry.to)))?;
    let (dest_dir, dest_name) =
        crate::deploy::dest_capability(&dest).map_err(|e| ExecError::Step {
            module: module.to_string(),
            step: "plan".into(),
            detail: format!("cannot open {} parent: {e}", entry.to),
        })?;
    let observed = store::journal::live_identity(&dest_dir, &dest_name)
        .map_err(|e| fail(format!("cannot inspect {}: {e}", entry.to)))?;
    let kept = |note: String| {
        tracing::warn!("{note}");
        Ok(Some(Op {
            module: module.to_string(),
            dest: dest.clone(),
            declared_to: entry.to.clone(),
            mode: entry.mode.clone(),
            kind: OpKind::Preserved,
            authority: None,
            observed: observed.clone(),
            intended: match &observed {
                Some(o) => Intended::Object(o.clone()),
                None => Intended::Removed,
            },
            produces: None,
            note: None,
            removing: None,
        }))
    };
    if entry.mode == Ownership::Merge {
        // the file is foreign — prune removes only our block, and only
        // if the block is still what we deployed
        let existing = match crate::deploy::observe(&dest_dir, &dest_name)? {
            Some(crate::deploy::Observation::File { bytes, mode }) => (bytes, mode),
            None => return Ok(None),
            _ => return kept(format!("kept {} — hosting file replaced", entry.to)),
        };
        let (bytes, splice_mode) = existing;
        let existing = String::from_utf8(bytes).map_err(|e| fail(e.to_string()))?;
        let blocks = crate::managed_blocks::ManagedBlockSet::parse(&existing, module)
            .map_err(|e| fail(e.to_string()))?;
        match blocks.blocks() {
            [] => return Ok(None),
            _ if blocks.intact(entry, splice_mode) => {
                let new = blocks.remove().expect("intact implies a block");
                let intended = if new.is_empty() {
                    Intended::Removed
                } else {
                    Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                        new.as_bytes(),
                        splice_mode,
                    )))
                };
                return Ok(Some(Op {
                    module: module.to_string(),
                    dest,
                    declared_to: entry.to.clone(),
                    mode: entry.mode.clone(),
                    kind: OpKind::Remove,
                    authority: Some(Authority::Update),
                    observed,
                    intended,
                    produces: None,
                    note: None,
                    removing: Some((entry.clone(), store_path.to_path_buf())),
                }));
            }
            _ => {
                return kept(format!(
                    "kept {} — block or hosting-file mode modified since deploy",
                    entry.to
                ));
            }
        }
    }
    // the drift guard runs FIRST (0026 §6) — a kept destination is
    // never journaled at all
    if !crate::deploy::intact_deployed(&dest, entry, store_path) {
        if dest.symlink_metadata().is_ok() {
            return kept(format!("kept {} — modified since deploy", entry.to));
        }
        return Ok(None); // already gone
    }
    let intended =
        crate::deploy::restore::prune_intent(entry, home).map_err(|e| fail(format!("{e}")))?;
    Ok(Some(Op {
        module: module.to_string(),
        dest,
        declared_to: entry.to.clone(),
        mode: entry.mode.clone(),
        kind: OpKind::Remove,
        authority: Some(Authority::Update),
        observed,
        intended,
        produces: None,
        note: None,
        removing: Some((entry.clone(), store_path.to_path_buf())),
    }))
}

/// A restore op for rollback (0034): the desired state is the TARGET
/// generation's manifest record, planned by the same per-destination
/// function apply uses. None when no safe restore exists (the caller
/// surfaces a Skipped note — 0030 §H8).
pub(crate) fn plan_restore_op(
    module: &str,
    entry: &store::DeployedEntry,
    store_path: &Path,
    prev: Option<&store::DeployedEntry>,
    home: &Path,
) -> Result<Option<Op>, ExecError> {
    let fail = |detail: String| ExecError::Step {
        module: module.to_string(),
        step: "rollback".into(),
        detail,
    };
    // the planner speaks IR entries; the manifest record synthesizes
    // one (the marker is deploy-time only — merge restore upserts
    // with the default)
    let ir_entry = Entry {
        from: entry.from.to_string_lossy().into_owned(),
        to: entry.to.clone(),
        mode: entry.mode.clone(),
        vars: entry.vars.clone(),
        marker: None,
        span: None,
    };
    let dest = store::canonical_dest(&entry.to)
        .map_err(|e| fail(format!("destination {:?}: {e}", entry.to)))?;
    let (dest_dir, dest_name) = crate::deploy::dest_capability(&dest)
        .map_err(|e| fail(format!("cannot open {} parent: {e}", entry.to)))?;
    let observed = crate::deploy::observe(&dest_dir, &dest_name)
        .map_err(|e| fail(format!("cannot inspect {}: {e}", entry.to)))?;
    let source = store_path.join(&entry.from);
    let view = DestView {
        module,
        entry: &ir_entry,
        dest,
        home,
        observed,
        prev,
        take_over: false, // rollback never absorbs
    };
    let mut op = match entry.mode {
        Ownership::Owned => {
            if !source.exists() {
                return Ok(None);
            }
            let already = std::fs::read_link(&view.dest)
                .map(|t| t == source)
                .unwrap_or(false);
            plan_entry_op(
                &view,
                ModeInput::Link {
                    source: &source,
                    content_hash: store::canonical_file_hash(&source)
                        .map_err(|e| fail(format!("{e}")))?
                        .into(),
                    already,
                },
            )?
        }
        Ownership::TrackedCopy | Ownership::Template => {
            let bytes = match std::fs::read(&source) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(fail(format!("{e}"))),
            };
            let rendered;
            let content: &[u8] = match entry.mode {
                Ownership::Template => {
                    rendered = crate::template::render_template(
                        &bytes,
                        &entry.vars,
                        &entry.from.to_string_lossy(),
                    )
                    .map_err(|e| fail(format!("{e}")))?;
                    &rendered
                }
                _ => &bytes,
            };
            // exact mode restoration (0031): the recorded landed mode;
            // an unrecorded one follows the payload's exec bit
            #[cfg(unix)]
            let payload_exec = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(&source)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            };
            #[cfg(not(unix))]
            let payload_exec = false;
            let intent_mode = entry
                .file_mode
                .unwrap_or(if payload_exec { 0o755 } else { 0o644 });
            plan_entry_op(
                &view,
                ModeInput::Write {
                    content,
                    permissions: WritePermissions::Exact(intent_mode),
                },
            )?
        }
        Ownership::Merge => {
            let payload = match std::fs::read_to_string(&source) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(fail(format!("{e}"))),
            };
            match &view.observed {
                None => {}
                Some(crate::deploy::Observation::File { bytes, .. })
                    if std::str::from_utf8(bytes).is_ok() => {}
                _ => return Ok(None),
            }
            plan_entry_op(
                &view,
                ModeInput::Merge {
                    payload: &payload,
                    permissions: entry
                        .file_mode
                        .map_or(WritePermissions::Preserve, WritePermissions::Exact),
                },
            )?
        }
    };
    if let Some(produced) = &mut op.produces
        && !produced.preserved_drift
    {
        produced.source_executable = entry.source_executable;
    }
    Ok(Some(op))
}
