use crate::{FetchError, FetchLimits};
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

pub(crate) fn validate_tree(root: &Path, limits: FetchLimits) -> Result<(), FetchError> {
    let mut budget = Budget::new(limits);
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for child in std::fs::read_dir(directory)? {
            let child = child?;
            budget.entry()?;
            let kind = child.file_type()?;
            if kind.is_dir() {
                pending.push(child.path());
            } else if kind.is_file() {
                budget.bytes(child.metadata()?.len())?;
            } else if !kind.is_symlink() {
                return Err(super::paths::violation(
                    &child.path(),
                    "is not a regular file, directory or link",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn copy_tree_filtered(
    from: &Path,
    to: &Path,
    skip: &[&str],
    limits: FetchLimits,
) -> Result<(), FetchError> {
    std::fs::create_dir_all(to)?;
    if to.canonicalize()?.starts_with(from.canonicalize()?) {
        return Err(super::paths::violation(
            to,
            "source contains its destination",
        ));
    }
    fn copy(
        root: &Path,
        destination: &Path,
        relative: &Path,
        skip: &[&str],
        budget: &mut Budget,
    ) -> Result<(), FetchError> {
        std::fs::create_dir_all(destination.join(relative))?;
        for child in std::fs::read_dir(root.join(relative))? {
            let child = child?;
            if relative.as_os_str().is_empty() && skip.iter().any(|name| child.file_name() == *name)
            {
                continue;
            }
            budget.entry()?;
            let rel = relative.join(child.file_name());
            let target = super::paths::destination(destination, &rel)?;
            let kind = child.file_type()?;
            if kind.is_dir() {
                copy(root, destination, &rel, skip, budget)?;
            } else if kind.is_symlink() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(std::fs::read_link(child.path())?, target)?;
                #[cfg(not(unix))]
                return Err(super::paths::violation(&rel, "symlinks require Unix"));
            } else if kind.is_file() {
                let input = super::super::tarball::regular_file(&child.path())?;
                let mode = input.metadata()?.permissions();
                let output = std::fs::File::create(&target)?;
                budget.copy(input, output, |_| {})?;
                std::fs::set_permissions(target, mode)?;
            } else {
                return Err(super::paths::violation(
                    &rel,
                    "special payload entries are unsupported",
                ));
            }
        }
        Ok(())
    }
    copy(from, to, Path::new(""), skip, &mut Budget::new(limits))
}
