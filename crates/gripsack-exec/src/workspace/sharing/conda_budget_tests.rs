//! Exercise the same original-archive preflight used before production import
//! and retained admission. No executable helper substitute is involved.
use super::*;
use crate::workspace::sharing::{
    closure::Closure,
    format::*,
    target::{PortableTarget, RuntimeAbi},
};
use gripsack_fetch::FetchLimits;
use std::num::{NonZeroU64, NonZeroUsize};

fn original(blobs: &Path, name: &str, size: usize, count: usize) -> (PortableConda, u64) {
    let version = if name == "python" { "3.12.7" } else { "1.0" };
    let mut tar = tar::Builder::new(Vec::new());
    let mut append = |path: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        tar.append_data(&mut header, path, bytes).unwrap();
    };
    append(
        "info/index.json",
        &serde_json::to_vec(&json!({
            "name":name,"version":version,"build":"0","build_number":0
        }))
        .unwrap(),
    );
    let paths: Vec<_> = (0..count)
        .map(|index| {
            json!({
                "_path":format!("share/file-{index}"),"path_type":"hardlink"
            })
        })
        .collect();
    append(
        "info/paths.json",
        &serde_json::to_vec(&json!({"paths_version":1,"paths":paths})).unwrap(),
    );
    let bytes = vec![0; size];
    for index in 0..count {
        append(&format!("share/file-{index}"), &bytes);
    }
    let expanded = tar.into_inner().unwrap();
    let expanded_size = expanded.len() as u64;
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(&expanded).unwrap();
    let compressed = encoder.finish().unwrap();
    let digest = Sha256Digest::of(&compressed).to_string();
    std::fs::write(blobs.join(&digest), &compressed).unwrap();
    let mut portable = fixture();
    let package = &mut portable.lock.packages[0];
    package.name = name.into();
    package.version = version.into();
    package.url = format!("https://example.invalid/channel/linux-64/{name}-{version}-0.tar.bz2");
    package.sha256 = digest.clone();
    package.size = Some(compressed.len() as u64);
    portable.archives[0].sha256 = digest;
    portable.archives[0].size = compressed.len() as u64;
    (portable, expanded_size)
}

fn manifest(records: &[PortableConda], native_bytes: u64) -> PortableManifest {
    let mut components: Vec<_> = records
        .iter()
        .enumerate()
        .map(|(index, record)| Component {
            id: format!("conda-{index}"),
            payload: None,
            tree: None,
            runtime: vec![],
            conda: Some(record.clone()),
            external: None,
            inventory: vec![],
        })
        .collect();
    if native_bytes != 0 {
        components.push(Component {
            id: "native".into(),
            payload: Some(BlobDescriptor {
                sha256: "a".repeat(64),
                size: 1,
                media_type: TREE_MEDIA_TYPE.into(),
            }),
            tree: Some("b".repeat(64)),
            runtime: vec![],
            conda: None,
            external: None,
            inventory: vec![PortableEntry {
                path: "data".into(),
                mode: 0o644,
                kind: EntryKind::File {
                    sha256: "c".repeat(64),
                    size: native_bytes,
                },
            }],
        });
    }
    PortableManifest {
        version: 1,
        namespace: "fixtures/budgets".into(),
        publisher: "fixture".into(),
        target: PortableTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            abi: RuntimeAbi::Glibc {
                minimum: "2.17".into(),
            },
            minimum_kernel: "3.2".into(),
            cpu: "x86_64".into(),
        },
        layout: RuntimeLayout::Environment {},
        roots: components
            .iter()
            .map(|component| component.id.clone())
            .collect(),
        components,
        commands: BTreeMap::new(),
        environment: BTreeMap::new(),
        licenses: vec![],
        redistribution_authorized: true,
    }
}

