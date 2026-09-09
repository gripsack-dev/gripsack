//! Bounded exhaustive two-destination recovery model (0042).
//!
//! Every independent record/mutation order and recovery/deletion order,
//! every issued-write crash window, and zero, one or two crashes are
//! explored. Recovery restarts from disk, not from its old program counter.
//! Kill exposes volatile state; power loss chooses every subset of the six
//! independently pending disk fields. Entry deletion has a shared barrier,
//! followed by a separate marker barrier, just like shipped cleanup.
//!
//! `previous` may be None: a fresh machine's first run starts with no
//! current generation at all, so the shipped classifier's (None, current)
//! branches — absent means uncommitted, at-target means committed — are
//! driven here, not just the Some(previous) rows.
//!
//! Filesystem mechanics and restore verification are assumed successful;
//! these are abstract identities, intentionally not hash fixtures (0036).
//! User edits here are durable replacement/creation edits, not deletions:
//! the shipped absent-live rule intentionally restores an existing prior.
//! No concurrent edits occur under the lifecycle lock. A new edit can occur
//! after EACH crash, including after one destination has been recovered.

use super::{Classification, RecoveryFacts, classify};
use crate::journal::recover::{RecoveryDecision, decide_from};
use crate::journal::{Intended, ObjectIdentity};
use std::collections::HashSet;

/// The symbolic vocabulary, mapped to the production's typed
/// identities (0045 F1): stand-ins ride the Link variant — typed
/// equality is what the kernel exercises, so only the equality
/// relationships between symbols matter. Cross-variant inequality is
/// pinned separately (journal::tests).
fn ident(content: &'static str) -> ObjectIdentity {
    ObjectIdentity::Link(content.to_string())
}

type Content = Option<&'static str>;
const ALL: u8 = 3;
const MAX_CRASHES: u8 = 2;

