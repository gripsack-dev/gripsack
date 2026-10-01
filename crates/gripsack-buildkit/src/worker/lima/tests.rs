use super::*;
use super::super::{CleanupConfirmation, MemoryBytes, OwnedWorker, WorkerProfile};
use crate::{identity::{AttemptId, AttemptIdentity, FenceEpoch, SessionId}, plan::{Architecture, BuildPlan, ExporterPlan, LinuxOs, Node, NodeIndex, Platform, ValidatedBuildPlan}, transport::{Bridge, ExportPaths}};
use serde_json::json;
use std::{num::NonZeroU16, os::unix::fs::DirBuilderExt, time::Duration};

fn fixture() -> (tempfile::TempDir, WorkerHome, InstanceRecord) {
    let root = tempfile::tempdir().unwrap();
    let home = WorkerHome::open(root.path(), WorkerProfile::parse("lima-test").unwrap()).unwrap();
    let record = InstanceRecord::fresh(&home).unwrap();
    (root, home, record)
}

#[test]
fn inadmissible_vm_memory_fails_before_creating_worker_state() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("not-created");
    let environment = OperatorEnvironment::capture().unwrap();
    for memory in [512 * 1024 * 1024, 1024 * 1024 * 1024 + 1] {
        let result = OwnedWorker::open(
            &home, WorkerProfile::parse("memory-test").unwrap(), &environment,
            WorkerOptions { memory: MemoryBytes::new(memory).unwrap(), ..WorkerOptions::default() },
            Instant::now() + Duration::from_secs(30),
        );
        assert!(matches!(result, Err(WorkerError::Invalid(_))));
        assert!(!home.exists(), "invalid VM request created private state");
    }
}

#[test]
fn foreign_namespace_and_cache_markers_never_become_authority() {
    let (_root, home, record) = fixture();
    let parent = super::super::home::private_child(&home.dir, Path::new("marker-tests")).unwrap();
    parent.create_dir("foreign").unwrap();
    assert!(owned_child(&parent, "foreign", &record).is_err());
    assert!(parent.entries().unwrap().all(|e| e.unwrap().file_name() == "foreign"));
    owned_child(&parent, "owned", &record).unwrap();
    let owned = gripsack_fs::open_dir_nofollow(&parent, Path::new("owned")).unwrap();
    let another = InstanceRecord::fresh(&home).unwrap();
    assert!(verify_owner(&owned, &another).is_err());
    assert!(verify_owner(&owned, &record).is_ok());
}

#[test]
fn vm_inventory_rejects_foreign_architecture_resources_and_broken_liveness() {
    let (_root, home, record) = fixture();
    let intent = VmIntent::new(&home, &record, &WorkerOptions::default(), &home.root).unwrap();
    let path = home.run.join("l/worker");
    let good = json!({
        "name": "worker", "dir": path, "status": "Running", "arch": "aarch64",
        "vm_type": "vz", "cpus": 2, "memory": intent.memory,
        "lima_version": "v2.0.3", "errors": null
    });
    let admitted: VmObservation = serde_json::from_value(good.clone()).unwrap();
    assert!(admitted.admit(&intent, &path).unwrap().running);
    for (field, bad) in [
        ("name", json!("foreign")), ("dir", json!("/tmp/foreign")),
        ("status", json!("Broken")), ("arch", json!("x86_64")),
        ("vm_type", json!("qemu")), ("cpus", json!(4)),
        ("memory", json!(1024)), ("errors", json!(["driver is not running"])),
    ] {
        let mut observed = good.clone();
        observed[field] = bad;
        let observed: VmObservation = serde_json::from_value(observed).unwrap();
        assert!(observed.admit(&intent, &path).is_err(), "accepted {field}");
    }
}

