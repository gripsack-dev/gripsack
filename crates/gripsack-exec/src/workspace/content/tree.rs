//! Artifact-side counterpart of treeFiles: bounded segment-prefix selection,
//! stable per-file expansion, and no implicit ownership of a whole directory.
use super::{Capture, File, PreparedFile, Source, prepare_file};
use crate::ExecError;
use gripsack_ir::workspace::WorkspaceDestination;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

const MAX_EXPANDED_FILES: usize = 10_000;

pub(super) fn prepare_into(
    capture: &mut Capture,
    declaration: &File<'_>,
    stage: &Path,
    prepared: &mut Vec<PreparedFile>,
) -> Result<(), ExecError> {
    let Some(Source::Tree {
        root,
        include,
        exclude,
        identity,
    }) = &declaration.source
    else {
        unreachable!("tree dispatch");
    };
    let root_cap = gripsack_fs::open(root)?;
    let mut pending = vec![(root_cap, PathBuf::new())];
    let mut selected = BTreeMap::new();
    let mut entries = 0usize;
    let mut bytes = 0u64;
    let mut names = 0u64;
    while let Some((directory, relative)) = pending.pop() {
        for entry in directory.entries()? {
            let entry = entry?;
            entries = entries
                .checked_add(1)
                .filter(|count| *count <= capture.maximum_entries)
                .ok_or_else(|| invalid("artifact tree inventory exceeds its entry bound"))?;
            let name = entry.file_name();
            let path = relative.join(&name);
            let text = path
                .to_str()
                .ok_or_else(|| invalid("artifact tree path is not UTF-8"))?;
            names = names
                .checked_add(text.len() as u64)
                .filter(|size| *size <= capture.maximum_metadata)
                .ok_or_else(|| {
                    invalid("artifact tree path inventory exceeds its metadata bound")
                })?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                // Includes may name a deeper child; never prune its ancestors.
                pending.push((
                    gripsack_fs::open_dir_nofollow(&directory, Path::new(&name))?,
                    path,
                ));
                continue;
            }
            if !kind.is_file() {
                return Err(invalid("artifact tree contains a symlink or special entry").into());
            }
            let covered = |prefix: &String| {
                text.strip_prefix(prefix.as_str())
                    .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
            };
            if !include.iter().any(covered) || exclude.iter().any(covered) {
                continue;
            }
            if selected.len() == MAX_EXPANDED_FILES {
                return Err(
                    invalid("artifact tree expands beyond the per-file declaration bound").into(),
                );
            }
            let metadata = directory.symlink_metadata(&name)?;
            bytes = bytes
                .checked_add(metadata.len())
                .filter(|size| *size <= capture.maximum_tree_bytes)
                .ok_or_else(|| invalid("artifact tree selected content exceeds its byte bound"))?;
            selected.insert(text.to_owned(), root.join(&path));
        }
    }
    for (relative, path) in selected {
        let destination = match declaration.destination {
            WorkspaceDestination::Symlink { path } => WorkspaceDestination::Symlink {
                path: join_destination(path, &relative),
            },
            WorkspaceDestination::TrackedCopy { path } => WorkspaceDestination::TrackedCopy {
                path: join_destination(path, &relative),
            },
            WorkspaceDestination::ManagedBlock { path, marker } => {
                WorkspaceDestination::ManagedBlock {
                    path: join_destination(path, &relative),
                    marker: marker.clone(),
                }
            }
        };
        let leaf = File {
            span: declaration.span,
            source: Some(Source::ArtifactFile {
                path,
                identity: serde_json::to_string(&(identity, &relative))?,
            }),
            content: declaration.content,
            destination: &destination,
        };
        prepared.push(prepare_file(capture, &leaf, stage)?);
    }
    Ok(())
}
fn join_destination(root: &str, relative: &str) -> String {
    format!("{}/{relative}", root.trim_end_matches('/'))
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
