//! The VM-level harness (0034's explorer extension): the kernel's
//! `ownership::decide_unit` is the DECISION (proved in
//! proofs/Kernel/Ownership.lean); this harness checks the LIFTING — that
//! the op planner, fed real observations and manifest records, computes
//! the decision the kernel gives for independently computed identities,
//! and that executing the op lands exactly its recorded intent. Enumerated abstract states are materialized
//! onto a real filesystem, planned with the shipped planner, executed
//! with the shipped executor, and checked: nothing drifts between the
//! model's answer and the machine's.

#[cfg(test)]
mod tests {
    use crate::ctx::Ctx;
    use crate::ops::{
        Authority, DestView, ModeInput, OpKind, PreviousRecord, execute_op, plan_entry_op,
        preview_ops, take_over_consent,
    };
    use gripsack_ir::HostName;
    use gripsack_kernel::ownership::{Decision, Recorded, Refusal, decide_unit};
    use gripsack_kernel::recover::{Ident, Intended as KernelIntended, Live as KernelLive};
    use gripsack_store as store;
    use gripsack_store::{DeploymentEntry, Ownership};
    use std::path::{Path, PathBuf};
    use store::hash::kernel;

    /// An abstract live state, materialized per case.
    #[derive(Clone, Copy)]
    enum Live {
        Absent,
        File(&'static str, u32),
        Link(&'static str),
    }

    /// The manifest's record: (deployed content, preserved flag).
    #[derive(Clone, Copy)]
    struct PrevCase(Option<(&'static str, bool)>);

    struct Case {
        live: Live,
        desired: &'static str,
        prev: PrevCase,
        take_over: bool,
    }

    fn entry(mode: Ownership, to: &Path) -> DeploymentEntry {
        DeploymentEntry {
            from: "payload".into(),
            to: to.to_string_lossy().into_owned(),
            mode,
            vars: Default::default(),
            marker: None,
            span: None,
        }
    }

    fn materialize(home: &Path, live: &Live) -> PathBuf {
        let dest = home.join("dest");
        match live {
            Live::Absent => {}
            Live::File(content, mode) => {
                std::fs::write(&dest, content).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(*mode))
                        .unwrap();
                }
                #[cfg(not(unix))]
                let _ = mode;
            }
            Live::Link(target) => {
                std::os::unix::fs::symlink(target, &dest).unwrap();
            }
        }
        dest
    }

    fn live_manifest_identity(dest: &Path) -> Option<String> {
        match std::fs::symlink_metadata(dest) {
            Err(_) => None,
            Ok(m) if m.file_type().is_symlink() => Some(
                store::canonical_bytes_hash(
                    std::fs::read_link(dest)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes(),
                )
                .to_string(),
            ),
            Ok(m) => {
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::MetadataExt;
                    m.mode() & 0o7777
                };
                #[cfg(not(unix))]
                let mode = 0o644;
                Some(
                    store::canonical_bytes_identity(&std::fs::read(dest).unwrap(), mode)
                        .to_string(),
                )
            }
        }
    }

    fn prev_entry(prev: &PrevCase) -> Option<store::DeployedEntry> {
        prev.0.map(|(content, preserved)| store::DeployedEntry {
            from: "payload".into(),
            to: String::new(),
            key: None,
            ownership: store::StoredOwnership::Legacy(Ownership::TrackedCopy),
            vars: Default::default(),
            file_mode: None,
            source_executable: None,
            hash: store::canonical_bytes_identity(content.as_bytes(), 0o644).into(),
            prior: None,
            preserved_drift: preserved,
        })
    }

    /// The live object's kernel identity, computed from the filesystem
    /// independently of the planner's observation.
    fn kernel_live(dest: &Path) -> KernelLive {
        match std::fs::symlink_metadata(dest) {
            Err(_) => KernelLive::Absent,
            Ok(m) if m.file_type().is_symlink() => KernelLive::Obj(kernel::link(
                std::fs::read_link(dest)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes(),
            )),
            Ok(m) => {
                use std::os::unix::fs::MetadataExt;
                KernelLive::Obj(kernel::file(&store::canonical_bytes_identity(
                    &std::fs::read(dest).unwrap(),
                    m.mode() & 0o7777,
                )))
            }
        }
    }

    fn file_ident(content: &str) -> Ident {
        kernel::file(&store::canonical_bytes_identity(content.as_bytes(), 0o644))
    }

