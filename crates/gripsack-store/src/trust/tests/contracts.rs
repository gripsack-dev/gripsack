//! Production producers and readers agree with their published wire shapes.
//! Relational policy/runtime/outcome checks intentionally exceed JSON Schema.
use super::*;
use crate::source_bundle::{SourceBundleDigest, SourceInventory};
use crate::trust::evaluation::{self, EvaluationOutcome, EvaluationSession};
use gripsack_process::{Control, Invocation, NativeInput, ProcessRole, Sha256Digest};
use serde_json::{Value, json};
use std::ffi::OsStr;

fn schemas() -> (
    jsonschema::Validator,
    jsonschema::Validator,
    jsonschema::Validator,
) {
    let source: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema-source-bundle-v1.json"
    )))
    .unwrap();
    let trust: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema-trust-v2.json"
    )))
    .unwrap();
    let evaluation: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema-evaluation-v1.json"
    )))
    .unwrap();
    let source_validator = jsonschema::validator_for(&source).unwrap();
    let trust_validator = jsonschema::validator_for(&trust).unwrap();
    let registry = jsonschema::Registry::new()
        .add("https://gripsack.dev/schema/trust/v2.json", trust)
        .unwrap()
        .prepare()
        .unwrap();
    let evaluation_validator = jsonschema::options()
        .with_registry(&registry)
        .build(&evaluation)
        .unwrap();
    (source_validator, trust_validator, evaluation_validator)
}

fn completed(fixture: &Fixture) -> (evaluation::EvaluationId, Value) {
    let bundle = fixture.capture();
    let policy = fixture.policy(&bundle, &());
    let mut session =
        EvaluationSession::begin(&fixture.home, &bundle, &policy, &GitProvenance::default())
            .unwrap();
    session
        .record_input(Sha256Digest::of(b"immutable fixture inputs"))
        .unwrap();
    let environment =
        OperatorEnvironment::admit([(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))])
            .unwrap();
    let limits = Limits::default();
    let deadline = Instant::now() + limits.timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Evaluator,
        &selected,
        &fixture.repo,
        Limits {
            operation_deadline: Some(deadline),
            ..limits
        },
    )
    .unwrap();
    let process = invocation
        .run(
            &[OsStr::new("-c"), OsStr::new("exit 0")],
            NativeInput::Bytes(b""),
            None,
            |_| Control::Continue,
        )
        .unwrap();
    assert!(process.success);
    session.record_process(process.receipt).unwrap();
    session.finish(EvaluationOutcome::Completed).unwrap();
    let receipt = evaluation::read(&fixture.home, session.id()).unwrap();
    assert_eq!(receipt.source, bundle.digest());
    assert_eq!(receipt.policy_digest, policy.digest().unwrap());
    (session.id(), serde_json::to_value(receipt).unwrap())
}

