//! Production conveniences for kernel types: formatting, parsing, ordering,
//! hashing and iteration. Nothing here is a decision and nothing here is
//! reached from the decision modules, so the Charon translation
//! (`[package.metadata.charon] start_from` in Cargo.toml) never sees it.
//! Ordering and hashing agree with `GenerationId::same`/`value` by
//! construction: both read the one `u64`.
use crate::{GenerationId, GenerationList};
use alloc::vec::Vec;
use core::{cmp::Ordering, fmt, hash, ops::Deref, str::FromStr};

impl fmt::Display for GenerationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

impl FromStr for GenerationId {
    type Err = core::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self::new)
    }
}

impl Ord for GenerationId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value().cmp(&other.value())
    }
}

impl PartialOrd for GenerationId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl hash::Hash for GenerationId {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.value().hash(state);
    }
}

impl Deref for GenerationList {
    type Target = [GenerationId];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl IntoIterator for GenerationList {
    type Item = GenerationId;
    type IntoIter = alloc::vec::IntoIter<GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a GenerationList {
    type Item = &'a GenerationId;
    type IntoIter = core::slice::Iter<'a, GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl Clone for GenerationList {
    fn clone(&self) -> Self {
        GenerationList::new(self.to_vec()).unwrap_or_else(GenerationList::empty)
    }
}

impl GenerationList {
    /// The admitted identities, for callers that need to own them again.
    pub fn to_vec(&self) -> Vec<GenerationId> {
        self.as_slice().to_vec()
    }
}

// ---- trait impls the decision modules would otherwise derive -------------
//
// Charon translates every impl in a module it starts from, derived or not,
// so the kernel modules carry no `#[derive]`; the impls live here, unreached
// by the translation. Equality delegates to the kernel's `same` functions,
// so `==` and the proved identity agree by construction.

use crate::{
    Classification,
    ids::{CmdId, DestId, Digest},
    journal::{DurableRecord, MutationAuthority, PublicationAction, RecordPublication, RecordRole},
    ownership::{Consent, Decision as UnitDecision, Recorded, Refusal as UnitRefusal, TakeOver},
    recover::{
        Command, Decision, Delegated, Effect, EntryFacts, Event, Failure, Ident, Intended,
        JournalFacts, JournalName, Live, MarkerFacts, Outcome, Phase, Prior, RecoverState, Refusal,
        Route, Scope, Verdict,
    },
    retention::GcAdmission,
    rollback::{
        Bytes, Command as RollbackCommand, Event as RollbackEvent, Outcome as RollbackOutcome,
        Phase as RollbackPhase, Place, PlanFacts, Profile, Refusal as RollbackRefusal, Restore,
        RollbackState, Settled, Target, UnitFacts,
    },
    selection::{RecoveryFacts, SelectionIdentity, TransactionId},
};

macro_rules! copy_impls {
    ($($type:ty),* $(,)?) => {$(
        impl Clone for $type {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl Copy for $type {}
    )*};
}
copy_impls!(
    GenerationId,
    DestId,
    Digest,
    CmdId,
    Classification,
    TransactionId,
    SelectionIdentity,
    RecoveryFacts,
    GcAdmission,
    RecordRole,
    PublicationAction,
    Ident,
    Live,
    Prior,
    Intended,
    Route,
    EntryFacts,
    MarkerFacts,
    JournalFacts,
    Decision,
    JournalName,
    Scope,
    Delegated,
    Effect,
    Outcome,
    Refusal,
    Command,
    Failure,
    Event,
    Phase,
    Verdict,
    RecoverState,
    Recorded,
    TakeOver,
    Consent,
    UnitRefusal,
    UnitDecision,
    Place,
    Bytes,
    Target,
    UnitFacts,
    Profile,
    PlanFacts,
    Settled,
    Restore,
    RollbackOutcome,
    RollbackRefusal,
    RollbackCommand,
    RollbackEvent,
    RollbackPhase,
    RollbackState,
);

macro_rules! same_eq {
    ($($type:ty => $same:expr),* $(,)?) => {$(
        impl PartialEq for $type {
            fn eq(&self, other: &Self) -> bool {
                let same: fn(&Self, &Self) -> bool = $same;
                same(self, other)
            }
        }
        impl Eq for $type {}
    )*};
}
same_eq!(
    GenerationId => |a, b| a.same(*b),
    DestId => |a, b| a.same(*b),
    Digest => |a, b| a.same(b),
    CmdId => |a, b| a.same(*b),
    TransactionId => |a, b| a.same(b),
    SelectionIdentity => |a, b| a.same(b),
    RecordRole => |a, b| a.same(*b),
    PublicationAction => |a, b| a.same(*b),
    Classification => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    GcAdmission => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Ident => |a, b| a.same(b),
    Live => |a, b| a.same(b),
    Prior => |a, b| match (a, b) {
        (Prior::Absent, Prior::Absent) => true,
        (Prior::Obj(a), Prior::Obj(b)) => a.same(b),
        _ => false,
    },
    Decision => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Outcome => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Phase => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Verdict => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Recorded => |a, b| match (a, b) {
        (Recorded::Unrecorded, Recorded::Unrecorded) => true,
        (Recorded::Observed, Recorded::Observed) => true,
        (Recorded::Wrote(a), Recorded::Wrote(b)) => a.same(b),
        _ => false,
    },
    TakeOver => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    UnitRefusal => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Route => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Place => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Bytes => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Profile => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    RollbackPhase => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    UnitDecision => |a, b| match (a, b) {
        (UnitDecision::Refuse(a), UnitDecision::Refuse(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
);

impl fmt::Debug for Recorded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Recorded::Unrecorded => f.write_str("Unrecorded"),
            Recorded::Observed => f.write_str("Observed"),
            Recorded::Wrote(ident) => f.debug_tuple("Wrote").field(ident).finish(),
        }
    }
}

impl fmt::Debug for Consent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Consent")
            .field("take_over", &self.take_over)
            .finish()
    }
}

impl fmt::Debug for UnitDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitDecision::Create => f.write_str("Create"),
            UnitDecision::Satisfied => f.write_str("Satisfied"),
            UnitDecision::ReplaceOwn => f.write_str("ReplaceOwn"),
            UnitDecision::TakeOver => f.write_str("TakeOver"),
            UnitDecision::Refuse(why) => f.debug_tuple("Refuse").field(why).finish(),
        }
    }
}

