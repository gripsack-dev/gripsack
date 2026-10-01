//! Native previews prepare the same captured content as apply, but never
//! publish it or execute a verifier, hook, builder or destination operation.
use super::{NativeProfiles, file_failure};
use crate::ctx::ExecError;
use crate::ops::{DestView, ModeInput, Op, OpKind, WritePermissions, plan_entry_op, plan_remove_op};
use gripsack_ir::Ownership;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(super) struct DeferredFile {
    entry: gripsack_ir::Entry,
    block: Option<store::ManagedBlockId>,
    tree: bool,
    output: String,
}
impl DeferredFile {
    pub fn new(file: &gripsack_ir::workspace_v6::WorkspaceFile) -> Result<Self, ExecError> {
        use gripsack_ir::{workspace::WorkspaceDestination, workspace_v6::WorkspaceSource};
        let (path, mode, block) = match &file.destination {
            WorkspaceDestination::Symlink { path } => (path, Ownership::Owned, None),
            WorkspaceDestination::TrackedCopy { path } => (path, Ownership::TrackedCopy, None),
            WorkspaceDestination::ManagedBlock { path, marker } =>
                (path, Ownership::Merge, Some(store::ManagedBlockId::from_marker(marker)?)),
        };
        let (output, tree) = match &file.source {
            Some(WorkspaceSource::ArtifactFile { output, .. }) => (output, false),
            Some(WorkspaceSource::Tree { output, .. }) => (output, true),
            _ => return Err(file_failure(&file.span, "deferred file has no producer")),
        };
        Ok(Self {
            entry: gripsack_ir::Entry {
                from: String::new(), to: path.clone(), mode, vars: Default::default(),
                marker: None, span: Some(file.span.clone()),
            },
            block, tree, output: output.clone(),
        })
    }
    pub fn note(&self) -> String {
        let span = self.entry.span.as_ref().expect("deferred declaration carries provenance");
        format!("{}:{}: {:?} → {} (matching frozen output unavailable; {} resolved at apply; no fetch/build performed)",
            span.file, span.line, self.output, self.entry.to,
            if self.tree { "tree membership, collisions and pruning" } else { "file content" })
    }
}

fn deferred(view: &DestView<'_>, note: String) -> Op {
    Op::new(view.module.to_owned(), view.dest.clone(), view.entry.to.clone(),
        view.entry.mode.clone(), OpKind::Deferred, None, view.observed_identity(),
        store::journal::Intended::Removed, None, Some(note))
}

