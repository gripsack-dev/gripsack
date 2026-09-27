//! Private authoritative metadata: pin real directories/files before chmod or
//! IO, and make permission changes durable through the existing fault boundary.
use gripsack_fs::{
    Dir, cap_std,
    fault::{Boundary, operation},
};
use std::{io, path::Path};

pub(crate) fn ensure_directory(parent: &Dir, name: &Path) -> io::Result<Dir> {
    gripsack_fs::create_dir_all(parent, name)?;
    let directory = gripsack_fs::open_dir_nofollow(parent, name)?;
    restrict_directory(&directory, name)?;
    Ok(directory)
}

pub(crate) fn restrict_directory(directory: &Dir, label: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if directory.metadata(".")?.permissions().mode() & 0o7777 != 0o700 {
            operation(Boundary::Mode, label, || {
                directory.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
            })?;
            gripsack_fs::fsync_dir(directory, Path::new("."))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (directory, label);
    Ok(())
}

pub(crate) fn restrict_file(file: &cap_std::fs::File, label: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o7777 != 0o600 {
            operation(Boundary::Mode, label, || {
                file.set_permissions(cap_std::fs::Permissions::from_mode(0o600))
            })?;
            operation(Boundary::FileSync, label, || file.sync_all())?;
        }
    }
    #[cfg(not(unix))]
    let _ = (file, label);
    Ok(())
}
