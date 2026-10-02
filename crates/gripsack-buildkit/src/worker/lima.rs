//! Owned Apple-Silicon VZ guest. Lifecycle decisions stay in the shared manager;
//! this module owns only pinned provisioning, observations and scoped VM effects.
mod assets;
mod authority;
mod capabilities;
mod config;
mod previous;
#[cfg(test)]
mod tests;
use super::{
    WorkerError, WorkerOptions,
    home::{InstanceRecord, WorkerHome},
    observation::{ContainerId, ContainerObservation},
};
use config::{CACHE_NAME, INSPECT_FORMAT, VM_NAME, VmIntent, VmObservation};
use gripsack_fs::cap_std::fs::{DirBuilderExt, MetadataExt};
use gripsack_process::{
    Control, EnvironmentOverlay, Invocation, Limits, NativeInput, OperatorEnvironment,
    ProcessLeases, ProcessRole, SelectedProgram, Sha256Digest, StdoutByteLimit,
};
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

// All execution-bearing inputs participate in the idle-replacement binding.
pub(super) const PINNED_WORKER_IMAGE: &str = concat!(
    "lima-vz-aarch64:",
    "22aee997df59e4fd448041b2d1214e48bd8eaf705d2d48a4307d65c1b179dc97:",
    "a40713938d74aaec811f74cb1fa8bfcb535d22e26b2a0ca1cc90ad9db898feb9:",
    "e5acfb5929f967fde3b925ddb39f79fd481a0e96774c641fab3a0e83950d7bfa"
);
const MAX_METADATA_BYTES: u64 = 64 * 1024;
const MAX_EFFECT_BYTES: usize = 1024 * 1024;
const INTENT: &str = "lima-instance.json";
const OWNER: &str = "gripsack-owner";

pub(super) fn validate_options(options: &WorkerOptions) -> Result<(), WorkerError> {
    if options.memory.bytes() < config::MINIMUM_VM_MEMORY
        || options.memory.bytes() % (1024 * 1024) != 0
    {
        return Err(WorkerError::Invalid(
            "Lima memory must be at least one GiB and an integral number of MiB",
        ));
    }
    Ok(())
}

pub(super) fn preflight(options: &WorkerOptions, deadline: Instant) -> Result<(), WorkerError> {
    capabilities::preflight(options, deadline)
}