impl NativeProfiles {
    pub(crate) fn preview(
        &self,
        home: &Path,
        previous: Option<&store::Generation>,
        adopting: &BTreeSet<String>,
    ) -> Result<Vec<Op>, ExecError> {
        let empty = BTreeMap::new();
        let modules = previous.map_or(&empty, |generation| &generation.modules);
        let lineage = crate::schedule::previous_ownership(modules);
        let mut operations = Vec::new();
        let mut declared_paths = BTreeSet::new();
        let mut declared_blocks = BTreeSet::new();
        let mut unknown_trees = Vec::new();
        for (owner, profile) in &self.profiles {
            if let Some(environment) = &profile.deferred_environment {
                operations.push(Op::new(owner.clone(), PathBuf::new(), String::new(), Ownership::Owned,
                    OpKind::Deferred, None, None, store::journal::Intended::Removed, None,
                    Some(format!("{}:{}: environment {environment:?} → profile activation (runtime admission deferred; no activation performed)",
                        profile.span.file, profile.span.line))));
            }
            for file in &profile.deferred {
                let entry = &file.entry;
                let dest = store::canonical_dest(&entry.to)
                    .map_err(|error| file_failure(entry.span.as_ref().unwrap_or(&profile.span), error))?;
                let key = store::OwnershipKey::new(dest.clone(), file.block.as_ref().map(store::ManagedBlockId::as_str));
                if file.tree {
                    // A tree root is a container, never a managed file. Resolve
                    // a directory alias so prior physical children stay covered.
                    let root = match std::fs::metadata(&dest) {
                        Ok(metadata) if metadata.is_dir() => dest.canonicalize()?,
                        Ok(_) => return Err(file_failure(entry.span.as_ref().unwrap_or(&profile.span), "tree destination root is not a directory")),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => dest.clone(),
                        Err(error) => return Err(file_failure(entry.span.as_ref().unwrap_or(&profile.span), error)),
                    };
                    unknown_trees.push(root);
                    operations.push(Op::new(owner.clone(), dest, entry.to.clone(), entry.mode.clone(),
                        OpKind::Deferred, None, None, store::journal::Intended::Removed, None, Some(file.note())));
                    continue;
                } else {
                    declared_paths.insert(dest.clone());
                    if entry.mode == Ownership::Merge { declared_blocks.insert(key.clone()); }
                }
                let view = DestView {
                    module: owner, entry, dest: dest.clone(), home,
                    observed: crate::deploy::observe_readonly(&dest)?,
                    prev: lineage.get(&key).copied(), take_over: adopting.contains(&entry.to),
                };
                operations.push(deferred(&view, file.note()));
            }
            for file in &profile.files {
                let entry = &file.entry;
                let span = entry.span.as_ref().unwrap_or(&profile.span);
                let dest =
                    store::canonical_dest(&entry.to).map_err(|error| file_failure(span, error))?;
                let key = store::OwnershipKey::new(
                    dest.clone(),
                    file.block_id.as_ref().map(store::ManagedBlockId::as_str),
                );
                declared_paths.insert(dest.clone());
                if entry.mode == Ownership::Merge {
                    declared_blocks.insert(key.clone());
                }
                let source = profile.stage.join(&entry.from);
                let view = DestView {
                    module: owner,
                    entry,
                    dest: dest.clone(),
                    home,
                    observed: crate::deploy::observe_readonly(&dest)?,
                    prev: lineage.get(&key).copied(),
                    take_over: adopting.contains(&entry.to),
                };
                if entry.mode == Ownership::Owned
                    && (!profile.deferred.is_empty() || profile.deferred_environment.is_some())
                {
                    operations.push(deferred(&view, format!("{}:{}: profile content identity awaits deferred files/environment; captured file bytes are known",
                        span.file, span.line)));
                    continue;
                }
                let operation = match entry.mode {
                    Ownership::Owned => {
                        let target = profile.store_path.join(&entry.from);
                        let already = std::fs::read_link(&dest).is_ok_and(|path| path == target);
                        plan_entry_op(
                            &view,
                            ModeInput::Link {
                                source: &target,
                                content_hash: store::canonical_file_hash(&source)?.into(),
                                already,
                            },
                        )?
                    }
                    Ownership::TrackedCopy => {
                        let content = std::fs::read(&source)?;
                        #[cfg(unix)]
                        let executable = {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::metadata(&source)?.permissions().mode() & 0o111 != 0
                        };
                        #[cfg(not(unix))]
                        let executable = false;
                        plan_entry_op(
                            &view,
                            ModeInput::Write {
                                content: &content,
                                permissions: WritePermissions::Source { executable },
                            },
                        )?
                    }
                    Ownership::Merge => {
                        let payload = std::fs::read_to_string(&source)?;
                        plan_entry_op(
                            &view,
                            ModeInput::Merge {
                                payload: &payload,
                                permissions: WritePermissions::Preserve,
                                block_id: file.block_id.as_ref(),
                            },
                        )?
                    }
                    Ownership::Template => {
                        return Err(file_failure(
                            span,
                            "workspace template was not prepared as content",
                        ));
                    }
                };
                operations.push(operation);
            }
        }
        if let Some(previous) = previous {
            let home_cap = gripsack_fs::open(home)?;
            for (owner, state) in &previous.modules {
                if self.partial && !self.profiles.contains_key(owner) {
                    continue;
                }
                for entry in &state.entries {
                    // Unknown tree membership is not an empty tree. In particular,
                    // it cannot authorize pruning previously deployed children.
                    if !unknown_trees.is_empty() {
                        let current = store::canonical_dest(&entry.to)?;
                        let retained = entry.key();
                        if unknown_trees.iter().any(|root| retained.starts_with(root) || current.starts_with(root)) {
                            continue;
                        }
                    }
                    let declared = if entry.ownership.policy() == Ownership::Merge {
                        declared_blocks.contains(&entry.ownership_key(owner))
                    } else {
                        declared_paths.contains(&entry.key())
                    };
                    if !declared
                        && !entry.preserved_drift
                        && let Some(operation) =
                            plan_remove_op(owner, entry, &state.store_path, &home_cap)?
                    {
                        operations.push(operation);
                    }
                }
            }
        }
        Ok(operations)
    }
}
