//! The exhaustive state-machine model of the transaction protocol
//! (plan/0028 — read that first; it is the teaching document).
//!
//! # What this file is
//!
//! The journal/flip/recovery protocol as an ABSTRACT state machine —
//! a few integers and options instead of files — plus an explorer
//! that walks every reachable state: every step order, every crash
//! point, both crash kinds, every durability outcome a power loss
//! permits, with and without a post-crash user edit. The transaction
//! oracle is asserted in every terminal state.
//!
//! # What makes the check honest
//!
//! The two decision points where every historical bug lived — *is
//! this interrupted run committed?* (`classify`) and *restore this
//! destination or keep it?* (`decide_from`) — are the REAL shipped
//! functions, extracted from `journal.rs` as pure functions that both
//! the production reconcile path and this model call. The explorer
//! explores; the decisions are the product's.
//!
//! What stays modeled rather than shared: filesystem mechanics
//! (atomic write, fsync, rename). Those are covered against the real
//! binary by the kill-point e2e. The boundary is deliberate:
//! protocol logic is proved here; mechanics are tested there.
//!
//! # The durability model (the subtle part)
//!
//! A process kill and a power loss are different events:
//!
//! - **kill**: every issued write persists (the kernel's page cache
//!   outlives the process) — the crash exposes `volatile`.
//! - **power loss**: only fsync'd data is guaranteed; writes since
//!   the last barrier persist in ANY subset (the disk reorders
//!   freely). The explorer enumerates every subset.
//!
//! Each step writes to `volatile`; barrier points (the fsyncs that
//! end `atomic_write`, the flip, and each cleanup phase) flush
//! `volatile` into `durable`. A step may also crash MID-WAY: the
//! effect reaches `volatile` but the flush never runs — which is why
//! cleanup needs two barriers (0.23) and why this model can prove
//! they suffice.
//!
//! # The oracle (plan/0020's sentence, machine-checked)
//!
//! After any crash and any recovery, every managed destination is the
//! previous generation's content, the committed target's content, or
//! a post-crash user edit that is KEPT — never an unexplained
//! mixture — and the journal drains completely. When `current`
//! matches neither the marker's previous nor its target, recovery
//! changes NOTHING and keeps the journal intact (fail closed).

use super::{Classification, Recovery, RecoveryFacts, RunOp, classify, decide_from};

/// Abstract contents. One destination is enough: the protocol
/// journals destinations independently, so a multi-destination run
/// is this machine iterated; the shared state (marker, flip,
/// cleanup) is what we model.
const PRIOR: u8 = 0; // the content deployed before this run
const DEPLOYED: u8 = 1; // the content this run deploys
const USER_EDIT: u8 = 2; // a post-crash user edit

/// The abstract filesystem.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Disk {
    /// None = the destination is absent (removal runs exercise this).
    dest: Option<u8>,
    current: Option<u64>,
    /// One journal entry: (prior content, intended content). The
    /// intended of a removal run is the REMOVED sentinel.
    entry: Option<(Option<u8>, &'static str)>,
    /// The run marker: (previous, target). Legacy (pre-0.23) markers
    /// carry `None` for previous and classify by the 0.22 direction
    /// rule — kept so the counterexample test can express them.
    marker: Option<(Option<u64>, u64)>,
}

/// Which transaction the model runs. Deploy covers apply and rollback
/// (identical step shapes; only prev/target differ). Prune is the
/// removal run: the destination exists and the intent is REMOVED.
#[derive(Clone, Copy)]
enum RunKind {
    Deploy,
    Prune,
}

/// The step sequence, with `[B]` marking a durability barrier:
///
/// ```text
/// 0. begin_run(prev, target)   [B]
/// 1. record(prior, intended)   [B]   <- durable BEFORE the mutation
/// 2. mutate                    [B]
/// 3. flip: current := target   [B]   <- the commit point
/// 4. cleanup: entry gone       [B]   <- barrier 1 (0023)
/// 5. cleanup: marker gone      [B]   <- barrier 2
/// ```
const STEPS: usize = 6;