impl Default for RollbackState {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RecoverState {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GenerationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenerationId")
            .field("number", &self.value())
            .finish()
    }
}

impl fmt::Debug for GenerationList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenerationList")
            .field("ids", &self.as_slice())
            .finish()
    }
}

impl fmt::Debug for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionId")
            .field("bytes", self.as_bytes())
            .finish()
    }
}

impl fmt::Debug for SelectionIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy { generation } => f
                .debug_struct("Legacy")
                .field("generation", generation)
                .finish(),
            Self::Transaction {
                generation,
                transaction,
            } => f
                .debug_struct("Transaction")
                .field("generation", generation)
                .field("transaction", transaction)
                .finish(),
        }
    }
}

impl fmt::Debug for RecoveryFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryFacts")
            .field("previous", &self.previous)
            .field("target", &self.target)
            .field("current", &self.current)
            .finish()
    }
}

impl fmt::Debug for RecordPublication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordPublication")
            .field("role", &self.role())
            .field("next", &self.action())
            .finish()
    }
}

impl fmt::Debug for DurableRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DurableRecord")
            .field("role", &self.role())
            .finish()
    }
}

impl fmt::Debug for MutationAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationAuthority")
            .field("entry", self.entry())
            .finish()
    }
}

macro_rules! unit_debug {
    ($($type:ident { $($variant:ident),* $(,)? }),* $(,)?) => {$(
        impl fmt::Debug for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $($type::$variant => stringify!($variant)),* })
            }
        }
    )*};
}
unit_debug!(
    Classification {
        Committed,
        Uncommitted,
        Ambiguous
    },
    GcAdmission {
        Admitted,
        RecoveryPending,
        CorruptCurrent
    },
    RecordRole {
        RunMarker,
        MutationEntry
    },
    PublicationAction {
        Stage,
        SyncFile,
        PublishName,
        SyncNamespace,
        Complete,
        Refused
    },
    Route { Leaf, Delegated },
    TakeOver { Withheld, Granted },
    UnitRefusal {
        Drifted,
        Unowned,
        Unsupported
    },
    Decision {
        Restore,
        Unchanged,
        Preserve,
        Refuse
    },
    Scope { Journal, Selection },
    Delegated {
        Committed,
        BeforeLeaves,
        AfterLeaves
    },
    Outcome {
        NoJournal,
        Committed,
        Recovered
    },
    Verdict {
        Pending,
        Committed,
        Uncommitted
    },
    Phase {
        Start,
        ReadingJournal,
        ReadingCurrent,
        SealingSelection,
        SettlingDirectories,
        OpeningLeaves,
        ReadingEntry,
        Observing,
        Restoring,
        Sealing,
        Preserving,
        ClosingLeaves,
        UnlinkingRecord,
        SyncingRecords,
        UnlinkingMarker,
        SyncingMarker,
        Finished,
        Aborted
    },
    Place {
        Outside,
        Generation
    },
    Bytes { Retained, Missing },
    Profile { Retained, Missing },
    RollbackOutcome { RolledBack },
    RollbackPhase {
        Start,
        ReadingPlan,
        ReadingUnit,
        Observing,
        Writing,
        Delegating,
        Reporting,
        Flipping,
        Finished,
        Aborted
    },
);

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Released(origin) => f.debug_tuple("Released").field(origin).finish(),
            Target::Wrote { ident, bytes } => f
                .debug_struct("Wrote")
                .field("ident", ident)
                .field("bytes", bytes)
                .finish(),
            Target::Preserved => f.write_str("Preserved"),
        }
    }
}

