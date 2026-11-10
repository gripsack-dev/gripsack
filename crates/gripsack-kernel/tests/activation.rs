//! The activation-machine explorer (plan/0056 F2 step 5): every batch of up
//! to three intents over a small record alphabet, every `current`, every hook
//! result, a launch-time drift and a crash after every command, driven
//! through the shipped `activation::activation_step` by a simulated host with
//! a durable ledger.
//!
//! The per-step guarantees are Lean theorems (proofs/Kernel/Activation.lean).
//! What this adds is the whole-run view against an oracle written without
//! the kernel: which hooks run, with which attempts, what the ledger holds
//! afterwards, and — across a crash and a replay — that a terminal intent
//! never runs again, a failed one is never retried and a superseded one runs
//! zero times.
use gripsack_kernel::GenerationId;
use gripsack_kernel::activation::{
    ActivationState, AttemptNumber, BatchFacts, Command, Event, HookResult, IntentState, Outcome,
    Refusal, activation_step,
};
use gripsack_kernel::recover::Failure;
use gripsack_kernel::selection::{SelectionIdentity, TransactionId};

fn attempt(n: u64) -> AttemptNumber {
    AttemptNumber::admit(n).unwrap()
}

fn selection(tx: u8) -> SelectionIdentity {
    SelectionIdentity::transaction(GenerationId::new(2), TransactionId::from_bytes([tx; 32]))
}

fn records() -> Vec<IntentState> {
    vec![
        IntentState::Pending,
        IntentState::Started {
            attempt: attempt(1),
        },
        IntentState::Started {
            attempt: attempt(u64::MAX),
        },
        IntentState::Succeeded {
            attempt: attempt(1),
        },
        IntentState::Failed {
            attempt: attempt(2),
        },
        IntentState::Superseded { last_attempt: None },
    ]
}

fn terminal(state: &IntentState) -> bool {
    matches!(
        state,
        IntentState::Succeeded { .. } | IntentState::Failed { .. } | IntentState::Superseded { .. }
    )
}

/// The simulated host: the durable ledger, the pointer, `current`, and what
/// ran.
#[derive(Clone)]
struct Host {
    ledger: Vec<IntentState>,
    pending: bool,
    archived: bool,
    current: Option<SelectionIdentity>,
    batch: SelectionIdentity,
    results: Vec<HookResult>,
    /// Launch number (0-based, across the run) at which `current` moves.
    drift_at: Option<usize>,
    /// Stop (a crash) after this many commands.
    crash_after: Option<usize>,
    /// Every launch: (intent, attempt).
    launched: Vec<(u32, u64)>,
}

enum End {
    Finished(Outcome),
    Aborted(Refusal),
    Crashed,
}

impl Host {
    fn run(&mut self) -> End {
        let mut state = ActivationState::new();
        let mut event = Event::Start;
        let mut issued = 0;
        let mut launches = 0;
        loop {
            let (next, command) = activation_step(state, event);
            state = next;
            if self.crash_after == Some(issued) {
                return End::Crashed;
            }
            issued += 1;
            event = match command {
                Command::ReadBatch => {
                    if self.pending {
                        Event::BatchRead(BatchFacts {
                            selection: self.batch,
                            intents: self.ledger.len() as u32,
                        })
                    } else {
                        Event::NoBatch
                    }
                }
                Command::ReadCurrent => Event::CurrentRead(self.current),
                Command::Authorize(sel) => {
                    assert_eq!(
                        Some(sel),
                        self.current,
                        "authorized a selection not current"
                    );
                    Event::Done
                }
                Command::ReadIntent(k) => Event::IntentRead(self.ledger[k as usize]),
                Command::Persist { intent, state } => {
                    let prior = self.ledger[intent as usize];
                    assert!(!terminal(&prior), "terminal record {prior:?} rewritten");
                    self.ledger[intent as usize] = state;
                    Event::Done
                }
                Command::RunHook {
                    intent,
                    attempt,
                    expect,
                } => {
                    assert_eq!(
                        self.ledger[intent as usize],
                        IntentState::Started { attempt },
                        "hook launched without its durable Started record"
                    );
                    if self.drift_at == Some(launches) {
                        self.current = Some(selection(9));
                    }
                    launches += 1;
                    if self.current != Some(expect) {
                        Event::Failed(Failure::Drift)
                    } else {
                        self.launched.push((intent, attempt.value()));
                        Event::Returned(self.results[intent as usize])
                    }
                }
                Command::Archive => {
                    assert!(
                        self.ledger.iter().all(terminal),
                        "archived unsettled intents"
                    );
                    self.archived = true;
                    Event::Done
                }
                Command::Clear => {
                    assert!(self.archived, "pointer cleared before the archive");
                    self.pending = false;
                    Event::Done
                }
                Command::Finish(outcome) => return End::Finished(outcome),
                Command::Abort(refusal) => return End::Aborted(refusal),
            };
        }
    }
}

