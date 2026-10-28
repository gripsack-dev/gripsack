//! Direct executor entrypoints reject unsupported profile capabilities and
//! never invoke inert catalog tasks as a side effect of apply or preview.

use gripsack_exec::{Ctx, ExecError, apply};
use gripsack_ir::{HostName, check, codes};
use serde_json::json;

#[test]
fn unsupported_profile_schedules_reject_before_home_mutation() {
    let sandbox = tempfile::tempdir().unwrap();
    let marker = sandbox.path().join("task-must-not-run");
    let ir = check(
        &json!({
            "ir_version": gripsack_ir::IR_VERSION,
            "host": {"os":"linux","arch":"x86_64"},
            "workspace": {
                "span":{"file":"gripsack.ts","line":1},
                "outputs":[
                    {"kind":"profile","name":"desktop","span":{"file":"gripsack.ts","line":3},
                     "schedules":["daily"]},
                    {"kind":"task","name":"touch","span":{"file":"gripsack.ts","line":5},
                     "context":{"kind":"host","mutable_paths":[]},
                     "steps":[{"command":{"kind":"exec","span":{"file":"gripsack.ts","line":6},
                        "argv":[{"kind":"literal","value":"/usr/bin/touch"},
                                {"kind":"literal","value":marker}]}}]},
                    {"kind":"schedule","name":"daily","span":{"file":"gripsack.ts","line":8},
                     "task":"touch","trigger":{"kind":"daily","time":"09:00"},"scope":"user"}
                ]
            }
        })
        .to_string(),
    )
    .expect("current workspace admitted");
    let home = sandbox.path().join("not-created");
    let ctx = Ctx {
        home: home.clone(),
        repository: gripsack_exec::Repository::direct(sandbox.path().to_path_buf()),
        only: vec![],
        host: HostName::parse("model").unwrap(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        fetch: Default::default(),
        home_dir: Default::default(),
    };
    assert!(matches!(apply(&ir, &ctx), Err(ExecError::Gate(d))
        if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE));
    assert!(
        !home.exists(),
        "capability rejection must precede home mutation"
    );
    let preview = gripsack_exec::ops::preview_ops(
        &ir,
        &ctx.repository,
        None,
        &Default::default(),
        Default::default(),
        &[],
    );
    assert!(matches!(preview, Err(ExecError::Gate(d))
        if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE));
    assert!(!home.exists());
    assert!(
        !marker.exists(),
        "inert task must not run during apply or preview"
    );
}

#[test]
fn selecting_an_inert_task_for_apply_cannot_create_an_empty_generation() {
    let sandbox = tempfile::tempdir().unwrap();
    let marker = sandbox.path().join("task-must-not-run");
    let ir = check(
        &json!({
            "ir_version": gripsack_ir::IR_VERSION,
            "host":{"os":"linux","arch":"x86_64"},
            "workspace":{
                "span":{"file":"gripsack.ts","line":1},
                "outputs":[{"kind":"task","name":"touch","span":{"file":"gripsack.ts","line":3},
                    "context":{"kind":"host","mutable_paths":[]},
                    "steps":[{"command":{"kind":"exec","span":{"file":"gripsack.ts","line":4},
                        "argv":[{"kind":"literal","value":"/usr/bin/touch"},
                                {"kind":"literal","value":marker}]}}]}]
            }
        })
        .to_string(),
    )
    .expect("inert catalog admitted");
    let home = sandbox.path().join("not-created");
    let mut ctx = Ctx {
        home: home.clone(),
        repository: gripsack_exec::Repository::direct(sandbox.path().to_path_buf()),
        only: vec![],
        host: HostName::parse("model").unwrap(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        fetch: Default::default(),
        home_dir: Default::default(),
    };
    for selection in [vec![], vec!["touch".to_owned()]] {
        ctx.only = selection;
        assert!(matches!(apply(&ir, &ctx), Err(ExecError::Gate(d))
            if d.code == codes::WORKSPACE_EXEC_UNAVAILABLE));
        assert!(!home.exists());
        assert!(!marker.exists());
    }
    let preview = gripsack_exec::ops::preview_ops(
        &ir,
        &ctx.repository,
        None,
        &Default::default(),
        Default::default(),
        &ctx.only,
    );
    assert!(matches!(preview, Err(ExecError::Gate(_))));
    assert!(!marker.exists());
}
