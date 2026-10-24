//! Old and malformed metadata never acquire directory mutation authority.
use super::*;

#[test]
fn directory_wire_unknown_version_quarantines_without_effects() {
    let (temporary, home, receipt) = fixture();
    let run = begin_run(
        &home,
        temporary.path(),
        None,
        crate::GenerationId::new(1),
        RunOp::Apply,
    )
    .unwrap();
    directory::persist(&run, &receipt, Direction::Forward).unwrap();
    let record_path = temporary
        .path()
        .join("journal")
        .join(directory::record_name(&receipt));
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["v"] = serde_json::json!(2);
    std::fs::write(record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(reconcile(&home, temporary.path()).is_err());
    assert_eq!(
        std::fs::read_link(&receipt.destination).unwrap(),
        receipt.predecessor_target
    );
    assert_eq!(pending_recovery(&home).unwrap().unwrap().quarantined, 1);
}

#[test]
fn old_manifest_versions_cannot_acquire_directory_authority() {
    let (_temporary, _home, receipt) = fixture();
    let manifest = generation(1, vec![receipt]);
    let mut wire = serde_json::to_value(&manifest).unwrap();
    assert_eq!(wire["version"], 3);
    assert_eq!(
        serde_json::from_value::<crate::Generation>(wire.clone()).unwrap(),
        manifest
    );
    wire["version"] = serde_json::json!(2);
    assert!(serde_json::from_value::<crate::Generation>(wire).is_err());
}
#[test]
fn malformed_directory_authority_never_reaches_destination_effects() {
    for corruption in [
        "version",
        "direction",
        "root",
        "inode",
        "escape",
        "duplicate",
    ] {
        let (temporary, home, receipt) = fixture();
        let run = begin_run(
            &home,
            temporary.path(),
            None,
            crate::GenerationId::new(1),
            RunOp::Apply,
        )
        .unwrap();
        directory::persist(&run, &receipt, Direction::Forward).unwrap();
        let path = temporary
            .path()
            .join("journal")
            .join(directory::record_name(&receipt));
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match corruption {
            "version" => {
                value.as_object_mut().unwrap().remove("v");
            }
            "direction" => value["direction"]["kind"] = serde_json::json!("unknown"),
            "root" => {
                value["transition"]["directories"]
                    .as_object_mut()
                    .unwrap()
                    .remove("");
            }
            "inode" => value["transition"]["directories"][""]["inode"] = serde_json::Value::Null,
            "escape" => value["transition"]["leaves"] = serde_json::json!(["../outside"]),
            _ => {}
        }
        let mut bytes = serde_json::to_string(&value).unwrap();
        if corruption == "duplicate" {
            let root = serde_json::to_string(&value["transition"]["directories"][""]).unwrap();
            bytes = bytes.replacen(
                "\"directories\":{",
                &format!("\"directories\":{{\"\":{root},"),
                1,
            );
        }
        std::fs::write(path, bytes).unwrap();
        assert!(reconcile(&home, temporary.path()).is_err(), "{corruption}");
        assert_eq!(
            std::fs::read_link(&receipt.destination).unwrap(),
            receipt.predecessor_target
        );
        assert_eq!(pending_recovery(&home).unwrap().unwrap().quarantined, 1);
    }
}