/// Every batch of `0..=3` intents over the record alphabet.
fn ledgers() -> Vec<Vec<IntentState>> {
    let mut all = vec![Vec::new()];
    let mut frontier = vec![Vec::new()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for ledger in &frontier {
            for record in records() {
                let mut grown: Vec<IntentState> = ledger.clone();
                grown.push(record);
                next.push(grown);
            }
        }
        all.extend(next.iter().cloned());
        frontier = next;
    }
    all
}

fn results(n: usize) -> Vec<Vec<HookResult>> {
    let mut all = vec![Vec::new()];
    for _ in 0..n {
        all = all
            .into_iter()
            .flat_map(|prefix: Vec<HookResult>| {
                [HookResult::Succeeded, HookResult::Failed].map(|r| {
                    let mut grown = prefix.clone();
                    grown.push(r);
                    grown
                })
            })
            .collect();
    }
    all
}

/// The oracle for one uninterrupted run with no drift.
fn expected(host: &Host) -> (Vec<IntentState>, Vec<(u32, u64)>, Option<usize>) {
    let bound = host.current == Some(host.batch);
    let mut ledger = host.ledger.clone();
    let mut launched = Vec::new();
    for (k, record) in host.ledger.iter().enumerate() {
        if !bound {
            ledger[k] = match record {
                IntentState::Pending => IntentState::Superseded { last_attempt: None },
                IntentState::Started { attempt } => IntentState::Superseded {
                    last_attempt: Some(*attempt),
                },
                other => *other,
            };
            continue;
        }
        let next = match record {
            IntentState::Pending => 1,
            IntentState::Started { attempt } if attempt.value() == u64::MAX => {
                return (ledger, launched, Some(k));
            }
            IntentState::Started { attempt } => attempt.value() + 1,
            _ => continue,
        };
        launched.push((k as u32, next));
        ledger[k] = match host.results[k] {
            HookResult::Succeeded => IntentState::Succeeded {
                attempt: attempt(next),
            },
            HookResult::Failed => IntentState::Failed {
                attempt: attempt(next),
            },
        };
    }
    (ledger, launched, None)
}

fn hosts() -> Vec<Host> {
    let mut all = Vec::new();
    for ledger in ledgers() {
        for current in [Some(selection(1)), Some(selection(2)), None] {
            for results in results(ledger.len()) {
                all.push(Host {
                    ledger: ledger.clone(),
                    pending: true,
                    archived: false,
                    current,
                    batch: selection(1),
                    results,
                    drift_at: None,
                    crash_after: None,
                    launched: Vec::new(),
                });
            }
        }
    }
    all
}

