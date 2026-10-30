use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

fn assert_ok<T>(result: HResult<T>) -> T {
    result.unwrap_or_else(|failure| panic!("{}: {}", failure.code, failure.message))
}

fn closure(wheel: LockedWheel) -> LockedCondaEnvironment {
    serde_json::from_value(serde_json::json!({
        "platform": "linux-64", "channels": ["https://conda.anaconda.org/conda-forge"],
        "channel_priority": "strict", "system_requirements": {},
        "packages": [{"name":"python","version":"3.12.8","build":"h123_cpython",
            "build_number":0,"subdir":"linux-64","channel":"https://conda.anaconda.org/conda-forge",
            "url":"https://conda.anaconda.org/conda-forge/linux-64/python-3.12.8-h123_cpython.conda",
            "sha256":"00".repeat(32)}],
        "wheels": [wheel], "materializer": {"bytecode":"suppress","receipt":"normalized_conda_meta"}
    })).unwrap()
}

fn wheel_fixture(directory: &Path, extra: Option<(&str, &[u8])>) -> (LockedWheel, ArchiveRef) {
    let mut members = BTreeMap::<String, Vec<u8>>::from([
        (
            "demo/__init__.py".into(),
            b"def main():\n    print('offline-wheel')\n".to_vec(),
        ),
        (
            "demo-1.0.dist-info/METADATA".into(),
            b"Metadata-Version: 2.3\nName: demo\nVersion: 1.0\nRequires-Python: >=3.10\n\n"
                .to_vec(),
        ),
        (
            "demo-1.0.dist-info/WHEEL".into(),
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n\n".to_vec(),
        ),
        (
            "demo-1.0.dist-info/entry_points.txt".into(),
            b"[console_scripts]\ndemo = demo:main\n".to_vec(),
        ),
        (
            "demo-1.0.data/data/share/demo/data.txt".into(),
            b"retained data\n".to_vec(),
        ),
    ]);
    if let Some((path, bytes)) = extra {
        members.insert(path.into(), bytes.to_vec());
    }
    let mut record = String::new();
    for (path, bytes) in &members {
        record.push_str(&format!(
            "{path},sha256={},{}\n",
            URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)),
            bytes.len()
        ));
    }
    record.push_str("demo-1.0.dist-info/RECORD,,\n");
    members.insert("demo-1.0.dist-info/RECORD".into(), record.into_bytes());
    let path = directory.join("original-archive");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, bytes) in members {
        zip.start_file(name, SimpleFileOptions::default().unix_permissions(0o644))
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let bytes = fs::read(&path).unwrap();
    let sha256 = crate::hex_lower(&Sha256::digest(&bytes));
    let wheel = LockedWheel {
        name: "demo".into(),
        version: "1.0".into(),
        filename: "demo-1.0-py3-none-any.whl".into(),
        url: "https://example.test/demo-1.0-py3-none-any.whl".into(),
        sha256: sha256.clone(),
        size: Some(bytes.len() as u64),
        requires_python: Some(">=3.10".into()),
        requires_dist: Vec::new(),
    };
    (
        wheel,
        ArchiveRef {
            sha256,
            path: path.to_str().unwrap().into(),
        },
    )
}

