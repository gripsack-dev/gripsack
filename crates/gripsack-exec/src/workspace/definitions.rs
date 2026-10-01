//! Definition identities use approved inventory metadata, not today's files or
//! the whole application checkout. Aliases include their captured target bytes.
use crate::{ExecError, Repository};
use gripsack_ir::workspace_v6::{identity::DefinitionDigest, lock::DefinitionPins};
use gripsack_store::source_bundle::{SourceEntry, SourceObject};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

pub(super) fn captured_definitions(repository: &Repository) -> Result<DefinitionPins, ExecError> {
    let Repository::Evaluated { sources, .. } = repository else {
        return Err(ExecError::Step {
            module: "workspace".into(),
            step: "definition-pins".into(),
            detail: "workspace production requires an approved captured frontend/import bundle"
                .into(),
        });
    };
    let entries = sources.inventory().entries();
    let frontend = subtree(entries, "frontend")?;
    let mut imports = BTreeMap::new();
    if sources.pinned_frontend().is_some() {
        imports.insert("@gripsack/core".into(), subtree(entries, "pin")?);
    }
    let prefix = "repo/node_modules/";
    let mut packages = BTreeSet::new();
    for entry in entries {
        let Some(relative) = entry.path.strip_prefix(prefix) else {
            continue;
        };
        if relative.starts_with('.') {
            continue;
        }
        let end = if relative.starts_with('@') {
            let Some(scope) = relative.find('/') else {
                continue;
            };
            relative[scope + 1..]
                .find('/')
                .map_or(relative.len(), |end| scope + 1 + end)
        } else {
            relative.find('/').unwrap_or(relative.len())
        };
        let package = &relative[..end];
        if package == "@gripsack/core" {
            continue;
        }
        packages.insert(&entry.path[..prefix.len() + end]);
    }
    for package in packages {
        imports.insert(
            package[prefix.len()..].to_owned(),
            subtree(entries, package)?,
        );
    }
    Ok(DefinitionPins { frontend, imports })
}

struct Fingerprint(Sha256);
impl Write for Fingerprint {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn subtree(entries: &[SourceEntry], root: &str) -> Result<DefinitionDigest, ExecError> {
    // The admitted inventory is sorted and globally bounded. A bit set avoids
    // copying metadata or allocating a tree node for every selected source.
    let mut included = vec![false; entries.len()];
    let mut pending = vec![root];
    while let Some(prefix) = pending.pop() {
        let index = entries
            .binary_search_by(|entry| entry.path.as_str().cmp(prefix))
            .map_err(|_| ExecError::Step {
                module: "workspace".into(),
                step: "definition-pins".into(),
                detail: format!("captured definition root {prefix:?} is absent"),
            })?;
        if included[index] {
            continue;
        }
        // A sibling such as `helpers-addon` sorts between `helpers` and
        // `helpers/body.ts`; descendant lookup includes the path separator.
        let descendants = format!("{prefix}/");
        let begin = entries.partition_point(|entry| entry.path.as_str() < descendants.as_str());
        let children = entries
            .iter()
            .enumerate()
            .skip(begin)
            .take_while(|(_, entry)| entry.path.starts_with(&descendants));
        for (index, entry) in std::iter::once((index, &entries[index])).chain(children) {
            if included[index] {
                continue;
            }
            included[index] = true;
            if let SourceObject::Alias { target } = &entry.object {
                pending.push(target);
            }
        }
    }
    let mut fingerprint = Fingerprint(Sha256::new());
    fingerprint.write_all(b"gripsack-definition-inventory-v1\0")?;
    for (entry, included) in entries.iter().zip(included) {
        if !included {
            continue;
        }
        let relative = entry
            .path
            .strip_prefix(root)
            .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'));
        // A role tag separates selected-root paths from out-of-root alias
        // targets; adjacent serialized tuples have unambiguous boundaries.
        serde_json::to_writer(
            &mut fingerprint,
            &(
                relative.is_some(),
                relative.unwrap_or(&entry.path),
                &entry.object,
            ),
        )?;
    }
    Ok(DefinitionDigest::from_bytes(
        fingerprint.0.finalize().into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_store::{prior::FileMode, source_bundle::SourceFileBytes};

    fn file(path: &str, content: &[u8]) -> SourceEntry {
        SourceEntry {
            path: path.into(),
            object: SourceObject::File {
                bytes: SourceFileBytes::try_from(content.len() as u64).unwrap(),
                sha256: gripsack_process::Sha256Digest::of(content),
                mode: FileMode::try_from(0o644).unwrap(),
            },
        }
    }

    #[test]
    fn definition_identity_follows_alias_targets_without_hashing_unrelated_sources() {
        let mut entries = vec![
            SourceEntry {
                path: "pin".into(),
                object: SourceObject::Directory,
            },
            SourceEntry {
                path: "pin/shared".into(),
                object: SourceObject::Alias {
                    target: "repo/helpers".into(),
                },
            },
            SourceEntry {
                path: "repo".into(),
                object: SourceObject::Directory,
            },
            file("repo/app.ts", b"application"),
            SourceEntry {
                path: "repo/helpers".into(),
                object: SourceObject::Directory,
            },
            SourceEntry {
                path: "repo/helpers-addon".into(),
                object: SourceObject::Directory,
            },
            file("repo/helpers-addon/other.ts", b"unrelated"),
            file("repo/helpers/body.ts", b"original"),
        ];
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let original = subtree(&entries, "pin").unwrap();
        let selected = entries
            .iter_mut()
            .find(|entry| entry.path == "repo/helpers/body.ts")
            .unwrap();
        *selected = file("repo/helpers/body.ts", b"changed");
        assert_ne!(
            subtree(&entries, "pin").unwrap(),
            original,
            "captured_alias_bytes_were_not_bound"
        );
        *entries
            .iter_mut()
            .find(|entry| entry.path == "repo/helpers/body.ts")
            .unwrap() = file("repo/helpers/body.ts", b"original");
        *entries
            .iter_mut()
            .find(|entry| entry.path == "repo/app.ts")
            .unwrap() = file("repo/app.ts", b"edited application");
        *entries
            .iter_mut()
            .find(|entry| entry.path == "repo/helpers-addon/other.ts")
            .unwrap() = file("repo/helpers-addon/other.ts", b"edited sibling");
        assert_eq!(
            subtree(&entries, "pin").unwrap(),
            original,
            "unrelated source invalidated a definition pin"
        );
    }
}
