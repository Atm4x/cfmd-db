use super::*;

#[test]
fn set_change_round_trips_and_adapts_to_universal_fine_change() {
    let old = BTreeSet::from([1, 2, 3]);
    let new = BTreeSet::from([2, 3, 4, 5]);
    let delta = SetChange::between(&old, &new);
    assert_eq!(delta.apply(&old), new);
    let universal = Change::Fine(delta.into_fine(&old));
    assert_eq!(universal.apply(&old), new);
}

#[test]
fn sequence_splice_has_exact_checked_semantics_and_adapter() {
    let splice = SeqSplice {
        start: 1,
        delete_count: 2,
        insert: vec![7, 8, 9],
    };
    let old = vec![1, 2, 3, 4];
    let expected = vec![1, 7, 8, 9, 4];
    assert_eq!(splice.apply(&old), Ok(expected.clone()));
    let universal = Change::Fine(splice.into_fine(&old).unwrap());
    assert_eq!(universal.apply(&old), expected);
}

#[test]
fn invalid_splice_is_not_silently_clamped() {
    let splice = SeqSplice {
        start: 2,
        delete_count: 5,
        insert: Vec::<i32>::new(),
    };
    assert_eq!(
        splice.apply(&[1, 2, 3]),
        Err(SeqChangeError::DeleteOutOfBounds)
    );
}

#[test]
fn change_composition_preserves_extensional_endpoint() {
    let first = Change::Fine(FineChange::new(FineChangeKind::Scalar, 2));
    let later = Change::Fine(FineChange::new(FineChangeKind::Scalar, 3));
    assert_eq!(first.compose(later).apply(&1), 3);
}

#[test]
fn prepared_rewrite_keeps_intent_separate_from_effect() {
    let rewrite = PreparedRewrite {
        spec: RewriteSpecId(SemanticId(11)),
        explicit_inputs: vec!["requested"],
        effect: RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, 42)),
        law_set: RewriteLawSetId(SemanticId(12)),
    };
    assert_eq!(rewrite.apply(&1), 42);
    assert_eq!(rewrite.spec, RewriteSpecId(SemanticId(11)));
    assert_eq!(rewrite.explicit_inputs, vec!["requested"]);
}

fn stable_snapshot() -> StableSeqSnapshot<i32> {
    StableSeqSnapshot {
        sequence: SemanticId(100),
        retained_anchor_histories: [SeqAnchorHistoryId(SemanticId(200))].into_iter().collect(),
        occurrences: vec![
            StableSeqOccurrence {
                id: SeqOccurrenceId(SemanticId(1)),
                value: 10,
            },
            StableSeqOccurrence {
                id: SeqOccurrenceId(SemanticId(2)),
                value: 20,
            },
            StableSeqOccurrence {
                id: SeqOccurrenceId(SemanticId(3)),
                value: 30,
            },
        ],
    }
}

fn middle_gap() -> StableSeqGapAnchor {
    StableSeqGapAnchor {
        id: SemanticId(300),
        history: SeqAnchorHistoryId(SemanticId(200)),
        left: Some(SeqOccurrenceId(SemanticId(2))),
        right: Some(SeqOccurrenceId(SemanticId(3))),
    }
}

#[test]
fn stable_seq_anchor_survives_snapshot_index_drift() {
    let intent = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    assert_eq!(stable_snapshot().resolve_intent(&intent).unwrap().start, 2);

    let mut shifted = stable_snapshot();
    shifted.occurrences.insert(
        0,
        StableSeqOccurrence {
            id: SeqOccurrenceId(SemanticId(9)),
            value: 90,
        },
    );
    assert_eq!(shifted.resolve_intent(&intent).unwrap().start, 3);
}

