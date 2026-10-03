use super::*;
use gripsack_ir::workspace_v6::lock::LockedCondaPackage;
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::{PermissionsExt, symlink};

#[derive(Clone)]
struct Member {
    path: &'static str,
    bytes: Vec<u8>,
    link: Option<&'static str>,
    mode: u32,
}
impl Member {
    fn file(path: &'static str, bytes: &[u8]) -> Self {
        Self {
            path,
            bytes: bytes.to_vec(),
            link: None,
            mode: 0o644,
        }
    }
    fn link(path: &'static str, target: &'static str) -> Self {
        Self {
            path,
            bytes: Vec::new(),
            link: Some(target),
            mode: 0o777,
        }
    }
}
fn tar(members: &[Member]) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    for member in members {
        let mut header = tar::Header::new_gnu();
        header.set_mode(member.mode);
        header.set_size(member.bytes.len() as u64);
        if let Some(target) = member.link {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name(target).unwrap();
        }
        header.set_cksum();
        archive
            .append_data(&mut header, member.path, member.bytes.as_slice())
            .unwrap();
    }
    archive.into_inner().unwrap()
}
fn encode(info: &[Member], payload: &[Member], conda: bool) -> Vec<u8> {
    if conda {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("metadata.json", options).unwrap();
        zip.write_all(br#"{"conda_pkg_format_version":2}"#).unwrap();
        // Real conda-forge packages retain licenses in the pkg tar even though
        // paths.json does not install them. Physical tar partition is not path authority.
        let mut package = payload.to_vec();
        package.push(Member::file("info/licenses/LICENSE", b"fixture license"));
        for (name, entries) in [
            ("info-fixture.tar.zst", info),
            ("pkg-fixture.tar.zst", package.as_slice()),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(&zstd::stream::encode_all(tar(entries).as_slice(), 1).unwrap())
                .unwrap();
        }
        zip.finish().unwrap().into_inner()
    } else {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut entries = info.to_vec();
        entries.extend_from_slice(payload);
        encoder.write_all(&tar(&entries)).unwrap();
        encoder.finish().unwrap()
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    tree: PathBuf,
    archives: BTreeMap<String, PathBuf>,
    locked: LockedCondaEnvironment,
}
impl Fixture {
    fn new(payload: Vec<Member>, conda: bool, legacy: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let tree = temporary.path().join("tree");
        fs::create_dir(&tree).unwrap();
        let index = json!({"name":"fixture","version":"1.0","build":"0","build_number":0});
        let mut info = vec![Member::file(
            "info/index.json",
            &serde_json::to_vec(&index).unwrap(),
        )];
        if legacy {
            info.push(Member::file(
                "info/files",
                payload
                    .iter()
                    .map(|entry| entry.path)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes(),
            ));
        } else {
            let paths: Vec<_> = payload.iter().map(|entry| json!({"_path":entry.path,"path_type":if entry.link.is_some() {"softlink"} else {"hardlink"}})).collect();
            info.push(Member::file(
                "info/paths.json",
                &serde_json::to_vec(&json!({"paths_version":1,"paths":paths})).unwrap(),
            ));
        }
        let bytes = encode(&info, &payload, conda);
        let digest = gripsack_process::Sha256Digest::of(&bytes).to_string();
        let path = temporary.path().join("archive");
        fs::write(&path, bytes).unwrap();
        let record: LockedCondaPackage = serde_json::from_value(json!({
            "name":"fixture","version":"1.0","build":"0","build_number":0,"subdir":"linux-64",
            "channel":"https://example.invalid/channel/", "url":"https://example.invalid/channel/linux-64/fixture-1.0-0.conda",
            "sha256":digest, "depends":[], "constrains":[]
        })).unwrap();
        let files: BTreeSet<_> = payload.iter().map(|entry| entry.path).collect();
        fs::create_dir(tree.join("conda-meta")).unwrap();
        gripsack_conda::receipt::write(
            fs::File::create(tree.join("conda-meta/fixture-1.0-0.json")).unwrap(),
            &record,
            &files,
            "/final prefix",
        )
        .unwrap();
        for member in &payload {
            let path = tree.join(member.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if let Some(target) = member.link {
                symlink(target, path).unwrap();
            } else {
                fs::write(&path, &member.bytes).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(member.mode)).unwrap();
            }
        }
        let locked = serde_json::from_value(json!({
            "platform":"linux-64", "channels":["https://example.invalid/channel/"], "channel_priority":"strict",
            "packages":[record], "virtual_packages":[], "system_requirements":{},
            "materializer":{"bytecode":"suppress","receipt":"normalized_conda_meta"}
        })).unwrap();
        Self {
            _temporary: temporary,
            tree,
            archives: BTreeMap::from([(digest, path)]),
            locked,
        }
    }
    fn validate(&self) -> Result<ValidatedTree, ExecError> {
        validate_tree(
            "fixture",
            &self.locked,
            &self.archives,
            &self.tree,
            "/final prefix",
            FileModes::Staged,
        )
    }
    fn refused(&self, detail: &str) {
        match self.validate() {
            Err(error) => assert!(error.to_string().contains(detail), "{error}"),
            Ok(_) => panic!("unexpected admission: {detail}"),
        }
    }
}

#[test]
fn absent_digests_never_skip_original_bytes_in_either_archive_format_or_legacy_inventory() {
    for (conda, legacy) in [(true, false), (false, false), (false, true)] {
        let fixture = Fixture::new(vec![Member::file("share/data", b"original")], conda, legacy);
        fixture.validate().unwrap();
        fs::write(fixture.tree.join("share/data"), b"modified").unwrap();
        fixture.refused("original archive byte identity");
    }
}

#[test]
fn staged_inventory_modes_links_and_byte_bounds_are_independent_of_helper_claims() {
    let fixture = Fixture::new(vec![Member::file("data", b"original")], true, false);
    fixture.validate().unwrap();
    fs::create_dir(fixture.tree.join("extra")).unwrap();
    fixture.refused("unaccounted directory");
    fs::remove_dir(fixture.tree.join("extra")).unwrap();
    fs::write(fixture.tree.join("extra"), b"extra").unwrap();
    fixture.refused("unaccounted staged file");
    fs::remove_file(fixture.tree.join("extra")).unwrap();
    fs::set_permissions(fixture.tree.join("data"), fs::Permissions::from_mode(0o755)).unwrap();
    fixture.refused("permissions differ");
    fs::set_permissions(fixture.tree.join("data"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::hard_link(
        fixture.tree.join("data"),
        fixture._temporary.path().join("cache-hardlink"),
    )
    .unwrap();
    fixture.refused("shared hardlink forbidden");
    fs::remove_file(fixture._temporary.path().join("cache-hardlink")).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.tree.join("data"))
        .unwrap()
        .set_len(MAX_ENTRY_BYTES + 1)
        .unwrap();
    fixture.refused("staged file exceeds its byte bound");
}

#[test]
fn composed_links_cannot_escape_cycle_or_traverse_unowned_paths() {
    let fixture = Fixture::new(
        vec![
            Member::file("dir/sub/data", b"ok"),
            Member::link("alias", "dir/sub"),
            Member::link("good", "alias/data"),
        ],
        true,
        false,
    );
    fixture.validate().unwrap();
    let fixture = Fixture::new(
        vec![
            Member::file("dir/sub/data", b"ok"),
            Member::link("alias", "dir/sub"),
            Member::link("escape", "alias/../../../outside"),
        ],
        true,
        false,
    );
    fixture.refused("escapes its prefix");
    let fixture = Fixture::new(
        vec![
            Member::link("first", "second"),
            Member::link("second", "first"),
        ],
        false,
        false,
    );
    fixture.refused("cycles or exceeds");
    let fixture = Fixture::new(vec![Member::link("dangling", "missing")], false, false);
    fixture.refused("missing/non-directory");
}

#[test]
fn clobbered_packages_and_mutated_original_archives_are_refused() {
    let mut fixture = Fixture::new(vec![Member::file("data", b"original")], true, false);
    fixture
        .locked
        .packages
        .push(fixture.locked.packages[0].clone());
    fixture.refused("claimed by two packages");
    fixture.locked.packages.pop();
    fs::write(fixture.archives.values().next().unwrap(), b"forged archive").unwrap();
    fixture.refused("differs from its frozen SHA-256");
}

#[test]
fn tar_declared_member_and_extension_bounds_refuse_before_payload_allocation() {
    for (kind, size) in [
        (tar::EntryType::Regular, MAX_ENTRY_BYTES + 1),
        (tar::EntryType::GNULongName, 32 * 1024 + 1),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_path("oversized").unwrap();
        header.set_mode(0o644);
        header.set_entry_type(kind);
        header.set_size(size);
        header.set_cksum();
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(header.as_bytes()).unwrap();
        let bytes = encoder.finish().unwrap();
        let digest = gripsack_process::Sha256Digest::of(&bytes).to_string();
        let path = temporary.path().join("archive");
        fs::write(&path, bytes).unwrap();
        let error = match CondaArchive::open(&path, &digest) {
            Err(error) => error,
            Ok(_) => panic!("oversized archive admitted"),
        };
        assert!(
            error.to_string().contains("exceeds its byte bound"),
            "{error}"
        );
    }
}

#[test]
fn published_modes_follow_shared_sealing_and_never_admit_restored_write_bits() {
    let mut executable = Member::file("bin/tool", b"#!/bin/sh\nexit 0\n");
    executable.mode = 0o755;
    let fixture = Fixture::new(vec![executable], true, false);
    fixture.validate().unwrap();
    let receipt = Path::new("conda-meta/fixture-1.0-0.json");
    fs::set_permissions(
        fixture.tree.join(receipt),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(
        fixture.validate().is_err(),
        "generated receipts must not become executable"
    );
    fs::set_permissions(
        fixture.tree.join(receipt),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let published = fixture._temporary.path().join("published");
    gripsack_fs::publish_dir_at(&fixture.tree, &published).unwrap();
    let verify_published = || {
        validate_tree(
            "fixture",
            &fixture.locked,
            &fixture.archives,
            &published,
            "/final prefix",
            FileModes::Published,
        )
    };
    verify_published().unwrap();
    fs::set_permissions(published.join(receipt), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        verify_published().is_err(),
        "published receipts must remain sealed"
    );
    fs::set_permissions(published.join(receipt), fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(
        published.join("bin/tool"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(
        verify_published().is_err(),
        "published files must remain sealed"
    );
    fs::set_permissions(
        published.join("bin/tool"),
        fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    assert!(
        verify_published().is_err(),
        "publication must preserve all execute bits"
    );
}

#[test]
fn mach_runtime_receipt_records_absolute_host_needs_not_relative_install_names() {
    let mut commands = Vec::new();
    for (kind, offset, name) in [
        (0xcu32, 24usize, "/usr/lib/libSystem.B.dylib"),
        (0xcu32, 24usize, "@rpath/libprivate.dylib"),
        (0xeu32, 12usize, "/usr/lib/dyld"),
    ] {
        let size = (offset + name.len() + 1).next_multiple_of(8);
        let mut command = vec![0; size];
        command[..4].copy_from_slice(&kind.to_le_bytes());
        command[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        command[8..12].copy_from_slice(&(offset as u32).to_le_bytes());
        command[offset..offset + name.len()].copy_from_slice(name.as_bytes());
        commands.extend(command);
    }
    let mut binary = Vec::new();
    for word in [
        0xfeedfacfu32,
        0x01000007,
        3,
        2,
        3,
        commands.len() as u32,
        0,
        0,
    ] {
        binary.extend(word.to_le_bytes());
    }
    binary.extend(commands);
    let mut fixture = Fixture::new(vec![Member::file("bin/mach", &binary)], true, false);
    fixture.locked.platform = "osx-64".into();
    let record = &mut fixture.locked.packages[0];
    record.subdir = "osx-64".into();
    record.url = record.url.replace("/linux-64/", "/osx-64/");
    gripsack_conda::receipt::write(
        fs::File::create(fixture.tree.join("conda-meta/fixture-1.0-0.json")).unwrap(),
        record,
        &BTreeSet::from(["bin/mach"]),
        "/final prefix",
    )
    .unwrap();
    let validated = fixture.validate().unwrap();
    assert_eq!(validated.system.libraries, ["/usr/lib/libSystem.B.dylib"]);
    assert_eq!(validated.system.loaders, ["/usr/lib/dyld"]);
}
