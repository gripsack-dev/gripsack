//! The ready-queue scheduler (0007 §5): a module becomes ready when
//! its dependencies have finished; up to N = cores run concurrently.
//! The DECISIONS — what's ready, when a completion releases its
//! dependents, when failure latches — are the verified pure kernel
//! `gripsack_policy::schedule::PureScheduler` (0047 §3): a module
//! provably starts only after every dependency finished OK, at most
//! once, and never after any failure. The threads translate
//! names↔indices at the boundary; the mutex/condvar bridge stays
//! tested (journeys, e2e), not wrapped.
//! Named resources (0007 §4) serialize through flock files under
//! `$GRIPSACK_HOME/locks/` — in-process parallelism and two concurrent
//! `grip` runs both respect them. The generation flip stays the single
//! global barrier: it happens in apply, after everything finishes.
mod coordination;

use coordination::{Completion, Coordinator};

use crate::ctx::{Ctx, ExecError};
use crate::lockfile::{LockEntry, Lockfile};
use crate::module::{ModuleOutcome, run_module};
use crate::report::StepReport;
use gripsack_ir::{Ir, Module};
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use std::sync::Arc;

/// What the scheduler produced: the new manifest state, the reports
/// (grouped per module, completion-ordered), and lockfile entries.
pub(crate) struct ScheduleOutcome {
    pub modules: BTreeMap<String, store::ModuleState>,
    pub reports: Vec<(String, Vec<StepReport>)>,
    pub lock_entries: BTreeMap<String, LockEntry>,
    /// The first module failure (name, error, partial state) — apply
    /// uses the partial state for its run-rollback (0001 §9).
    pub failed: Option<(String, ExecError, store::ModuleState)>,
}

struct State {
    error: Option<(String, ExecError, store::ModuleState)>,
    modules: BTreeMap<String, store::ModuleState>,
    /// Ready consumers see dependency pins resolved in this run, not merely
    /// the lockfile that existed when apply began (0035 F4, 0039).
    lock: Arc<Lockfile>,
    reports: Vec<(String, Vec<StepReport>)>,
    lock_entries: BTreeMap<String, LockEntry>,
}

