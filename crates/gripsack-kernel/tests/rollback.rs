//! The rollback-machine explorer (plan/0056 F2 step 8): every plan of up to
//! two units over a small identity alphabet, driven through the shipped
//! `rollback::rollback_step` by a simulated host that executes each command
//! faithfully (or, in the calibration, lands the wrong object).
//!
//! The per-step guarantees are Lean theorems (proofs/Kernel/Rollback.lean).
//! What this adds is the end-to-end view over a whole run: when the run
//! flips, every destination holds the target's identity, its release, or the
//! user's object it held before; no unit was written over an object that was
//! not gripsack's current write; a unit that needed a write the retained
//! bytes cannot reproduce, or a write into a generation directory, never
//! reaches the flip; and a host that lands the wrong object is caught.
use gripsack_kernel::recover::{Failure, Ident, Intended, Live, Route};
use gripsack_kernel::rollback::{
    Bytes, Command, Event, Place, PlanFacts, Profile, RollbackState, Target, UnitFacts,
    rollback_step,
};
use gripsack_kernel::selection::{SelectionIdentity, TransactionId};
use gripsack_kernel::{DestId, Digest, GenerationId, ownership::Recorded};

fn ident(n: u8) -> Ident {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    Ident::File(Digest::from_bytes(bytes))
}

/// gripsack's current write, the target's record, an origin it took over,
/// and a user's object.
const CURRENT: u8 = 1;
const TARGET: u8 = 2;
const ORIGIN: u8 = 3;
const USER: u8 = 4;

fn lives() -> Vec<Live> {
    let mut lives = vec![Live::Absent, Live::Unsupported];
    lives.extend([CURRENT, TARGET, ORIGIN, USER].map(|n| Live::Obj(ident(n))));
    lives
}

fn currents() -> Vec<Recorded> {
    vec![
        Recorded::Unrecorded,
        Recorded::Observed,
        Recorded::Wrote(ident(CURRENT)),
    ]
}

fn targets() -> Vec<Target> {
    vec![
        Target::Released(Intended::Removed),
        Target::Released(Intended::Obj(ident(ORIGIN))),
        Target::Wrote {
            ident: ident(TARGET),
            bytes: Bytes::Retained,
        },
        Target::Wrote {
            ident: ident(TARGET),
            bytes: Bytes::Missing,
        },
        Target::Preserved,
    ]
}

#[derive(Clone, Copy)]
struct Case {
    current: Recorded,
    target: Target,
    live: Live,
    place: Place,
    route: Route,
    /// A delegated unit's answer: settled, or unrestorable.
    delegated_ok: bool,
}

fn cases() -> Vec<Case> {
    let mut all = Vec::new();
    for current in currents() {
        for target in targets() {
            for live in lives() {
                for place in [Place::Outside, Place::Generation] {
                    all.push(Case {
                        current,
                        target,
                        live,
                        place,
                        route: Route::Leaf,
                        delegated_ok: true,
                    });
                }
            }
        }
    }
    for delegated_ok in [true, false] {
        all.push(Case {
            current: Recorded::Unrecorded,
            target: Target::Preserved,
            live: Live::Absent,
            place: Place::Outside,
            route: Route::Delegated,
            delegated_ok,
        });
    }
    all
}

fn selection(byte: u8) -> SelectionIdentity {
    SelectionIdentity::transaction(GenerationId::new(1), TransactionId::from_bytes([byte; 32]))
}

/// How the simulated host executes a write.
#[derive(Clone, Copy, PartialEq)]
enum Host {
    Faithful,
    /// Lands the current write instead of what was asked (a broken executor).
    LandsWrong,
}

struct Outcome {
    flipped: bool,
    finished: bool,
    /// Live objects at the end, per unit.
    world: Vec<Live>,
    /// Per unit: whether a write was issued.
    written: Vec<bool>,
}

fn run(units: &[Case], profile: Profile, previous: u8, host: Host) -> Outcome {
    let plan = PlanFacts {
        units: units.len() as u32,
        reselect: selection(9),
        previous: Some(selection(previous)),
        profile,
    };
    let mut world: Vec<Live> = units.iter().map(|case| case.live).collect();
    let mut written = vec![false; units.len()];
    let mut state = RollbackState::new();
    let mut event = Event::Start;
    let mut flipped = false;
    for _ in 0..64 {
        let (next, command) = rollback_step(state, event);
        state = next;
        event = match command {
            Command::ReadPlan => Event::PlanRead(plan),
            Command::ReadUnit(k) => {
                let case = units[k as usize];
                Event::UnitRead(UnitFacts {
                    dest: DestId::new(k),
                    place: case.place,
                    route: case.route,
                    current: case.current,
                    target: case.target,
                })
            }
            Command::Observe { unit, dest } => {
                assert_eq!(dest, DestId::new(unit));
                Event::Observed {
                    dest,
                    live: world[unit as usize],
                }
            }
            Command::Mutate {
                unit,
                expect,
                install,
                ..
            } => {
                let k = unit as usize;
                if world[k] != expect {
                    Event::Failed(Failure::Drift)
                } else {
                    written[k] = true;
                    world[k] = match (host, install) {
                        (Host::LandsWrong, _) => Live::Obj(ident(CURRENT)),
                        (Host::Faithful, Intended::Removed) => Live::Absent,
                        (Host::Faithful, Intended::Obj(object)) => Live::Obj(object),
                    };
                    Event::Landed { live: world[k] }
                }
            }
            Command::Delegate { unit, .. } => {
                if units[unit as usize].delegated_ok {
                    Event::Done
                } else {
                    Event::Refused
                }
            }
            Command::Report { .. } => Event::Done,
            Command::Flip(reselect) => {
                assert!(reselect == selection(9));
                flipped = true;
                Event::Done
            }
            Command::Finish(_) => {
                return Outcome {
                    flipped,
                    finished: true,
                    world,
                    written,
                };
            }
            Command::Abort(_) => {
                return Outcome {
                    flipped,
                    finished: false,
                    world,
                    written,
                };
            }
        };
    }
    panic!("the machine did not terminate");
}

