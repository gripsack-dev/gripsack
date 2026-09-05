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
    declared: bool,
}

/// The actions the model enumerates.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// The repo's content flips A↔B.
    SourceUpdate,
    /// The app/user writes to the live file.
    ExternalWrite,
    /// gripsack apply (no take-over).
    Apply,
    /// gripsack apply --take-over (adoption or explicit absorb).
    TakeOver,
    /// The module is undeclared (prune / rollback to empty).
    Undeclare,
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
        Action::Apply => {
            if !l.declared {
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
        Action::TakeOver => {
            if !l.declared {
                return l;
            }
            // a genuine take-over captures the origin ONCE per epoch
            if l.origin.is_none() {
                l.origin = Some(l.live);
            }
            l.live = l.desired;
            l.manifest = Some((l.desired, false));
            l
        }
        Action::Undeclare => {
            if !l.declared {
                return l;
            }
            // an epoch ends ONLY by restoring the origin (0029 §1):
            // managed content is replaced by what was there at
            // adoption; preserved drift was never ours and stays
            let managed = l.manifest.is_some_and(|(_, preserved)| !preserved);
            if managed && let Some(origin) = l.origin {
                l.live = origin;
            }
            l.origin = None;
            l.manifest = None;
            l.declared = false;
            l
        }
    }
}

/// Per-transition: an open epoch's origin survives everything except
/// relinquish, relinquish RESTORES it (0029 §1), and apply never
/// writes over preserved drift (0029 §2 — an external write between
/// applies is user drift, not a clobber; the apply following it must
/// still not write).
fn check_transition(state: &Lineage, action: Action, next: &Lineage) -> Result<(), String> {
    if matches!(action, Action::Apply)
        && state.manifest.is_some_and(|(_, p)| p)
        && state.live != state.desired
        && next.live != state.live
    {
        return Err(format!(
            "apply overwrote preserved drift: {state:?} -> {next:?}"
        ));
    }
    if state.origin.is_some() && !matches!(action, Action::Undeclare) && next.origin != state.origin
    {
        return Err(format!(
            "the origin was lost without relinquishing ownership: {state:?} -{action:?}-> {next:?}"
        ));
    }
    if matches!(action, Action::Undeclare)
        && state.declared
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
        Action::Apply,
        Action::TakeOver,
        Action::Undeclare,
    ];
    // the two meaningful starts: a foreign file (origin to capture)
    // and a fresh machine (nothing to capture)
    let starts = [
        Lineage {
            live: 0,
            desired: 1,
            manifest: None,
            origin: None,
            declared: true,
        },
        Lineage {
            live: 1,
            desired: 1,
            manifest: Some((1, false)),
            origin: None,
            declared: true,
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
                        assert!(allowed, "desired={desired} live={live:?} prev={prev:?} → {plan:?}");
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
