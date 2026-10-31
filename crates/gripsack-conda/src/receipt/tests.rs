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

#[test]
fn wheel_receipt_binds_original_requirements_ownership_and_final_prefix() {
    let wheel: LockedWheel = serde_json::from_value(json!({
        "name":"fixture-tool","version":"1.0","filename":"fixture_tool-1.0-py3-none-any.whl",
        "url":"https://example.invalid/fixture_tool-1.0-py3-none-any.whl",
        "sha256":"b".repeat(64),"size":256,
        "requires_python":">=3.12","requires_dist":["dependency>=2"]
    }))
    .unwrap();
    let files = BTreeSet::from([
        "bin/fixture-tool",
        "lib/python3.12/site-packages/fixture.py",
    ]);
    let mut bytes = Vec::new();
    write_wheel(&mut bytes, &wheel, &files, "/recipient prefix").unwrap();
    verify_wheel(bytes.as_slice(), &wheel, &files, "/recipient prefix").unwrap();
    assert_eq!(wheel_path(&wheel), "conda-meta/fixture-tool-1.0-wheel.json");
    assert!(verify_wheel(bytes.as_slice(), &wheel, &files, "/producer prefix").is_err());
    let mut changed = wheel.clone();
    changed.requires_dist.clear();
    assert!(verify_wheel(bytes.as_slice(), &changed, &files, "/recipient prefix").is_err());
    changed = wheel.clone();
    changed.sha256 = "c".repeat(64);
    assert!(verify_wheel(bytes.as_slice(), &changed, &files, "/recipient prefix").is_err());
    assert!(
        verify_wheel(
            bytes.as_slice(),
            &wheel,
            &BTreeSet::from(["bin/fixture-tool"]),
            "/recipient prefix"
        )
        .is_err()
    );
    bytes.extend_from_slice(b"{}");
    assert!(verify_wheel(bytes.as_slice(), &wheel, &files, "/recipient prefix").is_err());
}
