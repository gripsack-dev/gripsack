//! Independent admission of a completed private staging tree. The backend's
//! completion message does not construct this capability or publish an object.
use crate::ExecError;
use gripsack_ir::workspace::RecipeOutputKind;
use gripsack_store as store;
use std::path::{Path, PathBuf};

pub struct ValidatedTree {
    root: PathBuf,
    tree: store::hash::PayloadHash,
}
impl ValidatedTree {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn tree_hash(&self) -> &store::hash::PayloadHash {
        &self.tree
    }
}

pub fn validate_output_tree(
    root: &Path,
    kind: RecipeOutputKind,
    name: &str,
    limits: gripsack_fetch::FetchLimits,
) -> Result<ValidatedTree, ExecError> {
    let failure = |detail: String| ExecError::Step {
        module: name.into(),
        step: "staging".into(),
        detail,
    };
    gripsack_fetch::fetch::validate_tree(root, limits)
        .map_err(|error| failure(format!("staged output failed payload admission: {error}")))?;
    if kind == RecipeOutputKind::File {
        let mut found = false;
        find_single_file(root, root, &mut found)?;
        if !found {
            return Err(failure("declared file output is empty".into()));
        }
    }
    Ok(ValidatedTree {
        root: root.to_owned(),
        tree: store::canonical_tree_hash(root)?,
    })
}

fn find_single_file(root: &Path, directory: &Path, found: &mut bool) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            find_single_file(root, &path, found)?;
        } else if kind.is_file() && !*found {
            let relative = path
                .strip_prefix(root)
                .expect("walk stays under private staging");
            if relative.to_str().is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "file output path is not UTF-8",
                ));
            }
            *found = true;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "declared file output must contain exactly one regular file and no symlinks",
            ));
        }
    }
    Ok(())
}
