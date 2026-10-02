//! Native previews prepare the same captured content as apply, but never
//! publish it or execute a verifier, hook, builder or destination operation.
use super::{NativeProfiles, file_failure};
use crate::ctx::ExecError;
use crate::ops::{DestView, ModeInput, Op, WritePermissions, plan_entry_op, plan_remove_op};
use gripsack_ir::Ownership;
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

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
        for (owner, profile) in &self.profiles {
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
