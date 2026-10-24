//! Expand immutable repository aliases under logical destination names. The
//! held repository capability deliberately excludes sibling captured SDK roots.
use super::selection::{MAX_EXPANDED_FILES, MAX_TREE_DEPTH, Selection, destination, invalid};
use super::{Capture, Content, File, PreparedFile, Source, prepare_file};
use crate::ExecError;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn prepare_into(
    capture: &mut Capture,
    declaration: &File<'_>,
    stage: &Path,
    prepared: &mut Vec<PreparedFile>,
) -> Result<(), ExecError> {
    let Some(Source::RepoTree {
        path,
        include,
        exclude,
    }) = &declaration.source
    else {
        unreachable!("repository tree dispatch");
    };
    if !matches!(declaration.content, Content::Identity) {
        return Err(invalid("repository trees require identity content").into());
    }
    let physical = capture.repository.canonicalize(path)?;
    let mut inventory = Inventory {
        capture,
        selection: Selection::new(include, exclude),
        selected: BTreeMap::new(),
        entries: 0,
        bytes: 0,
        names: 0,
    };
    inventory.directory(&physical, Path::new(""), &mut Vec::new())?;
    for (relative, resolved) in inventory.selected {
        let destination = destination(declaration.destination, &relative);
        let leaf = File {
            span: declaration.span,
            source: Some(Source::RepoTreeFile {
                identity: serde_json::to_string(&(path, include, exclude, &relative, &resolved))?,
                path: resolved,
            }),
            content: Content::Identity,
            destination: &destination,
            checks: declaration.checks,
        };
        prepared.push(prepare_file(capture, &leaf, stage)?);
    }
    Ok(())
}

struct Inventory<'a> {
    capture: &'a Capture,
    selection: Selection<'a>,
    selected: BTreeMap<String, String>,
    entries: usize,
    bytes: u64,
    names: u64,
}

impl Inventory<'_> {
    fn directory(
        &mut self,
        physical: &Path,
        relative: &Path,
        ancestors: &mut Vec<PathBuf>,
    ) -> Result<(), ExecError> {
        if ancestors.len() >= MAX_TREE_DEPTH {
            return Err(invalid("repository tree exceeds its path-depth bound").into());
        }
        if ancestors.iter().any(|ancestor| ancestor == physical) {
            return Err(invalid("repository tree contains a directory alias cycle").into());
        }
        let directory = gripsack_fs::open_dir_nofollow(&self.capture.repository, physical)?;
        ancestors.push(physical.to_owned());
        for entry in directory.entries()? {
            let entry = entry?;
            self.entries = self
                .entries
                .checked_add(1)
                .filter(|count| *count <= self.capture.maximum_entries)
                .ok_or_else(|| invalid("repository tree inventory exceeds its entry bound"))?;
            let name = entry.file_name();
            let logical = relative.join(&name);
            let text = logical
                .to_str()
                .ok_or_else(|| invalid("repository tree path is not UTF-8"))?;
            self.names = self
                .names
                .checked_add(text.len() as u64)
                .filter(|size| *size <= self.capture.maximum_metadata)
                .ok_or_else(|| {
                    invalid("repository tree path inventory exceeds its metadata bound")
                })?;
            // Neither file_type nor target resolution runs for excluded entries
            // or irrelevant include branches. Exclusions are logical prefixes.
            if !self.selection.visits(text) {
                continue;
            }
            let resolved = self.capture.repository.canonicalize(physical.join(&name))?;
            let metadata = self.capture.repository.symlink_metadata(&resolved)?;
            if metadata.is_dir() {
                self.directory(&resolved, &logical, ancestors)?;
            } else if metadata.is_file() {
                if !self.selection.selects(text) {
                    continue;
                }
                if self.selected.len() == MAX_EXPANDED_FILES {
                    return Err(invalid(
                        "repository tree expands beyond the per-file declaration bound",
                    )
                    .into());
                }
                self.bytes = self
                    .bytes
                    .checked_add(metadata.len())
                    .filter(|size| *size <= self.capture.maximum_tree_bytes)
                    .ok_or_else(|| {
                        invalid("repository tree selected content exceeds its byte bound")
                    })?;
                let resolved = resolved
                    .to_str()
                    .ok_or_else(|| invalid("repository tree target is not UTF-8"))?;
                if self
                    .selected
                    .insert(text.to_owned(), resolved.to_owned())
                    .is_some()
                {
                    return Err(
                        invalid("repository tree has colliding logical destinations").into(),
                    );
                }
            } else {
                return Err(invalid("repository tree contains a special entry").into());
            }
        }
        ancestors.pop();
        Ok(())
    }
}
