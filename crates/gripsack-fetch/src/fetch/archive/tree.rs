use super::{
    links::{Graph, Kind},
    paths,
};
use crate::{FetchError, FetchLimits};
use gripsack_fs::cap_std::fs::{Dir, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

pub(super) struct Budget {
    limits: FetchLimits,
    entries: usize,
    bytes: u64,
}
impl Budget {
    pub(super) fn new(limits: FetchLimits) -> Self {
        Self {
            limits,
            entries: 0,
            bytes: 0,
        }
    }
    pub(super) fn entry(&mut self) -> Result<(), FetchError> {
        if self.entries == self.limits.archive_entries.get() {
            return Err(FetchError::TooManyEntries {
                limit: self.limits.archive_entries.get(),
            });
        }
        self.entries += 1;
        Ok(())
    }
    pub(super) fn bytes(&mut self, bytes: u64) -> Result<(), FetchError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|sum| *sum <= self.limits.expanded_bytes.get())
            .ok_or_else(|| FetchError::PayloadTooLarge {
                what: "expanded payload".into(),
                limit: self.limits.expanded_bytes.get(),
            })?;
        Ok(())
    }
    pub(super) fn copy(
        &mut self,
        mut input: impl Read,
        mut output: impl Write,
        mut observe: impl FnMut(&[u8]),
    ) -> Result<u64, FetchError> {
        let mut buffer = [0; 64 * 1024];
        let mut copied = 0u64;
        loop {
            let read = match input.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if read == 0 {
                break;
            }
            self.bytes(read as u64)?;
            copied += read as u64;
            observe(&buffer[..read]);
            output.write_all(&buffer[..read])?;
        }
        Ok(copied)
    }
}

fn admitted(root: &Dir, skip: &[&str], limits: FetchLimits) -> Result<Graph, FetchError> {
    let mut graph = Graph::new(limits);
    let mut budget = Budget::new(limits);
    let mut pending = vec![std::path::PathBuf::new()];
    while let Some(directory) = pending.pop() {
        let current = if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory.as_path()
        };
        for child in root.read_dir(current)? {
            let child = child?;
            if directory.as_os_str().is_empty()
                && skip.iter().any(|name| child.file_name() == *name)
            {
                continue;
            }
            budget.entry()?;
            let relative = directory.join(child.file_name());
            if relative.as_os_str().len() > 4096 {
                return Err(paths::violation(&relative, "name is too long"));
            }
            let kind = child.file_type()?;
            let node = if kind.is_dir() {
                pending.push(relative.clone());
                Kind::Directory { explicit: true }
            } else if kind.is_file() {
                let meta = root.symlink_metadata(&relative)?;
                if !meta.is_file() {
                    return Err(paths::violation(
                        &relative,
                        "payload file changed during admission",
                    ));
                }
                budget.bytes(meta.len())?;
                Kind::Regular
            } else if kind.is_symlink() {
                #[cfg(unix)]
                {
                    Kind::Symlink(root.read_link_contents(&relative)?)
                }
                #[cfg(not(unix))]
                return Err(paths::violation(&relative, "symlinks require Unix"));
            } else {
                return Err(paths::violation(
                    &relative,
                    "special payload entries are unsupported",
                ));
            };
            graph.insert(relative, node)?;
        }
    }
    graph.validate()?;
    Ok(graph)
}

pub(crate) fn validate_tree(root: &Path, limits: FetchLimits) -> Result<(), FetchError> {
    let source = paths::open_existing_root(root)?;
    admitted(&source, &[], limits)?;
    Ok(())
}

pub(crate) fn copy_tree_filtered(
    from: &Path,
    to: &Path,
    skip: &[&str],
    limits: FetchLimits,
) -> Result<(), FetchError> {
    let source = paths::open_existing_root(from)?;
    let graph = admitted(&source, skip, limits)?;
    let to_parent = to.parent().unwrap_or(Path::new(".")).canonicalize()?;
    let to_name = to
        .file_name()
        .ok_or_else(|| paths::violation(to, "destination has no directory name"))?;
    if to_parent.join(to_name).starts_with(from.canonicalize()?) {
        return Err(paths::violation(to, "source contains its destination"));
    }
    let destination = paths::open_root(to)?;
    let mut budget = Budget::new(limits);
    for (relative, kind) in graph.entries() {
        match kind {
            Kind::Directory { .. } => paths::directories(&destination, relative)?,
            Kind::Regular => {
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use gripsack_fs::cap_std::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
                }
                let mut input = source.open_with(relative, &options)?;
                let metadata = input.metadata()?;
                if !metadata.is_file() {
                    return Err(paths::violation(
                        relative,
                        "payload file changed during copy",
                    ));
                }
                let mut output = paths::file(&destination, relative)?;
                budget.copy(&mut input, &mut output, |_| {})?;
                output.set_permissions(metadata.permissions())?;
            }
            Kind::Symlink(_) => {}
            Kind::HardLink(_) => unreachable!("filesystem trees enumerate hard links as files"),
        }
    }
    #[cfg(unix)]
    for (relative, kind) in graph.entries() {
        if let Kind::Symlink(target) = kind {
            paths::symlink(&destination, relative, target)?;
        }
    }
    Ok(())
}
