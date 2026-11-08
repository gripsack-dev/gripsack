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
    journal::{DurableRecord, MutationAuthority, PublicationAction, RecordPublication, RecordRole},
    retention::GcAdmission,
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
    Classification,
    TransactionId,
    SelectionIdentity,
    RecoveryFacts,
    GcAdmission,
    RecordRole,
    PublicationAction,
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
    TransactionId => |a, b| a.same(b),
    SelectionIdentity => |a, b| a.same(b),
    RecordRole => |a, b| a.same(*b),
    PublicationAction => |a, b| a.same(*b),
    Classification => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
    GcAdmission => |a, b| core::mem::discriminant(a) == core::mem::discriminant(b),
);

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
);