/// A node in the exploration: the two filesystem copies plus where
/// the run stands.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Node {
    durable: Disk,
    volatile: Disk,
    step: usize,
    crashed: bool,
    user_edited: bool,
    /// Write 0.22-shaped markers (no previous generation, op-driven
    /// classification) — the counterexample test's knob.
    legacy: bool,
}

impl Node {
    fn initial(prev_gen: u64, kind: RunKind, legacy: bool) -> Node {
        let dest = match kind {
            RunKind::Deploy => Some(PRIOR),
            RunKind::Prune => Some(DEPLOYED),
        };
        let disk = Disk {
            dest,
            current: Some(prev_gen),
            entry: None,
            marker: None,
        };
        Node {
            durable: disk,
            volatile: disk,
            step: 0,
            crashed: false,
            user_edited: false,
            legacy,
        }
    }

    /// Barrier: everything written becomes durable.
    fn flush(&mut self) {
        self.durable = self.volatile;
    }
}

/// Execute step `i`'s effect on the volatile copy (no flush).
fn step_effect(disk: &mut Disk, i: usize, prev: u64, target: u64, kind: RunKind, legacy: bool) {
    match i {
        0 => disk.marker = Some((if legacy { None } else { Some(prev) }, target)),
        1 => {
            disk.entry = Some(match kind {
                RunKind::Deploy => (disk.dest, "1"),
                RunKind::Prune => (disk.dest, super::REMOVED),
            })
        }
        2 => match kind {
            RunKind::Deploy => disk.dest = Some(DEPLOYED),
            RunKind::Prune => disk.dest = None,
        },
        3 => disk.current = Some(target),
        4 => disk.entry = None,
        5 => disk.marker = None,
        _ => unreachable!(),
    }
}

/// A crash exposes a post-crash disk. Kill: every issued write
/// persists. Power loss: the durable state plus ANY subset of the
/// pending volatile changes (the disk reorders freely).
fn crash_disks(node: &Node) -> Vec<(Disk, &'static str)> {
    let mut out = vec![(node.volatile, "kill")];
    // power loss: per field that differs between durable and volatile,
    // choose either — every subset of pending writes
    let d = node.durable;
    let v = node.volatile;
    for dest in [d.dest, v.dest] {
        for current in [d.current, v.current] {
            for entry in [d.entry, v.entry] {
                for marker in [d.marker, v.marker] {
                    let disk = Disk {
                        dest,
                        current,
                        entry,
                        marker,
                    };
                    out.push((disk, "power loss"));
                }
            }
        }
    }
    out
}

/// Resolve a crashed node into terminal nodes: optionally the user
/// edits the destination, then reconcile runs the REAL decision
/// functions and the oracle is checked by the caller.
fn recover(
    mut disk: Disk,
    run_prev: u64,
    run_target: u64,
    classifier: &dyn Fn(&RecoveryFacts) -> Classification,
) -> (Disk, Option<Classification>) {
    // an empty journal — never started, or cleanup finished — means
    // NO recovery at all (the real reconcile returns before reading
    // the marker). The classification is then moot: the oracle checks
    // only that nothing changed.
    if disk.marker.is_none() && disk.entry.is_none() {
        return (disk, None);
    }
    let (prev, target) = disk.marker.unwrap_or((Some(run_prev), run_target));
    let class = match disk.marker {
        Some(_) => Some(classifier(&RecoveryFacts {
            previous: prev,
            target,
            current: disk.current,
            format: if prev.is_some() { 2 } else { 0 },
        })),
        // entries without a marker: the real rule — uncommitted
        None => Some(Classification::Uncommitted),
    };
    let class = match class {
        Some(class) => class,
        None => unreachable!("handled above"),
    };
    match class {
        Classification::Committed => {
            // the flip landed: content stands, cleanup only
            disk.entry = None;
            disk.marker = None;
        }
        Classification::Uncommitted => {
            if let Some((prior, intended)) = disk.entry {
                let live = disk.dest.map(|c| {
                    match c {
                        PRIOR => "0",
                        DEPLOYED => "1",
                        _ => "2", // USER_EDIT
                    }
                });
                let prior_id = prior.map(|c| if c == PRIOR { "0" } else { "1" });
                match decide_from(
                    live,
                    intended,
                    prior_id,
                    // the message placeholder; the decision never
                    // reads it
                    &super::PriorSerde::Absent,
                ) {
                    Recovery::Restore(_) => disk.dest = prior,
                    Recovery::Unchanged | Recovery::Keep(_) => {}
                }
            }
            disk.entry = None;
            disk.marker = None;
        }
        // fail closed: nothing changes, journal intact — Legacy
        // markers refuse the same way (0030 §11)
        Classification::Ambiguous | Classification::Legacy => {}
    }
    (disk, Some(class))
}