impl fmt::Debug for UnitFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnitFacts")
            .field("dest", &self.dest)
            .field("place", &self.place)
            .field("route", &self.route)
            .field("current", &self.current)
            .field("target", &self.target)
            .finish()
    }
}

impl fmt::Debug for PlanFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlanFacts")
            .field("units", &self.units)
            .field("reselect", &self.reselect)
            .field("previous", &self.previous)
            .field("profile", &self.profile)
            .finish()
    }
}

impl fmt::Debug for Settled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Settled::Restored => f.write_str("Restored"),
            Settled::Already => f.write_str("Already"),
            Settled::Kept(why) => f.debug_tuple("Kept").field(why).finish(),
            Settled::Untouched => f.write_str("Untouched"),
            Settled::Delegated => f.write_str("Delegated"),
        }
    }
}

impl fmt::Debug for Restore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Restore::Write(install) => f.debug_tuple("Write").field(install).finish(),
            Restore::Leave(settled) => f.debug_tuple("Leave").field(settled).finish(),
            Restore::Refuse(why) => f.debug_tuple("Refuse").field(why).finish(),
        }
    }
}

impl fmt::Debug for RollbackRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = |f: &mut fmt::Formatter<'_>, name: &str, unit: &u32, dest: &DestId| {
            f.debug_struct(name)
                .field("unit", unit)
                .field("dest", dest)
                .finish()
        };
        match self {
            RollbackRefusal::ProfileMissing => f.write_str("ProfileMissing"),
            RollbackRefusal::Reused => f.write_str("Reused"),
            RollbackRefusal::Unrestorable { unit: u, dest } => unit(f, "Unrestorable", u, dest),
            RollbackRefusal::Protected { unit: u, dest } => unit(f, "Protected", u, dest),
            RollbackRefusal::Drifted { unit: u, dest } => unit(f, "Drifted", u, dest),
            RollbackRefusal::NotRestored { unit: u, dest } => unit(f, "NotRestored", u, dest),
            RollbackRefusal::Malformed => f.write_str("Malformed"),
            RollbackRefusal::Failed => f.write_str("Failed"),
            RollbackRefusal::Protocol => f.write_str("Protocol"),
        }
    }
}

impl fmt::Debug for RollbackCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RollbackCommand::ReadPlan => f.write_str("ReadPlan"),
            RollbackCommand::ReadUnit(unit) => f.debug_tuple("ReadUnit").field(unit).finish(),
            RollbackCommand::Observe { unit, dest } => f
                .debug_struct("Observe")
                .field("unit", unit)
                .field("dest", dest)
                .finish(),
            RollbackCommand::Mutate {
                unit,
                dest,
                expect,
                install,
            } => f
                .debug_struct("Mutate")
                .field("unit", unit)
                .field("dest", dest)
                .field("expect", expect)
                .field("install", install)
                .finish(),
            RollbackCommand::Delegate { unit, dest } => f
                .debug_struct("Delegate")
                .field("unit", unit)
                .field("dest", dest)
                .finish(),
            RollbackCommand::Report {
                unit,
                dest,
                settled,
            } => f
                .debug_struct("Report")
                .field("unit", unit)
                .field("dest", dest)
                .field("settled", settled)
                .finish(),
            RollbackCommand::Flip(selection) => f.debug_tuple("Flip").field(selection).finish(),
            RollbackCommand::Finish(outcome) => f.debug_tuple("Finish").field(outcome).finish(),
            RollbackCommand::Abort(why) => f.debug_tuple("Abort").field(why).finish(),
        }
    }
}

