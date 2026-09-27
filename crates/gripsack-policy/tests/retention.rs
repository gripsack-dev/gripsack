use gripsack_policy::{
    GenerationId, GenerationInventory, GenerationList,
    retention::{GcAdmission, admit_gc, plan_prune},
};

#[test]
fn inventory_admits_only_strictly_ascending_identities() {
    for ids in [vec![], vec![0], vec![0, 1, u64::MAX]] {
        let ids: Vec<_> = ids.into_iter().map(GenerationId::new).collect();
        assert_eq!(GenerationInventory::new(&ids).unwrap().as_slice(), ids);
        assert_eq!(
            GenerationList::new(ids.clone())
                .unwrap()
                .inventory()
                .as_slice(),
            ids
        );
    }
    for ids in [[1, 1], [2, 1], [u64::MAX, 0]] {
        let ids = ids.map(GenerationId::new);
        assert!(GenerationInventory::new(&ids).is_none());
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
    let ids = [0, 1, u64::MAX].map(GenerationId::new);
    let inventory = GenerationInventory::new(&ids).unwrap();
    for (keep, current, expected) in [
        (None, None, vec![]),
        (Some(0), None, vec![0, 1, u64::MAX]),
        (Some(0), Some(1), vec![0, u64::MAX]),
        (Some(1), Some(0), vec![1]),
        (Some(2), Some(u64::MAX), vec![0]),
        (Some(u32::MAX), None, vec![]),
    ] {
        assert_eq!(
            plan_prune(inventory, current.map(GenerationId::new), keep),
            expected
                .into_iter()
                .map(GenerationId::new)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        admit_gc(false, Some(GenerationId::new(2)), inventory),
        GcAdmission::CorruptCurrent
    );
    assert_eq!(
        admit_gc(true, Some(GenerationId::new(2)), inventory),
        GcAdmission::RecoveryPending
    );
    let empty = GenerationInventory::new(&[]).unwrap();
    assert_eq!(admit_gc(false, None, empty), GcAdmission::Admitted);
    assert!(plan_prune(empty, None, Some(0)).is_empty());
}
