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
    use crate::ops::{Authority, DestView, ModeInput, OpKind, execute_op, plan_entry_op};
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
            mode: Ownership::TrackedCopy,
            vars: Default::default(),
            file_mode: None,
            hash: store::canonical_bytes_identity(content.as_bytes(), 0o644).to_string(),
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
                intent_mode: 0o644,
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
    }

    fn live_label(live: &Live) -> String {
        match live {
            Live::Absent => "absent".into(),
            Live::File(c, m) => format!("file({c},{m:o})"),
            Live::Link(t) => format!("link({t})"),
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
        }
    }
}