impl fmt::Debug for RollbackEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RollbackEvent::Start => f.write_str("Start"),
            RollbackEvent::PlanRead(plan) => f.debug_tuple("PlanRead").field(plan).finish(),
            RollbackEvent::UnitRead(unit) => f.debug_tuple("UnitRead").field(unit).finish(),
            RollbackEvent::Observed { dest, live } => f
                .debug_struct("Observed")
                .field("dest", dest)
                .field("live", live)
                .finish(),
            RollbackEvent::Landed { live } => f.debug_struct("Landed").field("live", live).finish(),
            RollbackEvent::Done => f.write_str("Done"),
            RollbackEvent::Refused => f.write_str("Refused"),
            RollbackEvent::Failed(failure) => f.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

impl fmt::Debug for RollbackState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RollbackState")
            .field("phase", &self.phase)
            .field("plan", &self.plan)
            .field("index", &self.index)
            .field("unit", &self.unit)
            .field("install", &self.install)
            .finish()
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ident::File(digest) => f.debug_tuple("File").field(digest).finish(),
            Ident::Link(digest) => f.debug_tuple("Link").field(digest).finish(),
        }
    }
}

impl fmt::Debug for Live {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Live::Absent => f.write_str("Absent"),
            Live::Obj(ident) => f.debug_tuple("Obj").field(ident).finish(),
            Live::Unsupported => f.write_str("Unsupported"),
        }
    }
}

impl fmt::Debug for Prior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Prior::Absent => f.write_str("Absent"),
            Prior::Obj(ident) => f.debug_tuple("Obj").field(ident).finish(),
        }
    }
}

impl fmt::Debug for Intended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Intended::Removed => f.write_str("Removed"),
            Intended::Obj(ident) => f.debug_tuple("Obj").field(ident).finish(),
        }
    }
}

impl fmt::Debug for EntryFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryFacts")
            .field("dest", &self.dest)
            .field("prior", &self.prior)
            .field("before", &self.before)
            .field("intended", &self.intended)
            .field("route", &self.route)
            .finish()
    }
}

impl fmt::Debug for MarkerFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarkerFacts")
            .field("previous", &self.previous)
            .field("target", &self.target)
            .finish()
    }
}

impl fmt::Debug for JournalFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JournalFacts")
            .field("entries", &self.entries)
            .field("directories", &self.directories)
            .field("reverse_roots", &self.reverse_roots)
            .field("marker", &self.marker)
            .finish()
    }
}

impl fmt::Debug for JournalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalName::Record(index) => f.debug_tuple("Record").field(index).finish(),
            JournalName::Marker => f.write_str("Marker"),
        }
    }
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::Restore(prior) => f.debug_tuple("Restore").field(prior).finish(),
        }
    }
}

impl fmt::Debug for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let entry = |f: &mut fmt::Formatter<'_>, name, entry: &u32, dest: &DestId| {
            f.debug_struct(name)
                .field("entry", entry)
                .field("dest", dest)
                .finish()
        };
        match self {
            Refusal::Ambiguous => f.write_str("Ambiguous"),
            Refusal::MarkerMissing => f.write_str("MarkerMissing"),
            Refusal::Malformed => f.write_str("Malformed"),
            Refusal::Unrestorable { entry: e, dest } => entry(f, "Unrestorable", e, dest),
            Refusal::Drifted { entry: e, dest } => entry(f, "Drifted", e, dest),
            Refusal::NotRestored { entry: e, dest } => entry(f, "NotRestored", e, dest),
            Refusal::Failed => f.write_str("Failed"),
            Refusal::Protocol => f.write_str("Protocol"),
        }
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Drift => f.write_str("Drift"),
            Failure::Io(errno) => f.debug_tuple("Io").field(errno).finish(),
        }
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::ReadJournal => f.write_str("ReadJournal"),
            Command::ReadCurrent => f.write_str("ReadCurrent"),
            Command::ReadEntry(index) => f.debug_tuple("ReadEntry").field(index).finish(),
            Command::Observe { entry, dest } => f
                .debug_struct("Observe")
                .field("entry", entry)
                .field("dest", dest)
                .finish(),
            Command::Mutate {
                entry,
                dest,
                expect,
                effect,
            } => f
                .debug_struct("Mutate")
                .field("entry", entry)
                .field("dest", dest)
                .field("expect", expect)
                .field("effect", effect)
                .finish(),
            Command::Seal {
                entry,
                dest,
                expect,
                prior,
            } => f
                .debug_struct("Seal")
                .field("entry", entry)
                .field("dest", dest)
                .field("expect", expect)
                .field("prior", prior)
                .finish(),
            Command::Preserve { entry, dest } => f
                .debug_struct("Preserve")
                .field("entry", entry)
                .field("dest", dest)
                .finish(),
            Command::Delegate(step) => f.debug_tuple("Delegate").field(step).finish(),
            Command::Unlink(name) => f.debug_tuple("Unlink").field(name).finish(),
            Command::Barrier(scope) => f.debug_tuple("Barrier").field(scope).finish(),
            Command::Finish(outcome) => f.debug_tuple("Finish").field(outcome).finish(),
            Command::Abort(refusal) => f.debug_tuple("Abort").field(refusal).finish(),
        }
    }
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Start => f.write_str("Start"),
            Event::NoJournal => f.write_str("NoJournal"),
            Event::JournalRead(journal) => f.debug_tuple("JournalRead").field(journal).finish(),
            Event::CurrentRead(current) => f.debug_tuple("CurrentRead").field(current).finish(),
            Event::EntryRead(facts) => f.debug_tuple("EntryRead").field(facts).finish(),
            Event::Observed { dest, live } => f
                .debug_struct("Observed")
                .field("dest", dest)
                .field("live", live)
                .finish(),
            Event::Landed { live } => f.debug_struct("Landed").field("live", live).finish(),
            Event::Done => f.write_str("Done"),
            Event::Failed(failure) => f.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

