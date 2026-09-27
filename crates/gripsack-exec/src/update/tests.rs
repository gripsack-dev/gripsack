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
    let check = update(&ir, &ctx, UpdateMode::Check).unwrap();
    assert!(matches!(check[0].status, UpdateStatus::Bumped { .. }));
    let publish = update(&ir, &ctx, UpdateMode::Publish).unwrap();
    assert!(matches!(publish[0].status, UpdateStatus::Bumped { .. }));
    let current = update(&ir, &ctx, UpdateMode::Check).unwrap();
    assert!(matches!(current[0].status, UpdateStatus::Unchanged));
}