#[derive(Clone, Copy, Debug)]
struct Scenario {
    previous: Option<u64>,
    target: u64,
    prior: [Content; 2],
    intended: [Content; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Entry {
    prior: Content,
    intended: Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Disk {
    dest: [Content; 2],
    entries: [Option<Entry>; 2],
    current: Option<u64>,
    marker: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Phase {
    Begin,
    Work { recorded: u8, mutated: u8 },
    Flip,
    Classify,
    Restore(u8),
    Delete,
    RemoveMarker,
    Done,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    durable: Disk,
    volatile: Disk,
    phase: Phase,
    // An issued operation has not yet reached its barrier.
    barrier: bool,
    crashes: u8,
    edits: [Content; 2],
}

impl Node {
    fn initial(s: Scenario) -> Self {
        let disk = Disk {
            dest: s.prior,
            entries: [None; 2],
            current: s.previous,
            marker: false,
        };
        Self {
            durable: disk,
            volatile: disk,
            phase: Phase::Begin,
            barrier: false,
            crashes: 0,
            edits: [None; 2],
        }
    }

    fn issued(mut self, disk: Disk, phase: Phase, barrier: bool) -> Self {
        self.volatile = disk;
        self.phase = phase;
        self.barrier = barrier;
        self
    }
}

#[derive(Clone, Copy)]
enum Policy {
    Shipped,
    // Historical direction heuristic: falsely commits interrupted apply.
    NumericCommit,
}

fn classification(d: Disk, s: Scenario, policy: Policy) -> Classification {
    if !d.marker {
        return Classification::Uncommitted;
    }
    let facts = RecoveryFacts {
        previous: s.previous,
        target: s.target,
        current: d.current,
    };
    let shipped = classify(&facts);
    match policy {
        // The pre-0028 direction heuristic, driven through the same disk:
        // absent current counts as zero, so a fresh machine's interrupted
        // first run is falsely committed too.
        Policy::NumericCommit if d.current.is_none_or(|c| c <= s.target) => {
            Classification::Committed
        }
        _ => shipped,
    }
}

/// One protocol instruction; barriers are separate explorer instructions so
/// a crash can occur after the effect but before durability is guaranteed.
fn successors(n: Node, s: Scenario, policy: Policy) -> Vec<Node> {
    if n.barrier {
        let mut next = n;
        next.durable = next.volatile;
        next.barrier = false;
        return vec![next];
    }
    let mut d = n.volatile;
    let mut out = Vec::new();
    match n.phase {
        Phase::Begin => {
            d.marker = true;
            out.push(n.issued(
                d,
                Phase::Work {
                    recorded: 0,
                    mutated: 0,
                },
                true,
            ));
        }
        Phase::Work { recorded, mutated } => {
            for i in 0..2 {
                let bit = 1 << i;
                let mut next = d;
                if recorded & bit == 0 {
                    next.entries[i] = Some(Entry {
                        prior: s.prior[i],
                        intended: s.intended[i],
                    });
                    out.push(n.issued(
                        next,
                        Phase::Work {
                            recorded: recorded | bit,
                            mutated,
                        },
                        true,
                    ));
                } else if mutated & bit == 0 {
                    next.dest[i] = s.intended[i];
                    out.push(n.issued(
                        next,
                        Phase::Work {
                            recorded,
                            mutated: mutated | bit,
                        },
                        true,
                    ));
                }
            }
            if mutated == ALL {
                out.push(n.issued(d, Phase::Flip, false));
            }
        }
        Phase::Flip => {
            d.current = Some(s.target);
            out.push(n.issued(d, Phase::Delete, true));
        }
        Phase::Classify => {
            let phase = match classification(d, s, policy) {
                Classification::Committed => Phase::Delete,
                Classification::Uncommitted => Phase::Restore(0),
                Classification::Ambiguous => Phase::Blocked,
            };
            out.push(n.issued(d, phase, false));
        }
        Phase::Restore(done) => {
            for i in 0..2 {
                let bit = 1 << i;
                if done & bit != 0 {
                    continue;
                }
                let mut next = d;
                if let Some(entry) = d.entries[i] {
                    let intended = match entry.intended {
                        Some(content) => Intended::Object(ident(content)),
                        None => Intended::Removed,
                    };
                    match decide_from(
                        d.dest[i].map(ident).as_ref(),
                        &intended,
                        entry.prior.map(ident).as_ref(),
                    ) {
                        RecoveryDecision::Restore => next.dest[i] = entry.prior,
                        RecoveryDecision::Keep | RecoveryDecision::Unchanged => {}
                    }
                }
                out.push(n.issued(next, Phase::Restore(done | bit), true));
            }
            if done == ALL {
                out.push(n.issued(d, Phase::Delete, false));
            }
        }
        Phase::Delete => {
            for i in 0..2 {
                if d.entries[i].is_some() {
                    let mut next = d;
                    next.entries[i] = None;
                    // Individual unlinks are NOT barriers. Both can be pending.
                    out.push(n.issued(next, Phase::Delete, false));
                }
            }
            if d.entries == [None; 2] {
                out.push(n.issued(d, Phase::RemoveMarker, true));
            }
        }
        Phase::RemoveMarker => {
            d.marker = false;
            out.push(n.issued(d, Phase::Done, true));
        }
        Phase::Done | Phase::Blocked => {}
    }
    out
}

/// Enumerate kill and all power-loss outcomes; deduplicate equivalent
/// outcomes without relying on randomized hash iteration order.
fn exposures(n: Node) -> Vec<Disk> {
    let mut out = vec![n.volatile];
    for mask in 0..64 {
        let mut disk = n.durable;
        for i in 0..2 {
            if mask & (1 << i) != 0 {
                disk.dest[i] = n.volatile.dest[i];
            }
            if mask & (1 << (i + 2)) != 0 {
                disk.entries[i] = n.volatile.entries[i];
            }
        }
        if mask & 16 != 0 {
            disk.current = n.volatile.current;
        }
        if mask & 32 != 0 {
            disk.marker = n.volatile.marker;
        }
        if !out.contains(&disk) {
            out.push(disk);
        }
    }
    out
}

fn restarts(n: Node) -> Vec<Node> {
    if n.crashes == MAX_CRASHES {
        return Vec::new();
    }
    let mut out = Vec::new();
    for disk in exposures(n) {
        for edit_mask in 0..4 {
            let mut next = n;
            next.crashes += 1;
            next.phase = Phase::Classify;
            next.barrier = false;
            next.volatile = disk;
            for i in 0..2 {
                if edit_mask & (1 << i) != 0 {
                    let edit = match (next.crashes, i) {
                        (1, 0) => "user-a-first",
                        (1, _) => "user-b-first",
                        (_, 0) => "user-a-second",
                        _ => "user-b-second",
                    };
                    next.edits[i] = Some(edit);
                    next.volatile.dest[i] = Some(edit);
                }
            }
            next.durable = next.volatile;
            out.push(next);
        }
    }
    out
}

/// Independent oracle: expected contents come from transaction identity and
/// edit history, NEVER from the policy's classification or recovery choice.
fn oracle(n: Node, s: Scenario) -> Result<(), &'static str> {
    for d in [n.durable, n.volatile] {
        for i in 0..2 {
            if n.edits[i].is_some() && d.dest[i] != n.edits[i] {
                return Err("lost user edit");
            }
        }
        if !d.marker && d.entries != [None; 2] {
            return Err("false cleanup: entries outlived marker");
        }
        // Once evidence has been removed, that destination must already be
        // correct, even if another entry or the marker remains.
        for i in 0..2 {
            if d.entries[i].is_none() {
                let expected = n.edits[i].or(if d.current == Some(s.target) {
                    s.intended[i]
                } else {
                    s.prior[i]
                });
                if d.dest[i] != expected {
                    return Err("false commit/cleanup: destination lost recovery evidence");
                }
            }
        }
    }
    if n.phase == Phase::Done && !n.barrier {
        let d = n.durable;
        if d.marker || d.entries != [None; 2] {
            return Err("terminal journal did not drain");
        }
        for i in 0..2 {
            let expected = n.edits[i].or(if d.current == Some(s.target) {
                s.intended[i]
            } else {
                s.prior[i]
            });
            if d.dest[i] != expected {
                return Err("wrong prior or committed state");
            }
        }
    }
    Ok(())
}

fn explore(s: Scenario, policy: Policy) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut stack = vec![Node::initial(s)];
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        oracle(n, s).map_err(|why| format!("{why}\nscenario={s:?}\nnode={n:?}"))?;
        stack.extend(restarts(n));
        stack.extend(successors(n, s, policy));
    }
    Ok(())
}

#[test]
fn exhaustive_independent_destinations_and_two_crashes() {
    for (previous, target) in [(Some(1), 2), (Some(2), 1), (None, 2)] {
        // Replacement + creation, two prunes, and a mixed deployment/prune.
        // The None row is the fresh machine: no current generation exists
        // until the first flip, so an interrupted first run must classify
        // uncommitted and restore, never commit by direction.
        for (prior, intended) in [
            ([Some("old-a"), None], [Some("new-a"), Some("new-b")]),
            ([Some("old-a"), Some("old-b")], [None, None]),
            ([Some("old-a"), Some("old-b")], [Some("new-a"), None]),
        ] {
            let s = Scenario {
                previous,
                target,
                prior,
                intended,
            };
            if let Err(why) = explore(s, Policy::Shipped) {
                panic!("{why}");
            }
        }
    }
}

#[test]
fn numeric_commit_mutant_fails_the_identical_oracle() {
    let s = Scenario {
        previous: Some(1),
        target: 2,
        prior: [Some("old-a"), Some("old-b")],
        intended: [Some("new-a"), None],
    };
    let failure =
        explore(s, Policy::NumericCommit).expect_err("mutant must lose recovery evidence");
    assert!(failure.contains("false commit/cleanup"), "{failure}");
}

#[test]
fn partial_restore_then_second_crash_keeps_new_edit_and_restores_other_dest() {
    let s = Scenario {
        previous: Some(2),
        target: 1,
        prior: [Some("old-a"), Some("old-b")],
        intended: [Some("new-a"), None],
    };
    let mut n = Node::initial(s);
    n.volatile.marker = true;
    n.volatile.dest = s.intended;
    n.volatile.entries = std::array::from_fn(|i| {
        Some(Entry {
            prior: s.prior[i],
            intended: s.intended[i],
        })
    });
    n.durable = n.volatile;
    n.crashes = 1;
    n.phase = Phase::Restore(0);
    n = successors(n, s, Policy::Shipped)[0];
    n = successors(n, s, Policy::Shipped)[0]; // restore barrier
    assert_eq!(n.durable.dest, [Some("old-a"), None]);
    n = restarts(n)
        .into_iter()
        .find(|x| x.edits[0] == Some("user-a-second") && x.edits[1].is_none())
        .unwrap();
    while n.phase != Phase::Done || n.barrier {
        oracle(n, s).unwrap();
        n = successors(n, s, Policy::Shipped)[0];
    }
    oracle(n, s).unwrap();
    assert_eq!(n.durable.dest, [Some("user-a-second"), Some("old-b")]);
    assert_eq!(n.durable.entries, [None; 2]);
    assert!(!n.durable.marker);
}

#[test]
fn ambiguous_current_retains_partial_journal_without_touching_destinations() {
    let s = Scenario {
        previous: Some(1),
        target: 2,
        prior: [Some("a"), Some("b")],
        intended: [None, None],
    };
    let mut n = Node::initial(s);
    n.volatile.current = Some(99);
    n.volatile.marker = true;
    n.volatile.entries[1] = Some(Entry {
        prior: s.prior[1],
        intended: None,
    });
    n.volatile.dest[1] = Some("user-b-second");
    n.durable = n.volatile;
    n.phase = Phase::Classify;
    let next = successors(n, s, Policy::Shipped);
    assert_eq!(next[0].phase, Phase::Blocked);
    assert_eq!(next[0].volatile, n.volatile);
    assert_eq!(next[0].durable, n.durable);
    assert!(successors(next[0], s, Policy::Shipped).is_empty());
}