impl fmt::Debug for RecoverState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoverState")
            .field("phase", &self.phase)
            .field("journal", &self.journal)
            .field("records", &self.records)
            .field("current", &self.current)
            .field("verdict", &self.verdict)
            .field("index", &self.index)
            .field("leaves_open", &self.leaves_open)
            .field("entry", &self.entry)
            .finish()
    }
}

impl fmt::Debug for DestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DestId").field(&self.value()).finish()
    }
}

impl fmt::Debug for CmdId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CmdId").field(&self.value()).finish()
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digest(")?;
        for byte in self.as_bytes() {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

// Interned indices order and hash by their one number, like GenerationId.
macro_rules! numeric_order {
    ($($type:ty),* $(,)?) => {$(
        impl Ord for $type {
            fn cmp(&self, other: &Self) -> Ordering {
                self.value().cmp(&other.value())
            }
        }
        impl PartialOrd for $type {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl hash::Hash for $type {
            fn hash<H: hash::Hasher>(&self, state: &mut H) {
                self.value().hash(state);
            }
        }
    )*};
}
numeric_order!(DestId, CmdId);

impl hash::Hash for Digest {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

// ---- the GC machine (plan/0056 F2 step 4) ---------------------------------

use crate::gc::{
    Command as GcCommand, Event as GcEvent, GcState, InventoryFacts, Mode as GcMode,
    Outcome as GcOutcome, Phase as GcPhase, Refusal as GcRefusal,
};
use crate::retention::{
    Candidate, Fate, Manifest, ManifestFacts, Scope as GcScope, Stamp, Unclosed,
};

copy_impls!(
    GcMode, GcOutcome, GcRefusal, GcCommand, GcPhase, Fate, GcScope, Stamp, Unclosed,
);

same_eq!(
    GcMode => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    GcOutcome => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    GcPhase => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Fate => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    GcScope => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Stamp => |a, b| a.filesystem == b.filesystem && a.inode == b.inode,
    GcRefusal => |a, b| match (a, b) {
        (
            GcRefusal::MissingPredecessor { generation: a },
            GcRefusal::MissingPredecessor { generation: b },
        ) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
    GcCommand => |a, b| match (a, b) {
        (GcCommand::ReadManifest(a), GcCommand::ReadManifest(b)) => a == b,
        (
            GcCommand::Retire { generation: a, current: x },
            GcCommand::Retire { generation: b, current: y },
        ) => a == b && x == y,
        (
            GcCommand::Remove { candidate: a, expect: x },
            GcCommand::Remove { candidate: b, expect: y },
        ) => a == b && x == y,
        (GcCommand::Finish(a), GcCommand::Finish(b)) => a == b,
        (GcCommand::Abort(a), GcCommand::Abort(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
);

unit_debug!(
    GcMode { Preview, Collect },
    GcOutcome {
        Previewed,
        Collected
    },
    GcPhase {
        Start,
        ReadingInventory,
        ReadingManifests,
        ReadingRoots,
        ReadingCandidates,
        Planning,
        SealingFloor,
        Retiring,
        Sealing,
        Delegating,
        Removing,
        Finished,
        Aborted
    },
    Fate { Retained, Retired },
    GcScope {
        Store,
        Prior,
        Staging
    },
);

impl fmt::Debug for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stamp")
            .field("filesystem", &self.filesystem)
            .field("inode", &self.inode)
            .finish()
    }
}

impl fmt::Debug for Unclosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, generation) = match self {
            Unclosed::MissingPredecessor { generation } => ("MissingPredecessor", generation),
            Unclosed::Required { generation } => ("Required", generation),
        };
        f.debug_struct(name)
            .field("generation", generation)
            .finish()
    }
}

impl fmt::Debug for GcRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GcRefusal::RecoveryPending => f.write_str("RecoveryPending"),
            GcRefusal::CorruptCurrent => f.write_str("CorruptCurrent"),
            GcRefusal::MissingPredecessor { generation } => f
                .debug_struct("MissingPredecessor")
                .field("generation", generation)
                .finish(),
            GcRefusal::Malformed => f.write_str("Malformed"),
            GcRefusal::Drifted => f.write_str("Drifted"),
            GcRefusal::Failed => f.write_str("Failed"),
            GcRefusal::Protocol => f.write_str("Protocol"),
        }
    }
}

