//! Exact lifecycle selection identity, separate from the selected generation.
//! The store admits pointer/marker bytes and allocates transaction identities;
//! this kernel compares only admitted values and never touches the filesystem.
use crate::{Classification, GenerationId};
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct TransactionId {
    bytes: [u8; 32],
}

impl TransactionId {
    pub closed spec fn view(self) -> Seq<u8> { self.bytes@ }

    pub fn from_bytes(bytes: [u8; 32]) -> (id: Self)
        ensures id.view() == bytes@,
    {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> (bytes: &[u8; 32])
        ensures bytes@ == self.view(),
    {
        &self.bytes
    }
}

/// Historical pointers remain generation-scoped. New pointers bind a unique
/// transaction even when rollback reactivates the already-current generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum SelectionIdentity {
    Legacy { generation: GenerationId },
    Transaction { generation: GenerationId, transaction: TransactionId },
}

impl SelectionIdentity {
    pub open spec fn generation_view(self) -> GenerationId {
        match self {
            Self::Legacy { generation } => generation,
            Self::Transaction { generation, .. } => generation,
        }
    }

    pub open spec fn transaction_view(self) -> Option<TransactionId> {
        match self {
            Self::Legacy { .. } => None,
            Self::Transaction { transaction, .. } => Some(transaction),
        }
    }

    pub fn legacy(generation: GenerationId) -> (identity: Self)
        ensures identity == (Self::Legacy { generation }),
    {
        Self::Legacy { generation }
    }

    pub fn transaction(generation: GenerationId, transaction: TransactionId) -> (identity: Self)
        ensures identity == (Self::Transaction { generation, transaction }),
    {
        Self::Transaction { generation, transaction }
    }

    pub fn generation(&self) -> (generation: GenerationId)
        ensures generation == self.generation_view(),
    {
        match self {
            Self::Legacy { generation } => *generation,
            Self::Transaction { generation, .. } => *generation,
        }
    }

    pub fn transaction_id(&self) -> (transaction: Option<&TransactionId>)
        ensures match transaction {
            None => self.transaction_view().is_none(),
            Some(id) => self.transaction_view() == Some(*id),
        },
    {
        match self {
            Self::Legacy { .. } => None,
            Self::Transaction { transaction, .. } => Some(transaction),
        }
    }
}

/// One observed transaction, with borrowed identity roles named explicitly.
pub struct RecoveryFacts<'a> {
    pub previous: Option<&'a SelectionIdentity>,
    pub target: &'a SelectionIdentity,
    pub current: Option<&'a SelectionIdentity>,
}

pub fn classify(facts: &RecoveryFacts<'_>) -> (result: Classification)
    ensures
        (result == Classification::Committed) <==> facts.current == Some(facts.target),
        (result == Classification::Uncommitted) <==> (
            (facts.current.is_none() && facts.previous.is_none())
            || (facts.previous.is_some() && facts.current == facts.previous
                && facts.current != Some(facts.target))
        ),
        (result == Classification::Ambiguous) <==> (
            facts.current != Some(facts.target)
            && !(facts.current.is_none() && facts.previous.is_none())
            && !(facts.previous.is_some() && facts.current == facts.previous)
        ),
{
    match (facts.previous, facts.current) {
        (Some(_), Some(current)) if current == facts.target => Classification::Committed,
        (Some(previous), Some(current)) if current == previous => Classification::Uncommitted,
        (Some(_), _) => Classification::Ambiguous,
        (None, Some(current)) if current == facts.target => Classification::Committed,
        (None, None) => Classification::Uncommitted,
        (None, Some(_)) => Classification::Ambiguous,
    }
}

}
