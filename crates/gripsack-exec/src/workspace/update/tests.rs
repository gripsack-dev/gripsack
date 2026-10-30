//! Real captured-workspace updates: complete surveys and publication authority.
use super::*;
use crate::Repository;
use gripsack_ir::{HostName, workspace_model::lock::WorkspaceLock};
use gripsack_store::{
    source_bundle::{SourceBundle, SourceCapturePolicy},
    trust::evaluation::EvaluationId,
};
use std::{collections::BTreeSet, fs, path::PathBuf, sync::Arc};

struct Fixture {
    temporary: tempfile::TempDir,
    repo: PathBuf,
    frontend: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        let home = temporary.path().join("home");
        for path in [&repo, &frontend, &home] {
            fs::create_dir(path).unwrap();
        }
        fs::write(repo.join("gripsack.ts"), "// captured workspace\n").unwrap();
        fs::write(repo.join("source"), "captured source\n").unwrap();
        fs::write(frontend.join("driver.ts"), "// captured frontend\n").unwrap();
        Self {
            temporary,
            repo,
            frontend,
            home,
        }
    }

    fn external(&self, name: &str, bytes: &str) -> PathBuf {
        let path = self.temporary.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn context(&self) -> Ctx {
        let sources = SourceBundle::capture(
            &self.repo,
            &self.frontend,
            None,
            &self.home,
            SourceCapturePolicy::default(),
        )
        .unwrap();
        Ctx {
            home: self.home.clone(),
            repository: Repository::evaluated(
                Arc::new(sources),
                EvaluationId::parse(&"a".repeat(64)).unwrap(),
            ),
            only: Vec::new(),
            host: HostName::parse("survey").unwrap(),
            on_progress: None,
            take_over: false,
            take_over_entries: None,
            home_dir: Default::default(),
            fetch: Default::default(),
        }
    }

    fn lock_bytes(&self) -> Vec<u8> {
        fs::read(self.repo.join("gripsack.lock")).expect("published lock missing")
    }

    fn cache(&self) -> BTreeSet<std::ffi::OsString> {
        match fs::read_dir(self.home.join("store")) {
            Ok(entries) => entries.map(|entry| entry.unwrap().file_name()).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => panic!("cannot inspect source cache: {error}"),
        }
    }
}

