//! The production worker/notification seam. The decision kernel remains
//! `PureScheduler`; Loom substitutes only synchronization, never transitions.

use gripsack_policy::schedule::PureScheduler;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[cfg(all(test, feature = "loom-tests"))]
use loom::sync::{Condvar, Mutex};
#[cfg(not(all(test, feature = "loom-tests")))]
use std::sync::{Condvar, Mutex};

/// One admitted worker result. A panic is a failure, not an abandoned start.
pub(super) enum Completion<T> {
    Success(T),
    Failed(T),
    Panicked,
}

struct State<T> {
    kernel: PureScheduler,
    running: usize,
    data: T,
}

pub(super) struct Coordinator<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Coordinator<T> {
    pub(super) fn new(dependencies: &[Vec<usize>], data: T) -> Self {
        Self {
            state: Mutex::new(State {
                kernel: PureScheduler::new(dependencies),
                running: 0,
                data,
            }),
            changed: Condvar::new(),
        }
    }

    /// Claim one module, execute outside the lock, then atomically publish
    /// its result and finish the kernel transition before waking consumers.
    /// `prepare` and `record` are internal in-memory bookkeeping; only
    /// `execute` may run user/provider code. Every execute panic is caught.
    pub(super) fn run_next<I, O>(
        &self,
        prepare: impl FnOnce(usize, &T) -> I,
        execute: impl FnOnce(usize, I) -> Completion<O>,
        record: impl FnOnce(usize, Completion<O>, &mut T),
    ) -> bool {
        let (index, input) = {
            let mut state = self.state.lock().expect("scheduler coordination");
            loop {
                if let Some(index) = state.kernel.start_next() {
                    // The kernel starts each admitted node at most once;
                    // concurrent starts cannot exceed the graph length.
                    state.running += 1;
                    break (index, prepare(index, &state.data));
                }
                if state.running == 0 {
                    return false;
                }
                state = self.changed.wait(state).expect("scheduler coordination");
            }
        };
        let completion = catch_unwind(AssertUnwindSafe(|| execute(index, input)))
            .unwrap_or(Completion::Panicked);
        let succeeded = matches!(&completion, Completion::Success(_));
        let mut state = self.state.lock().expect("scheduler coordination");
        record(index, completion, &mut state.data);
        state.running -= 1;
        if succeeded {
            state.kernel.finish_ok(index);
        } else {
            state.kernel.finish_fail(index);
        }
        // Success, ordinary failure and panic all release waiters. Holding
        // the same mutex across the predicate/transition prevents lost wakeups.
        self.changed.notify_all();
        true
    }

    pub(super) fn into_inner(self) -> T {
        self.state
            .into_inner()
            .expect("scheduler coordination")
            .data
    }
}

#[cfg(all(test, feature = "loom-tests"))]
#[path = "coordination_model.rs"]
mod model;
