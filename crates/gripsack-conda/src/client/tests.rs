//! Real process fixtures: bytes cross supervised native stdio, not a mock
//! exchange. A correct echo is still only advisory, never a validated tree.
use super::*;
use crate::protocol::{self, ImportedResponse, MaterializedResponse, ResolvedResponse};
use gripsack_process::ProcessDisposition;
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

fn closure(platform: &str) -> LockedCondaEnvironment {
    serde_json::from_value(serde_json::json!({
        "platform": platform, "channels": ["https://example.invalid/conda"],
        "channel_priority": "strict", "system_requirements": {}, "packages": [],
        "materializer": {"bytecode": "suppress", "receipt": "normalized_conda_meta"}
    }))
    .unwrap()
}

fn resolved(attempt: u64, platform: &str) -> Response {
    Response::Resolved(ResolvedResponse {
        attempt,
        environment: closure(platform),
    })
}

fn frame(response: &Response) -> Vec<u8> {
    let mut bytes = Vec::new();
    protocol::write_response_frame(&mut bytes, response).unwrap();
    bytes
}

struct Fixture {
    directory: tempfile::TempDir,
    helper: CondaHelper,
}

impl Fixture {
    fn new(bytes: &[u8], terminal: &str, budget: Duration) -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("response"), bytes).unwrap();
        let path = directory.path().join("helper");
        // EOF must be observed before response. The PID is the supervised
        // leader, not a shell descendant used as a surrogate for cleanup.
        fs::write(&path, format!(
            "#!/bin/sh\nset -eu\nprintf '%s' \"$$\" > pid\ncat > request\ncat response\n{terminal}\n"
        )).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let helper =
            CondaHelper::select(&path, None, directory.path(), Instant::now() + budget).unwrap();
        Self { directory, helper }
    }

    fn resolve(&self) -> Result<LockedCondaEnvironment, CondaError> {
        self.helper.resolve(
            41,
            &[],
            &BTreeMap::new(),
            "linux-64",
            &[],
            &Default::default(),
        )
    }

    fn assert_reaped(&self) {
        let pid: libc::pid_t = fs::read_to_string(self.directory.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        let mut status = 0;
        // SAFETY: the fixture's leader PID and a writable status word; WNOHANG
        // cannot block. ECHILD proves native supervision already reaped it.
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        // SAFETY: signal zero observes existence without signalling a process.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}

#[test]
fn success_frame_cannot_survive_nonzero_native_exit() {
    let fixture = Fixture::new(
        &frame(&resolved(41, "linux-64")),
        "exit 23",
        Duration::from_secs(5),
    );
    match fixture.resolve().unwrap_err() {
        CondaError::Process { receipt, .. } => {
            assert_eq!(receipt.disposition, ProcessDisposition::Exited);
            assert_eq!(receipt.exit_code, Some(23));
        }
        error => panic!("unexpected failure: {error:?}"),
    }
    fixture.assert_reaped();
}

#[test]
fn success_frame_cannot_survive_truncation_trailing_or_conflicting_frames() {
    let good = frame(&resolved(41, "linux-64"));
    let mut trailing = good.clone();
    trailing.push(0);
    let mut conflicting = good.clone();
    conflicting.extend(frame(&resolved(40, "osx-arm64")));
    let mut malformed = 1u64.to_le_bytes().to_vec();
    malformed.push(b'{');
    for bytes in [
        &good[..0],
        &good[..4],
        &good[..good.len() - 1],
        trailing.as_slice(),
        conflicting.as_slice(),
        malformed.as_slice(),
    ] {
        let fixture = Fixture::new(bytes, "exit 0", Duration::from_secs(5));
        let error = fixture.resolve().unwrap_err();
        assert!(
            matches!(error, CondaError::Truncated | CondaError::Frame(_)),
            "{error:?}"
        );
    }
}

#[test]
fn success_frame_cannot_survive_stale_attempt_wrong_platform_or_operation() {
    for response in [
        resolved(40, "linux-64"),
        resolved(41, "osx-arm64"),
        Response::Imported(ImportedResponse {
            attempt: 41,
            environment: closure("linux-64"),
        }),
    ] {
        let fixture = Fixture::new(&frame(&response), "exit 0", Duration::from_secs(5));
        assert!(matches!(fixture.resolve(), Err(CondaError::Echo(_))));
    }
    for (attempt, platform) in [(40, "linux-64"), (41, "osx-arm64")] {
        let response = Response::Imported(ImportedResponse {
            attempt,
            environment: closure(platform),
        });
        let fixture = Fixture::new(&frame(&response), "exit 0", Duration::from_secs(5));
        assert!(matches!(
            fixture
                .helper
                .import_pixi(41, "", "", "default", "linux-64"),
            Err(CondaError::Echo(_))
        ));
    }
}

#[test]
fn oversized_or_wrong_version_frame_never_admits_success() {
    let oversize = (protocol::MAX_RESPONSE_BYTES + 1).to_le_bytes();
    let fixture = Fixture::new(&oversize, "exit 0", Duration::from_secs(5));
    assert!(matches!(
        fixture.resolve(),
        Err(CondaError::Oversize { .. })
    ));
    let mut bytes = frame(&resolved(41, "linux-64"));
    let offset = bytes
        .windows(b"\"protocol\":3".len())
        .position(|part| part == b"\"protocol\":3")
        .unwrap();
    bytes[offset + b"\"protocol\":".len()] = b'1';
    let fixture = Fixture::new(&bytes, "exit 0", Duration::from_secs(5));
    assert!(matches!(fixture.resolve(), Err(CondaError::Frame(_))));
}

#[test]
fn stdout_limit_after_valid_success_remains_terminal_failure() {
    let mut bytes = frame(&resolved(41, "linux-64"));
    bytes.resize(
        protocol::MAX_RESPONSE_BYTES as usize + protocol::FRAME_HEADER_BYTES + 1,
        b'x',
    );
    let fixture = Fixture::new(&bytes, "exit 0", Duration::from_secs(5));
    match fixture.resolve().unwrap_err() {
        CondaError::Process { receipt, .. } => {
            assert_eq!(receipt.disposition, ProcessDisposition::StdoutLimit)
        }
        error => panic!("unexpected failure: {error:?}"),
    }
    fixture.assert_reaped();
}

#[test]
fn bounded_deadline_kills_and_reaps_helper_even_after_success() {
    let start = Instant::now();
    let fixture = Fixture::new(
        &frame(&resolved(41, "linux-64")),
        "trap '' TERM\nwhile :; do :; done",
        Duration::from_millis(600),
    );
    match fixture.resolve().unwrap_err() {
        CondaError::Process { receipt, .. } => {
            assert_eq!(receipt.disposition, ProcessDisposition::Deadline);
            assert_eq!(receipt.signal, Some(libc::SIGKILL));
        }
        error => panic!("unexpected failure: {error:?}"),
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    fixture.assert_reaped();
}

#[test]
fn materialize_binds_full_closure_platform_prefix_attempt_and_operation() {
    let environment = closure("linux-64");
    let digest = conda_closure_digest(&environment).to_string();
    let mut other_closure = environment.clone();
    other_closure
        .channels
        .push("https://example.invalid/other".into());
    let other_digest = conda_closure_digest(&other_closure).to_string();
    for (attempt, lock_digest, platform, final_prefix) in [
        (40, digest.as_str(), "linux-64", "/final"),
        (41, other_digest.as_str(), "linux-64", "/final"),
        (41, digest.as_str(), "osx-arm64", "/final"),
        (41, digest.as_str(), "linux-64", "/other"),
    ] {
        let response = Response::Materialized(MaterializedResponse {
            attempt,
            lock_digest: lock_digest.into(),
            platform: platform.into(),
            final_prefix: final_prefix.into(),
            packages: vec![],
        });
        let fixture = Fixture::new(&frame(&response), "exit 0", Duration::from_secs(5));
        assert!(matches!(
            fixture.helper.materialize(
                41,
                &environment,
                Path::new("/final"),
                Path::new("/staging"),
                &[]
            ),
            Err(CondaError::Echo(_))
        ));
    }
    let fixture = Fixture::new(
        &frame(&resolved(41, "linux-64")),
        "exit 0",
        Duration::from_secs(5),
    );
    assert!(matches!(
        fixture.helper.materialize(
            41,
            &environment,
            Path::new("/final"),
            Path::new("/staging"),
            &[]
        ),
        Err(CondaError::Echo(_))
    ));
}