    /// The kernel's answer for the case.
    fn expected_plan(case: &Case, dest: &Path) -> Decision {
        let recorded = match case.prev.0 {
            None => Recorded::Unrecorded,
            Some((_, true)) => Recorded::Observed,
            Some((content, false)) => Recorded::Wrote(file_ident(content)),
        };
        decide_unit(
            &recorded,
            &kernel_live(dest),
            &KernelIntended::Obj(file_ident(case.desired)),
            &take_over_consent(case.take_over),
        )
    }

    /// The kernel's decision as the op kind/authority the planner must
    /// produce for `kind` ("write" or "link").
    fn expected_shape(decision: Decision, kind: &'static str) -> (&'static str, Option<Authority>) {
        match decision {
            Decision::Create => (kind, Some(Authority::Fresh)),
            Decision::ReplaceOwn => (kind, Some(Authority::Update)),
            Decision::TakeOver => (kind, Some(Authority::TakeOver)),
            Decision::Satisfied => ("satisfied", None),
            // a copy preserves drift silently; a link refusal names it
            Decision::Refuse(_) if kind == "write" => ("preserved", None),
            Decision::Refuse(Refusal::Drifted) => ("preserved", Some(Authority::Drifted)),
            Decision::Refuse(_) => ("preserved", Some(Authority::Foreign)),
        }
    }

    fn actual_shape(
        kind: &OpKind,
        authority: Option<Authority>,
    ) -> (&'static str, Option<Authority>) {
        match kind {
            OpKind::Write { .. } => ("write", authority),
            OpKind::Link { .. } => ("link", authority),
            OpKind::Satisfied => ("satisfied", authority),
            OpKind::Preserved => ("preserved", authority),
            other => panic!("unexpected op kind for a whole-object unit: {other:?}"),
        }
    }

    #[test]
    fn the_planned_op_matches_the_algebra_and_executes_to_its_intent() {
        let mut checked = 0usize;
        for live in [
            Live::Absent,
            Live::File("0", 0o644),
            Live::File("1", 0o644),
            Live::File("2", 0o644),
            Live::File("3", 0o644),
            Live::File("1", 0o600), // chmod-only drift
            Live::Link("foreign-target"),
        ] {
            for desired in ["1", "2"] {
                for prev in [
                    PrevCase(None),
                    PrevCase(Some(("1", false))),
                    PrevCase(Some(("3", true))),
                ] {
                    for take_over in [false, true] {
                        let case = Case {
                            live,
                            desired,
                            prev,
                            take_over,
                        };
                        check_one(&case);
                        checked += 1;
                    }
                }
            }
        }
        eprintln!("op model: {checked} cases planned + executed, zero divergences");
    }

    fn check_one(case: &Case) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let payload_dir = home.join("payload");
        std::fs::create_dir_all(&payload_dir).unwrap();
        std::fs::write(payload_dir.join("payload"), case.desired).unwrap();

        // materialize live state
        let dest = materialize(home, &case.live);
        // reset the materialized dest's name into the entry
        let entry = entry(Ownership::TrackedCopy, &dest);

        let prev = prev_entry(&case.prev);
        let (dest_dir, dest_name) = crate::deploy::dest_capability(&dest).unwrap();
        let observed = crate::deploy::observe(&dest_dir, &dest_name).unwrap();
        let view = DestView {
            output: "m",
            entry: &entry,
            dest: dest.clone(),
            home,
            observed,
            prev: prev.as_ref().map(|entry| PreviousRecord {
                entry,
                store_path: &payload_dir,
            }),
            consent: take_over_consent(case.take_over),
        };
        let op = plan_entry_op(
            &view,
            ModeInput::Write {
                content: case.desired.as_bytes(),
                permissions: crate::ops::WritePermissions::Exact(0o644),
            },
        )
        .unwrap();

        // the planner lifts the kernel: same decision
        let expected = expected_shape(expected_plan(case, &dest), "write");
        assert_eq!(
            actual_shape(op.kind(), op.authority()),
            expected,
            "case: live={:?} desired={} prev={:?} take_over={}",
            live_label(&case.live),
            case.desired,
            case.prev.0,
            case.take_over
        );

