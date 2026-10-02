use super::{CheckedDefinition, Lowered, pb};
use crate::{
    identity::{
        AttemptId, AttemptIdentity, FenceEpoch, LlbVertexDigest, SessionId, SnapshotDigest,
    },
    plan::{
        Architecture, BuildPlan, ExporterPlan, LinuxOs, Mount, Node, NodeIndex, Platform,
        ValidatedBuildPlan,
    },
    transport::Bridge,
};
use gripsack_process::{OperatorEnvironment, SelectedProgram};
use protobuf::Message;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

fn index(value: usize) -> NodeIndex {
    NodeIndex::new(value).unwrap()
}

fn graph() -> ValidatedBuildPlan {
    ValidatedBuildPlan::admit(BuildPlan {
        platform: Platform {
            os: LinuxOs::Linux,
            architecture: Architecture::Amd64,
        },
        nodes: vec![
            Node::Image {
                reference: format!("docker.io/library/toolchain@sha256:{}", "a".repeat(64)),
            },
            Node::Local {
                name: "captured".into(),
                digest: SnapshotDigest::of(b"captured source identity"),
            },
            Node::Process {
                root: index(0),
                argv: vec![
                    "/bin/cp".into(),
                    "/source/value".into(),
                    "/output/value".into(),
                ],
                env: vec!["PATH=/bin".into()],
                cwd: "/output".into(),
                mounts: vec![
                    Mount {
                        source: Some(index(1)),
                        destination: "/source".into(),
                        readonly: true,
                    },
                    Mount {
                        source: None,
                        destination: "/output".into(),
                        readonly: false,
                    },
                ],
                output: "/output".into(),
            },
            Node::Process {
                root: index(0),
                argv: vec!["/bin/test".into(), "-s".into(), "/subject/value".into()],
                env: vec!["PATH=/bin".into()],
                cwd: "/check".into(),
                mounts: vec![
                    Mount {
                        source: Some(index(2)),
                        destination: "/subject".into(),
                        readonly: true,
                    },
                    Mount {
                        source: None,
                        destination: "/check".into(),
                        readonly: false,
                    },
                ],
                output: "/check".into(),
            },
            Node::File {
                input: Some(index(3)),
                path: "/passed".into(),
                data: b"check complete".to_vec(),
                mode: 0o444,
            },
            Node::Copy {
                input: Some(index(2)),
                source: index(4),
                source_path: "/passed".into(),
                destination: "/checks/passed".into(),
                contents: false,
            },
        ],
        root: index(5),
        exporter: ExporterPlan::Local,
    })
    .unwrap()
}

fn upstream(plan: &ValidatedBuildPlan) -> Lowered {
    let binary = PathBuf::from(std::env::var_os("GRIPSACK_TEST_BRIDGE").expect(
        "real pinned bridge required: run the compose test gate or set GRIPSACK_TEST_BRIDGE",
    ));
    let directory = tempfile::tempdir().unwrap();
    let environment = OperatorEnvironment::capture().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let program = SelectedProgram::select(&environment, &binary, None, deadline).unwrap();
    let bridge = Bridge::new(&environment, &program, directory.path(), deadline);
    let identity = AttemptIdentity {
        session: SessionId::new("checker-regression").unwrap(),
        attempt: AttemptId::new(1).unwrap(),
        epoch: FenceEpoch::new(1).unwrap(),
    };
    let checked = bridge
        .lower(plan, &identity, None)
        .expect("real upstream definition must pass independent admission");
    Lowered {
        definition: checked.definition,
        witness: checked.witness,
        exporter: checked.exporter,
    }
}

