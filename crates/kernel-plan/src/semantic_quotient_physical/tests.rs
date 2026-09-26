use super::*;
use crate::physical_delta::PhysicalRelationDelta;

fn sid(value: u64) -> SemanticId {
    SemanticId::new(u128::from(value))
}

#[test]
fn gamma_quotient_leaf_factor_prunes_intra_row_coordinate_conflict() {
    let equivalence = sid(737);
    let mut cache = SemanticQuotientKeyCache::new();
    cache.insert(
        (0, 0, equivalence),
        vec![
            kernel_semantics::CanonicalEqKey::I64(1),
            kernel_semantics::CanonicalEqKey::I64(1),
        ],
    );
    cache.insert(
        (0, 1, equivalence),
        vec![
            kernel_semantics::CanonicalEqKey::I64(1),
            kernel_semantics::CanonicalEqKey::I64(2),
        ],
    );
    let leaf = quotient_leaf_keys(0, &[0, 1], equivalence, 2, &cache).unwrap();
    assert_eq!(
        leaf.keys,
        vec![Some(kernel_semantics::CanonicalEqKey::I64(1)), None]
    );
}

#[test]
fn gamma_quotient_support_fixpoint_propagates_across_coordinates() {
    let q1 = sid(740);
    let q2 = sid(741);
    let a = kernel_semantics::CanonicalEqKey::TextExact("a".into());
    let b = kernel_semantics::CanonicalEqKey::TextExact("b".into());
    let x = kernel_semantics::CanonicalEqKey::TextExact("x".into());
    let y = kernel_semantics::CanonicalEqKey::TextExact("y".into());
    let mut cache = SemanticQuotientKeyCache::new();
    cache.insert((0, 0, q1), vec![a.clone()]);
    cache.insert((1, 0, q1), vec![a, b]);
    cache.insert((1, 1, q2), vec![x.clone(), y]);
    cache.insert((2, 0, q2), vec![x]);
    let q1_leaves = vec![
        quotient_leaf_keys(0, &[0], q1, 1, &cache).unwrap(),
        quotient_leaf_keys(1, &[0], q1, 2, &cache).unwrap(),
    ];
    let q2_leaves = vec![
        quotient_leaf_keys(1, &[1], q2, 2, &cache).unwrap(),
        quotient_leaf_keys(2, &[0], q2, 1, &cache).unwrap(),
    ];
    let q1_support = quotient_key_leaf_support(&q1_leaves);
    let q2_support = quotient_key_leaf_support(&q2_leaves);
    let mut constraints = vec![
        SemanticQuotientConstraint {
            live_key_leaf_support: q1_support.clone(),
            key_leaf_support: q1_support,
            leaves: q1_leaves,
        },
        SemanticQuotientConstraint {
            live_key_leaf_support: q2_support.clone(),
            key_leaf_support: q2_support,
            leaves: q2_leaves,
        },
    ];
    let handles = vec![
        vec![PhysicalRowId {
            slot: 0,
            generation: 0,
        }],
        vec![
            PhysicalRowId {
                slot: 1,
                generation: 0,
            },
            PhysicalRowId {
                slot: 2,
                generation: 0,
            },
        ],
        vec![PhysicalRowId {
            slot: 3,
            generation: 0,
        }],
    ];
    let mut legacy_constraints = constraints.clone();
    let legacy_masks = quotient_support_masks_legacy(&handles, &mut legacy_constraints);
    let mut masks = quotient_support_masks(&handles, &mut constraints).unwrap();
    assert_eq!(masks, legacy_masks);
    assert!(mask_contains(&masks[0], 0));
    assert!(mask_contains(&masks[1], 0));
    assert!(!mask_contains(&masks[1], 1));
    assert!(mask_contains(&masks[2], 0));

    for constraint in &mut constraints {
        remove_live_ordinal_from_constraint(constraint, 0, 0);
    }
    assert!(mask_clear(&mut masks[0], 0));
    propagate_quotient_support_deletions(&handles, &mut constraints, &mut masks, &[0]);
    assert!(masks.iter().all(|mask| mask.iter().all(|word| *word == 0)));
}

