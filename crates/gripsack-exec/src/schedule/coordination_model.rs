//! Bounded systematic concurrency checks, not an unbounded proof.
//! Two workers, up to four nodes and two preemptions. Production code
//! supplies claim/wait/execute/catch/finish/notify; only primitives differ.

use super::{Completion, Coordinator};
use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResultKind {
    Success,
    Failed,
    Panicked,
}

fn explore(dependencies: Vec<Vec<usize>>, first_result: ResultKind) {
    let mut model = loom::model::Builder::new();
    model.preemption_bound = Some(2);
    model.max_branches = 2_000;
    model.check(move || {
        let coordinator = Arc::new(Coordinator::new(
            &dependencies,
            vec![None; dependencies.len()],
        ));
        let starts = Arc::new(
            (0..dependencies.len())
                .map(|_| AtomicUsize::new(0))
                .collect::<Vec<_>>(),
        );
        let mut workers = Vec::new();
        for _ in 0..2 {
            let shared = Arc::clone(&coordinator);
            let starts = Arc::clone(&starts);
            let dependencies = dependencies.clone();
            workers.push(loom::thread::spawn(move || {
                while shared.run_next(
                    |index, completed| {
                        for dependency in &dependencies[index] {
                            assert_eq!(completed[*dependency], Some(ResultKind::Success));
                        }
                    },
                    |index, ()| {
                        assert_eq!(starts[index].fetch_add(1, Ordering::SeqCst), 0);
                        loom::thread::yield_now();
                        if index == 0 {
                            match first_result {
                                ResultKind::Success => Completion::Success(()),
                                ResultKind::Failed => Completion::Failed(()),
                                ResultKind::Panicked => panic!("injected worker panic"),
                            }
                        } else {
                            Completion::Success(())
                        }
                    },
                    |index, result, completed| {
                        assert!(completed[index].is_none(), "duplicate worker completion");
                        completed[index] = Some(match result {
                            Completion::Success(()) => ResultKind::Success,
                            Completion::Failed(()) => ResultKind::Failed,
                            Completion::Panicked => ResultKind::Panicked,
                        });
                    },
                ) {}
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let coordinator = Arc::try_unwrap(coordinator).ok().unwrap();
        let completed = coordinator.into_inner();
        for (index, completion) in completed.iter().enumerate() {
            assert_eq!(
                starts[index].load(Ordering::SeqCst),
                usize::from(completion.is_some())
            );
        }
        assert_eq!(completed[0], Some(first_result));
        if first_result == ResultKind::Success {
            assert!(
                completed
                    .iter()
                    .all(|result| *result == Some(ResultKind::Success))
            );
        } else {
            assert_eq!(completed[1], None, "failure authorized its dependent");
        }
    });
}

#[test]
fn diamond_completion_wakes_waiters_and_publishes_dependencies() {
    explore(
        vec![vec![], vec![0], vec![0], vec![1, 2]],
        ResultKind::Success,
    );
}

#[test]
fn failure_latches_and_drains_without_lost_notification() {
    explore(vec![vec![], vec![0], vec![]], ResultKind::Failed);
}

#[test]
fn panic_completes_once_and_wakes_waiters() {
    explore(vec![vec![], vec![0], vec![]], ResultKind::Panicked);
}
