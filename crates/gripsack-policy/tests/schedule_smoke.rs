// Runtime smoke: the kernel's ghost state must be inert under the
// plain-cargo build (specs erased) — a Diamond DAG runs to completion,
// the failure latch sticks.
#[test]
fn pure_scheduler_runs_a_diamond() {
    // a -> {b, c} -> d
    let deps = vec![vec![], vec![0], vec![0], vec![1, 2]];
    let mut k = gripsack_policy::schedule::PureScheduler::new(&deps);
    assert!(!k.failed());
    assert_eq!(k.start_next(), Some(0));
    assert_eq!(k.start_next(), None); // nothing else ready yet
    k.finish_ok(0);
    let mut seen = vec![k.start_next().unwrap(), k.start_next().unwrap()];
    seen.sort();
    assert_eq!(seen, vec![1, 2]);
    assert_eq!(k.start_next(), None);
    k.finish_ok(1);
    k.finish_ok(2);
    assert_eq!(k.start_next(), Some(3));
    k.finish_ok(3);
    assert_eq!(k.start_next(), None);
}

#[test]
fn pure_scheduler_latches_on_failure() {
    let deps = vec![vec![], vec![0]];
    let mut k = gripsack_policy::schedule::PureScheduler::new(&deps);
    assert_eq!(k.start_next(), Some(0));
    k.finish_fail(0);
    assert!(k.failed());
    assert_eq!(k.start_next(), None); // the dependent never starts
}
