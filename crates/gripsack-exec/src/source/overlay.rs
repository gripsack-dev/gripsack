//! Capture selected repo bytes once, hash that snapshot, then move it into the
//! fetched tree. Fetched symlinks can never redirect a repo-overlay write.

use crate::ctx::ExecError;
use gripsack_ir::prepared::PreparedModule;
use std::io;
use std::path::Path;

#[cfg(test)]
mod tests;

struct Snapshot {
    root: gripsack_fs::Dir,
    temporary: tempfile::TempDir,
}

pub(crate) struct Overlay {
    directory: Option<Snapshot>,
    hash: Option<String>,
}

impl Overlay {
    pub(crate) fn capture(
        plan: &PreparedModule,
        repo: &Path,
        stage: &Path,
    ) -> Result<Self, ExecError> {
        let mut directory = None;
        for entry in plan.entries() {
            let source = repo.join(&entry.from);
            match source.symlink_metadata() {
                Ok(metadata) => {
                    if metadata.is_dir()
                        && stage.canonicalize()?.starts_with(source.canonicalize()?)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "overlay source contains its staging directory",
                        )
                        .into());
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            if directory.is_none() {
                let temporary = tempfile::Builder::new()
                    .prefix("grip-overlay-")
                    .tempdir_in(stage.parent().unwrap_or(Path::new(".")))?;
                let root = gripsack_fs::open(temporary.path())?;
                directory = Some(Snapshot { root, temporary });
            }
            copy(
                &source,
                &directory.as_ref().expect("snapshot created").root,
                Path::new(&entry.from),
            )?;
        }
        let hash = directory
            .as_ref()
            .map(|directory| {
                gripsack_store::canonical_tree_hash(directory.temporary.path()).map(String::from)
            })
            .transpose()?;
        Ok(Self { directory, hash })
    }

    pub(crate) fn into_hash(self) -> Option<String> {
        self.hash
    }

    pub(crate) fn merge(self, stage: &Path) -> Result<Option<String>, ExecError> {
        std::fs::create_dir_all(stage)?;
        if let Some(directory) = &self.directory {
            let destination = gripsack_fs::open(stage)?;
            move_entries(&directory.root, &destination, Path::new(""))?;
        }
        Ok(self.hash)
    }
}

fn real_directories(root: &gripsack_fs::Dir, relative: &Path) -> io::Result<()> {
    let mut path = std::path::PathBuf::new();
    for component in relative.components() {
        path.push(component);
        match root.symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "overlay ancestor {} is not a real directory",
                        path.display()
                    ),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => root.create_dir(&path)?,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn copy(source: &Path, destination: &gripsack_fs::Dir, relative: &Path) -> io::Result<()> {
    let metadata = source.symlink_metadata()?;
    if metadata.is_dir() {
        real_directories(destination, relative)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy(
                &entry.path(),
                destination,
                &relative.join(entry.file_name()),
            )?;
        }
        return Ok(());
    }
    if let Some(parent) = relative.parent() {
        real_directories(destination, parent)?;
    }
    match destination.remove_file(relative) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if metadata.is_symlink() {
        return gripsack_fs::symlink_replace(destination, relative, &std::fs::read_link(source)?);
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "repo overlay contains a special file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let mut input = options.open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "repo source is no longer a regular file",
        ));
    }
    let mut options = gripsack_fs::cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut output = destination.open_with(relative, &options)?;
    io::copy(&mut input, &mut output)?;
    output.set_permissions(gripsack_fs::cap_std::fs::Permissions::from_std(
        metadata.permissions(),
    ))
}

fn move_entries(
    source: &gripsack_fs::Dir,
    destination: &gripsack_fs::Dir,
    relative: &Path,
) -> io::Result<()> {
    let directory = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    for entry in source.read_dir(directory)? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            real_directories(destination, &path)?;
            move_entries(source, destination, &path)?;
        } else {
            // rename replaces a leaf symlink rather than writing through it.
            source.rename(&path, destination, &path)?;
        }
    }
    Ok(())
}
