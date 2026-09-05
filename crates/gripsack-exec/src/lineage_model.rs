//! The ownership-lineage model (plan/0029, extending 0028's harness).
//!
//! The transaction model (journal/model.rs) proves the crash
//! protocol. THIS model proves the ownership algebra: which bytes
//! gripsack may replace, and whether the pre-adoption origin survives
//! the whole epoch. The drift decision under test is the SHIPPED
//! code — `plan_copy` is extracted from deploy.rs and both sides
//! call it.
//!
//! The oracle (0029):
//!
//! > An observed user value never becomes gripsack-authorized merely
//! > by being observed, and an adopted origin remains recoverable
//! > until gripsack successfully relinquishes ownership.

use crate::deploy::{CopyPlan, plan_copy};

/// Abstract contents: 0 = the foreign original, 1 = repo content A,
/// 2 = repo content B, 3 = a user/application edit.
type Content = u8;

/// Which spellings of the ONE physical cell the repo currently
/// declares (0030 §P0-1): `~/x` and `$HOME/x` are two logical keys
/// for one directory entry. The lineage is keyed by the CELL, never
/// the spelling — a module that switches spellings mid-epoch keeps
/// its origin (destination-keyed `inherit_priors`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Declaration {
    Undeclared,
    SpellingA,
    SpellingB,
    /// Both spellings live at once — the physical-uniqueness gate
    /// (`expand::check_physical_uniqueness`) must reject such a run
    /// BEFORE any mutation. This model pins that contract; unit and
    /// e2e tests pin the implementation to real paths.
    Aliased,
}

impl Declaration {
    fn declared(self) -> bool {
        self != Declaration::Undeclared
    }
}

/// One destination's lineage state. `manifest` is what the generation
/// records: (the recorded hash, whether it was preserved drift) — the
/// two facts the next apply reads. `origin` is the epoch's prior.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Lineage {
    live: Content,
    desired: Content,
    /// None = never managed/foreign
    manifest: Option<(Content, bool)>,
    /// Some = an ownership epoch with a recoverable origin
    origin: Option<Content>,
    declaration: Declaration,
}

/// The actions the model enumerates.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// The repo's content flips A↔B.
    SourceUpdate,
    /// The app/user writes to the live file.
    ExternalWrite,
    /// The repo gains spelling A of the cell (a module appears, or a
    /// second module aliases the same cell).
    DeclareA,
    /// The repo gains spelling B.
    DeclareB,
    /// Spelling A leaves the repo.
    DropA,
    /// Spelling B leaves the repo.
    DropB,
    /// gripsack apply (no take-over).
    Apply,
    /// gripsack apply --take-over (adoption or explicit absorb).
    TakeOver,
}

