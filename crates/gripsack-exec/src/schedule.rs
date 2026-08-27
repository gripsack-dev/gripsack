//! The ready-queue scheduler (0007 §5): a module becomes ready when
//! its dependencies have finished; up to N = cores run concurrently.
//! Named resources (0007 §4) serialize through flock files under
//! `$GRIPSACK_HOME/locks/` — in-process parallelism and two concurrent
//! `grip` runs both respect them. The generation flip stays the single
//! global barrier: it happens in apply, after everything finishes.

use crate::ctx::{Ctx, ExecError};
use crate::lockfile::{LockEntry, Lockfile};
use crate::module::{ModuleOutcome, run_module};
use crate::report::StepReport;
use gripsack_ir::{Ir, Module, Step};
use gripsack_store as store;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use std::sync::{Condvar, Mutex};

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
    ready: VecDeque<String>,
    running: BTreeSet<String>,
    error: Option<(String, ExecError, store::ModuleState)>,
    modules: BTreeMap<String, store::ModuleState>,
    reports: Vec<(String, Vec<StepReport>)>,
    lock_entries: BTreeMap<String, LockEntry>,
}

pub(crate) fn run_all(
    ir: &Ir,
    steps_by_module: &BTreeMap<String, Vec<Step>>,
    order: &[String],
    ctx: &Ctx,
    prev: &BTreeMap<String, store::ModuleState>,
    lock: &Lockfile,
) -> Result<ScheduleOutcome, ExecError> {
    // adjacency from module.depends; indegree counts unfinished deps
    let wanted: BTreeSet<&str> = order.iter().map(String::as_str).collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &wanted {
        let module = &ir.modules[*name];
        let mut deps = 0;
        for dep in &module.depends {
            if wanted.contains(dep.module.as_str()) {
                dependents
                    .entry(dep.module.as_str())
                    .or_default()
                    .push(name);
                deps += 1;
            }
        }
        indegree.insert(name, deps);
    }

    let state = Mutex::new(State {
        ready: indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| n.to_string())
            .collect(),
        running: BTreeSet::new(),
        error: None,
        modules: BTreeMap::new(),
        reports: Vec::new(),
        lock_entries: BTreeMap::new(),
    });
    let condvar = Condvar::new();
    let dependents = &dependents;
    let indegree = &Mutex::new(indegree);

    let workers = ctx.jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let next = {
                        let mut st = state.lock().expect("scheduler state");
                        loop {
                            if st.error.is_some() {
                                // stop picking new work; drain what's
                                // running — and WAIT like the success
                                // path does, or every idle worker
                                // busy-spins until the drain (N1)
                                if st.running.is_empty() {
                                    return;
                                }
                                st = condvar.wait(st).expect("scheduler state");
                                continue;
                            }
                            if let Some(name) = st.ready.pop_front() {
                                st.running.insert(name.clone());
                                break Some(name);
                            }
                            if st.running.is_empty() {
                                return; // queue drained, nothing in flight
                            }
                            st = condvar.wait(st).expect("scheduler state");
                        }
                    };
                    let Some(name) = next else {
                        continue;
                    };
                    let result = run_one(&name, ir, steps_by_module, ctx, prev, lock);
                    let mut st = state.lock().expect("scheduler state");
                    st.running.remove(&name);
                    match result {
                        Ok(outcome) if outcome.error.is_none() => {
                            st.reports.push((name.clone(), outcome.reports));
                            st.modules.insert(name.clone(), outcome.state);
                            if let Some(entry) = outcome.lock_entry {
                                st.lock_entries.insert(name.clone(), entry);
                            }
                            if let Some(deps) = dependents.get(name.as_str()) {
                                let mut indeg = indegree.lock().expect("scheduler indegree");
                                let mut st_ready = Vec::new();
                                for dependent in deps {
                                    let d =
                                        indeg.get_mut(dependent).expect("dependent in indegree");
                                    *d -= 1;
                                    if *d == 0 {
                                        st_ready.push(dependent.to_string());
                                    }
                                }
                                drop(indeg);
                                for ready in st_ready {
                                    st.ready.push_back(ready);
                                }
                            }
                        }
                        Ok(outcome) => {
                            // a phase failed mid-module: keep the
                            // partial state for the run-rollback, stop
                            // scheduling new work
                            if st.error.is_none() {
                                let e = outcome.error.expect("marked by the guard");
                                st.error = Some((name.clone(), e, outcome.state));
                            }
                            st.reports.push((name.clone(), outcome.reports));
                        }
                        Err(e) => {
                            if st.error.is_none() {
                                st.error = Some((
                                    name.clone(),
                                    e,
                                    store::ModuleState {
                                        store_path: PathBuf::new(),
                                        entries: vec![],
                                        env: vec![],
                                    },
                                ));
                            }
                        }
                    }
                    condvar.notify_all();
                }
            });
        }
    });

    let st = state.into_inner().expect("scheduler state");
    Ok(ScheduleOutcome {
        modules: st.modules,
        reports: st.reports,
        lock_entries: st.lock_entries,
        failed: st.error,
    })
}

/// One module — steps acquire their own resources (0007 §4, N4); the
/// scheduler only orders and fans out.
fn run_one(
    name: &str,
    ir: &Ir,
    steps_by_module: &BTreeMap<String, Vec<Step>>,
    ctx: &Ctx,
    prev: &BTreeMap<String, store::ModuleState>,
    lock: &Lockfile,
) -> Result<ModuleOutcome, ExecError> {
    let module: &Module = &ir.modules[name];
    let steps = &steps_by_module[name];
    run_module(
        name,
        module,
        ir,
        steps,
        ctx,
        prev.get(name),
        lock.modules.get(name),
    )
}
