//! The GC-machine explorer (plan/0056 F2 step 4): every history of up to
//! three generations — each naming at most one predecessor and pinning at most
//! one path — with every `current`, every `keep`, both modes, external roots
//! and pending recovery, driven through the shipped `gc::gc_step` by a
//! simulated host that executes each command on a model home.
//!
//! The per-step guarantees are Lean theorems (proofs/Kernel/Gc.lean). What
//! this adds is the end-to-end view against an oracle written independently
//! of the kernel: the generations left are exactly the oldest-excess prune
//! plan's complement closed under predecessors, the payloads left are exactly
//! the roots of those generations and the external roots, a preview or a
//! pending recovery changes nothing, and a retained generation whose
//! predecessor is missing stops the run before any effect.
use gripsack_kernel::gc::{Command, Event, GcState, InventoryFacts, Mode, Refusal, gc_step};
use gripsack_kernel::recover::Failure;
use gripsack_kernel::retention::{Candidate, ManifestFacts, Scope, Stamp};
use gripsack_kernel::{GenerationId, GenerationList};
use std::collections::BTreeSet;

const PATHS: [&str; 4] = ["store/a", "store/b", "prior/c", "generations/.staging-9-1"];

#[derive(Clone, Debug)]
struct History {
    /// Generation `i + 1`: its predecessor (0 names a missing generation)
    /// and the path it pins.
    generations: Vec<(Option<u64>, Option<usize>)>,
    current: Option<u64>,
    keep: Option<usize>,
    external: Option<usize>,
    pending: bool,
}

#[derive(Debug, PartialEq)]
enum End {
    Finished,
    Aborted(Refusal),
}

struct Home {
    generations: BTreeSet<u64>,
    paths: BTreeSet<usize>,
    effects: usize,
    barrier_after_last_retire: bool,
}

fn id(n: u64) -> GenerationId {
    GenerationId::new(n)
}

fn run(history: &History, mode: Mode, drift_on_remove: bool) -> (End, Home) {
    let count = history.generations.len() as u64;
    let mut home = Home {
        generations: (1..=count).collect(),
        paths: (0..PATHS.len()).collect(),
        effects: 0,
        barrier_after_last_retire: true,
    };
    let mut state = GcState::new(mode, history.keep);
    let mut event = Event::Start;
    let mut retiring = None;
    loop {
        let (next, command) = gc_step(state, event);
        state = next;
        event = match command {
            Command::ReadInventory => Event::Inventory(InventoryFacts {
                recovery_pending: history.pending,
                current: history.current.map(id),
                generations: GenerationList::new((1..=count).map(id).collect()).unwrap(),
            }),
            Command::ReadManifest(generation) => {
                let (predecessor, root) = history.generations[generation.value() as usize - 1];
                Event::Manifest(ManifestFacts {
                    id: generation,
                    predecessors: predecessor.map(id).into_iter().collect(),
                    roots: root
                        .map(|path| PATHS[path].as_bytes().to_vec())
                        .into_iter()
                        .collect(),
                })
            }
            Command::ReadRoots => Event::Roots(
                history
                    .external
                    .map(|path| PATHS[path].as_bytes().to_vec())
                    .into_iter()
                    .collect(),
            ),
            Command::ReadCandidates => Event::Candidates(
                (0..PATHS.len())
                    .map(|path| Candidate {
                        scope: if path == 3 {
                            Scope::Staging
                        } else {
                            Scope::Store
                        },
                        path: PATHS[path].as_bytes().to_vec(),
                        stamp: Stamp {
                            filesystem: 1,
                            inode: path as u64,
                        },
                    })
                    .collect(),
            ),
            Command::Plan => Event::Done,
            Command::SealFloor => {
                home.effects += 1;
                Event::Done
            }
            Command::Retire {
                generation,
                current,
            } => {
                home.effects += 1;
                assert_eq!(
                    current,
                    history.current.map(id),
                    "the re-check names the inventory"
                );
                // newest first
                if let Some(previous) = retiring {
                    assert!(generation.value() < previous, "{history:?}");
                }
                retiring = Some(generation.value());
                home.barrier_after_last_retire = false;
                assert!(home.generations.remove(&generation.value()));
                Event::Done
            }
            Command::Barrier => {
                home.effects += 1;
                home.barrier_after_last_retire = true;
                Event::Done
            }
            Command::Delegate => {
                home.effects += 1;
                Event::Done
            }
            Command::Remove { candidate, expect } => {
                home.effects += 1;
                assert!(home.barrier_after_last_retire);
                assert_eq!(expect.inode, candidate as u64);
                if drift_on_remove {
                    Event::Failed(Failure::Drift)
                } else {
                    assert!(home.paths.remove(&candidate));
                    Event::Done
                }
            }
            Command::Finish(_) => return (End::Finished, home),
            Command::Abort(refusal) => return (End::Aborted(refusal), home),
        };
    }
}