impl fmt::Debug for GcCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GcCommand::ReadInventory => f.write_str("ReadInventory"),
            GcCommand::ReadManifest(generation) => {
                f.debug_tuple("ReadManifest").field(generation).finish()
            }
            GcCommand::ReadRoots => f.write_str("ReadRoots"),
            GcCommand::ReadCandidates => f.write_str("ReadCandidates"),
            GcCommand::Plan => f.write_str("Plan"),
            GcCommand::SealFloor => f.write_str("SealFloor"),
            GcCommand::Retire {
                generation,
                current,
            } => f
                .debug_struct("Retire")
                .field("generation", generation)
                .field("current", current)
                .finish(),
            GcCommand::Barrier => f.write_str("Barrier"),
            GcCommand::Delegate => f.write_str("Delegate"),
            GcCommand::Remove { candidate, expect } => f
                .debug_struct("Remove")
                .field("candidate", candidate)
                .field("expect", expect)
                .finish(),
            GcCommand::Finish(outcome) => f.debug_tuple("Finish").field(outcome).finish(),
            GcCommand::Abort(refusal) => f.debug_tuple("Abort").field(refusal).finish(),
        }
    }
}

impl fmt::Debug for InventoryFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InventoryFacts")
            .field("recovery_pending", &self.recovery_pending)
            .field("current", &self.current)
            .field("generations", &self.generations)
            .finish()
    }
}

impl fmt::Debug for ManifestFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManifestFacts")
            .field("id", &self.id)
            .field("predecessors", &self.predecessors)
            .field("roots", &self.roots.len())
            .finish()
    }
}

impl fmt::Debug for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manifest")
            .field("facts", &self.facts)
            .field("fate", &self.fate)
            .finish()
    }
}

impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Candidate")
            .field("scope", &self.scope)
            .field("path", &alloc::string::String::from_utf8_lossy(&self.path))
            .field("stamp", &self.stamp)
            .finish()
    }
}

impl fmt::Debug for GcEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GcEvent::Start => f.write_str("Start"),
            GcEvent::Inventory(facts) => f.debug_tuple("Inventory").field(facts).finish(),
            GcEvent::Manifest(facts) => f.debug_tuple("Manifest").field(facts).finish(),
            GcEvent::Roots(roots) => f.debug_tuple("Roots").field(&roots.len()).finish(),
            GcEvent::Candidates(candidates) => {
                f.debug_tuple("Candidates").field(candidates).finish()
            }
            GcEvent::Done => f.write_str("Done"),
            GcEvent::Failed(failure) => f.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

impl fmt::Debug for GcState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcState")
            .field("phase", &self.phase)
            .field("mode", &self.mode)
            .field("keep", &self.keep)
            .field("inventory", &self.inventory)
            .field("prune", &self.prune)
            .field("manifests", &self.manifests)
            .field("doomed", &self.doomed)
            .field("index", &self.index)
            .finish()
    }
}

impl GcState {
    /// The generations this run retires, newest first (the order the
    /// machine issues `Retire` in).
    pub fn retired(&self) -> impl Iterator<Item = GenerationId> + '_ {
        self.manifests
            .iter()
            .filter(|manifest| !manifest.fate.retained())
            .map(|manifest| manifest.facts.id)
    }
}

// ---- managed blocks (`block.rs`) -------------------------------------------

use crate::block::{
    Block, BlockDecision, BlockRefusal, Malformation, Newline, ScanFailure, SpliceRefusal,
};

unit_debug!(
    Malformation {
        Metadata,
        Nested,
        Orphan,
        Interleaved,
        Unclosed
    },
    BlockRefusal {
        Foreign,
        Unrestorable
    },
    Newline { Lf, CrLf },
);

