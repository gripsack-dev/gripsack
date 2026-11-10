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
//!
//! The directory sub-protocol (F2 step 9) is explored by a second walk: every
//! sequence of up to three items over two destinations — clears, deploys,
//! releases and directory steps (plan, evolve, hold, finish) answered
//! settled, settled-with-a-change or refused — against an oracle that never
//! lets a whole-object write land on a held destination, never commands a
//! directory step for a claimed one, aborts an item out of stage order, and
//! flips only after every directory step settled.
use gripsack_kernel::apply::{
    ApplyState, Command, CommitInputs, Event, OnDrift, Outcome, Refusal, Settled, Slot, UnitFacts,
    Want, apply_step,
};
use gripsack_kernel::directory::{Action, DirectoryFacts};
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
                for want in [Want::Release(origin), Want::Clear(origin)] {
                    all.push(Case {
                        facts: UnitFacts {
                            dest: DestId::new(dest),
                            slot: Slot::Whole,
                            route: Route::Leaf,
                            recorded,
                            want,
                            consent: consent(false),
                            drift: OnDrift::Keep,
                        },
                        live,
                    });
                }
            }
        }
    }
    all
}

fn is_release(facts: &UnitFacts) -> bool {
    matches!(facts.want, Want::Release(_))
}

fn is_deploy(facts: &UnitFacts) -> bool {
    matches!(facts.want, Want::Deploy { .. })
}

