use super::*;
use crate::plan::{Architecture, LinuxOs};
use flate2::{Compression, GzBuilder};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Seek, Write};

fn blob_hash(bytes: &[u8]) -> String { BlobDigest::from_bytes(Sha256::digest(bytes).into()).to_string() }
fn expected() -> ImageConfig { ImageConfig { entrypoint:vec![],args:vec![],env:vec![],cwd:"/".into(),user:"0:0".into() } }
fn platform() -> Platform { Platform { os:LinuxOs::Linux,architecture:Architecture::Amd64 } }
fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8], kind: tar::EntryType, target: Option<&str>) {
    let mut header = tar::Header::new_ustar();
    header.set_path(name).unwrap();
    header.set_entry_type(kind);
    header.set_size(data.len() as u64);
    header.set_uid(1000); header.set_gid(1000); header.set_mode(0o644); header.set_mtime(0);
    if let Some(target) = target { header.set_link_name(target).unwrap(); }
    header.set_cksum();
    builder.append(&header, data).unwrap();
}
fn layer(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new()); build(&mut tar); tar.finish().unwrap(); tar.into_inner().unwrap()
}

// Test-only malformed-input construction, not an image production path. Every
// semantic mutant below rebuilds all affected descriptor hashes and lengths.
fn image(layers: Vec<Vec<u8>>, mutate: impl FnOnce(&mut Value, &mut Value, &mut BTreeMap<String, Vec<u8>>)) -> File {
    let mut blobs = BTreeMap::new();
    let mut layer_descriptors = Vec::new();
    let mut diff_ids = Vec::new();
    for layer in layers {
        diff_ids.push(blob_hash(&layer));
        let mut encoder = GzBuilder::new().mtime(0).write(Vec::new(), Compression::new(6));
        encoder.write_all(&layer).unwrap();
        let compressed = encoder.finish().unwrap();
        let digest = blob_hash(&compressed);
        layer_descriptors.push(json!({"mediaType":model::LAYER_MEDIA,"digest":digest,"size":compressed.len()}));
        blobs.insert(digest, compressed);
    }
    let mut config = json!({"created":model::EPOCH,"architecture":"amd64","os":"linux","config":{"WorkingDir":"/","User":"0:0"},"rootfs":{"type":"layers","diff_ids":diff_ids},"history":layer_descriptors.iter().map(|_| json!({"created":model::EPOCH,"created_by":"fixture"})).collect::<Vec<_>>()});
    let mut manifest = json!({"schemaVersion":2,"mediaType":model::MANIFEST_MEDIA,"layers":layer_descriptors});
    mutate(&mut config, &mut manifest, &mut blobs);
    let config = serde_json::to_vec(&config).unwrap();
    let config_digest = blob_hash(&config);
    manifest["config"] = json!({"mediaType":model::CONFIG_MEDIA,"digest":config_digest,"size":config.len()});
    blobs.insert(config_digest, config);
    let manifest = serde_json::to_vec(&manifest).unwrap();
    let manifest_digest = blob_hash(&manifest);
    let index = json!({"schemaVersion":2,"mediaType":model::INDEX_MEDIA,"manifests":[{"mediaType":model::MANIFEST_MEDIA,"digest":manifest_digest,"size":manifest.len()}]});
    blobs.insert(manifest_digest, manifest);
    let mut tar = tar::Builder::new(Vec::new());
    append(&mut tar,"oci-layout",br#"{"imageLayoutVersion":"1.0.0"}"#,tar::EntryType::Regular,None);
    append(&mut tar,"index.json",&serde_json::to_vec(&index).unwrap(),tar::EntryType::Regular,None);
    for (digest, bytes) in blobs { append(&mut tar,&format!("blobs/{}",digest.replace(':',"/")),&bytes,tar::EntryType::Regular,None); }
    tar.finish().unwrap();
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&tar.into_inner().unwrap()).unwrap(); file.rewind().unwrap(); file
}
fn data_layer() -> Vec<u8> { layer(|tar| append(tar,"value",b"retained runtime data",tar::EntryType::Regular,None)) }

