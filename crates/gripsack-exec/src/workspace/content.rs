//! Capture each declared repository source once; render candidates independently
//! of their destination policy. Only immutable candidates enter the store.
use crate::ctx::ExecError;
use gripsack_ir::workspace::{
    WorkspaceContent, WorkspaceDestination, WorkspaceFile, WorkspaceSource,
};
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

struct CapturedFile {
    path: PathBuf,
    hash: store::hash::PayloadHash,
    executable: bool,
}

pub(super) struct Capture<'a> {
    pub temporary: tempfile::TempDir,
    repository: gripsack_fs::Dir,
    sources: BTreeMap<&'a str, CapturedFile>,
    maximum_bytes: u64,
}

impl<'a> Capture<'a> {
    pub fn new(repo: &Path, limits: gripsack_fetch::FetchLimits) -> Result<Self, ExecError> {
        Ok(Self {
            temporary: tempfile::Builder::new()
                .prefix("grip-workspace-")
                .tempdir()?,
            repository: gripsack_fs::open(repo)?,
            sources: BTreeMap::new(),
            maximum_bytes: limits.download_bytes.get(),
        })
    }

    fn file(&mut self, path: &'a str) -> Result<&CapturedFile, ExecError> {
        let index = self.sources.len();
        match self.sources.entry(path) {
            std::collections::btree_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let mut options = gripsack_fs::cap_std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use gripsack_fs::cap_std::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                let source = self.repository.open_with(path, &options)?;
                let metadata = source.metadata()?;
                if !metadata.is_file() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "a repository file must be regular, not a link, directory or special object").into());
                }
                let destination = self.temporary.path().join(format!("source-{index}"));
                let mut target = std::fs::File::create(&destination)?;
                let bound = self.maximum_bytes.checked_add(1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "file capture bound is too large",
                    )
                })?;
                let copied = io::copy(&mut io::Read::take(source, bound), &mut target)?;
                if copied > self.maximum_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "repository file exceeds max_download_bytes capture policy",
                    )
                    .into());
                }
                #[cfg(unix)]
                let executable = {
                    use gripsack_fs::cap_std::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                set_private_mode(&target, executable)?;
                drop(target);
                let hash = store::canonical_file_hash(&destination)?;
                Ok(entry.insert(CapturedFile {
                    path: destination,
                    hash,
                    executable,
                }))
            }
        }
    }
}

pub(super) struct PreparedFile {
    pub entry: Entry,
    pub block_id: Option<store::ManagedBlockId>,
}

#[derive(Serialize)]
struct ContentRecipe<'a> {
    format: &'static str,
    source: Option<&'a WorkspaceSource>,
    source_hash: Option<&'a str>,
    content: &'a WorkspaceContent,
    executable: bool,
}

pub(super) fn prepare_file<'a>(
    capture: &mut Capture<'a>,
    file: &'a WorkspaceFile,
    root: &Path,
) -> Result<PreparedFile, ExecError> {
    let source = match file.source.as_ref() {
        Some(WorkspaceSource::RepoFile { path }) => Some(capture.file(path)?),
        Some(WorkspaceSource::ArtifactFile { .. }) => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "artifact file needs native package realization",
            )
            .into());
        }
        None => None,
    };
    let executable = source.is_some_and(|source| source.executable);
    let recipe = serde_json::to_vec(&ContentRecipe {
        format: "gripsack-file-content-v1",
        source: file.source.as_ref(),
        source_hash: source.map(|source| source.hash.as_str()),
        content: &file.content,
        executable,
    })?;
    let identity = store::hash::hex_sha256(&recipe);
    let directory = root.join(&identity);
    if !directory.exists() {
        std::fs::create_dir(&directory)?;
        if let Some(source) = source {
            std::fs::hard_link(&source.path, directory.join("source"))?;
        }
        let output = directory.join("output");
        match &file.content {
            WorkspaceContent::Identity => {
                let source = source.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "identity content requires a captured source",
                    )
                })?;
                std::fs::hard_link(&source.path, &output)?;
            }
            WorkspaceContent::Literal { text } => {
                write_private(&output, text.as_bytes(), executable)?
            }
            WorkspaceContent::Template {
                template,
                variables,
            } => {
                if source.is_none() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "template content requires a captured source",
                    )
                    .into());
                }
                let rendered = crate::template::render_template(
                    template.as_bytes(),
                    variables,
                    &file.span.file,
                )?;
                write_private(&output, &rendered, executable)?;
            }
        }
        write_private(&directory.join("recipe.json"), &recipe, false)?;
    }
    let (destination, mode, block_id) = match &file.destination {
        WorkspaceDestination::Symlink { path } => (path, Ownership::Owned, None),
        WorkspaceDestination::TrackedCopy { path } => (path, Ownership::TrackedCopy, None),
        WorkspaceDestination::ManagedBlock { path, marker } => {
            let payload = std::fs::read_to_string(directory.join("output"))?;
            crate::managed_blocks::validate_payload(&payload).map_err(io::Error::other)?;
            (
                path,
                Ownership::Merge,
                Some(store::ManagedBlockId::from_marker(marker)?),
            )
        }
    };
    Ok(PreparedFile {
        entry: Entry {
            from: format!("{identity}/output"),
            to: destination.clone(),
            mode,
            vars: Default::default(),
            marker: None,
            span: Some(file.span.clone()),
        },
        block_id,
    })
}

fn write_private(path: &Path, bytes: &[u8], executable: bool) -> io::Result<()> {
    let mut file = std::fs::File::create_new(path)?;
    set_private_mode(&file, executable)?;
    file.write_all(bytes)
}

fn set_private_mode(file: &std::fs::File, executable: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(if executable {
            0o700
        } else {
            0o600
        }))?;
    }
    #[cfg(not(unix))]
    let _ = (file, executable);
    Ok(())
}
