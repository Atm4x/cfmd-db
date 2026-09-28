use super::*;

fn coord(observable: u128, class: u128) -> SemanticClassCoordinate {
    SemanticClassCoordinate {
        observable: RevisionObservableId(observable),
        class: EqClassId(class),
    }
}

#[test]
fn set_change_addresses_semantic_classes_not_representatives() {
    let change = SemanticSetChange {
        observable: RevisionObservableId(1),
        inserted: [(EqClassId(3), "new")].into_iter().collect(),
        removed: [EqClassId(1)].into_iter().collect(),
    };
    let next = change
        .apply_classified([(coord(1, 1), "A"), (coord(1, 2), "B")])
        .unwrap();
    assert_eq!(
        next,
        [(EqClassId(2), "B"), (EqClassId(3), "new")]
            .into_iter()
            .collect()
    );
}

#[test]
fn bag_change_updates_multiplicity_per_semantic_class() {
    let change = SemanticBagChange {
        observable: RevisionObservableId(4),
        classes: [(
            EqClassId(7),
            SemanticBagClassChange {
                representative: None,
                inserted: 2,
                removed: 1,
            },
        )]
        .into_iter()
        .collect(),
    };
    let next = change.apply_classified([(coord(4, 7), "A", 3)]).unwrap();
    assert_eq!(next[&EqClassId(7)], ("A", 4));
}

#[test]
fn map_rejects_two_source_keys_in_the_same_semantic_class() {
    let change = SemanticMapChange::<&str, i64> {
        key_observable: RevisionObservableId(9),
        upserted: BTreeMap::new(),
        removed: BTreeSet::new(),
    };
    assert!(matches!(
        change.apply_classified([(coord(9, 2), "A", 1), (coord(9, 2), "a", 2),]),
        Err(SemanticCollectionChangeError::DuplicateMapKeyClass(_))
    ));
}
