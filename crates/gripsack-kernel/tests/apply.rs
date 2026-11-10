//! The apply-machine explorer (plan/0056 F2 step 7): every run of up to two
//! units — deploys and releases over a small identity alphabet, on one or two
//! destinations — driven through the shipped `apply::apply_step` by a
//! simulated host with a disk, then committed under every commit fact.
//!
//! The per-step guarantees are Lean theorems (proofs/Kernel/Apply.lean). What
//! this adds is the whole-run view against an oracle written without the
//! kernel: every write lands exactly the unit's want over absence, gripsack's
//! own write or a consented object; a refused unit never reaches the flip; a
//! release never touches a destination a deploy of the run claims (FND-3); a
//! deploy after a release aborts; the run is vacuous exactly when nothing
//! changed and nothing landed; and a host that lands the wrong object is
//! caught before the flip.
use gripsack_kernel::apply::{
    ApplyState, Command, CommitInputs, Event, OnDrift, Outcome, Refusal, Settled, Slot, UnitFacts,
    Want, apply_step,
};
use gripsack_kernel::ownership::{Consent, Recorded, TakeOver};
use gripsack_kernel::recover::{Ident, Intended, Live, Route};
use gripsack_kernel::selection::{SelectionIdentity, TransactionId};
use gripsack_kernel::{DestId, Digest, GenerationId};
use std::collections::BTreeMap;

fn ident(n: u8) -> Ident {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    Ident::File(Digest::from_bytes(bytes))
}

/// gripsack's last write, the deploy's intent, the adoption a take-over
/// installs, a user's object, and the origin an epoch recorded.
const OWN: u8 = 1;
const WANT: u8 = 2;
const ADOPT: u8 = 3;
const USER: u8 = 4;
const ORIGIN: u8 = 5;

#[derive(Clone, Copy, Debug)]
struct Case {
    facts: UnitFacts,
    /// What the destination holds before the run, if this unit is the first
    /// to observe it.
    live: Live,
}

fn consent(granted: bool) -> Consent {
    Consent {
        take_over: if granted {
            TakeOver::Granted
        } else {
            TakeOver::Withheld
        },
    }
}

fn units(dest: u32) -> Vec<Case> {
    let lives = [
        Live::Absent,
        Live::Obj(ident(OWN)),
        Live::Obj(ident(WANT)),
        Live::Obj(ident(USER)),
    ];
    let records = [
        Recorded::Unrecorded,
        Recorded::Observed,
        Recorded::Wrote(ident(OWN)),
    ];
    let mut all = Vec::new();
    for live in lives {
        for recorded in records {
            for granted in [false, true] {
                for drift in [OnDrift::Keep, OnDrift::Stop] {
                    all.push(Case {
                        facts: UnitFacts {
                            dest: DestId::new(dest),
                            slot: Slot::Whole,
                            route: Route::Leaf,
                            recorded,
                            want: Want::Deploy {
                                ident: ident(WANT),
                                adopted: ident(ADOPT),
                            },
                            consent: consent(granted),
                            drift,
                        },
                        live,
                    });
                }
            }
            for origin in [Intended::Removed, Intended::Obj(ident(ORIGIN))] {
                all.push(Case {
                    facts: UnitFacts {
                        dest: DestId::new(dest),
                        slot: Slot::Whole,
                        route: Route::Leaf,
                        recorded,
                        want: Want::Release(origin),
                        consent: consent(false),
                        drift: OnDrift::Keep,
                    },
                    live,
                });
            }
        }
    }
    all
}

fn is_release(facts: &UnitFacts) -> bool {
    matches!(facts.want, Want::Release(_))
}