#[test]
fn guest_identity_architecture_version_and_resources_are_not_config_claims() {
    let (_root, home, record) = fixture();
    let intent = VmIntent::new(&home, &record, &WorkerOptions::default(), &home.root).unwrap();
    let good = json!({"arch":"aarch64", "cpus":2, "memory_kib":(intent.memory-128*1024*1024)/1024,
        "workers":[{"labels":{"dev.gripsack.owner":record.owner.to_string(), "dev.gripsack.home":home.identity.to_string(), "dev.gripsack.instance":intent.id.as_str()},
        "platforms":[{"os":"linux","architecture":"arm64"}], "buildkitVersion":{"version":"v0.33.0"}}]});
    config::admit_guest(&serde_json::to_vec(&good).unwrap(), &intent).unwrap();
    for (pointer, bad) in [
        ("/arch", json!("x86_64")), ("/cpus", json!(1)), ("/memory_kib", json!(1024)),
        ("/workers/0/labels/dev.gripsack.owner", json!("foreign")),
        ("/workers/0/labels/dev.gripsack.instance", json!("stale")),
        ("/workers/0/buildkitVersion/version", json!("v0.32.0")),
        ("/workers/0/platforms/0/architecture", json!("amd64")),
    ] {
        let mut observed = good.clone();
        *observed.pointer_mut(pointer).unwrap() = bad;
        assert!(config::admit_guest(&serde_json::to_vec(&observed).unwrap(), &intent).is_err(), "accepted {pointer}");
    }
}

#[test]
fn persisted_vm_configuration_cannot_redirect_mounts_or_resources() {
    let (_root, home, record) = fixture();
    let intent = VmIntent::new(&home, &record, &WorkerOptions::default(), &home.root).unwrap();
    let environment = OperatorEnvironment::capture().unwrap();
    let provider = Provider::new(&environment, &home, Instant::now()+Duration::from_secs(30)).unwrap();
    let lima = super::super::home::private_child(&home.dir, Path::new("configuration-tests")).unwrap();
    lima.create_dir("worker").unwrap();
    for pointer in ["/cpus", "/mounts/0/location", "/provision/1/script"] {
        let mut observed = intent.configuration.clone();
        *observed.pointer_mut(pointer).unwrap() = json!("foreign");
        gripsack_fs::atomic_write(&lima, Path::new("worker/lima.yaml"), &serde_json::to_vec(&observed).unwrap()).unwrap();
        assert!(provider.configuration(&lima, &intent).is_err(), "accepted {pointer}");
    }
    gripsack_fs::atomic_write(&lima, Path::new("worker/lima.yaml"), &serde_json::to_vec(&intent.configuration).unwrap()).unwrap();
    provider.configuration(&lima, &intent).unwrap();
}