/// The oracle. Returns the violation as a readable string.
fn check_oracle(
    crashed_disk: &Disk,
    disk: &Disk,
    class: Option<Classification>,
    kind: RunKind,
    user_edited: bool,
    prev: u64,
    target: u64,
) -> Result<(), String> {
    let bad = |why: &str| -> String {
        format!("{why}\n  crashed at: {crashed_disk:?}\n  after recovery: {disk:?}")
    };
    let Some(class) = class else {
        // the journal was empty: recovery must have changed nothing
        return if disk == crashed_disk {
            Ok(())
        } else {
            Err(bad("an empty journal means recovery is a no-op"))
        };
    };
    match class {
        Classification::Ambiguous | Classification::Legacy => {
            if *disk != *crashed_disk {
                return Err(bad("ambiguous state must change NOTHING"));
            }
        }
        Classification::Committed => {
            if disk.marker.is_some() || disk.entry.is_some() {
                return Err(bad("a committed run must drain the journal"));
            }
            if disk.current != Some(target) {
                return Err(bad("a committed run must point at the target"));
            }
            // committed cleanup never touches the destination
            if disk.dest != crashed_disk.dest {
                return Err(bad("committed recovery must not write destinations"));
            }
        }
        Classification::Uncommitted => {
            if disk.marker.is_some() || disk.entry.is_some() {
                return Err(bad("an uncommitted run must drain the journal"));
            }
            if disk.current != Some(prev) {
                return Err(bad("an uncommitted run must keep current == previous"));
            }
            let expected = if user_edited {
                Some(USER_EDIT) // post-crash edits are never overwritten
            } else {
                match kind {
                    RunKind::Deploy => Some(PRIOR),
                    RunKind::Prune => Some(DEPLOYED),
                }
            };
            if disk.dest != expected {
                return Err(bad(
                    "recovery left an unexplained mixture (or overwrote a user edit)",
                ));
            }
        }
    }
    Ok(())
}

