use super::format::*;
use super::target::{PortableTarget, RuntimeAbi};
use std::collections::BTreeMap;

fn manifest() -> PortableManifest {
    PortableManifest {
        version: 1,
        namespace: "fixtures/tools".into(),
        publisher: "fixture".into(),
        target: PortableTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            abi: RuntimeAbi::Linux {},
            minimum_kernel: "3.2".into(),
            cpu: "x86_64".into(),
        },
        layout: RuntimeLayout::Relocatable {
            contract: RuntimeContract {
                behavior: "static command; no producer paths".into(),
            },
        },
        components: vec![Component {
            id: "app".into(),
            payload: Some(BlobDescriptor {
                sha256: "a".repeat(64),
                size: 10,
                media_type: TREE_MEDIA_TYPE.into(),
            }),
            tree: Some("b".repeat(64)),
            runtime: vec![],
            conda: None,
            external: None,
            inventory: vec![PortableEntry {
                path: "bin/tool".into(),
                mode: 0o755,
                kind: EntryKind::File {
                    sha256: "c".repeat(64),
                    size: 1,
                },
            }],
        }],
        roots: vec!["app".into()],
        commands: BTreeMap::from([(
            "tool".into(),
            PortableCommand {
                component: "app".into(),
                executable: "bin/tool".into(),
                args: vec![],
            },
        )]),
        environment: BTreeMap::new(),
        licenses: vec![],
        redistribution_authorized: true,
    }
}
#[test]
fn canonical_roundtrip_and_exact_manifest_identity() {
    let value = manifest();
    let bytes = value.encode().unwrap();
    assert_eq!(PortableManifest::decode(&bytes).unwrap(), value);
    let mut changed = value.clone();
    changed.publisher = "other".into();
    assert_ne!(changed.digest().unwrap(), value.digest().unwrap());
    assert_eq!(changed.components[0].tree, value.components[0].tree);
    let mut whitespace = bytes.clone();
    whitespace.push(b'\n');
    assert!(PortableManifest::decode(&whitespace).is_err());
}
#[test]
fn aggregate_cycles_have_finite_identity_and_complete_reachability() {
    let mut value = manifest();
    let mut other = value.components[0].clone();
    other.id = "lib".into();
    other.runtime = vec!["app".into()];
    value.components[0].runtime = vec!["lib".into()];
    value.components.push(other);
    assert!(value.encode().is_ok());
    value.components[0].runtime.clear();
    assert!(value.encode().is_err());
    value.components[0].runtime = vec!["missing".into()];
    assert!(value.encode().is_err());
}
#[test]
fn unknown_authority_and_duplicate_keys_are_not_inert_extensions() {
    let value = manifest();
    let bytes = value.encode().unwrap();
    for injected in [
        "\"tasks\":{},",
        "\"hooks\":[],",
        "\"schedule\":{},",
        "\"version\":1,",
    ] {
        let mut attack = Vec::from(b"{");
        attack.extend_from_slice(injected.as_bytes());
        attack.extend_from_slice(&bytes[1..]);
        assert!(PortableManifest::decode(&attack).is_err());
    }
}
#[test]
fn path_mode_graph_and_byte_bounds_fail_closed() {
    for path in ["../secret", "/host", "a/../b", "a\\b", "a//b", "é"] {
        let mut value = manifest();
        value.components[0].inventory[0].path = path.into();
        assert!(value.encode().is_err());
    }
    for mode in [0o4755, 0o2755, 0o777, 0o666] {
        let mut value = manifest();
        value.components[0].inventory[0].mode = mode;
        assert!(value.encode().is_err());
    }
    let mut value = manifest();
    value.components[0].payload.as_mut().unwrap().size = MAX_PAYLOAD_BYTES + 1;
    assert!(value.encode().is_err());
    let mut value = manifest();
    value.environment.insert(
        "TOKEN_PATH".into(),
        PortableEnvironmentValue::Literal {
            value: "/producer/home".into(),
        },
    );
    assert!(value.encode().is_err());
    let mut value = manifest();
    value.redistribution_authorized = false;
    assert!(value.encode().is_err());
}
#[test]
fn unsupported_target_and_incoherent_conda_layout_reject() {
    let mut value = manifest();
    value.target.os = "darwin".into();
    assert!(value.encode().is_err());
    let mut value = manifest();
    value.components[0].payload = None;
    assert!(value.encode().is_err());
}

#[test]
fn independent_references_have_distinct_authority_and_one_materialization_kind() {
    let mut value = manifest();
    let component = &mut value.components[0];
    component.external = Some(ExternalReference {
        manifest: "d".repeat(64),
        namespace: "upstream/tools".into(),
        publisher: "upstream".into(),
    });
    assert!(value.encode().is_err());
    let component = &mut value.components[0];
    component.payload = None;
    component.tree = None;
    component.inventory.clear();
    assert!(value.encode().is_ok());
    value.components[0].external.as_mut().unwrap().namespace = "../upstream".into();
    assert!(value.encode().is_err());
}

#[test]
fn environment_data_uses_only_literal_or_managed_component_selectors() {
    let mut value = manifest();
    value.layout = RuntimeLayout::Environment {};
    value.environment.insert(
        "CMAKE_PREFIX_PATH".into(),
        PortableEnvironmentValue::Artifact {
            output: "app".into(),
            selector: ".".into(),
        },
    );
    assert!(value.encode().is_ok());
    value.environment.insert(
        "PKG_CONFIG_PATH".into(),
        PortableEnvironmentValue::Artifact {
            output: "app".into(),
            selector: "../host".into(),
        },
    );
    assert!(value.encode().is_err());
    value.environment.remove("PKG_CONFIG_PATH");
    for key in ["PATH", "LD_PRELOAD", "PYTHONHOME", "BASH_ENV"] {
        value.environment.insert(
            key.into(),
            PortableEnvironmentValue::Literal {
                value: "unsafe".into(),
            },
        );
        assert!(value.encode().is_err());
        value.environment.remove(key);
    }
    let mut wire = serde_json::to_value(&value).unwrap();
    wire["environment"]["CMAKE_PREFIX_PATH"] =
        serde_json::json!({"kind":"input","input":"host-secrets"});
    assert!(serde_json::from_value::<PortableManifest>(wire).is_err());
}
