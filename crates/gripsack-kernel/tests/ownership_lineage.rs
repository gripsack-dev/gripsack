//! The ownership-lineage explorer (plan/0029, ported from
//! `gripsack-exec/src/lineage_model.rs` in plan/0056 F2): every action
//! sequence up to a depth over one physical destination, with the shipped
//! kernel decision `ownership::decide_unit` choosing every deploy and prune.
//!
//! The per-call guarantees (K1, drift, identity-only) are Lean theorems
//! (proofs/Kernel/Ownership.lean) and are not re-tested here (plan/0057 D7).
//! What this explorer adds is the *sequence* view the theorems do not state:
//! the adoption origin survives the whole ownership epoch and comes back when
//! the epoch ends, aliases are rejected before any mutation, gripsack's own
//! write always converges, and — the per-run consequence of K1 — no apply or
//! prune ever changes a live object that is not gripsack's last write unless
//! the user took it over.
//!
//! The model is faithful to identities the host computes: a file is its
//! bytes plus mode, a link is its target. A link gripsack writes points at the
//! store payload of the desired content; a user can point a link anywhere,
//! including into the store. The calibration at the end replays the legacy
//! link rule (a recorded entry, or any link into the store, is "ours") and
//! requires the explorer to catch it.
use gripsack_kernel::Digest;
use gripsack_kernel::ownership::{Consent, Decision, Recorded, Refusal, TakeOver, decide_unit};
use gripsack_kernel::recover::{Ident, Intended, Live};
use std::collections::HashSet;

/// Abstract repo contents: 1 and 2 (the repo flips between them); 0 is the
/// foreign original, 3 a user/application edit.
type Content = u8;

/// A destination's live object. A link target 1/2 is the store payload of
/// that content; 3 points outside the store.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Object {
    File { content: Content, chmodded: bool },
    Link { target: Content },
}

impl Object {
    fn ident(self) -> Ident {
        let mut bytes = [0u8; 32];
        match self {
            Object::File { content, chmodded } => {
                bytes[1] = content;
                bytes[2] = chmodded as u8;
                Ident::File(Digest::from_bytes(bytes))
            }
            Object::Link { target } => {
                bytes[1] = target;
                Ident::Link(Digest::from_bytes(bytes))
            }
        }
    }

    fn into_store(self) -> bool {
        matches!(self, Object::Link { target: 1 | 2 })
    }
}

/// Which spellings of the one physical destination the repo declares
/// (0030 §P0-1): the lineage is keyed by the destination, never the
/// spelling. Both at once must be rejected before any mutation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Declaration {
    Undeclared,
    SpellingA,
    SpellingB,
    Aliased,
}

impl Declaration {
    fn declared(self) -> bool {
        self != Declaration::Undeclared
    }

    fn with(self, spelling: Declaration) -> Declaration {
        match (self, spelling) {
            (Declaration::Undeclared, s) => s,
            (Declaration::SpellingA, Declaration::SpellingB)
            | (Declaration::SpellingB, Declaration::SpellingA)
            | (Declaration::Aliased, _) => Declaration::Aliased,
            (d, _) => d,
        }
    }

    fn without(self, spelling: Declaration) -> Declaration {
        match (self, spelling) {
            (Declaration::Aliased, Declaration::SpellingA) => Declaration::SpellingB,
            (Declaration::Aliased, Declaration::SpellingB) => Declaration::SpellingA,
            (d, s) if d == s => Declaration::Undeclared,
            (d, _) => d,
        }
    }
}

/// Copies and links share one lineage epoch (the origin rides across a mode
/// change); what gripsack writes differs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Mode {
    Copy,
    Link,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Lineage {
    live: Option<Object>,
    desired: Content,
    /// The manifest record: the object and whether it is preserved drift
    /// (observed, never written).
    manifest: Option<(Object, bool)>,
    /// The epoch's adoption origin, restored when the epoch ends.
    origin: Option<Option<Object>>,
    declaration: Declaration,
    mode: Mode,
}