#[test]
fn emitted_source_and_approval_shapes_reject_unknown_versions_fields_and_units() {
    let fixture = Fixture::new();
    let bundle = fixture.capture();
    let policy = fixture.policy(&bundle, &());
    fixture.approve(&bundle, &policy);
    let (source_schema, trust_schema, _) = schemas();
    let inventory: Value = serde_json::from_slice(bundle.inventory_bytes()).unwrap();
    let approval = serde_json::to_value(list(&fixture.home).unwrap()).unwrap();
    source_schema.validate(&inventory).unwrap();
    trust_schema.validate(&approval).unwrap();
    let mut inventories = Vec::new();
    let mut version = inventory.clone();
    version["version"] = json!(2);
    inventories.push(version);
    let mut extra = inventory.clone();
    extra["unreviewed_root"] = json!("outside");
    inventories.push(extra);
    let mut missing = inventory.clone();
    missing.as_object_mut().unwrap().remove("entries");
    inventories.push(missing);
    let file = inventory["entries"]
        .as_array()
        .unwrap()
        .iter()
        .position(|entry| entry["object"]["kind"] == "file")
        .unwrap();
    let mut oversized = inventory.clone();
    oversized["entries"][file]["object"]["bytes"] = json!(64 * 1024 * 1024 + 1);
    inventories.push(oversized);
    let mut mode = inventory;
    mode["entries"][file]["object"]["mode"] = json!(8192);
    inventories.push(mode);
    for document in inventories {
        assert!(!source_schema.is_valid(&document), "{document}");
        let bytes = serde_json::to_vec(&document).unwrap();
        let digest = SourceBundleDigest::parse(&crate::hash::hex_sha256(&bytes)).unwrap();
        assert!(SourceInventory::decode(&bytes, digest).is_err());
    }
    let mut approvals = Vec::new();
    let mut version = approval.clone();
    version["version"] = json!(3);
    approvals.push(version);
    let mut extra = approval.clone();
    extra["approved"][0]["future_authority"] = json!(true);
    approvals.push(extra);
    let mut missing = approval.clone();
    missing["approved"][0]
        .as_object_mut()
        .unwrap()
        .remove("bundle");
    approvals.push(missing);
    let mut grants = approval;
    grants["approved"][0]["policy"]["read_roots"] = json!(["repository", "outside"]);
    approvals.push(grants);
    for document in approvals {
        assert!(!trust_schema.is_valid(&document), "{document}");
        let admitted = serde_json::from_value::<TrustListing>(document)
            .ok()
            .and_then(|value| value.validate().ok());
        assert!(admitted.is_none());
    }
}

#[test]
fn persisted_receipt_admission_checks_shape_and_bound_execution_not_just_json() {
    let fixture = Fixture::new();
    let (id, receipt) = completed(&fixture);
    let (_, _, schema) = schemas();
    schema.validate(&receipt).unwrap();
    let path = fixture.home.join("evaluations").join(format!("{id}.json"));
    let mut malformed = Vec::new();
    let mut version = receipt.clone();
    version["version"] = json!(2);
    malformed.push(version);
    let mut extra = receipt.clone();
    extra["rounds"][0]["process"]["environment_values"] = json!({"secret":"not-authorized"});
    malformed.push(extra);
    let mut missing = receipt.clone();
    missing.as_object_mut().unwrap().remove("source");
    malformed.push(missing);
    let mut input = receipt.clone();
    input["rounds"][0]["input_sha256"] = json!("bad-digest");
    malformed.push(input);
    for document in malformed {
        assert!(!schema.is_valid(&document));
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(evaluation::read(&fixture.home, id).is_err());
    }
    let mut inconsistent = Vec::new();
    let mut source = receipt.clone();
    source["source"] = json!("0".repeat(64));
    inconsistent.push(source);
    let mut policy = receipt.clone();
    policy["policy_digest"] = json!("0".repeat(64));
    inconsistent.push(policy);
    let mut runtime = receipt.clone();
    runtime["rounds"][0]["process"]["executable_sha256"] = json!("0".repeat(64));
    inconsistent.push(runtime);
    let mut order = receipt.clone();
    order["rounds"][0]["number"] = json!(2);
    inconsistent.push(order);
    let mut absent = receipt.clone();
    absent["rounds"][0]["process"] = Value::Null;
    inconsistent.push(absent);
    let mut rejected = receipt.clone();
    rejected["outcome"] = json!("rejected");
    inconsistent.push(rejected);
    let mut failed = receipt.clone();
    failed["rounds"][0]["process"]["exit_code"] = json!(7);
    inconsistent.push(failed);
    let mut identity = receipt;
    identity["id"] = json!("0".repeat(64));
    inconsistent.push(identity);
    for document in inconsistent {
        schema.validate(&document).unwrap();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(evaluation::read(&fixture.home, id).is_err(), "{document}");
    }
    fs::write(&path, vec![b' '; 128 * 1024 + 1]).unwrap();
    assert!(evaluation::read(&fixture.home, id).is_err());
}