#[test]
fn uv_installs_exact_independent_inventory_at_final_prefix_without_bytecode() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, archive) = wheel_fixture(
        temporary.path(),
        Some(("demo/__pycache__/old.pyc", b"suppressed")),
    );
    let closure = closure(wheel);
    let archives = BTreeMap::from([(archive.sha256.as_str(), &archive)]);
    let mut owners = HashMap::from([("bin/python".into(), "python".into())]);
    let staging = temporary.path().join("staging");
    fs::create_dir(&staging).unwrap();
    fs::create_dir(staging.join("bin")).unwrap();
    fs::write(
        staging.join("bin/python"),
        b"Conda interpreter is not executed by installer",
    )
    .unwrap();
    let final_prefix = Path::new("/a different/final prefix");
    let prepared = assert_ok(prepare(
        &closure,
        &archives,
        CondaInventory {
            packages: &[],
            paths: &[],
            python: None,
        },
        &mut owners,
        &BTreeSet::new(),
        final_prefix,
    ));
    let expected: Vec<_> = prepared.wheels[0]
        .inventory
        .iter()
        .map(|file| (file.path().to_owned(), file.bytes().to_vec(), file.mode()))
        .collect();
    let receipts = assert_ok(prepared.install(&staging, final_prefix));
    assert_eq!(receipts.len(), 1);
    for (path, bytes, mode) in expected {
        assert_eq!(
            fs::read(staging.join(&path)).unwrap(),
            bytes,
            "{}",
            path.display()
        );
        assert_eq!(
            fs::metadata(staging.join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            mode
        );
    }
    assert!(
        !staging
            .join("lib/python3.12/site-packages/demo/__pycache__")
            .exists()
    );
    let script = fs::read_to_string(staging.join("bin/demo")).unwrap();
    assert!(script.contains("/a different/final prefix/bin/python"));
    assert!(!script.contains(staging.to_str().unwrap()));
    assert_eq!(
        fs::read(staging.join("share/demo/data.txt")).unwrap(),
        b"retained data\n"
    );
    let installed: BTreeSet<_> = owners
        .iter()
        .filter(|(path, owner)| owner.as_str() == "wheel:demo" && !path.starts_with("conda-meta/"))
        .map(|(path, _)| path.clone())
        .collect();
    gripsack_conda::receipt::verify_wheel(
        fs::File::open(staging.join(&receipts[0].wheel_meta)).unwrap(),
        &closure.wheels[0],
        &installed,
        final_prefix.to_str().unwrap(),
    )
    .unwrap();
}

#[test]
fn conda_file_and_ancestor_collisions_reject_before_installation() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, archive) = wheel_fixture(temporary.path(), None);
    let closure = closure(wheel);
    let archives = BTreeMap::from([(archive.sha256.as_str(), &archive)]);
    for collision in [
        "bin/demo",
        "share/demo",
        "lib/python3.12/site-packages/demo/__init__.py",
    ] {
        let mut owners = HashMap::from([
            ("bin/python".into(), "python".into()),
            (collision.into(), "conda-owner".into()),
        ]);
        let result = prepare(
            &closure,
            &archives,
            CondaInventory {
                packages: &[],
                paths: &[],
                python: None,
            },
            &mut owners,
            &BTreeSet::new(),
            Path::new("/final"),
        );
        assert!(
            matches!(result, Err(failure) if failure.code == "clobber"),
            "{collision}"
        );
    }
}

#[test]
fn uv_temporary_data_footprint_cannot_clobber_conda() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, archive) = wheel_fixture(temporary.path(), None);
    let closure = closure(wheel);
    let archives = BTreeMap::from([(archive.sha256.as_str(), &archive)]);
    let mut owners = HashMap::from([
        ("bin/python".into(), "python".into()),
        (
            "lib/python3.12/site-packages/demo-1.0.data/data/share/demo/data.txt".into(),
            "conda-owner".into(),
        ),
    ]);
    let result = prepare(
        &closure,
        &archives,
        CondaInventory {
            packages: &[],
            paths: &[],
            python: None,
        },
        &mut owners,
        &BTreeSet::new(),
        Path::new("/final"),
    );
    assert!(matches!(result, Err(failure) if failure.code == "clobber"));
}

#[test]
fn arbitrary_publisher_scripts_reject_before_effects() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, archive) = wheel_fixture(
        temporary.path(),
        Some(("demo-1.0.data/scripts/run", b"#!/bin/sh\necho forbidden\n")),
    );
    let closure = closure(wheel);
    let archives = BTreeMap::from([(archive.sha256.as_str(), &archive)]);
    let mut owners = HashMap::from([("bin/python".into(), "python".into())]);
    assert!(
        prepare(
            &closure,
            &archives,
            CondaInventory {
                packages: &[],
                paths: &[],
                python: None
            },
            &mut owners,
            &BTreeSet::new(),
            Path::new("/final")
        )
        .is_err()
    );
}

