//! An admitted workspace must never fall through to an empty legacy
//! module apply, even through direct executor APIs (not only the CLI).

use gripsack_exec::{Ctx, ExecError, PlanError, UpdateMode, apply, build_order, update};
use gripsack_ir::{check, codes};
use serde_json::json;

fn workspace_ir(version: u32) -> String {
    let layout = if version == 4 {
        json!("relocatable")
    } else {
        json!({"kind": "relocatable"})
    };
    json!({
        "ir_version": version,
        "host": {"os": "linux", "arch": "x86_64"},
        "workspace": {
            "span": {"file": "gripsack.ts", "line": 1},
            "outputs": [{
                "kind": "package", "name": "tool",
                "span": {"file": "gripsack.ts", "line": 3},
                "producer": {"kind": "provider", "provider": {
                    "fetch": {"kind": "file", "path": "tool.bin"},
                    "span": {"file": "gripsack.ts", "line": 4}
                }},
                "commands": {"tool": "bin/tool"},
                "target": {"os": "linux", "arch": "x86_64"},
                "layout": layout
            }]
        }
    })
    .to_string()
}

#[test]
fn every_direct_executor_entry_rejects_before_home_mutation() {
    for version in [4, 5] {
        let ir = check(&workspace_ir(version)).expect("versioned workspace admitted");
        assert!(ir.has_workspace());
        let ordering = build_order(&ir).unwrap_err();
        assert!(
            matches!(&ordering, PlanError::WorkspaceUnavailable(d) if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE)
        );

        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("not-created");
        let ctx = Ctx {
            home: home.clone(),
            repo: sandbox.path().to_path_buf(),
            only: vec![],
            host: "model".into(),
            on_progress: None,
            take_over: false,
            take_over_entries: None,
            jobs: Some(1),
            fetch: Default::default(),
            home_dir: Default::default(),
        };
        assert!(
            matches!(&apply(&ir, &ctx), Err(ExecError::Gate(d)) if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE)
        );
        assert!(!home.exists(), "apply must not create home for v{version}");
        assert!(
            matches!(&update(&ir, &ctx, UpdateMode::Check), Err(ExecError::Gate(d)) if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE)
        );
        assert!(!home.exists(), "update must not create home for v{version}");
        let ops = gripsack_exec::ops::preview_ops(
            &ir,
            sandbox.path(),
            None,
            &Default::default(),
            &gripsack_exec::lockfile::Lockfile::default(),
        );
        assert!(
            matches!(&ops, Err(ExecError::Gate(d)) if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE)
        );
        assert!(
            !home.exists(),
            "preview must remain read-only for v{version}"
        );
    }
}
