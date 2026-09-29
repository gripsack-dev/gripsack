use std::os::unix::fs::MetadataExt;

use super::*;

#[test]
fn check_and_publish_agree_on_metadata_only_pin_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let source = repo.join("source.txt");
    std::fs::write(&source, b"unchanged source bytes").unwrap();
    let ctx = Ctx {
        home: temporary.path().join("home"),
        repo,
        only: vec![],
        host: gripsack_ir::HostName::parse("testhost").unwrap(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        jobs: Some(1),
        fetch: Default::default(),
        home_dir: Default::default(),
    };
    let ir: Ir = serde_json::from_value(serde_json::json!({
        "ir_version": gripsack_ir::IR_VERSION,
        "modules": {"source": {"fetch": {"kind": "file", "path": source}}}
    }))
    .unwrap();
    update(&ir, &ctx, UpdateMode::Publish).unwrap();
    let crate::lockfile::LockRead::Parsed(mut lock) = crate::lockfile::read(&ctx.repo, &ctx.host)
    else {
        panic!("published lock missing");
    };
    lock.modules
        .get_mut("source")
        .unwrap()
        .resolved
        .as_mut()
        .unwrap()
        .api_url = Some("https://unused.invalid/old-source-metadata".into());
    crate::lockfile::write(&ctx.repo, &ctx.host, &lock).unwrap();
    let lock_path = crate::lockfile::path(&ctx.repo, &ctx.host);
    let before_check = std::fs::read(&lock_path).unwrap();
    let pinned_lock = std::fs::File::open(&lock_path).unwrap();
    let check = update(&ir, &ctx, UpdateMode::Check).unwrap();
    assert!(
        matches!(check.reports()[0].status, UpdateStatus::Bumped { .. }),
        "check_full_entry_change_was_missed",
    );
    assert_eq!(
        std::fs::read(&lock_path).unwrap(),
        before_check,
        "check_published_lock"
    );
    assert_eq!(
        std::fs::metadata(&lock_path).unwrap().ino(),
        pinned_lock.metadata().unwrap().ino(),
        "check_published_lock",
    );
    let publish = update(&ir, &ctx, UpdateMode::Publish).unwrap();
    assert!(matches!(
        publish.reports()[0].status,
        UpdateStatus::Bumped { .. }
    ));
    let current = update(&ir, &ctx, UpdateMode::Check).unwrap();
    assert!(matches!(
        current.reports()[0].status,
        UpdateStatus::Unchanged
    ));
    let mut formatted_lock = std::fs::read(&lock_path).unwrap();
    formatted_lock.extend_from_slice(b" \n");
    std::fs::write(&lock_path, &formatted_lock).unwrap();
    let unchanged = update(&ir, &ctx, UpdateMode::Publish).unwrap();
    assert_eq!(
        unchanged.summary().outcome(),
        crate::UpdateCheckOutcome::Current
    );
    assert_eq!(
        std::fs::read(&lock_path).unwrap(),
        formatted_lock,
        "unchanged_update_rewrote_lock",
    );
}

#[test]
fn selected_survey_preserves_every_result_and_only_complete_publish_changes_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let source = repo.join("source.txt");
    let missing_source = repo.join("missing.txt");
    std::fs::write(&source, b"first source").unwrap();
    let mut ctx = Ctx {
        home: temporary.path().join("home"),
        repo,
        only: vec!["alpha".into()],
        host: gripsack_ir::HostName::parse("testhost").unwrap(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        jobs: Some(1),
        fetch: Default::default(),
        home_dir: Default::default(),
    };
    let ir: Ir = serde_json::from_value(serde_json::json!({
        "ir_version": gripsack_ir::IR_VERSION,
        "modules": {
            "alpha": {"fetch": {"kind": "file", "path": source}},
            "gamma": {},
            "omega": {"fetch": {"kind": "file", "path": missing_source}}
        }
    }))
    .unwrap();
    update(&ir, &ctx, UpdateMode::Publish).unwrap();
    let lock_path = crate::lockfile::path(&ctx.repo, &ctx.host);
    let lock_before = std::fs::read(&lock_path).unwrap();
    let store_root = ctx.home.join("store");
    let store_before = gripsack_store::canonical_tree_hash(&store_root).unwrap();
    std::fs::write(&source, b"changed source").unwrap();
    ctx.only = ["omega", "alpha", "gamma", "missing", "missing"]
        .into_iter()
        .map(String::from)
        .collect();

    let survey = update(&ir, &ctx, UpdateMode::Check)
        .expect("selected_survey_did_not_account_for_all_modules");
    let observed: std::collections::BTreeMap<_, _> = survey
        .reports()
        .iter()
        .map(|report| {
            let kind = match report.status {
                UpdateStatus::Unchanged => "unchanged",
                UpdateStatus::Bumped { .. } => "changed",
                UpdateStatus::Skipped { .. } => "skipped",
                UpdateStatus::Failed { .. } => "failed",
            };
            (report.module.as_str(), kind)
        })
        .collect();
    assert_eq!(
        observed,
        std::collections::BTreeMap::from([
            ("alpha", "changed"),
            ("gamma", "skipped"),
            ("missing", "failed"),
            ("omega", "failed"),
        ]),
        "selected_survey_lost_or_reclassified_a_module",
    );
    let summary = survey.summary();
    assert_eq!(summary.selected(), 4);
    assert_eq!(
        (
            summary.unchanged(),
            summary.changed(),
            summary.skipped(),
            summary.failed()
        ),
        (0, 1, 1, 2),
        "survey_disposition_counts_misclassified",
    );
    assert_eq!(summary.outcome(), crate::UpdateCheckOutcome::Incomplete);
    assert!(!summary.publishes_lock(UpdateMode::Publish));
    assert_eq!(
        std::fs::read(&lock_path).unwrap(),
        lock_before,
        "check_published_lock"
    );
    assert_eq!(
        gripsack_store::canonical_tree_hash(&store_root).unwrap(),
        store_before,
        "check_published_source_cache",
    );

    assert!(update(&ir, &ctx, UpdateMode::Publish).is_err());
    assert_eq!(
        std::fs::read(&lock_path).unwrap(),
        lock_before,
        "failed_update_partially_published_lock",
    );
    std::fs::write(&missing_source, b"repaired source").unwrap();
    let repaired = update(&ir, &ctx, UpdateMode::Publish).unwrap();
    assert_eq!(repaired.summary().changed(), 2);
    assert_eq!(repaired.summary().skipped(), 2);
    assert_eq!(repaired.summary().failed(), 0);
    assert!(repaired.summary().publishes_lock(UpdateMode::Publish));
    let crate::lockfile::LockRead::Parsed(lock) = crate::lockfile::read(&ctx.repo, &ctx.host)
    else {
        panic!("completed update did not publish its lock");
    };
    assert_eq!(
        lock.modules.keys().map(String::as_str).collect::<Vec<_>>(),
        ["alpha", "omega"],
    );
}
