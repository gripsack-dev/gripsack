//! M-V4: deterministic byte-level properties over the production decoders and
//! recovery path. No random generator, fuzz runner or corpus replay is involved.
use super::marker::{RunMarker, RunOp, begin_run};
use super::recover::reconcile;
use super::{Entry, Intended, ObjectIdentity, entry_name, record};
use serde_json::{Value, json};
use std::path::Path;

struct ByteCase {
    name: String,
    bytes: Vec<u8>,
}

impl ByteCase {
    fn json(name: impl Into<String>, value: &Value) -> Self {
        Self {
            name: name.into(),
            bytes: serde_json::to_vec(value).unwrap(),
        }
    }
}

fn duplicate_member(object: &Value, key: &str, value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(object).unwrap();
    assert_eq!(bytes.pop(), Some(b'}'));
    bytes.push(b',');
    serde_json::to_writer(&mut bytes, key).unwrap();
    bytes.push(b':');
    serde_json::to_writer(&mut bytes, value).unwrap();
    bytes.push(b'}');
    bytes
}

fn raw_member(object: &Value, key: &str, raw: &[u8]) -> Vec<u8> {
    let mut bytes = vec![b'{'];
    for (index, (name, value)) in object.as_object().unwrap().iter().enumerate() {
        if index != 0 {
            bytes.push(b',');
        }
        serde_json::to_writer(&mut bytes, name).unwrap();
        bytes.push(b':');
        if name == key {
            bytes.extend_from_slice(raw);
        } else {
            serde_json::to_writer(&mut bytes, value).unwrap();
        }
    }
    bytes.push(b'}');
    bytes
}

fn marker_value() -> Value {
    json!({"previous_generation": null, "target_generation": 1, "op": "apply"})
}

fn transaction_marker_value() -> Value {
    json!({
        "version": 1, "previous_selection": null,
        "target_selection": {"kind": "transaction", "generation": 1, "transaction": "a".repeat(64)},
        "op": "apply",
    })
}

fn rejected_markers() -> Vec<ByteCase> {
    let base = marker_value();
    let mut cases = Vec::new();
    for key in ["previous_generation", "target_generation", "op"] {
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove(key);
        cases.push(ByteCase::json(format!("missing_{key}"), &missing));
        let invalid = if key == "op" {
            vec![
                Value::Null,
                json!(true),
                json!(1),
                json!([]),
                json!({}),
                json!("unknown"),
            ]
        } else {
            let mut values = vec![
                json!(true),
                json!("1"),
                json!(-1),
                json!(1.5),
                json!([]),
                json!({}),
            ];
            if key == "target_generation" {
                values.push(Value::Null);
            }
            values
        };
        for (index, value) in invalid.into_iter().enumerate() {
            let mut wrong = base.clone();
            wrong[key] = value;
            cases.push(ByteCase::json(format!("wrong_type_{key}_{index}"), &wrong));
        }
        cases.push(ByteCase {
            name: format!("duplicate_{key}"),
            bytes: duplicate_member(&base, key, &base[key]),
        });
    }
    // Explicit null is a seen identity field, including when it appears first.
    for (first, second) in [(Value::Null, json!(7)), (json!(7), Value::Null)] {
        let mut value = base.clone();
        value["previous_generation"] = first;
        cases.push(ByteCase {
            name: format!("duplicate_previous_{second}"),
            bytes: duplicate_member(&value, "previous_generation", &second),
        });
    }
    for key in ["previous_generation", "target_generation"] {
        cases.push(ByteCase {
            name: format!("u64_overflow_{key}"),
            bytes: raw_member(&base, key, b"18446744073709551616"),
        });
    }
    let transaction = transaction_marker_value();
    for key in ["version", "previous_selection", "target_selection", "op"] {
        let mut missing = transaction.clone();
        missing.as_object_mut().unwrap().remove(key);
        cases.push(ByteCase::json(
            format!("transaction_missing_{key}"),
            &missing,
        ));
        cases.push(ByteCase {
            name: format!("transaction_duplicate_{key}"),
            bytes: duplicate_member(&transaction, key, &transaction[key]),
        });
    }
    for version in [Value::Null, json!(0), json!(2), json!("1"), json!(1.5)] {
        let mut value = transaction.clone();
        value["version"] = version;
        cases.push(ByteCase::json("unsupported_transaction_version", &value));
    }
    for key in ["previous_generation", "target_generation"] {
        let mut value = transaction.clone();
        value[key] = json!(1);
        cases.push(ByteCase::json(format!("mixed_transaction_{key}"), &value));
    }
    for field in ["previous_selection", "target_selection"] {
        for invalid in [
            json!(1),
            json!([]),
            json!({}),
            json!({"kind": "unknown", "generation": 1}),
            json!({"kind": "transaction", "generation": 1}),
            json!({"kind": "transaction", "transaction": "a".repeat(64)}),
            json!({"kind": "transaction", "generation": "1", "transaction": "a".repeat(64)}),
            json!({"kind": "transaction", "generation": 1, "transaction": "A".repeat(64)}),
            json!({"kind": "transaction", "generation": 1, "transaction": "a".repeat(63)}),
            json!({"kind": "transaction", "generation": 1, "transaction": "g".repeat(64)}),
            json!({"kind": "legacy", "generation": 1, "transaction": "a".repeat(64)}),
        ] {
            let mut value = transaction.clone();
            value[field] = invalid;
            cases.push(ByteCase::json(
                format!("invalid_transaction_{field}"),
                &value,
            ));
        }
    }
    for target in [Value::Null, json!({"kind": "legacy", "generation": 1})] {
        let mut value = transaction.clone();
        value["target_selection"] = target;
        cases.push(ByteCase::json(
            "transaction_target_without_identity",
            &value,
        ));
    }
    let mut reused = transaction.clone();
    reused["previous_selection"] = reused["target_selection"].clone();
    cases.push(ByteCase::json("transaction_reused_identity", &reused));
    cases
}

