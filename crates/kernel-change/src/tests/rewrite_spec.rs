use super::*;

#[test]
fn rewrite_spec_is_authority_for_identity_law_set_and_footprint() {
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId(500)),
        law_set: RewriteLawSetId(SemanticId(501)),
        footprint: RewriteFootprint {
            writes: [(
                SemanticWriteCoordinate::ProductField(SemanticId(10)),
                RewriteActionLaw::IdempotentAssign {
                    semantic_value: SemanticId(700),
                },
            )]
            .into_iter()
            .collect(),
            ..RewriteFootprint::default()
        },
    };
    let prepared = spec.prepare(
        vec![SemanticId(900)],
        RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, 7_i64)),
    );
    assert_eq!(prepared.spec, spec.id);
    assert_eq!(prepared.law_set, spec.law_set);
    assert_eq!(prepared.explicit_inputs, vec![SemanticId(900)]);
    assert_eq!(
        spec.pair_law_with(&spec),
        PairRewriteLaw::SameIdempotentIntent
    );
}