/// The oracle: the oldest excess beyond `keep` minus `current`, whose
/// complement is closed under predecessors.
fn expected_generations(history: &History) -> Result<BTreeSet<u64>, u64> {
    let count = history.generations.len();
    let excess = match history.keep {
        Some(keep) if count > keep => count - keep,
        _ => 0,
    };
    let pruned: BTreeSet<u64> = (1..=excess as u64)
        .filter(|g| history.current != Some(*g))
        .collect();
    let mut kept: BTreeSet<u64> = (1..=count as u64).filter(|g| !pruned.contains(g)).collect();
    loop {
        let mut grown = false;
        for g in kept.clone() {
            if let (Some(predecessor), _) = history.generations[g as usize - 1]
                && predecessor != 0
            {
                grown |= kept.insert(predecessor);
            }
        }
        if !grown {
            break;
        }
    }
    // the newest retained generation whose predecessor is missing
    match kept
        .iter()
        .rev()
        .find(|g| history.generations[**g as usize - 1].0 == Some(0))
    {
        Some(g) => Err(*g),
        None => Ok(kept),
    }
}

fn histories() -> Vec<History> {
    let mut generations: Vec<Vec<(Option<u64>, Option<usize>)>> = vec![vec![]];
    for n in 1..=3u64 {
        let mut longer = Vec::new();
        for prefix in generations.iter().filter(|g| g.len() as u64 == n - 1) {
            // a predecessor: none, any older generation, or a missing one
            let predecessors = std::iter::once(None)
                .chain((0..n).map(Some))
                .collect::<Vec<_>>();
            for predecessor in predecessors {
                for root in [None, Some(0), Some(1), Some(2)] {
                    let mut history = prefix.clone();
                    history.push((predecessor, root));
                    longer.push(history);
                }
            }
        }
        generations.extend(longer);
    }
    let mut all = Vec::new();
    for generations in generations {
        let count = generations.len() as u64;
        let currents = std::iter::once(None).chain((1..=count).map(Some));
        for current in currents {
            for keep in std::iter::once(None).chain((0..=count as usize).map(Some)) {
                for external in [None, Some(1)] {
                    all.push(History {
                        generations: generations.clone(),
                        current,
                        keep,
                        external,
                        pending: false,
                    });
                }
            }
        }
    }
    all
}

#[test]
fn collection_leaves_exactly_the_closed_retained_set_and_its_roots() {
    let histories = histories();
    let mut collected = 0;
    for history in &histories {
        let (end, home) = run(history, Mode::Collect, false);
        match expected_generations(history) {
            Err(generation) => {
                assert_eq!(
                    end,
                    End::Aborted(Refusal::MissingPredecessor {
                        generation: id(generation)
                    }),
                    "{history:?}"
                );
                assert_eq!(home.effects, 0, "{history:?}");
            }
            Ok(kept) => {
                assert_eq!(end, End::Finished, "{history:?}");
                assert_eq!(home.generations, kept, "{history:?}");
                let mut roots: BTreeSet<usize> = kept
                    .iter()
                    .filter_map(|g| history.generations[*g as usize - 1].1)
                    .collect();
                roots.extend(history.external);
                assert_eq!(home.paths, roots, "{history:?}");
                assert!(
                    history
                        .current
                        .is_none_or(|c| home.generations.contains(&c)),
                    "{history:?}"
                );
                collected += 1;
            }
        }
    }
    assert!(
        collected > 10_000,
        "{collected} of {} collected",
        histories.len()
    );
}

#[test]
fn a_preview_and_a_pending_recovery_change_nothing() {
    for history in histories() {
        let (end, home) = run(&history, Mode::Preview, false);
        assert_eq!(home.effects, 0, "{history:?}");
        if expected_generations(&history).is_ok() {
            assert_eq!(end, End::Finished);
        }
        let pending = History {
            pending: true,
            ..history
        };
        let (end, home) = run(&pending, Mode::Collect, false);
        assert_eq!(end, End::Aborted(Refusal::RecoveryPending));
        assert_eq!(home.effects, 0);
    }
}

#[test]
fn a_current_outside_the_inventory_is_refused() {
    let history = History {
        generations: vec![(None, Some(0)), (None, None)],
        current: Some(7),
        keep: Some(0),
        external: None,
        pending: false,
    };
    let (end, home) = run(&history, Mode::Collect, false);
    assert_eq!(end, End::Aborted(Refusal::CorruptCurrent));
    assert_eq!(home.effects, 0);
}

#[test]
fn a_drifted_removal_stops_the_run() {
    let history = History {
        generations: vec![(None, Some(0))],
        current: Some(1),
        keep: None,
        external: None,
        pending: false,
    };
    let (end, home) = run(&history, Mode::Collect, true);
    assert_eq!(end, End::Aborted(Refusal::Drifted));
    assert_eq!(home.paths.len(), PATHS.len());
}
