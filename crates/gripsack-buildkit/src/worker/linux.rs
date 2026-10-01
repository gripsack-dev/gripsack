//! The one qualified local Linux effect boundary. No ambient Docker context,
//! host network entitlement, broad HOME mount or process-sandbox workaround.
use super::{
    WorkerError,
    home::{InstanceRecord, WorkerHome},
    observation::{ContainerId, ContainerObservation, nano_cpus},
};
use gripsack_process::{
    Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessLeases, ProcessRole,
    SelectedProgram, StdoutByteLimit,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Instant,
};

pub const PINNED_WORKER_IMAGE: &str =
    "moby/buildkit:v0.33.0@sha256:6c2fa84a6b61ccd72899dde4239f8d5717f05f9a8ca6f3cad185fb1a95a94de3";
const DOCKER_SOCKET: &str = "unix:///var/run/docker.sock";
const MAX_DOCKER_OUTPUT: usize = 1024 * 1024;
const OWNER_LABEL: &str = "dev.gripsack.owner";
const HOME_LABEL: &str = "dev.gripsack.home";
const PROFILE_LABEL: &str = "dev.gripsack.profile";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeObservation {
    name: String,
    /// Docker emits null when a volume has no labels; absent labels are
    /// NEVER owned.
    labels: Option<BTreeMap<String, String>>,
}

