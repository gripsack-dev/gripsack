use super::{WorkerError, WorkerProfile, manager::MINIMUM_MEMORY_BYTES, observation, provider};
use crate::identity::{FenceEpoch, WorkerHomeId, WorkerOwnerId};
use gripsack_fs::cap_std::fs::{DirBuilderExt as _, MetadataExt as _};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    num::{NonZeroU16, NonZeroU64},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) const RECORD_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const LOCK_WAIT_INTERVAL: Duration = Duration::from_millis(25);
const RECORD: &str = "instance.json";
const PREVIOUS_RECORD: &str = "previous-instance.json";
/// Disposable child of the runtime namespace holding the daemon socket.
const RUN_CHILD: &str = "run";
const SOCKET_NAME: &str = "buildkitd.sock";
/// Durable owner-nonce marker: the namespace's ownership evidence.
const OWNER_MARKER: &str = "owner";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecordState {
    Provisioning,
    Starting,
    Ready,
    Stopping,
    Stopped,
    Quarantined,
}
impl RecordState {
    pub(super) fn phase(self) -> super::WorkerPhase {
        match self {
            Self::Provisioning | Self::Starting => super::WorkerPhase::Provisioning,
            Self::Ready => super::WorkerPhase::Ready,
            Self::Stopping => super::WorkerPhase::Stopping,
            Self::Stopped => super::WorkerPhase::Stopped,
            Self::Quarantined => super::WorkerPhase::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstanceRecord {
    pub version: u32,
    pub home_identity: WorkerHomeId,
    pub owner: WorkerOwnerId,
    pub profile: String,
    /// Durable image of the recorded container instance. This is the
    /// instance's own binding, NOT the current pin: a pin change must not
    /// corrupt the record of a still-owned previous instance.
    pub image: String,
    /// Resource binding of the recorded container (`--cpus`/`--memory`).
    /// `None` only on records written before resources were durable; the
    /// next verified observation re-binds them. Never silently defaulted.
    #[serde(default)]
    pub cpus: Option<NonZeroU16>,
    #[serde(default)]
    pub memory: Option<u64>,
    pub state: RecordState,
    pub container_id: Option<observation::ContainerId>,
    pub epoch: Option<FenceEpoch>,
    pub next_epoch: NonZeroU64,
    pub next_lease: NonZeroU64,
}
impl InstanceRecord {
    pub(super) fn fresh(home: &WorkerHome) -> Result<Self, WorkerError> {
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
        Ok(Self {
            version: RECORD_VERSION,
            home_identity: home.identity,
            owner: WorkerOwnerId::from_bytes(nonce),
            profile: home.profile.as_str().to_owned(),
            image: provider::PINNED_WORKER_IMAGE.to_owned(),
            cpus: None,
            memory: None,
            state: RecordState::Stopped,
            container_id: None,
            epoch: None,
            next_epoch: NonZeroU64::MIN,
            next_lease: NonZeroU64::MIN,
        })
    }
    fn validate(&self, home: &WorkerHome) -> Result<(), WorkerError> {
        if self.version != RECORD_VERSION
            || self.home_identity != home.identity
            || self.profile != home.profile.as_str()
        {
            return Err(WorkerError::Corrupt(
                "worker version/home/profile binding changed",
            ));
        }
        // The recorded image is the instance's own binding; a current-pin
        // change is a replacement decision, never record corruption.
        if self.image.is_empty() {
            return Err(WorkerError::Corrupt("worker image binding is empty"));
        }
        match (self.cpus, self.memory) {
            (Some(_), Some(memory)) if memory >= MINIMUM_MEMORY_BYTES => {}
            (None, None) => {}
            _ => {
                return Err(WorkerError::Corrupt(
                    "worker resource binding is partial or below the Docker minimum",
                ));
            }
        }
        if self.state == RecordState::Ready && (self.container_id.is_none() || self.epoch.is_none())
        {
            return Err(WorkerError::Corrupt(
                "ready worker lacks its container/epoch identity",
            ));
        }
        if self
            .epoch
            .is_some_and(|epoch| epoch.get() >= self.next_epoch.get())
        {
            return Err(WorkerError::Corrupt(
                "worker epoch is not below its durable next-epoch fence",
            ));
        }
        Ok(())
    }
}

pub(super) struct WorkerHome {
    pub root: PathBuf,
    pub dir: gripsack_fs::Dir,
    pub profile: WorkerProfile,
    pub identity: WorkerHomeId,
    pub resource_name: String,
    pub run: PathBuf,
    pub leases: gripsack_fs::Dir,
}
impl WorkerHome {
    pub(super) fn open(home: &Path, profile: WorkerProfile) -> Result<Self, WorkerError> {
        if !home.is_absolute() {
            return Err(WorkerError::Invalid("worker home must be absolute"));
        }
        let home_dir = gripsack_fs::open_or_create(home)?;
        let home = home.canonicalize()?;
        let root = home.join("buildkit").join(profile.as_str());
        let parent = private_child(&home_dir, Path::new("buildkit"))?;
        let dir = private_child(&parent, Path::new(profile.as_str()))?;
        let identity = WorkerHomeId::of(root.as_os_str().as_encoded_bytes());
        let resource_name = format!("gripsack-worker-{identity}");
        // A short private runtime namespace keeps long/spaced homes below the
        // Unix socket pathname bound (socket at <namespace>/run/buildkitd.sock,
        // ~100 bytes against the 108-byte sun_path limit). Its durable owner
        // marker binds the namespace to this record's owner incarnation.
        let runtime_identity = identity.to_string();
        // Lima's own control sockets also fit Darwin's 104-byte sockaddr limit.
        // The full home identity and random incarnation remain in durable records.
        let runtime_identity = if cfg!(target_os = "macos") {
            &runtime_identity[..32]
        } else {
            &runtime_identity
        };
        let run = Path::new("/tmp")
            .canonicalize()?
            .join(format!("gripsack-bk-{runtime_identity}"));
        let leases = private_child(&dir, Path::new("leases"))?;
        let configuration = private_child(&dir, Path::new("docker-config"))?;
        if configuration.entries()?.next().is_some() {
            return Err(WorkerError::Corrupt(
                "owned Docker configuration must stay empty; ambient credentials/context are not worker authority",
            ));
        }
        Ok(Self {
            root,
            dir,
            profile,
            identity,
            resource_name,
            run,
            leases,
        })
    }
    pub(super) fn lock(&self, deadline: Instant) -> Result<gripsack_fs::FlockGuard, WorkerError> {
        loop {
            if let Some(lock) = gripsack_fs::FlockGuard::try_acquire_in(&self.dir, "lifecycle")? {
                return Ok(lock);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(WorkerError::Deadline);
            }
            std::thread::sleep(remaining.min(LOCK_WAIT_INTERVAL));
        }
    }
    pub(super) fn load(&self) -> Result<Option<InstanceRecord>, WorkerError> {
        self.load_record(RECORD)
    }
    pub(super) fn load_previous(&self) -> Result<Option<InstanceRecord>, WorkerError> {
        self.load_record(PREVIOUS_RECORD)
    }
    fn load_record(&self, name: &str) -> Result<Option<InstanceRecord>, WorkerError> {
        let file = match gripsack_fs::open_file_nofollow(&self.dir, Path::new(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_RECORD_BYTES {
            return Err(WorkerError::Corrupt("worker record kind/size"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(WorkerError::Corrupt("worker record grew beyond its bound"));
        }
        let record: InstanceRecord = serde_json::from_slice(&bytes)?;
        record.validate(self)?;
        Ok(Some(record))
    }
    pub(super) fn store(&self, record: &InstanceRecord) -> Result<(), WorkerError> {
        record.validate(self)?;
        let bytes = serde_json::to_vec(record)?;
        gripsack_fs::atomic_write(&self.dir, Path::new(RECORD), &bytes)?;
        Ok(())
    }
    /// Written before parking the stopped instance. Until the replacement's
    /// observed handshake succeeds, this record and its provider configuration
    /// remain recovery authority, not disposable cache.
    pub(super) fn preserve_previous(&self, record: &InstanceRecord) -> Result<(), WorkerError> {
        record.validate(self)?;
        if record.state != RecordState::Stopped || record.container_id.is_none() {
            return Err(WorkerError::Corrupt(
                "previous instance must be identified and stopped",
            ));
        }
        if let Some(existing) = self.load_previous()? {
            if existing != *record {
                return Err(WorkerError::Corrupt(
                    "another previous instance is already protected",
                ));
            }
            return Ok(());
        }
        gripsack_fs::atomic_write(
            &self.dir,
            Path::new(PREVIOUS_RECORD),
            &serde_json::to_vec(record)?,
        )?;
        Ok(())
    }
    pub(super) fn clear_previous(&self, expected: &InstanceRecord) -> Result<(), WorkerError> {
        let Some(previous) = self.load_previous()? else {
            return Ok(());
        };
        if previous != *expected {
            return Err(WorkerError::Corrupt(
                "previous instance changed before retirement",
            ));
        }
        gripsack_fs::remove_file(&self.dir, Path::new(PREVIOUS_RECORD))?;
        gripsack_fs::fsync_pinned_dir(&self.dir, Path::new(PREVIOUS_RECORD))?;
        Ok(())
    }
    /// The runtime NAMESPACE root carries the durable owner marker as
    /// persistent control state; the daemon socket lives in its child `run/`
    /// directory, which alone is disposable. The namespace is published
    /// atomically: a fully marker-staged sibling directory renamed with
    /// no-replace semantics, so a crash leaves only nothing, pre-publication
    /// scratch, or a complete marker-bearing namespace. Pre-existing unmarked
    /// directories are foreign and NEVER adopted — emptiness is not ownership
    /// evidence; the owner nonce is.
    pub(super) fn prepare_runtime(&self, record: &InstanceRecord) -> Result<(), WorkerError> {
        let namespace = match self.namespace_directory() {
            Ok(cap) => {
                // An existing namespace without its marker is not ours —
                // never adopt it, whatever it contains.
                self.verify_runtime_owner(&cap, record)
                    .map_err(|_| WorkerError::Foreign(self.run.display().to_string()))?;
                cap
            }
            Err(WorkerError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                self.publish_namespace(record)?
            }
            Err(error) => return Err(error),
        };
        // Disposable child under a verified owned parent: recreate freely.
        private_child(&namespace, Path::new(RUN_CHILD))?;
        Ok(())
    }
    fn publish_namespace(&self, record: &InstanceRecord) -> Result<gripsack_fs::Dir, WorkerError> {
        let parent = self.namespace_parent()?;
        let name = self.namespace_name()?;
        // A crash before the marker must not reserve the next attempt's name.
        // Unproven remnants remain untouched; a fresh private nonce avoids them.
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(std::io::Error::other)?;
        let mut encoded = [0; 32];
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for (pair, byte) in encoded.as_chunks_mut::<2>().0.iter_mut().zip(random) {
            pair[0] = HEX[(byte >> 4) as usize];
            pair[1] = HEX[(byte & 15) as usize];
        }
        let nonce = std::str::from_utf8(&encoded).expect("hex digits are UTF-8");
        let staging = PathBuf::from(format!("{}.staging-{nonce}", name.display()));
        let mut staging_builder = gripsack_fs::cap_std::fs::DirBuilder::new();
        staging_builder.mode(0o700);
        parent.create_dir_with(&staging, &staging_builder)?;
        let result = (|| {
            let staged = gripsack_fs::open_dir_nofollow(&parent, &staging)?;
            let metadata = staged.metadata(".")?;
            // SAFETY: geteuid has no pointer/memory preconditions.
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(WorkerError::Foreign(staging.display().to_string()));
            }
            // The marker completes the staged namespace; atomic_write is
            // itself temp+fsync+rename, so a present marker is whole.
            gripsack_fs::atomic_write(
                &staged,
                Path::new(OWNER_MARKER),
                record.owner.to_string().as_bytes(),
            )?;
            // The staged entry must be durable before the rename publishes it.
            gripsack_fs::fsync_pinned_dir(&parent, &staging)?;
            match gripsack_fs::rename_noreplace(&parent, &staging, &name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // A concurrent publisher won the name. Its marker decides
                    // ownership below; our staged directory is removed by the
                    // caller's cleanup since we created it in this call.
                }
                Err(error) => return Err(error.into()),
            }
            gripsack_fs::fsync_pinned_dir(&parent, &name)?;
            Ok(())
        })();
        // Also reclaim our completed scratch when another publisher won the
        // no-replace rename. Never erase a markerless/foreign remnant on error.
        match self.reclaim_staging(&parent, &staging, record) {
            Ok(()) => {}
            Err(WorkerError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if result.is_ok() => return Err(error),
            Err(_) => {} // Original failure wins; uncertain scratch stays quarantined.
        }
        result?;
        let namespace = self.namespace_directory()?;
        self.verify_runtime_owner(&namespace, record)?;
        Ok(namespace)
    }
    /// Interrupted pre-publication scratch: reclaim (remove and re-stage)
    /// ONLY when the staging directory's own valid owner marker proves it
    /// ours and it holds nothing else. Anything else stays foreign.
    fn reclaim_staging(
        &self,
        parent: &gripsack_fs::Dir,
        staging: &Path,
        record: &InstanceRecord,
    ) -> Result<(), WorkerError> {
        let cap = gripsack_fs::open_dir_nofollow(parent, staging)?;
        let metadata = cap.metadata(".")?;
        // SAFETY: geteuid has no pointer/memory preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(WorkerError::Foreign(staging.display().to_string()));
        }
        self.verify_runtime_owner(&cap, record)
            .map_err(|_| WorkerError::Foreign(staging.display().to_string()))?;
        if cap.entries()?.count() != 1 {
            return Err(WorkerError::Foreign(staging.display().to_string()));
        }
        // Fully staged by a crashed publication of this same owner: the
        // marker is the ownership evidence, so removal is safe.
        gripsack_fs::remove_file(&cap, Path::new(OWNER_MARKER))?;
        parent.remove_dir(staging)?;
        gripsack_fs::fsync_pinned_dir(parent, staging)?;
        Ok(())
    }
    fn namespace_parent(&self) -> Result<gripsack_fs::Dir, WorkerError> {
        Ok(gripsack_fs::open(self.run.parent().ok_or(
            WorkerError::Corrupt("runtime root lacks a parent"),
        )?)?)
    }
    fn namespace_name(&self) -> Result<PathBuf, WorkerError> {
        Ok(PathBuf::from(self.run.file_name().ok_or(
            WorkerError::Corrupt("runtime root lacks a name"),
        )?))
    }
    pub(super) fn namespace_directory(&self) -> Result<gripsack_fs::Dir, WorkerError> {
        let parent = self.namespace_parent()?;
        let cap = gripsack_fs::open_dir_nofollow(&parent, &self.namespace_name()?)?;
        let metadata = cap.metadata(".")?;
        // SAFETY: geteuid has no pointer/memory preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(WorkerError::Foreign(self.run.display().to_string()));
        }
        Ok(cap)
    }
    pub(super) fn verify_runtime_owner(
        &self,
        directory: &gripsack_fs::Dir,
        record: &InstanceRecord,
    ) -> Result<(), WorkerError> {
        let mut marker = gripsack_fs::open_file_nofollow(directory, Path::new(OWNER_MARKER))?;
        let mut bytes = [0; 64];
        if marker.metadata()?.len() != bytes.len() as u64 {
            return Err(WorkerError::Foreign(self.run.display().to_string()));
        }
        marker.read_exact(&mut bytes)?;
        if bytes != record.owner.to_string().as_bytes() || marker.read(&mut [0])? != 0 {
            return Err(WorkerError::Foreign(self.run.display().to_string()));
        }
        Ok(())
    }
    /// Called only after the owned container is removed with the lifecycle
    /// lock held. Removes the disposable child `run/` directory; the
    /// namespace root and its owner marker are persistent control state and
    /// stay (like instance.json). Ownership authority is the verified marker,
    /// so an interrupted teardown (socket unlinked, run/ left behind) resumes
    /// without any name-based adoption, and foreign content stays untouched.
    pub(super) fn remove_runtime(&self, record: &InstanceRecord) -> Result<(), WorkerError> {
        let namespace = match self.namespace_directory() {
            Ok(cap) => cap,
            Err(WorkerError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        // Unmarked or wrongly marked namespaces are foreign and untouched.
        self.verify_runtime_owner(&namespace, record)
            .map_err(|_| WorkerError::Foreign(self.run.display().to_string()))?;
        let run = match gripsack_fs::open_dir_nofollow(&namespace, Path::new(RUN_CHILD)) {
            Ok(cap) => cap,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in run.entries()? {
            let name = entry?.file_name();
            if name != SOCKET_NAME {
                return Err(WorkerError::Foreign(
                    self.run_dir().join(name).display().to_string(),
                ));
            }
        }
        match gripsack_fs::remove_file(&run, Path::new(SOCKET_NAME)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        namespace.remove_dir(Path::new(RUN_CHILD))?;
        gripsack_fs::fsync_pinned_dir(&namespace, &self.run)?;
        Ok(())
    }
    pub(super) fn run_dir(&self) -> PathBuf {
        self.run.join(RUN_CHILD)
    }
    pub(super) fn socket(&self) -> PathBuf {
        self.run_dir().join(SOCKET_NAME)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    pub(super) fn cache_volume(&self) -> String {
        format!("{}-cache", self.resource_name)
    }
}

pub(super) fn private_child(
    parent: &gripsack_fs::Dir,
    name: &Path,
) -> Result<gripsack_fs::Dir, WorkerError> {
    gripsack_fs::create_dir_all(parent, name)?;
    let directory = gripsack_fs::open_dir_nofollow(parent, name)?;
    let metadata = directory.metadata(".")?;
    // SAFETY: geteuid has no pointer/memory preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(WorkerError::Foreign(name.display().to_string()));
    }
    if metadata.mode() & 0o7777 != 0o700 {
        directory.set_permissions(
            ".",
            gripsack_fs::cap_std::fs::Permissions::from_std(std::fs::Permissions::from_mode(0o700)),
        )?;
        gripsack_fs::fsync_pinned_dir(&directory, name)?;
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    #[test]
    fn worker_state_cannot_follow_private_namespace_aliases() {
        for alias_parent in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            let foreign = root.path().join("foreign");
            std::fs::create_dir(&home).unwrap();
            std::fs::create_dir(&foreign).unwrap();
            std::fs::write(foreign.join("keep"), b"not worker state").unwrap();
            std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o755)).unwrap();
            let alias = if alias_parent {
                home.join("buildkit")
            } else {
                std::fs::create_dir(home.join("buildkit")).unwrap();
                home.join("buildkit/default")
            };
            std::os::unix::fs::symlink(&foreign, alias).unwrap();
            assert!(
                WorkerHome::open(&home, WorkerProfile::parse("default").unwrap()).is_err(),
                "worker_namespace_alias_was_followed"
            );
            let names = std::fs::read_dir(&foreign)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            assert_eq!(names, vec![std::ffi::OsString::from("keep")]);
            assert_eq!(
                std::fs::read(foreign.join("keep")).unwrap(),
                b"not worker state"
            );
            assert_eq!(std::fs::metadata(&foreign).unwrap().mode() & 0o7777, 0o755);
        }
    }

    struct Fixture {
        _root: tempfile::TempDir,
        home: WorkerHome,
        record: InstanceRecord,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let home_path = root.path().join("home");
            std::fs::create_dir(&home_path).unwrap();
            let home =
                WorkerHome::open(&home_path, WorkerProfile::parse("default").unwrap()).unwrap();
            let record = InstanceRecord::fresh(&home).unwrap();
            Self {
                _root: root,
                home,
                record,
            }
        }
        /// A foreign squatter at the predictable namespace name: an empty,
        /// mode-0700, uid-owned directory with NO owner marker.
        fn create_unmarked_namespace(&self) {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&self.home.run)
                .unwrap();
        }
        fn staging_path(&self) -> PathBuf {
            let name = self.home.run.file_name().unwrap().to_string_lossy();
            self.home.run.with_file_name(format!("{name}.staging"))
        }
        fn marker_bytes(&self) -> Vec<u8> {
            std::fs::read(self.home.run.join(OWNER_MARKER)).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home.run);
            let _ = std::fs::remove_dir_all(self.staging_path());
        }
    }

    #[test]
    fn namespace_publishes_atomically_and_run_child_is_disposable() {
        let fixture = Fixture::new();
        fixture.home.prepare_runtime(&fixture.record).unwrap();
        assert_eq!(
            fixture.marker_bytes(),
            fixture.record.owner.to_string().as_bytes(),
            "owner_marker_not_published"
        );
        assert!(fixture.home.run_dir().is_dir(), "run_child_missing");
        // Retry over the completed publication is idempotent.
        fixture.home.prepare_runtime(&fixture.record).unwrap();
        // Teardown removes only the disposable child; the marker-bearing
        // namespace is persistent control state.
        fixture.home.remove_runtime(&fixture.record).unwrap();
        assert!(!fixture.home.run_dir().exists(), "run_child_survived");
        assert!(
            fixture.home.run.is_dir(),
            "persistent_namespace_was_removed"
        );
        assert_eq!(
            fixture.marker_bytes(),
            fixture.record.owner.to_string().as_bytes(),
            "owner_marker_lost_on_teardown"
        );
        // A crashed teardown retry (run/ already gone) and a fresh start are fine.
        fixture.home.remove_runtime(&fixture.record).unwrap();
        fixture.home.prepare_runtime(&fixture.record).unwrap();
        assert!(fixture.home.run_dir().is_dir(), "run_child_not_recreated");
    }

    #[test]
    fn preexisting_empty_unmarked_namespace_is_refused_and_untouched() {
        let fixture = Fixture::new();
        fixture.create_unmarked_namespace();
        assert!(
            matches!(
                fixture.home.prepare_runtime(&fixture.record),
                Err(WorkerError::Foreign(_))
            ),
            "empty_unmarked_namespace_was_adopted"
        );
        assert!(
            matches!(
                fixture.home.remove_runtime(&fixture.record),
                Err(WorkerError::Foreign(_))
            ),
            "empty_unmarked_namespace_was_removed"
        );
        assert!(fixture.home.run.is_dir(), "foreign_namespace_was_erased");
        assert_eq!(
            std::fs::read_dir(&fixture.home.run).unwrap().count(),
            0,
            "foreign_namespace_content_changed"
        );
    }

    #[test]
    fn unproven_staging_neither_blocks_retry_nor_lends_ownership() {
        for with_junk in [false, true] {
            let fixture = Fixture::new();
            let staging = fixture.staging_path();
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&staging)
                .unwrap();
            if with_junk {
                // Even a marker of a DIFFERENT owner is no evidence for us.
                let other = InstanceRecord::fresh(&fixture.home).unwrap();
                std::fs::write(
                    staging.join(OWNER_MARKER),
                    other.owner.to_string().as_bytes(),
                )
                .unwrap();
            }
            let before = std::fs::read(staging.join(OWNER_MARKER)).ok();
            fixture.home.prepare_runtime(&fixture.record).unwrap();
            assert_eq!(std::fs::read(staging.join(OWNER_MARKER)).ok(), before);
            assert_eq!(
                std::fs::read_dir(&staging).unwrap().count(),
                usize::from(with_junk)
            );
            assert!(fixture.home.run_dir().is_dir());
            assert_eq!(
                fixture.marker_bytes(),
                fixture.record.owner.to_string().as_bytes()
            );
        }
    }

    #[test]
    fn foreign_owner_marker_is_never_adopted() {
        let fixture = Fixture::new();
        fixture.create_unmarked_namespace();
        let other = InstanceRecord::fresh(&fixture.home).unwrap();
        assert_ne!(other.owner, fixture.record.owner);
        std::fs::write(
            fixture.home.run.join(OWNER_MARKER),
            other.owner.to_string().as_bytes(),
        )
        .unwrap();
        assert!(
            matches!(
                fixture.home.prepare_runtime(&fixture.record),
                Err(WorkerError::Foreign(_))
            ),
            "foreign_owner_marker_was_adopted"
        );
        assert!(
            matches!(
                fixture.home.remove_runtime(&fixture.record),
                Err(WorkerError::Foreign(_))
            ),
            "foreign_marked_namespace_was_removed"
        );
        assert_eq!(
            fixture.marker_bytes(),
            other.owner.to_string().as_bytes(),
            "foreign_marker_was_erased"
        );
    }

    #[test]
    fn teardown_refuses_unexpected_entries_without_erasing() {
        let fixture = Fixture::new();
        fixture.home.prepare_runtime(&fixture.record).unwrap();
        std::fs::write(fixture.home.run_dir().join("foreign"), b"keep").unwrap();
        assert!(
            matches!(
                fixture.home.remove_runtime(&fixture.record),
                Err(WorkerError::Foreign(_))
            ),
            "run_child_with_foreign_entry_was_removed"
        );
        assert_eq!(
            std::fs::read(fixture.home.run_dir().join("foreign")).unwrap(),
            b"keep",
            "foreign_entry_was_erased"
        );
        assert!(fixture.home.run_dir().is_dir(), "run_child_was_erased");
        assert_eq!(
            fixture.marker_bytes(),
            fixture.record.owner.to_string().as_bytes(),
            "owner_marker_was_erased"
        );
    }

    #[test]
    fn instance_binding_survives_record_reload() {
        let fixture = Fixture::new();
        let mut record = fixture.record.clone();
        record.cpus = Some(NonZeroU16::new(4).unwrap());
        record.memory = Some(8 * 1024 * 1024 * 1024);
        record.state = RecordState::Ready;
        record.container_id = Some(
            observation::ContainerId::try_from("b".repeat(64)).expect("valid container identity"),
        );
        record.epoch = Some(FenceEpoch::new(3).expect("nonzero epoch"));
        record.next_epoch = NonZeroU64::new(4).unwrap();
        record.next_lease = NonZeroU64::new(9).unwrap();
        fixture.home.store(&record).unwrap();
        let loaded = fixture.home.load().unwrap().expect("record persisted");
        assert_eq!(loaded, record, "restart_lost_instance_binding");
        // A record whose resource binding was cleared (the legacy shape)
        // still loads, explicitly unbound rather than silently defaulted.
        let mut unbound = loaded;
        unbound.cpus = None;
        unbound.memory = None;
        fixture.home.store(&unbound).unwrap();
        let reloaded = fixture
            .home
            .load()
            .unwrap()
            .expect("unbound record persisted");
        assert_eq!(reloaded.cpus, None);
        assert_eq!(reloaded.memory, None);
    }

    #[test]
    fn previous_image_record_is_not_corruption() {
        let fixture = Fixture::new();
        let mut record = fixture.record.clone();
        record.image = "moby/buildkit:v0.32.0@sha256:previous".to_owned();
        fixture.home.store(&record).unwrap();
        let loaded = fixture
            .home
            .load()
            .unwrap()
            .expect("previous image record loads");
        assert_eq!(loaded.image, record.image, "previous_pin_record_rejected");
    }
}