fn workspace(sources: &[(&str, PathBuf)]) -> Ir {
    let outputs: Vec<_> = sources
        .iter()
        .map(|(name, path)| {
            serde_json::json!({
                "kind": "package", "name": name, "span": {"file":"gripsack.ts","line":2},
                "producer": {"kind":"provider", "provider":{
                    "kind":"fetch", "fetch":{"kind":"file","path":path},
                    "span":{"file":"gripsack.ts","line":3}
                }},
                "commands": {}, "layout":{"kind":"relocatable"},
                "target":{"os":"linux","arch":"x86_64"}
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "ir_version":gripsack_ir::IR_VERSION,
        "host":{"os":"linux","arch":"x86_64"},
        "workspace":{"span":{"file":"gripsack.ts","line":1},"outputs":outputs}
    }))
    .unwrap()
}

#[test]
fn check_and_publish_agree_on_metadata_only_pin_changes() {
    let fixture = Fixture::new();
    let source = fixture.external("external", "unchanged source bytes\n");
    let ir = workspace(&[("tool", source)]);
    update(&ir, &fixture.context(), UpdateMode::Publish).unwrap();
    let mut lock: WorkspaceLock = serde_json::from_slice(&fixture.lock_bytes()).unwrap();
    let pin = &mut lock.resolutions.values_mut().next().unwrap().pins[0];
    pin.resolved.version = Some("retained metadata".into());
    fs::write(
        fixture.repo.join("gripsack.lock"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let before = fixture.lock_bytes();
    let cache = fixture.cache();

    let check = update(&ir, &fixture.context(), UpdateMode::Check).unwrap();
    assert_eq!(
        check.summary().changed(),
        1,
        "check_full_entry_change_was_missed"
    );
    assert_eq!(check.reports()[0].output, "tool");
    assert_eq!(fixture.lock_bytes(), before, "check_published_lock");
    assert_eq!(fixture.cache(), cache, "check_published_source_cache");

    let published = update(&ir, &fixture.context(), UpdateMode::Publish).unwrap();
    assert_eq!(published.summary().changed(), check.summary().changed());
    let after = fixture.lock_bytes();
    assert_ne!(after, before, "publish_dropped_lock_effect");
    let lock: WorkspaceLock = serde_json::from_slice(&after).unwrap();
    assert!(
        lock.resolutions.values().next().unwrap().pins[0]
            .resolved
            .version
            .is_none()
    );
    let current = update(&ir, &fixture.context(), UpdateMode::Check).unwrap();
    assert_eq!(
        current.summary().outcome(),
        crate::report::UpdateCheckOutcome::Current
    );
}

#[test]
fn explicit_update_migrates_retained_lock_format_without_replacing_unselected_pins() {
    let fixture = Fixture::new();
    let ir = workspace(&[
        ("tool", fixture.external("tool", "selected tool\n")),
        ("other", fixture.external("other", "unselected tool\n")),
    ]);
    update(&ir, &fixture.context(), UpdateMode::Publish).unwrap();
    let mut legacy: WorkspaceLock = serde_json::from_slice(&fixture.lock_bytes()).unwrap();
    legacy.lock_version = 2;
    let before = serde_json::to_vec_pretty(&legacy).unwrap();
    WorkspaceLock::from_json(std::str::from_utf8(&before).unwrap()).unwrap();
    fs::write(fixture.repo.join("gripsack.lock"), &before).unwrap();
    let mut context = fixture.context();
    context.only = vec!["tool".into()];

    let checked = update(&ir, &context, UpdateMode::Check).unwrap();
    assert_eq!(checked.summary().changed(), 1);
    assert_eq!(fixture.lock_bytes(), before);
    update(&ir, &context, UpdateMode::Publish).unwrap();
    let current =
        WorkspaceLock::from_json(std::str::from_utf8(&fixture.lock_bytes()).unwrap()).unwrap();
    assert_eq!(current.lock_version, WORKSPACE_LOCK_VERSION);
    assert_eq!(current.definitions, legacy.definitions);
    assert_eq!(current.resolutions, legacy.resolutions);
}

#[test]
fn selected_survey_preserves_every_result_and_only_complete_publish_changes_lock() {
    let fixture = Fixture::new();
    let changed = fixture.external("changed", "old source\n");
    let unchanged = fixture.external("unchanged", "same source\n");
    let mut sources = vec![
        ("a-changed", changed.clone()),
        ("c-unchanged", unchanged),
        ("d-skipped", PathBuf::from("source")),
    ];
    update(
        &workspace(&sources),
        &fixture.context(),
        UpdateMode::Publish,
    )
    .expect("selected_survey_did_not_account_for_all_outputs");
    let before = fixture.lock_bytes();
    let cache = fixture.cache();
    fs::write(&changed, "new source\n").unwrap();
    sources.push(("b-missing", fixture.temporary.path().join("missing")));
    let ir = workspace(&sources);
    let check = update(&ir, &fixture.context(), UpdateMode::Check)
        .expect("selected_survey_did_not_account_for_all_outputs");
    let names: Vec<_> = check
        .reports()
        .iter()
        .map(|report| report.output.as_str())
        .collect();
    assert_eq!(
        names,
        ["a-changed", "b-missing", "c-unchanged", "d-skipped"],
        "selected_survey_did_not_account_for_all_outputs"
    );
    assert_eq!(
        (
            check.summary().changed(),
            check.summary().failed(),
            check.summary().unchanged(),
            check.summary().skipped()
        ),
        (1, 1, 1, 1),
        "survey_disposition_counts_misclassified"
    );
    assert_eq!(
        check.summary().outcome(),
        crate::report::UpdateCheckOutcome::Incomplete,
        "survey_error_did_not_take_precedence_over_change"
    );
    assert_eq!(fixture.cache(), cache, "check_published_source_cache");
    assert_eq!(fixture.lock_bytes(), before, "check_published_lock");

    assert!(update(&ir, &fixture.context(), UpdateMode::Publish).is_err());
    assert_eq!(
        fixture.lock_bytes(),
        before,
        "failed_update_partially_published_lock"
    );

    fixture.external("missing", "newly available source\n");
    let published = update(&ir, &fixture.context(), UpdateMode::Publish).unwrap();
    assert_eq!(published.summary().changed(), 2);
    assert_ne!(fixture.lock_bytes(), before, "publish_dropped_lock_effect");
    let current = update(&ir, &fixture.context(), UpdateMode::Check).unwrap();
    assert_eq!(
        current.summary().outcome(),
        crate::report::UpdateCheckOutcome::Current
    );
    assert_eq!(current.summary().unchanged(), 3);
    assert_eq!(current.summary().skipped(), 1);
}
