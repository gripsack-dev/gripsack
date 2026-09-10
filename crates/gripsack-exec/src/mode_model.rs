//! 0043: mode policy through the shipped planner AND filesystem executor.
//! TLC supplies the independent transition policy; these cases bind it to Rust.
//! Source-policy selection in deploy/preview is covered by executable-template e2e.

use crate::ctx::Ctx;
use crate::ops::{
    DestView, ModeInput, Op, OpKind, WritePermissions, execute_op, plan_entry_op, plan_remove_op,
    plan_restore_op,
};
use gripsack_ir::{Entry, Ownership};
use gripsack_store as store;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const MODES: [u32; 3] = [0o644, 0o755, 0o600];

fn context(home: &Path) -> Ctx {
    Ctx {
        home: home.into(),
        repo: home.into(),
        only: vec![],
        host: "model".into(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        jobs: Some(1),
        home_dir: Default::default(),
        fetch: Default::default(),
    }
}

fn entry(home: &Path, mode: Ownership) -> Entry {
    Entry {
        from: "payload".into(),
        to: home.join("dest").display().to_string(),
        mode,
        vars: Default::default(),
        marker: None,
        span: None,
    }
}

fn permissions(dest: &Path) -> u32 {
    std::fs::metadata(dest).unwrap().permissions().mode() & 0o7777
}

fn chmod(dest: &Path, mode: u32) {
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn plan(
    ctx: &Ctx,
    entry: &Entry,
    prev: Option<&store::DeployedEntry>,
    content: &[u8],
    policy: WritePermissions,
    takeover: bool,
) -> Op {
    let dest = Path::new(&entry.to);
    let observed = crate::deploy::observe_readonly(dest).unwrap();
    let input = if entry.mode == Ownership::Merge {
        ModeInput::Merge {
            payload: std::str::from_utf8(content).unwrap(),
            permissions: policy,
        }
    } else {
        ModeInput::Write {
            content,
            permissions: policy,
        }
    };
    plan_entry_op(
        &DestView {
            module: "m",
            entry,
            dest: dest.into(),
            home: &ctx.home,
            observed,
            prev,
            take_over: takeover,
        },
        input,
    )
    .unwrap()
}

fn execute(ctx: &Ctx, op: &Op) -> store::DeployedEntry {
    execute_op(
        ctx.home_dir().unwrap(),
        &ctx.home,
        op.as_executable().unwrap(),
    )
    .unwrap();
    let p = op.produces().unwrap();
    store::DeployedEntry {
        from: p.from.clone(),
        to: op.declared_to().to_string(),
        key: Some(op.dest().to_path_buf()),
        mode: p.mode.clone(),
        vars: p.vars.clone(),
        hash: p.hash.clone(),
        file_mode: p.file_mode,
        source_executable: p.source_executable,
        prior: None,
        preserved_drift: p.preserved_drift,
    }
}

#[test]
fn whole_file_modes_survive_update_restore_drift_and_prune() {
    let mut cases = 0;
    for ownership in [Ownership::TrackedCopy, Ownership::Template] {
        for source_mode in MODES {
            for acquired in [None, Some(0o600), Some(0o644), Some(0o755)] {
                let dir = tempfile::tempdir().unwrap();
                let ctx = context(dir.path());
                let entry = entry(dir.path(), ownership.clone());
                let dest = Path::new(&entry.to);
                let payload_dir = dir.path().join("store-payload");
                std::fs::create_dir(&payload_dir).unwrap();
                std::fs::write(payload_dir.join("payload"), b"one").unwrap();
                chmod(&payload_dir.join("payload"), source_mode);
                if let Some(mode) = acquired {
                    std::fs::write(dest, b"foreign").unwrap();
                    chmod(dest, mode);
                }
                let executable = source_mode & 0o111 != 0;
                let source = WritePermissions::Source { executable };
                let expected = acquired.unwrap_or(if executable { 0o755 } else { 0o644 });
                let first = execute(
                    &ctx,
                    &plan(&ctx, &entry, None, b"one", source, acquired.is_some()),
                );
                assert_eq!(
                    permissions(dest),
                    expected,
                    "{ownership:?}, source={source_mode:o}"
                );
                let unchanged = plan(&ctx, &entry, Some(&first), b"one", source, false);
                assert!(matches!(unchanged.kind(), OpKind::Satisfied));
                let updated = execute(
                    &ctx,
                    &plan(&ctx, &entry, Some(&first), b"two", source, false),
                );
                assert_eq!(std::fs::read(dest).unwrap(), b"two");
                assert_eq!(permissions(dest), expected);
                let toggled = execute(
                    &ctx,
                    &plan(
                        &ctx,
                        &entry,
                        Some(&updated),
                        b"two",
                        WritePermissions::Source {
                            executable: !executable,
                        },
                        false,
                    ),
                );
                let delta = if executable {
                    expected & !0o111
                } else {
                    expected | ((expected & 0o444) >> 2)
                };
                assert_eq!(
                    permissions(dest),
                    delta,
                    "execute delta widened acquired permissions"
                );
                let restore = plan_restore_op("m", &first, &payload_dir, Some(&toggled), &ctx.home)
                    .unwrap()
                    .unwrap();
                let restored = execute(&ctx, &restore);
                assert_eq!(std::fs::read(dest).unwrap(), b"one");
                assert_eq!(
                    permissions(dest),
                    expected,
                    "rollback must restore the full recorded mode"
                );
                let changed_mode = if expected == 0o600 { 0o640 } else { 0o600 };
                chmod(dest, changed_mode);
                let mut prev = restored;
                for _ in 0..2 {
                    let drift = plan(&ctx, &entry, Some(&prev), b"one", source, false);
                    assert!(
                        matches!(drift.kind(), OpKind::Preserved),
                        "chmod must not be satisfied"
                    );
                    prev = execute(&ctx, &drift);
                    assert_eq!(permissions(dest), changed_mode);
                    let prune = plan_remove_op("m", &prev, &payload_dir, &ctx.home)
                        .unwrap()
                        .unwrap();
                    assert!(
                        matches!(prune.kind(), OpKind::Preserved),
                        "observation became delete authority"
                    );
                    execute_op(
                        ctx.home_dir().unwrap(),
                        &ctx.home,
                        prune.as_executable().unwrap(),
                    )
                    .unwrap();
                    assert_eq!(std::fs::read(dest).unwrap(), b"one");
                }
                cases += 1;
            }
        }
    }
    eprintln!(
        "mode explorer: {cases} copy/template deployment, source-delta, rollback, repeated-drift and prune traces"
    );
}

#[test]
fn merge_host_modes_are_planned_and_drift_never_authorizes_prune() {
    for initial in [None, Some(0o600), Some(0o644), Some(0o755)] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(dir.path());
        let entry = entry(dir.path(), Ownership::Merge);
        let dest = Path::new(&entry.to);
        if let Some(mode) = initial {
            std::fs::write(dest, b"foreign\n").unwrap();
            chmod(dest, mode);
        }
        let expected = initial.unwrap_or(0o644);
        let first = execute(
            &ctx,
            &plan(
                &ctx,
                &entry,
                None,
                b"managed",
                WritePermissions::Preserve,
                false,
            ),
        );
        assert_eq!(permissions(dest), expected);
        assert_eq!(
            crate::managed_blocks::ManagedBlockSet::parse(
                &std::fs::read_to_string(dest).unwrap(),
                "m"
            )
            .unwrap()
            .blocks()[0]
                .mode,
            Some(expected)
        );
        let unchanged = plan(
            &ctx,
            &entry,
            Some(&first),
            b"managed",
            WritePermissions::Preserve,
            false,
        );
        assert!(matches!(unchanged.kind(), OpKind::Satisfied));
        let drift_mode = if expected == 0o600 { 0o644 } else { 0o600 };
        chmod(dest, drift_mode);
        let original = std::fs::read(dest).unwrap();
        let mut prev = first;
        for _ in 0..2 {
            let drift = plan(
                &ctx,
                &entry,
                Some(&prev),
                b"managed",
                WritePermissions::Preserve,
                false,
            );
            assert!(matches!(drift.kind(), OpKind::Preserved));
            prev = execute(&ctx, &drift);
            let prune = plan_remove_op("m", &prev, dir.path(), &ctx.home)
                .unwrap()
                .unwrap();
            assert!(matches!(prune.kind(), OpKind::Preserved));
            execute_op(
                ctx.home_dir().unwrap(),
                &ctx.home,
                prune.as_executable().unwrap(),
            )
            .unwrap();
            assert_eq!(std::fs::read(dest).unwrap(), original);
            assert_eq!(permissions(dest), drift_mode);
        }
        chmod(dest, expected);
        let converged = execute(
            &ctx,
            &plan(
                &ctx,
                &entry,
                Some(&prev),
                b"managed",
                WritePermissions::Preserve,
                false,
            ),
        );
        let prune = plan_remove_op("m", &converged, dir.path(), &ctx.home)
            .unwrap()
            .unwrap();
        execute_op(
            ctx.home_dir().unwrap(),
            &ctx.home,
            prune.as_executable().unwrap(),
        )
        .unwrap();
        if initial.is_some() {
            assert_eq!(std::fs::read(dest).unwrap(), b"foreign\n");
            assert_eq!(permissions(dest), expected);
        } else {
            assert!(!dest.exists());
        }
    }
}

#[test]
fn links_carry_the_payload_mode_without_normalizing_it() {
    for mode in MODES {
        let dir = tempfile::tempdir().unwrap();
        let ctx = context(dir.path());
        let entry = entry(dir.path(), Ownership::Owned);
        let source = dir.path().join("payload");
        std::fs::write(&source, b"payload").unwrap();
        chmod(&source, mode);
        let op = plan_entry_op(
            &DestView {
                module: "m",
                entry: &entry,
                dest: Path::new(&entry.to).into(),
                home: &ctx.home,
                observed: None,
                prev: None,
                take_over: false,
            },
            ModeInput::Link {
                source: &source,
                content_hash: store::canonical_file_hash(&source).unwrap().into(),
                already: false,
            },
        )
        .unwrap();
        execute_op(
            ctx.home_dir().unwrap(),
            &ctx.home,
            op.as_executable().unwrap(),
        )
        .unwrap();
        assert_eq!(std::fs::read_link(&entry.to).unwrap(), source);
        assert_eq!(permissions(Path::new(&entry.to)), mode);
    }
}