pub(super) struct Docker<'a> {
    environment: &'a OperatorEnvironment,
    program: SelectedProgram,
    config: PathBuf,
    directory: PathBuf,
    deadline: Instant,
}
pub(super) type Provider<'a> = Docker<'a>;
impl<'a> Docker<'a> {
    pub(super) fn new(
        environment: &'a OperatorEnvironment,
        home: &WorkerHome,
        deadline: Instant,
    ) -> Result<Self, WorkerError> {
        let program = SelectedProgram::select(environment, Path::new("docker"), None, deadline)?;
        Ok(Self {
            environment,
            program,
            config: home.root.join("docker-config"),
            directory: home.root.clone(),
            deadline,
        })
    }
    fn invoke(
        &self,
        arguments: &[OsString],
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(gripsack_process::NativeOutcome, Vec<u8>), WorkerError> {
        let limits = Limits {
            timeout: self.deadline.saturating_duration_since(Instant::now()),
            operation_deadline: Some(self.deadline),
            stdout_bytes: StdoutByteLimit::new(MAX_DOCKER_OUTPUT as u64),
            ..Limits::default()
        };
        let invocation = Invocation::admit(
            self.environment,
            ProcessRole::Build,
            &self.program,
            &self.directory,
            limits,
        )?
        .retain_leases(ProcessLeases {
            worker: Some(lock.duplicate_handle()?),
            retention: None,
        })?;
        let mut argv = Vec::with_capacity(arguments.len() + 4);
        argv.extend([
            OsStr::new("--host"),
            OsStr::new(DOCKER_SOCKET),
            OsStr::new("--config"),
            self.config.as_os_str(),
        ]);
        argv.extend(arguments.iter().map(OsString::as_os_str));
        let mut stdout = Vec::new();
        let outcome = invocation.run(&argv, NativeInput::Bytes(b""), None, |bytes| {
            if bytes.len() <= MAX_DOCKER_OUTPUT.saturating_sub(stdout.len()) {
                stdout.extend_from_slice(bytes);
            }
            Control::Continue
        })?;
        Ok((outcome, stdout))
    }
    fn run(
        &self,
        arguments: &[OsString],
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Vec<u8>, WorkerError> {
        let (outcome, stdout) = self.invoke(arguments, lock)?;
        if !outcome.success {
            return Err(WorkerError::Effect(
                String::from_utf8_lossy(&outcome.stderr).into_owned(),
            ));
        }
        Ok(stdout)
    }
    pub(super) fn provision_image(
        &self,
        offline: bool,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let (outcome, _) = self.invoke(
            &args(&[
                "image",
                "inspect",
                "--format",
                "{{.Id}}",
                PINNED_WORKER_IMAGE,
            ]),
            lock,
        )?;
        if outcome.success {
            return Ok(());
        }
        let missing = outcome.receipt.exit_code == Some(1)
            && outcome.receipt.disposition == gripsack_process::ProcessDisposition::Exited
            && String::from_utf8_lossy(&outcome.stderr)
                .trim_start()
                .starts_with("Error: No such image:");
        if !missing {
            return Err(WorkerError::Effect(
                String::from_utf8_lossy(&outcome.stderr).into_owned(),
            ));
        }
        if offline {
            return Err(WorkerError::OfflineInput(PINNED_WORKER_IMAGE));
        }
        self.run(&args(&["pull", PINNED_WORKER_IMAGE]), lock)?;
        Ok(())
    }
    pub(super) fn inspect(
        &self,
        home: &WorkerHome,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<Option<ContainerObservation>, WorkerError> {
        let names = self.run(
            &args(&[
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--filter",
                &format!("name=^/{}$", home.resource_name),
                "--format",
                "{{.ID}}",
            ]),
            lock,
        )?;
        let names = std::str::from_utf8(&names)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 identity"))?
            .trim();
        if names.is_empty() {
            return Ok(None);
        }
        let id = ContainerId::try_from(names.to_owned())?;
        let format = r#"{"id":{{json .Id}},"running":{{json .State.Running}},"image":{{json .Config.Image}},"nano_cpus":{{json .HostConfig.NanoCpus}},"memory":{{json .HostConfig.Memory}},"labels":{{json .Config.Labels}}}"#;
        let bytes = self.run(
            &args(&["container", "inspect", "--format", format, id.as_str()]),
            lock,
        )?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
    /// The recorded durable binding is the authority: image, owned labels
    /// AND the resource limits. An unbound (legacy) record skips the
    /// resource comparison here and is re-bound from this verified
    /// observation by the caller.
    pub(super) fn verify_container(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        observed: &ContainerObservation,
    ) -> Result<(), WorkerError> {
        let resources_match = match (record.cpus, record.memory) {
            (Some(cpus), Some(memory)) => {
                observed.nano_cpus == nano_cpus(cpus) && observed.memory == memory
            }
            (None, None) => true,
            _ => return Err(WorkerError::Corrupt("worker resource binding is partial")),
        };
        if observed.image != record.image
            || !resources_match
            || !observed
                .labels
                .as_ref()
                .is_some_and(|labels| owned_labels(labels, home, record))
        {
            return Err(WorkerError::Foreign(observed.id.as_str().to_owned()));
        }
        Ok(())
    }
    pub(super) fn ensure_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        let name = home.cache_volume();
        if self.verify_volume(home, record, lock)? {
            return Ok(());
        }
        let mut arguments = args(&["volume", "create"]);
        add_labels(&mut arguments, home, record);
        arguments.push(name.into());
        self.run(&arguments, lock)?;
        if !self.verify_volume(home, record, lock)? {
            return Err(WorkerError::Corrupt("created worker volume disappeared"));
        }
        Ok(())
    }
    pub(super) fn verify_volume(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<bool, WorkerError> {
        let name = home.cache_volume();
        let found = self.run(
            &args(&[
                "volume",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ]),
            lock,
        )?;
        let found = std::str::from_utf8(&found)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 volume name"))?
            .trim();
        if found.is_empty() {
            return Ok(false);
        }
        if found != name {
            return Err(WorkerError::Foreign(found.to_owned()));
        }
        let bytes = self.run(
            &args(&[
                "volume",
                "inspect",
                "--format",
                r#"{"name":{{json .Name}},"labels":{{json .Labels}}}"#,
                &name,
            ]),
            lock,
        )?;
        let volume: VolumeObservation = serde_json::from_slice(&bytes)?;
        let owned = volume
            .labels
            .as_ref()
            .is_some_and(|labels| owned_labels(labels, home, record));
        if volume.name != name || !owned {
            return Err(WorkerError::Foreign(name));
        }
        Ok(true)
    }
    pub(super) fn create(
        &self,
        home: &WorkerHome,
        record: &InstanceRecord,
        options: &super::WorkerOptions,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<ContainerId, WorkerError> {
        let mut arguments = args(&[
            "container",
            "create",
            "--pull=never",
            "--name",
            &home.resource_name,
            "--privileged",
            "--cpus",
            &options.cpus.get().to_string(),
            "--memory",
            &options.memory.bytes().to_string(),
            "--mount",
            &format!(
                "type=volume,source={},target=/var/lib/buildkit",
                home.cache_volume()
            ),
            "--mount",
            &format!(
                "type=bind,source={},target=/run/gripsack",
                home.run_dir().display()
            ),
        ]);
        add_labels(&mut arguments, home, record);
        // SAFETY: getegid has no pointer/memory preconditions.
        let group = unsafe { libc::getegid() }.to_string();
        arguments.extend(args(&[
            PINNED_WORKER_IMAGE,
            "--addr",
            "unix:///run/gripsack/buildkitd.sock",
            "--group",
            &group,
            "--containerd-worker=false",
            "--oci-worker-platform",
            "linux/amd64",
            "--oci-worker-net=bridge",
            "--cdi-disabled",
        ]));
        let output = self.run(&arguments, lock)?;
        let id = std::str::from_utf8(&output)
            .map_err(|_| WorkerError::Corrupt("Docker emitted non-UTF-8 container ID"))?
            .trim();
        Ok(ContainerId::try_from(id.to_owned())?)
    }
    pub(super) fn start(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(&args(&["container", "start", id.as_str()]), lock)?;
        Ok(())
    }
    pub(super) fn stop(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(
            &args(&["container", "stop", "--timeout", "10", id.as_str()]),
            lock,
        )?;
        Ok(())
    }
    pub(super) fn remove_container(
        &self,
        id: &ContainerId,
        lock: &gripsack_fs::FlockGuard,
    ) -> Result<(), WorkerError> {
        self.run(&args(&["container", "rm", id.as_str()]), lock)?;
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
        self.run(&args(&["volume", "rm", &home.cache_volume()]), lock)?;
        Ok(())
    }
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn owned_labels(
    labels: &BTreeMap<String, String>,
    home: &WorkerHome,
    record: &InstanceRecord,
) -> bool {
    labels
        .get(OWNER_LABEL)
        .is_some_and(|value| value == &record.owner.to_string())
        && labels
            .get(HOME_LABEL)
            .is_some_and(|value| value == &home.identity.to_string())
        && labels
            .get(PROFILE_LABEL)
            .is_some_and(|value| value == home.profile.as_str())
}
fn add_labels(arguments: &mut Vec<OsString>, home: &WorkerHome, record: &InstanceRecord) {
    for (key, value) in [
        (OWNER_LABEL, record.owner.to_string()),
        (HOME_LABEL, home.identity.to_string()),
        (PROFILE_LABEL, home.profile.as_str().to_owned()),
    ] {
        arguments.push("--label".into());
        arguments.push(format!("{key}={value}").into());
    }
}
