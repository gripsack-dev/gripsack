//! Journal effect ordering; concrete IO acknowledgements remain external facts.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum RecordRole { RunMarker, MutationEntry }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum PublicationAction { Stage, SyncFile, PublishName, SyncNamespace, Complete, Refused }

#[derive(Debug)]
pub struct RecordPublication { role: RecordRole, next: PublicationAction }
#[derive(Debug)]
pub struct DurableRecord { role: RecordRole }

impl DurableRecord {
    pub closed spec fn role(&self) -> RecordRole { self.role }
}

impl RecordPublication {
    pub closed spec fn role(&self) -> RecordRole { self.role }
    pub closed spec fn next_action(&self) -> PublicationAction { self.next }

    pub fn new(role: RecordRole) -> (publication: Self)
        ensures publication.role() == role, publication.next_action() == PublicationAction::Stage,
    { Self { role, next: PublicationAction::Stage } }

    pub fn action(&self) -> (action: PublicationAction)
        ensures action == self.next_action(), no_unwind
    { self.next }

    pub fn acknowledge(&mut self, action: PublicationAction, succeeded: bool) -> (accepted: bool)
        ensures
            final(self).role() == old(self).role(),
            accepted <==> succeeded && action == old(self).next_action()
                && action != PublicationAction::Complete && action != PublicationAction::Refused,
            final(self).next_action() == if !accepted { PublicationAction::Refused }
                else { match action {
                    PublicationAction::Stage => PublicationAction::SyncFile,
                    PublicationAction::SyncFile => PublicationAction::PublishName,
                    PublicationAction::PublishName => PublicationAction::SyncNamespace,
                    PublicationAction::SyncNamespace => PublicationAction::Complete,
                    _ => PublicationAction::Refused,
                } },
        no_unwind
    {
        if !succeeded || action != self.next {
            self.next = PublicationAction::Refused;
            return false;
        }
        self.next = match action {
            PublicationAction::Stage => PublicationAction::SyncFile,
            PublicationAction::SyncFile => PublicationAction::PublishName,
            PublicationAction::PublishName => PublicationAction::SyncNamespace,
            PublicationAction::SyncNamespace => PublicationAction::Complete,
            _ => PublicationAction::Refused,
        };
        self.next != PublicationAction::Refused
    }

    pub fn finish(self) -> (record: Option<DurableRecord>)
        ensures record.is_some() <==> self.next_action() == PublicationAction::Complete,
            record.is_some() ==> record.unwrap().role() == self.role(),
        no_unwind
    {
        if self.next == PublicationAction::Complete { Some(DurableRecord { role: self.role }) }
        else { None }
    }
}

#[derive(Debug)]
pub struct MutationAuthority<'a> {
    _marker: &'a DurableRecord,
    #[cfg(verus_keep_ghost)]
    ghost entry: DurableRecord,
}
impl<'a> MutationAuthority<'a> {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self._marker.role() == RecordRole::RunMarker && self.entry.role() == RecordRole::MutationEntry
    }
}

