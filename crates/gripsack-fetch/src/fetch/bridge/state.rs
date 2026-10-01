use crate::FetchError;
use gripsack_fs::{Dir, FlockGuard};
use gripsack_fs::cap_std::fs::MetadataExt;
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};

/// One pinned version-directory inode and its provisioning lock. No cache
/// mutation follows ambient ancestor paths or sweeps filenames as ownership.
pub(super) struct ToolSlot {
    pub directory: Dir,
    pub executable: PathBuf,
    _lock: FlockGuard,
}
impl ToolSlot {
    pub fn open(home: &Path, version: &str, deadline: Instant) -> Result<Self, FetchError> {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "bridge provisioning deadline expired").into());
        }
        let root = gripsack_fs::open_or_create(home)?;
        let home = home.canonicalize()?;
        let locks = child(&root, "locks")?;
        let lock = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"bridge provisioning lock deadline expired").into()); }
            if let Some(lock) = FlockGuard::try_acquire_in(&locks, "buildkit-bridge")? { break lock; }
            std::thread::sleep(remaining.min(Duration::from_millis(25)));
        };
        let tools = child(&root, "tools")?;
        let name = format!("buildkit-bridge-{version}");
        let directory = child(&tools, &name)?;
        Ok(Self { directory, executable:home.join("tools").join(name).join("bridge"), _lock:lock })
    }
}
fn child(parent: &Dir, name: &str) -> Result<Dir, FetchError> {
    gripsack_fs::create_dir_all(parent, Path::new(name))?;
    let child = gripsack_fs::open_dir_nofollow(parent, Path::new(name))?;
    // SAFETY: geteuid has no preconditions.
    if child.metadata(".")?.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,"bridge tool namespace belongs to another user").into());
    }
    Ok(child)
}