impl fmt::Debug for ScanFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScanFailure")
            .field("line", &self.line)
            .field("malformation", &self.malformation)
            .finish()
    }
}

impl fmt::Debug for SpliceRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpliceRefusal::Malformed(failure) => f.debug_tuple("Malformed").field(failure).finish(),
            SpliceRefusal::Unordered => f.write_str("Unordered"),
            SpliceRefusal::Unrecognized => f.write_str("Unrecognized"),
        }
    }
}

impl fmt::Debug for BlockDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockDecision::Write => f.write_str("Write"),
            BlockDecision::Satisfied => f.write_str("Satisfied"),
            BlockDecision::Preserve => f.write_str("Preserve"),
            BlockDecision::Remove => f.write_str("Remove"),
            BlockDecision::Keep => f.write_str("Keep"),
            BlockDecision::Absent => f.write_str("Absent"),
            BlockDecision::Refuse(why) => f.debug_tuple("Refuse").field(why).finish(),
        }
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Block")
            .field("span", &(self.start..self.end))
            .field("content", &(self.content_start..self.content_end))
            .field("sha", &self.sha)
            .field("mode", &self.mode)
            .finish()
    }
}

// ---- the activation machine and publication (plan/0056 F2 step 5) ----------

use crate::activation::{
    ActivationState, AttemptNumber, BatchFacts, Command as ActivationCommand,
    Event as ActivationEvent, HookResult, IntentState, Mode as ActivationMode,
    Outcome as ActivationOutcome, Phase as ActivationPhase, Refusal as ActivationRefusal,
    StartDecision,
};
use crate::publication::{Allocation, Commit, CommitFacts, CommitRefusal, FloorAction};

copy_impls!(
    AttemptNumber,
    IntentState,
    HookResult,
    StartDecision,
    BatchFacts,
    ActivationMode,
    ActivationOutcome,
    ActivationRefusal,
    ActivationCommand,
    ActivationEvent,
    ActivationPhase,
    ActivationState,
    Allocation,
    FloorAction,
    CommitFacts,
    CommitRefusal,
    Commit,
);

fn same_attempt(a: &Option<AttemptNumber>, b: &Option<AttemptNumber>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(*b),
        _ => false,
    }
}

same_eq!(
    AttemptNumber => |a, b| a.same(*b),
    IntentState => |a, b| match (a, b) {
        (IntentState::Pending, IntentState::Pending) => true,
        (IntentState::Started { attempt: a }, IntentState::Started { attempt: b })
        | (IntentState::Succeeded { attempt: a }, IntentState::Succeeded { attempt: b })
        | (IntentState::Failed { attempt: a }, IntentState::Failed { attempt: b }) => a.same(*b),
        (
            IntentState::Superseded { last_attempt: a },
            IntentState::Superseded { last_attempt: b },
        ) => same_attempt(a, b),
        _ => false,
    },
    HookResult => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    StartDecision => |a, b| match (a, b) {
        (StartDecision::Start(a), StartDecision::Start(b)) => a.same(*b),
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
    ActivationMode => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    ActivationOutcome => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    ActivationPhase => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    ActivationRefusal => |a, b| match (a, b) {
        (ActivationRefusal::Exhausted { intent: a }, ActivationRefusal::Exhausted { intent: b })
        | (ActivationRefusal::Drifted { intent: a }, ActivationRefusal::Drifted { intent: b }) => {
            a == b
        }
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
    ActivationCommand => |a, b| match (a, b) {
        (ActivationCommand::Authorize(a), ActivationCommand::Authorize(b)) => a == b,
        (ActivationCommand::ReadIntent(a), ActivationCommand::ReadIntent(b)) => a == b,
        (
            ActivationCommand::Persist { intent: a, state: x },
            ActivationCommand::Persist { intent: b, state: y },
        ) => a == b && x == y,
        (
            ActivationCommand::RunHook { intent: a, attempt: x, expect: s },
            ActivationCommand::RunHook { intent: b, attempt: y, expect: t },
        ) => a == b && x == y && s == t,
        (ActivationCommand::Finish(a), ActivationCommand::Finish(b)) => a == b,
        (ActivationCommand::Abort(a), ActivationCommand::Abort(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
    FloorAction => |a, b| match (a, b) {
        (FloorAction::Raise(a), FloorAction::Raise(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
    CommitRefusal => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    Commit => |a, b| match (a, b) {
        (Commit::Refuse(a), Commit::Refuse(b)) => a == b,
        _ => core::mem::discriminant(a) == core::mem::discriminant(b),
    },
);

impl fmt::Debug for AttemptNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

impl fmt::Display for AttemptNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}

impl fmt::Debug for IntentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntentState::Pending => f.write_str("Pending"),
            IntentState::Started { attempt } => write!(f, "Started({attempt:?})"),
            IntentState::Succeeded { attempt } => write!(f, "Succeeded({attempt:?})"),
            IntentState::Failed { attempt } => write!(f, "Failed({attempt:?})"),
            IntentState::Superseded { last_attempt } => {
                write!(f, "Superseded({last_attempt:?})")
            }
        }
    }
}

impl fmt::Debug for StartDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartDecision::Start(attempt) => write!(f, "Start({attempt:?})"),
            StartDecision::Terminal => f.write_str("Terminal"),
            StartDecision::Exhausted => f.write_str("Exhausted"),
        }
    }
}

unit_debug!(
    HookResult { Succeeded, Failed },
    ActivationMode { Run, Supersede },
    ActivationOutcome {
        Nothing,
        Activated,
        Superseded
    },
    ActivationPhase {
        Start,
        ReadingBatch,
        ReadingCurrent,
        Authorizing,
        ReadingIntent,
        Superseding,
        Starting,
        Running,
        Settling,
        Archiving,
        Clearing,
        Finished,
        Aborted
    },
    CommitRefusal {
        Reused,
        Misbound,
        BelowFloor,
        Exists
    },
);

impl fmt::Debug for ActivationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActivationRefusal::Exhausted { intent } => write!(f, "Exhausted({intent})"),
            ActivationRefusal::Drifted { intent } => write!(f, "Drifted({intent})"),
            ActivationRefusal::Malformed => f.write_str("Malformed"),
            ActivationRefusal::Failed => f.write_str("Failed"),
            ActivationRefusal::Protocol => f.write_str("Protocol"),
        }
    }
}

