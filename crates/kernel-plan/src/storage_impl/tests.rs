// HOSTILE[P170][TEST-LOCAL][CLEAN:P169.T]: storage primitive invariants live with their owner.
use super::{OrderedPhysicalRowBucket, PersistentOrdMap, PhysicalRowId};

#[test]
fn ordered_index_bucket_preserves_logical_order_across_delete_and_slot_reuse() {
    let first = PhysicalRowId {
        slot: 4,
        generation: 0,
    };
    let removed = PhysicalRowId {
        slot: 1,
        generation: 0,
    };
    let third = PhysicalRowId {
        slot: 9,
        generation: 0,
    };
    let reused = PhysicalRowId {
        slot: 1,
        generation: 1,
    };
    let mut bucket = OrderedPhysicalRowBucket::default();
    bucket.push(first).unwrap();
    bucket.push(removed).unwrap();
    bucket.push(third).unwrap();
    assert!(bucket.remove(removed));
    bucket.push(reused).unwrap();

    assert_eq!(
        (&bucket).into_iter().copied().collect::<Vec<_>>(),
        vec![first, third, reused]
    );
    assert!(!bucket.contains(&removed));
    assert!(bucket.contains(&reused));
}

#[test]
fn ordered_physical_bucket_directory_path_copies_only_touched_bucket_state() {
    let mut buckets = PersistentOrdMap::<i64, OrderedPhysicalRowBucket>::default();
    for key in [7_i64, 11_i64] {
        let mut bucket = OrderedPhysicalRowBucket::default();
        for slot in 0..2048_usize {
            bucket
                .push(PhysicalRowId {
                    slot: slot + usize::try_from(key).unwrap() * 10_000,
                    generation: 0,
                })
                .unwrap();
        }
        buckets.insert(key, bucket);
    }

    let snapshot = buckets.clone();
    let removed = PhysicalRowId {
        slot: 7 * 10_000 + 1024,
        generation: 0,
    };
    let mut touched = buckets.get(&7).unwrap().clone();
    assert!(touched.remove(removed));
    buckets.insert(7, touched);

    assert!(!snapshot.shares_root_with(&buckets));
    let old_untouched = snapshot.get(&11).unwrap();
    let new_untouched = buckets.get(&11).unwrap();
    assert!(old_untouched.rows.shares_root_with(&new_untouched.rows));
    assert!(
        old_untouched
            .ordinals
            .shares_root_with(&new_untouched.ordinals)
    );
    assert!(snapshot.get(&7).unwrap().contains(&removed));
    assert!(!buckets.get(&7).unwrap().contains(&removed));
}