        // execute: writes land the recorded intent; inert ops move
        // nothing
        let before = store::journal::live_identity(&dest_dir, &dest_name).unwrap();
        let ctx = model_ctx(home);
        let capability = ctx.home_dir().unwrap();
        let run = store::journal::begin_run(
            capability,
            home,
            None,
            store::GenerationId::new(1),
            store::journal::RunOp::Apply,
        )
        .unwrap();
        let (_report, prior) = execute_op(&run, op.as_executable().unwrap()).unwrap();
        assert_eq!(
            prior.is_some(),
            op.authority() == Some(Authority::TakeOver),
            "only a genuine acquisition captures a manifest prior"
        );
        let after = store::journal::live_identity(&dest_dir, &dest_name).unwrap();
        match op.kind() {
            OpKind::Write { .. } | OpKind::Link { .. } | OpKind::MergeUpsert { .. } => {
                assert_eq!(
                    after.as_ref(),
                    op.intended().as_object(),
                    "the executed op did not land its intent"
                );
            }
            OpKind::Satisfied | OpKind::Preserved => {
                assert_eq!(before, after, "an inert op moved the filesystem");
            }
            _ => unreachable!(),
        }
        if let Some(produced) = &op.produces() {
            assert_eq!(
                live_manifest_identity(&dest).as_deref(),
                Some(produced.hash.as_str()),
                "the manifest receipt does not describe what execution left"
            );
        }
    }