/// Rehash the entire dependent graph after changing one operation. A rejected
/// mutant must expose changed semantics, not merely a dangling old digest.
fn alter(original: &Lowered, node: usize, mutation: impl FnOnce(&mut pb::Op)) -> Lowered {
    let mut lowered = original.clone();
    let mut definition = pb::Definition::parse_from_bytes(&lowered.definition).unwrap();
    let target = &original.witness[node].vertex;
    let mut mutation = Some(mutation);
    let mut rewritten = BTreeMap::<String, String>::new();
    for bytes in &mut definition.def {
        let old = LlbVertexDigest::of(bytes);
        let mut operation = pb::Op::parse_from_bytes(bytes).unwrap();
        for input in &mut operation.inputs {
            if let Some(digest) = rewritten.get(&input.digest) {
                input.digest.clone_from(digest);
            }
        }
        if &old == target {
            mutation.take().unwrap()(&mut operation);
        }
        *bytes = operation.write_to_bytes().unwrap();
        rewritten.insert(old.to_string(), LlbVertexDigest::of(bytes).to_string());
    }
    assert!(
        mutation.is_none(),
        "mutated operation was not in the checked witness"
    );
    for entry in &mut definition.metadata {
        entry.key = rewritten[&entry.key].clone();
    }
    if let Some(source) = definition.source.as_mut() {
        for entry in &mut source.locations {
            entry.key = rewritten[&entry.key].clone();
        }
    }
    for witness in &mut lowered.witness {
        witness.vertex = rewritten[witness.vertex.as_str()]
            .clone()
            .try_into()
            .unwrap();
    }
    lowered.definition = definition.write_to_bytes().unwrap();
    lowered
}

#[test]
fn actual_upstream_graph_rejects_policy_and_command_substitutions() {
    let plan = graph();
    let original = upstream(&plan);
    type ExecMutation = (&'static str, fn(&mut pb::ExecOp));
    let changes: &[ExecMutation] = &[
        ("network entitlement", |exec| exec.network = 0),
        ("insecure executor", |exec| exec.security = 1),
        ("argv substitution", |exec| {
            exec.meta.as_mut().unwrap().args[0] = "/bin/false".into()
        }),
        ("ambient environment", |exec| {
            exec.meta
                .as_mut()
                .unwrap()
                .env
                .push("UNDECLARED=value".into())
        }),
        ("working directory", |exec| {
            exec.meta.as_mut().unwrap().cwd = "/different".into()
        }),
        ("accepted failing exit", |exec| {
            exec.meta.as_mut().unwrap().valid_exit_codes.push(1)
        }),
        ("writable source", |exec| {
            exec.mounts
                .iter_mut()
                .find(|mount| mount.dest == "/source")
                .unwrap()
                .readonly = false
        }),
        ("hidden cache policy", |exec| {
            exec.mounts
                .iter_mut()
                .find(|mount| mount.dest == "/source")
                .unwrap()
                .content_cache = 1
        }),
        ("secret mount", |exec| {
            exec.mounts
                .iter_mut()
                .find(|mount| mount.dest == "/source")
                .unwrap()
                .mount_type = 1
        }),
    ];
    for (name, mutation) in changes {
        let changed = alter(&original, 2, |operation| {
            let Some(pb::op::Op::Exec(exec)) = &mut operation.op else {
                panic!("process fixture");
            };
            mutation(exec);
        });
        assert!(
            CheckedDefinition::validate(&plan, changed).is_err(),
            "{name} reached checked execution authority"
        );
    }
}

#[test]
fn actual_upstream_graph_rejects_source_root_and_unknown_field_substitutions() {
    let plan = graph();
    let original = upstream(&plan);
    let source = alter(&original, 1, |operation| {
        let Some(pb::op::Op::Source(source)) = &mut operation.op else {
            panic!("local fixture");
        };
        source.attrs[0].value = "b".repeat(64);
    });
    assert!(
        CheckedDefinition::validate(&plan, source).is_err(),
        "captured source pin was replaced"
    );
    let unknown = alter(&original, 2, |operation| {
        let Some(pb::op::Op::Exec(exec)) = &mut operation.op else {
            panic!("process fixture");
        };
        exec.meta
            .as_mut()
            .unwrap()
            .special_fields
            .mut_unknown_fields()
            .add_varint(99, 1);
    });
    assert!(
        CheckedDefinition::validate(&plan, unknown).is_err(),
        "unknown nested process authority was accepted"
    );
    let mut missing_check = original.clone();
    let mut definition = pb::Definition::parse_from_bytes(&missing_check.definition).unwrap();
    let terminal = definition.def.last_mut().unwrap();
    let mut operation = pb::Op::parse_from_bytes(terminal).unwrap();
    operation.inputs[0].digest = original.witness[2].vertex.to_string();
    *terminal = operation.write_to_bytes().unwrap();
    missing_check.definition = definition.write_to_bytes().unwrap();
    assert!(
        CheckedDefinition::validate(&plan, missing_check).is_err(),
        "required validator was pruned from the selected export"
    );
    let mut substituted = original;
    substituted.witness[2].output = 1;
    assert!(
        CheckedDefinition::validate(&plan, substituted).is_err(),
        "another process output acquired publication authority"
    );
}
