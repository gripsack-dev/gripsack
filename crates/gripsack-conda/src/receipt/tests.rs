use super::*;
use serde_json::json;

fn record() -> LockedCondaPackage {
    serde_json::from_value(json!({
        "name": "python", "version": "3.12.0", "build": "hfixture_0",
        "build_number": 0, "subdir": "linux-64",
        "channel": "https://example.invalid/channel/",
        "url": "https://example.invalid/channel/linux-64/python-3.12.0-hfixture_0.conda",
        "sha256": "a".repeat(64),
        "depends": ["__glibc >=2.17"], "constrains": ["openssl >=3"],
        "python_site_packages_path": "lib/python3.12/site-packages",
        "run_exports": {"weak": ["python_abi 3.12.* *_cp312"]}
    }))
    .unwrap()
}

#[test]
fn receipts_cannot_drop_or_substitute_frozen_dependency_and_layout_authority() {
    let frozen = record();
    let files = BTreeSet::from(["bin/python".to_string()]);
    let mut honest = Vec::new();
    write(&mut honest, &frozen, &files, "/final prefix").unwrap();
    verify(honest.as_slice(), &frozen, &files, "/final prefix").unwrap();
    assert!(verify(honest.as_slice(), &frozen, &files, "/different prefix").is_err());

    let mut changed = frozen.clone();
    changed.depends.clear();
    let mut constrained = frozen.clone();
    constrained.constrains = vec!["openssl <3".into()];
    let mut layout = frozen.clone();
    layout.python_site_packages_path = Some("lib/python3.13/site-packages".into());
    let mut exports = frozen.clone();
    exports.run_exports = None;
    for substituted in [changed, constrained, layout, exports] {
        let mut forged = Vec::new();
        write(&mut forged, &substituted, &files, "/final prefix").unwrap();
        assert!(verify(forged.as_slice(), &frozen, &files, "/final prefix").is_err());
    }
    let forged_files = BTreeSet::from(["bin/python".to_string(), "unowned".to_string()]);
    let mut forged = Vec::new();
    write(&mut forged, &frozen, &forged_files, "/final prefix").unwrap();
    assert!(verify(forged.as_slice(), &frozen, &files, "/final prefix").is_err());
    for length in [0, honest.len() / 2, honest.len() - 1] {
        assert!(verify(&honest[..length], &frozen, &files, "/final prefix").is_err());
    }
    honest.extend_from_slice(b"{}");
    assert!(verify(honest.as_slice(), &frozen, &files, "/final prefix").is_err());
}