fn entry_value(version: u8, destination: &str, prior: Value, after: Value) -> Value {
    let mut value = json!({"v": version, "dest": destination, "prior": prior, "after": after});
    if version == 2 {
        value["before"] = value["prior"].clone();
    }
    value
}

fn rejected_entries(destination: &str) -> Vec<ByteCase> {
    let base = entry_value(
        2,
        destination,
        json!({"kind": "absent"}),
        json!({"kind": "link", "target": "installed"}),
    );
    let mut cases = Vec::new();
    for version in [1, 2] {
        let mut versioned = base.clone();
        versioned["v"] = json!(version);
        if version == 1 {
            versioned.as_object_mut().unwrap().remove("before");
        }
        for key in versioned.as_object().unwrap().keys() {
            let mut missing = versioned.clone();
            missing.as_object_mut().unwrap().remove(key);
            cases.push(ByteCase::json(
                format!("v{version}_missing_{key}"),
                &missing,
            ));
            for (index, wrong) in [Value::Null, json!(true), json!([])]
                .into_iter()
                .enumerate()
            {
                let mut value = versioned.clone();
                value[key] = wrong;
                cases.push(ByteCase::json(
                    format!("v{version}_wrong_{key}_{index}"),
                    &value,
                ));
            }
            cases.push(ByteCase {
                name: format!("v{version}_duplicate_{key}"),
                bytes: duplicate_member(&versioned, key, &versioned[key]),
            });
        }
    }
    for version in [
        json!(0),
        json!(3),
        json!(255),
        json!(256),
        json!(-1),
        json!(1.5),
        json!("2"),
    ] {
        let mut value = base.clone();
        value["v"] = version.clone();
        cases.push(ByteCase::json(
            format!("unsupported_version_{version}"),
            &value,
        ));
    }
    let mut version_one_with_before = base.clone();
    version_one_with_before["v"] = json!(1);
    cases.push(ByteCase::json(
        "v1_cannot_reinterpret_v2_before",
        &version_one_with_before,
    ));
    for key in ["prior", "before"] {
        for prior in [
            json!({}),
            json!({"kind": "unknown"}),
            json!({"kind": "file", "mode": 0o600}),
            json!({"kind": "file", "hash": null, "mode": 0o600}),
            json!({"kind": "file", "hash": "a".repeat(64)}),
            json!({"kind": "symlink"}),
            json!({"kind": "symlink", "target": null}),
        ] {
            let mut value = base.clone();
            value[key] = prior;
            cases.push(ByteCase::json(format!("invalid_{key}_shape"), &value));
        }
        for hash in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "g".repeat(64),
            "é".repeat(32),
        ] {
            let mut value = base.clone();
            value[key] = json!({"kind": "file", "hash": hash, "mode": 0o600});
            cases.push(ByteCase::json(format!("invalid_{key}_hash"), &value));
        }
        for mode in [
            json!(-1),
            json!(0o10000),
            json!(u32::MAX),
            json!(u64::MAX),
            json!(0.0),
            Value::Null,
        ] {
            let mut value = base.clone();
            value[key] = json!({"kind": "file", "hash": "a".repeat(64), "mode": mode});
            cases.push(ByteCase::json(format!("invalid_{key}_mode"), &value));
        }
        cases.push(ByteCase {
            name: format!("duplicate_{key}_kind"),
            bytes: raw_member(&base, key, br#"{"kind":"absent","kind":"absent"}"#),
        });
        let file = json!({"kind": "file", "hash": "a".repeat(64), "mode": 0});
        for member in ["hash", "mode"] {
            cases.push(ByteCase {
                name: format!("duplicate_{key}_{member}"),
                bytes: raw_member(&base, key, &duplicate_member(&file, member, &file[member])),
            });
        }
        let link = json!({"kind": "symlink", "target": "prior"});
        cases.push(ByteCase {
            name: format!("duplicate_{key}_target"),
            bytes: raw_member(
                &base,
                key,
                &duplicate_member(&link, "target", &link["target"]),
            ),
        });
    }
    for after in [
        json!({}),
        json!({"kind": "unknown"}),
        json!({"kind": "file"}),
        json!({"kind": "file", "identity": null}),
        json!({"kind": "file", "identity": "g".repeat(64)}),
        json!({"kind": "file", "identity": "a".repeat(63)}),
        json!({"kind": "file", "identity": "a".repeat(65)}),
        json!({"kind": "link"}),
        json!({"kind": "link", "target": null}),
    ] {
        let mut value = base.clone();
        value["after"] = after;
        cases.push(ByteCase::json("invalid_intended_identity", &value));
    }
    for (member, identity) in [
        ("kind", json!({"kind": "removed"})),
        (
            "identity",
            json!({"kind": "file", "identity": "b".repeat(64)}),
        ),
        ("target", json!({"kind": "link", "target": "installed"})),
    ] {
        cases.push(ByteCase {
            name: format!("duplicate_after_{member}"),
            bytes: raw_member(
                &base,
                "after",
                &duplicate_member(&identity, member, &identity[member]),
            ),
        });
    }
    cases.push(ByteCase {
        name: "legacy_string_intent".into(),
        bytes: raw_member(&base, "after", br#""gripsack:removed""#),
    });
    cases
}

#[test]
fn required_marker_fields_never_default_to_fresh_state() {
    let base = marker_value();
    let fresh: RunMarker = serde_json::from_slice(&serde_json::to_vec(&base).unwrap()).unwrap();
    assert_eq!(fresh.previous, None);
    assert_eq!(fresh.target.generation(), crate::GenerationId::new(1));
    assert_eq!(fresh.op, RunOp::Apply);
    for key in ["previous_generation", "target_generation", "op"] {
        let mut value = base.clone();
        value.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_slice::<RunMarker>(&serde_json::to_vec(&value).unwrap()).is_err(),
            "required_marker_field_omitted: {key}"
        );
    }
    println!("JOURNAL_REQUIRED_MARKER_FIELDS=3");
}

