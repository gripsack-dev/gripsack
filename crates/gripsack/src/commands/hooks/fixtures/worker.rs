use super::{FixtureCase, FixtureRoot, SimulationMode, invalid, store};
use gripsack_policy::activation::IntentState;
use gripsack_process::{ByteBinding, Enforcement, Sha256Digest};
use serde::Serialize;
use std::io;

#[derive(Serialize)]
pub(super) struct ObservedIntent {
    case: FixtureCase,
    pub(super) intent: store::activation::IntentId,
    activation: store::activation::ActivationId,
    pub(super) attempt: u64,
    pub(super) succeeded: bool,
    pub(super) ambiguous: bool,
    pub(super) pending: bool,
    byte_binding: Option<ByteBinding>,
    enforcement: Option<Enforcement>,
}

pub(super) fn run(root: FixtureRoot, case: FixtureCase, mode: SimulationMode) -> io::Result<()> {
    // This internal worker is dispatched before tracing/thread creation. All
    // action text below is generated here; no user hooks are accepted as input.
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let home_path = root.home(case);
    let session = gripsack_exec::LifecycleSession::acquire(&home_path)?;
    let home = gripsack_fs::open(&home_path)?;
    if store::journal::pending_recovery(&home)?.is_some() {
        return Err(invalid(
            "fixture workers never reconcile arbitrary destination journals",
        ));
    }
    let action = root.action(case)?;
    let expected = Sha256Digest::of(&serde_json::to_vec(&action).map_err(io::Error::other)?);
    match store::current_generation(&home_path)? {
        None => {
            if store::activation::has_pending(&home)? {
                return Err(invalid(
                    "fixture has activation without its generated selection",
                ));
            }
            let generation = store::GenerationId::new(1);
            let pending = store::journal::begin_run(
                &home,
                &home_path,
                None,
                generation,
                store::journal::RunOp::Apply,
            )?;
            store::write_manifest(
                &home,
                &store::Generation {
                    number: generation,
                    modules: Default::default(),
                },
            )?;
            let _prepared = store::activation::prepare(
                &home,
                &pending,
                vec![store::activation::PendingIntent {
                    module: "fixture".into(),
                    action,
                    trigger: gripsack_ir::Trigger::PostActivate,
                }],
            )?;
            let committed = store::flip(pending)?;
            store::journal::commit_run(committed)?;
        }
        Some(generation) if generation == store::GenerationId::new(1) => {}
        Some(_) => return Err(invalid("fixture worker cannot select another generation")),
    }
    let mut count = 0;
    store::activation::inspect(&home, |row| {
        count += 1;
        if row.action_sha256 != expected
            || row.contributors.len() != 1
            || row.contributors[0].module != "fixture"
            || row.contributors[0].trigger != gripsack_ir::Trigger::PostActivate
        {
            return Err(invalid("fixture worker refuses a non-fixture action"));
        }
        Ok(())
    })?;
    if count != 1 {
        return Err(invalid("fixture worker requires its one generated intent"));
    }
    // SAFETY: this dedicated first-party worker has not started any threads.
    // The variable selects only an existing production crash seam.
    unsafe {
        match mode {
            SimulationMode::Clean => std::env::remove_var("GRIPSACK_CRASH_AFTER"),
            SimulationMode::Duplicate => {
                std::env::set_var("GRIPSACK_CRASH_AFTER", "hook-after-effect")
            }
            SimulationMode::CrashAfterStart => {
                std::env::set_var("GRIPSACK_CRASH_AFTER", "hook-after-start")
            }
        }
    }
    gripsack_exec::activate::resume_activation(&session)?;
    if !observe(&root, case)?.succeeded {
        return Err(invalid("fixture hook did not succeed"));
    }
    Ok(())
}

pub(super) fn observe(root: &FixtureRoot, case: FixtureCase) -> io::Result<ObservedIntent> {
    let home = gripsack_fs::open(&root.home(case))?;
    let mut observed = None;
    store::activation::inspect(&home, |row| {
        if observed.is_some() {
            return Err(invalid("fixture produced more than its one intent"));
        }
        let store::activation::HookState::Current(state) = row.state else {
            return Err(invalid("fixture lost its transaction-bound identity"));
        };
        observed = Some(ObservedIntent {
            case,
            intent: row
                .intent
                .ok_or_else(|| invalid("fixture intent has no identity"))?,
            activation: row
                .activation
                .ok_or_else(|| invalid("fixture activation has no identity"))?,
            attempt: state
                .attempt()
                .ok_or_else(|| invalid("fixture has no started attempt"))?
                .value(),
            succeeded: matches!(state, IntentState::Succeeded { .. }),
            ambiguous: matches!(state, IntentState::Started { .. }),
            pending: row.pending,
            byte_binding: row
                .processes
                .first()
                .map(|process| process.byte_binding.clone()),
            enforcement: row
                .processes
                .first()
                .map(|process| process.enforcement.clone()),
        });
        Ok(())
    })?;
    observed.ok_or_else(|| invalid("fixture has no recorded intent"))
}
