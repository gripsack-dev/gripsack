//! Apply and preview never invoke inert catalog tasks or create empty generations.

use gripsack_exec::{Ctx, ExecError, apply};
use gripsack_ir::{HostName, check, codes};
use serde_json::json;

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