/// This test is deliberately not an availability skip. A runner without working
/// Virtualization.framework fails acquisition and cannot claim qualification.
#[test]
#[ignore = "requires native Apple Silicon virtualization and GRIPSACK_BUILDKIT_TEST_BRIDGE"]
fn owned_lima_checked_solve_cache_and_lifecycle() {
    let bridge_path = std::env::var_os("GRIPSACK_BUILDKIT_TEST_BRIDGE").expect("matching native Go bridge is required");
    let (root, home, _) = fixture();
    // Unknown failures retain all control state for inspection/reconciliation.
    // Only observed successful stop and cache removal authorize fixture cleanup.
    let root = std::mem::ManuallyDrop::new(root);
    println!("LIMA_QUALIFICATION_HOME={}", root.path().display());
    let environment = OperatorEnvironment::capture().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1200);
    let options = WorkerOptions::default();
    let worker = OwnedWorker::open(root.path(), home.profile.clone(), &environment, options, deadline).unwrap();
    let provider = Provider::new(&environment, &home, deadline).unwrap();
    let selected = SelectedProgram::select(&environment, Path::new(&bridge_path), None, deadline).unwrap();
    let bridge = Bridge::new(&environment, &selected, root.path(), deadline);
    let inputs = root.path().join("inputs");
    std::fs::DirBuilder::new().mode(0o700).create(&inputs).unwrap();
    let plan = ValidatedBuildPlan::admit(BuildPlan {
        platform: Platform { os: LinuxOs::Linux, architecture: Architecture::Arm64 },
        nodes: vec![Node::Directory { input: None, path: "/payload".into(), mode: 0o755 }, Node::File { input: Some(NodeIndex::new(0).unwrap()), path: "/payload/value".into(), data: b"native Mac owned Linux solve\n".to_vec(), mode: 0o444 }],
        root: NodeIndex::new(1).unwrap(), exporter: ExporterPlan::Local,
    }).unwrap();
    let mut first_instance = None;
    let mut first_cache = serde_json::Value::Null;
    for attempt in 1..=2 {
        let lease = worker.acquire().expect("native VZ worker acquisition must actually run");
        if let Some(id) = &first_instance { assert_eq!(id, lease.instance_id()); } else { first_instance = Some(lease.instance_id().to_owned()); }
        let identity = AttemptIdentity { session: SessionId::new("native-mac-lima").unwrap(), attempt: AttemptId::new(attempt).unwrap(), epoch: FenceEpoch::new(attempt).unwrap() };
        let checked = bridge.lower(&plan, &identity, None).unwrap();
        let output = root.path().join(format!("output-{attempt}"));
        std::fs::DirBuilder::new().mode(0o700).create(&output).unwrap();
        let result = bridge.execute(checked, &identity, ExportPaths { worker: &lease, retention: None, inputs: &inputs, destination: &output }, |_,_,_,_| {}).unwrap();
        assert_eq!(std::fs::read(result.destination().join("payload/value")).unwrap(), b"native Mac owned Linux solve\n");
        let lock = home.lock(deadline).unwrap();
        let bytes = provider.run(&args(&["shell", VM_NAME, "sudo", "--non-interactive", "/opt/gripsack-buildkit/bin/buildctl", "--addr", "unix:///run/gripsack/buildkitd.sock", "du", "--format", "{{json .}}"]), &lock).unwrap();
        let cache: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if attempt == 1 { first_cache = cache; } else {
            assert!(first_cache.as_array().unwrap().iter().any(|before| cache.as_array().unwrap().iter().any(|after| before["id"] == after["id"] && after["usageCount"].as_u64() > before["usageCount"].as_u64())), "second checked solve did not reuse a retained cache record");
        }
        drop(lock);
        assert!(matches!(worker.stop(), Err(WorkerError::LiveLeases(1))));
        let upgraded = OwnedWorker::open(root.path(), home.profile.clone(), &environment, WorkerOptions { cpus: NonZeroU16::new(3).unwrap(), ..options }, deadline).unwrap();
        assert!(matches!(upgraded.acquire(), Err(WorkerError::LiveLeases(1))));
        worker.release(lease, CleanupConfirmation::Confirmed).unwrap();
        assert!(!worker.status().unwrap().running);
    }
    let upgraded = OwnedWorker::open(root.path(), home.profile.clone(), &environment, WorkerOptions { cpus: NonZeroU16::new(3).unwrap(), memory: MemoryBytes::new(3*1024*1024*1024).unwrap(), ..options }, deadline).unwrap();
    let lease = upgraded.acquire().unwrap();
    assert_ne!(Some(lease.instance_id()), first_instance.as_deref());
    // A live daemon death must not be accepted merely because the VM still runs.
    let lock = home.lock(deadline).unwrap();
    provider.run(&args(&["shell", VM_NAME, "sudo", "--non-interactive", "systemctl", "stop", "gripsack-buildkit.service"]), &lock).unwrap();
    drop(lock);
    assert!(upgraded.status().is_err(), "VM liveness hid a dead BuildKit daemon");
    assert!(upgraded.acquire().is_err());
    upgraded.release(lease, CleanupConfirmation::Confirmed).unwrap();
    upgraded.stop().unwrap();
    let record = home.load().unwrap().unwrap();
    let lima = provider.owned_home(&record).unwrap().unwrap();
    let disk = gripsack_fs::open_dir_nofollow(&lima, Path::new("_disks/cache")).unwrap();
    gripsack_fs::atomic_write(&disk, Path::new(OWNER), b"foreign").unwrap();
    assert!(upgraded.cache_cleanup().is_err());
    assert!(disk.symlink_metadata("datadisk").unwrap().is_file());
    gripsack_fs::atomic_write(&disk, Path::new(OWNER), &owner_bytes(&record)).unwrap();
    upgraded.cache_cleanup().unwrap();
    assert!(provider.inspect(&home, &home.lock(deadline).unwrap()).unwrap().is_none());
    assert!(!provider.verify_volume(&home, &record, &home.lock(deadline).unwrap()).unwrap());
    assert_eq!(std::fs::read(root.path().join("output-1/payload/value")).unwrap(), b"native Mac owned Linux solve\n");
    std::fs::remove_dir_all(&home.run).unwrap();
    std::mem::ManuallyDrop::into_inner(root).close().unwrap();
    println!("NATIVE_MAC_LIMA_CHECKED_SOLVE_CACHE_REUSE_STOP_CLEAN_AND_NEGATIVES=passed");
}