fn closure(root: &Path, manifest: &PortableManifest) -> Closure {
    std::fs::write(root.join("manifest.json"), manifest.encode().unwrap()).unwrap();
    // Closure decoding precedes signature admission; these tests exercise only
    // resource admission, never claim publisher authorization from this record.
    std::fs::write(root.join("envelope.json"), b"{}").unwrap();
    Closure::read(&gripsack_fs::open(root).unwrap(), FetchLimits::default()).unwrap()
}

fn limits(bytes: u64, entries: usize) -> FetchLimits {
    FetchLimits {
        expanded_bytes: NonZeroU64::new(bytes).unwrap(),
        archive_entries: NonZeroUsize::new(entries).unwrap(),
        ..FetchLimits::default()
    }
}

#[test]
fn compressed_conda_and_entry_floods_fail_during_production_preflight() {
    let home = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let (large, _) = original(blobs.path(), "large", 2 * 1024 * 1024, 1);
    assert!(large.archives[0].size < 16 * 1024);
    let graph = closure(home.path(), &manifest(&[large], 0));
    let error = graph
        .preflight_conda(home.path(), Some(blobs.path()), limits(16 * 1024, 100))
        .err()
        .expect("compressed payload must exceed remaining expansion")
        .to_string();
    assert!(error.contains("byte bound"), "{error}");
    let (many, _) = original(blobs.path(), "many", 0, 64);
    let graph = closure(home.path(), &manifest(&[many], 0));
    let error = graph
        .preflight_conda(home.path(), Some(blobs.path()), limits(1024 * 1024, 16))
        .err()
        .expect("original entries must count despite empty signed Conda inventory")
        .to_string();
    assert!(error.contains("entry count"), "{error}");
}

#[test]
fn generated_receipts_and_directories_consume_remaining_budget() {
    let home = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let (record, raw_bytes) = original(blobs.path(), "generated", 1024, 1);
    let graph = closure(home.path(), &manifest(&[record], 0));
    let error = graph
        .preflight_conda(home.path(), Some(blobs.path()), limits(raw_bytes, 100))
        .err()
        .expect("original tar alone leaves no room for generated receipt")
        .to_string();
    assert!(error.contains("byte bound"), "{error}");
    // Three raw members + root/component bookkeeping leave no room for the
    // generated conda-meta receipt and implicit installation directories.
    assert!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), limits(1024 * 1024, 6))
            .is_err()
    );
    assert_eq!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), FetchLimits::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn mixed_members_share_one_budget_on_cold_warm_and_retained_preflight() {
    let home = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let (first, _) = original(blobs.path(), "first", 64 * 1024, 1);
    let (second, _) = original(blobs.path(), "second", 64 * 1024, 1);
    let bound = limits(128 * 1024, 100);
    for record in [&first, &second] {
        let single = closure(
            home.path(),
            &manifest(std::slice::from_ref(record), 32 * 1024),
        );
        assert!(
            single
                .preflight_conda(home.path(), Some(blobs.path()), bound)
                .is_ok()
        );
    }
    let graph = closure(
        home.path(),
        &manifest(&[first.clone(), second.clone()], 32 * 1024),
    );
    assert!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), bound)
            .is_err()
    );
    for record in [&first, &second] {
        let descriptor = &record.archives[0];
        let object = conda::archive_object(home.path(), &descriptor.sha256);
        std::fs::create_dir_all(&object).unwrap();
        std::fs::copy(
            blobs.path().join(&descriptor.sha256),
            object.join("archive"),
        )
        .unwrap();
    }
    // Warm originals cannot make an oversized incoming closure admissible.
    assert!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), bound)
            .is_err()
    );
    assert!(graph.preflight_conda(home.path(), None, bound).is_err());
    assert_eq!(
        graph
            .preflight_conda(home.path(), None, FetchLimits::default())
            .unwrap()
            .len(),
        2
    );
    std::fs::remove_file(
        conda::archive_object(home.path(), &second.archives[0].sha256).join("archive"),
    )
    .unwrap();
    assert!(
        graph
            .preflight_conda(home.path(), None, FetchLimits::default())
            .is_err()
    );
}