#[test]
fn stable_seq_resolution_fails_typed_for_missing_occurrence_and_expired_anchor() {
    let replace = StableSeqRewriteIntent::Replace {
        sequence: SemanticId(100),
        occurrence: SeqOccurrenceId(SemanticId(99)),
        value: 1,
    };
    assert_eq!(
        stable_snapshot().resolve_intent(&replace),
        Err(StableSeqRewriteError::MissingOccurrence(SeqOccurrenceId(
            SemanticId(99)
        )))
    );

    let mut anchor = middle_gap();
    anchor.history = SeqAnchorHistoryId(SemanticId(999));
    let insert = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor,
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    assert_eq!(
        stable_snapshot().resolve_intent(&insert),
        Err(StableSeqRewriteError::ExpiredAnchorHistory(
            SeqAnchorHistoryId(SemanticId(999))
        ))
    );
}

#[test]
fn stable_seq_pair_policy_requires_order_for_same_gap_and_conflicts_on_same_occurrence() {
    let insert = |occurrence, value| StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(occurrence)),
        value,
    };
    assert_eq!(
        classify_stable_seq_pair(&insert(4, 40), &insert(5, 50)),
        StableSeqPairDecision::SameGapOrderingRequired(middle_gap())
    );
    assert_eq!(
        infer_pair_rewrite_law(
            &insert(4, 40).rewrite_footprint(),
            &insert(5, 50).rewrite_footprint(),
        ),
        PairRewriteLaw::Unknown
    );

    let replace = StableSeqRewriteIntent::Replace {
        sequence: SemanticId(100),
        occurrence: SeqOccurrenceId(SemanticId(2)),
        value: 200,
    };
    let delete = StableSeqRewriteIntent::<i32>::Delete {
        sequence: SemanticId(100),
        occurrence: SeqOccurrenceId(SemanticId(2)),
    };
    assert_eq!(
        classify_stable_seq_pair(&replace, &delete),
        StableSeqPairDecision::ConflictingOccurrenceRewrite(SeqOccurrenceId(SemanticId(2)))
    );
}

#[test]
fn stable_seq_delete_of_anchor_endpoint_conflicts_with_concurrent_insert() {
    let delete = StableSeqRewriteIntent::<i32>::Delete {
        sequence: SemanticId(100),
        occurrence: SeqOccurrenceId(SemanticId(2)),
    };
    let insert = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    assert_eq!(
        classify_stable_seq_pair(&delete, &insert),
        StableSeqPairDecision::ConflictingOccurrenceRewrite(SeqOccurrenceId(SemanticId(2)))
    );
    assert_ne!(
        infer_pair_rewrite_law(&delete.rewrite_footprint(), &insert.rewrite_footprint()),
        PairRewriteLaw::StrongCommute
    );
}

#[test]
fn stable_seq_prepared_rewrite_preserves_anchored_intent_and_semantic_coordinates() {
    let intent = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(700)),
        law_set: RewriteLawSetId(SemanticId(701)),
        footprint: intent.rewrite_footprint(),
    };
    let prepared = spec
        .prepare_stable_seq(&stable_snapshot(), intent.clone())
        .unwrap();
    assert_eq!(prepared.spec, spec.id);
    assert_eq!(prepared.law_set, spec.law_set);
    assert_eq!(prepared.explicit_inputs, vec![intent.clone()]);
    assert_eq!(prepared.apply(&stable_snapshot().occurrences)[2].value, 40);
    assert_eq!(
        intent.write_coordinates(),
        [
            SemanticWriteCoordinate::SeqAnchor {
                sequence: SemanticId(100),
                anchor: SemanticId(300),
            },
            SemanticWriteCoordinate::SeqOccurrence {
                sequence: SemanticId(100),
                occurrence: SemanticId(4),
            },
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        classify_stable_seq_pair(
            &intent,
            &StableSeqRewriteIntent::Insert {
                sequence: SemanticId(100),
                anchor: middle_gap(),
                occurrence: SeqOccurrenceId(SemanticId(5)),
                value: 50,
            },
        )
        .coordination_decision(),
        PairCoordinationDecision::RequiresCoordination
    );
}