#[test]
fn descriptor_hashes_are_not_a_substitute_for_runtime_and_diffid_admission() {
    let mut valid = image(vec![data_layer()], |_,_,_|{});
    let admitted = validate(&mut valid,platform(),&expected(),OciLimits::default()).unwrap();
    assert_eq!(admitted.files()["/value"], FileEntry { mode:0o644,uid:1000,gid:1000,kind:FileKind::File{size:21,digest:BlobDigest::parse(&blob_hash(b"retained runtime data")).unwrap()} });
    let mutations: [fn(&mut Value,&mut Value,&mut BTreeMap<String,Vec<u8>>);4] = [
        |config,_,_| config["architecture"] = json!("arm64"),
        |config,_,_| config["config"]["Env"] = json!(["LD_PRELOAD=/foreign"]),
        |config,_,_| config["rootfs"]["diff_ids"][0] = json!(format!("sha256:{}","0".repeat(64))),
        |_,manifest,blobs| { let digest = manifest["layers"][0]["digest"].as_str().unwrap(); blobs.get_mut(digest).unwrap()[20] ^= 1; },
    ];
    for mutate in mutations {
        let mut invalid = image(vec![data_layer()],mutate);
        assert!(validate(&mut invalid,platform(),&expected(),OciLimits::default()).is_err());
    }
}

#[test]
fn whiteouts_precede_same_layer_additions_and_hardlink_cycles_cannot_use_lower_files() {
    let lower = layer(|tar| { append(tar,"dir/old",b"old",tar::EntryType::Regular,None); append(tar,"a",b"lower",tar::EntryType::Regular,None); });
    let upper = layer(|tar| { append(tar,"dir/new",b"new",tar::EntryType::Regular,None); append(tar,"dir/.wh..wh..opq",b"",tar::EntryType::Regular,None); append(tar,"b",b"",tar::EntryType::Link,Some("a")); });
    let mut archive = image(vec![lower.clone(),upper],|_,_,_|{});
    let checked = validate(&mut archive,platform(),&expected(),OciLimits::default()).unwrap();
    assert!(!checked.files().contains_key("/dir/old"));
    assert_eq!(checked.files()["/dir/new"].kind,FileKind::File{size:3,digest:BlobDigest::parse(&blob_hash(b"new")).unwrap()});
    assert_eq!(checked.files()["/a"],checked.files()["/b"]);
    let cycle = layer(|tar| { append(tar,"a",b"",tar::EntryType::Link,Some("b")); append(tar,"b",b"",tar::EntryType::Link,Some("a")); });
    let mut archive = image(vec![lower,cycle],|_,_,_|{});
    assert!(validate(&mut archive,platform(),&expected(),OciLimits::default()).is_err());
}

#[test]
fn traversal_symlink_parents_and_expansion_budgets_refuse_publication() {
    let escaping = layer(|tar| {
        let mut header = tar::Header::new_ustar();
        header.as_mut_bytes()[..9].copy_from_slice(b"../escape");
        header.set_size(0); header.set_entry_type(tar::EntryType::Regular); header.set_mode(0o644); header.set_uid(0); header.set_gid(0); header.set_mtime(0); header.set_cksum();
        tar.append(&header,Cursor::new([])).unwrap();
    });
    let alias = layer(|tar| { append(tar,"alias",b"",tar::EntryType::Symlink,Some("/elsewhere")); append(tar,"alias/value",b"hidden",tar::EntryType::Regular,None); });
    for layer in [escaping,alias] {
        let mut archive = image(vec![layer],|_,_,_|{});
        assert!(validate(&mut archive,platform(),&expected(),OciLimits::default()).is_err());
    }
    let mut archive = image(vec![data_layer()],|_,_,_|{});
    let limits = OciLimits { expanded_bytes:512,..OciLimits::default() };
    assert!(validate(&mut archive,platform(),&expected(),limits).is_err());
}

#[test]
fn interpreter_templates_are_data_until_selected_for_execution() {
    let payload = layer(|tar| append(tar,"template",b"#!\n",tar::EntryType::Regular,None));
    let mut archive = image(vec![payload.clone()],|_,_,_|{});
    let checked = validate(&mut archive,platform(),&expected(),OciLimits::default()).unwrap();
    assert_eq!(checked.files()["/template"].kind,FileKind::File {
        size:3,digest:BlobDigest::parse(&blob_hash(b"#!\n")).unwrap(),
    });
    let mut selected = image(vec![payload],|config,_,_| config["config"]["Entrypoint"] = json!(["/template"]));
    let config = ImageConfig { entrypoint:vec!["/template".into()],..expected() };
    assert!(validate(&mut selected,platform(),&config,OciLimits::default()).is_err());
}
