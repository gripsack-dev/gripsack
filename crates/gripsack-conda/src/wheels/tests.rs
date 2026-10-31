use super::*;

fn fixture(
    extra: &[(&str, &[u8], u32)],
    bad_record: bool,
) -> (tempfile::NamedTempFile, LockedWheel) {
    let mut members = vec![
        (
            "demo-1.0.dist-info/METADATA",
            b"Metadata-Version: 2.3\nName: demo\nVersion: 1.0\nRequires-Python: >=3.8\n\n".to_vec(),
            0o644,
        ),
        (
            "demo-1.0.dist-info/WHEEL",
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n\n".to_vec(),
            0o644,
        ),
        (
            "demo/__init__.py",
            b"def main(): return 0\n".to_vec(),
            0o644,
        ),
        (
            "demo-1.0.dist-info/entry_points.txt",
            b"[console_scripts]\ndemo = demo:main\n".to_vec(),
            0o644,
        ),
    ];
    members.extend(
        extra
            .iter()
            .map(|(name, bytes, mode)| (*name, bytes.to_vec(), *mode)),
    );
    let mut record = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    for (name, bytes, _) in &members {
        record
            .serialize((
                name,
                if bad_record && *name == "demo/__init__.py" {
                    "sha256=invalid".to_owned()
                } else {
                    hash(bytes)
                },
                bytes.len(),
            ))
            .unwrap();
    }
    record
        .serialize(("demo-1.0.dist-info/RECORD", "", ""))
        .unwrap();
    members.push((
        "demo-1.0.dist-info/RECORD",
        record.into_inner().unwrap(),
        0o644,
    ));
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut zip = zip::ZipWriter::new(file.as_file_mut());
        for (name, bytes, mode) in members {
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default().unix_permissions(mode),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    let bytes = fs::read(file.path()).unwrap();
    let lock = LockedWheel {
        name: "demo".into(),
        version: "1.0".into(),
        filename: "demo-1.0-py3-none-any.whl".into(),
        url: "https://example.invalid/demo-1.0-py3-none-any.whl".into(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        size: Some(bytes.len() as u64),
        requires_python: Some(">=3.8".into()),
        requires_dist: Vec::new(),
    };
    (file, lock)
}

#[test]
fn remaining_closure_bounds_stop_wheel_expansion_and_entry_admission() {
    let payload = vec![0; 2 * 1024 * 1024];
    let (file, record) = fixture(&[("demo/data", &payload, 0o644)], false);
    let error = WheelArchive::admit_bounded(file.path(), &record, 16 * 1024, 100)
        .unwrap_err()
        .to_string();
    assert!(error.contains("expanded wheel exceeds bound"), "{error}");
    let (file, record) = fixture(&[], false);
    let error = WheelArchive::admit_bounded(file.path(), &record, 1024 * 1024, 4)
        .unwrap_err()
        .to_string();
    assert!(error.contains("too many ZIP entries"), "{error}");
    let archive = WheelArchive::admit_bounded(file.path(), &record, 1024 * 1024, 5).unwrap();
    let installed = archive
        .installation_inventory("3.12", Path::new("/recipient/prefix"))
        .unwrap();
    let generated: Vec<_> = installed
        .iter()
        .filter(|file| file.is_generated())
        .collect();
    assert_eq!(
        generated.len(),
        2,
        "console script and regenerated RECORD are additional costs"
    );
    assert!(
        generated
            .iter()
            .any(|file| file.path() == Path::new("bin/demo"))
    );
}
#[test]
fn original_archive_and_record_hashes_are_independent() {
    let (file, mut lock) = fixture(&[], false);
    assert!(WheelArchive::admit(file.path(), &lock).is_ok());
    lock.sha256 = "0".repeat(64);
    assert!(
        WheelArchive::admit(file.path(), &lock)
            .unwrap_err()
            .to_string()
            .contains("SHA256")
    );
    let (file, lock) = fixture(&[], true);
    assert!(
        WheelArchive::admit(file.path(), &lock)
            .unwrap_err()
            .to_string()
            .contains("RECORD")
    );
}
#[test]
fn rejects_path_mode_and_startup_payloads() {
    for name in [
        "../escape",
        "/absolute",
        "demo/../escape",
        "demo\\escape",
        "evil.pth",
        "sitecustomize.py",
        "usercustomize/__init__.py",
        "demo-1.0.data/scripts/evil",
        "Demo/extra.py",
    ] {
        let (file, lock) = fixture(&[(name, b"import attack", 0o644)], false);
        assert!(WheelArchive::admit(file.path(), &lock).is_err(), "{name}");
    }
    let (file, lock) = fixture(&[("demo/writable.py", b"attack", 0o666)], false);
    assert!(WheelArchive::admit(file.path(), &lock).is_err());
}
#[test]
fn metadata_identity_and_dependency_fields_are_bound() {
    let (file, mut lock) = fixture(&[], false);
    lock.version = "2.0".into();
    assert!(WheelArchive::admit(file.path(), &lock).is_err());
    lock.version = "1.0".into();
    lock.requires_dist.push("undeclared".into());
    assert!(WheelArchive::admit(file.path(), &lock).is_err());
}
#[test]
fn final_prefix_scripts_and_record_are_derived_and_bytecode_suppressed() {
    let (file, lock) = fixture(
        &[("demo/__pycache__/cached.cpython-312.pyc", b"opaque", 0o644)],
        false,
    );
    let archive = WheelArchive::admit(file.path(), &lock).unwrap();
    assert!(
        !archive
            .extracted_dir()
            .join("demo/__pycache__/cached.cpython-312.pyc")
            .exists()
    );
    let inventory = archive
        .installation_inventory("3.12.8", Path::new("/final/prefix"))
        .unwrap();
    assert!(
        !inventory
            .iter()
            .any(|f| f.path().extension().is_some_and(|e| e == "pyc"))
    );
    let script = inventory
        .iter()
        .find(|f| f.path() == Path::new("bin/demo"))
        .unwrap();
    assert!(script.bytes().starts_with(b"#!/final/prefix/bin/python\n"));
    assert_eq!(script.mode(), 0o755);
    let record = inventory
        .iter()
        .find(|f| f.path().ends_with("demo-1.0.dist-info/RECORD"))
        .unwrap();
    let text = std::str::from_utf8(record.bytes()).unwrap();
    assert!(text.contains(&format!(
        "../../../bin/demo,{},{}",
        hash(script.bytes()),
        script.bytes().len()
    )));
    assert!(!text.contains("cached.cpython"));
}
#[test]
fn native_inspection_uses_url_filename_not_cas_basename() {
    let (file, lock) = fixture(&[], false);
    let source = WheelSource {
        name: lock.name.clone(),
        version: lock.version.clone(),
        url: lock.url.clone(),
        sha256: lock.sha256.clone(),
    };
    let frozen = inspect(file.path(), &source).unwrap();
    assert_eq!(frozen.filename, lock.filename);
    assert_eq!(frozen.requires_python, lock.requires_python);
}
#[test]
fn complete_record_inventory_required() {
    let mut files = BTreeMap::new();
    files.insert(
        "RECORD".into(),
        Member {
            bytes: Arc::new(b"RECORD,,\n".to_vec()),
            mode: 0o644,
        },
    );
    files.insert(
        "unlisted.py".into(),
        Member {
            bytes: Arc::new(b"payload".to_vec()),
            mode: 0o644,
        },
    );
    assert!(validate_record(&files, "RECORD").is_err());
}
#[test]
fn rejects_symlink_zip_members() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut writer = zip::ZipWriter::new(file.as_file_mut());
        writer
            .add_symlink(
                "demo/link",
                "../../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();
    }
    let bytes = fs::read(file.path()).unwrap();
    let error = WheelArchive::load(
        file.path(),
        "demo-1.0-py3-none-any.whl",
        &hex::encode(Sha256::digest(&bytes)),
        None,
        MAX_EXPANDED,
        MAX_ENTRIES,
    )
    .unwrap_err();
    assert!(error.to_string().contains("type or permissions"));
}
#[test]
fn spreading_cannot_overwrite_modules_or_claim_receipts() {
    for name in [
        "demo-1.0.data/purelib/demo/__init__.py",
        "demo-1.0.data/data/conda-meta/python.json",
        "demo-1.0.data/purelib/foreign-1.0.dist-info/METADATA",
    ] {
        let (file, lock) = fixture(&[(name, b"payload", 0o644)], false);
        let archive = WheelArchive::admit(file.path(), &lock).unwrap();
        assert!(
            archive
                .installation_inventory("3.12.8", Path::new("/final/prefix"))
                .is_err(),
            "{name}"
        );
    }
}
#[test]
fn long_prefix_uses_maintained_shell_quoting() {
    let (file, lock) = fixture(&[], false);
    let archive = WheelArchive::admit(file.path(), &lock).unwrap();
    let prefix = format!("/long prefix's/{}", "x".repeat(128));
    let inventory = archive
        .installation_inventory("3.12.8", Path::new(&prefix))
        .unwrap();
    let script = inventory
        .iter()
        .find(|f| f.path() == Path::new("bin/demo"))
        .unwrap();
    let executable = format!("{prefix}/bin/python");
    let expected = format!(
        "#!/bin/sh\n'''exec' '{}' \"$0\" \"$@\"\n' '''\n",
        uv_shell::escape_posix_for_single_quotes(&executable)
    );
    assert!(script.bytes().starts_with(expected.as_bytes()));
}
#[test]
fn admits_confined_path_only_pth_but_not_code_or_external_paths() {
    let (file, lock) = fixture(
        &[("demo.pth", b"# local import paths\n.\ndemo\n", 0o644)],
        false,
    );
    assert!(WheelArchive::admit(file.path(), &lock).is_ok());
    for bytes in [
        b"import os\n".as_slice(),
        b"import\tos\n",
        b"../outside\n",
        b"/outside\n",
    ] {
        let (file, lock) = fixture(&[("demo.pth", bytes, 0o644)], false);
        assert!(WheelArchive::admit(file.path(), &lock).is_err());
    }
}