    /// FND-1 and the store-prefix widening at the lifting: an owned link
    /// is gripsack's only when the manifest records exactly that link (the
    /// previous generation's `store_path/from`). A user's file or link —
    /// including one into the store — is refused (or taken over), and the
    /// recorded link is replaced only while it is still live.
    #[test]
    fn owned_links_follow_recorded_identity_only() {
        let mut checked = 0usize;
        for live in [
            "absent",
            "recorded",
            "intended",
            "user-file",
            "user-store-link",
            "user-link",
        ] {
            for recorded in [false, true] {
                for take_over in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let home = dir.path();
                    let old_store = home.join("store/old");
                    let new_store = home.join("store/new");
                    for root in [&old_store, &new_store] {
                        std::fs::create_dir_all(root).unwrap();
                        std::fs::write(root.join("payload"), "payload").unwrap();
                    }
                    let source = new_store.join("payload");
                    let dest = home.join("dest");
                    match live {
                        "absent" => {}
                        "recorded" => {
                            std::os::unix::fs::symlink(old_store.join("payload"), &dest).unwrap()
                        }
                        "intended" => std::os::unix::fs::symlink(&source, &dest).unwrap(),
                        "user-file" => std::fs::write(&dest, "mine").unwrap(),
                        "user-store-link" => {
                            std::os::unix::fs::symlink(home.join("store/made-by-me"), &dest)
                                .unwrap()
                        }
                        _ => std::os::unix::fs::symlink("/elsewhere", &dest).unwrap(),
                    }
                    let entry = entry(Ownership::Owned, &dest);
                    let prev = store::DeployedEntry {
                        from: "payload".into(),
                        to: String::new(),
                        key: None,
                        ownership: store::StoredOwnership::Legacy(Ownership::Owned),
                        vars: Default::default(),
                        file_mode: None,
                        source_executable: None,
                        hash: store::canonical_file_hash(&source).unwrap().into(),
                        prior: None,
                        preserved_drift: false,
                    };
                    let view = DestView {
                        output: "m",
                        entry: &entry,
                        dest: dest.clone(),
                        home,
                        observed: crate::deploy::observe_readonly(&dest).unwrap(),
                        prev: recorded.then_some(PreviousRecord {
                            entry: &prev,
                            store_path: &old_store,
                        }),
                        consent: take_over_consent(take_over),
                    };
                    let op = plan_entry_op(
                        &view,
                        ModeInput::Link {
                            source: &source,
                            content_hash: prev.hash.clone(),
                        },
                    )
                    .unwrap();
                    let recorded_identity = if recorded {
                        Recorded::Wrote(kernel::link(
                            old_store.join("payload").as_os_str().as_encoded_bytes(),
                        ))
                    } else {
                        Recorded::Unrecorded
                    };
                    let decision = decide_unit(
                        &recorded_identity,
                        &kernel_live(&dest),
                        &KernelIntended::Obj(kernel::link(source.as_os_str().as_encoded_bytes())),
                        &take_over_consent(take_over),
                    );
                    assert_eq!(
                        actual_shape(op.kind(), op.authority()),
                        expected_shape(decision, "link"),
                        "live={live} recorded={recorded} take_over={take_over}"
                    );
                    // the two defects, stated directly
                    let writes = matches!(op.kind(), OpKind::Link { .. });
                    if !take_over && live.starts_with("user") {
                        assert!(
                            !writes,
                            "a user's object was re-pointed: {live} recorded={recorded}"
                        );
                    }
                    if live == "recorded" && recorded {
                        assert_eq!(op.authority(), Some(Authority::Update));
                    }
                    checked += 1;
                }
            }
        }
        eprintln!("owned-link lifting: {checked} cases, zero divergences");
    }

    #[test]
    fn legacy_template_receipts_do_not_acquire_intact_owned_bytes() {
        for preserved in [false, true] {
            for live_mode in [0o600, 0o640] {
                for take_over in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let home = dir.path();
                    let dest = materialize(home, &Live::File("old", live_mode));
                    let entry = entry(Ownership::Template, &dest);
                    let mut prev = prev_entry(&PrevCase(Some(("old", preserved)))).unwrap();
                    prev.ownership = store::StoredOwnership::Legacy(Ownership::Template);
                    prev.hash = store::canonical_bytes_hash(b"old").into();
                    prev.file_mode = Some(0o600);
                    let view = DestView {
                        output: "m",
                        entry: &entry,
                        dest: dest.clone(),
                        home,
                        observed: crate::deploy::observe_readonly(&dest).unwrap(),
                        prev: Some(PreviousRecord {
                            entry: &prev,
                            store_path: home,
                        }),
                        consent: take_over_consent(take_over),
                    };
                    let op = plan_entry_op(
                        &view,
                        ModeInput::Write {
                            content: b"new",
                            permissions: crate::ops::WritePermissions::Exact(0o600),
                        },
                    )
                    .unwrap();
                    let expected = if !preserved && live_mode == 0o600 {
                        ("write", Some(Authority::Update))
                    } else if take_over {
                        ("write", Some(Authority::TakeOver))
                    } else {
                        ("preserved", None)
                    };
                    assert_eq!(actual_shape(op.kind(), op.authority()), expected);
                }
            }
        }
    }

    fn live_label(live: &Live) -> String {
        match live {
            Live::Absent => "absent".into(),
            Live::File(c, m) => format!("file({c},{m:o})"),
            Live::Link(t) => format!("link({t})"),
        }
    }

    /// 0035 F1 end to end through the planner: the same file declared
    /// by two spellings plans satisfied on the second — the canonical
    /// key, never the string, decides.
    #[test]
    fn a_spelling_change_plans_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let payload_dir = home.join("payload");
        std::fs::create_dir_all(&payload_dir).unwrap();
        std::fs::write(payload_dir.join("payload"), "1").unwrap();

        // deploy via the tilde spelling
        let tilde = home.join("dest");
        let entry_tilde = entry(Ownership::TrackedCopy, &tilde);
        let ctx = model_ctx(home);
        let capability = ctx.home_dir().unwrap();
        let run = store::journal::begin_run(
            capability,
            home,
            None,
            store::GenerationId::new(1),
            store::journal::RunOp::Apply,
        )
        .unwrap();
        let (dest_dir, dest_name) = crate::deploy::dest_capability(&tilde).unwrap();
        let observed = crate::deploy::observe(&dest_dir, &dest_name).unwrap();
        let view = DestView {
            output: "m",
            entry: &entry_tilde,
            dest: tilde.clone(),
            home,
            observed,
            prev: None,
            consent: take_over_consent(false),
        };
        let op = plan_entry_op(
            &view,
            ModeInput::Write {
                content: b"1",
                permissions: crate::ops::WritePermissions::Exact(0o644),
            },
        )
        .unwrap();
        assert!(matches!(op.kind(), OpKind::Write { .. }));
        let _ = execute_op(&run, op.as_executable().unwrap()).unwrap();
        let produced = op.produces().expect("a write produces an entry");

        // the same file declared by its absolute spelling: prev's key
        // joins the lineage, and the op is satisfied — not a prune,
        // not a rewrite
        let entry_abs = entry(Ownership::TrackedCopy, &tilde);
        let (dest_dir, dest_name) = crate::deploy::dest_capability(&tilde).unwrap();
        let observed = crate::deploy::observe(&dest_dir, &dest_name).unwrap();
        let prev = store::DeployedEntry {
            from: produced.from.clone(),
            // the recorded spelling is the OLD one — the new entry
            // declares the same file differently
            to: "~/dest".into(),
            key: None, // pre-0.32 shape: the read path canonicalizes
            ownership: store::StoredOwnership::Legacy(Ownership::TrackedCopy),
            vars: Default::default(),
            file_mode: produced.file_mode,
            source_executable: produced.source_executable,
            prior: None,
            hash: produced.hash.clone(),
            preserved_drift: false,
        };
        let view = DestView {
            output: "m",
            entry: &entry_abs,
            dest: tilde.clone(),
            home,
            observed,
            prev: Some(PreviousRecord {
                entry: &prev,
                store_path: &payload_dir,
            }),
            consent: take_over_consent(false),
        };
        let op = plan_entry_op(
            &view,
            ModeInput::Write {
                content: b"1",
                permissions: crate::ops::WritePermissions::Exact(0o644),
            },
        )
        .unwrap();
        assert!(
            matches!(op.kind(), OpKind::Satisfied),
            "the absolute spelling of a deployed file must plan satisfied, got {:?}",
            op.kind()
        );
    }

    /// Inert task declarations contribute no destination operations and cannot
    /// change the ownership plan of a selected profile.
    #[test]
    fn an_inert_task_plans_zero_destination_ops() {
        for live in [
            Live::Absent,
            Live::File("foreign", 0o600),
            Live::Link("foreign"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path();
            let dest = materialize(repo, &live);
            std::fs::write(repo.join("payload"), "built artifact").unwrap();
            let mut ir: gripsack_ir::Ir = serde_json::from_value(serde_json::json!({
                "ir_version": gripsack_ir::IR_VERSION,
                "host": {"os":"linux","arch":"x86_64"},
                "workspace": {
                    "span":{"file":"model.ts","line":1},
                    "outputs":[{
                        "kind":"profile","name":"consumer",
                        "span":{"file":"model.ts","line":2},
                        "files":[{
                            "span":{"file":"model.ts","line":3},
                            "source":{"kind":"repo_file","path":"payload"},
                            "content":{"kind":"identity"},
                            "destination":{"kind":"tracked_copy","path":dest}
                        }]
                    }]
                }
            }))
            .unwrap();
            let adopting = Default::default();
            let repository = crate::Repository::direct(repo.to_path_buf());
            let runtime =
                preview_ops(&ir, &repository, None, &adopting, Default::default(), &[]).unwrap();
            ir.workspace.outputs.push(
                serde_json::from_value(serde_json::json!({
                    "kind":"task","name":"compiler",
                    "span":{"file":"model.ts","line":4},
                    "context":{"kind":"host","mutable_paths":[]},
                    "steps":[{"command":{
                        "kind":"exec","span":{"file":"model.ts","line":5},
                        "argv":[{"kind":"literal","value":"/bin/false"}]
                    }}]
                }))
                .unwrap(),
            );
            let build =
                preview_ops(&ir, &repository, None, &adopting, Default::default(), &[]).unwrap();
            assert!(
                build.iter().all(|op| op.output != "compiler"),
                "an uninvoked task must produce no destination or effect operation"
            );
            assert_eq!(build.len(), runtime.len());

            let control = runtime.iter().find(|o| o.output == "consumer").unwrap();
            let actual = build.iter().find(|o| o.output == "consumer").unwrap();
            assert_eq!(actual.dest, control.dest);
            assert_eq!(
                std::mem::discriminant(&actual.kind),
                std::mem::discriminant(&control.kind)
            );
            assert_eq!(actual.authority, control.authority);
            assert_eq!(actual.observed, control.observed);
            assert_eq!(actual.intended, control.intended);
        }
    }
    fn model_ctx(home: &Path) -> Ctx {
        Ctx {
            home: home.to_path_buf(),
            repository: crate::Repository::direct(home.to_path_buf()),
            only: vec![],
            host: HostName::parse("test").unwrap(),
            on_progress: None,
            take_over: false,
            take_over_entries: Default::default(),
            home_dir: std::sync::OnceLock::new(),
            fetch: std::sync::Arc::new(gripsack_fetch::FetchContext::new(Default::default())),
        }
    }
}