#[test]
fn shared_nested_environment_charges_realizations_not_reference_multiplicity() {
    let home = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let (record, _) = original(blobs.path(), "nested", 64 * 1024, 1);
    let child = manifest(std::slice::from_ref(&record), 32 * 1024);
    let identity = child.digest().unwrap();
    let mut parent = manifest(&[record], 0);
    parent.components = ["left", "right"]
        .into_iter()
        .map(|id| Component {
            id: id.into(),
            payload: None,
            tree: None,
            runtime: vec![],
            conda: None,
            inventory: vec![],
            external: Some(ExternalReference {
                manifest: identity.clone(),
                namespace: child.namespace.clone(),
                publisher: child.publisher.clone(),
            }),
        })
        .collect();
    parent.roots = vec!["left".into(), "right".into()];
    let directory = home.path().join("manifests").join(&identity);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("manifest.json"), child.encode().unwrap()).unwrap();
    std::fs::write(directory.join("envelope.json"), b"{}").unwrap();
    let graph = closure(home.path(), &parent);
    assert_eq!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), limits(128 * 1024, 100))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn conda_wheels_and_generated_scripts_share_the_originals_budget() {
    use base64::Engine;
    let home = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let (mut portable, conda_bytes) = original(blobs.path(), "python", 4096, 1);
    let members = [
        (
            "demo-1.0.dist-info/METADATA",
            b"Metadata-Version: 2.3\nName: demo\nVersion: 1.0\n\n".to_vec(),
        ),
        (
            "demo-1.0.dist-info/WHEEL",
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n\n".to_vec(),
        ),
        ("demo.py", vec![b' '; 64 * 1024]),
        (
            "demo-1.0.dist-info/entry_points.txt",
            b"[console_scripts]\ndemo = demo:main\n".to_vec(),
        ),
    ];
    let mut rows = String::new();
    for (name, bytes) in &members {
        let digest = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(bytes));
        rows.push_str(&format!("{name},sha256={digest},{}\n", bytes.len()));
    }
    rows.push_str("demo-1.0.dist-info/RECORD,,\n");
    let wheel_bytes = members
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .sum::<u64>()
        + rows.len() as u64;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
    for (name, bytes) in members {
        zip.start_file(name, options).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.start_file("demo-1.0.dist-info/RECORD", options)
        .unwrap();
    zip.write_all(rows.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let digest = Sha256Digest::of(&bytes).to_string();
    std::fs::write(blobs.path().join(&digest), &bytes).unwrap();
    portable.lock.wheels.push(
        serde_json::from_value(json!({
            "name":"demo", "version":"1.0", "filename":"demo-1.0-py3-none-any.whl",
            "url":"https://example.invalid/demo-1.0-py3-none-any.whl",
            "sha256":digest, "size":bytes.len(), "requires_dist":[]
        }))
        .unwrap(),
    );
    portable.archives.push(BlobDescriptor {
        sha256: digest,
        size: bytes.len() as u64,
        media_type: WHEEL_MEDIA_TYPE.into(),
    });
    portable
        .archives
        .sort_by(|left, right| left.sha256.cmp(&right.sha256));
    let graph = closure(home.path(), &manifest(&[portable], 0));
    // All originals fit exactly, but their generated scripts, normalized RECORD,
    // Conda/wheel receipts and installation directories do not.
    let error = graph
        .preflight_conda(
            home.path(),
            Some(blobs.path()),
            limits(conda_bytes + wheel_bytes, 100),
        )
        .err()
        .expect("generated wheel and Conda costs must share the original budget")
        .to_string();
    assert!(
        error.contains("bound") || error.contains("acquisition limits"),
        "{error}"
    );
    assert_eq!(
        graph
            .preflight_conda(home.path(), Some(blobs.path()), FetchLimits::default())
            .unwrap()
            .len(),
        1
    );
}
