//! Journal effect ordering: one durable record is published through an exact
//! sequence of acknowledged effects. Concrete IO acknowledgements remain
//! external facts the store reports. The journal-run machine
//! (`journal_run.rs`) issues the marker's and every entry's publication in
//! this order and decides what may follow (F2 step 3).
//!
//! Proved (proofs/Kernel/Journal.lean): `RecordPublication::acknowledge`
//! accepts exactly the expected next action when it succeeded, advances to
//! its successor, and refuses forever otherwise; `finish` yields a record iff
//! the sequence completed.

pub enum RecordRole {
    RunMarker,
    MutationEntry,
}

impl RecordRole {
    pub fn same(self, other: RecordRole) -> bool {
        matches!(
            (self, other),
            (Self::RunMarker, Self::RunMarker) | (Self::MutationEntry, Self::MutationEntry)
        )
    }
}

pub enum PublicationAction {
    Stage,
    SyncFile,
    PublishName,
    SyncNamespace,
    Complete,
    Refused,
}

impl PublicationAction {
    pub fn same(self, other: PublicationAction) -> bool {
        matches!(
            (self, other),
            (Self::Stage, Self::Stage)
                | (Self::SyncFile, Self::SyncFile)
                | (Self::PublishName, Self::PublishName)
                | (Self::SyncNamespace, Self::SyncNamespace)
                | (Self::Complete, Self::Complete)
                | (Self::Refused, Self::Refused)
        )
    }

    /// The effect that must follow an acknowledged one: stage, sync the
    /// file, publish the name, sync the namespace, done. Nothing follows
    /// completion or refusal.
    pub fn successor(self) -> PublicationAction {
        match self {
            Self::Stage => Self::SyncFile,
            Self::SyncFile => Self::PublishName,
            Self::PublishName => Self::SyncNamespace,
            Self::SyncNamespace => Self::Complete,
            Self::Complete | Self::Refused => Self::Refused,
        }
    }
}

/// A record being published: its role and the next effect it expects.
pub struct RecordPublication {
    role: RecordRole,
    next: PublicationAction,
}

/// A record whose publication completed in order.
pub struct DurableRecord {
    role: RecordRole,
}

impl DurableRecord {
    pub fn role(&self) -> RecordRole {
        self.role
    }
}

impl RecordPublication {
    pub fn new(role: RecordRole) -> Self {
        Self {
            role,
            next: PublicationAction::Stage,
        }
    }

    pub fn role(&self) -> RecordRole {
        self.role
    }

    pub fn action(&self) -> PublicationAction {
        self.next
    }

    /// Record the outcome of one effect. Accepted iff it succeeded and was
    /// the expected next action other than completion or refusal; any other
    /// acknowledgement refuses the publication for good.
    pub fn acknowledge(&mut self, action: PublicationAction, succeeded: bool) -> bool {
        if !succeeded || !action.same(self.next) {
            self.next = PublicationAction::Refused;
            return false;
        }
        self.next = action.successor();
        !self.next.same(PublicationAction::Refused)
    }

    pub fn finish(self) -> Option<DurableRecord> {
        if self.next.same(PublicationAction::Complete) {
            Some(DurableRecord { role: self.role })
        } else {
            None
        }
    }
}