/// What the target asks a write to install, if anything.
fn installs(target: Target) -> Option<Live> {
    match target {
        Target::Released(Intended::Removed) => Some(Live::Absent),
        Target::Released(Intended::Obj(object)) => Some(Live::Obj(object)),
        Target::Wrote { ident, .. } => Some(Live::Obj(ident)),
        Target::Preserved => None,
    }
}

fn ours(case: &Case) -> bool {
    match (case.current, case.live) {
        (Recorded::Wrote(written), Live::Obj(live)) => written == live,
        _ => false,
    }
}

/// Whether a faithful run must write this unit: the live object is absent or
/// gripsack's own write, and it is not already what the target asks.
fn needs_write(case: &Case) -> bool {
    if case.route == Route::Delegated {
        return false;
    }
    let Some(want) = installs(case.target) else {
        return false;
    };
    let releasing = matches!(case.target, Target::Released(_));
    match case.live {
        Live::Absent => !releasing && want != Live::Absent,
        Live::Unsupported => false,
        live => ours(case) && live != want,
    }
}

fn check(units: &[Case], outcome: &Outcome) {
    for (k, case) in units.iter().enumerate() {
        // K1: a write only over absence or gripsack's own current write
        if outcome.written[k] {
            assert!(
                case.live == Live::Absent || ours(case),
                "unit {k} written over a foreign object: {case_live:?}",
                case_live = case.live
            );
            // never into a generation directory
            assert!(
                case.place == Place::Outside,
                "unit {k} written in a generation"
            );
            // K2: exactly what the target records (or its release)
            assert_eq!(Some(outcome.world[k]), installs(case.target));
        } else {
            assert_eq!(
                outcome.world[k], case.live,
                "unit {k} changed without a write"
            );
        }
    }
    if outcome.flipped {
        assert!(outcome.finished);
        for (k, case) in units.iter().enumerate() {
            // a needed write that cannot be exact never reaches the flip
            if needs_write(case) {
                assert!(
                    outcome.written[k],
                    "unit {k} needed a write and was skipped"
                );
                assert!(matches!(
                    case.target,
                    Target::Released(_)
                        | Target::Wrote {
                            bytes: Bytes::Retained,
                            ..
                        }
                ));
            }
            assert!(case.route == Route::Leaf || case.delegated_ok);
        }
    } else {
        // every refusal has a reason in the plan
        let refusable = units.iter().any(|case| {
            (needs_write(case)
                && (case.place == Place::Generation
                    || matches!(
                        case.target,
                        Target::Wrote {
                            bytes: Bytes::Missing,
                            ..
                        }
                    )))
                || (case.route == Route::Delegated && !case.delegated_ok)
        });
        assert!(refusable, "a run refused without cause");
    }
}

#[test]
fn every_plan_of_up_to_two_units_restores_exactly_or_refuses() {
    let all = cases();
    let mut runs = 0;
    let mut flips = 0;
    for a in &all {
        let outcome = run(&[*a], Profile::Retained, 7, Host::Faithful);
        check(&[*a], &outcome);
        runs += 1;
        flips += usize::from(outcome.flipped);
        for b in &all {
            let units = [*a, *b];
            let outcome = run(&units, Profile::Retained, 7, Host::Faithful);
            check(&units, &outcome);
            runs += 1;
            flips += usize::from(outcome.flipped);
        }
    }
    assert!(runs > 30_000, "explored {runs} runs");
    assert!(flips > 0 && flips < runs, "flips {flips} of {runs}");
}

#[test]
fn an_empty_plan_flips_only_onto_an_admitted_selection() {
    assert!(run(&[], Profile::Retained, 7, Host::Faithful).flipped);
    // FND-2: a target without its own profile is refused, never backfilled
    let missing = run(&[], Profile::Missing, 7, Host::Faithful);
    assert!(!missing.flipped && !missing.finished);
    // K6: the reserved transaction must differ from the one it replaces
    let reused = run(&[], Profile::Retained, 9, Host::Faithful);
    assert!(!reused.flipped && !reused.finished);
}

#[test]
fn calibration_a_host_that_lands_the_wrong_object_is_caught() {
    // the explorer's oracle would accept nothing here: the machine itself
    // must refuse before the flip
    let case = Case {
        current: Recorded::Wrote(ident(CURRENT)),
        target: Target::Wrote {
            ident: ident(TARGET),
            bytes: Bytes::Retained,
        },
        live: Live::Absent,
        place: Place::Outside,
        route: Route::Leaf,
        delegated_ok: true,
    };
    let outcome = run(&[case], Profile::Retained, 7, Host::LandsWrong);
    assert!(outcome.written[0]);
    assert!(!outcome.flipped, "a wrong landing reached the flip");
}
