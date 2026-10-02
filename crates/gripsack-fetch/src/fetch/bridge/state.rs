use crate::FetchError;
use gripsack_fs::cap_std::fs::{MetadataExt, PermissionsExt};
use gripsack_fs::{Dir, FlockGuard};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// One pinned version-directory inode and its provisioning lock. No cache
/// mutation follows ambient ancestor paths or sweeps filenames as ownership.
pub(super) struct ToolSlot {
    pub directory: Dir,
    pub executable: PathBuf,
    pub name: &'static str,
    _lock: FlockGuard,
}
impl ToolSlot {
    pub fn open(
        home: &Path,
        domain: super::Domain,
        version: &str,
        deadline: Instant,
    ) -> Result<Self, FetchError> {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper provisioning deadline expired",
            )
            .into());
        }
        let root = gripsack_fs::open_or_create(home)?;
        owned_directory(&root)?;
        let home = home.canonicalize()?;
        let locks = child(&root, "locks")?;
        let lock = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "helper provisioning lock deadline expired",
                )
                .into());
            }
            if let Some(lock) = FlockGuard::try_acquire_in(&locks, domain.namespace())? {
                break lock;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(25)));
        };
        let tools = child(&root, "tools")?;
        let name = format!("{}-{version}", domain.namespace());
        let directory = child(&tools, &name)?;
        Ok(Self {
            directory,
            name: domain.executable(),
            executable: home.join("tools").join(name).join(domain.executable()),
            _lock: lock,
        })
    }
}
fn child(parent: &Dir, name: &str) -> Result<Dir, FetchError> {
    gripsack_fs::create_dir_all(parent, Path::new(name))?;
    let child = gripsack_fs::open_dir_nofollow(parent, Path::new(name))?;
    owned_directory(&child)?;
    Ok(child)
}

fn owned_directory(directory: &Dir) -> Result<(), FetchError> {
    let metadata = directory.metadata(".")?;
    // SAFETY: geteuid has no preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o022 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "helper namespace must be owned and not writable by another user",
        )
        .into());
    }
    Ok(())
}