impl fmt::Debug for BatchFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BatchFacts")
            .field("selection", &self.selection)
            .field("intents", &self.intents)
            .finish()
    }
}

impl fmt::Debug for ActivationCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActivationCommand::ReadBatch => f.write_str("ReadBatch"),
            ActivationCommand::ReadCurrent => f.write_str("ReadCurrent"),
            ActivationCommand::Authorize(selection) => write!(f, "Authorize({selection:?})"),
            ActivationCommand::ReadIntent(intent) => write!(f, "ReadIntent({intent})"),
            ActivationCommand::Persist { intent, state } => {
                write!(f, "Persist({intent}, {state:?})")
            }
            ActivationCommand::RunHook {
                intent,
                attempt,
                expect,
            } => write!(f, "RunHook({intent}, {attempt:?}, {expect:?})"),
            ActivationCommand::Archive => f.write_str("Archive"),
            ActivationCommand::Clear => f.write_str("Clear"),
            ActivationCommand::Finish(outcome) => write!(f, "Finish({outcome:?})"),
            ActivationCommand::Abort(refusal) => write!(f, "Abort({refusal:?})"),
        }
    }
}

impl fmt::Debug for ActivationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActivationEvent::Start => f.write_str("Start"),
            ActivationEvent::NoBatch => f.write_str("NoBatch"),
            ActivationEvent::BatchRead(batch) => write!(f, "BatchRead({batch:?})"),
            ActivationEvent::CurrentRead(current) => write!(f, "CurrentRead({current:?})"),
            ActivationEvent::IntentRead(state) => write!(f, "IntentRead({state:?})"),
            ActivationEvent::Done => f.write_str("Done"),
            ActivationEvent::Returned(result) => write!(f, "Returned({result:?})"),
            ActivationEvent::Failed(failure) => write!(f, "Failed({failure:?})"),
        }
    }
}

impl fmt::Debug for ActivationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivationState")
            .field("phase", &self.phase)
            .field("mode", &self.mode)
            .field("batch", &self.batch)
            .field("current", &self.current)
            .field("index", &self.index)
            .field("record", &self.record)
            .finish()
    }
}

impl Default for ActivationState {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for FloorAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloorAction::Nothing => f.write_str("Nothing"),
            FloorAction::Seal => f.write_str("Seal"),
            FloorAction::Raise(floor) => write!(f, "Raise({floor:?})"),
        }
    }
}

impl fmt::Debug for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Commit::Vacuous => f.write_str("Vacuous"),
            Commit::Publish => f.write_str("Publish"),
            Commit::Refuse(why) => write!(f, "Refuse({why:?})"),
        }
    }
}
