//! Read-only worker facts. Missing resources and unavailable measurements are
//! deliberately distinct from a measured zero; every observation names its source.
use super::{
    WorkerError, WorkerOptions, home::InstanceRecord, observation::ContainerObservation, provider,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerObservation<T> {
    Absent,
    Unavailable(String),
    Observed { value: T, source: &'static str },
}
impl<T> WorkerObservation<T> {
    pub(super) fn measured(value: T, source: &'static str) -> Self {
        Self::Observed { value, source }
    }
    pub(super) fn unavailable(reason: &str) -> Self {
        Self::Unavailable(reason.to_owned())
    }
}

#[derive(Debug)]
pub struct WorkerFact<T> {
    pub configured: Option<T>,
    pub observed: WorkerObservation<T>,
}
impl<T> WorkerFact<T> {
    fn new(configured: Option<T>) -> Self {
        Self {
            configured,
            observed: WorkerObservation::Absent,
        }
    }
}

#[derive(Debug)]
pub struct WorkerDiskUsage {
    /// Exact owned scope, never global Docker/host usage.
    pub scope: String,
    /// Accounting differs between Docker writable layers and host allocated blocks.
    pub accounting: &'static str,
    pub bytes: WorkerObservation<u64>,
}

#[derive(Debug)]
pub struct WorkerInspection {
    /// Unbootstrapped settings are a request, not a persisted instance binding.
    pub configuration_source: &'static str,
    pub image: WorkerFact<String>,
    pub image_id: WorkerObservation<String>,
    pub vm_image: WorkerFact<String>,
    pub helper_version: WorkerFact<String>,
    pub daemon_version: WorkerFact<String>,
    pub target: WorkerFact<String>,
    pub cpus: WorkerFact<u16>,
    pub memory_bytes: WorkerFact<u64>,
    pub cache_scope: String,
    pub cache_present: WorkerObservation<bool>,
    pub disk: Vec<WorkerDiskUsage>,
}
impl WorkerInspection {
    pub(super) fn configured(
        record: Option<&InstanceRecord>,
        options: &WorkerOptions,
        target: &str,
        cache_scope: String,
    ) -> Self {
        let image = record.map_or(provider::PINNED_WORKER_IMAGE, |r| r.image.as_str());
        Self {
            configuration_source: if record.is_some() {
                "durable instance record"
            } else {
                "requested settings; not provisioned"
            },
            image: WorkerFact::new(Some(image.to_owned())),
            image_id: WorkerObservation::Absent,
            vm_image: WorkerFact::new(None),
            helper_version: WorkerFact {
                configured: None,
                observed: WorkerObservation::unavailable(
                    "helper not queried; no instance observed",
                ),
            },
            daemon_version: WorkerFact::new(
                (image == provider::PINNED_WORKER_IMAGE)
                    .then(|| crate::protocol::EXPECTED_DAEMON_VERSION.to_owned()),
            ),
            target: WorkerFact::new(Some(target.to_owned())),
            cpus: WorkerFact::new(
                record.map_or(Some(options.cpus.get()), |r| r.cpus.map(|c| c.get())),
            ),
            memory_bytes: WorkerFact::new(
                record.map_or(Some(options.memory.bytes()), |r| r.memory),
            ),
            cache_scope,
            cache_present: WorkerObservation::Absent,
            disk: Vec::new(),
        }
    }
    pub(super) fn instance(
        &mut self,
        observed: &ContainerObservation,
        source: &'static str,
    ) -> Result<(), WorkerError> {
        self.image.observed = WorkerObservation::measured(observed.image.clone(), source);
        self.cpus.observed = WorkerObservation::measured(
            super::observation::cpus_from_nano(observed.nano_cpus)
                .ok_or(WorkerError::Corrupt(
                    "observed worker CPU allocation is not integral",
                ))?
                .get(),
            source,
        );
        self.memory_bytes.observed = WorkerObservation::measured(observed.memory, source);
        self.daemon_version.observed =
            WorkerObservation::unavailable("worker stopped; no daemon query performed");
        Ok(())
    }
}

/// Admit the pinned client's bounded JSON worker response without inventing a
/// daemon version or platform from the requested image.
pub(super) fn daemon_facts(bytes: &[u8], target: &str) -> Result<(String, String), WorkerError> {
    let workers: Vec<serde_json::Value> = serde_json::from_slice(bytes)?;
    if workers.len() != 1 {
        return Err(WorkerError::Corrupt(
            "owned daemon must expose exactly one worker",
        ));
    }
    let worker = &workers[0];
    let version = worker["buildkitVersion"]["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or(WorkerError::Corrupt("daemon omitted its version"))?;
    let (os, arch) = target
        .split_once('/')
        .ok_or(WorkerError::Corrupt("invalid configured worker target"))?;
    if !worker["platforms"].as_array().is_some_and(|platforms| {
        platforms
            .iter()
            .any(|p| p["os"] == os && p["architecture"] == arch)
    }) {
        return Err(WorkerError::Corrupt(
            "daemon does not advertise the configured Linux target",
        ));
    }
    Ok((version.to_owned(), target.to_owned()))
}

pub(super) fn du_bytes(bytes: &[u8]) -> Result<u64, WorkerError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| WorkerError::Corrupt("disk measurement is not UTF-8"))?;
    let mut lines = text.lines();
    let line = lines
        .next()
        .ok_or(WorkerError::Corrupt("disk measurement is absent"))?;
    if lines.next().is_some() {
        return Err(WorkerError::Corrupt(
            "disk measurement contains multiple scopes",
        ));
    }
    let (count, _) = line
        .split_once(char::is_whitespace)
        .filter(|(_, scope)| !scope.trim().is_empty())
        .ok_or(WorkerError::Corrupt("disk measurement omitted its scope"))?;
    count
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(1024))
        .ok_or(WorkerError::Corrupt(
            "disk measurement is invalid or overflowed",
        ))
}

