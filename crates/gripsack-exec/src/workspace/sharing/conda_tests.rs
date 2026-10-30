use super::*;
use serde_json::json;

#[path = "conda_budget_tests.rs"]
mod budgets;

// These bytes exercise transfer/inventory admission only. They are not a
// runnable Conda package or a replacement for the core-approved helper.
const ARCHIVE_BYTES: &[u8] = b"retained original Conda archive input";

fn fixture() -> PortableConda {
    let digest = Sha256Digest::of(ARCHIVE_BYTES).to_string();
    let lock = serde_json::from_value(json!({
        "platform":"linux-64",
        "channels":["https://example.invalid/channel/"],
        "channel_priority":"strict",
        "system_requirements":{},
        "packages":[{
            "name":"fixture", "version":"1.0", "build":"0", "build_number":0,
            "subdir":"linux-64", "channel":"https://example.invalid/channel/",
            "url":"https://example.invalid/channel/linux-64/fixture-1.0-0.conda",
            "sha256":digest, "size":ARCHIVE_BYTES.len()
        }],
        "materializer":{"bytecode":"suppress","receipt":"normalized_conda_meta"}
    }))
    .unwrap();
    PortableConda {
        lock,
        archives: vec![BlobDescriptor {
            sha256: digest,
            size: ARCHIVE_BYTES.len() as u64,
            media_type: ARCHIVE_MEDIA_TYPE.into(),
        }],
        helper: BlobDescriptor {
            sha256: gripsack_conda::provision::approved_pin()
                .unwrap()
                .to_string(),
            size: 1,
            media_type: HELPER_MEDIA_TYPE.into(),
        },
        helper_notices: PortableConda::approved_helper_notices().clone(),
    }
}

fn refused(record: &PortableConda, blobs: &Path) -> String {
    match verify_inputs("fixture", record, blobs) {
        Ok(_) => panic!("unavailable or tampered offline inputs were admitted"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn incompatible_locked_host_requirements_fail_before_offline_inputs() {
    let mut package_requirement = fixture();
    package_requirement.lock.packages[0].depends = vec!["__linux >=9999".into()];
    // A producer's historical solve facts cannot authorize this recipient.
    package_requirement.lock.virtual_packages = serde_json::from_value(json!([
        {"name":"__linux", "version":"9999", "build":"0"}
    ]))
    .unwrap();
    let mut system_requirement = fixture();
    system_requirement.lock.system_requirements = serde_json::from_value(json!({
        "virtual_packages":[{"name":"__linux", "minimum_version":"9999"}]
    }))
    .unwrap();
    system_requirement.lock.virtual_packages = package_requirement.lock.virtual_packages.clone();
    for record in [package_requirement, system_requirement] {
        record.validate().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let import_error = refused(&record, empty.path());
        assert!(
            import_error.contains("__linux") && import_error.contains("9999"),
            "{import_error}"
        );
        let retained_error =
            inspect_retained("fixture", empty.path(), &record, &PreparedConda::new())
                .err()
                .expect("incompatible retained lock must be rejected")
                .to_string();
        assert!(
            retained_error.contains("__linux") && retained_error.contains("9999"),
            "{retained_error}"
        );
    }
}

#[test]
fn helper_notices_are_required_core_authority_and_transport_inputs() {
    let mut record = fixture();
    record.validate().unwrap();
    assert_eq!(record.helper_notices.len(), 4);
    assert_eq!(record.descriptors().count(), record.archives.len() + 1 + 4);
    record.helper_notices.remove("LICENSE-rattler");
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("core-approved inventory")
    );
    let mut record = fixture();
    record
        .helper_notices
        .get_mut("LICENSE-gripsack")
        .unwrap()
        .sha256 = "0".repeat(64);
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("core-approved inventory")
    );
}

#[test]
fn missing_and_tampered_helper_notices_are_rejected_offline() {
    let record = fixture();
    let blobs = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let descriptor = record.helper_notices.values().next().unwrap();
    let error = notices::spool(
        "fixture",
        &record.helper_notices,
        blobs.path(),
        staging.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains(&descriptor.sha256) && error.contains("unavailable"),
        "{error}"
    );
    std::fs::write(
        blobs.path().join(&descriptor.sha256),
        vec![b'x'; descriptor.size as usize],
    )
    .unwrap();
    let staging = tempfile::tempdir().unwrap();
    let error = notices::spool(
        "fixture",
        &record.helper_notices,
        blobs.path(),
        staging.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains(&descriptor.sha256) && error.contains("digest or size mismatch"),
        "{error}"
    );
}

#[test]
fn complete_lock_and_exact_core_pin_round_trip() {
    let record = fixture();
    record.validate().unwrap();
    let bytes = serde_json::to_vec(&record).unwrap();
    let decoded: PortableConda = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(record, decoded);
    assert_eq!(
        decoded.lock.packages[0].url,
        "https://example.invalid/channel/linux-64/fixture-1.0-0.conda"
    );
}

#[test]
fn missing_original_archive_is_named_without_url_fallback() {
    let record = fixture();
    let directory = tempfile::tempdir().unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.archives[0].sha256), "{error}");
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn complete_archives_still_require_bundled_helper() {
    let record = fixture();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(&record.archives[0].sha256),
        ARCHIVE_BYTES,
    )
    .unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.helper.sha256), "{error}");
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn equal_length_archive_tamper_is_rejected_before_helper_lookup() {
    let record = fixture();
    let directory = tempfile::tempdir().unwrap();
    let mut corrupt = ARCHIVE_BYTES.to_vec();
    corrupt[0] ^= 1;
    std::fs::write(directory.path().join(&record.archives[0].sha256), corrupt).unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.archives[0].sha256), "{error}");
    assert!(error.contains("digest or size mismatch"), "{error}");
}