pub fn admit_mutation<'a>(marker: &'a DurableRecord, entry: DurableRecord) -> (authority: Option<MutationAuthority<'a>>)
    ensures authority.is_some() <==> marker.role() == RecordRole::RunMarker && entry.role() == RecordRole::MutationEntry,
    no_unwind
{
    if marker.role == RecordRole::RunMarker && entry.role == RecordRole::MutationEntry {
        Some(MutationAuthority {
            _marker: marker,
            #[cfg(verus_keep_ghost)]
            entry,
        })
    } else { None }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum CleanupScope { Uncommitted, Committed }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum CleanupAction {
    ReconcileEntry { index: usize },
    RemoveEntry { index: usize },
    SyncEntries,
    RemoveMarker,
    SyncMarker,
    Complete,
    Refused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
enum CleanupPhase { Entries, MarkerRemoval, MarkerBarrier, Complete, Refused }

pub struct CleanupProgress {
    total: usize,
    reconciled: usize,
    removed: usize,
    phase: CleanupPhase,
}
impl CleanupProgress {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self.removed <= self.reconciled <= self.total
            && (self.phase != CleanupPhase::Entries && self.phase != CleanupPhase::Refused ==>
                self.reconciled == self.total && self.removed == self.total)
    }

    pub closed spec fn total(&self) -> usize { self.total }
    pub closed spec fn reconciled(&self) -> usize { self.reconciled }
    pub closed spec fn removed(&self) -> usize { self.removed }
    pub closed spec fn refused(&self) -> bool { self.phase == CleanupPhase::Refused }
    pub closed spec fn expected_action(&self) -> CleanupAction {
        match self.phase {
            CleanupPhase::Entries => if self.reconciled < self.total {
                CleanupAction::ReconcileEntry { index: self.reconciled }
            } else if self.removed < self.total {
                CleanupAction::RemoveEntry { index: self.removed }
            } else { CleanupAction::SyncEntries },
            CleanupPhase::MarkerRemoval => CleanupAction::RemoveMarker,
            CleanupPhase::MarkerBarrier => CleanupAction::SyncMarker,
            CleanupPhase::Complete => CleanupAction::Complete,
            CleanupPhase::Refused => CleanupAction::Refused,
        }
    }

    pub fn new(scope: CleanupScope, total: usize) -> (cleanup: Self)
        ensures cleanup.total() == total, cleanup.removed() == 0,
            cleanup.reconciled() == if scope == CleanupScope::Committed { total as int } else { 0int },
            !cleanup.refused(),
            cleanup.expected_action() == if total == 0 { CleanupAction::SyncEntries }
                else if scope == CleanupScope::Uncommitted { CleanupAction::ReconcileEntry { index: 0 } }
                else { CleanupAction::RemoveEntry { index: 0 } },
    {
        Self { total, reconciled: if scope == CleanupScope::Committed { total } else { 0 },
            removed: 0, phase: CleanupPhase::Entries }
    }

    pub fn action(&self) -> (action: CleanupAction)
        ensures action == self.expected_action(),
        no_unwind
    {
        match self.phase {
            CleanupPhase::Entries => if self.reconciled < self.total {
                CleanupAction::ReconcileEntry { index: self.reconciled }
            } else if self.removed < self.total {
                CleanupAction::RemoveEntry { index: self.removed }
            } else { CleanupAction::SyncEntries },
            CleanupPhase::MarkerRemoval => CleanupAction::RemoveMarker,
            CleanupPhase::MarkerBarrier => CleanupAction::SyncMarker,
            CleanupPhase::Complete => CleanupAction::Complete,
            CleanupPhase::Refused => CleanupAction::Refused,
        }
    }

    pub fn acknowledge(&mut self, action: CleanupAction, succeeded: bool) -> (accepted: bool)
        ensures
            final(self).total() == old(self).total(),
            accepted <==> succeeded && action == old(self).expected_action()
                && action != CleanupAction::Complete && action != CleanupAction::Refused,
            final(self).refused() <==> !accepted,
            final(self).reconciled() == old(self).reconciled() + if accepted && (match action { CleanupAction::ReconcileEntry { .. } => true, _ => false }) { 1int } else { 0int },
            final(self).removed() == old(self).removed() + if accepted && (match action { CleanupAction::RemoveEntry { .. } => true, _ => false }) { 1int } else { 0int },
            final(self).expected_action() == if !accepted { CleanupAction::Refused } else {
                match action {
                    CleanupAction::ReconcileEntry { .. } =>
                        if old(self).reconciled() + 1 < old(self).total() {
                            CleanupAction::ReconcileEntry { index: (old(self).reconciled() + 1) as usize }
                        } else if old(self).removed() < old(self).total() {
                            CleanupAction::RemoveEntry { index: old(self).removed() }
                        } else { CleanupAction::SyncEntries },
                    CleanupAction::RemoveEntry { .. } =>
                        if old(self).removed() + 1 < old(self).total() {
                            CleanupAction::RemoveEntry { index: (old(self).removed() + 1) as usize }
                        } else { CleanupAction::SyncEntries },
                    CleanupAction::SyncEntries => CleanupAction::RemoveMarker,
                    CleanupAction::RemoveMarker => CleanupAction::SyncMarker,
                    CleanupAction::SyncMarker => CleanupAction::Complete,
                    _ => CleanupAction::Refused,
                }
            },
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if !succeeded || action != self.action() {
            self.phase = CleanupPhase::Refused;
            return false;
        }
        match action {
            CleanupAction::ReconcileEntry { index: _ } => { self.reconciled += 1; }
            CleanupAction::RemoveEntry { index: _ } => { self.removed += 1; }
            CleanupAction::SyncEntries => { self.phase = CleanupPhase::MarkerRemoval; }
            CleanupAction::RemoveMarker => { self.phase = CleanupPhase::MarkerBarrier; }
            CleanupAction::SyncMarker => { self.phase = CleanupPhase::Complete; }
            _ => { self.phase = CleanupPhase::Refused; return false; }
        }
        true
    }
}

}
