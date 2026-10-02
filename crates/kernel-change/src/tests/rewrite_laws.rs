use super::*;

fn field(id: u128) -> SemanticWriteCoordinate {
    SemanticWriteCoordinate::ProductField(SemanticId(id))
}

#[test]
fn disjoint_product_fields_strongly_commute_without_cross_guards() {
    let left = RewriteFootprint {
        writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    let right = RewriteFootprint {
        writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    assert_eq!(
        infer_pair_rewrite_law(&left, &right),
        PairRewriteLaw::StrongCommute
    );
    assert_eq!(
        coordination_decision(infer_pair_rewrite_law(&left, &right)),
        PairCoordinationDecision::CoordinationFree
    );
}

#[test]
fn disjoint_writes_do_not_commute_when_guard_reads_other_write() {
    let left = RewriteFootprint {
        reads: [field(2)].into_iter().collect(),
        writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    let right = RewriteFootprint {
        writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    assert_eq!(
        infer_pair_rewrite_law(&left, &right),
        PairRewriteLaw::Unknown
    );
    assert_eq!(
        coordination_decision(infer_pair_rewrite_law(&left, &right)),
        PairCoordinationDecision::RequiresCoordination
    );
}

#[test]
fn identical_assignment_is_idempotent_but_different_assignment_conflicts() {
    let mk = |value| RewriteFootprint {
        writes: [(
            field(1),
            RewriteActionLaw::IdempotentAssign {
                semantic_value: SemanticId(value),
            },
        )]
        .into_iter()
        .collect(),
        ..RewriteFootprint::default()
    };
    assert_eq!(
        infer_pair_rewrite_law(&mk(7), &mk(7)),
        PairRewriteLaw::SameIdempotentIntent
    );
    assert_eq!(
        infer_pair_rewrite_law(&mk(7), &mk(8)),
        PairRewriteLaw::DefiniteIntentConflict
    );
    assert_eq!(
        coordination_decision(infer_pair_rewrite_law(&mk(7), &mk(8))),
        PairCoordinationDecision::IntentConflict
    );
}

#[test]
fn invariant_obligation_blocks_coordination_free_claim_until_vmf_discharge() {
    let left = RewriteFootprint {
        writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
        invariant_obligations: [SemanticId(99)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    let right = RewriteFootprint {
        writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
        ..RewriteFootprint::default()
    };
    assert_eq!(
        infer_pair_rewrite_law(&left, &right),
        PairRewriteLaw::Unknown
    );
}

fn coordination_spec(id: u128, footprint: RewriteFootprint) -> RewriteSpec {
    RewriteSpec {
        id: RewriteSpecId(SemanticId(id)),
        law_set: RewriteLawSetId(SemanticId(77)),
        footprint,
    }
}

fn opaque_write_footprint(coordinate: u128) -> RewriteFootprint {
    RewriteFootprint {
        writes: [(field(coordinate), RewriteActionLaw::Opaque)]
            .into_iter()
            .collect(),
        ..RewriteFootprint::default()
    }
}

fn assign_footprint(coordinate: u128, value: u128) -> RewriteFootprint {
    RewriteFootprint {
        writes: [(
            field(coordinate),
            RewriteActionLaw::IdempotentAssign {
                semantic_value: SemanticId(value),
            },
        )]
        .into_iter()
        .collect(),
        ..RewriteFootprint::default()
    }
}

#[test]
fn compiled_coordination_graph_is_complete_for_current_footprint_law() {
    let make_rewrite =
        |spec: &RewriteSpec| spec.prepare(Vec::<()>::new(), RewriteEffect::Replace(0_i32));

    let disjoint = coordination_spec(1, opaque_write_footprint(1));
    let guarded = coordination_spec(
        2,
        RewriteFootprint {
            reads: [field(1)].into_iter().collect(),
            ..opaque_write_footprint(2)
        },
    );
    let invariant = coordination_spec(
        3,
        RewriteFootprint {
            invariant_obligations: [SemanticId(9000)].into_iter().collect(),
            ..opaque_write_footprint(3)
        },
    );
    let assign_a = coordination_spec(4, assign_footprint(4, 1));
    let assign_b = coordination_spec(5, assign_footprint(4, 2));
    let same_assign = coordination_spec(6, assign_footprint(5, 7));
    let same_assign_2 = coordination_spec(7, same_assign.footprint.clone());

    let specs = [
        &disjoint,
        &guarded,
        &invariant,
        &assign_a,
        &assign_b,
        &same_assign,
        &same_assign_2,
    ];
    let mut registry = RewriteCoordinationRegistry::default();
    for spec in specs {
        registry.register(spec).unwrap();
    }
    let rewrites = [
        make_rewrite(&disjoint),
        make_rewrite(&guarded),
        make_rewrite(&invariant),
        make_rewrite(&assign_a),
        make_rewrite(&assign_b),
        make_rewrite(&same_assign),
        make_rewrite(&same_assign_2),
    ];
    let graph = registry.compile(rewrites.iter().enumerate()).unwrap();

    assert_eq!(
        graph.decision(0, 1),
        PairCoordinationDecision::RequiresCoordination
    );
    assert_eq!(
        graph.decision(0, 2),
        PairCoordinationDecision::RequiresCoordination
    );
    assert_eq!(
        graph.decision(3, 4),
        PairCoordinationDecision::IntentConflict
    );
    assert_eq!(
        graph.decision(5, 6),
        PairCoordinationDecision::CoordinationFree
    );
    assert_eq!(
        graph.decision(0, 5),
        PairCoordinationDecision::CoordinationFree
    );
}

#[test]
fn generic_write_action_maps_preserve_idempotent_presence_law() {
    use std::collections::BTreeMap;

    let left = BTreeMap::from([
        (1_u8, RewriteActionLaw::EnsurePresent),
        (2_u8, RewriteActionLaw::EnsurePresent),
    ]);
    let right = BTreeMap::from([
        (1_u8, RewriteActionLaw::EnsurePresent),
        (3_u8, RewriteActionLaw::EnsurePresent),
    ]);
    assert_eq!(
        infer_write_action_law(&left, &right),
        PairRewriteLaw::StrongCommute
    );

    let opposite = BTreeMap::from([(1_u8, RewriteActionLaw::EnsureAbsent)]);
    assert_eq!(
        infer_write_action_law(&left, &opposite),
        PairRewriteLaw::DefiniteIntentConflict
    );
}
