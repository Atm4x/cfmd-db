const PAGE: usize = 256;
const BRANCH: usize = 32;

mod ordered;
pub use ordered::{
    PersistentOrdMap, PersistentOrdMapIter, PersistentOrdMapStorageProbe, PersistentOrdSet,
};

mod vector;
pub use vector::{PersistentVec, PersistentVecIter, PersistentVecStorageProbe};

#[cfg(test)]
mod tests {
    use super::{PAGE, PersistentOrdMap, PersistentVec};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct CloneCounted {
        value: u32,
        clones: Arc<AtomicUsize>,
    }

    impl Clone for CloneCounted {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::Relaxed);
            Self {
                value: self.value,
                clones: Arc::clone(&self.clones),
            }
        }
    }

    #[test]
    fn ordered_map_novel_insert_clones_only_search_path() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.insert(
            65_536,
            CloneCounted {
                value: 65_536,
                clones: Arc::clone(&clones),
            },
        );
        assert!(clones.load(Ordering::Relaxed) < 128);
        assert_eq!(snapshot.get(&65_536).map(|value| value.value), None);
        assert_eq!(map.get(&65_536).map(|value| value.value), Some(65_536));
    }

    #[test]
    fn ordered_map_get_mut_clones_only_search_path() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.get_mut(&32_768).unwrap().value = 7;
        assert!(clones.load(Ordering::Relaxed) < 128);
        assert_eq!(snapshot.get(&32_768).map(|value| value.value), Some(32_768));
        assert_eq!(map.get(&32_768).map(|value| value.value), Some(7));
    }

    #[test]
    fn ordered_map_path_copy_preserves_snapshot_and_order() {
        let mut original = PersistentOrdMap::default();
        for key in 0_u32..10_000 {
            original.insert(key, key * 2);
        }
        let snapshot = original.clone();
        assert!(original.shares_root_with(&snapshot));

        original.insert(10_000, 20_000);
        original.insert(5_000, 7);

        assert_eq!(snapshot.get(&10_000), None);
        assert_eq!(snapshot.get(&5_000), Some(&10_000));
        assert_eq!(original.get(&5_000), Some(&7));
        assert_eq!(snapshot.len(), 10_000);
        assert_eq!(original.len(), 10_001);
        assert!(!original.shares_root_with(&snapshot));
        assert!(snapshot.keys().copied().eq(0_u32..10_000));
    }

    #[test]
    fn ordered_map_remove_path_copies_and_preserves_snapshot() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(key, key * 3);
        }
        let snapshot = map.clone();
        assert_eq!(map.remove(&32_768), Some(98_304));
        assert_eq!(map.remove(&0), Some(0));
        assert_eq!(map.remove(&65_535), Some(196_605));
        assert_eq!(map.remove(&99_999), None);
        assert_eq!(snapshot.get(&32_768), Some(&98_304));
        assert_eq!(map.get(&32_768), None);
        assert_eq!(snapshot.len(), 65_536);
        assert_eq!(map.len(), 65_533);
        assert!(map.keys().copied().is_sorted());
        let height = map.root.as_ref().map_or(0, |root| root.height);
        assert!(
            height <= 24,
            "AVL height grew unexpectedly after removals: {height}"
        );
    }

    #[test]
    fn ordered_map_bidirectional_iteration_preserves_exact_order() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..10_000 {
            map.insert(key, key * 2);
        }
        assert!(map.keys().copied().eq(0_u32..10_000));
        assert!(map.keys().rev().copied().eq((0_u32..10_000).rev()));
        let mut iter = map.iter();
        assert_eq!(iter.next().map(|(key, _)| *key), Some(0));
        assert_eq!(iter.next_back().map(|(key, _)| *key), Some(9_999));
        assert_eq!(iter.len(), 9_998);
    }

    #[test]
    fn ordered_map_predecessor_and_successor_are_strict_and_logical() {
        let mut map = PersistentOrdMap::default();
        for key in [10_u32, 20, 40, 80] {
            map.insert(key, key * 10);
        }
        assert_eq!(map.predecessor(&10).map(|(key, _)| *key), None);
        assert_eq!(map.predecessor(&20).map(|(key, _)| *key), Some(10));
        assert_eq!(map.predecessor(&39).map(|(key, _)| *key), Some(20));
        assert_eq!(map.predecessor(&100).map(|(key, _)| *key), Some(80));
        assert_eq!(map.successor(&80).map(|(key, _)| *key), None);
        assert_eq!(map.successor(&40).map(|(key, _)| *key), Some(80));
        assert_eq!(map.successor(&21).map(|(key, _)| *key), Some(40));
        assert_eq!(map.successor(&0).map(|(key, _)| *key), Some(10));
    }

    #[test]
    fn ordered_map_balances_monotone_insertions() {
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..65_536 {
            map.insert(key, key);
        }
        let height = map.root.as_ref().map_or(0, |root| root.height);
        assert!(height <= 24, "AVL height grew unexpectedly: {height}");
    }

    #[test]
    fn remove_shifts_once_per_page_and_preserves_snapshot() {
        let original = PersistentVec::from_vec((0_u64..1_025).collect());
        let mut next = original.clone();

        assert_eq!(next.remove(300), 300);
        assert_eq!(next.len(), 1_024);
        assert_eq!(next[299], 299);
        assert_eq!(next[300], 301);
        assert_eq!(next[511], 512);
        assert_eq!(next[1_023], 1_024);

        assert_eq!(original.len(), 1_025);
        assert_eq!(original[255], 255);
        assert_eq!(original[1_024], 1_024);
        assert!(original.shares_page_with(&next, 0));
    }

    #[test]
    fn remove_handles_single_page_and_last_singleton_page() {
        let mut single_page = PersistentVec::from_vec((0_u64..16).collect());
        assert_eq!(single_page.remove(7), 7);
        assert!(single_page.iter().copied().eq((0_u64..7).chain(8..16)));

        let mut boundary = PersistentVec::from_vec((0_u64..=PAGE as u64).collect());
        assert_eq!(boundary.remove(PAGE - 1), (PAGE - 1) as u64);
        assert_eq!(boundary.len(), PAGE);
        assert_eq!(boundary[PAGE - 1], PAGE as u64);
    }

    #[test]
    fn bulk_resize_preserves_prefix_sharing_and_exact_contents() {
        let original = PersistentVec::from_vec((0_u64..1_024).collect());
        let mut shrunk = original.clone();
        shrunk.resize(300, 99);
        assert_eq!(shrunk.len(), 300);
        assert_eq!(shrunk[299], 299);
        assert!(original.shares_page_with(&shrunk, 0));

        shrunk.resize_with(900, || 7);
        assert_eq!(shrunk.len(), 900);
        assert_eq!(shrunk[299], 299);
        assert!(shrunk.iter().skip(300).all(|value| *value == 7));

        shrunk.resize(0, 0);
        assert!(shrunk.is_empty());
        shrunk.push(42);
        assert_eq!(shrunk.as_slice(), &[42]);
    }

    #[test]
    fn same_page_swap_and_swap_remove_preserve_semantics() {
        let original = PersistentVec::from_vec((0_u64..64).collect());
        let mut next = original.clone();
        next.swap(7, 31);
        assert_eq!(next[7], 31);
        assert_eq!(next[31], 7);
        assert_eq!(original[7], 7);

        assert_eq!(next.swap_remove(12), 12);
        assert_eq!(next.len(), 63);
        assert_eq!(next[12], 63);
        assert_eq!(original.len(), 64);
    }

    #[test]
    fn snapshot_mutation_preserves_old_root_contents() {
        let original = PersistentVec::from_vec((0_u64..10_000).collect());
        let mut next = original.clone();
        next.set(9_999, 77);
        assert_eq!(original[9_999], 9_999);
        assert_eq!(next[9_999], 77);
        assert_eq!(original[42], next[42]);
    }

    #[test]
    fn changed_indices_skips_shared_pages_and_reports_appends() {
        let original = PersistentVec::from_vec((0_u64..20_000).collect());
        let mut next = original.clone();
        next.set(17, 99);
        next.set(19_001, 101);
        next.push(20_000);
        assert_eq!(original.changed_indices(&next), vec![17, 19_001, 20_000]);
    }

    #[test]
    fn changed_indices_preserves_locality_across_radix_height_growth() {
        let original = PersistentVec::from_vec((0_u64..8_192).collect());
        let mut next = original.clone();
        next.push(8_192);
        assert_eq!(original.changed_indices(&next), vec![8_192]);
    }

    #[test]
    fn ordered_map_retain_all_preserves_root_without_cloning_values() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..4096 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.retain(|_, _| true);
        assert!(map.shares_root_with(&snapshot));
        assert_eq!(clones.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn ordered_map_bulk_retain_filters_in_one_tree_pass() {
        let clones = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentOrdMap::default();
        for key in 0_u32..4096 {
            map.insert(
                key,
                CloneCounted {
                    value: key,
                    clones: Arc::clone(&clones),
                },
            );
        }
        let snapshot = map.clone();
        clones.store(0, Ordering::Relaxed);
        map.retain(|key, _| key % 2 == 0);
        let clone_count = clones.load(Ordering::Relaxed);

        assert_eq!(map.len(), 2048);
        assert_eq!(snapshot.len(), 4096);
        assert!(
            map.iter()
                .all(|(key, value)| *key % 2 == 0 && value.value == *key)
        );
        assert!(snapshot.iter().all(|(key, value)| value.value == *key));
        let height = map.root.as_ref().map_or(0, |root| root.height);
        assert!(
            height <= 24,
            "AVL height grew unexpectedly after retain: {height}"
        );
        assert!(
            clone_count < 12_000,
            "bulk retain cloned {clone_count} values"
        );
    }
}