pub(crate) fn run_all(
    ir: &Ir,
    steps_by_module: &BTreeMap<String, gripsack_ir::prepared::PreparedModule>,
    order: &[String],
    ctx: &Ctx,
    prev: &BTreeMap<String, store::ModuleState>,
    lock: &Lockfile,
) -> Result<ScheduleOutcome, ExecError> {
    let recipes = crate::resolve::RecipeGraph::new(
        ir,
        &ctx.repo,
        steps_by_module,
        order.iter().map(String::as_str),
    )?;
    let lineage = previous_ownership(prev);
    let run_span = tracing::Span::current();
    let dispatcher = tracing::dispatcher::get_default(Clone::clone);
    // The kernel's input view (0047 §3): name-sorted indices (so the
    // FIFO order matches the pre-kernel name-sorted readiness), edges
    // restricted to this run's wanted set and deduplicated — the
    // kernel's admission requires in-range, duplicate-free edges.
    let wanted: BTreeSet<&str> = order.iter().map(String::as_str).collect();
    let names: Vec<&str> = wanted.iter().copied().collect();
    let index_of: BTreeMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let deps: Vec<Vec<usize>> = names
        .iter()
        .map(|name| {
            let module = &ir.modules[*name];
            gripsack_ir::dependencies::ordering_dependencies(name, module)
                .iter()
                .filter_map(|dep| index_of.get(dep).copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .collect();

    let build_only = gripsack_ir::dependencies::build_only_modules(&ir.modules);
    let closures: BTreeMap<&str, Vec<&str>> = wanted
        .iter()
        .map(|name| {
            let reachable = gripsack_ir::dependencies::build_closure_names(&ir.modules, name);
            let closure = order
                .iter()
                .map(String::as_str)
                .filter(|dep| reachable.contains(dep))
                .collect();
            (*name, closure)
        })
        .collect();

    let coordinator = Coordinator::new(
        &deps,
        State {
            error: None,
            modules: BTreeMap::new(),
            lock: Arc::new(lock.clone()),
            reports: Vec::new(),
            lock_entries: BTreeMap::new(),
        },
    );
    let names = &names;

    let workers = ctx.jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let _dispatch = tracing::dispatcher::set_default(&dispatcher);
                let _run = run_span.enter();
                while coordinator.run_next(
                    |index, state| {
                        let name = names[index];
                        (
                            crate::closure::BuildEnv::compose(&closures[name], &state.modules),
                            Arc::clone(&state.lock),
                        )
                    },
                    |index, (build_env, current_lock)| {
                        let name = names[index];
                        let _module_span = tracing::info_span!("module", module = %name).entered();
                        #[cfg(debug_assertions)]
                        if std::env::var("GRIPSACK_PANIC_MODULE").as_deref() == Ok(name) {
                            panic!("injected module worker panic");
                        }
                        let result = build_env.and_then(|build_env| {
                            run_one(
                                name,
                                ir,
                                steps_by_module,
                                ctx,
                                prev,
                                &current_lock,
                                Scheduled {
                                    build_env,
                                    build_only: build_only.contains(name),
                                    recipes: &recipes,
                                    lineage: &lineage,
                                },
                            )
                        });
                        if result.as_ref().is_ok_and(|outcome| outcome.error.is_none()) {
                            Completion::Success(result)
                        } else {
                            Completion::Failed(result)
                        }
                    },
                    |index, completion, state| {
                        let name = names[index];
                        let result = match completion {
                            Completion::Success(result) | Completion::Failed(result) => result,
                            Completion::Panicked => Err(ExecError::WorkerPanicked {
                                module: name.to_owned(),
                            }),
                        };
                        match result {
                            Ok(outcome) if outcome.error.is_none() => {
                                state.reports.push((name.to_owned(), outcome.reports));
                                state.modules.insert(name.to_owned(), outcome.state);
                                if let Some(entry) = outcome.lock_entry {
                                    if state.lock.modules.get(name) != Some(&entry) {
                                        Arc::make_mut(&mut state.lock)
                                            .modules
                                            .insert(name.to_owned(), entry.clone());
                                        recipes.invalidate(name);
                                    }
                                    state.lock_entries.insert(name.to_owned(), entry);
                                }
                            }
                            Ok(outcome) => {
                                // Keep partial deployment state; apply compensates
                                // through the same durable journal as crash recovery.
                                if state.error.is_none() {
                                    state.error = Some((
                                        name.to_owned(),
                                        outcome.error.expect("failed module outcome"),
                                        outcome.state,
                                    ));
                                }
                                state.reports.push((name.to_owned(), outcome.reports));
                            }
                            Err(error) => {
                                if state.error.is_none() {
                                    state.error = Some((
                                        name.to_owned(),
                                        error,
                                        store::ModuleState {
                                            store_path: PathBuf::new(),
                                            build_only: false,
                                            intents: vec![],
                                            verified: None,
                                            entries: vec![],
                                            env: vec![],
                                            tree256: None,
                                            build_closure: vec![],
                                        },
                                    ));
                                }
                            }
                        }
                    },
                ) {}
            });
        }
    });

    let st = coordinator.into_inner();
    Ok(ScheduleOutcome {
        modules: st.modules,
        reports: st.reports,
        lock_entries: st.lock_entries,
        failed: st.error,
    })
}

/// One module — steps acquire their own resources (0007 §4, N4); the
/// scheduler only orders and fans out.
/// Lineage is destination-GLOBAL (0030 §H4): the previous generation's
/// entry for each canonical destination, whichever module owned it —
/// a rename keeps full authority (update, not preserve-as-foreign).
pub(crate) fn previous_ownership(
    prev: &BTreeMap<String, store::ModuleState>,
) -> BTreeMap<store::OwnershipKey, &store::DeployedEntry> {
    let mut map = BTreeMap::new();
    for (name, state) in prev {
        for entry in &state.entries {
            map.insert(entry.ownership_key(name), entry);
        }
    }
    map
}

/// What the scheduler adds to one module run beyond the static graph
/// inputs (0039): the composed build closure and the build-only
/// verdict. A struct, not two more positional arguments.
struct Scheduled<'a> {
    build_env: crate::closure::BuildEnv,
    build_only: bool,
    recipes: &'a crate::resolve::RecipeGraph,
    lineage: &'a BTreeMap<store::OwnershipKey, &'a store::DeployedEntry>,
}

fn run_one(
    name: &str,
    ir: &Ir,
    steps_by_module: &BTreeMap<String, gripsack_ir::prepared::PreparedModule>,
    ctx: &Ctx,
    prev: &BTreeMap<String, store::ModuleState>,
    lock: &Lockfile,
    scheduled: Scheduled<'_>,
) -> Result<ModuleOutcome, ExecError> {
    let module: &Module = &ir.modules[name];
    let plan = &steps_by_module[name];
    run_module(
        crate::module::ModuleInputs {
            name,
            module,
            recipes: scheduled.recipes,
            plan,
            prev_map: scheduled.lineage,
            prev_module: prev.get(name),
            locked: lock.modules.get(name),
            lock,
            build_env: scheduled.build_env,
            build_only: scheduled.build_only,
        },
        ctx,
    )
}