#[test]
fn stable_seq_structural_preparation_defers_full_endpoint_materialization() {
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Debug)]
    struct CountedValue {
        value: i32,
        clones: Rc<Cell<usize>>,
    }

    impl Clone for CountedValue {
        fn clone(&self) -> Self {
            self.clones.set(self.clones.get() + 1);
            Self {
                value: self.value,
                clones: Rc::clone(&self.clones),
            }
        }
    }

    impl PartialEq for CountedValue {
        fn eq(&self, other: &Self) -> bool {
            self.value == other.value
        }
    }

    impl Eq for CountedValue {}

    let clones = Rc::new(Cell::new(0));
    let snapshot = StableSeqSnapshot {
        sequence: SemanticId(800),
        retained_anchor_histories: BTreeSet::new(),
        occurrences: (0..64)
            .map(|index| StableSeqOccurrence {
                id: SeqOccurrenceId(SemanticId(1_000 + index)),
                value: CountedValue {
                    value: i32::try_from(index).unwrap(),
                    clones: Rc::clone(&clones),
                },
            })
            .collect(),
    };
    let intent = StableSeqRewriteIntent::Replace {
        sequence: snapshot.sequence,
        occurrence: SeqOccurrenceId(SemanticId(1_032)),
        value: CountedValue {
            value: 999,
            clones: Rc::clone(&clones),
        },
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(810)),
        law_set: RewriteLawSetId(SemanticId(811)),
        footprint: intent.rewrite_footprint(),
    };
    let prepared_snapshot = snapshot.prepare().unwrap();
    let structural = spec
        .prepare_stable_seq_structural_on(&prepared_snapshot, intent)
        .unwrap();

    assert_eq!(
        clones.get(),
        1,
        "preparation must clone only the replacement payload"
    );
    assert_eq!(structural.effect().start, 32);
    assert_eq!(structural.effect().delete_count, 1);
    assert_eq!(structural.effect().insert.len(), 1);

    let materialized = structural.materialize(&snapshot.occurrences).unwrap();
    assert_eq!(
        materialized.apply(&snapshot.occurrences)[32].value.value,
        999
    );
    assert!(
        clones.get() >= snapshot.occurrences.len(),
        "full endpoint cloning belongs to materialization, not structural preparation"
    );
}

#[test]
fn prepared_stable_seq_snapshot_rejects_duplicate_occurrence_identity() {
    let mut snapshot = stable_snapshot();
    snapshot.occurrences.push(StableSeqOccurrence {
        id: SeqOccurrenceId(SemanticId(2)),
        value: 999,
    });
    assert_eq!(
        snapshot.prepare().unwrap_err(),
        StableSeqRewriteError::DuplicateOccurrence(SeqOccurrenceId(SemanticId(2)))
    );
}

#[test]
fn prepared_stable_seq_snapshot_reuses_one_authoritative_index_across_specs() {
    let snapshot = stable_snapshot();
    let intents = [
        StableSeqRewriteIntent::Replace {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(2)),
            value: 200,
        },
        StableSeqRewriteIntent::Delete {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(3)),
        },
        StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor: middle_gap(),
            occurrence: SeqOccurrenceId(SemanticId(4)),
            value: 40,
        },
    ];

    let prepared_snapshot = snapshot.prepare().unwrap();
    let specs = intents
        .iter()
        .enumerate()
        .map(|(offset, intent)| RewriteSpec {
            id: RewriteSpecId(SemanticId(710 + offset as u128)),
            law_set: RewriteLawSetId(SemanticId(720 + offset as u128)),
            footprint: intent.rewrite_footprint(),
        })
        .collect::<Vec<_>>();
    let prepared = specs
        .iter()
        .zip(intents.iter().cloned())
        .map(|(spec, intent)| spec.prepare_stable_seq_on(&prepared_snapshot, intent))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(prepared.len(), 3);
    assert_eq!(prepared[0].explicit_inputs, vec![intents[0].clone()]);
    assert_eq!(prepared[1].explicit_inputs, vec![intents[1].clone()]);
    assert_eq!(prepared[2].explicit_inputs, vec![intents[2].clone()]);

    let replaced = prepared[0].apply(&snapshot.occurrences);
    assert_eq!(
        replaced.iter().map(|entry| entry.value).collect::<Vec<_>>(),
        vec![10, 200, 30]
    );
    let deleted = prepared[1].apply(&snapshot.occurrences);
    assert_eq!(
        deleted.iter().map(|entry| entry.value).collect::<Vec<_>>(),
        vec![10, 20]
    );
    let inserted = prepared[2].apply(&snapshot.occurrences);
    assert_eq!(
        inserted.iter().map(|entry| entry.value).collect::<Vec<_>>(),
        vec![10, 20, 40, 30]
    );
}