impl Lineage {
    fn intended(&self) -> Object {
        match self.mode {
            Mode::Copy => Object::File {
                content: self.desired,
                chmodded: false,
            },
            Mode::Link => Object::Link {
                target: self.desired,
            },
        }
    }

    fn recorded(&self) -> Recorded {
        match self.manifest {
            None => Recorded::Unrecorded,
            Some((_, true)) => Recorded::Observed,
            Some((object, false)) => Recorded::Wrote(object.ident()),
        }
    }

    fn live(&self) -> Live {
        match self.live {
            None => Live::Absent,
            Some(object) => Live::Obj(object.ident()),
        }
    }

    /// The live object is exactly gripsack's last write.
    fn intact(&self) -> bool {
        matches!(self.manifest, Some((object, false)) if self.live == Some(object))
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    SourceUpdate,
    /// The user/app replaces the destination with a file of their own.
    ExternalWrite,
    /// The user chmods a file in place (no-op on a link: chmod through it
    /// hits the read-only store payload).
    ExternalChmod,
    /// The user replaces the destination with their own link outside the
    /// store.
    ExternalLink,
    /// The user replaces the destination with their own link into the
    /// store, at the payload the repo does NOT currently want.
    ExternalStoreLink,
    DeclareA,
    DeclareB,
    DropA,
    DropB,
    ModeChange,
    Apply,
    TakeOver,
}

const ACTIONS: [Action; 12] = [
    Action::SourceUpdate,
    Action::ExternalWrite,
    Action::ExternalChmod,
    Action::ExternalLink,
    Action::ExternalStoreLink,
    Action::ModeChange,
    Action::DeclareA,
    Action::DeclareB,
    Action::DropA,
    Action::DropB,
    Action::Apply,
    Action::TakeOver,
];

type Decide = fn(&Recorded, &Live, &Intended, &Consent, &Lineage) -> Decision;

fn shipped(
    recorded: &Recorded,
    live: &Live,
    intended: &Intended,
    consent: &Consent,
    _: &Lineage,
) -> Decision {
    decide_unit(recorded, live, intended, consent)
}

fn step(mut l: Lineage, action: Action, decide: Decide) -> Lineage {
    match action {
        Action::SourceUpdate => l.desired = if l.desired == 1 { 2 } else { 1 },
        Action::ExternalWrite => {
            l.live = Some(Object::File {
                content: 3,
                chmodded: false,
            })
        }
        Action::ExternalChmod => {
            if let Some(Object::File { content, .. }) = l.live {
                l.live = Some(Object::File {
                    content,
                    chmodded: true,
                });
            }
        }
        Action::ExternalLink => l.live = Some(Object::Link { target: 3 }),
        Action::ExternalStoreLink => {
            l.live = Some(Object::Link {
                target: if l.desired == 1 { 2 } else { 1 },
            })
        }
        Action::ModeChange => {
            l.mode = match l.mode {
                Mode::Copy => Mode::Link,
                Mode::Link => Mode::Copy,
            }
        }
        Action::DeclareA => l.declaration = l.declaration.with(Declaration::SpellingA),
        Action::DeclareB => l.declaration = l.declaration.with(Declaration::SpellingB),
        Action::DropA => {
            let next = l.declaration.without(Declaration::SpellingA);
            return undeclare(l, next, decide);
        }
        Action::DropB => {
            let next = l.declaration.without(Declaration::SpellingB);
            return undeclare(l, next, decide);
        }
        Action::Apply | Action::TakeOver => {
            // the alias gate rejects the run before any mutation
            if l.declaration == Declaration::Aliased || !l.declaration.declared() {
                return l;
            }
            let consent = Consent {
                take_over: if matches!(action, Action::TakeOver) {
                    TakeOver::Granted
                } else {
                    TakeOver::Withheld
                },
            };
            let intended = l.intended();
            let decision = decide(
                &l.recorded(),
                &l.live(),
                &Intended::Obj(intended.ident()),
                &consent,
                &l,
            );
            match decision {
                Decision::Create | Decision::ReplaceOwn => {
                    l.live = Some(intended);
                    l.manifest = Some((intended, false));
                }
                Decision::TakeOver => {
                    // a genuine take-over captures the origin once per epoch
                    if l.origin.is_none() {
                        l.origin = Some(l.live);
                    }
                    l.live = Some(intended);
                    l.manifest = Some((intended, false));
                }
                // reconvergence: the live object is now gripsack's
                Decision::Satisfied => l.manifest = l.live.map(|o| (o, false)),
                // a copy records the observation as preserved drift; a link
                // refusal fails the run and changes nothing
                Decision::Refuse(_) => {
                    if l.mode == Mode::Copy {
                        l.manifest = l.live.map(|o| (o, true));
                    }
                }
            }
        }
    }
    l
}

/// The last spelling going away ends the epoch: prune decides with the same
/// kernel function (intent: removed, no consent). gripsack's intact write is
/// replaced by the origin (or removed); anything else stays.
fn undeclare(mut l: Lineage, next: Declaration, decide: Decide) -> Lineage {
    if next.declared() || !l.declaration.declared() {
        l.declaration = next;
        return l;
    }
    let decision = decide(
        &l.recorded(),
        &l.live(),
        &Intended::Removed,
        &Consent {
            take_over: TakeOver::Withheld,
        },
        &l,
    );
    if decision == Decision::ReplaceOwn {
        l.live = l.origin.unwrap_or(None);
    }
    l.origin = None;
    l.manifest = None;
    l.declaration = Declaration::Undeclared;
    l
}

fn check(state: &Lineage, action: Action, next: &Lineage) -> Result<(), String> {
    let applies = matches!(action, Action::Apply | Action::TakeOver);
    if state.declaration == Declaration::Aliased && applies && next != state {
        return Err("an aliased run mutated the lineage instead of being rejected".into());
    }
    // K1 over a run: without take-over, gripsack changes only absence or its
    // own last write — never preserved drift, never a user's replacement of
    // an owned link, never a user link into the store
    let gripsack_runs = matches!(action, Action::Apply | Action::DropA | Action::DropB);
    if gripsack_runs && next.live != state.live && state.live.is_some() && !state.intact() {
        return Err("a live object that is not gripsack's last write was changed".into());
    }
    // convergence: an intact or absent single-spelling destination reaches
    // the intent on apply
    if matches!(action, Action::Apply)
        && matches!(
            state.declaration,
            Declaration::SpellingA | Declaration::SpellingB
        )
        && (state.live.is_none() || state.intact())
        && next.live != Some(state.intended())
    {
        return Err("gripsack's own destination did not converge".into());
    }
    let epoch_ends = state.declaration.declared() && next.declaration == Declaration::Undeclared;
    if state.origin.is_some() && !epoch_ends && next.origin != state.origin {
        return Err("the origin was lost without relinquishing ownership".into());
    }
    if epoch_ends
        && state.intact()
        && let Some(origin) = state.origin
        && next.live != origin
    {
        return Err("undeclare did not restore the origin".into());
    }
    Ok(())
}

/// A violated oracle: the state, the action taken from it, and why.
struct Violation {
    state: Lineage,
    action: Action,
    trace: Vec<Action>,
    why: String,
}

/// Every sequence up to `depth` from a foreign file, a fresh machine and an
/// owned link; returns (states explored, violations).
fn explore(depth: usize, decide: Decide) -> (usize, Vec<Violation>) {
    let starts = [
        Lineage {
            live: Some(Object::File {
                content: 0,
                chmodded: false,
            }),
            desired: 1,
            manifest: None,
            origin: None,
            declaration: Declaration::SpellingA,
            mode: Mode::Copy,
        },
        Lineage {
            live: Some(Object::File {
                content: 1,
                chmodded: false,
            }),
            desired: 1,
            manifest: Some((
                Object::File {
                    content: 1,
                    chmodded: false,
                },
                false,
            )),
            origin: None,
            declaration: Declaration::SpellingA,
            mode: Mode::Copy,
        },
        Lineage {
            live: Some(Object::Link { target: 1 }),
            desired: 1,
            manifest: Some((Object::Link { target: 1 }, false)),
            origin: None,
            declaration: Declaration::SpellingA,
            mode: Mode::Link,
        },
    ];
    let mut seen = HashSet::new();
    let mut stack: Vec<(Lineage, Vec<Action>)> =
        starts.iter().map(|l| (l.clone(), Vec::new())).collect();
    let mut violations = Vec::new();
    while let Some((state, trace)) = stack.pop() {
        if !seen.insert((state.clone(), trace.len())) || trace.len() >= depth {
            continue;
        }
        for action in ACTIONS {
            let next = step(state.clone(), action, decide);
            if let Err(why) = check(&state, action, &next) {
                violations.push(Violation {
                    state: state.clone(),
                    action,
                    trace: trace.clone(),
                    why,
                });
            }
            let mut longer = trace.clone();
            longer.push(action);
            stack.push((next, longer));
        }
    }
    (seen.len(), violations)
}

#[test]
fn ownership_lineage_holds_over_every_sequence() {
    let (explored, violations) = explore(6, shipped);
    if let Some(v) = violations.first() {
        panic!(
            "{} lineage violation(s), first: {:?} then {:?}: {}\n  from {:?}",
            violations.len(),
            v.trace,
            v.action,
            v.why,
            v.state
        );
    }
    eprintln!("ownership lineage: {explored} states explored, zero violations");
}

/// The pre-0056 link rule, replayed for calibration: a destination with a
/// non-drift record, or holding any link into the store, is "ours" and gets
/// re-pointed; anything else is refused or taken over.
fn legacy_link_rule(
    recorded: &Recorded,
    live: &Live,
    intended: &Intended,
    consent: &Consent,
    lineage: &Lineage,
) -> Decision {
    if lineage.mode == Mode::Copy || matches!(intended, Intended::Removed) {
        return decide_unit(recorded, live, intended, consent);
    }
    let ours = lineage.live.is_some_and(Object::into_store);
    let on_record = matches!(recorded, Recorded::Wrote(_));
    match live {
        Live::Absent => Decision::Create,
        _ if lineage.live == Some(lineage.intended()) => Decision::Satisfied,
        _ if ours || on_record => Decision::ReplaceOwn,
        _ if consent.grants_take_over() => Decision::TakeOver,
        _ => Decision::Refuse(Refusal::Unowned),
    }
}

/// Calibration: the explorer must catch both halves of the legacy link rule —
/// FND-1 (a user's replacement of an owned link is re-pointed because a
/// record exists) and the store-prefix widening (an unrecorded user link into
/// the store is re-pointed because of where it points).
#[test]
fn the_legacy_link_rule_is_caught() {
    let (_, violations) = explore(5, legacy_link_rule);
    let applied = |v: &&Violation| matches!(v.action, Action::Apply) && v.state.mode == Mode::Link;
    let fnd1 = violations.iter().filter(applied).any(|v| {
        matches!(v.state.manifest, Some((_, false)))
            && matches!(
                v.state.live,
                Some(Object::File { content: 3, .. }) | Some(Object::Link { target: 3 })
            )
    });
    let widening = violations
        .iter()
        .filter(applied)
        .any(|v| v.state.manifest.is_none() && v.state.live.is_some_and(Object::into_store));
    assert!(fnd1, "the explorer missed FND-1 under the legacy rule");
    assert!(widening, "the explorer missed the store-prefix widening");
}
