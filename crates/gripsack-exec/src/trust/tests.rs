mod contracts;

use super::*;
use gripsack_host::process::{Limits, OperatorEnvironment, SelectedProgram};
use std::path::Path;
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
}

#[test]
fn native_policy_and_source_changes_move_the_policy_digest() {
    let fixture = Fixture::new();
    let original = fixture.capture();
    let policy = fixture.policy(&original, &());
    let expanded = fixture.policy(
        &original,
        &serde_json::json!({"fetcher":"explicit-native-fixture"}),
    );
    assert_ne!(policy.digest().unwrap(), expanded.digest().unwrap());
    fs::write(
        fixture.repo.join("module.ts"),
        b"export const version = 2;\n",
    )
    .unwrap();
    let changed = fixture.capture();
    assert_ne!(original.digest(), changed.digest());
    let changed_policy = fixture.policy(&changed, &());
    assert_ne!(policy.digest().unwrap(), changed_policy.digest().unwrap());
}