#[test]
fn prepared_stable_seq_snapshot_does_not_reinterpret_later_intent_against_earlier_endpoint() {
    let snapshot = stable_snapshot();
    let inserted = SeqOccurrenceId(SemanticId(4));
    let insert = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: inserted,
        value: 40,
    };
    let replace = StableSeqRewriteIntent::Replace {
        sequence: SemanticId(100),
        occurrence: inserted,
        value: 400,
    };
    let insert_spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(730)),
        law_set: RewriteLawSetId(SemanticId(731)),
        footprint: insert.rewrite_footprint(),
    };
    let replace_spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(732)),
        law_set: RewriteLawSetId(SemanticId(733)),
        footprint: replace.rewrite_footprint(),
    };
    let prepared_snapshot = snapshot.prepare().unwrap();
    insert_spec
        .prepare_stable_seq_on(&prepared_snapshot, insert)
        .unwrap();
    let result = replace_spec.prepare_stable_seq_on(&prepared_snapshot, replace);
    assert_eq!(
        result.unwrap_err(),
        StableSeqRewriteError::MissingOccurrence(inserted)
    );
}

#[test]
fn stable_seq_preparation_rejects_underdeclared_rewrite_footprint() {
    let intent = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(740)),
        law_set: RewriteLawSetId(SemanticId(741)),
        footprint: RewriteFootprint::default(),
    };
    assert_eq!(
        spec.prepare_stable_seq(&stable_snapshot(), intent),
        Err(StableSeqRewriteError::FootprintMismatch)
    );
}

#[test]
fn stable_seq_declared_footprints_keep_same_gap_in_compiled_coordination_graph() {
    let snapshot = stable_snapshot();
    let prepared_snapshot = snapshot.prepare().unwrap();
    let left_intent = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(4)),
        value: 40,
    };
    let right_intent = StableSeqRewriteIntent::Insert {
        sequence: SemanticId(100),
        anchor: middle_gap(),
        occurrence: SeqOccurrenceId(SemanticId(5)),
        value: 50,
    };
    let left_spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(750)),
        law_set: RewriteLawSetId(SemanticId(751)),
        footprint: left_intent.rewrite_footprint(),
    };
    let right_spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(752)),
        law_set: RewriteLawSetId(SemanticId(753)),
        footprint: right_intent.rewrite_footprint(),
    };
    let left = left_spec
        .prepare_stable_seq_on(&prepared_snapshot, left_intent)
        .unwrap();
    let right = right_spec
        .prepare_stable_seq_on(&prepared_snapshot, right_intent)
        .unwrap();
    let mut registry = RewriteCoordinationRegistry::default();
    registry.register(&left_spec).unwrap();
    registry.register(&right_spec).unwrap();
    let graph = registry.compile([(0_u8, &left), (1_u8, &right)]).unwrap();
    assert_eq!(
        graph.decision(0, 1),
        PairCoordinationDecision::RequiresCoordination
    );
}