/// The stage rank the oracle expects (crate::directory::Stage, written out).
fn unit_rank(facts: &UnitFacts) -> u8 {
    match facts.want {
        Want::Clear(_) => 1,
        Want::Deploy { .. } => 4,
        Want::Release(_) => 5,
    }
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
        // a clear empties gripsack's own write, and refuses anything else
        Want::Clear(origin) => match live {
            Live::Absent => Ok(None),
            Live::Obj(_) if ours && matches!(origin, Intended::Removed) => {
                Ok(Some(Intended::Removed))
            }
            _ => Err(()),
        },
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
                    if is_deploy(&case.facts) {
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
            Command::Directory { .. } => panic!("no directory item was handed over"),
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
    // a unit the oracle refuses stops the run before the flip
    let mut disk: BTreeMap<u32, Live> = BTreeMap::new();
    let mut refused = false;
    let mut last = 0;
    for case in cases {
        if unit_rank(&case.facts) < last {
            assert!(
                matches!(outcome.aborted, Some(Refusal::Order { .. })),
                "a unit out of stage order (a deploy after a release) aborts"
            );
            return;
        }
        last = unit_rank(&case.facts);
        let live = *disk.entry(case.facts.dest.value()).or_insert(case.live);
        let claimed = is_release(&case.facts)
            && cases
                .iter()
                .any(|other| is_deploy(&other.facts) && other.facts.dest == case.facts.dest);
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
        assert!(matches!(
            outcome.aborted,
            Some(Refusal::Refused { .. } | Refusal::Blocked { .. })
        ));
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

/// One item of a directory-protocol run.
#[derive(Clone, Copy, Debug)]
enum Item {
    Unit(UnitFacts),
    /// A directory item and how the host answers its step.
    Dir(DirectoryFacts, Answer),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Settled { mutated: bool },
    Refused,
}

fn rank(item: &Item) -> u8 {
    match item {
        Item::Unit(facts) => unit_rank(facts),
        Item::Dir(facts, _) => match facts.action {
            Action::Bind | Action::Birth | Action::Plan => 0,
            Action::Evolve | Action::Shape | Action::Exchange | Action::Reactivate => 2,
            Action::Hold => 3,
            Action::Finish | Action::Retire => 6,
        },
    }
}

/// A small alphabet: gripsack's own write cleared, deployed over and
/// released, a user's object cleared, and the directory steps of an apply.
fn items() -> Vec<Item> {
    let mut all = Vec::new();
    for dest in [0, 1] {
        let unit = |want: Want| UnitFacts {
            dest: DestId::new(dest),
            slot: Slot::Whole,
            route: Route::Leaf,
            recorded: Recorded::Wrote(ident(OWN)),
            want,
            consent: consent(false),
            drift: OnDrift::Keep,
        };
        all.push(Item::Unit(unit(Want::Clear(Intended::Removed))));
        all.push(Item::Unit(unit(Want::Deploy {
            ident: ident(WANT),
            adopted: ident(ADOPT),
        })));
        all.push(Item::Unit(unit(Want::Release(Intended::Removed))));
        for action in [Action::Plan, Action::Evolve, Action::Finish] {
            for answer in [
                Answer::Settled { mutated: false },
                Answer::Settled { mutated: true },
                Answer::Refused,
            ] {
                all.push(Item::Dir(
                    DirectoryFacts {
                        dest: DestId::new(dest),
                        action,
                    },
                    answer,
                ));
            }
        }
        all.push(Item::Dir(
            DirectoryFacts {
                dest: DestId::new(dest),
                action: Action::Hold,
            },
            Answer::Settled { mutated: false },
        ));
    }
    all
}

struct DirectoryRun {
    outcome: Run,
    /// Every directory step commanded, and whether its answer was settled.
    steps: Vec<(u32, bool)>,
}

fn run_items(items: &[Item], inputs: CommitInputs) -> DirectoryRun {
    // gripsack's own write stands on both destinations
    let mut out = Run {
        finished: None,
        flipped: false,
        aborted: None,
        disk: BTreeMap::from([(0, Live::Obj(ident(OWN))), (1, Live::Obj(ident(OWN)))]),
        landed: false,
    };
    let mut steps = Vec::new();
    let mut held: Vec<u32> = Vec::new();
    let mut claimed: Vec<u32> = Vec::new();
    let mut state = ApplyState::new();
    let mut event = Event::Start;
    loop {
        let (next, command) = apply_step(state, event);
        state = next;
        event = match command {
            Command::ReadUnit(index) => match items.get(index as usize) {
                Some(Item::Unit(facts)) => {
                    if is_deploy(facts) {
                        claimed.push(facts.dest.value());
                    }
                    Event::UnitRead(*facts)
                }
                Some(Item::Dir(facts, _)) => {
                    if facts.action == Action::Hold {
                        held.push(facts.dest.value());
                    }
                    Event::DirectoryRead(*facts)
                }
                None => Event::Exhausted,
            },
            Command::Observe { dest, .. } => Event::Observed {
                dest,
                live: out.disk[&dest.value()],
            },
            Command::Mutate { dest, install, .. } => {
                assert!(
                    !held.contains(&dest.value()),
                    "a whole-object write on a held destination"
                );
                let landed = match install {
                    Intended::Removed => Live::Absent,
                    Intended::Obj(o) => Live::Obj(o),
                };
                out.disk.insert(dest.value(), landed);
                out.landed = true;
                Event::Landed { live: landed }
            }
            Command::Directory { unit, dest, action } => {
                let Item::Dir(facts, answer) = items[unit as usize] else {
                    panic!("a directory step for a unit");
                };
                assert_eq!((facts.dest, facts.action), (dest, action));
                assert_ne!(action, Action::Hold, "a hold is never executed");
                assert!(
                    !claimed.contains(&dest.value()),
                    "a directory step at a claimed destination"
                );
                steps.push((unit, false));
                match answer {
                    Answer::Settled { mutated } => {
                        steps.last_mut().unwrap().1 = true;
                        Event::Directed { mutated }
                    }
                    Answer::Refused => Event::Refused,
                }
            }
            Command::Report { .. } => Event::Done,
            Command::Delegate { .. } => panic!("no delegated unit"),
            Command::ReadCommit => Event::CommitRead(inputs),
            Command::Publish(_) => Event::Done,
            Command::Flip(_) => {
                assert!(
                    steps.iter().all(|(_, settled)| *settled),
                    "the flip follows every directory step's settlement"
                );
                out.flipped = true;
                Event::Done
            }
            Command::Activate => Event::Done,
            Command::Finish(outcome) => {
                out.finished = Some(outcome);
                return DirectoryRun {
                    outcome: out,
                    steps,
                };
            }
            Command::Abort(refusal) => {
                out.aborted = Some(refusal);
                return DirectoryRun {
                    outcome: out,
                    steps,
                };
            }
        };
    }
}

/// What the oracle expects of an item sequence: the first item that aborts
/// the run (and why), else whether anything changed the disk.
fn expect(items: &[Item]) -> Result<bool, &'static str> {
    let mut held: Vec<u32> = Vec::new();
    let mut claimed: Vec<u32> = Vec::new();
    let mut changed = false;
    let mut disk = BTreeMap::from([(0u32, Live::Obj(ident(OWN))), (1, Live::Obj(ident(OWN)))]);
    let mut last = 0;
    for item in items {
        if rank(item) < last {
            return Err("order");
        }
        last = rank(item);
        match item {
            Item::Unit(facts) => {
                let dest = facts.dest.value();
                match facts.want {
                    Want::Deploy { .. } | Want::Clear(_) if held.contains(&dest) => {
                        return Err("overlap");
                    }
                    Want::Release(_) if claimed.contains(&dest) || held.contains(&dest) => {
                        continue;
                    }
                    Want::Deploy { .. } => claimed.push(dest),
                    _ => {}
                }
                match oracle(facts, disk[&dest]) {
                    Err(()) => return Err("refused"),
                    Ok(Some(Intended::Obj(o))) => {
                        disk.insert(dest, Live::Obj(o));
                        changed = true;
                    }
                    Ok(Some(Intended::Removed)) => {
                        disk.insert(dest, Live::Absent);
                        changed = true;
                    }
                    Ok(None) => {}
                }
            }
            Item::Dir(facts, answer) => {
                let dest = facts.dest.value();
                if claimed.contains(&dest) {
                    return Err("overlap");
                }
                if facts.action == Action::Hold {
                    held.push(dest);
                    continue;
                }
                match answer {
                    Answer::Refused => return Err("directory"),
                    Answer::Settled { mutated } => changed |= mutated,
                }
            }
        }
    }
    Ok(changed)
}

#[test]
fn the_directory_sub_protocol_is_fenced() {
    let alphabet = items();
    let inputs = CommitInputs {
        previous: Some(selection(1, 1)),
        reserved: selection(2, 2),
        allocated: GenerationId::new(2),
        floor: Some(GenerationId::new(1)),
        unchanged: true,
        exists: false,
    };
    let mut runs = 0usize;
    let mut flips = 0usize;
    let mut sequences: Vec<Vec<Item>> = alphabet.iter().map(|item| vec![*item]).collect();
    for a in &alphabet {
        for b in &alphabet {
            sequences.push(vec![*a, *b]);
            for c in &alphabet {
                sequences.push(vec![*a, *b, *c]);
            }
        }
    }
    for sequence in &sequences {
        let run = run_items(sequence, inputs);
        runs += 1;
        let outcome = &run.outcome;
        match expect(sequence) {
            Err(why) => {
                assert!(!outcome.flipped, "{sequence:?}");
                let refusal = outcome.aborted.expect("an aborted run");
                let matches = match why {
                    "order" => matches!(refusal, Refusal::Order { .. }),
                    "overlap" => matches!(refusal, Refusal::Overlap { .. }),
                    "refused" => matches!(refusal, Refusal::Refused { .. }),
                    _ => matches!(refusal, Refusal::Directory { .. }),
                };
                assert!(matches, "{sequence:?}: {why} vs {refusal:?}");
            }
            Ok(changed) => {
                assert!(
                    outcome.aborted.is_none(),
                    "{sequence:?}: {:?}",
                    outcome.aborted
                );
                // unchanged manifests: vacuous exactly when nothing changed
                if changed {
                    assert!(outcome.flipped, "{sequence:?}");
                } else {
                    assert!(
                        matches!(outcome.finished, Some(Outcome::Vacuous)),
                        "{sequence:?}"
                    );
                }
                flips += usize::from(outcome.flipped);
                assert!(run.steps.iter().all(|(_, settled)| *settled));
            }
        }
    }
    assert!(runs > 15_000, "{runs} runs");
    assert!(flips > 1_000, "{flips} flips");
}
