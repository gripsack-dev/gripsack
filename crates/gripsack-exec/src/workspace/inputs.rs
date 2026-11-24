//! Captured input materialization uses the same segment-prefix selection as
//! treeFiles: include named paths/subtrees, then apply exclusion precedence.
//! No wildcard engine or mutable-checkout fallback is introduced here.
use crate::source_bundle::{SourceBundle, SourceObject};
use crate::{Ctx, ExecError, LifecycleSession, Repository};
use gripsack_host::store;
use gripsack_ir::{
    Diagnostic, codes,
    workspace_model::{InputOrigin, WorkspaceCatalog, WorkspaceInput, identity::ArtifactDigest},
};
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
impl InputKind {
    pub(super) fn identity(self, tree: &store::hash::PayloadHash) -> ArtifactDigest {
        let mut identity = Sha256::new();
        identity.update(match self {
            Self::File => b"gripsack/input-file/v1\0".as_slice(),
            Self::Directory => b"gripsack/input-directory/v1\0".as_slice(),
        });
        identity.update(tree.as_str().as_bytes());
        ArtifactDigest::from_bytes(identity.finalize().into())
    }
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

/// Check the inputs used by workspace commands without staging bytes or
/// consulting the store. Unused declarations do not require materialization.
pub fn check_command_inputs(
    workspace: &WorkspaceCatalog,
    sources: &SourceBundle,
) -> Vec<Diagnostic> {
    if workspace.inputs.is_empty() {
        return Vec::new();
    }
    let mut needed = BTreeSet::new();
    for output in &workspace.outputs {
        super::prepare::collect_inputs(output, &mut needed);
    }
    workspace
        .inputs
        .iter()
        .filter(|input| needed.contains(input.name.as_str()))
        .filter_map(|input| captured_input_path(sources, input).err())
        .collect()
}

/// Share the capture-time resolution and kind check with read-only preflight.
fn captured_input_path(
    sources: &SourceBundle,
    declaration: &WorkspaceInput,
) -> Result<PathBuf, Diagnostic> {
    let (path, kind) = match &declaration.origin {
        InputOrigin::RepoFile { path } => (path, InputKind::File),
        InputOrigin::RepoDirectory { path, .. } => (path, InputKind::Directory),
    };
    let failure = |code, detail: &str| {
        Diagnostic::error(
            code,
            format!("input {:?} at {path:?}: {detail}", declaration.name),
        )
        .with_label(
            Some(declaration.span.clone()),
            "captured input declared here",
        )
    };
    let unavailable = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            failure(codes::MISSING_SOURCE, "no such captured repository source")
        }
        _ => failure(codes::EXEC_STEP, &error.to_string()),
    };
    let source = sources
        .materialization_path(Path::new(path))
        .map_err(unavailable)?;
    let metadata = std::fs::symlink_metadata(&source).map_err(unavailable)?;
    let detail = match kind {
        InputKind::File if !metadata.is_file() => "inputFile must select a captured regular file",
        InputKind::Directory if !metadata.is_dir() => {
            "inputDirectory must select a captured directory"
        }
        _ => return Ok(source),
    };
    Err(failure(codes::EXEC_STEP, detail))
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
    let source = captured_input_path(sources, declaration).map_err(ExecError::Gate)?;
    let temporary = tempfile::Builder::new()
        .prefix("grip-workspace-input-")
        .tempdir()?;
    let stage = temporary.path().join("payload");
    std::fs::create_dir(&stage)?;
    let kind = match &declaration.origin {
        InputOrigin::RepoFile { .. } => {
            std::fs::hard_link(&source, stage.join("value"))?;
            InputKind::File
        }
        InputOrigin::RepoDirectory {
            path,
            include,
            exclude,
        } => {
            let root = Path::new(path);
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
    Ok((
        temporary,
        CapturedInput {
            identity: kind.identity(checked.tree_hash()),
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
