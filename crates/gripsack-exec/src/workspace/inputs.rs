//! Captured input materialization uses the same segment-prefix selection as
//! treeFiles: include named paths/subtrees, then apply exclusion precedence.
//! No wildcard engine or mutable-checkout fallback is introduced here.
use crate::{Ctx, ExecError, LifecycleSession, Repository};
use gripsack_ir::{
    Diagnostic, codes,
    workspace_model::{InputOrigin, WorkspaceInput, identity::ArtifactDigest},
};
use gripsack_store::{self as store, source_bundle::SourceObject};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy)]
pub(super) enum InputKind {
    File,
    Directory,
}
pub(super) struct CapturedInput {
    pub artifact: Arc<super::artifact::Artifact>,
    pub identity: ArtifactDigest,
    pub kind: InputKind,
}
impl CapturedInput {
    pub fn binding_path(&self) -> PathBuf {
        match self.kind {
            InputKind::File => self.artifact.payload.join("value"),
            InputKind::Directory => self.artifact.payload.clone(),
        }
    }
}

pub(super) fn capture(
    ctx: &Ctx,
    session: &LifecycleSession,
    declaration: &WorkspaceInput,
) -> Result<CapturedInput, ExecError> {
    if session.home() != ctx.home {
        return Err(std::io::Error::other("input publication belongs to another home").into());
    }
    let (_temporary, mut captured) =
        capture_readonly(&ctx.repository, &ctx.home, declaration, ctx.fetch.limits())?;
    if !captured.artifact.root.exists() {
        crate::source::publish(
            ctx,
            &declaration.name,
            &captured.artifact.payload,
            &captured.artifact.root,
        )?;
    }
    let artifact = Arc::get_mut(&mut captured.artifact).expect("newly captured input is private");
    artifact.payload = artifact.root.clone();
    Ok(captured)
}

/// Private captured bytes and identity only; never creates a store or lock.
pub(super) fn capture_readonly(
    repository: &Repository,
    home: &Path,
    declaration: &WorkspaceInput,
    limits: gripsack_fetch::FetchLimits,
) -> Result<(tempfile::TempDir, CapturedInput), ExecError> {
    let failure = |message: String| {
        ExecError::Gate(Diagnostic::error(codes::EXEC_STEP, message).with_label(
            Some(declaration.span.clone()),
            "captured input declared here",
        ))
    };
    let Repository::Evaluated { sources, .. } = repository else {
        return Err(failure(
            "captured inputs require an approved source bundle".into(),
        ));
    };
    let temporary = tempfile::Builder::new()
        .prefix("grip-workspace-input-")
        .tempdir()?;
    let stage = temporary.path().join("payload");
    std::fs::create_dir(&stage)?;
    let kind = match &declaration.origin {
        InputOrigin::RepoFile { path } => {
            let source = sources.materialization_path(Path::new(path))?;
            if !std::fs::symlink_metadata(&source)?.is_file() {
                return Err(failure(
                    "inputFile must select a captured regular file".into(),
                ));
            }
            std::fs::hard_link(&source, stage.join("value"))?;
            InputKind::File
        }
        InputOrigin::RepoDirectory {
            path,
            include,
            exclude,
        } => {
            let root = Path::new(path);
            if !std::fs::symlink_metadata(sources.materialization_path(root)?)?.is_dir() {
                return Err(failure(
                    "inputDirectory must select a captured directory".into(),
                ));
            }
            let relative_root = if path == "." { Path::new("") } else { root };
            sources.visit_materialized([path.as_str()], |relative, physical, object| {
                let relative = relative
                    .strip_prefix(relative_root)
                    .map_err(std::io::Error::other)?;
                if relative.as_os_str().is_empty() {
                    return Ok(());
                }
                let text = relative.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "captured selection is not UTF-8",
                    )
                })?;
                let covered = |prefix: &String| {
                    text.strip_prefix(prefix.as_str())
                        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                };
                if !include.iter().any(covered) || exclude.iter().any(covered) {
                    return Ok(());
                }
                let destination = stage.join(relative);
                match object {
                    SourceObject::Directory => std::fs::create_dir_all(destination),
                    SourceObject::File { .. } => {
                        if let Some(parent) = destination.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::hard_link(physical, destination)
                    }
                    SourceObject::Alias { .. } => Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "input alias was not resolved by capture",
                    )),
                }
            })?;
            InputKind::Directory
        }
    };
    let checked = super::stage::validate_output_tree(
        &stage,
        gripsack_ir::workspace::RecipeOutputKind::Tree,
        &declaration.name,
        limits,
    )?;
    let root = store::content_path(home, "workspace-input", checked.tree_hash().as_str());
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {
            let retained = super::stage::validate_output_tree(
                &root,
                gripsack_ir::workspace::RecipeOutputKind::Tree,
                &declaration.name,
                limits,
            )?;
            if retained.tree_hash() != checked.tree_hash() {
                return Err(failure("retained captured-input bytes changed".into()));
            }
        }
        Ok(_) => {
            return Err(failure(
                "captured-input store root is not a real directory".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut identity = Sha256::new();
    identity.update(match kind {
        InputKind::File => b"gripsack/input-file/v1\0".as_slice(),
        InputKind::Directory => b"gripsack/input-directory/v1\0".as_slice(),
    });
    identity.update(checked.tree_hash().as_str().as_bytes());
    Ok((
        temporary,
        CapturedInput {
            identity: ArtifactDigest::from_bytes(identity.finalize().into()),
            kind,
            artifact: Arc::new(super::artifact::Artifact {
                payload: stage,
                retention: BTreeSet::from([root.clone()]),
                root,
                tree: checked.tree_hash().clone(),
            }),
        },
    ))
}