pub(super) struct Provider<'a> {
    environment: &'a OperatorEnvironment,
    home: WorkerHome,
    deadline: Instant,
    assets_relative: PathBuf,
}
impl<'a> Provider<'a> {
    pub(super) fn new(
        environment: &'a OperatorEnvironment,
        home: &WorkerHome,
        deadline: Instant,
    ) -> Result<Self, WorkerError> {
        Ok(Self {
            environment,
            deadline,
            assets_relative: Path::new("lima-assets")
                .join(Sha256Digest::of(PINNED_WORKER_IMAGE.as_bytes()).to_string()),
            home: WorkerHome {
                root: home.root.clone(),
                dir: home.dir.try_clone()?,
                profile: home.profile.clone(),
                identity: home.identity,
                resource_name: home.resource_name.clone(),
                run: home.run.clone(),
                leases: home.leases.try_clone()?,
            },
        })
    }
    pub(super) fn preflight(&self, options: &WorkerOptions) -> Result<(), WorkerError> {
        preflight(options, self.deadline)
    }
    fn assets_path(&self) -> PathBuf {
        self.home.root.join(&self.assets_relative)
    }
    fn lima_home(&self) -> PathBuf {
        self.home.run.join("l")
    }
    fn record(&self) -> Result<InstanceRecord, WorkerError> {
        self.home.load()?.ok_or(WorkerError::Corrupt(
            "Lima effect lacks its durable manager record",
        ))
    }
    fn owned_home(&self, record: &InstanceRecord) -> Result<Option<gripsack_fs::Dir>, WorkerError> {
        let namespace = match self.home.namespace_directory() {
            Ok(namespace) => namespace,
            Err(WorkerError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        self.home.verify_runtime_owner(&namespace, record)?;
        let lima = match gripsack_fs::open_dir_nofollow(&namespace, Path::new("l")) {
            Ok(lima) => lima,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        verify_owner(&lima, record)?;
        // Never permit Lima's implicit config overlays to alter an owned VM.
        for name in [
            "_config/default.yaml",
            "_config/override.yaml",
            "_config/base.yaml",
        ] {
            match lima.symlink_metadata(name) {
                Ok(_) => return Err(WorkerError::Foreign(name.to_owned())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(Some(lima))
    }
    fn run(
        &self,
        arguments: &[OsString],
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Vec<u8>, WorkerError> {
        let record = self.record()?;
        self.owned_home(&record)?
            .ok_or(WorkerError::Corrupt("Lima namespace is absent"))?;
        let assets = gripsack_fs::open_dir_nofollow(&self.home.dir, &self.assets_relative)?;
        // Lima discovers executable driver plugins beside argv[0]. The admitted
        // release installs only the controller and guest agent; never execute
        // extra plugins from a pre-existing or modified helper directory.
        for name in ["libexec", "lib"] {
            match assets.symlink_metadata(name) {
                Ok(_) => {
                    return Err(WorkerError::Foreign(format!(
                        "Lima helper plugin directory {name}"
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let binary = self.assets_path().join("bin/limactl");
        let program = SelectedProgram::select(
            self.environment,
            &binary,
            Some(Sha256Digest::parse(assets::LIMACTL_SHA256)?),
            self.deadline,
        )?;
        let overlay = EnvironmentOverlay::admit(
            [
                (
                    OsString::from("LIMA_HOME"),
                    self.lima_home().into_os_string(),
                ),
                (OsString::from("LIMA_SHELLENV_ALLOW"), OsString::new()),
                (OsString::from("LIMA_SHELLENV_BLOCK"), OsString::from("*")),
            ],
            [],
            [],
        )?;
        let mut output = Vec::new();
        let argv: Vec<&OsStr> = arguments.iter().map(OsString::as_os_str).collect();
        let outcome = Invocation::admit(
            self.environment,
            ProcessRole::Build,
            &program,
            &self.home.root,
            Limits {
                timeout: self.deadline.saturating_duration_since(Instant::now()),
                operation_deadline: Some(self.deadline),
                stdout_bytes: StdoutByteLimit::new(MAX_EFFECT_BYTES as u64),
                ..Limits::default()
            },
        )?
        .with_overlay(overlay)
        .retain_leases(ProcessLeases {
            worker: Some(lock.duplicate_handle()?),
            retention: None,
        })?
        .run(&argv, NativeInput::Bytes(b""), None, |bytes| {
            if bytes.len() <= MAX_EFFECT_BYTES.saturating_sub(output.len()) {
                output.extend_from_slice(bytes);
            }
            Control::Continue
        })?;
        if !outcome.success {
            return Err(WorkerError::Effect(
                String::from_utf8_lossy(&outcome.stderr).into_owned(),
            ));
        }
        Ok(output)
    }
    pub(super) fn prepare_control(
        &self,
        offline: bool,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let record = self.record()?;
        self.home.prepare_runtime(&record)?;
        let namespace = self.home.namespace_directory()?;
        self.home.verify_runtime_owner(&namespace, &record)?;
        owned_child(&namespace, "l", &record)?;
        let parent = super::home::private_child(&self.home.dir, Path::new("lima-assets"))?;
        let directory = super::home::private_child(
            &parent,
            Path::new(
                self.assets_relative
                    .file_name()
                    .expect("content-addressed assets"),
            ),
        )?;
        let lima = assets::provision(
            &directory,
            &assets::LIMA,
            self.environment,
            offline,
            self.deadline,
            lock,
        )?;
        assets::install_lima(&directory, lima, self.deadline)
    }
    pub(super) fn provision_image(
        &self,
        offline: bool,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let directory = gripsack_fs::open_dir_nofollow(&self.home.dir, &self.assets_relative)?;
        assets::provision(
            &directory,
            &assets::GUEST,
            self.environment,
            offline,
            self.deadline,
            lock,
        )?;
        let tools = super::home::private_child(&directory, Path::new("guest-tools"))?;
        assets::provision(
            &tools,
            &assets::BUILDKIT,
            self.environment,
            offline,
            self.deadline,
            lock,
        )?;
        Ok(())
    }
    pub(super) fn inspect(
        &self,
        _home: &WorkerHome,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        self.inspect_named(&self.record()?, VM_NAME, INTENT, lock)
    }
    pub(super) fn verify_container(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        observed: &ContainerObservation,
    ) -> Result<(), WorkerError> {
        if observed.image != record.image
            || record
                .cpus
                .is_some_and(|cpus| observed.nano_cpus != super::observation::nano_cpus(cpus))
            || record
                .memory
                .is_some_and(|memory| observed.memory != memory)
            || !observed.labels.as_ref().is_some_and(|labels| {
                labels.get("dev.gripsack.owner") == Some(&record.owner.to_string())
                    && labels.get("dev.gripsack.home") == Some(&home.identity.to_string())
            })
        {
            return Err(WorkerError::Foreign(observed.id.as_str().into()));
        }
        Ok(())
    }
    pub(super) fn configured_details(
        &self,
        _home: &WorkerHome,
        record: Option<&InstanceRecord>,
        options: &WorkerOptions,
    ) -> super::inspection::WorkerInspection {
        let mut details = super::inspection::WorkerInspection::configured(
            record,
            options,
            "linux/arm64",
            self.lima_home().join("_disks/cache").display().to_string(),
        );
        if record.is_none_or(|record| record.image == PINNED_WORKER_IMAGE) {
            details.vm_image.configured = Some(format!(
                "Ubuntu 24.04 arm64 release-20251213 sha256:{}",
                assets::GUEST.sha256
            ));
            details.helper_version.configured = Some("Lima v2.0.3".to_owned());
        }
        details
    }

    pub(super) fn status_details(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        observed: Option<&ContainerObservation>,
        details: &mut super::inspection::WorkerInspection,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        use super::inspection::{WorkerDiskUsage, WorkerObservation as Fact};
        let Some(lima) = self.owned_home(record)? else {
            return Ok(());
        };
        let cache = self.verify_volume(home, record, lock)?;
        details.cache_present = Fact::measured(cache, "owned Lima cache disk and owner receipt");
        let cache_bytes = if cache {
            let directory = gripsack_fs::open_dir_nofollow(&lima, Path::new("_disks/cache"))?;
            match super::inspection::allocated_bytes(&directory, self.deadline) {
                Ok(bytes) => Fact::measured(bytes, "host filesystem allocated blocks"),
                Err(error) => Fact::Unavailable(error.to_string()),
            }
        } else {
            Fact::Absent
        };
        details.disk.push(WorkerDiskUsage {
            scope: self.lima_home().join("_disks/cache").display().to_string(),
            accounting: "host allocated cache-directory bytes; not virtual disk capacity; excludes symlink targets",
            bytes: cache_bytes,
        });
        if let Some(observed) = observed {
            self.verify_container(home, record, observed)?;
            details.instance(
                observed,
                "admitted Lima instance metadata and persisted configuration",
            )?;
            let intent = self.intent(record)?;
            self.configuration(&lima, &intent)?;
            details.helper_version.configured = Some(format!("Lima {}", intent.lima_version));
            details.helper_version.observed = match self.run(&args(&["--version"]), lock) {
                Ok(bytes) => Fact::measured(
                    String::from_utf8(bytes)
                        .map_err(|_| WorkerError::Corrupt("Lima version is not UTF-8"))?
                        .trim()
                        .to_owned(),
                    "byte-verified limactl --version",
                ),
                Err(error) => Fact::Unavailable(error.to_string()),
            };
            let images = &intent.configuration["images"];
            let digest = images[0]["digest"].as_str().ok_or(WorkerError::Corrupt(
                "owned VM configuration omitted image digest",
            ))?;
            let location = images[0]["location"].as_str().ok_or(WorkerError::Corrupt(
                "owned VM configuration omitted image location",
            ))?;
            details.vm_image.observed = Fact::measured(
                format!("{location} {digest}"),
                "persisted VM image declaration; not a rehash of mutable guest disk",
            );
            details.image_id = Fact::unavailable(
                "VM disks are mutable; the configured bootstrap image digest is reported separately",
            );
            details.target.observed =
                Fact::unavailable("worker stopped; guest platform not queried");
            let directory = gripsack_fs::open_dir_nofollow(&lima, Path::new(VM_NAME))?;
            details.disk.push(WorkerDiskUsage {
                scope: self.lima_home().join(VM_NAME).display().to_string(),
                accounting: "host allocated VM-directory bytes; not virtual disk capacity; excludes symlink targets",
                bytes: match super::inspection::allocated_bytes(&directory, self.deadline) {
                    Ok(bytes) => Fact::measured(bytes, "host filesystem allocated blocks"),
                    Err(error) => Fact::Unavailable(error.to_string()),
                },
            });
            if observed.running {
                let bytes = self.run(
                    &args(&[
                        "shell",
                        VM_NAME,
                        "sudo",
                        "--non-interactive",
                        "/opt/gripsack-buildkit/bin/buildctl",
                        "--addr",
                        &format!("unix://{}", config::GUEST_SOCKET),
                        "debug",
                        "workers",
                        "--format",
                        "{{json .}}",
                    ]),
                    lock,
                )?;
                let (version, target) = super::inspection::daemon_facts(&bytes, "linux/arm64")?;
                details.daemon_version.observed = Fact::measured(
                    version,
                    "BuildKit debug workers in the existing owned guest",
                );
                details.target.observed =
                    Fact::measured(target, "BuildKit advertised guest worker platforms");
                if cache {
                    let bytes = self.run(
                        &args(&[
                            "shell",
                            VM_NAME,
                            "sudo",
                            "--non-interactive",
                            "du",
                            "-sk",
                            "/mnt/lima-cache/buildkit",
                        ]),
                        lock,
                    );
                    details.disk.push(WorkerDiskUsage {
                        scope: "/mnt/lima-cache/buildkit".to_owned(),
                        accounting: "guest allocated BuildKit cache bytes (1-KiB du units); live snapshot may change",
                        bytes: match bytes {
                            Ok(bytes) => Fact::measured(super::inspection::du_bytes(&bytes)?, "du -sk inside the existing owned guest"),
                            Err(error) => Fact::Unavailable(error.to_string()),
                        },
                    });
                }
            }
        }
        Ok(())
    }
    pub(super) fn ensure_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if self.verify_volume(home, record, lock)? {
            return Ok(());
        }
        self.run(
            &args(&[
                "disk", "create", CACHE_NAME, "--size", "32GiB", "--format", "raw",
            ]),
            lock,
        )?;
        let lima = self
            .owned_home(record)?
            .ok_or(WorkerError::Corrupt("Lima namespace disappeared"))?;
        let disk = gripsack_fs::open_dir_nofollow(&lima, Path::new("_disks/cache"))?;
        gripsack_fs::atomic_write(&disk, Path::new(OWNER), &owner_bytes(record))?;
        self.verify_volume(home, record, lock)?;
        Ok(())
    }
    pub(super) fn verify_volume(
        &self,
        _home: &WorkerHome,
        record: &InstanceRecord,
        _lock: &gripsack_fs::FlockGuard,
    ) -> Result<bool, WorkerError> {
        let Some(lima) = self.owned_home(record)? else {
            return Ok(false);
        };
        let disk = match gripsack_fs::open_dir_nofollow(&lima, Path::new("_disks/cache")) {
            Ok(disk) => disk,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        verify_owner(&disk, record)?;
        let data = gripsack_fs::open_file_nofollow(&disk, Path::new("datadisk"))?;
        if !data.metadata()?.is_file() {
            return Err(WorkerError::Foreign(
                "Lima cache disk is not regular".into(),
            ));
        }
        Ok(true)
    }
    pub(super) fn create(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        options: &WorkerOptions,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<ContainerId, WorkerError> {
        let lima = self
            .owned_home(record)?
            .ok_or(WorkerError::Corrupt("Lima namespace disappeared"))?;
        if lima.symlink_metadata(VM_NAME).is_ok() {
            return Err(WorkerError::Foreign("pre-existing Lima VM".into()));
        }
        let intent = VmIntent::new(home, record, options, &self.assets_path())?;
        self.store_intent(INTENT, &intent)?;
        gripsack_fs::atomic_write(
            &lima,
            Path::new("worker-input.yaml"),
            &serde_json::to_vec(&intent.configuration)?,
        )?;
        self.run(
            &[
                OsString::from("create"),
                OsString::from("--tty=false"),
                OsString::from("--name=worker"),
                self.lima_home().join("worker-input.yaml").into_os_string(),
            ],
            lock,
        )?;
        self.configuration(&lima, &intent)?;
        Ok(intent.id)
    }
    fn verify_id(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<ContainerObservation, WorkerError> {
        let observed = self
            .inspect(&self.home, lock)?
            .ok_or(WorkerError::Corrupt("owned Lima VM disappeared"))?;
        if &observed.id != id {
            return Err(WorkerError::Foreign(observed.id.as_str().into()));
        }
        Ok(observed)
    }
    pub(super) fn start(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.verify_id(id, lock)?;
        let mut intent = self.intent(&self.record()?)?;
        intent.boot_session = capabilities::boot_session_id()?.try_into()?;
        intent.lifecycle = config::VmLifecycle::MayBeRunning;
        self.store_intent(INTENT, &intent)?;
        self.run(&args(&["start", "--tty=false", VM_NAME]), lock)?;
        Ok(())
    }
    pub(super) fn validate_ready(
        &self,
        _home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.owned_home(record)?
            .ok_or(WorkerError::Corrupt("Lima namespace disappeared"))?;
        let intent = self.intent(record)?;
        let script = format!(
            r#"set -eu
systemctl is-active --quiet gripsack-buildkit.service
printf '%s  %s\n' '{daemon}' /opt/gripsack-buildkit/bin/buildkitd '{client}' /opt/gripsack-buildkit/bin/buildctl | sha256sum --status -c -
printf '{{"arch":"%s","cpus":%s,"memory_kib":' "$(uname -m)" "$(getconf _NPROCESSORS_ONLN)"
while read key value unit; do if [ "$key" = 'MemTotal:' ]; then printf '%s' "$value"; break; fi; done < /proc/meminfo
printf ',"workers":'
/opt/gripsack-buildkit/bin/buildctl --addr unix://{socket} debug workers --format '{{{{json .}}}}'
printf '}}\n'
"#,
            daemon = assets::DAEMON_SHA256,
            client = assets::CLIENT_SHA256,
            socket = config::GUEST_SOCKET
        );
        let bytes = self.run(
            &args(&[
                "shell",
                VM_NAME,
                "sudo",
                "--non-interactive",
                "sh",
                "-c",
                &script,
            ]),
            lock,
        )?;
        config::admit_guest(&bytes, &intent)
    }
    pub(super) fn stop(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.verify_id(id, lock)?;
        self.run(&args(&["stop", VM_NAME]), lock)?;
        if self.verify_id(id, lock)?.running {
            return Err(WorkerError::Corrupt("Lima stop did not terminate the VM"));
        }
        Ok(())
    }
    pub(super) fn remove_container(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if self.verify_id(id, lock)?.running {
            return Err(WorkerError::Corrupt("refusing deletion of running Lima VM"));
        }
        let mut intent = self.intent(&self.record()?)?;
        intent.lifecycle = config::VmLifecycle::Removing;
        self.store_intent(INTENT, &intent)?;
        self.run(&args(&["delete", "--force", VM_NAME]), lock)?;
        if self.inspect(&self.home, lock)?.is_some() {
            return Err(WorkerError::Corrupt("Lima VM deletion was not observed"));
        }
        intent.lifecycle = config::VmLifecycle::Removed;
        self.store_intent(INTENT, &intent)?;
        Ok(())
    }
    pub(super) fn remove_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        if !self.verify_volume(home, record, lock)? {
            return Ok(());
        }
        self.run(&args(&["disk", "delete", CACHE_NAME]), lock)?;
        if self.verify_volume(home, record, lock)? {
            return Err(WorkerError::Corrupt("Lima cache deletion was not observed"));
        }
        Ok(())
    }
}
fn args(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}
fn bounded_read(directory: &gripsack_fs::Dir, name: &Path) -> Result<Vec<u8>, WorkerError> {
    let file = gripsack_fs::open_file_nofollow(directory, name)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_METADATA_BYTES {
        return Err(WorkerError::Corrupt("Lima control state kind/size"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(WorkerError::Corrupt(
            "Lima control state grew beyond its limit",
        ));
    }
    Ok(bytes)
}
fn owner_bytes(record: &InstanceRecord) -> Vec<u8> {
    format!("{}:{}", record.home_identity, record.owner).into_bytes()
}
fn verify_owner(directory: &gripsack_fs::Dir, record: &InstanceRecord) -> Result<(), WorkerError> {
    let metadata = directory.metadata(".")?;
    // SAFETY: geteuid has no pointer or memory preconditions.
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || bounded_read(directory, Path::new(OWNER))? != owner_bytes(record)
    {
        return Err(WorkerError::Foreign(
            "Lima namespace/cache owner marker".into(),
        ));
    }
    Ok(())
}
fn owned_child(
    parent: &gripsack_fs::Dir,
    name: &str,
    record: &InstanceRecord,
) -> Result<(), WorkerError> {
    match gripsack_fs::open_dir_nofollow(parent, Path::new(name)) {
        Ok(directory) => return verify_owner(&directory, record),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
    let staging = format!("{name}-{}", Sha256Digest::of(&nonce));
    let mut builder = gripsack_fs::cap_std::fs::DirBuilder::new();
    builder.mode(0o700);
    parent.create_dir_with(&staging, &builder)?;
    let directory = gripsack_fs::open_dir_nofollow(parent, Path::new(&staging))?;
    gripsack_fs::atomic_write(&directory, Path::new(OWNER), &owner_bytes(record))?;
    gripsack_fs::rename_noreplace(parent, Path::new(&staging), Path::new(name))?;
    gripsack_fs::fsync_pinned_dir(parent, Path::new(name))?;
    Ok(())
}
