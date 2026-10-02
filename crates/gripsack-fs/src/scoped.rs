//! Pin a real child object without following its final symlink. Path
//! containment remains cap-std's responsibility; raw syscall flags live here.
use crate::{Dir, cap_std};
use std::{io, path::Path};

/// Open an existing directory relative to a capability, refusing a final
/// symlink on supported Unix hosts. This does not create or chmod anything.
pub fn open_dir_nofollow(parent: &Dir, name: &Path) -> io::Result<Dir> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a directory",
        ));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

/// Open a regular file without following its final symlink. Nonblocking open
/// lets special-file admission reject a FIFO instead of waiting for a writer.
pub fn open_file_nofollow(directory: &Dir, name: &Path) -> io::Result<cap_std::fs::File> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = directory.open_with(name, &options)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a regular file",
        ));
    }
    Ok(file)
}