#[test]
fn helper_tamper_cannot_be_authorized_by_bundle_descriptor() {
    let mut record = fixture();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(&record.archives[0].sha256),
        ARCHIVE_BYTES,
    )
    .unwrap();
    std::fs::write(directory.path().join(&record.helper.sha256), b"x").unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.helper.sha256), "{error}");
    assert!(error.contains("digest or size mismatch"), "{error}");
    record.helper.sha256 = Sha256Digest::of(b"x").to_string();
    let error = record.validate().unwrap_err().to_string();
    assert!(error.contains("core-approved pin"), "{error}");
}

#[test]
fn archive_inventory_must_be_exact_and_canonical() {
    let mut record = fixture();
    record.archives.clear();
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("complete locked closure")
    );
    let mut record = fixture();
    record.archives.push(record.archives[0].clone());
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("canonical unique")
    );
    let mut record = fixture();
    record.archives[0].size += 1;
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("size differs from its lock")
    );
}

#[test]
fn original_bytes_are_copied_without_transform_or_shared_inode() {
    let record = fixture();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("original");
    let destination = directory.path().join("private-spool");
    std::fs::write(&source, ARCHIVE_BYTES).unwrap();
    verify_copy(
        "fixture",
        &source,
        &record.archives[0],
        &mut File::create(&destination).unwrap(),
    )
    .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), ARCHIVE_BYTES);
    std::fs::write(&destination, b"changed spool").unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), ARCHIVE_BYTES);
}

#[test]
fn symlinked_archive_and_oversized_descriptor_are_rejected() {
    let record = fixture();
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    std::fs::write(&original, ARCHIVE_BYTES).unwrap();
    std::os::unix::fs::symlink(&original, directory.path().join(&record.archives[0].sha256))
        .unwrap();
    assert!(refused(&record, directory.path()).contains("unavailable"));
    let mut oversized = record;
    oversized.helper.size = MAX_PAYLOAD_BYTES + 1;
    assert!(
        oversized
            .validate()
            .unwrap_err()
            .to_string()
            .contains("payload bound")
    );
}

#[test]
fn wheel_source_and_automation_fields_are_not_silently_discarded() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value["lock"]["sdists"] = json!([]);
    assert!(serde_json::from_value::<PortableConda>(value).is_err());
    let mut value = serde_json::to_value(fixture()).unwrap();
    value["post_install"] = json!(["publisher command"]);
    assert!(serde_json::from_value::<PortableConda>(value).is_err());
}

fn wheel_fixture() -> PortableConda {
    let mut record = fixture();
    let python = &mut record.lock.packages[0];
    python.name = "python".into();
    python.version = "3.12.7".into();
    python.url = "https://example.invalid/channel/linux-64/python-3.12.7-0.conda".into();
    let digest = Sha256Digest::of(b"original wheel bytes").to_string();
    record.lock.wheels.push(
        serde_json::from_value(json!({
            "name": "gripsack-offline-wheel",
            "version": "1.0",
            "filename": "gripsack_offline_wheel-1.0-py3-none-any.whl",
            "url": "https://example.invalid/gripsack_offline_wheel-1.0-py3-none-any.whl",
            "sha256": digest,
            "size": 20,
            "requires_python": ">=3.8",
            "requires_dist": []
        }))
        .unwrap(),
    );
    record.archives.push(BlobDescriptor {
        sha256: digest,
        size: 20,
        media_type: WHEEL_MEDIA_TYPE.into(),
    });
    record
        .archives
        .sort_by(|left, right| left.sha256.cmp(&right.sha256));
    record
}

#[test]
fn wheel_original_is_required_by_the_complete_offline_inventory() {
    let record = wheel_fixture();
    record.validate().unwrap();
    let bytes = serde_json::to_vec(&record).unwrap();
    assert_eq!(
        serde_json::from_slice::<PortableConda>(&bytes).unwrap(),
        record
    );
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(&record.lock.packages[0].sha256),
        ARCHIVE_BYTES,
    )
    .unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.lock.wheels[0].sha256), "{error}");
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn wheel_tamper_is_rejected_before_helper_execution() {
    let record = wheel_fixture();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(&record.lock.packages[0].sha256),
        ARCHIVE_BYTES,
    )
    .unwrap();
    std::fs::write(
        directory.path().join(&record.lock.wheels[0].sha256),
        b"tampered wheel bytes",
    )
    .unwrap();
    let error = refused(&record, directory.path());
    assert!(error.contains(&record.lock.wheels[0].sha256), "{error}");
    assert!(error.contains("digest or size mismatch"), "{error}");
}

#[test]
fn wheel_descriptor_cannot_be_relabelled_as_conda_archive() {
    let mut record = wheel_fixture();
    let digest = record.lock.wheels[0].sha256.clone();
    record
        .archives
        .iter_mut()
        .find(|entry| entry.sha256 == digest)
        .unwrap()
        .media_type = ARCHIVE_MEDIA_TYPE.into();
    assert!(
        record
            .validate()
            .unwrap_err()
            .to_string()
            .contains("media type")
    );
}