#[test]
fn binary_tag_admission_binds_cpython_abi_and_explicit_libc_floor() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, _) = wheel_fixture(temporary.path(), None);
    let mut closure = closure(wheel);
    closure.wheels[0].filename = "demo-1.0-cp312-cp312-manylinux_2_17_x86_64.whl".into();
    assert!(validate_lock(&closure).is_err());
    closure.virtual_packages.push(
        serde_json::from_value(serde_json::json!({
            "name": "__glibc", "version": "2.40", "build": "0"
        }))
        .unwrap(),
    );
    assert!(
        validate_lock(&closure).is_err(),
        "historical solve facts must not grant runtime authority"
    );
    closure.system_requirements.virtual_packages.push(
        serde_json::from_value(serde_json::json!({
            "name": "__glibc", "minimum_version": "2.17"
        }))
        .unwrap(),
    );
    assert_ok(validate_lock(&closure));
    for filename in [
        "demo-1.0-cp311-cp311-manylinux_2_17_x86_64.whl",
        "demo-1.0-cp312-cp312-manylinux_2_28_x86_64.whl",
        "demo-1.0-cp312-cp312-manylinux_2_17_aarch64.whl",
        "demo-1.0-cp312-cp312t-manylinux_2_17_x86_64.whl",
    ] {
        closure.wheels[0].filename = filename.into();
        assert!(validate_lock(&closure).is_err(), "{filename}");
    }
    closure.wheels[0].filename = "demo-1.0-cp39-abi3-manylinux_2_17_x86_64.whl".into();
    assert_ok(validate_lock(&closure));
}

#[test]
fn closure_uses_actual_conda_distribution_metadata_and_extra_fixedpoint() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, _) = wheel_fixture(temporary.path(), None);
    let mut closure = closure(wheel);
    closure.wheels[0].requires_dist =
        vec!["different_project[fast]>=2; sys_platform == 'linux'".into()];
    assert!(gripsack_conda::wheels::validate_dependencies(&closure, &[]).is_err());
    let metadata = vec![
        ("lib/python3.12/site-packages/different_project-2.0.dist-info/METADATA".into(),
            b"Metadata-Version: 2.3\nName: Different.Project\nVersion: 2.0\nRequires-Dist: accelerator>=1; extra == 'fast'\n\n".to_vec()),
    ];
    assert!(
        gripsack_conda::wheels::validate_dependencies(&closure, &metadata).is_err(),
        "an activated dependency extra requires its complete closure"
    );
    let mut complete = metadata;
    complete.push((
        "lib/python3.12/site-packages/accelerator-1.0.dist-info/METADATA".into(),
        b"Metadata-Version: 2.3\nName: accelerator\nVersion: 1.0\n\n".to_vec(),
    ));
    gripsack_conda::wheels::validate_dependencies(&closure, &complete).unwrap();
    complete.push((
        "lib/python3.12/site-packages/demo-1.0.dist-info/METADATA".into(),
        b"Metadata-Version: 2.3\nName: demo\nVersion: 1.0\n\n".to_vec(),
    ));
    assert!(
        gripsack_conda::wheels::validate_dependencies(&closure, &complete).is_err(),
        "Conda and wheel must never both own one normalized Python distribution"
    );
}

#[test]
fn string_markers_preserve_conditional_dependency_admission() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, _) = wheel_fixture(temporary.path(), None);
    let mut closure = closure(wheel);
    for (requirement, admitted) in [
        ("demo; sys_platform == 'linux'", true),
        ("missing; sys_platform == 'win32'", true),
        ("missing; sys_platform != 'linux'", true),
        ("missing; sys_platform == 'linux'", false),
        ("missing; sys_platform != 'win32'", false),
        ("demo; sys_platform ~= 'linux'", false),
        ("demo; python_version == 'not-a-version'", false),
        ("demo; os_name == sys_platform", false),
    ] {
        closure.wheels[0].requires_dist = vec![requirement.into()];
        assert_eq!(
            gripsack_conda::wheels::validate_dependencies(&closure, &[]).is_ok(),
            admitted,
            "{requirement}"
        );
    }
}

#[test]
fn dependency_versions_python_constraints_and_unretained_urls_reject() {
    let temporary = tempfile::tempdir().unwrap();
    let (wheel, _) = wheel_fixture(temporary.path(), None);
    let mut closure = closure(wheel);
    for requirement in [
        "missing>=1",
        "demo>=2",
        "demo @ https://elsewhere.test/demo.whl",
        "demo; platform_release == '6.1'",
    ] {
        closure.wheels[0].requires_dist = vec![requirement.into()];
        assert!(
            gripsack_conda::wheels::validate_dependencies(&closure, &[]).is_err(),
            "{requirement}"
        );
    }
    closure.wheels[0].requires_dist.clear();
    closure.wheels[0].requires_python = Some(">=3.13".into());
    assert!(validate_lock(&closure).is_err());
}