#[test]
fn marker_scalar_boundaries_roundtrip_without_identity_loss() {
    let mut count = 0;
    for previous in [None, Some(0), Some(1), Some(u64::MAX)] {
        for target in [0, 1, u64::MAX] {
            for op in [RunOp::Apply, RunOp::Rollback] {
                let value: RunMarker = serde_json::from_value(json!({
                    "previous_generation": previous,
                    "target_generation": target,
                    "op": op,
                }))
                .unwrap();
                let encoded = serde_json::to_vec(&value).unwrap();
                let decoded: RunMarker = serde_json::from_slice(&encoded).unwrap();
                assert_eq!(
                    decoded
                        .previous
                        .as_ref()
                        .map(|selection| selection.generation()),
                    previous.map(crate::GenerationId::new)
                );
                assert_eq!(
                    decoded.target.generation(),
                    crate::GenerationId::new(target)
                );
                assert_eq!(decoded.op, op);
                count += 1;
            }
        }
    }
    for previous in [
        Value::Null,
        json!({"kind": "legacy", "generation": 1}),
        json!({"kind": "transaction", "generation": 1, "transaction": "b".repeat(64)}),
    ] {
        for target in [0, 1, u64::MAX] {
            for op in [RunOp::Apply, RunOp::Rollback] {
                let mut wire = transaction_marker_value();
                wire["previous_selection"] = previous.clone();
                wire["target_selection"]["generation"] = json!(target);
                wire["op"] = json!(op);
                let decoded: RunMarker = serde_json::from_value(wire.clone()).unwrap();
                assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
                count += 1;
            }
        }
    }
    println!("JOURNAL_MARKER_ROUNDTRIPS={count}");
}

