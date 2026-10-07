//! Strict persisted-generation admission shared by readers and publishers.
use super::Generation;
use crate::{ManagedBlockId, prior::FileMode};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

enum DestinationClaim<'a> {
    Whole,
    Blocks {
        ids: BTreeSet<&'a ManagedBlockId>,
        mode: FileMode,
    },
}

pub(super) fn validate(
    manifest: &Generation,
    generation: crate::GenerationId,
    home: &Path,
) -> io::Result<()> {
    let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidData, why);
    if manifest.number != generation {
        return Err(invalid(format!(
            "generation {generation}'s manifest claims number {} — directory and identity disagree",
            manifest.number
        )));
    }
    let mut destinations: BTreeMap<String, DestinationClaim<'_>> = BTreeMap::new();
    for (name, state) in &manifest.modules {
        if state.build_only
            && (!state.entries.is_empty() || !state.intents.is_empty() || !state.env.is_empty())
        {
            return Err(invalid(format!(
                "module {name:?}: build-only state contains deployment effects"
            )));
        }
        if state.env.iter().any(|value| !value.valid_structured()) {
            return Err(invalid(format!(
                "module {name:?}: invalid structured environment contribution"
            )));
        }
        crate::paths::validate_store_root(home, &state.store_path)
            .map_err(|error| invalid(format!("module {name:?}: {error}")))?;
        for path in &state.build_closure {
            crate::paths::validate_store_root(home, path)
                .map_err(|error| invalid(format!("module {name:?} build closure: {error}")))?;
        }
        for intent in &state.intents {
            if let crate::activation::ActivationAction::WorkspaceHook { context, .. } = &intent.action
                && (context.parent() != Some(state.store_path.as_path())
                    || context.file_name().is_none())
            {
                return Err(invalid(format!(
                    "module {name:?}: workspace hook context is outside its retained profile"
                )));
            }
        }
        for entry in &state.entries {
            if entry
                .from
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(invalid(format!(
                    "module {name:?}: source {:?} is not a plain relative path",
                    entry.from
                )));
            }
            // Legacy duplicate-path state remains invalid. New workspace blocks
            // have explicit identities and may share a physical destination.
            let block = entry.ownership.block_id();
            let mode = entry
                .file_mode
                .map(FileMode::try_from)
                .transpose()
                .map_err(|error| {
                    invalid(format!(
                        "module {name:?}: destination {:?}: {error}",
                        entry.to
                    ))
                })?;
            let block_mode = block
                .map(|_| {
                    mode.ok_or_else(|| {
                        invalid(format!(
                            "module {name:?}: managed block at {:?} has no hosting-file mode",
                            entry.to
                        ))
                    })
                })
                .transpose()?;
            let key = entry.key().to_string_lossy().to_lowercase();
            let duplicate = match destinations.entry(key) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(match (block, block_mode) {
                        (Some(id), Some(mode)) => DestinationClaim::Blocks {
                            ids: BTreeSet::from([id]),
                            mode,
                        },
                        _ => DestinationClaim::Whole,
                    });
                    false
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    match (slot.get_mut(), block) {
                        (DestinationClaim::Blocks { ids, mode }, Some(identity)) => {
                            Some(*mode) != block_mode || !ids.insert(identity)
                        }
                        _ => true,
                    }
                }
            };
            if duplicate {
                return Err(invalid(format!(
                    "destination {:?} has conflicting ownership in generation {generation}",
                    entry.to
                )));
            }
            if entry.hash.as_str().len() != 64
                || !entry
                    .hash
                    .as_str()
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(invalid(format!(
                    "module {name:?}: destination {:?} has a malformed content hash",
                    entry.to
                )));
            }
        }
    }
    Ok(())
}
