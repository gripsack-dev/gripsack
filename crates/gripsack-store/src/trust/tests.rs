mod contracts;

use super::*;
use gripsack_process::{Limits, OperatorEnvironment, SelectedProgram};
use std::{ffi::OsString, fs, path::PathBuf, time::Instant};

struct Fixture {
    _temporary: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    frontend: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("state");
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&frontend).unwrap();
        fs::write(repo.join("module.ts"), b"export const version = 1;\n").unwrap();
        fs::write(frontend.join("driver.ts"), b"// trusted fixture driver\n").unwrap();
        Self {
            _temporary: temporary,
            home,
            repo,
            frontend,
        }
    }

    fn capture(&self) -> SourceBundle {
        SourceBundle::capture(
            &self.repo,
            &self.frontend,
            None,
            &self.home,
            crate::source_bundle::SourceCapturePolicy::default(),
        )
        .unwrap()
    }

    fn policy(&self, bundle: &SourceBundle, native: &impl serde::Serialize) -> EvaluationPolicy {
        let environment =
            OperatorEnvironment::admit([(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))])
                .unwrap();
        let limits = Limits::default();
        let selected = SelectedProgram::select(
            &environment,
            Path::new("/bin/sh"),
            None,
            Instant::now() + limits.timeout,
        )
        .unwrap();
        EvaluationPolicy::capture(
            bundle,
            selected.identity(),
            native,
            limits,
            4,
            16 * 1024 * 1024,
        )
        .unwrap()
    }

    fn approve(&self, bundle: &SourceBundle, policy: &EvaluationPolicy) {
        approve(
            &self.home,
            bundle,
            policy,
            bundle.digest(),
            policy.digest().unwrap(),
            &GitProvenance::default(),
        )
        .unwrap();
    }
}

#[test]
fn source_or_native_policy_change_requires_explicit_renewal() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    let policy = fixture.policy(&original, &());
    assert_eq!(
        status(&fixture.home, &original, &policy).unwrap(),
        ApprovalStatus::Unapproved
    );
    fixture.approve(&original, &policy);
    assert_eq!(
        status(&fixture.home, &original, &policy).unwrap(),
        ApprovalStatus::Approved
    );
    let expanded = fixture.policy(
        &original,
        &serde_json::json!({"fetcher":"explicit-native-fixture"}),
    );
    assert_eq!(
        status(&fixture.home, &original, &expanded).unwrap(),
        ApprovalStatus::Changed {
            source_changed: false,
            policy_changed: true
        }
    );
    assert!(
        approve(
            &fixture.home,
            &original,
            &expanded,
            original.digest(),
            policy.digest().unwrap(),
            &GitProvenance::default()
        )
        .is_err()
    );
    fs::write(
        fixture.repo.join("module.ts"),
        b"export const version = 2;\n",
    )
    .unwrap();
    let changed = fixture.capture();
    let changed_policy = fixture.policy(&changed, &());
    assert!(matches!(
        status(&fixture.home, &changed, &changed_policy).unwrap(),
        ApprovalStatus::Changed {
            source_changed: true,
            ..
        }
    ));
    assert!(
        approve(
            &fixture.home,
            &changed,
            &changed_policy,
            original.digest(),
            changed_policy.digest().unwrap(),
            &GitProvenance::default()
        )
        .is_err()
    );
    assert_eq!(
        list(&fixture.home).unwrap().approved[0].bundle,
        original.digest()
    );
    let difference = inspect(&fixture.home, &changed, &changed_policy).unwrap();
    assert!(
        difference
            .changes
            .iter()
            .any(|change| change.path == "repo/module.ts"
                && matches!(change.change, InventoryChangeKind::Changed))
    );
    fixture.approve(&changed, &changed_policy);
    assert_eq!(
        status(&fixture.home, &changed, &changed_policy).unwrap(),
        ApprovalStatus::Approved
    );
    assert_eq!(list(&fixture.home).unwrap().approved.len(), 1);
}

#[test]
fn legacy_records_are_visible_but_never_invent_source_authority() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("trust.toml"), format!(
        "[[repos]]\npath = {:?}\nremote = \"https://owner:secret@example.test/repo?token=secret\"\ntrusted_at = \"2026-09-01T00:00:00Z\"\n",
        fixture.repo.to_str().unwrap(),
    )).unwrap();
    let bundle = fixture.capture();
    let policy = fixture.policy(&bundle, &());
    assert_eq!(
        status(&fixture.home, &bundle, &policy).unwrap(),
        ApprovalStatus::Legacy
    );
    let legacy = list(&fixture.home).unwrap();
    assert!(legacy.approved.is_empty());
    assert_eq!(
        legacy.legacy[0].remote.as_deref(),
        Some("https://example.test/repo")
    );
    fixture.approve(&bundle, &policy);
    let renewed = list(&fixture.home).unwrap();
    assert!(renewed.legacy.is_empty());
    assert_eq!(
        status(&fixture.home, &bundle, &policy).unwrap(),
        ApprovalStatus::Approved
    );
    assert!(remove(&fixture.home, &fixture.repo).unwrap());
    assert_eq!(
        status(&fixture.home, &bundle, &policy).unwrap(),
        ApprovalStatus::Unapproved
    );
}

#[test]
fn missing_corrupt_or_redirected_inventory_cannot_authorize_source() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let bundle = fixture.capture();
    let policy = fixture.policy(&bundle, &());
    fixture.approve(&bundle, &policy);
    let inventory = fixture
        .home
        .join("trust/inventories")
        .join(format!("{}.json", bundle.digest()));
    assert_eq!(
        fs::metadata(&inventory).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&inventory, b"corrupt retained inventory").unwrap();
    assert!(status(&fixture.home, &bundle, &policy).is_err());
    fs::remove_file(&inventory).unwrap();
    assert!(status(&fixture.home, &bundle, &policy).is_err());
    let foreign = fixture.repo.join("foreign-metadata");
    fs::write(&foreign, bundle.inventory_bytes()).unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&foreign, &inventory).unwrap();
    assert!(status(&fixture.home, &bundle, &policy).is_err());
    assert_eq!(
        fs::metadata(&foreign).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(fs::read(&foreign).unwrap(), bundle.inventory_bytes());
}

#[test]
fn concurrent_source_approvals_preserve_both_repositories() {
    let first = Fixture::new();
    let second = Fixture::new();
    let first_bundle = first.capture();
    let second_bundle = second.capture();
    let first_policy = first.policy(&first_bundle, &());
    let second_policy = second.policy(&second_bundle, &());
    std::thread::scope(|scope| {
        let left = scope.spawn(|| first.approve(&first_bundle, &first_policy));
        let right = scope.spawn(|| first.approve(&second_bundle, &second_policy));
        left.join().unwrap();
        right.join().unwrap();
    });
    let approvals = list(&first.home).unwrap();
    assert_eq!(approvals.approved.len(), 2);
    assert_eq!(
        status(&first.home, &first_bundle, &first_policy).unwrap(),
        ApprovalStatus::Approved
    );
    assert_eq!(
        status(&first.home, &second_bundle, &second_policy).unwrap(),
        ApprovalStatus::Approved
    );
}