#[test]
fn marker_wrong_types_duplicates_and_overflow_do_not_admit_facts() {
    let cases = rejected_markers();
    for case in &cases {
        assert!(
            serde_json::from_slice::<RunMarker>(&case.bytes).is_err(),
            "{}",
            case.name
        );
    }
    println!("JOURNAL_MARKER_REJECTIONS={}", cases.len());
}

#[test]
fn entry_versions_and_distinct_prior_states_roundtrip() {
    let priors = [
        json!({"kind": "absent"}),
        json!({"kind": "file", "hash": "a".repeat(64), "mode": 0o600}),
        json!({"kind": "symlink", "target": "prior-λ"}),
    ];
    let intended = [
        json!({"kind": "removed"}),
        json!({"kind": "file", "identity": "b".repeat(64)}),
        json!({"kind": "link", "target": "installed-界"}),
    ];
    let mut count = 0;
    for version in [1, 2] {
        for (index, prior) in priors.iter().enumerate() {
            for after in &intended {
                let mut value = entry_value(version, "/fixture/𐐷", prior.clone(), after.clone());
                if version == 2 {
                    value["before"] = priors[(index + 1) % priors.len()].clone();
                }
                let entry = Entry::from_wire(&serde_json::to_vec(&value).unwrap()).unwrap();
                assert_eq!(serde_json::to_value(&entry).unwrap(), value);
                assert_eq!(
                    Entry::from_wire(&serde_json::to_vec(&entry).unwrap()).unwrap(),
                    entry
                );
                count += 1;
            }
        }
    }
    for mode in [0, 0o7777] {
        let value = entry_value(
            2,
            "/fixture",
            json!({"kind":"file", "hash":"c".repeat(64), "mode":mode}),
            json!({"kind":"removed"}),
        );
        let entry = Entry::from_wire(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&entry).unwrap(), value);
        count += 1;
    }
    println!("JOURNAL_ENTRY_ROUNDTRIPS={count}");
}

#[test]
fn entry_missing_duplicate_version_and_scalar_bytes_are_rejected() {
    let cases = rejected_entries("/fixture/target");
    for case in &cases {
        assert!(Entry::from_wire(&case.bytes).is_err(), "{}", case.name);
    }
    println!("JOURNAL_ENTRY_REJECTIONS={}", cases.len());
}

#[test]
fn truncation_and_invalid_framing_never_admit_partial_records() {
    let marker = serde_json::to_vec(&marker_value()).unwrap();
    let entry = serde_json::to_vec(&entry_value(
        2,
        "/fixture/𐐷",
        json!({"kind":"absent"}),
        json!({"kind":"removed"}),
    ))
    .unwrap();
    for end in 0..marker.len() {
        assert!(
            serde_json::from_slice::<RunMarker>(&marker[..end]).is_err(),
            "marker prefix {end}"
        );
    }
    for end in 0..entry.len() {
        assert!(
            Entry::from_wire(&entry[..end]).is_err(),
            "entry prefix {end}"
        );
    }
    for suffix in [b"null".as_slice(), b"{}", b"\xff"] {
        let mut bad_marker = marker.clone();
        bad_marker.extend_from_slice(suffix);
        assert!(serde_json::from_slice::<RunMarker>(&bad_marker).is_err());
        let mut bad_entry = entry.clone();
        bad_entry.extend_from_slice(suffix);
        assert!(Entry::from_wire(&bad_entry).is_err());
    }
    for invalid in [b"null".as_slice(), b"[]", b"0", b"\xff"] {
        assert!(serde_json::from_slice::<RunMarker>(invalid).is_err());
        assert!(Entry::from_wire(invalid).is_err());
    }
    println!("JOURNAL_TRUNCATED_PREFIXES={}", marker.len() + entry.len());
}

