use super::{Bridge, TransportError};
use crate::{
    identity::{AttemptId, AttemptIdentity, FenceEpoch, SessionId, WorkerInstanceId},
    plan::{
        Architecture, BuildPlan, ExporterPlan, LinuxOs, Node, NodeIndex, Platform,
        ValidatedBuildPlan,
    },
    protocol::{self, FailureCode, FromBridge, ProtocolError, WorkerBinding},
};
use gripsack_process::{OperatorEnvironment, SelectedProgram};
use std::{
    fmt::Write as _,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};

fn identity() -> AttemptIdentity {
    AttemptIdentity {
        session: SessionId::new("hostile-peer").unwrap(),
        attempt: AttemptId::new(2).unwrap(),
        epoch: FenceEpoch::new(3).unwrap(),
    }
}
fn rejection(identity: &AttemptIdentity, message: &str) -> FromBridge {
    FromBridge::Failed {
        session: identity.session.clone(),
        attempt: identity.attempt,
        epoch: identity.epoch,
        worker: None,
        code: FailureCode::Rejected,
        message: message.into(),
        vertices: Vec::new(),
    }
}
fn lower_from_peer(frames: &[FromBridge], suffix: &[u8], status: i32) -> TransportError {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("peer");
    let mut script = String::from("#!/bin/sh\nprintf '");
    for frame in frames {
        for byte in protocol::encode_frame(frame).unwrap() {
            write!(&mut script, "\\{byte:03o}").unwrap();
        }
    }
    for byte in suffix {
        write!(&mut script, "\\{byte:03o}").unwrap();
    }
    writeln!(&mut script, "'\nexit {status}").unwrap();
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let environment = OperatorEnvironment::capture().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let selected = SelectedProgram::select(&environment, &path, None, deadline).unwrap();
    let bridge = Bridge::new(&environment, &selected, directory.path(), deadline);
    let plan = ValidatedBuildPlan::admit(BuildPlan {
        platform: Platform {
            os: LinuxOs::Linux,
            architecture: Architecture::Amd64,
        },
        nodes: vec![Node::File {
            input: None,
            path: "/value".into(),
            data: b"value".to_vec(),
            mode: 0o444,
        }],
        root: NodeIndex::new(0).unwrap(),
        exporter: ExporterPlan::Local,
    })
    .unwrap();
    bridge
        .lower(&plan, &identity(), None)
        .expect_err("hostile peer acquired checked definition authority")
}

#[test]
fn equal_control_replay_is_idempotent_but_changed_terminal_is_not() {
    let first = rejection(&identity(), "first refusal");
    assert!(matches!(
        lower_from_peer(&[first.clone(), first.clone()], &[], 0),
        TransportError::Rejected {
            code: FailureCode::Rejected,
            ..
        }
    ));
    assert!(matches!(
        lower_from_peer(
            &[first, rejection(&identity(), "different refusal")],
            &[],
            0
        ),
        TransportError::Protocol(ProtocolError::Transition)
    ));
}

#[test]
fn every_call_identity_component_is_checked_before_replay() {
    let expected = identity();
    for received in [
        AttemptIdentity {
            session: SessionId::new("another-peer").unwrap(),
            ..expected.clone()
        },
        AttemptIdentity {
            attempt: AttemptId::new(9).unwrap(),
            ..expected.clone()
        },
        AttemptIdentity {
            epoch: FenceEpoch::new(9).unwrap(),
            ..expected.clone()
        },
    ] {
        assert!(matches!(
            lower_from_peer(
                &[
                    rejection(&expected, "refused"),
                    rejection(&received, "refused")
                ],
                &[],
                0
            ),
            TransportError::Protocol(ProtocolError::Identity)
        ));
    }
    let mut forbidden_worker = rejection(&expected, "refused");
    if let FromBridge::Failed { worker, .. } = &mut forbidden_worker {
        *worker = Some(WorkerBinding {
            instance: WorkerInstanceId::of(b"worker"),
            epoch: FenceEpoch::new(1).unwrap(),
        });
    }
    assert!(matches!(
        lower_from_peer(&[forbidden_worker], &[], 0),
        TransportError::Protocol(ProtocolError::Identity)
    ));
}

#[test]
fn eof_truncated_tail_and_failed_process_do_not_impersonate_a_result() {
    assert!(matches!(
        lower_from_peer(&[], &[], 0),
        TransportError::NoTerminal
    ));
    let refused = rejection(&identity(), "refused");
    assert!(matches!(
        lower_from_peer(std::slice::from_ref(&refused), &[0, 0, 0], 0),
        TransportError::Protocol(ProtocolError::TruncatedFrame)
    ));
    assert!(matches!(
        lower_from_peer(&[refused], &[], 7),
        TransportError::Process { .. }
    ));
}