#[test]
fn gamma_quotient_bfc_factors_duplicate_support_classes_linearly() {
    let equivalence = sid(7_490);
    let duplicate_rows = 4_096_usize;
    let key = kernel_semantics::CanonicalEqKey::I64(7);
    let mut cache = SemanticQuotientKeyCache::new();
    cache.insert(
        (0, 0, equivalence),
        std::iter::repeat_n(key.clone(), duplicate_rows).collect(),
    );
    cache.insert(
        (1, 0, equivalence),
        std::iter::repeat_n(key, duplicate_rows).collect(),
    );
    let leaves = vec![
        quotient_leaf_keys(0, &[0], equivalence, duplicate_rows, &cache).unwrap(),
        quotient_leaf_keys(1, &[0], equivalence, duplicate_rows, &cache).unwrap(),
    ];
    let key_leaf_support = quotient_key_leaf_support(&leaves);
    let constraints = [SemanticQuotientConstraint {
        leaves,
        live_key_leaf_support: key_leaf_support.clone(),
        key_leaf_support,
    }];
    let handles = (0..2)
        .map(|_| {
            (0..duplicate_rows)
                .map(|slot| PhysicalRowId {
                    slot,
                    generation: 0,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let handle_refs = handles.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let constraint_refs = constraints.iter().collect::<Vec<_>>();
    let (atoms_by_handle, mut atom_count) = semantic_quotient_initial_bfc_atoms(&handle_refs);
    let atom_refs = atoms_by_handle.iter().collect::<Vec<_>>();
    let mut group_atoms = BTreeMap::new();
    ensure_semantic_quotient_group_atoms(&constraint_refs, &mut group_atoms, &mut atom_count);
    let program = compile_semantic_quotient_bfc_program(
        &handle_refs,
        &constraint_refs,
        &atom_refs,
        &group_atoms,
        atom_count,
    )
    .unwrap();

    assert_eq!(group_atoms.len(), 2);
    assert_eq!(program.requirements().len(), duplicate_rows * 2 + 2);
    let supporter_cells = program
        .requirements()
        .iter()
        .map(|requirement| requirement.supporters.len())
        .sum::<usize>();
    assert_eq!(supporter_cells, duplicate_rows * 4);
    assert!(supporter_cells < duplicate_rows * duplicate_rows);
}

#[test]
fn quotient_key_support_counts_leaves_not_duplicate_rows() {
    let equivalence = sid(749);
    let a = kernel_semantics::CanonicalEqKey::TextExact("a".into());
    let b = kernel_semantics::CanonicalEqKey::TextExact("b".into());
    let c = kernel_semantics::CanonicalEqKey::TextExact("c".into());
    let mut cache = SemanticQuotientKeyCache::new();
    cache.insert((0, 0, equivalence), vec![a.clone(), a.clone(), b.clone()]);
    cache.insert((1, 0, equivalence), vec![a.clone(), b.clone(), b.clone()]);
    cache.insert((2, 0, equivalence), vec![a.clone(), c.clone()]);
    let leaves = vec![
        quotient_leaf_keys(0, &[0], equivalence, 3, &cache).unwrap(),
        quotient_leaf_keys(1, &[0], equivalence, 3, &cache).unwrap(),
        quotient_leaf_keys(2, &[0], equivalence, 2, &cache).unwrap(),
    ];
    let support = quotient_key_leaf_support(&leaves);
    assert_eq!(support.get(&a), Some(&3));
    assert_eq!(support.get(&b), Some(&2));
    assert_eq!(support.get(&c), Some(&1));

    let mut constraint = SemanticQuotientConstraint {
        leaves,
        live_key_leaf_support: support.clone(),
        key_leaf_support: support,
    };
    let mut masks = [
        full_ordinal_mask(3),
        full_ordinal_mask(3),
        full_ordinal_mask(2),
    ];
    assert_eq!(
        constraint_viable_keys(&constraint),
        BTreeSet::from([a.clone()])
    );
    remove_live_ordinal_from_constraint(&mut constraint, 0, 0);
    assert!(mask_clear(&mut masks[0], 0));
    assert_eq!(constraint.leaves[0].live_rows_by_key.get(&a), Some(&1));
    assert_eq!(constraint.live_key_leaf_support.get(&a), Some(&3));
    assert_eq!(
        constraint_viable_keys(&constraint),
        BTreeSet::from([a.clone()])
    );
    remove_live_ordinal_from_constraint(&mut constraint, 0, 1);
    assert!(mask_clear(&mut masks[0], 1));
    assert_eq!(constraint.leaves[0].live_rows_by_key.get(&a), None);
    assert_eq!(constraint.live_key_leaf_support.get(&a), Some(&2));
    assert!(constraint_viable_keys(&constraint).is_empty());
}

#[test]
fn qcn_stable_occurrence_coordinate_tombstones_reused_physical_slots() {
    let old = [0_usize, 1, 2].map(|slot| PhysicalRowId {
        slot,
        generation: 0,
    });
    let rows = StableSemanticQuotientRows::from_dense(&old);
    let replacement = PhysicalRowId {
        slot: 1,
        generation: 1,
    };
    let delta = PhysicalRelationDelta {
        removed: vec![(old[1], vec![])],
        inserted: vec![(replacement, vec![])],
    };
    let (next, inserted) = rows.apply_delta(&delta).unwrap();

    assert_eq!(inserted, vec![replacement]);
    assert_eq!(next.live_count, 3);
    assert_eq!(next.by_ordinal.len(), 4);
    assert_eq!(next.by_ordinal[1], None);
    assert_eq!(next.by_ordinal[3], Some(replacement));
    assert_eq!(
        next.dense_handles(),
        vec![old[0], old[2], replacement],
        "tombstones must preserve logical insertion order while slot reuse gets a fresh QCN ordinal"
    );
    assert_eq!(next.ordinal_by_slot[1], Some((1, 3)));
}

#[test]
fn qcn_stable_occurrence_coordinate_compaction_bounds_churn_history() {
    let live = 8_usize;
    let initial = (0..live)
        .map(|slot| PhysicalRowId {
            slot,
            generation: 0,
        })
        .collect::<Vec<_>>();
    let mut rows = StableSemanticQuotientRows::from_dense(&initial);
    let mut generations = vec![0_u64; live];
    for step in 0..(StableSemanticQuotientRows::COMPACTION_SLACK + live * 3) {
        let slot = step % live;
        let old = PhysicalRowId {
            slot,
            generation: generations[slot],
        };
        generations[slot] += 1;
        let new = PhysicalRowId {
            slot,
            generation: generations[slot],
        };
        let delta = PhysicalRelationDelta {
            removed: vec![(old, vec![])],
            inserted: vec![(new, vec![])],
        };
        let (next, _) = rows.apply_delta(&delta).unwrap();
        rows = if next.should_compact() {
            StableSemanticQuotientRows::from_dense(&next.dense_handles())
        } else {
            next
        };
    }
    assert_eq!(rows.live_count, live);
    assert!(
        rows.by_ordinal.len() <= live * 2 + StableSemanticQuotientRows::COMPACTION_SLACK,
        "reconstructible QCN occurrence history must remain bounded under slot reuse churn"
    );
}

#[test]
fn qcn_stable_key_buckets_follow_tombstones_without_dense_rebuild() {
    let key_a = kernel_semantics::CanonicalEqKey::I64(1);
    let key_b = kernel_semantics::CanonicalEqKey::I64(2);
    let mut keys = StableSemanticQuotientKeyState::from_dense(&[
        Some(key_a.clone()),
        Some(key_b.clone()),
        Some(key_a.clone()),
    ]);

    assert_eq!(keys.live_ordinals(&key_a).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(keys.remove_ordinal(0), Some(key_a.clone()));
    assert_eq!(keys.live_ordinals(&key_a).collect::<Vec<_>>(), vec![2]);
    keys.push_ordinal(3, Some(key_a.clone()));
    assert_eq!(keys.live_ordinals(&key_a).collect::<Vec<_>>(), vec![2, 3]);
    assert!(keys.contains_key(&key_b));
}

#[test]
fn qcn_stable_key_bucket_compaction_bounds_churn_history() {
    let key = kernel_semantics::CanonicalEqKey::I64(7);
    let mut keys = StableSemanticQuotientKeyState::from_dense(&[Some(key.clone())]);
    let churn = StableSemanticQuotientKeyState::BUCKET_COMPACTION_SLACK + 2_048;
    let mut live_ordinal = 0_usize;
    for ordinal in 1..=churn {
        assert_eq!(keys.remove_ordinal(live_ordinal), Some(key.clone()));
        keys.push_ordinal(ordinal, Some(key.clone()));
        live_ordinal = ordinal;
    }
    let bucket = keys.bucket_id_by_key[&key];
    assert_eq!(keys.buckets[bucket].live_count, 1);
    assert!(
        keys.buckets[bucket].ordinals.len()
            <= StableSemanticQuotientKeyState::BUCKET_COMPACTION_SLACK + 2
    );
}
