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

impl Lineage {
    /// The live object's identity as the shipped code would hash it:
    /// content, plus the mode when drifted (0031).
    fn live_identity(&self) -> String {
        if self.mode_drifted {
            format!("{}·m", self.live)
        } else {
            self.live.to_string()
        }
    }
}

impl Declaration {
    fn declared(self) -> bool {
        self != Declaration::Undeclared
    }
}

/// One destination's lineage state. `manifest` is what the generation
/// records: (the recorded hash, whether it was preserved drift) — the
/// two facts the next apply reads. `origin` is the epoch's prior.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Lineage {
    live: Content,
    /// The live object's mode differs from what gripsack recorded
    /// (0031): a chmod-only change is drift — the identity strings
    /// below carry it as a `·m` suffix, exactly as the shipped
    /// mode-aware preimage makes it a different hash
    mode_drifted: bool,
    desired: Content,
    /// None = never managed/foreign. The recorded identity is a
    /// STRING (the hash domain): a preserved observation may carry
    /// the mode-drift marker, which no Content value can name
    manifest: Option<(String, bool)>,
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
    /// The user chmods the live file without touching its bytes
    /// (0031) — drift, not an invisible no-op.
    ExternalChmod,
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
        Action::ExternalChmod => {
            l.mode_drifted = true;
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
                l.mode_drifted = false;
                l.manifest = Some((l.desired.to_string(), false));
                return l;
            }
            // owned bindings so the borrowed tuple never outlives them
            let desired = l.desired.to_string();
            let live = l.live_identity();
            let prev = l.manifest.clone();
            let prev_ref = prev.as_ref().map(|(h, p)| (h.as_str(), *p));
            // the REAL decision function — this is the point
            match plan_copy(&desired, Some(&live), prev_ref, false) {
                CopyPlan::Fresh | CopyPlan::Satisfied | CopyPlan::Update => {
                    l.live = l.desired;
                    l.mode_drifted = false;
                    l.manifest = Some((l.desired.to_string(), false));
                }
                CopyPlan::Preserve => {
                    // observed, never authority (0029 §2) — the
                    // recorded observation carries the mode drift
                    l.manifest = Some((l.live_identity(), true));
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
        // exact restoration: bytes AND mode (0031)
        l.mode_drifted = false;
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
        && state.manifest.as_ref().is_some_and(|(_, p)| *p)
        && state.live != state.desired
        && next.live != state.live
    {
        return Err(format!(
            "apply overwrote preserved drift: {state:?} -> {next:?}"
        ));
    }
    // chmod-only drift (0031): an apply must not silently re-mode a
    // managed file — the mode-aware identity reads it as drift, and
    // drift is preserved
    if matches!(action, Action::Apply)
        && state.mode_drifted
        && state.declaration.declared()
        && (next.live != state.live || !next.mode_drifted)
    {
        return Err(format!(
            "apply reverted chmod-only drift instead of preserving it: {state:?} -> {next:?}"
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
        && state
            .manifest
            .as_ref()
            .is_some_and(|(_, preserved)| !preserved)
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
        Action::ExternalChmod,
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
            mode_drifted: false,
            desired: 1,
            manifest: None,
            origin: None,
            declaration: Declaration::SpellingA,
        },
        Lineage {
            live: 1,
            mode_drifted: false,
            desired: 1,
            manifest: Some(("1".to_string(), false)),
            origin: None,
            declaration: Declaration::SpellingA,
        },
    ];
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<(Lineage, Vec<Action>)> =
        starts.iter().map(|l| (l.clone(), Vec::new())).collect();
    let mut explored = 0usize;
    let mut violations = Vec::new();
    while let Some((state, trace)) = stack.pop() {
        if !seen.insert((state.clone(), trace.len())) {
            continue;
        }
        explored += 1;
        if trace.len() >= DEPTH {
            continue;
        }
        for action in actions {
            let next = step(state.clone(), action);
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

/// Chmod-only drift (0031): the same bytes with a different mode is
/// drift — preserved like any drift, absorbed only by an explicit
/// take-over. The `·m` marker stands for the mode-aware preimage:
/// the shipped hash functions make "1" and "1·chmodded" different
/// identities, and this table proves the algebra on that fact.
#[test]
fn chmod_only_drift_is_preserved_never_silently_reverted() {
    // managed file, mode drifted, no take-over: preserve
    assert_eq!(
        plan_copy("1", Some("1·m"), Some(("1", false)), false),
        CopyPlan::Preserve
    );
    // an update is authorized ONLY off the exact recorded identity —
    // "1·m" is not "1"
    assert_eq!(
        plan_copy("2", Some("1·m"), Some(("1", false)), false),
        CopyPlan::Preserve
    );
    // explicit absorb opens a new epoch over the drifted mode
    assert_eq!(
        plan_copy("1", Some("1·m"), Some(("1", false)), true),
        CopyPlan::TakeOver
    );
    // and the happy path is unchanged: exact identity match stays
    // satisfied, recorded content updates cleanly
    assert_eq!(
        plan_copy("1", Some("1"), Some(("1", false)), false),
        CopyPlan::Satisfied
    );
    assert_eq!(
        plan_copy("2", Some("1"), Some(("1", false)), false),
        CopyPlan::Update
    );
}