#[test]
fn every_uninterrupted_run_matches_the_oracle() {
    let mut runs = 0;
    for start in hosts() {
        let mut host = start.clone();
        let end = host.run();
        let (ledger, launched, exhausted) = expected(&start);
        assert_eq!(host.launched, launched, "launches of {:?}", start.ledger);
        match exhausted {
            Some(k) => {
                assert!(
                    matches!(end, End::Aborted(Refusal::Exhausted { intent }) if intent as usize == k)
                );
                assert!(host.pending, "an exhausted batch keeps its evidence");
                assert_eq!(&host.ledger[..k], &ledger[..k]);
                assert_eq!(&host.ledger[k..], &start.ledger[k..]);
            }
            None => {
                let outcome = if start.current == Some(start.batch) {
                    Outcome::Activated
                } else {
                    Outcome::Superseded
                };
                assert!(matches!(end, End::Finished(o) if o == outcome));
                assert_eq!(host.ledger, ledger);
                assert!(host.archived && !host.pending);
            }
        }
        runs += 1;
    }
    assert!(runs > 2000, "explored {runs} runs");
}

#[test]
fn no_pending_batch_does_nothing() {
    let mut host = Host {
        ledger: Vec::new(),
        pending: false,
        archived: false,
        current: Some(selection(1)),
        batch: selection(1),
        results: Vec::new(),
        drift_at: None,
        crash_after: None,
        launched: Vec::new(),
    };
    assert!(matches!(host.run(), End::Finished(Outcome::Nothing)));
}

#[test]
fn a_launch_after_current_moved_stops_every_later_hook() {
    for start in hosts() {
        if start.current != Some(start.batch) {
            continue;
        }
        for drift in 0..3 {
            let mut host = Host {
                drift_at: Some(drift),
                ..start.clone()
            };
            let end = host.run();
            let (_, launched, _) = expected(&start);
            if drift >= launched.len() {
                continue;
            }
            assert!(matches!(end, End::Aborted(Refusal::Drifted { .. })));
            assert_eq!(
                host.launched,
                launched[..drift],
                "a hook ran after the drift"
            );
            assert!(host.pending && !host.archived);
            // The replay sees another selection: the rest is superseded, and
            // nothing more runs.
            host.drift_at = None;
            let before = host.launched.len();
            assert!(matches!(host.run(), End::Finished(Outcome::Superseded)));
            assert_eq!(host.launched.len(), before);
            assert!(host.ledger.iter().all(terminal));
        }
    }
}

/// Crash after every command, then replay to the end: each intent's hook ran
/// in at most one attempt per run it was durably started in, never after a
/// terminal record, and the replay's attempts continue the interrupted one.
#[test]
fn crash_then_replay_never_reruns_a_terminal_intent() {
    let mut runs = 0;
    for start in hosts() {
        if start.ledger.iter().any(|r| {
            *r == (IntentState::Started {
                attempt: attempt(u64::MAX),
            })
        }) {
            continue;
        }
        for cut in 0..40 {
            let mut host = Host {
                crash_after: Some(cut),
                ..start.clone()
            };
            if !matches!(host.run(), End::Crashed) {
                break;
            }
            let after_crash = host.ledger.clone();
            let first = host.launched.clone();
            host.crash_after = None;
            let end = host.run();
            assert!(matches!(end, End::Finished(_)), "the replay finishes");
            let second = &host.launched[first.len()..];
            for (k, a) in second {
                let before = after_crash[*k as usize];
                assert!(!terminal(&before), "terminal intent {k} re-ran");
                match before {
                    IntentState::Pending => assert_eq!(*a, 1),
                    IntentState::Started { attempt } => assert_eq!(*a, attempt.value() + 1),
                    _ => unreachable!(),
                }
            }
            for (k, _) in &first {
                // a hook that ran and was recorded terminal before the crash
                // never runs again
                if terminal(&after_crash[*k as usize]) {
                    assert!(second.iter().all(|(j, _)| j != k), "intent {k} retried");
                }
            }
            for (k, record) in start.ledger.iter().enumerate() {
                if terminal(record) {
                    assert!(host.launched.iter().all(|(j, _)| *j as usize != k));
                    assert_eq!(host.ledger[k], *record, "terminal record changed");
                }
            }
            runs += 1;
        }
    }
    assert!(runs > 10_000, "explored {runs} crash cuts");
}