/// Count allocated blocks, not sparse virtual capacity. No symlink traversal,
/// mount crossing, hardlink double counting, or unbounded directory walk.
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(super) fn allocated_bytes(
    directory: &gripsack_fs::Dir,
    deadline: std::time::Instant,
) -> Result<u64, WorkerError> {
    use gripsack_fs::cap_std::fs::MetadataExt;
    fn visit(
        directory: &gripsack_fs::Dir,
        device: u64,
        seen: &mut std::collections::BTreeSet<(u64, u64)>,
        remaining: &mut usize,
        depth: usize,
        deadline: std::time::Instant,
    ) -> Result<u64, WorkerError> {
        if depth > 64 {
            return Err(WorkerError::Corrupt(
                "disk measurement depth limit exceeded",
            ));
        }
        let mut bytes = 0_u64;
        for entry in directory.entries()? {
            if std::time::Instant::now() >= deadline {
                return Err(WorkerError::Deadline);
            }
            *remaining = remaining.checked_sub(1).ok_or(WorkerError::Corrupt(
                "disk measurement entry limit exceeded",
            ))?;
            let entry = entry?;
            let name = entry.file_name();
            let metadata = directory.symlink_metadata(&name)?;
            if metadata.dev() != device {
                return Err(WorkerError::Corrupt(
                    "disk measurement refuses a nested mount",
                ));
            }
            if !seen.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            let allocated = metadata
                .blocks()
                .checked_mul(512)
                .ok_or(WorkerError::Corrupt("disk block count overflowed"))?;
            bytes = bytes
                .checked_add(allocated)
                .ok_or(WorkerError::Corrupt("disk usage overflowed"))?;
            if metadata.is_dir() {
                let child = gripsack_fs::open_dir_nofollow(directory, std::path::Path::new(&name))?;
                bytes = bytes
                    .checked_add(visit(&child, device, seen, remaining, depth + 1, deadline)?)
                    .ok_or(WorkerError::Corrupt("disk usage overflowed"))?;
            }
        }
        Ok(bytes)
    }
    let metadata = directory.dir_metadata()?;
    visit(
        directory,
        metadata.dev(),
        &mut std::collections::BTreeSet::new(),
        &mut 100_000,
        0,
        deadline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocated_measurement_does_not_follow_links_or_charge_sparse_capacity() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        std::fs::write(foreign.path().join("payload"), vec![1; 64 * 1024]).unwrap();
        let sparse = root.path().join("sparse");
        std::fs::File::create(&sparse)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        std::fs::hard_link(&sparse, root.path().join("same-inode")).unwrap();
        let link = root.path().join("foreign");
        symlink(foreign.path(), &link).unwrap();
        let directory = gripsack_fs::open(root.path()).unwrap();
        let expected = (std::fs::metadata(&sparse).unwrap().blocks()
            + std::fs::symlink_metadata(&link).unwrap().blocks())
            * 512;
        assert_eq!(
            allocated_bytes(
                &directory,
                std::time::Instant::now() + std::time::Duration::from_secs(2)
            )
            .unwrap(),
            expected
        );
        assert!(allocated_bytes(&directory, std::time::Instant::now()).is_err());
    }
    #[test]
    fn disk_measurement_distinguishes_zero_missing_and_overflow() {
        assert_eq!(du_bytes(b"0\t/var/lib/buildkit\n").unwrap(), 0);
        assert_eq!(du_bytes(b"17\t/var/lib/buildkit\n").unwrap(), 17 * 1024);
        for invalid in [
            b"".as_slice(),
            b"17",
            b"17\t",
            b"-1\t/cache\n",
            b"18446744073709551615\t/cache\n",
            b"1\t/a\n2\t/b\n",
        ] {
            assert!(du_bytes(invalid).is_err());
        }
    }
    #[test]
    fn daemon_facts_require_actual_version_and_target() {
        let worker = serde_json::json!({"buildkitVersion":{"version":"v0.33.0"},"platforms":[{"os":"linux","architecture":"arm64"}]});
        let bytes = serde_json::to_vec(&vec![worker.clone()]).unwrap();
        assert_eq!(
            daemon_facts(&bytes, "linux/arm64").unwrap(),
            ("v0.33.0".to_owned(), "linux/arm64".to_owned())
        );
        assert!(daemon_facts(&bytes, "linux/amd64").is_err());
        assert!(daemon_facts(b"[]", "linux/arm64").is_err());
        assert!(
            daemon_facts(
                &serde_json::to_vec(&vec![worker.clone(), worker]).unwrap(),
                "linux/arm64"
            )
            .is_err()
        );
        assert!(
            daemon_facts(
                br#"[{"platforms":[{"os":"linux","architecture":"arm64"}]}]"#,
                "linux/arm64"
            )
            .is_err()
        );
    }
}
