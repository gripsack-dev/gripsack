//! Behaviour guards for the kernel's production callers. The guarantees
//! themselves are Lean theorems over the Charon/Aeneas translation
//! (proofs/Kernel/); these tests pin the concrete values callers rely on.
use gripsack_kernel::{
    Classification, GenerationId, GenerationList,
    journal::{PublicationAction, RecordPublication, RecordRole, admit_mutation},
    retention::{GcAdmission, admit_gc, plan_delete, plan_prune},
    selection::{RecoveryFacts, SelectionIdentity, TransactionId, classify},
};

fn list(ids: &[u64]) -> GenerationList {
    GenerationList::new(ids.iter().copied().map(GenerationId::new).collect()).unwrap()
}

#[test]
fn inventory_admits_only_strictly_ascending_identities() {
    for ids in [vec![], vec![0], vec![0, 1, u64::MAX]] {
        let expected: Vec<_> = ids.iter().copied().map(GenerationId::new).collect();
        assert_eq!(list(&ids).as_slice(), expected);
    }
    for ids in [[1, 1], [2, 1], [u64::MAX, 0]] {
        let ids = ids.map(GenerationId::new);
        assert!(GenerationList::new(ids.to_vec()).is_none());
    }
    assert_eq!(GenerationId::new(u64::MAX).checked_next(), None);
    assert_eq!(GenerationId::new(0).checked_previous(), None);
    assert_eq!(
        GenerationId::new(u64::MAX - 1).checked_next(),
        Some(GenerationId::new(u64::MAX))
    );
}

#[test]
fn exact_pruning_boundaries_preserve_current_without_shifting_the_prefix() {
    let inventory = list(&[0, 1, u64::MAX]);
    for (keep, current, expected) in [
        (None, None, vec![]),
        (Some(0), None, vec![0, 1, u64::MAX]),
        (Some(0), Some(1), vec![0, u64::MAX]),
        (Some(1), Some(0), vec![1]),
        (Some(2), Some(u64::MAX), vec![0]),
        (Some(u32::MAX), None, vec![]),
    ] {
        assert_eq!(
            plan_prune(&inventory, current.map(GenerationId::new), keep),
            expected
                .into_iter()
                .map(GenerationId::new)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        admit_gc(false, Some(GenerationId::new(2)), &inventory),
        GcAdmission::CorruptCurrent
    );
    assert_eq!(
        admit_gc(true, Some(GenerationId::new(2)), &inventory),
        GcAdmission::RecoveryPending
    );
    assert_eq!(
        admit_gc(false, Some(GenerationId::new(1)), &inventory),
        GcAdmission::Admitted
    );
    let empty = list(&[]);
    assert_eq!(admit_gc(false, None, &empty), GcAdmission::Admitted);
    assert!(plan_prune(&empty, None, Some(0)).is_empty());
}

#[test]
fn deletion_positions_are_the_unreferenced_candidates_in_order() {
    let referenced: Vec<&[u8]> = vec![b"a", b"c"];
    let candidates: Vec<&[u8]> = vec![b"c", b"b", b"a", b"", b"ab"];
    assert_eq!(plan_delete(&referenced, &candidates), vec![1, 3, 4]);
    assert_eq!(plan_delete(&[], &candidates), vec![0, 1, 2, 3, 4]);
    assert!(plan_delete(&candidates, &candidates).is_empty());
}

#[test]
fn classification_is_exact_identity() {
    let generation = GenerationId::new(7);
    let a = SelectionIdentity::transaction(generation, TransactionId::from_bytes([1; 32]));
    let b = SelectionIdentity::transaction(generation, TransactionId::from_bytes([2; 32]));
    let legacy = SelectionIdentity::legacy(generation);
    let facts = |previous, target, current| RecoveryFacts {
        previous,
        target,
        current,
    };
    assert_eq!(
        classify(facts(Some(a), b, Some(b))),
        Classification::Committed
    );
    assert_eq!(classify(facts(None, b, Some(b))), Classification::Committed);
    assert_eq!(
        classify(facts(Some(a), b, Some(a))),
        Classification::Uncommitted
    );
    assert_eq!(classify(facts(None, b, None)), Classification::Uncommitted);
    // same generation, different transaction bytes: never committed
    assert_eq!(classify(facts(None, b, Some(a))), Classification::Ambiguous);
    // legacy pointer versus transaction pointer of the same generation
    assert_eq!(
        classify(facts(None, b, Some(legacy))),
        Classification::Ambiguous
    );
    assert_eq!(classify(facts(Some(a), b, None)), Classification::Ambiguous);
    assert_eq!(
        classify(facts(Some(legacy), b, Some(a))),
        Classification::Ambiguous
    );
}

#[test]
fn publication_accepts_only_the_expected_effect_in_order() {
    let mut publication = RecordPublication::new(RecordRole::MutationEntry);
    for action in [
        PublicationAction::Stage,
        PublicationAction::SyncFile,
        PublicationAction::PublishName,
        PublicationAction::SyncNamespace,
    ] {
        assert_eq!(publication.action(), action);
        assert!(publication.acknowledge(action, true));
    }
    assert_eq!(publication.action(), PublicationAction::Complete);
    assert!(!publication.acknowledge(PublicationAction::Complete, true));
    assert!(publication.finish().is_none());

    let mut publication = RecordPublication::new(RecordRole::RunMarker);
    assert!(!publication.acknowledge(PublicationAction::SyncFile, true));
    assert_eq!(publication.action(), PublicationAction::Refused);
    assert!(!publication.acknowledge(PublicationAction::Refused, true));

    let mut publication = RecordPublication::new(RecordRole::RunMarker);
    assert!(!publication.acknowledge(PublicationAction::Stage, false));
    assert!(publication.finish().is_none());

    let mut marker = RecordPublication::new(RecordRole::RunMarker);
    let mut entry = RecordPublication::new(RecordRole::MutationEntry);
    for action in [
        PublicationAction::Stage,
        PublicationAction::SyncFile,
        PublicationAction::PublishName,
        PublicationAction::SyncNamespace,
    ] {
        assert!(marker.acknowledge(action, true));
        assert!(entry.acknowledge(action, true));
    }
    let marker = marker.finish().unwrap();
    let entry = entry.finish().unwrap();
    assert_eq!(marker.role(), RecordRole::RunMarker);
    assert!(admit_mutation(&marker, entry).is_some());
    let mut another = RecordPublication::new(RecordRole::RunMarker);
    for action in [
        PublicationAction::Stage,
        PublicationAction::SyncFile,
        PublicationAction::PublishName,
        PublicationAction::SyncNamespace,
    ] {
        assert!(another.acknowledge(action, true));
    }
    assert!(admit_mutation(&marker, another.finish().unwrap()).is_none());
}