struct RecoveryFixture {
    temporary: tempfile::TempDir,
    home: gripsack_fs::Dir,
    destination: std::path::PathBuf,
    marker_path: std::path::PathBuf,
    entry_path: std::path::PathBuf,
}

impl RecoveryFixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let home = gripsack_fs::open_or_create(temporary.path()).unwrap();
        let destination = temporary.path().join("destination");
        let journal = temporary.path().join("journal");
        let marker_path = journal.join("run.json");
        let entry_path = journal.join(entry_name(&destination));
        let run = begin_run(
            &home,
            temporary.path(),
            None,
            crate::GenerationId::new(1),
            RunOp::Apply,
        )
        .unwrap();
        let captured = super::capture(
            &run,
            gripsack_fs::open_or_create(temporary.path()).unwrap(),
            Path::new("destination").to_path_buf(),
            &destination,
            None,
        )
        .unwrap();
        drop(
            record(
                captured,
                &Intended::Object(ObjectIdentity::Link("installed".into())),
            )
            .unwrap(),
        );
        drop(run);
        home.symlink("installed", Path::new("destination")).unwrap();
        Self {
            temporary,
            home,
            destination,
            marker_path,
            entry_path,
        }
    }

    fn marker(&self) -> &Path {
        &self.marker_path
    }

    fn entry(&self) -> &Path {
        &self.entry_path
    }
}

#[test]
fn rejected_metadata_cannot_reach_destination_effects() {
    let markers = rejected_markers();
    for case in &markers {
        let fixture = RecoveryFixture::new();
        let original_entry = std::fs::read(fixture.entry()).unwrap();
        std::fs::write(fixture.marker(), &case.bytes).unwrap();
        let result = reconcile(&fixture.home, fixture.temporary.path());
        assert!(
            result.is_err(),
            "recovery_admitted_invalid_marker: {}",
            case.name
        );
        assert_eq!(
            std::fs::read_link(&fixture.destination).unwrap(),
            Path::new("installed")
        );
        assert_eq!(std::fs::read(fixture.marker()).unwrap(), case.bytes);
        assert_eq!(std::fs::read(fixture.entry()).unwrap(), original_entry);
    }
    let fixture = RecoveryFixture::new();
    let entries = rejected_entries(fixture.destination.to_str().unwrap());
    let original_marker = std::fs::read(fixture.marker()).unwrap();
    let retained = fixture
        .temporary
        .path()
        .join("journal/quarantine")
        .join(fixture.entry().file_name().unwrap());
    for case in &entries {
        std::fs::write(fixture.entry(), &case.bytes).unwrap();
        assert!(
            reconcile(&fixture.home, fixture.temporary.path()).is_err(),
            "recovery_admitted_invalid_entry: {}",
            case.name
        );
        assert_eq!(
            std::fs::read_link(&fixture.destination).unwrap(),
            Path::new("installed")
        );
        assert_eq!(std::fs::read(&retained).unwrap(), case.bytes);
        assert_eq!(std::fs::read(fixture.marker()).unwrap(), original_marker);
        std::fs::remove_file(&retained).unwrap();
    }
    // Positive control: explicit null really is a legitimate fresh transaction.
    let fixture = RecoveryFixture::new();
    reconcile(&fixture.home, fixture.temporary.path()).unwrap();
    assert_eq!(
        std::fs::symlink_metadata(&fixture.destination)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    println!(
        "JOURNAL_EFFECT_REJECTIONS={}",
        markers.len() + entries.len()
    );
}