/// Explicit native fixture lane, paired with e2e/test_package_native_wheel.py.
/// No solve, download, compiler, or ambient Python is used by this regression.
#[tokio::test]
#[ignore = "requires explicitly prepared GRIPSACK_TEST_NATIVE_WHEEL_BUNDLE"]
async fn prepared_native_wheel_rejects_incompatible_abi_before_linking() {
    let bundle = std::path::PathBuf::from(
        std::env::var_os("GRIPSACK_TEST_NATIVE_WHEEL_BUNDLE")
            .expect("prepare e2e/test_package_native_wheel.py and set its bundle path"),
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let mut closure: LockedCondaEnvironment =
        serde_json::from_value(manifest["components"][0]["conda"]["lock"].clone()).unwrap();
    let python = closure
        .packages
        .iter()
        .find(|record| record.name == "python")
        .unwrap();
    assert!(python.version.starts_with("3.12."));
    let record = closure
        .wheels
        .iter_mut()
        .find(|record| record.name == "gripsack-native-wheel")
        .unwrap();
    let old_digest = record.sha256.clone();
    let mut original =
        zip::ZipArchive::new(fs::File::open(bundle.join("blobs").join(&old_digest)).unwrap())
            .unwrap();
    let mut members = BTreeMap::<String, Vec<u8>>::new();
    for index in 0..original.len() {
        let mut entry = original.by_index(index).unwrap();
        assert!(!entry.is_dir());
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        members.insert(name, bytes);
    }
    let extension = &members["gripsack_native_wheel/_native.abi3.so"];
    assert_eq!(&extension[..6], b"\x7fELF\x02\x01");
    assert_eq!(u16::from_le_bytes([extension[18], extension[19]]), 62);
    let dist = "gripsack_native_wheel-1.0.dist-info";
    let tag = "cp313-cp313-manylinux_2_17_x86_64";
    members.insert(
        format!("{dist}/WHEEL"),
        format!("Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: {tag}\n").into_bytes(),
    );
    members.remove(&format!("{dist}/RECORD"));
    let mut csv = String::new();
    for (name, bytes) in &members {
        csv.push_str(&format!(
            "{name},sha256={},{}\n",
            URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)),
            bytes.len(),
        ));
    }
    csv.push_str(&format!("{dist}/RECORD,,\n"));
    members.insert(format!("{dist}/RECORD"), csv.into_bytes());
    let temporary = tempfile::tempdir().unwrap();
    let archive_path = temporary.path().join("incompatible-original.whl");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
    for (name, bytes) in members {
        zip.start_file(name, SimpleFileOptions::default().unix_permissions(0o644))
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let bytes = fs::read(&archive_path).unwrap();
    record.filename = format!("gripsack_native_wheel-1.0-{tag}.whl");
    record.url = format!("https://fixture.invalid/{}", record.filename);
    record.sha256 = crate::hex_lower(&Sha256::digest(&bytes));
    record.size = Some(bytes.len() as u64);
    // The incompatible archive is independently valid: this is an ABI refusal,
    // not a corrupted ZIP/RECORD/hash or lock/archive identity mismatch.
    WheelArchive::admit(&archive_path, record).unwrap();
    let changed_digest = record.sha256.clone();
    let archives = closure
        .packages
        .iter()
        .map(|record| &record.sha256)
        .chain(closure.wheels.iter().map(|record| &record.sha256))
        .map(|digest| ArchiveRef {
            sha256: digest.clone(),
            path: if digest == &changed_digest {
                archive_path.to_str().unwrap().to_owned()
            } else {
                bundle
                    .join("blobs")
                    .join(digest)
                    .to_str()
                    .unwrap()
                    .to_owned()
            },
        })
        .collect();
    let staging = temporary.path().join("untouched-staging");
    fs::create_dir(&staging).unwrap();
    let final_prefix = temporary.path().join("final-prefix");
    let request = gripsack_conda::protocol::MaterializeRequest {
        attempt: 1,
        lock_digest: gripsack_ir::workspace_model::identity::conda_closure_digest(&closure)
            .to_string(),
        closure: std::borrow::Cow::Owned(closure),
        final_prefix: final_prefix.to_str().unwrap().into(),
        staging_dir: staging.to_str().unwrap().into(),
        archives,
    };
    let error = match crate::materialize(request).await {
        Ok(_) => panic!("incompatible native wheel unexpectedly materialized"),
        Err(error) => error,
    };
    assert_eq!(error.code, "invalid_wheel");
    assert!(error.message.contains("incompatible"), "{}", error.message);
    assert!(fs::read_dir(staging).unwrap().next().is_none());
    assert!(!final_prefix.exists());
}
