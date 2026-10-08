use super::same_source_declaration;
use gripsack_ir::workspace_model::{LockedCondaSource, LockedPixiSource, LockedSource};
use std::collections::BTreeMap;

fn declared_pixi() -> LockedPixiSource {
    LockedPixiSource {
        manifest: "manifest".into(),
        lock: "lock".into(),
        environment: "default".into(),
        manifest_sha256: None,
        lock_sha256: None,
    }
}

#[test]
fn resolved_pixi_input_identities_are_not_declaration_fields() {
    let declared = declared_pixi();
    let mut captured = declared.clone();
    captured.manifest_sha256 = Some("ab".repeat(32));
    captured.lock_sha256 = Some("cd".repeat(32));
    assert!(same_source_declaration(
        &LockedSource::PixiLock(captured),
        &LockedSource::PixiLock(declared),
    ));
}

#[test]
fn changed_pixi_manifest_lock_or_environment_is_not_the_frozen_declaration() {
    let mut captured = declared_pixi();
    captured.manifest_sha256 = Some("ab".repeat(32));
    captured.lock_sha256 = Some("cd".repeat(32));
    let captured = LockedSource::PixiLock(captured);
    let mut manifest = declared_pixi();
    manifest.manifest = "other-manifest".into();
    let mut lock = declared_pixi();
    lock.lock = "other-lock".into();
    let mut environment = declared_pixi();
    environment.environment = "other-environment".into();
    for changed in [manifest, lock, environment] {
        assert!(!same_source_declaration(
            &captured,
            &LockedSource::PixiLock(changed),
        ));
    }
}

#[test]
fn declaration_comparison_preserves_source_kind_and_conda_requirements() {
    let original = LockedCondaSource {
        channels: vec!["conda-forge".into()],
        packages: BTreeMap::from([("nodejs".into(), "=26.10.0".into())]),
        platforms: vec![],
    };
    let mut changed = original.clone();
    changed.packages.insert("nodejs".into(), "*".into());
    let original = LockedSource::CondaEnvironment(original);
    assert!(!same_source_declaration(
        &original,
        &LockedSource::CondaEnvironment(changed),
    ));
    assert!(!same_source_declaration(
        &original,
        &LockedSource::PixiLock(declared_pixi()),
    ));
}
