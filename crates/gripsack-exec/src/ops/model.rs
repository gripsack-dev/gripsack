//! The VM-level harness (0034's explorer extension): the lineage
//! explorer proves the DECISION algebra (plan_copy/plan_link); this
//! harness proves the LIFTING — that the op planner computes the same
//! decision the algebra does, and that executing the op lands exactly
//! its recorded intent. Enumerated abstract states are materialized
//! onto a real filesystem, planned with the shipped planner, executed
//! with the shipped executor, and checked: nothing drifts between the
//! model's answer and the machine's.

#[cfg(test)]
mod tests {
    use crate::ctx::Ctx;
    use crate::ops::{
        Authority, DestView, ModeInput, OpKind, execute_op, plan_entry_op, preview_ops,
    };
    use gripsack_ir::{Entry, Ownership};
    use gripsack_store as store;
    use std::path::{Path, PathBuf};

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

    fn entry(mode: Ownership, to: &Path) -> Entry {
        Entry {
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
            mode: Ownership::TrackedCopy,
            vars: Default::default(),
            file_mode: None,
            source_executable: None,
            hash: store::canonical_bytes_identity(content.as_bytes(), 0o644).into(),
            prior: None,
            preserved_drift: preserved,
        })
    }

    /// The algebra's answer for the case, via the shipped plan_copy.
    fn expected_plan(case: &Case, dest: &Path) -> crate::deploy::CopyPlan {
        let desired = store::canonical_bytes_identity(case.desired.as_bytes(), 0o644).to_string();
        let live = live_manifest_identity(dest);
        let prev = prev_entry(&case.prev);
        let prev_pair = prev.as_ref().map(|e| (e.hash.as_str(), e.preserved_drift));
        crate::deploy::plan_copy(&desired, live.as_deref(), prev_pair, case.take_over)
    }

    /// plan_copy's answer as the op kind/authority the planner must
    /// produce.
    fn expected_shape(plan: crate::deploy::CopyPlan) -> (&'static str, Option<Authority>) {
        match plan {
            crate::deploy::CopyPlan::Fresh => ("write", Some(Authority::Fresh)),
            crate::deploy::CopyPlan::Update => ("write", Some(Authority::Update)),
            crate::deploy::CopyPlan::TakeOver => ("write", Some(Authority::TakeOver)),
            crate::deploy::CopyPlan::Satisfied => ("satisfied", None),
            crate::deploy::CopyPlan::Preserve => ("preserved", None),
        }
    }

    fn actual_shape(
        kind: &OpKind,
        authority: Option<Authority>,
    ) -> (&'static str, Option<Authority>) {
        match kind {
            OpKind::Write { .. } => ("write", authority),
            OpKind::Satisfied => ("satisfied", authority),
            OpKind::Preserved => ("preserved", authority),
            other => panic!("unexpected op kind for a copy: {other:?}"),
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
            module: "m",
            entry: &entry,
            dest: dest.clone(),
            home,
            observed,
            prev: prev.as_ref(),
            take_over: case.take_over,
        };
        let op = plan_entry_op(
            &view,
            ModeInput::Write {
                content: case.desired.as_bytes(),
                permissions: crate::ops::WritePermissions::Exact(0o644),
            },
        )
        .unwrap();

        // the planner IS the algebra: same decision
        let expected = expected_shape(expected_plan(case, &dest));
        assert_eq!(
            actual_shape(&op.kind, op.authority),
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
        let (_report, _prior) = execute_op(ctx.home_dir().unwrap(), &ctx.home, &op).unwrap();
        let after = store::journal::live_identity(&dest_dir, &dest_name).unwrap();
        match op.kind {
            OpKind::Write { .. } | OpKind::Link { .. } | OpKind::MergeUpsert { .. } => {
                assert_eq!(
                    after.as_ref().map(|i| i.to_wire()),
                    Some(op.intended.to_wire()),
                    "the executed op did not land its intent"
                );
            }
            OpKind::Satisfied | OpKind::Preserved => {
                assert_eq!(before, after, "an inert op moved the filesystem");
            }
            _ => unreachable!(),
        }
        if let Some(produced) = &op.produces {
            assert_eq!(
                live_manifest_identity(&dest).as_deref(),
                Some(produced.hash.as_str()),
                "the manifest receipt does not describe what execution left"
            );
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
        let (dest_dir, dest_name) = crate::deploy::dest_capability(&tilde).unwrap();
        let observed = crate::deploy::observe(&dest_dir, &dest_name).unwrap();
        let view = DestView {
            module: "m",
            entry: &entry_tilde,
            dest: tilde.clone(),
            home,
            observed,
            prev: None,
            take_over: false,
        };
        let op = plan_entry_op(
            &view,
            ModeInput::Write {
                content: b"1",
                permissions: crate::ops::WritePermissions::Exact(0o644),
            },
        )
        .unwrap();
        assert!(matches!(op.kind, OpKind::Write { .. }));
        let _ = execute_op(ctx.home_dir().unwrap(), &ctx.home, &op).unwrap();
        let produced = op.produces.expect("a write produces an entry");

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
            mode: Ownership::TrackedCopy,
            vars: Default::default(),
            file_mode: produced.file_mode,
            source_executable: produced.source_executable,
            prior: None,
            hash: produced.hash.clone(),
            preserved_drift: false,
        };
        let view = DestView {
            module: "m",
            entry: &entry_abs,
            dest: tilde.clone(),
            home,
            observed,
            prev: Some(&prev),
            take_over: false,
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
            matches!(op.kind, OpKind::Satisfied),
            "the absolute spelling of a deployed file must plan satisfied, got {:?}",
            op.kind
        );
    }

    /// 0039's case class: a build-only dep in the graph plans ZERO
    /// destination ops — one marker instead — and the consumer's
    /// destination ops are exactly what they'd be without the edge.
    /// The shipped planner (preview_ops), not a reimplementation.
    #[test]
    fn a_build_only_dep_plans_zero_destination_ops() {
        for live in [
            Live::Absent,
            Live::File("foreign", 0o600),
            Live::Link("foreign"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path();
            let dest = materialize(repo, &live);
            std::fs::write(repo.join("payload"), "built artifact").unwrap();
            let mut ir = gripsack_ir::Ir {
                ir_version: gripsack_ir::IR_VERSION,
                host: Default::default(),
                resources: vec![],
                modules: [
                    (
                        "consumer".into(),
                        gripsack_ir::Module {
                            install: vec![entry(Ownership::TrackedCopy, &dest)],
                            ..Default::default()
                        },
                    ),
                    (
                        "compiler".into(),
                        gripsack_ir::Module {
                            install: vec![entry(
                                Ownership::TrackedCopy,
                                &repo.join("compiler-bin"),
                            )],
                            ..Default::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            };
            let lock = crate::lockfile::Lockfile::default();
            let adopting = Default::default();
            let runtime = preview_ops(&ir, repo, None, &adopting, &lock).unwrap();
            ir.modules
                .get_mut("consumer")
                .unwrap()
                .depends
                .push(gripsack_ir::Dependency {
                    module: "compiler".into(),
                    edge: gripsack_ir::EdgeKind::Build,
                    span: None,
                });
            let build = preview_ops(&ir, repo, None, &adopting, &lock).unwrap();
            let compiler: Vec<_> = build.iter().filter(|o| o.module == "compiler").collect();
            assert_eq!(
                compiler.len(),
                1,
                "exactly one closure marker per build-only module"
            );
            assert!(matches!(compiler[0].kind, OpKind::RunEffect));
            assert!(compiler[0].dest.as_os_str().is_empty());
            assert!(
                runtime
                    .iter()
                    .any(|o| o.module == "compiler" && !o.dest.as_os_str().is_empty())
            );

            let control = runtime.iter().find(|o| o.module == "consumer").unwrap();
            let actual = build.iter().find(|o| o.module == "consumer").unwrap();
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
            repo: home.to_path_buf(),
            only: vec![],
            host: "test".into(),
            on_progress: None,
            take_over: false,
            take_over_entries: Default::default(),
            jobs: Some(1),
            home_dir: std::sync::OnceLock::new(),
            fetch: std::sync::Arc::new(gripsack_fetch::FetchContext::new(Default::default())),
        }
    }
}
