//! Real captured-workspace updates: complete surveys and publication authority.
use super::*;
use crate::Repository;
use crate::{
    source_bundle::{SourceBundle, SourceCapturePolicy},
    trust::evaluation::EvaluationId,
};
use gripsack_ir::{HostName, workspace_model::lock::WorkspaceLock};
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

#[test]
fn scoped_update_migrates_large_legacy_rows_and_resumes_segment_compaction() {
    use crate::workspace::{conda, roots};
    use gripsack_ir::workspace_model::{
        identity::conda_closure_digest,
        lock::{LockedCondaEnvironment, ResolvedPinFields},
    };
    use serde_json::json;

    let mut fixture = Fixture::new();
    // Long but ordinary home paths reach the real 1-MiB metadata boundary
    // with a small lock. GC admits references to absent/repaired store bytes;
    // this tests that same persisted-root contract, not a mock GC.
    for _ in 0..5 {
        fixture.home.push("h".repeat(160));
    }
    fs::create_dir_all(&fixture.home).unwrap();
    let names = ["anchor", "old-a", "old-b", "old-c", "survives"];
    let sources = names.map(|name| (name, PathBuf::from("source")));
    let ir = workspace(&sources);
    let scoped = |names: &[&str]| {
        let mut ctx = fixture.context();
        ctx.only = names.iter().map(|s| s.to_string()).collect();
        ctx
    };
    update(&ir, &scoped(&["anchor"]), UpdateMode::Publish).unwrap();
    let mut lock: WorkspaceLock = serde_json::from_slice(&fixture.lock_bytes()).unwrap();
    let resolution = lock.resolutions.entry("linux-x86_64".into()).or_insert(PlatformResolution {
        pins: Vec::new(), transitive: Vec::new(),
    });
    let session = LifecycleSession::acquire(&fixture.home).unwrap();
    let mut members = BTreeSet::new();
    let mut survivor = BTreeSet::new();
    for (group, name) in names[1..].iter().enumerate() {
        let mut paths = BTreeSet::new();
        let packages: Vec<_> = (0..850).map(|n| {
            let digest = format!("{group:02x}{n:030x}{n:032x}");
            paths.insert(conda::archive_object(&fixture.home, &digest));
            json!({
                "name": format!("pkg-{n:04}"), "version":"1.0", "build":"0", "build_number":0,
                "subdir":"linux-64", "channel":"https://example.invalid", "sha256":digest,
                "url":format!("https://example.invalid/linux-64/pkg-{n:04}-1.0-0.conda"),
            })
        }).collect();
        let frozen: LockedCondaEnvironment = serde_json::from_value(json!({
            "platform":"linux-64", "channels":["https://example.invalid"], "channel_priority":"strict",
            "system_requirements":{}, "packages":packages,
            "materializer":{"bytecode":"suppress","receipt":"normalized_conda_meta"}
        })).unwrap();
        resolution.pins.push(LockedPin {
            output: name.to_string(),
            source: serde_json::from_value(json!({
                "kind":"conda_environment", "channels":["https://example.invalid"], "packages":{"pkg-0000":"*"},
                "platforms":[]
            })).unwrap(),
            resolved: ResolvedPinFields { sha256: Some(conda_closure_digest(&frozen).to_string()), ..Default::default() },
            conda: Some(frozen),
        });
        members.extend(paths.iter().map(|p| p.to_str().unwrap().to_owned()));
        if *name == "survives" {
            survivor = paths.iter().map(|p| p.to_str().unwrap().to_owned()).collect();
        }
        let closure = roots::RetentionSet::admit(&session, paths).unwrap();
        roots::register_output_root(&session, &roots::built_output_id(&fixture.repo, name).unwrap(), &closure).unwrap();
    }
    drop(session);
    lock.validate().unwrap();
    fs::write(fixture.repo.join("gripsack.lock"), serde_json::to_vec(&lock).unwrap()).unwrap();
    let original = fixture.lock_bytes();
    update(&ir, &scoped(&["anchor"]), UpdateMode::Publish).unwrap();
    assert_eq!(fixture.lock_bytes(), original, "root-only migration rewrote the lock");
    let home = gripsack_host::fs::open(&fixture.home).unwrap();
    assert_eq!(roots::inventory(&home, &fixture.home).unwrap(), members);
    let directory = fixture.home.join("roots/outputs");
    let mut segments = Vec::new();
    for index in 0.. {
        let id = roots::RootId::from_identity(&serde_json::to_vec(&(&fixture.repo, "conda-lock", index)).unwrap());
        let path = directory.join(format!("{}.json", id.as_str()));
        if !path.exists() { break; }
        segments.push((path.clone(), fs::read(path).unwrap()));
    }
    assert!(segments.len() >= 3, "fixture must cross multiple root bounds");
    assert_eq!(fs::read_dir(&directory).unwrap().count(), segments.len());
    update(&ir, &scoped(&["old-a", "old-b", "old-c"]), UpdateMode::Publish).unwrap();
    assert_eq!(roots::inventory(&home, &fixture.home).unwrap(), survivor);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    let settled = fixture.lock_bytes();

    // The durable replacement of slot zero precedes each highest-first tail
    // unlink. Reconstruct every observable remaining tail from its real bytes;
    // an unchanged scoped retry must find and trim it without losing survivors.
    for tail in 1..segments.len() {
        for (path, bytes) in &segments[1..=tail] {
            fs::write(path, bytes).unwrap();
        }
        assert!(survivor.is_subset(&roots::inventory(&home, &fixture.home).unwrap()));
        update(&ir, &scoped(&["anchor"]), UpdateMode::Publish).unwrap();
        assert_eq!(fixture.lock_bytes(), settled);
        assert_eq!(roots::inventory(&home, &fixture.home).unwrap(), survivor);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    }
}
