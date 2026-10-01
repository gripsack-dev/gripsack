//! VM declaration and observed guest/worker admission, separate from lifecycle effects.
use super::{assets, PINNED_WORKER_IMAGE};
use super::super::{home::{InstanceRecord, WorkerHome}, observation::{ContainerId, ContainerObservation, nano_cpus}, WorkerError, WorkerOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, num::NonZeroU16, path::Path};

pub(super) const VM_NAME: &str = "worker";
pub(super) const CACHE_NAME: &str = "cache";
pub(super) const GUEST_SOCKET: &str = "/run/gripsack/buildkitd.sock";
pub(super) const MINIMUM_VM_MEMORY: u64 = 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VmIntent {
    pub id: ContainerId,
    pub owner: crate::identity::WorkerOwnerId,
    pub home: crate::identity::WorkerHomeId,
    pub image: String,
    pub lima_version: String,
    pub cpus: NonZeroU16,
    pub memory: u64,
    pub configuration: Value,
}
impl VmIntent {
    pub fn new(home: &WorkerHome, record: &InstanceRecord, options: &WorkerOptions, assets: &Path) -> Result<Self, WorkerError> {
        super::validate_options(options)?;
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(std::io::Error::other)?;
        let id = ContainerId::try_from(gripsack_process::Sha256Digest::of(&nonce).to_string())?;
        let owner = record.owner.to_string();
        let identity = home.identity.to_string();
        let script = format!(r#"#!/bin/sh
set -eu
printf '%s  %s\n' '{archive}' /mnt/gripsack-tools/buildkit.tar.gz | sha256sum -c -
install -d -m 0755 /opt/gripsack-buildkit
# Input is the verified upstream release, never a recipe archive.
tar -xzf /mnt/gripsack-tools/buildkit.tar.gz -C /opt/gripsack-buildkit
printf '%s  %s\n' '{daemon}' /opt/gripsack-buildkit/bin/buildkitd '{client}' /opt/gripsack-buildkit/bin/buildctl | sha256sum -c -
cat > /etc/systemd/system/gripsack-buildkit.service <<'UNIT'
[Unit]
Description=Gripsack owned BuildKit worker
After=local-fs.target
RequiresMountsFor=/mnt/lima-cache
[Service]
Type=simple
Environment=PATH=/opt/gripsack-buildkit/bin:/usr/sbin:/usr/bin:/sbin:/bin
RuntimeDirectory=gripsack
RuntimeDirectoryMode=0755
ExecStart=/opt/gripsack-buildkit/bin/buildkitd --root /mnt/lima-cache/buildkit --addr unix://{socket} --group gripsack --containerd-worker=false --oci-worker=true --oci-worker-platform linux/arm64 --oci-worker-label dev.gripsack.owner={owner} --oci-worker-label dev.gripsack.home={identity} --oci-worker-label dev.gripsack.instance={id}
KillMode=control-group
[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now gripsack-buildkit.service
"#, archive=assets::BUILDKIT.sha256, daemon=assets::DAEMON_SHA256, client=assets::CLIENT_SHA256, socket=GUEST_SOCKET, id=id.as_str());
        let configuration = json!({
            "minimumLimaVersion": "2.0.3", "vmType": "vz", "arch": "aarch64",
            "cpus": options.cpus.get(), "memory": format!("{}MiB", options.memory.bytes() / (1024*1024)), "disk": "8GiB",
            "images": [{"location": assets.join("ubuntu.img"), "arch": "aarch64", "digest": format!("sha256:{}", assets::GUEST.sha256)}],
            "mountType": "virtiofs",
            "mounts": [{"location": assets.join("guest-tools"), "mountPoint": "/mnt/gripsack-tools", "writable": false}],
            "additionalDisks": [{"name": CACHE_NAME, "format": true, "fsType": "ext4"}],
            "containerd": {"system": false, "user": false},
            "user": {"name": "gripsack", "uid": 1000, "home": "/home/gripsack"},
            "ssh": {"loadDotSSHPubKeys": false, "forwardAgent": false, "forwardX11": false},
            "rosetta": {"enabled": false, "binfmt": false},
            "propagateProxyEnv": false,
            "portForwards": [{"guestSocket": GUEST_SOCKET, "hostSocket": home.socket()}, {"guestIP": "0.0.0.0", "proto": "any", "ignore": true}],
            "provision": [
                {"mode": "dependency", "skipDefaultDependencyResolution": true, "script": "#!/bin/sh\nset -eu\ncommand -v systemctl\ncommand -v tar\ncommand -v sha256sum\n"},
                {"mode": "system", "script": script}
            ],
        });
        Ok(Self { id, owner: record.owner, home: home.identity, image: PINNED_WORKER_IMAGE.to_owned(), lima_version: "v2.0.3".to_owned(), cpus: options.cpus, memory: options.memory.bytes(), configuration })
    }
    pub fn verify_owner(&self, home: &WorkerHome, record: &InstanceRecord) -> Result<(), WorkerError> {
        if self.owner != record.owner || self.home != home.identity {
            return Err(WorkerError::Foreign("Lima instance incarnation".to_owned()));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VmObservation {
    pub name: String,
    pub dir: String,
    pub status: String,
    pub arch: String,
    pub vm_type: String,
    pub cpus: u16,
    pub memory: u64,
    pub lima_version: String,
    pub errors: Option<Vec<Value>>,
}
pub(super) const INSPECT_FORMAT: &str = r#"{"name":{{json .Name}},"dir":{{json .Dir}},"status":{{json .Status}},"arch":{{json .Arch}},"vm_type":{{json .VMType}},"cpus":{{json .CPUs}},"memory":{{json .Memory}},"lima_version":{{json .LimaVersion}},"errors":{{json .Errors}}}"#;
impl VmObservation {
    pub fn admit(self, intent: &VmIntent, directory: &Path) -> Result<ContainerObservation, WorkerError> {
        if self.name != VM_NAME || Path::new(&self.dir) != directory || self.arch != "aarch64" || self.vm_type != "vz"
            || self.cpus != intent.cpus.get() || self.memory != intent.memory || self.lima_version != intent.lima_version
            || self.errors.is_some_and(|errors| !errors.is_empty()) || !matches!(self.status.as_str(), "Running" | "Stopped") {
            return Err(WorkerError::Corrupt("Lima VM metadata, resources or liveness disagree with its owned intent"));
        }
        Ok(ContainerObservation { id: intent.id.clone(), running: self.status == "Running", image: intent.image.clone(), nano_cpus: nano_cpus(intent.cpus), memory: intent.memory,
            labels: Some(BTreeMap::from([("dev.gripsack.owner".into(), intent.owner.to_string()), ("dev.gripsack.home".into(), intent.home.to_string())])) })
    }
}

pub(super) fn admit_guest(bytes: &[u8], intent: &VmIntent) -> Result<(), WorkerError> {
    let value: Value = serde_json::from_slice(bytes)?;
    let memory = value["memory_kib"].as_u64().and_then(|v| v.checked_mul(1024));
    // Firmware/kernel reserved RAM is not MemTotal. The VM allocation itself is
    // exact above; guest usable memory must be within the bounded reservation.
    const MAX_RESERVED_MEMORY: u64 = 256 * 1024 * 1024;
    if value["arch"] != "aarch64" || value["cpus"].as_u64() != Some(intent.cpus.get() as u64)
        || !memory.is_some_and(|m| m <= intent.memory && intent.memory - m <= MAX_RESERVED_MEMORY) {
        return Err(WorkerError::Corrupt("running guest architecture/resources disagree with VM allocation"));
    }
    let workers = value["workers"].as_array().ok_or(WorkerError::Corrupt("BuildKit omitted worker metadata"))?;
    if workers.len() != 1 { return Err(WorkerError::Corrupt("owned VM must expose exactly one BuildKit worker")); }
    let worker = &workers[0];
    let labels = &worker["labels"];
    if labels["dev.gripsack.owner"] != intent.owner.to_string() || labels["dev.gripsack.home"] != intent.home.to_string()
        || labels["dev.gripsack.instance"] != intent.id.as_str() || worker["buildkitVersion"]["version"] != "v0.33.0"
        || !worker["platforms"].as_array().is_some_and(|platforms| platforms.iter().any(|p| p["os"] == "linux" && p["architecture"] == "arm64")) {
        return Err(WorkerError::Corrupt("running BuildKit identity/version/platform differs from its owned VM"));
    }
    Ok(())
}