/// Walk every reachable state of one run kind and direction. Returns
/// (states explored, violations).
fn explore(
    prev: u64,
    target: u64,
    kind: RunKind,
    legacy: bool,
    classifier: &dyn Fn(&RecoveryFacts) -> Classification,
) -> (usize, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(Node::initial(prev, kind, legacy), String::new())];
    let mut violations = Vec::new();
    let mut explored = 0usize;

    while let Some((node, trace)) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        explored += 1;

        if node.crashed {
            // resolve the crash: kill exposes volatile; a power loss
            // exposes durable plus EVERY subset of the pending writes
            for (crashed_disk, crash_kind) in crash_disks(&node) {
                for edit in [false, true] {
                    let mut disk = crashed_disk;
                    // the edit may CREATE the destination — for a
                    // removal run, that is exactly the REMOVED
                    // sentinel's keep case
                    if edit {
                        disk.dest = Some(USER_EDIT);
                    }
                    let (after, class) = recover(disk, prev, target, classifier);
                    if let Err(violation) =
                        check_oracle(&disk, &after, class, kind, edit, prev, target)
                    {
                        violations
                            .push(format!("{trace} {crash_kind} (edit: {edit})\n{violation}"));
                    }
                }
            }
            continue;
        }

        if node.step >= STEPS {
            // the clean run: no crash, nothing to reconcile — the
            // oracle reduces to "committed, drained, and the
            // destination is the deployed content"
            let terminal = node.durable;
            let mut violations_here = check_oracle(
                &terminal,
                &terminal,
                Some(Classification::Committed),
                kind,
                false,
                prev,
                target,
            );
            // committed also requires the destination landed
            if terminal.dest
                != match kind {
                    RunKind::Deploy => Some(DEPLOYED),
                    RunKind::Prune => None,
                }
            {
                violations_here = Err(format!("clean run: wrong final destination: {terminal:?}"));
            }
            if let Err(violation) = violations_here {
                violations.push(format!("{trace}\nclean run\n{violation}"));
            }
            continue;
        }

        // three continuations: the full step, a crash before it, a
        // crash midway through it (effect written, flush never ran)
        let i = node.step;

        let mut full = node.clone();
        step_effect(&mut full.volatile, i, prev, target, kind, full.legacy);
        full.flush();
        full.step += 1;
        stack.push((full, format!("{trace} step{i}\n")));

        let mut before = node.clone();
        before.crashed = true;
        stack.push((before, format!("{trace} CRASH before step{i}\n")));

        let mut midway = node.clone();
        step_effect(&mut midway.volatile, i, prev, target, kind, midway.legacy);
        midway.crashed = true; // note: no flush — the write is pending
        stack.push((midway, format!("{trace} CRASH mid-step{i}\n")));
    }
    (explored, violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headline proof: both run kinds, both directions, every
    /// crash point, both crash kinds, every power-loss subset, with
    /// and without a post-crash user edit — the oracle holds in every
    /// terminal state.
    #[test]
    fn every_schedule_satisfies_the_oracle() {
        let mut total = 0;
        for (prev, target) in [(1, 2), (2, 1)] {
            for kind in [RunKind::Deploy, RunKind::Prune] {
                let (explored, violations) = explore(prev, target, kind, false, &classify);
                total += explored;
                assert!(
                    violations.is_empty(),
                    "prev={prev} target={target} {kind:?}: {} violation(s):\n{}",
                    violations.len(),
                    violations.join("\n---\n")
                );
            }
        }
        eprintln!("transaction model: {total} states explored, zero violations");
    }

    /// The counterexample, kept: pre-0.23 markers (no previous
    /// generation) classify by direction — a roll-FORWARD killed
    /// before its flip reads `current(1) <= target(2)` as committed
    /// and abandons a half-restored destination. The explorer finds
    /// it in milliseconds; the shipped marker shape makes the class
    /// unrepresentable.
    #[test]
    fn legacy_markers_misclassify_rollforward_and_shipped_ones_cannot() {
        // the 0.22 rule, kept model-local as archaeology (production
        // refuses legacy markers since 0.26)
        // the 0.22 rule classified by the RUN's direction (the
        // marker's op field was never read even then)
        fn legacy_classify(direction: RunOp, facts: &RecoveryFacts) -> Classification {
            match (direction, facts.current) {
                (RunOp::Apply, Some(c)) if c >= facts.target => Classification::Committed,
                (RunOp::Rollback, Some(c)) if c <= facts.target => Classification::Committed,
                _ => Classification::Uncommitted,
            }
        }
        let (_, violations) = explore(1, 2, RunKind::Deploy, true, &|facts| {
            legacy_classify(RunOp::Rollback, facts)
        });
        assert!(
            violations
                .iter()
                .any(|v| v.contains("must point at the target")),
            "the legacy direction rule must produce the known counterexample; got: {violations:?}"
        );
        // the same schedule space with shipped markers: clean
        // shipped markers through the REAL classifier: clean. And
        // legacy markers through it: refuse (Ambiguous), never guess
        let (_, violations) = explore(1, 2, RunKind::Deploy, false, &classify);
        assert!(violations.is_empty(), "{violations:?}");
        let (_, violations) = explore(1, 2, RunKind::Deploy, true, &classify);
        assert!(violations.is_empty(), "{violations:?}");
    }

    impl std::fmt::Debug for RunKind {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                RunKind::Deploy => write!(f, "Deploy"),
                RunKind::Prune => write!(f, "Prune"),
            }
        }
    }
}
