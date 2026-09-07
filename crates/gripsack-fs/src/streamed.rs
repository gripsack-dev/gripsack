//! Streamed, mode-checked atomic publication. Errors retain the commit point.

use super::fault::{Boundary, operation};
use super::{Dir, create_dir_all, fsync_dir, parent_rel, remove_file, temp_name};
use std::fmt;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Debug)]
struct Published(io::Error);

impl fmt::Display for Published {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "publication occurred; durability is uncertain: {}",
            self.0
        )
    }
}

impl std::error::Error for Published {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Whether this primitive changed the destination before returning an error.
/// A true result must never trigger rollback of the published executable.
pub fn publication_occurred(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Published>())
}

/// Stream into a private sibling, then write → mode → fsync → rename → dir fsync.
/// Existing temporary names are never removed or reused. Only a temporary file
/// successfully created by this call is eligible for pre-publication cleanup.
pub fn atomic_copy_with_mode(
    dir: &Dir,
    name: &Path,
    source: &mut impl Read,
    mode: u32,
) -> io::Result<()> {
    let parent = parent_rel(name);
    create_dir_all(dir, parent)?;
    // Unlike the parent's create_temp, a collision here must not unlink a file
    // belonging to another writer or a previous process with a reused PID.
    let (tmp, mut file) = loop {
        let tmp = parent.join(temp_name("tmp-copy", name));
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        match dir.open_with(&tmp, &options) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let mut committed = false;
    let result = (|| {
        operation(Boundary::Write, name, || {
            io::copy(source, &mut file).map(|_| ())
        })?;
        operation(Boundary::Mode, name, || {
            file.set_permissions(cap_std::fs::Permissions::from_std(
                std::fs::Permissions::from_mode(mode),
            ))
        })?;
        operation(Boundary::FileSync, name, || file.sync_all())?;
        operation(Boundary::FilePublish, name, || {
            dir.rename(&tmp, dir, name)?;
            // Must precede the operation wrapper's After checkpoint.
            committed = true;
            Ok(())
        })?;
        fsync_dir(dir, parent)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(e) if committed => Err(io::Error::new(e.kind(), Published(e))),
        Err(e) => {
            drop(file);
            if let Err(cleanup) = remove_file(dir, &tmp) {
                return Err(io::Error::new(
                    e.kind(),
                    format!("{e}; cannot remove private temporary file {tmp:?}: {cleanup}"),
                ));
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fault::{self, Edge};
    use super::*;

    struct BrokenReader(bool);
    impl Read for BrokenReader {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if bytes.is_empty() {
                return Ok(0);
            }
            if self.0 {
                return Err(io::Error::other("source failed"));
            }
            self.0 = true;
            bytes[0] = b'x';
            Ok(1)
        }
    }

    #[test]
    fn failed_stream_keeps_old_destination_and_cleans_private_temp() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("grip"), b"old").unwrap();
        std::fs::set_permissions(
            root.path().join("grip"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let dir = super::super::open(root.path()).unwrap();
        let error = atomic_copy_with_mode(&dir, Path::new("grip"), &mut BrokenReader(false), 0o755)
            .unwrap_err();
        assert!(!publication_occurred(&error));
        assert_eq!(std::fs::read(root.path().join("grip")).unwrap(), b"old");
        assert_eq!(
            std::fs::metadata(root.path().join("grip"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn publishes_complete_bytes_and_exact_mode_in_real_operation_order() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("grip"), b"old").unwrap();
        let dir = super::super::open(root.path()).unwrap();
        let bytes = vec![42; 128 * 1024 + 17];
        let (result, events) = fault::capture(false, || {
            atomic_copy_with_mode(&dir, Path::new("grip"), &mut bytes.as_slice(), 0o755)
        });
        result.unwrap();
        assert_eq!(std::fs::read(root.path().join("grip")).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(root.path().join("grip"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        let order: Vec<_> = events
            .iter()
            .filter(|event| {
                event.edge == Edge::After
                    && matches!(
                        event.boundary,
                        Boundary::Write
                            | Boundary::Mode
                            | Boundary::FileSync
                            | Boundary::FilePublish
                            | Boundary::DirSync
                    )
            })
            .map(|event| event.boundary)
            .collect();
        assert_eq!(
            order,
            vec![
                Boundary::Write,
                Boundary::Mode,
                Boundary::FileSync,
                Boundary::FilePublish,
                Boundary::DirSync
            ]
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
