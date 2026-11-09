//! The per-unit ownership decision (plan/0056 §5 K1, F2 step 6 in part):
//! what gripsack may do to one whole-object destination — a tracked copy, a
//! template, an owned link, or the removal of one of those — given three
//! identities and the user's consent.
//!
//! - **recorded**: what the previous generation's manifest says gripsack
//!   last wrote at this destination ([`Recorded::Wrote`]), an observation it
//!   kept without writing (preserved drift, [`Recorded::Observed`]), or
//!   nothing ([`Recorded::Unrecorded`]);
//! - **live**: what the destination holds right now (one observation);
//! - **intended**: what this run wants there (an object, or its removal);
//! - **consent**: whether the user explicitly allowed absorbing a foreign
//!   or drifted object at this destination (`--take-over`, adoption).
//!
//! Ownership comes from recorded identity only. The kernel sees digests,
//! never paths: where a link points cannot make it gripsack's (the bug of
//! treating every link into `$GRIPSACK_HOME` as ours), and a record cannot
//! authorize replacing an object that is no longer the one recorded (FND-1:
//! a user's replacement of an owned link was re-pointed).
//!
//! Proved (proofs/Kernel/Ownership.lean): a decision that writes, replaces
//! or removes the destination is issued only if nothing is live, the live
//! object is exactly gripsack's last write, or consent was given (K1); a
//! drifted destination is never written without consent; the decision
//! depends on identities only through equality; and gripsack's own write is
//! always replaceable, an absent destination always creatable.
//!
//! The host's journal re-check pins the decision to the exact live object it
//! was made on: the mutation is refused if the destination changed between
//! observation and act (S5).
//!
//! Aeneas shape (lib.rs): no derives (impls in `convenience.rs`), equality as
//! explicit `same` functions, owned `Copy` facts, no closures.
use crate::recover::{Ident, Intended, Live, intends};

/// What the previous generation's manifest records for this destination.
pub enum Recorded {
    /// No record: gripsack has not written here in the current ownership
    /// epoch (or never did).
    Unrecorded,
    /// The record is an observation gripsack kept without writing (preserved
    /// drift, 0029 §2). It confers no authority.
    Observed,
    /// The exact object gripsack last wrote here.
    Wrote(Ident),
}

impl Recorded {
    /// Whether `object` is exactly gripsack's last write here.
    pub fn wrote(&self, object: &Ident) -> bool {
        match self {
            Recorded::Wrote(ident) => ident.same(object),
            Recorded::Observed => false,
            Recorded::Unrecorded => false,
        }
    }
}

/// The user's answer to "absorb what stands at this destination?".
pub enum TakeOver {
    Withheld,
    Granted,
}

/// Explicit user consent for one destination. Today the product has one
/// kind: take-over (`--take-over`, `grip adopt`, a saved plan's adoption
/// set), scoped per destination by the host.
pub struct Consent {
    pub take_over: TakeOver,
}

impl Consent {
    pub fn grants_take_over(&self) -> bool {
        match self.take_over {
            TakeOver::Granted => true,
            TakeOver::Withheld => false,
        }
    }
}

/// Why a destination is left alone.
pub enum Refusal {
    /// gripsack recorded this destination, but the live object is not its
    /// last write (or the record is preserved drift): the user's change.
    Drifted,
    /// Nothing records this destination as gripsack's.
    Unowned,
    /// The live object has no identity gripsack can record (a directory, a
    /// special file).
    Unsupported,
}

/// The per-unit decision.
pub enum Decision {
    /// Nothing is live: create the intended object.
    Create,
    /// Nothing to do: the intended object is live, or the removal already
    /// happened.
    Satisfied,
    /// The live object is gripsack's last write: replace it with the intended
    /// object (or remove it).
    ReplaceOwn,
    /// Consent absorbs the live object; the host captures it as the prior.
    TakeOver,
    /// Leave the destination untouched.
    Refuse(Refusal),
}

impl Decision {
    /// Whether this decision changes the destination.
    pub fn writes(&self) -> bool {
        match self {
            Decision::Create => true,
            Decision::ReplaceOwn => true,
            Decision::TakeOver => true,
            Decision::Satisfied => false,
            Decision::Refuse(_) => false,
        }
    }
}

/// Decide one unit. Order: an object without identity is refused; absence is
/// created (or a removal is already done); gripsack's own last write is kept
/// when it is already the intent and replaced otherwise; consent absorbs
/// anything else, even bytes that already match (a take-over opens a new
/// ownership epoch); a foreign object that already is the intent is
/// satisfied (reconvergence); everything else is refused.
pub fn decide_unit(
    recorded: &Recorded,
    live: &Live,
    intended: &Intended,
    consent: &Consent,
) -> Decision {
    match live {
        Live::Unsupported => Decision::Refuse(Refusal::Unsupported),
        Live::Absent => match intended {
            Intended::Removed => Decision::Satisfied,
            Intended::Obj(_) => Decision::Create,
        },
        Live::Obj(object) => {
            if recorded.wrote(object) {
                if intends(intended, object) {
                    Decision::Satisfied
                } else {
                    Decision::ReplaceOwn
                }
            } else if acquires(consent, intended) {
                Decision::TakeOver
            } else if intends(intended, object) {
                Decision::Satisfied
            } else {
                Decision::Refuse(refusal(recorded))
            }
        }
    }
}

/// Take-over acquires an object for a deploy; it never authorizes a removal.
fn acquires(consent: &Consent, intended: &Intended) -> bool {
    match intended {
        Intended::Removed => false,
        Intended::Obj(_) => consent.grants_take_over(),
    }
}

fn refusal(recorded: &Recorded) -> Refusal {
    match recorded {
        Recorded::Unrecorded => Refusal::Unowned,
        Recorded::Observed => Refusal::Drifted,
        Recorded::Wrote(_) => Refusal::Drifted,
    }
}
