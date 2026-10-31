use super::super::{
    format::{
        BlobDescriptor, Component, EntryKind, PortableCommand, PortableEntry, RuntimeContract,
        TREE_MEDIA_TYPE,
    },
    trust,
};
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::SigningKey;
use gripsack_store::{
    source_bundle::{SourceBundle, SourceCapturePolicy},
    trust::evaluation::EvaluationId,
};
use std::{fs, io::Write};

fn admit_environment(
    home: &Path,
    manifest: &str,
    limits: FetchLimits,
) -> Result<Option<Arc<super::super::environment::AdmittedEnvironment>>, ExecError> {
    super::super::ReadmissionScope::new(home, limits).admit_environment(manifest)
}

fn readmit_fixed<'a>(
    home: &Path,
    payload: &Path,
    prefix: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), ExecError> {
    super::super::ReadmissionScope::new(home, FetchLimits::default())
        .readmit_fixed(payload, prefix, paths)
}

struct Fixture {
    _temporary: tempfile::TempDir,
    ctx: Ctx,
    bundle: PathBuf,
    producer: PathBuf,
    manifest: PortableManifest,
    policy: serde_json::Value,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("recipient with spaces");
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        let producer = temporary.path().join("producer");
        let bundle = producer.join("bundle");
        for path in [
            &home,
            &repo,
            &frontend,
            &producer,
            &bundle,
            &bundle.join("blobs"),
        ] {
            fs::create_dir(path).unwrap();
        }
        fs::write(repo.join("gripsack.ts"), "// captured workspace\n").unwrap();
        fs::write(frontend.join("driver.ts"), "// captured frontend\n").unwrap();
        let sources = SourceBundle::capture(
            &repo,
            &frontend,
            None,
            &home,
            SourceCapturePolicy::default(),
        )
        .unwrap();
        let ctx = Ctx {
            home: home.clone(),
            repository: crate::Repository::evaluated(
                Arc::new(sources),
                EvaluationId::parse(&"a".repeat(64)).unwrap(),
            ),
            only: vec![],
            host: gripsack_ir::HostName::parse("portable-test").unwrap(),
            on_progress: None,
            take_over: false,
            take_over_entries: None,
            home_dir: Default::default(),
            fetch: Default::default(),
        };
        let component = producer.join("component");
        fs::create_dir_all(component.join("bin")).unwrap();
        let tool = b"#!/bin/sh\nexit 0\n";
        let license = b"fixture license\n";
        fs::write(component.join("bin/tool"), tool).unwrap();
        fs::write(component.join("LICENSE"), license).unwrap();
        for (path, mode) in [
            (component.join("bin"), 0o755),
            (component.join("bin/tool"), 0o755),
            (component.join("LICENSE"), 0o644),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let mut archive = tar::Builder::new(Vec::new());
        for (path, mode, bytes, kind) in [
            (
                "LICENSE",
                0o644,
                license.as_slice(),
                tar::EntryType::Regular,
            ),
            ("bin", 0o755, &b""[..], tar::EntryType::Directory),
            ("bin/tool", 0o755, tool.as_slice(), tar::EntryType::Regular),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(mode);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            archive.append_data(&mut header, path, bytes).unwrap();
        }
        let archive = archive.into_inner().unwrap();
        let descriptor = BlobDescriptor {
            sha256: gripsack_store::hash::hex_sha256(&archive),
            size: archive.len() as u64,
            media_type: TREE_MEDIA_TYPE.into(),
        };
        fs::write(bundle.join("blobs").join(&descriptor.sha256), archive).unwrap();
        let manifest = PortableManifest {
            version: 1,
            namespace: "tools".into(),
            publisher: "fixture".into(),
            target: super::super::target::PortableTarget {
                os: "linux".into(),
                arch: "x86_64".into(),
                abi: super::super::target::RuntimeAbi::Linux {},
                minimum_kernel: "4.18".into(),
                cpu: "x86_64".into(),
            },
            layout: RuntimeLayout::Relocatable {
                contract: RuntimeContract {
                    behavior: "passive fixture; no import execution".into(),
                },
            },
            components: vec![Component {
                id: "tool".into(),
                payload: Some(descriptor),
                tree: Some(
                    gripsack_store::canonical_tree_hash(&component)
                        .unwrap()
                        .to_string(),
                ),
                runtime: vec![],
                conda: None,
                external: None,
                inventory: vec![
                    PortableEntry {
                        path: "LICENSE".into(),
                        mode: 0o644,
                        kind: EntryKind::File {
                            sha256: gripsack_store::hash::hex_sha256(license),
                            size: license.len() as u64,
                        },
                    },
                    PortableEntry {
                        path: "bin".into(),
                        mode: 0o755,
                        kind: EntryKind::Directory,
                    },
                    PortableEntry {
                        path: "bin/tool".into(),
                        mode: 0o755,
                        kind: EntryKind::File {
                            sha256: gripsack_store::hash::hex_sha256(tool),
                            size: tool.len() as u64,
                        },
                    },
                ],
            }],
            roots: vec!["tool".into()],
            commands: BTreeMap::from([(
                "tool".into(),
                PortableCommand {
                    component: "tool".into(),
                    executable: "bin/tool".into(),
                    args: vec![],
                },
            )]),
            environment: BTreeMap::new(),
            licenses: vec!["components/tool/LICENSE".into()],
            redistribution_authorized: true,
        };
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let policy = serde_json::json!({"version":1,"keys":[{"key_id":"test-key","publisher":"fixture","namespaces":["tools"],
            "public_key":STANDARD.encode(key.verifying_key().as_bytes()),"allowed_layouts":["relocatable"],
            "required_validators":["manifest-integrity","payload-integrity","layout-safety"]}],"revoked_keys":[],"revoked_manifests":[]});
        fs::create_dir(home.join("sharing")).unwrap();
        fs::write(
            home.join("sharing/trust.json"),
            serde_json::to_vec(&policy).unwrap(),
        )
        .unwrap();
        let fixture = Self {
            _temporary: temporary,
            ctx,
            bundle,
            producer,
            manifest,
            policy,
        };
        fixture.sign();
        fixture
    }
    fn sign(&self) {
        let key = self.producer.join("key");
        fs::write(&key, [7u8; 32]).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let bytes = self.manifest.encode().unwrap();
        fs::write(
            self.bundle.join("envelope.json"),
            trust::sign(&bytes, &key, "test-key").unwrap(),
        )
        .unwrap();
        fs::write(self.bundle.join("manifest.json"), bytes).unwrap();
    }
    fn import(&self) -> Result<PackageFacts, ExecError> {
        let session = LifecycleSession::acquire(&self.ctx.home)?;
        import_bundle(&self.ctx, &session, &self.bundle)
    }
    fn policy(&self, policy: &serde_json::Value) {
        fs::write(
            self.ctx.home.join("sharing/trust.json"),
            serde_json::to_vec(policy).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn native_pack_publishes_transfer_bytes_on_a_different_filesystem() {
    use std::os::unix::fs::MetadataExt;

    let fixture = Fixture::new();
    let capture_device = fs::metadata(std::env::temp_dir()).unwrap().dev();
    let shared_memory_device = fs::metadata("/dev/shm").unwrap().dev();
    let destination_parent = if capture_device == shared_memory_device {
        "/tmp"
    } else {
        "/dev/shm"
    };
    let destination = tempfile::tempdir_in(destination_parent).unwrap();
    assert_ne!(
        fs::metadata(destination.path()).unwrap().dev(),
        capture_device,
        "the Linux container gate must provide distinct temporary filesystems"
    );
    fs::create_dir(fixture.producer.join("components")).unwrap();
    fs::rename(
        fixture.producer.join("component"),
        fixture.producer.join("components/tool"),
    )
    .unwrap();
    let output = destination.path().join("bundle");
    let session = LifecycleSession::acquire(&fixture.ctx.home).unwrap();
    let identity = super::super::pack(
        &fixture.ctx,
        &session,
        &fixture.manifest.encode().unwrap(),
        &fixture.producer,
        &output,
        &fixture.producer.join("key"),
        "test-key",
    )
    .unwrap();
    let imported = import_bundle(&fixture.ctx, &session, &output).unwrap();
    assert_eq!(imported.manifest, identity);
    assert_eq!(
        fs::read(imported.payload.join("components/tool/bin/tool")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
}

#[test]
fn imported_closure_outlives_producer_and_exports_exact_originals() {
    let fixture = Fixture::new();
    let original = fs::read(fixture.bundle.join("manifest.json")).unwrap();
    let facts = fixture.import().unwrap();
    fs::remove_dir_all(&fixture.producer).unwrap();
    let source = admit_source(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap()
        .prepared;
    assert!(
        source
            .artifact
            .retention
            .contains(&imported_root(&fixture.ctx.home, &facts.manifest).unwrap())
    );
    assert!(source.artifact.retention.contains(&facts.payload));
    readmit_roots(
        &fixture.ctx.home,
        source.artifact.retention.iter().map(PathBuf::as_path),
    )
    .unwrap();
    assert_eq!(
        fs::read(facts.payload.join("components/tool/bin/tool")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    let output = fixture._temporary.path().join("export");
    export_bundle(&fixture.ctx.home, &facts.manifest, &output).unwrap();
    assert_eq!(fs::read(output.join("manifest.json")).unwrap(), original);
    assert_eq!(
        fs::read_dir(fixture.ctx.home.join("roots/outputs"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn pack_import_preserves_private_directory_modes_without_relaxing_inventory() {
    let fixture = Fixture::new();
    let input = fixture.producer.join("input");
    fs::create_dir_all(input.join("components")).unwrap();
    let component = input.join("components/tool");
    fs::rename(fixture.producer.join("component"), &component).unwrap();
    // The result of umask 077 without changing process-global test state.
    fs::set_permissions(component.join("bin"), fs::Permissions::from_mode(0o700)).unwrap();
    let bundle = fixture.producer.join("private-bundle");
    let session = LifecycleSession::acquire(&fixture.ctx.home).unwrap();
    let manifest = super::super::pack(
        &fixture.ctx,
        &session,
        &serde_json::to_vec(&fixture.manifest).unwrap(),
        &input,
        &bundle,
        &fixture.producer.join("key"),
        "test-key",
    )
    .unwrap();
    let facts = import_bundle(&fixture.ctx, &session, &bundle).unwrap();
    assert_eq!(facts.manifest, manifest);
    let directory = facts.payload.join("components/tool/bin");
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(inspect(&fixture.ctx.home, &manifest).unwrap().is_some());
    fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(inspect(&fixture.ctx.home, &manifest).is_err());
}

#[test]
fn warm_receipts_recheck_current_key_digest_layout_and_validator_policy() {
    let fixture = Fixture::new();
    let facts = fixture.import().unwrap();
    for changed in [
        {
            let mut p = fixture.policy.clone();
            p["revoked_keys"] = serde_json::json!(["test-key"]);
            p
        },
        {
            let mut p = fixture.policy.clone();
            p["revoked_manifests"] = serde_json::json!([facts.manifest]);
            p
        },
        {
            let mut p = fixture.policy.clone();
            p["keys"][0]["allowed_layouts"] = serde_json::json!(["fixed"]);
            p
        },
        {
            let mut p = fixture.policy.clone();
            p["keys"][0]["required_validators"] = serde_json::json!(["publisher-executable"]);
            p
        },
    ] {
        fixture.policy(&changed);
        assert!(inspect(&fixture.ctx.home, &facts.manifest).is_err());
        let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
        assert!(readmit_roots(&fixture.ctx.home, [root.as_path()]).is_err());
    }
    fixture.policy(&fixture.policy);
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
}

#[test]
fn missing_and_tampered_blobs_never_publish_import_receipts() {
    for missing in [true, false] {
        let fixture = Fixture::new();
        let blob = fixture.bundle.join("blobs").join(
            &fixture.manifest.components[0]
                .payload
                .as_ref()
                .unwrap()
                .sha256,
        );
        if missing {
            fs::remove_file(blob).unwrap();
        } else {
            fs::OpenOptions::new()
                .append(true)
                .open(blob)
                .unwrap()
                .write_all(b"tamper")
                .unwrap();
        }
        assert!(fixture.import().is_err());
        assert!(
            !imported_root(&fixture.ctx.home, &fixture.manifest.digest().unwrap())
                .unwrap()
                .exists()
        );
    }
}

#[test]
fn retained_payload_tampering_and_missing_receipts_fail_closed() {
    let fixture = Fixture::new();
    let facts = fixture.import().unwrap();
    let tool = facts.payload.join("components/tool/bin/tool");
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&tool, b"#!/bin/sh\nexit 9\n").unwrap();
    assert!(inspect(&fixture.ctx.home, &facts.manifest).is_err());
    let absent = gripsack_store::content_path(&fixture.ctx.home, "package-import", &"f".repeat(64));
    assert!(readmit_roots(&fixture.ctx.home, [absent.as_path()]).is_err());
}

#[test]
fn signed_inventory_command_and_expansion_mismatches_are_rejected() {
    let mut fixture = Fixture::new();
    fixture.manifest.components[0].inventory.pop();
    fixture.sign();
    assert!(fixture.import().is_err());
    let mut fixture = Fixture::new();
    fixture
        .manifest
        .commands
        .get_mut("tool")
        .unwrap()
        .executable = "missing".into();
    fixture.sign();
    assert!(fixture.import().is_err());
    let mut fixture = Fixture::new();
    let limits = FetchLimits {
        expanded_bytes: std::num::NonZeroU64::new(1).unwrap(),
        ..FetchLimits::default()
    };
    fixture.ctx.fetch = Arc::new(gripsack_fetch::FetchContext::new(limits));
    assert!(fixture.import().is_err());
}

#[test]
fn exact_signed_bytes_and_requested_identity_cannot_be_swapped() {
    let fixture = Fixture::new();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(fixture.bundle.join("manifest.json"))
        .unwrap();
    file.write_all(b"\n").unwrap();
    assert!(fixture.import().is_err());
    assert!(admit_source(&fixture.ctx.home, "../../outside", FetchLimits::default()).is_err());
}

#[test]
fn warm_import_still_rejects_tampered_incoming_transport() {
    let fixture = Fixture::new();
    let facts = fixture.import().unwrap();
    let blob = fixture.bundle.join("blobs").join(&facts.payload_digests[0]);
    fs::OpenOptions::new()
        .append(true)
        .open(blob)
        .unwrap()
        .write_all(b"changed transport")
        .unwrap();
    assert!(fixture.import().is_err());
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
}

#[test]
fn fixed_placement_requires_signed_exact_recipient_prefix() {
    let mut fixture = Fixture::new();
    let aggregate = fixture.producer.join("aggregate");
    let destination = aggregate.join("components/tool");
    fixture
        .ctx
        .fetch
        .fetch(
            &FetchSpec::File {
                path: fixture.producer.join("component").to_str().unwrap().into(),
            },
            &destination,
            None,
        )
        .unwrap();
    let tree = gripsack_store::canonical_tree_hash(&aggregate).unwrap();
    let prefix = gripsack_store::content_path(&fixture.ctx.home, "package-payload", tree.as_str());
    fixture.manifest.layout = RuntimeLayout::Fixed {
        prefix: prefix.to_str().unwrap().into(),
        contract: RuntimeContract {
            behavior: "exact prefix only".into(),
        },
    };
    fixture.policy["keys"][0]["allowed_layouts"] = serde_json::json!(["fixed"]);
    fixture.policy(&fixture.policy);
    fixture.sign();
    let facts = fixture.import().unwrap();
    let source = admit_source(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap()
        .prepared;
    readmit_fixed(
        &fixture.ctx.home,
        &facts.payload,
        &prefix,
        source.artifact.retention.iter().map(PathBuf::as_path),
    )
    .unwrap();
    assert!(
        readmit_fixed(
            &fixture.ctx.home,
            &facts.payload,
            &prefix,
            std::iter::empty()
        )
        .is_err()
    );
    assert!(
        readmit_fixed(
            &fixture.ctx.home,
            &facts.payload,
            Path::new("/different-prefix"),
            source.artifact.retention.iter().map(PathBuf::as_path)
        )
        .is_err()
    );
    let RuntimeLayout::Fixed { prefix, .. } = &mut fixture.manifest.layout else {
        unreachable!()
    };
    *prefix = "/different-prefix".into();
    fixture.sign();
    assert!(fixture.import().is_err());
}

#[test]
fn bounded_records_and_symlinked_blob_inputs_are_rejected() {
    let fixture = Fixture::new();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.bundle.join("manifest.json"))
        .unwrap()
        .set_len(format::MAX_MANIFEST_BYTES as u64 + 1)
        .unwrap();
    assert!(fixture.import().is_err());
    let fixture = Fixture::new();
    let blob = fixture.bundle.join("blobs").join(
        &fixture.manifest.components[0]
            .payload
            .as_ref()
            .unwrap()
            .sha256,
    );
    let outside = fixture.producer.join("outside");
    fs::rename(&blob, &outside).unwrap();
    std::os::unix::fs::symlink(outside, blob).unwrap();
    assert!(fixture.import().is_err());
}

#[test]
fn empty_tree_uses_the_signed_tar_media_contract() {
    let mut fixture = Fixture::new();
    let empty = fixture.producer.join("empty");
    fs::create_dir(&empty).unwrap();
    let archive = tar::Builder::new(Vec::new()).into_inner().unwrap();
    let descriptor = BlobDescriptor {
        sha256: gripsack_store::hash::hex_sha256(&archive),
        size: archive.len() as u64,
        media_type: TREE_MEDIA_TYPE.into(),
    };
    fs::write(
        fixture.bundle.join("blobs").join(&descriptor.sha256),
        archive,
    )
    .unwrap();
    let component = &mut fixture.manifest.components[0];
    component.payload = Some(descriptor);
    component.tree = Some(
        gripsack_store::canonical_tree_hash(&empty)
            .unwrap()
            .to_string(),
    );
    component.inventory.clear();
    fixture.manifest.commands.clear();
    fixture.manifest.licenses.clear();
    fixture.sign();
    fixture.import().unwrap();
}

impl Fixture {
    fn external(&mut self) -> (String, PortableManifest) {
        let mut child = self.manifest.clone();
        child.namespace = "upstream".into();
        child.publisher = "upstream-builder".into();
        // Permitted local runtime cycles do not become digest cycles.
        child.components[0].runtime = vec!["tool".into()];
        let key = SigningKey::from_bytes(&[8u8; 32]);
        self.policy["keys"].as_array_mut().unwrap().push(serde_json::json!({
            "key_id":"upstream-key", "publisher":"upstream-builder",
            "namespaces":["upstream"], "public_key":STANDARD.encode(key.verifying_key().as_bytes()),
            "allowed_layouts":["relocatable","fixed","conda"],
            "required_validators":["manifest-integrity","payload-integrity","layout-safety"]
        }));
        self.policy(&self.policy);
        let identity = self.write_external(&child, 8, "upstream-key");
        self.manifest.components[0]
            .runtime
            .push("dependency".into());
        self.manifest.components.push(Component {
            id: "dependency".into(),
            payload: None,
            tree: None,
            conda: None,
            external: Some(format::ExternalReference {
                manifest: identity.clone(),
                namespace: child.namespace.clone(),
                publisher: child.publisher.clone(),
            }),
            inventory: vec![],
            runtime: vec![],
        });
        self.manifest.commands.insert(
            "upstream".into(),
            PortableCommand {
                component: "dependency".into(),
                executable: "components/tool/bin/tool".into(),
                args: vec![],
            },
        );
        self.sign();
        (identity, child)
    }

    fn write_external(&self, manifest: &PortableManifest, seed: u8, key_id: &str) -> String {
        let identity = manifest.digest().unwrap();
        let directory = self.bundle.join("manifests").join(&identity);
        fs::create_dir_all(&directory).unwrap();
        let key = self.producer.join(format!("key-{seed}"));
        fs::write(&key, [seed; 32]).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let bytes = manifest.encode().unwrap();
        fs::write(
            directory.join("envelope.json"),
            trust::sign(&bytes, &key, key_id).unwrap(),
        )
        .unwrap();
        fs::write(directory.join("manifest.json"), bytes).unwrap();
        identity
    }
}

#[test]
fn independent_closure_round_trip_retains_originals_without_exposing_upstream() {
    let mut fixture = Fixture::new();
    let (child, _) = fixture.external();
    let facts = fixture.import().unwrap();
    assert!(inspect(&fixture.ctx.home, &child).unwrap().is_none());
    assert_eq!(
        fs::read_dir(fixture.ctx.home.join("roots/outputs"))
            .unwrap()
            .count(),
        1
    );
    fs::remove_dir_all(&fixture.producer).unwrap();
    let source = admit_source(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap()
        .prepared;
    assert_eq!(source.artifact.retention.len(), 2);
    assert_eq!(
        fs::read(
            facts
                .payload
                .join("components/dependency/components/tool/bin/tool")
        )
        .unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    let output = fixture._temporary.path().join("complete-export");
    export_bundle(&fixture.ctx.home, &facts.manifest, &output).unwrap();
    let bundle =
        super::super::closure::ClosureBundle::read(&output, FetchLimits::default()).unwrap();
    bundle.validate().unwrap();
    assert!(bundle.external_manifests.contains_key(&child));
    assert_eq!(bundle.blobs.len(), 1);
    let second = Fixture::new();
    second.policy(&fixture.policy);
    let session = LifecycleSession::acquire(&second.ctx.home).unwrap();
    let imported = import_bundle(&second.ctx, &session, &output).unwrap();
    assert_eq!(imported.manifest, facts.manifest);
    assert_eq!(imported.realized_tree, facts.realized_tree);
}

#[test]
fn every_managed_consume_readmits_independent_publisher_and_digest() {
    let mut fixture = Fixture::new();
    let (child, _) = fixture.external();
    let facts = fixture.import().unwrap();
    let mut revoked = fixture.policy.clone();
    revoked["revoked_manifests"] = serde_json::json!([child]);
    fixture.policy(&revoked);
    assert!(admit_source(&fixture.ctx.home, &facts.manifest, FetchLimits::default()).is_err());
    assert!(
        export_bundle(
            &fixture.ctx.home,
            &facts.manifest,
            &fixture._temporary.path().join("denied-export")
        )
        .is_err()
    );
    fixture.policy(&fixture.policy);
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
    let mut removed = fixture.policy.clone();
    removed["keys"].as_array_mut().unwrap().remove(1);
    fixture.policy(&removed);
    assert!(inspect(&fixture.ctx.home, &facts.manifest).is_err());
    assert!(facts.payload.exists());
}

#[test]
fn root_authority_cannot_impersonate_an_independent_upstream() {
    let mut fixture = Fixture::new();
    let (child, manifest) = fixture.external();
    fixture.write_external(&manifest, 7, "test-key");
    assert!(fixture.import().is_err());
    // A valid upstream signature also cannot satisfy a different reference scope.
    fixture.write_external(&manifest, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .namespace = "tools".into();
    fixture.sign();
    assert!(
        fixture
            .import()
            .unwrap_err()
            .to_string()
            .contains("namespace/publisher")
    );
    assert!(inspect(&fixture.ctx.home, &child).unwrap().is_none());
}

#[test]
fn missing_tampered_and_warm_incoming_external_objects_reject() {
    let mut fixture = Fixture::new();
    let (child, manifest) = fixture.external();
    let directory = fixture.bundle.join("manifests").join(&child);
    fs::remove_file(directory.join("manifest.json")).unwrap();
    assert!(fixture.import().is_err());
    fixture.write_external(&manifest, 8, "upstream-key");
    let facts = fixture.import().unwrap();
    fs::write(directory.join("envelope.json"), b"tampered").unwrap();
    assert!(fixture.import().is_err());
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
    let retained = imported_root(&fixture.ctx.home, &facts.manifest)
        .unwrap()
        .join("manifests")
        .join(child)
        .join("envelope.json");
    fs::set_permissions(&retained, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(retained, b"tampered").unwrap();
    assert!(inspect(&fixture.ctx.home, &facts.manifest).is_err());
}

#[test]
fn fixed_external_root_is_not_prefix_copied_into_an_aggregate() {
    let mut fixture = Fixture::new();
    let (_, mut child) = fixture.external();
    child.layout = RuntimeLayout::Fixed {
        prefix: "/producer/fixed-prefix".into(),
        contract: RuntimeContract {
            behavior: "exact prefix only".into(),
        },
    };
    let identity = fixture.write_external(&child, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = identity;
    fixture.sign();
    assert!(
        fixture
            .import()
            .unwrap_err()
            .to_string()
            .contains("embedded aggregate members require relocatable")
    );
}

#[test]
fn external_cycle_kernel_distinguishes_local_cycles_and_bounds_depth() {
    let fixture = Fixture::new();
    let mut local = fixture.manifest.clone();
    local.components[0].runtime = vec!["tool".into()];
    let mut graph = Closure {
        root: "root".into(),
        objects: BTreeMap::new(),
        manifests: BTreeMap::from([("root".into(), local.clone())]),
        blobs: BTreeMap::new(),
    };
    graph.check_graph(FetchLimits::default()).unwrap();
    // The pure graph kernel is exercised with synthetic identities: producing
    // a cryptographic digest cycle is not a feasible signed-wire fixture.
    let external = |identity: &str| Component {
        id: "dependency".into(),
        payload: None,
        tree: None,
        conda: None,
        external: Some(format::ExternalReference {
            manifest: identity.into(),
            namespace: "tools".into(),
            publisher: "fixture".into(),
        }),
        inventory: vec![],
        runtime: vec![],
    };
    graph
        .manifests
        .get_mut("root")
        .unwrap()
        .components
        .push(external("root"));
    assert!(
        graph
            .check_graph(FetchLimits::default())
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    graph.manifests.get_mut("root").unwrap().components.pop();
    let mut previous = "root".to_owned();
    for index in 0..=super::super::closure::MAX_CLOSURE_DEPTH {
        let next = format!("child-{index}");
        graph
            .manifests
            .get_mut(&previous)
            .unwrap()
            .components
            .push(external(&next));
        graph.manifests.insert(next.clone(), local.clone());
        previous = next;
    }
    assert!(
        graph
            .check_graph(FetchLimits::default())
            .unwrap_err()
            .to_string()
            .contains("depth")
    );
}

#[test]
fn transitive_diamond_closure_materializes_each_path_and_requires_every_object() {
    let mut fixture = Fixture::new();
    let (leaf, mut middle) = fixture.external();
    let reference = fixture.manifest.components[1].clone();
    middle.components[0].runtime.push(reference.id.clone());
    middle.components.push(reference.clone());
    let middle_id = fixture.write_external(&middle, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = middle_id.clone();
    let mut direct = reference;
    direct.id = "direct-leaf".into();
    fixture.manifest.components[0]
        .runtime
        .push(direct.id.clone());
    fixture.manifest.components.push(direct);
    fixture.sign();
    let leaf_dir = fixture.bundle.join("manifests").join(&leaf);
    let manifest_bytes = fs::read(leaf_dir.join("manifest.json")).unwrap();
    fs::remove_file(leaf_dir.join("manifest.json")).unwrap();
    assert!(fixture.import().is_err());
    fs::write(leaf_dir.join("manifest.json"), manifest_bytes).unwrap();
    let facts = fixture.import().unwrap();
    for path in [
        "components/dependency/components/dependency/components/tool/bin/tool",
        "components/direct-leaf/components/tool/bin/tool",
    ] {
        assert_eq!(
            fs::read(facts.payload.join(path)).unwrap(),
            b"#!/bin/sh\nexit 0\n"
        );
    }
    let output = fixture._temporary.path().join("diamond-export");
    export_bundle(&fixture.ctx.home, &facts.manifest, &output).unwrap();
    let bundle =
        super::super::closure::ClosureBundle::read(&output, FetchLimits::default()).unwrap();
    assert_eq!(bundle.external_manifests.len(), 2);
    assert!(bundle.external_manifests.contains_key(&middle_id));
    assert!(bundle.external_manifests.contains_key(&leaf));
    assert_eq!(bundle.blobs.len(), 1);
}

#[test]
fn independent_target_requirement_is_not_waived_by_compatible_root() {
    let mut fixture = Fixture::new();
    let (_, mut child) = fixture.external();
    child.target.minimum_kernel = "9999.0".into();
    let identity = fixture.write_external(&child, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = identity;
    fixture.sign();
    assert!(fixture.import().is_err());
}

#[test]
fn independent_blob_tamper_and_tightened_layout_policy_reject() {
    let mut fixture = Fixture::new();
    let (identity, _) = fixture.external();
    let facts = fixture.import().unwrap();
    let mut policy = fixture.policy.clone();
    policy["keys"][1]["allowed_layouts"] = serde_json::json!(["fixed"]);
    fixture.policy(&policy);
    assert!(admit_source(&fixture.ctx.home, &facts.manifest, FetchLimits::default()).is_err());
    fixture.policy(&fixture.policy);
    let mut bundle =
        super::super::closure::ClosureBundle::read(&fixture.bundle, FetchLimits::default())
            .unwrap();
    bundle.blobs.values_mut().next().unwrap()[0] ^= 1;
    assert!(bundle.validate().is_err());
    bundle.external_manifests.remove(&identity);
    assert!(bundle.validate().is_err());
}

#[test]
fn closure_budgets_apply_to_metadata_members_entries_and_expansion() {
    let fixture = Fixture::new();
    let directory = gripsack_fs::open(&fixture.bundle).unwrap();
    let mut limits = FetchLimits {
        download_bytes: std::num::NonZeroU64::new(1).unwrap(),
        ..Default::default()
    };
    assert!(Closure::read(&directory, limits).is_err());
    limits = FetchLimits {
        expanded_bytes: std::num::NonZeroU64::new(1).unwrap(),
        ..Default::default()
    };
    assert!(Closure::read(&directory, limits).is_err());
    limits = FetchLimits {
        archive_entries: std::num::NonZeroUsize::new(1).unwrap(),
        ..Default::default()
    };
    assert!(Closure::read(&directory, limits).is_err());

    let mut root = fixture.manifest.clone();
    let mut objects = BTreeMap::new();
    for index in 0..super::super::closure::MAX_CLOSURE_MANIFESTS {
        let mut child = fixture.manifest.clone();
        child
            .commands
            .get_mut("tool")
            .unwrap()
            .args
            .push(index.to_string());
        let identity = child.digest().unwrap();
        objects.insert(
            identity.clone(),
            super::super::closure::ExternalManifest {
                manifest: child.encode().unwrap(),
                envelope: vec![],
            },
        );
        let id = format!("child-{index}");
        root.components[0].runtime.push(id.clone());
        root.components.push(Component {
            id,
            payload: None,
            tree: None,
            conda: None,
            external: Some(format::ExternalReference {
                manifest: identity,
                namespace: child.namespace,
                publisher: child.publisher,
            }),
            inventory: vec![],
            runtime: vec![],
        });
    }
    let bundle = super::super::closure::ClosureBundle {
        manifest: root.encode().unwrap(),
        envelope: vec![],
        external_manifests: objects,
        blobs: BTreeMap::new(),
    };
    assert!(
        bundle
            .validate()
            .unwrap_err()
            .to_string()
            .contains("manifest count")
    );
}

#[test]
fn inspection_reports_current_independent_authority_and_structured_target() {
    let mut fixture = Fixture::new();
    let (child, child_manifest) = fixture.external();
    let facts = fixture.import().unwrap();
    let value = serde_json::to_value(&facts).unwrap();
    assert_eq!(value["target"]["abi"]["kind"], "linux");
    assert_eq!(value["external"].as_array().unwrap().len(), 1);
    assert_eq!(value["external"][0]["manifest"], child);
    assert_eq!(value["external"][0]["publisher"], child_manifest.publisher);
    assert_eq!(value["external"][0]["namespace"], child_manifest.namespace);
    assert_eq!(value["external"][0]["key_id"], "upstream-key");
    assert_eq!(
        value["external"][0]["trust_revision"],
        value["trust_revision"]
    );
    assert_eq!(value["external"][0]["target"], value["target"]);
}

impl Fixture {
    fn environment(&mut self) {
        self.manifest.layout = RuntimeLayout::Environment {};
        for key in self.policy["keys"].as_array_mut().unwrap() {
            key["allowed_layouts"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!("environment"));
        }
        self.policy(&self.policy);
        self.sign();
    }
}

#[test]
fn passive_environment_retains_original_members_without_embedding_prefixes() {
    let mut fixture = Fixture::new();
    let (external, _) = fixture.external();
    fixture.manifest.roots = vec!["dependency".into(), "tool".into()];
    fixture.environment();
    let original = fs::read(
        fixture.bundle.join("blobs").join(
            &fixture.manifest.components[0]
                .payload
                .as_ref()
                .unwrap()
                .sha256,
        ),
    )
    .unwrap();
    let facts = fixture.import().unwrap();
    assert_eq!(facts.layout, "environment");
    assert!(facts.materialization.is_none());
    assert!(!facts.payload.join("components").exists());
    assert!(!facts.payload.join("bin").exists());
    let admitted = admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    assert_eq!(admitted.manifest.roots, vec!["dependency", "tool"]);
    assert_eq!(admitted.components.len(), 2);
    for (id, member) in &admitted.components {
        let super::super::environment::AdmittedMember::Package(package) = member else {
            panic!("expected package member")
        };
        let source = &package.source;
        let executable = if id == "tool" {
            "bin/tool"
        } else {
            "components/tool/bin/tool"
        };
        assert_eq!(
            fs::read(source.artifact.payload.join(executable)).unwrap(),
            b"#!/bin/sh\nexit 0\n"
        );
        assert!(!source.artifact.payload.starts_with(&facts.payload));
        assert!(
            admitted
                .artifact
                .retention
                .contains(&source.artifact.payload)
        );
        assert!(
            source
                .artifact
                .retention
                .contains(&imported_root(&fixture.ctx.home, &facts.manifest).unwrap())
        );
    }
    fs::remove_dir_all(&fixture.producer).unwrap();
    let output = fixture._temporary.path().join("environment-export");
    export_bundle(&fixture.ctx.home, &facts.manifest, &output).unwrap();
    let exported =
        super::super::closure::ClosureBundle::read(&output, FetchLimits::default()).unwrap();
    assert_eq!(exported.blobs.values().next().unwrap(), &original);
    assert!(exported.external_manifests.contains_key(&external));
    let second = Fixture::new();
    second.policy(&fixture.policy);
    let session = LifecycleSession::acquire(&second.ctx.home).unwrap();
    let imported = import_bundle(&second.ctx, &session, &output).unwrap();
    assert_eq!(imported.manifest, facts.manifest);
    assert_ne!(imported.realized_tree, facts.realized_tree);
    assert_ne!(imported.payload, facts.payload);
}

#[test]
fn environment_readmission_rejects_revoked_or_missing_members() {
    let mut fixture = Fixture::new();
    let (external, _) = fixture.external();
    fixture.environment();
    let facts = fixture.import().unwrap();
    let mut revoked = fixture.policy.clone();
    revoked["revoked_manifests"] = serde_json::json!([external]);
    fixture.policy(&revoked);
    assert!(admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default()).is_err());
    fixture.policy(&fixture.policy);
    let admitted = admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let super::super::environment::AdmittedMember::Package(package) = &admitted.components["tool"]
    else {
        panic!("native member missing")
    };
    let source = &package.source;
    fs::set_permissions(
        source.artifact.payload.join("bin"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::remove_file(source.artifact.payload.join("bin/tool")).unwrap();
    assert!(inspect(&fixture.ctx.home, &facts.manifest).is_err());
    assert!(
        export_bundle(
            &fixture.ctx.home,
            &facts.manifest,
            &fixture._temporary.path().join("broken-export")
        )
        .is_err()
    );
}

#[test]
fn nested_environment_catalog_preserves_order_and_independent_authority() {
    let mut fixture = Fixture::new();
    let (_, mut child) = fixture.external();
    child.layout = RuntimeLayout::Environment {};
    let child_identity = fixture.write_external(&child, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = child_identity.clone();
    fixture
        .manifest
        .commands
        .get_mut("upstream")
        .unwrap()
        .executable = "bin/tool".into();
    fixture.manifest.roots = vec!["dependency".into(), "tool".into()];
    fixture.environment();
    let facts = fixture.import().unwrap();
    let admitted = admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let super::super::environment::AdmittedMember::Environment(nested) =
        &admitted.components["dependency"]
    else {
        panic!("nested environment missing")
    };
    assert_eq!(nested.identity, child_identity);
    assert_eq!(nested.manifest.roots, vec!["tool"]);
    assert_eq!(admitted.manifest.roots, vec!["dependency", "tool"]);
    assert_eq!(facts.external.len(), 1);
    assert_eq!(facts.external[0].key_id, "upstream-key");
    assert!(
        inspect(&fixture.ctx.home, &child_identity)
            .unwrap()
            .is_none()
    );
    fixture
        .manifest
        .commands
        .get_mut("upstream")
        .unwrap()
        .executable = "components/tool/bin/tool".into();
    fixture.sign();
    let invalid = fixture.manifest.digest().unwrap();
    assert!(
        fixture
            .import()
            .unwrap_err()
            .to_string()
            .contains("signed bin/<alias>")
    );
    assert!(!imported_root(&fixture.ctx.home, &invalid).unwrap().exists());
}

#[test]
fn repeated_external_packages_share_receipts_only_within_the_admitted_root() {
    use super::super::environment::{AdmittedEnvironment, AdmittedMember, AdmittedPackage};

    fn package<'a>(root: &'a AdmittedEnvironment, id: &str) -> &'a Arc<AdmittedPackage> {
        let AdmittedMember::Package(package) = &root.components[id] else {
            panic!("package missing");
        };
        package
    }

    let mut fixture = Fixture::new();
    fixture.external();
    let mut repeated = fixture.manifest.components[1].clone();
    repeated.id = "repeated".into();
    fixture.manifest.components.push(repeated);
    fixture.manifest.roots = vec!["tool".into(), "dependency".into(), "repeated".into()];
    fixture.environment();
    let first = fixture.import().unwrap();
    let admitted = admit_environment(&fixture.ctx.home, &first.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let original = package(&admitted, "dependency");

    fixture
        .manifest
        .commands
        .get_mut("tool")
        .unwrap()
        .args
        .push("other-root".into());
    fixture.sign();
    let second = fixture.import().unwrap();
    let other = admit_environment(&fixture.ctx.home, &second.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let independent = package(&other, "dependency");
    let first_root = imported_root(&fixture.ctx.home, &first.manifest).unwrap();
    let second_root = imported_root(&fixture.ctx.home, &second.manifest).unwrap();
    assert!(original.source.artifact.retention.contains(&first_root));
    assert!(!original.source.artifact.retention.contains(&second_root));
    assert!(independent.source.artifact.retention.contains(&second_root));
    assert!(!independent.source.artifact.retention.contains(&first_root));
}

#[test]
fn passive_shared_environment_dag_stays_bounded_and_root_authority_local() {
    use super::super::environment::{AdmittedEnvironment, AdmittedMember};

    fn assert_shared(root: &Arc<AdmittedEnvironment>) {
        let mut node = Arc::clone(root);
        let mut identities = BTreeSet::new();
        for _ in 0..25 {
            assert!(identities.insert(node.identity.clone()));
            let AdmittedMember::Environment(left) = &node.components["left"] else {
                panic!("left environment missing");
            };
            let AdmittedMember::Environment(right) = &node.components["right"] else {
                panic!("right environment missing");
            };
            assert_eq!(left.identity, right.identity);
            node = Arc::clone(left);
        }
        assert!(identities.insert(node.identity.clone()));
        assert_eq!(identities.len(), 26);
        assert!(matches!(
            node.components["tool"],
            AdmittedMember::Package(_)
        ));
        assert!(!root.admits_fixed(Path::new("/absent-shared-environment-member")));
    }

    let mut fixture = Fixture::new();
    fixture.environment();
    let mut child = fixture.manifest.clone();
    for _ in 0..25 {
        let identity = fixture.write_external(&child, 7, "test-key");
        let mut parent = fixture.manifest.clone();
        parent.components = ["left", "right"]
            .into_iter()
            .map(|id| Component {
                id: id.into(),
                payload: None,
                tree: None,
                runtime: vec![],
                conda: None,
                external: Some(format::ExternalReference {
                    manifest: identity.clone(),
                    namespace: child.namespace.clone(),
                    publisher: child.publisher.clone(),
                }),
                inventory: vec![],
            })
            .collect();
        parent.roots = vec!["left".into(), "right".into()];
        parent.commands.get_mut("tool").unwrap().component = "left".into();
        parent.licenses.clear();
        child = parent;
    }
    fixture.manifest = child;
    fixture.sign();
    let first = fixture.import().unwrap();
    assert_eq!(first.external.len(), 25);
    let admitted = admit_environment(&fixture.ctx.home, &first.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    assert_shared(&admitted);
    let mut scope = super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
    scope
        .readmit_roots(admitted.artifact.retention.iter().map(PathBuf::as_path))
        .unwrap();
    let scoped = scope.admit_environment(&first.manifest).unwrap().unwrap();
    for _ in 0..26 {
        let repeated = scope.admit_environment(&first.manifest).unwrap().unwrap();
        assert!(Arc::ptr_eq(&scoped, &repeated));
    }
    assert_eq!(scope.production_loads(), 1);
    assert_shared(&scoped);
    assert!(
        inspect(&fixture.ctx.home, &first.manifest)
            .unwrap()
            .is_some()
    );
    assert_eq!(fixture.import().unwrap().manifest, first.manifest);

    // The same descendants admitted under a different signed root must get a
    // fresh catalog carrying that root's retention, never a global cache hit.
    fixture
        .manifest
        .commands
        .get_mut("tool")
        .unwrap()
        .args
        .push("other-root".into());
    fixture.sign();
    let second = fixture.import().unwrap();
    assert_ne!(first.manifest, second.manifest);
    let other = admit_environment(&fixture.ctx.home, &second.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    assert_shared(&other);
    let AdmittedMember::Environment(first_child) = &admitted.components["left"] else {
        panic!("first child missing");
    };
    let AdmittedMember::Environment(second_child) = &other.components["left"] else {
        panic!("second child missing");
    };
    assert_eq!(first_child.identity, second_child.identity);
    let first_root = imported_root(&fixture.ctx.home, &first.manifest).unwrap();
    let second_root = imported_root(&fixture.ctx.home, &second.manifest).unwrap();
    assert!(first_child.artifact.retention.contains(&first_root));
    assert!(!first_child.artifact.retention.contains(&second_root));
    assert!(second_child.artifact.retention.contains(&second_root));
    assert!(!second_child.artifact.retention.contains(&first_root));

    let mut revoked = fixture.policy.clone();
    revoked["revoked_manifests"] = serde_json::json!([first.manifest]);
    fixture.policy(&revoked);
    assert!(admit_environment(&fixture.ctx.home, &first.manifest, FetchLimits::default()).is_err());
    assert!(
        admit_environment(&fixture.ctx.home, &second.manifest, FetchLimits::default())
            .unwrap()
            .is_some()
    );
}

#[test]
fn incomplete_environment_inputs_never_publish_root_authority() {
    let mut fixture = Fixture::new();
    fixture.environment();
    let identity = fixture.manifest.digest().unwrap();
    let blob = fixture.bundle.join("blobs").join(
        &fixture.manifest.components[0]
            .payload
            .as_ref()
            .unwrap()
            .sha256,
    );
    let original = fs::read(&blob).unwrap();
    fs::write(&blob, b"interrupted original").unwrap();
    assert!(fixture.import().is_err());
    assert!(inspect(&fixture.ctx.home, &identity).unwrap().is_none());
    assert!(
        !imported_root(&fixture.ctx.home, &identity)
            .unwrap()
            .exists()
    );
    fs::write(blob, original).unwrap();
    let facts = fixture.import().unwrap();
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
}

#[test]
fn environment_fixed_member_requires_its_exact_real_destination() {
    let mut fixture = Fixture::new();
    let (_, mut child) = fixture.external();
    let embedded = fixture.import().unwrap();
    let tree = gripsack_store::canonical_tree_hash(&embedded.payload.join("components/dependency"))
        .unwrap();
    let expected =
        gripsack_store::content_path(&fixture.ctx.home, "package-payload", tree.as_str());
    child.layout = RuntimeLayout::Fixed {
        prefix: expected.to_str().unwrap().into(),
        contract: RuntimeContract {
            behavior: "exact recipient prefix".into(),
        },
    };
    let identity = fixture.write_external(&child, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = identity;
    fixture.environment();
    let facts = fixture.import().unwrap();
    let admitted = admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let super::super::environment::AdmittedMember::Package(package) =
        &admitted.components["dependency"]
    else {
        panic!("fixed member missing")
    };
    let super::super::environment::AdmittedPackage {
        source,
        fixed_prefix,
        ..
    } = package.as_ref();
    assert_eq!(fixed_prefix.as_ref(), Some(&expected));
    assert_eq!(source.artifact.payload, expected);
    readmit_fixed(
        &fixture.ctx.home,
        &expected,
        &expected,
        source.artifact.retention.iter().map(PathBuf::as_path),
    )
    .unwrap();
    child.layout = RuntimeLayout::Fixed {
        prefix: "/incompatible/producer/prefix".into(),
        contract: RuntimeContract {
            behavior: "no prefix copying".into(),
        },
    };
    let identity = fixture.write_external(&child, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = identity;
    fixture.sign();
    assert!(
        fixture
            .import()
            .unwrap_err()
            .to_string()
            .contains("fixed environment member")
    );
}

#[test]
fn mixed_local_archive_set_and_native_require_complete_originals_before_publication() {
    let mut fixture = Fixture::new();
    // This is inventory admission coverage, not a runnable helper substitute.
    // The missing archive must be rejected before either member is published.
    let digest = gripsack_store::hash::hex_sha256(b"original Conda archive");
    let portable = super::super::conda::PortableConda {
        lock: serde_json::from_value(serde_json::json!({
            "platform":"linux-64", "channels":["https://example.invalid/channel/"],
            "channel_priority":"strict", "system_requirements":{},
            "packages":[{
                "name":"fixture", "version":"1.0", "build":"0", "build_number":0,
                "subdir":"linux-64", "channel":"https://example.invalid/channel/",
                "url":"https://example.invalid/channel/linux-64/fixture-1.0-0.conda",
                "sha256":digest, "size":22
            }],
            "materializer":{"bytecode":"suppress","receipt":"normalized_conda_meta"}
        }))
        .unwrap(),
        archives: vec![BlobDescriptor {
            sha256: digest,
            size: 22,
            media_type: super::super::conda::ARCHIVE_MEDIA_TYPE.into(),
        }],
        helper: BlobDescriptor {
            sha256: gripsack_conda::provision::approved_pin()
                .unwrap()
                .to_string(),
            size: 1,
            media_type: super::super::conda::HELPER_MEDIA_TYPE.into(),
        },
        helper_notices: super::super::conda::PortableConda::approved_helper_notices().clone(),
    };
    fixture.manifest.components.push(Component {
        id: "python".into(),
        payload: None,
        tree: None,
        runtime: vec![],
        conda: Some(portable),
        external: None,
        inventory: vec![],
    });
    fixture.manifest.roots = vec!["python".into(), "tool".into()];
    fixture.environment();
    let identity = fixture.manifest.digest().unwrap();
    assert!(fixture.import().is_err());
    assert!(inspect(&fixture.ctx.home, &identity).unwrap().is_none());
    let native = gripsack_store::content_path(
        &fixture.ctx.home,
        "package-payload",
        fixture.manifest.components[0].tree.as_ref().unwrap(),
    );
    assert!(!native.exists());
}

#[test]
fn repeated_external_dag_is_charged_per_embedded_occurrence_before_effects() {
    let fixture = Fixture::new();
    let mut graph = Closure {
        root: "level-0".into(),
        objects: BTreeMap::new(),
        manifests: BTreeMap::new(),
        blobs: BTreeMap::new(),
    };
    // Only 21 distinct manifests, but over a million leaf occurrences. Graph
    // admission must not mistake content deduplication for bounded extraction.
    for level in 0..20 {
        let mut manifest = fixture.manifest.clone();
        manifest.components = ["left", "right"]
            .into_iter()
            .map(|id| Component {
                id: id.into(),
                payload: None,
                tree: None,
                conda: None,
                external: Some(format::ExternalReference {
                    manifest: format!("level-{}", level + 1),
                    namespace: "tools".into(),
                    publisher: "fixture".into(),
                }),
                inventory: vec![],
                runtime: vec![],
            })
            .collect();
        graph.manifests.insert(format!("level-{level}"), manifest);
    }
    graph
        .manifests
        .insert("level-20".into(), fixture.manifest.clone());
    let error = graph
        .check_graph(FetchLimits::default())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("inventory exceeds acquisition limits"),
        "{error}"
    );
    // A shallow shared diamond must also charge the same large leaf twice,
    // independently of distinct manifest count and archive digest inventory.
    graph.root = "level-19".into();
    let leaf = graph.manifests.get_mut("level-20").unwrap();
    let EntryKind::File { size, .. } = &mut leaf.components[0].inventory[0].kind else {
        panic!("fixture file missing")
    };
    *size = super::super::closure::MAX_CLOSURE_EXPANDED_BYTES / 2 + 1;
    let limits = FetchLimits {
        expanded_bytes: std::num::NonZeroU64::new(u64::MAX).unwrap(),
        ..Default::default()
    };
    assert!(graph.check_graph(limits).is_err());
}

#[test]
fn environment_does_not_reconstruct_embedded_descendants_as_extra_roots() {
    let mut fixture = Fixture::new();
    let (leaf, mut middle) = fixture.external();
    let dependency = fixture.manifest.components[1].clone();
    middle.components[0].runtime.push(dependency.id.clone());
    middle.components.push(dependency);
    let middle_id = fixture.write_external(&middle, 8, "upstream-key");
    fixture.manifest.components[1]
        .external
        .as_mut()
        .unwrap()
        .manifest = middle_id.clone();
    fixture.environment();
    let facts = fixture.import().unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(facts.payload.join("environment.json")).unwrap()).unwrap();
    let members = metadata["members"].as_object().unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.contains_key(&middle_id));
    assert!(!members.contains_key(&leaf));
    assert_eq!(facts.external.len(), 2);
    let admitted = admit_environment(&fixture.ctx.home, &facts.manifest, FetchLimits::default())
        .unwrap()
        .unwrap();
    let super::super::environment::AdmittedMember::Package(package) =
        &admitted.components["dependency"]
    else {
        panic!("aggregate member missing")
    };
    let source = &package.source;
    assert_eq!(
        fs::read(
            source
                .artifact
                .payload
                .join("components/dependency/components/tool/bin/tool")
        )
        .unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
}

#[test]
fn complete_multi_manifest_environment_receipt_exceeds_old_cap_and_round_trips() {
    let mut fixture = Fixture::new();
    fixture.environment();
    let template = fixture.manifest.clone();
    fixture.manifest.components.clear();
    fixture.manifest.roots.clear();
    fixture.manifest.commands.clear();
    fixture.manifest.licenses.clear();
    for index in 0..3 {
        let mut child = template.clone();
        child.commands.clear();
        // Each selected command remains inside its 256-argument/4096-byte
        // bounds; three roughly 2 MiB manifests fit the complete signed
        // metadata budget but produce more than the old 5 MiB receipt cap.
        for command in 0..40 {
            child.commands.insert(
                format!("tool-{index}-{command:02}"),
                PortableCommand {
                    component: "tool".into(),
                    executable: "bin/tool".into(),
                    args: vec!["x".repeat(4096); 12],
                },
            );
        }
        let digest = fixture.write_external(&child, 7, "test-key");
        let member = format!("child-{index}");
        fixture.manifest.components.push(Component {
            id: member.clone(),
            payload: None,
            tree: None,
            runtime: vec![],
            conda: None,
            external: Some(format::ExternalReference {
                manifest: digest,
                namespace: child.namespace.clone(),
                publisher: child.publisher.clone(),
            }),
            inventory: vec![],
        });
        fixture.manifest.roots.push(member.clone());
        fixture.manifest.commands.insert(
            format!("selected-{index}"),
            PortableCommand {
                component: member,
                executable: format!("bin/tool-{index}-00"),
                args: vec![],
            },
        );
    }
    fixture.sign();
    let facts = fixture.import().unwrap();
    let record = imported_root(&fixture.ctx.home, &facts.manifest)
        .unwrap()
        .join(receipt::RECORD_FILE);
    let size = fs::metadata(&record).unwrap().len();
    assert!(
        size > 5 * 1024 * 1024,
        "fixture must cross the former reader cap"
    );
    assert!(size <= receipt::MAX_MATERIALIZATION_RECORD_BYTES as u64);
    assert!(
        inspect(&fixture.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
    assert_eq!(fixture.import().unwrap().manifest, facts.manifest);
    let exported = fixture._temporary.path().join("large-environment-export");
    export_bundle(&fixture.ctx.home, &facts.manifest, &exported).unwrap();
    let recipient = Fixture::new();
    recipient.policy(&fixture.policy);
    let session = LifecycleSession::acquire(&recipient.ctx.home).unwrap();
    let reimported = import_bundle(&recipient.ctx, &session, &exported).unwrap();
    assert_eq!(reimported.manifest, facts.manifest);
    assert!(
        inspect(&recipient.ctx.home, &facts.manifest)
            .unwrap()
            .is_some()
    );
}

#[test]
fn operation_readmission_verifies_each_complete_view_once() {
    let mut fixture = Fixture::new();
    fixture.external();
    let facts = fixture.import().unwrap();
    let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
    let mut scope = super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
    let original_pin = scope.pin_input(&root).unwrap();
    let producer_pin = scope.pin_input(&facts.payload).unwrap();
    for _ in 0..26 {
        scope
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .unwrap();
    }
    assert_eq!(scope.production_loads(), 1);
    assert_eq!(scope.pinned_inputs(), 2);
    assert!(Arc::ptr_eq(&original_pin, &scope.pin_input(&root).unwrap()));
    assert!(Arc::ptr_eq(
        &producer_pin,
        &scope.pin_input(&facts.payload).unwrap()
    ));
    let mut fresh = super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
    fresh
        .readmit_roots([root.as_path(), facts.payload.as_path()])
        .unwrap();
    assert_eq!(fresh.production_loads(), 1);
}

#[test]
fn root_only_readmission_discovers_then_binds_the_complete_view() {
    let fixture = Fixture::new();
    let facts = fixture.import().unwrap();
    let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
    let mut scope = super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
    for _ in 0..26 {
        scope.readmit_roots([root.as_path()]).unwrap();
    }
    assert_eq!(scope.production_loads(), 2);
}

#[test]
fn operation_readmission_refuses_policy_removal_and_independent_revocation() {
    for remove in [false, true] {
        let mut fixture = Fixture::new();
        let (child, _) = fixture.external();
        let facts = fixture.import().unwrap();
        let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
        let mut scope =
            super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
        scope
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .unwrap();
        if remove {
            fs::remove_file(fixture.ctx.home.join("sharing/trust.json")).unwrap();
        } else {
            let mut policy = fixture.policy.clone();
            policy["revoked_manifests"] = serde_json::json!([child]);
            fixture.policy(&policy);
        }
        assert!(scope.readmit_roots([root.as_path()]).is_err());
        assert_eq!(scope.production_loads(), 1);
        // Restoring policy cannot resurrect a failed operation's acceptance.
        fixture.policy(&fixture.policy);
        assert!(scope.readmit_roots([root.as_path()]).is_err());
        assert!(readmit_roots(&fixture.ctx.home, [root.as_path()]).is_ok());
    }
}

#[test]
fn operation_and_fresh_readmission_reject_changed_original_receipt_blob_and_payload() {
    for input in [
        "manifest.json",
        "envelope.json",
        "receipt.json",
        "blob",
        "payload",
    ] {
        let fixture = Fixture::new();
        let facts = fixture.import().unwrap();
        let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
        let mut scope =
            super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
        scope
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .unwrap();
        let path = match input {
            "blob" => root.join("blobs").join(
                &fixture.manifest.components[0]
                    .payload
                    .as_ref()
                    .unwrap()
                    .sha256,
            ),
            "payload" => facts.payload.join("components/tool/bin/tool"),
            name => root.join(name),
        };
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(scope.readmit_roots([root.as_path()]).is_err(), "{input}");
        assert_eq!(scope.production_loads(), 1);
        assert!(
            readmit_roots(&fixture.ctx.home, [root.as_path()]).is_err(),
            "{input}"
        );
    }
}

#[test]
fn operation_readmission_never_reuses_missing_inputs_or_replaced_roots() {
    for replace in [false, true] {
        let fixture = Fixture::new();
        let facts = fixture.import().unwrap();
        let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
        let mut scope =
            super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
        scope
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .unwrap();
        if replace {
            let old = root.with_extension("old");
            fs::rename(&root, &old).unwrap();
            fs::create_dir(&root).unwrap();
        } else {
            fs::remove_file(root.join("receipt.json")).unwrap();
        }
        assert!(scope.readmit_roots([root.as_path()]).is_err());
        assert_eq!(scope.production_loads(), 1);
    }
}

#[test]
fn operation_readmission_is_home_and_budget_bound() {
    let fixture = Fixture::new();
    let facts = fixture.import().unwrap();
    let root = imported_root(&fixture.ctx.home, &facts.manifest).unwrap();
    let mut admitted =
        super::super::ReadmissionScope::new(&fixture.ctx.home, FetchLimits::default());
    admitted
        .readmit_roots([root.as_path(), facts.payload.as_path()])
        .unwrap();
    assert_eq!(admitted.home(), fixture.ctx.home);
    let other = Fixture::new();
    let mut other_home =
        super::super::ReadmissionScope::new(&other.ctx.home, FetchLimits::default());
    assert!(
        other_home
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .is_err()
    );
    let limits = FetchLimits {
        download_bytes: std::num::NonZeroU64::new(1).unwrap(),
        ..Default::default()
    };
    let mut restricted = super::super::ReadmissionScope::new(&fixture.ctx.home, limits);
    assert!(
        restricted
            .readmit_roots([root.as_path(), facts.payload.as_path()])
            .is_err()
    );
    assert_eq!(restricted.production_loads(), 1);
    assert_eq!(admitted.production_loads(), 1);
}

#[test]
fn retained_snapshot_walks_wide_trees_without_pinning_sibling_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path();
    let root = gripsack_store::content_path(home, "package-payload", &"a".repeat(64));
    fs::create_dir_all(&root).unwrap();
    // Exceed common per-process descriptor limits without changing the test
    // process's limits (other tests may be running concurrently).
    for index in 0..2048 {
        fs::create_dir(root.join(format!("member-{index}"))).unwrap();
    }
    let snapshot =
        super::super::RetainedInputSnapshot::capture(home, &root, FetchLimits::default()).unwrap();
    snapshot.check(home, &root).unwrap();
    fs::write(root.join("member-2047/changed"), b"changed").unwrap();
    assert!(snapshot.check(home, &root).is_err());
}

#[test]
fn operation_snapshot_pool_shares_256_member_scc_base_and_view_producers() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path();
    let mut scope = super::super::ReadmissionScope::new(home, FetchLimits::default());
    let mut members = Vec::new();
    for index in 0..256 {
        let identity =
            |member: usize| gripsack_process::Sha256Digest::of(&member.to_le_bytes()).to_string();
        let base = gripsack_store::content_path(home, "package", &identity(index * 3));
        let view = gripsack_store::content_path(home, "package", &identity(index * 3 + 1));
        let producer =
            gripsack_store::content_path(home, "package-payload", &identity(index * 3 + 2));
        for root in [&base, &view, &producer] {
            fs::create_dir_all(root).unwrap();
        }
        // Base and runtime-view receipts have distinct identities but the same
        // producer. Holding both pairs must not pin that producer twice.
        let base_evidence = (
            scope.pin_input(&base).unwrap(),
            scope.pin_input(&producer).unwrap(),
        );
        let view_evidence = (
            scope.pin_input(&view).unwrap(),
            scope.pin_input(&producer).unwrap(),
        );
        members.push((base, base_evidence, view_evidence));
    }
    // A single 256-member runtime ring revisits every base from another view;
    // these graph edges must also share the existing exact-root evidence.
    for index in 0..members.len() {
        let next = &members[(index + 1) % members.len()];
        scope.pin_input(&next.0).unwrap();
    }
    assert_eq!(scope.pinned_inputs(), 768);
    fs::write(members[0].0.join("changed"), b"changed").unwrap();
    assert!(scope.check_input(&members[0].0).is_err());
    assert_eq!(scope.pinned_inputs(), 0);
    assert!(scope.pin_input(&members[1].0).is_err());
}