/// The oracle's K1: may this unit write over `live`, and with what?
fn oracle(facts: &UnitFacts, live: Live) -> Result<Option<Intended>, ()> {
    let ours = matches!((facts.recorded, live), (Recorded::Wrote(w), Live::Obj(o)) if w == o);
    let granted = facts.consent.take_over == TakeOver::Granted;
    match facts.want {
        Want::Deploy {
            ident: want,
            adopted,
        } => match live {
            Live::Absent => Ok(Some(Intended::Obj(want))),
            Live::Obj(o) if ours => Ok((o != want).then_some(Intended::Obj(want))),
            Live::Obj(_) if granted => Ok(Some(Intended::Obj(adopted))),
            Live::Obj(o) if o == want => Ok(None),
            _ => match facts.drift {
                OnDrift::Keep => Ok(None),
                OnDrift::Stop => Err(()),
            },
        },
        Want::Release(origin) => Ok(ours.then_some(origin)),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Host {
    Faithful,
    /// Lands the user's object instead of what was asked.
    LandsWrong,
}

struct Run {
    finished: Option<Outcome>,
    flipped: bool,
    aborted: Option<Refusal>,
    disk: BTreeMap<u32, Live>,
    landed: bool,
}

fn selection(byte: u8, generation: u64) -> SelectionIdentity {
    SelectionIdentity::transaction(
        GenerationId::new(generation),
        TransactionId::from_bytes([byte; 32]),
    )
}

fn run(cases: &[Case], inputs: CommitInputs, host: Host) -> Run {
    let mut disk: BTreeMap<u32, Live> = BTreeMap::new();
    for case in cases {
        disk.entry(case.facts.dest.value()).or_insert(case.live);
    }
    let mut out = Run {
        finished: None,
        flipped: false,
        aborted: None,
        disk,
        landed: false,
    };
    let mut state = ApplyState::new();
    let mut event = Event::Start;
    let mut published = false;
    let mut declared: Vec<u32> = Vec::new();
    loop {
        let (next, command) = apply_step(state, event);
        state = next;
        event = match command {
            Command::ReadUnit(index) => match cases.get(index as usize) {
                Some(case) => {
                    if !is_release(&case.facts) {
                        declared.push(case.facts.dest.value());
                    }
                    Event::UnitRead(case.facts)
                }
                None => Event::Exhausted,
            },
            Command::Observe { unit, dest } => {
                assert_eq!(cases[unit as usize].facts.dest, dest);
                Event::Observed {
                    dest,
                    live: out.disk[&dest.value()],
                }
            }
            Command::Mutate {
                unit,
                dest,
                expect,
                install,
                write,
            } => {
                let facts = cases[unit as usize].facts;
                assert_eq!(
                    expect,
                    out.disk[&dest.value()],
                    "the write answers the observation"
                );
                // K2 and K1, against the oracle
                let expected = oracle(&facts, expect).expect("a refused unit is never written");
                match (expected, install) {
                    (Some(Intended::Obj(a)), Intended::Obj(b)) => assert_eq!(a, b),
                    (Some(Intended::Removed), Intended::Removed) => {}
                    (wanted, got) => panic!("installs {got:?}, oracle {wanted:?} ({write:?})"),
                }
                // FND-3: a release never writes a destination a deploy claims
                if is_release(&facts) {
                    assert!(
                        !declared.contains(&dest.value()),
                        "a claimed destination was released"
                    );
                }
                let landed = match (host, install) {
                    (Host::LandsWrong, _) => Live::Obj(ident(USER)),
                    (Host::Faithful, Intended::Removed) => Live::Absent,
                    (Host::Faithful, Intended::Obj(o)) => Live::Obj(o),
                };
                out.disk.insert(dest.value(), landed);
                out.landed = true;
                Event::Landed { live: landed }
            }
            Command::Report { unit, settled, .. } => {
                let facts = cases[unit as usize].facts;
                if let Settled::Declared = settled {
                    assert!(is_release(&facts) && declared.contains(&facts.dest.value()));
                }
                Event::Done
            }
            Command::Delegate { .. } => Event::Delegated { mutated: false },
            Command::DirectoryStep => Event::Done,
            Command::ReadCommit => Event::CommitRead(inputs),
            Command::Publish(generation) => {
                assert_eq!(generation, inputs.allocated);
                published = true;
                Event::Done
            }
            Command::Flip(to) => {
                assert!(published, "the flip follows the publication");
                assert_eq!(to, inputs.reserved);
                out.flipped = true;
                Event::Done
            }
            Command::Activate => Event::Done,
            Command::Finish(outcome) => {
                out.finished = Some(outcome);
                return out;
            }
            Command::Abort(refusal) => {
                out.aborted = Some(refusal);
                return out;
            }
        };
    }
}

fn commits() -> Vec<CommitInputs> {
    let mut all = Vec::new();
    for previous in [None, Some(selection(1, 1))] {
        for unchanged in [false, true] {
            all.push(CommitInputs {
                previous,
                reserved: selection(2, 2),
                allocated: GenerationId::new(2),
                floor: Some(GenerationId::new(1)),
                unchanged,
                exists: false,
            });
        }
    }
    all
}

#[test]
fn every_run_writes_only_what_it_may_and_flips_only_when_settled() {
    let mut runs = 0usize;
    let mut flips = 0usize;
    let first = units(0);
    let mut seconds = units(0);
    seconds.extend(units(1));
    for inputs in commits() {
        for a in &first {
            let one = [*a];
            let outcome = run(&one, inputs, Host::Faithful);
            check(&one, inputs, &outcome);
            runs += 1;
            flips += usize::from(outcome.flipped);
            for b in &seconds {
                let two = [*a, *b];
                let outcome = run(&two, inputs, Host::Faithful);
                check(&two, inputs, &outcome);
                runs += 1;
                flips += usize::from(outcome.flipped);
            }
        }
    }
    assert!(runs > 30_000, "{runs} runs");
    assert!(flips > 1_000, "{flips} flips");
}

fn check(cases: &[Case], inputs: CommitInputs, outcome: &Run) {
    let order = cases
        .windows(2)
        .any(|pair| is_release(&pair[0].facts) && !is_release(&pair[1].facts));
    if order {
        assert!(
            matches!(outcome.aborted, Some(Refusal::Order { .. })),
            "a deploy after a release aborts"
        );
        return;
    }
    // a unit the oracle refuses stops the run before the flip
    let mut disk: BTreeMap<u32, Live> = BTreeMap::new();
    let mut refused = false;
    for case in cases {
        let live = *disk.entry(case.facts.dest.value()).or_insert(case.live);
        let claimed = is_release(&case.facts)
            && cases
                .iter()
                .any(|other| !is_release(&other.facts) && other.facts.dest == case.facts.dest);
        if claimed {
            continue;
        }
        match oracle(&case.facts, live) {
            Err(()) => {
                refused = true;
                break;
            }
            Ok(Some(Intended::Obj(o))) => {
                disk.insert(case.facts.dest.value(), Live::Obj(o));
            }
            Ok(Some(Intended::Removed)) => {
                disk.insert(case.facts.dest.value(), Live::Absent);
            }
            Ok(None) => {}
        }
    }
    if refused {
        assert!(matches!(outcome.aborted, Some(Refusal::Refused { .. })));
        assert!(!outcome.flipped);
        return;
    }
    assert_eq!(outcome.disk, disk, "the disk is the oracle's");
    let vacuous = inputs.previous.is_some() && inputs.unchanged && !outcome.landed;
    if vacuous {
        assert!(matches!(outcome.finished, Some(Outcome::Vacuous)) && !outcome.flipped);
    } else {
        assert!(matches!(outcome.finished, Some(Outcome::Applied)) && outcome.flipped);
    }
}

#[test]
fn a_host_that_lands_the_wrong_object_never_flips() {
    let inputs = commits()[0];
    let mut caught = 0;
    for case in units(0) {
        let outcome = run(&[case], inputs, Host::LandsWrong);
        if matches!(outcome.aborted, Some(Refusal::NotApplied { .. })) {
            caught += 1;
        }
        // a write that happens to land the user's object as intended is
        // impossible here: no want is the user's object
        if outcome.landed {
            assert!(!outcome.flipped, "{case:?}");
        }
    }
    assert!(caught > 10);
}

#[test]
fn an_unpublishable_commit_never_flips() {
    let case = units(0)[0];
    for (reserved, allocated, floor, exists) in [
        // a reused transaction
        (selection(1, 2), GenerationId::new(2), None, false),
        // bound to another generation
        (selection(2, 3), GenerationId::new(2), None, false),
        // not above the floor
        (
            selection(2, 2),
            GenerationId::new(2),
            Some(GenerationId::new(2)),
            false,
        ),
        // already published
        (selection(2, 2), GenerationId::new(2), None, true),
    ] {
        let inputs = CommitInputs {
            previous: Some(selection(1, 2)),
            reserved,
            allocated,
            floor,
            unchanged: false,
            exists,
        };
        let outcome = run(&[case], inputs, Host::Faithful);
        assert!(matches!(outcome.aborted, Some(Refusal::Unpublishable(_))));
        assert!(!outcome.flipped);
    }
}