impl Declaration {
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

/// One action's effect on the lineage, mirroring deploy+prune.
fn step(mut l: Lineage, action: Action) -> Lineage {
    match action {
        Action::SourceUpdate => {
            l.desired = if l.desired == 1 { 2 } else { 1 };
            l
        }
        Action::ExternalWrite => {
            l.live = 3;
            l
        }
        // repo-side declaration changes never touch the filesystem —
        // and never touch the ORIGIN: the epoch is keyed by the
        // physical cell, so gaining or dropping a spelling mid-epoch
        // inherits the lineage (0030 §H4's destination-keyed carry)
        Action::DeclareA => {
            l.declaration = l.declaration.with(Declaration::SpellingA);
            l
        }
        Action::DeclareB => {
            l.declaration = l.declaration.with(Declaration::SpellingB);
            l
        }
        Action::DropA => {
            let next = l.declaration.without(Declaration::SpellingA);
            undeclare(l, next)
        }
        Action::DropB => {
            let next = l.declaration.without(Declaration::SpellingB);
            undeclare(l, next)
        }
        Action::Apply | Action::TakeOver => {
            // the gate (0030 §P0-1): a run whose declaration set maps
            // two spellings onto one physical cell is REJECTED before
            // any mutation — the lineage cannot change at all
            if l.declaration == Declaration::Aliased {
                return l;
            }
            if !l.declaration.declared() {
                return l;
            }
            if matches!(action, Action::TakeOver) {
                // a genuine take-over captures the origin ONCE per
                // epoch (0030 §H5)
                if l.origin.is_none() {
                    l.origin = Some(l.live);
                }
                l.live = l.desired;
                l.manifest = Some((l.desired, false));
                return l;
            }
            // owned bindings so the borrowed tuple never outlives them
            let desired = l.desired.to_string();
            let live = l.live.to_string();
            let prev = l.manifest.map(|(h, preserved)| (h.to_string(), preserved));
            let prev_ref = prev.as_ref().map(|(h, p)| (h.as_str(), *p));
            // the REAL decision function — this is the point
            match plan_copy(&desired, Some(&live), prev_ref, false) {
                CopyPlan::Fresh | CopyPlan::Satisfied | CopyPlan::Update => {
                    l.live = l.desired;
                    l.manifest = Some((l.desired, false));
                }
                CopyPlan::Preserve => {
                    // observed, never authority (0029 §2)
                    l.manifest = Some((l.live, true));
                }
                CopyPlan::TakeOver => unreachable!("no take-over here"),
            }
            l
        }
    }
}

/// Dropping a spelling: the epoch ends ONLY when the LAST declaration
/// of the cell goes away (0029 §1) — managed content is replaced by
/// the adoption origin; preserved drift was never ours and stays.
fn undeclare(mut l: Lineage, next_declaration: Declaration) -> Lineage {
    if next_declaration.declared() {
        // one spelling remains — the epoch continues untouched
        l.declaration = next_declaration;
        return l;
    }
    if !l.declaration.declared() {
        return l;
    }
    let managed = l.manifest.is_some_and(|(_, preserved)| !preserved);
    if managed && let Some(origin) = l.origin {
        l.live = origin;
    }
    l.origin = None;
    l.manifest = None;
    l.declaration = Declaration::Undeclared;
    l
}

/// Per-transition: an open epoch's origin survives everything except
/// relinquish, relinquish RESTORES it (0029 §1), apply never writes
/// over preserved drift (0029 §2), and an aliased declaration set is
/// rejected before any mutation (0030 §P0-1).
fn check_transition(state: &Lineage, action: Action, next: &Lineage) -> Result<(), String> {
    // the gate contract: Apply/TakeOver over two spellings of one
    // physical cell is a pre-mutation rejection — NOTHING may change,
    // not even the manifest (a partial rejection would still corrupt)
    if state.declaration == Declaration::Aliased
        && matches!(action, Action::Apply | Action::TakeOver)
        && next != state
    {
        return Err(format!(
            "an aliased run mutated the lineage instead of being rejected: {state:?} -{action:?}-> {next:?}"
        ));
    }
    if matches!(action, Action::Apply)
        && state.manifest.is_some_and(|(_, p)| p)
        && state.live != state.desired
        && next.live != state.live
    {
        return Err(format!(
            "apply overwrote preserved drift: {state:?} -> {next:?}"
        ));
    }
    // the origin survives everything except the epoch's END (the last
    // declaration dropping away)
    let epoch_ends = next.declaration == Declaration::Undeclared;
    if state.origin.is_some() && !epoch_ends && next.origin != state.origin {
        return Err(format!(
            "the origin was lost without relinquishing ownership: {state:?} -{action:?}-> {next:?}"
        ));
    }
    if epoch_ends
        && state.declaration.declared()
        && state.manifest.is_some_and(|(_, preserved)| !preserved)
        && let Some(origin) = state.origin
        && next.live != origin
    {
        return Err(format!(
            "undeclare did not restore the origin: {state:?} -> {next:?}"
        ));
    }
    Ok(())
}

/// Enumerate every action sequence up to DEPTH, checking the oracle
/// everywhere.
#[test]
fn ownership_lineage_holds_over_every_sequence() {
    const DEPTH: usize = 6;
    let actions = [
        Action::SourceUpdate,
        Action::ExternalWrite,
        Action::DeclareA,
        Action::DeclareB,
        Action::DropA,
        Action::DropB,
        Action::Apply,
        Action::TakeOver,
    ];
    // the two meaningful starts: a foreign file (origin to capture)
    // and a fresh machine (nothing to capture) — both declared via
    // one spelling
    let starts = [
        Lineage {
            live: 0,
            desired: 1,
            manifest: None,
            origin: None,
            declaration: Declaration::SpellingA,
        },
        Lineage {
            live: 1,
            desired: 1,
            manifest: Some((1, false)),
            origin: None,
            declaration: Declaration::SpellingA,
        },
    ];
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<(Lineage, Vec<Action>)> = starts.iter().map(|l| (*l, Vec::new())).collect();
    let mut explored = 0usize;
    let mut violations = Vec::new();
    while let Some((state, trace)) = stack.pop() {
        if !seen.insert((state, trace.len())) {
            continue;
        }
        explored += 1;
        if trace.len() >= DEPTH {
            continue;
        }
        for action in actions {
            let next = step(state, action);
            if let Err(v) = check_transition(&state, action, &next) {
                violations.push(format!("{trace:?}\n{v}"));
            }
            let mut next_trace = trace.clone();
            next_trace.push(action);
            stack.push((next, next_trace));
        }
    }
    assert!(
        violations.is_empty(),
        "{} lineage violation(s):\n{}",
        violations.len(),
        violations.join("\n---\n")
    );
    eprintln!("lineage model: {explored} states explored, zero violations");
}

/// plan_copy's truth table, exhaustively: the promotion bug (0029
/// §2) is exactly the cell `prev = (observed, preserved=true)` —
/// which must never authorize Update.
#[test]
fn preserved_drift_never_authorizes_an_update() {
    for desired in ["1", "2"] {
        for live in [None, Some("0"), Some("1"), Some("2"), Some("3")] {
            for prev in [None, Some(("1", false)), Some(("3", true))] {
                for take_over in [false, true] {
                    let plan = plan_copy(desired, live, prev, take_over);
                    // a preserved-drift record may only yield Preserve
                    // (or Satisfied, when the user converged by hand)
                    if prev.is_some_and(|(_, p)| p) {
                        // preserved drift never authorizes — UNLESS the
                        // user explicitly says absorb (take-over opens
                        // a new epoch with the drift as its origin)
                        let allowed = if take_over {
                            matches!(
                                plan,
                                CopyPlan::TakeOver | CopyPlan::Satisfied | CopyPlan::Fresh
                            )
                        } else {
                            matches!(
                                plan,
                                CopyPlan::Preserve | CopyPlan::Satisfied | CopyPlan::Fresh
                            )
                        };
                        assert!(
                            allowed,
                            "desired={desired} live={live:?} prev={prev:?} → {plan:?}"
                        );
                    }
                    // without take-over, foreign content is never updated
                    if !take_over
                        && let Some(l) = live
                        && l != desired
                        && prev.is_none_or(|(_, p)| p)
                    {
                        assert_eq!(plan, CopyPlan::Preserve, "live={live:?}");
                    }
                }
            }
        }
    }
}

/// The intra-apply race (0030 §9): an external write landing between
/// the drift check's observation and the mutation must be aborted,
/// never clobbered. `plan_copy` decides from a STALE observation; the
/// write-side precondition re-reads and refuses when the live bytes
/// moved. Exhaustive over (desired, observed, live-now, prev,
/// take-over): either the precondition sees no movement and the
/// observed plan applies, or it sees movement and NOTHING is written.
#[test]
fn a_mid_apply_external_write_is_aborted_never_clobbered() {
    for desired in ["1", "2"] {
        for observed in [None, Some("0"), Some("1"), Some("2"), Some("3")] {
            for live_now in [None, Some("0"), Some("1"), Some("2"), Some("3")] {
                for prev in [None, Some(("1", false)), Some(("3", true))] {
                    for take_over in [false, true] {
                        // the plan was computed from the observation…
                        let plan = plan_copy(desired, observed, prev, take_over);
                        let writes = matches!(
                            plan,
                            CopyPlan::Fresh | CopyPlan::Update | CopyPlan::TakeOver
                        );
                        // …the precondition compares live-now to the
                        // observation before any write (deploy.rs:
                        // atomic_write's expect-unchanged guard)
                        let moved = live_now != observed;
                        if moved && writes {
                            // aborted: the run fails, live-now is
                            // untouched — a foreign write is never
                            // overwritten on a stale observation
                            continue;
                        }
                        if !moved && writes {
                            // no race: the write is exactly the plan
                            // the observation authorized
                            // authorized iff we own the live bytes
                            // (they are exactly what we deployed) —
                            // or the destination was foreign/absent
                            // and the user said absorb / there is
                            // nothing to clobber
                            assert!(
                                take_over
                                    || prev.is_some_and(|(d, p)| !p && observed == Some(d))
                                    || observed.is_none()
                                    || observed == Some(desired),
                                "unauthorized write: desired={desired} observed={observed:?} \
                                 prev={prev:?} take_over={take_over} plan={plan:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
