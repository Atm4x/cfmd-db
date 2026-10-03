use super::artifact_codec::{decode_physical_artifact_specs, encode_physical_artifact_specs};
use crate::binary_codec::{Cursor, push_len};
use crate::*;
use kernel_change::RevisionEffectId;
use kernel_model::Value;
use kernel_query::{AggregateSpec, OrderDirection, RelExpr};
use kernel_types::{ClientTransactionId, MaterializationId, RevisionId, SemanticId};
use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn current_test_transactions() -> BTreeMap<DurableTransactionKey, DurableCommittedTransaction> {
    [
        (
            DurableTransactionKey::new(IdempotencyEpoch::ZERO, ClientTransactionId::new(11)),
            DurableTransactionIntent::FullRevision {
                target_revision: RevisionId::new(12),
                encoded_target_revision: vec![1, 2, 3],
                materializations: None,
                semantic_modules: Vec::new(),
            },
        ),
        (
            DurableTransactionKey::new(IdempotencyEpoch::ZERO, ClientTransactionId::new(13)),
            DurableTransactionIntent::RelationData {
                source_revision: RevisionId::new(12),
                target_revision: RevisionId::new(14),
                semantic_revision: kernel_types::SemanticRevision::new(
                    kernel_types::SchemaRevisionId::new(3),
                    kernel_types::SemanticEnvId::new(4),
                ),
                relation_mutations: vec![crate::DurableRelationMutation {
                    relation: SemanticId::new(9),
                    inserted: vec![vec![kernel_model::Value::I64(5)]],
                    removed: Vec::new(),
                    object_field_writes: Vec::new(),
                    authorization: crate::DurableRelationAuthorization::default(),
                }],
                client_guard_digest: None,
                semantic_modules: Vec::new(),
            },
        ),
    ]
    .into_iter()
    .map(|(key, intent)| {
        let target = intent.target_revision();
        (
            key,
            DurableCommittedTransaction::from_descriptor_intent(target, &intent),
        )
    })
    .collect()
}

#[test]
fn metadata_roundtrip_covers_all_current_query_variants_and_transactions() {
    let eq = SemanticId::new(90);
    let left = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(SemanticId::new(1))),
        column: 0,
        value: Value::Text("A".into()),
        equivalence: eq,
    };
    let query = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Group {
            input: Box::new(RelExpr::Distinct {
                input: Box::new(RelExpr::FilterEqColumns {
                    input: Box::new(RelExpr::JoinEq {
                        left: Box::new(RelExpr::Project {
                            input: Box::new(left),
                            columns: vec![0, 1],
                        }),
                        right: Box::new(RelExpr::PromoteToBag(Box::new(RelExpr::Scan(
                            SemanticId::new(2),
                        )))),
                        left_column: 0,
                        right_column: 0,
                        equivalence: eq,
                    }),
                    left_column: 1,
                    right_column: 3,
                    equivalence: eq,
                }),
                column_equivalences: vec![eq, eq],
            }),
            group_columns: vec![0],
            group_equivalences: vec![eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: eq,
            },
        }),
        column: 0,
        ordering: SemanticId::new(91),
        direction: OrderDirection::Descending,
        k: 5,
    };
    let metadata = DurableStoreMetadata {
        external_freshness: None,
        current_idempotency_epoch: IdempotencyEpoch::ZERO,
        minimum_retry_epoch: IdempotencyEpoch::ZERO,
        materializations: vec![
            DurableMaterializationSpec {
                id: MaterializationId::new(7),
                query,
            },
            DurableMaterializationSpec {
                id: MaterializationId::new(8),
                query: RelExpr::Difference {
                    left: Box::new(RelExpr::Scan(SemanticId::new(3))),
                    right: Box::new(RelExpr::Scan(SemanticId::new(4))),
                },
            },
            DurableMaterializationSpec {
                id: MaterializationId::new(9),
                query: RelExpr::AntiJoin {
                    left: Box::new(RelExpr::Scan(SemanticId::new(5))),
                    right: Box::new(RelExpr::Scan(SemanticId::new(6))),
                    left_column: 0,
                    right_column: 1,
                    equivalence: eq,
                },
            },
        ],
        physical_artifacts: current_physical_artifact_specs(eq),
        migration_complements: Vec::new(),
        historical_epoch_anchors: BTreeMap::new(),
        committed_transactions: current_test_transactions(),
        semantic_modules: Vec::new(),
        ..DurableStoreMetadata::default()
    };
    assert_eq!(decode(&encode(&metadata).unwrap()).unwrap(), metadata);
}

#[test]
fn relation_rewrite_transaction_intent_roundtrips_in_metadata() {
    let transaction_id = ClientTransactionId::new(15);
    let intent = DurableTransactionIntent::RelationRewrite {
        source_revision: RevisionId::new(14),
        target_revision: RevisionId::new(16),
        semantic_revision: kernel_types::SemanticRevision::new(
            kernel_types::SchemaRevisionId::new(3),
            kernel_types::SemanticEnvId::new(4),
        ),
        relation_mutations: vec![crate::DurableRelationMutation {
            relation: SemanticId::new(10),
            inserted: vec![vec![kernel_model::Value::I64(6)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        rewrite_intents: vec![crate::DurableRelationRewriteIntent {
            relation: SemanticId::new(10),
            rewrite_spec: SemanticId::new(110),
            law_set: SemanticId::new(111),
        }],
        semantic_modules: Vec::new(),
    };
    let metadata = DurableStoreMetadata {
        external_freshness: None,
        current_idempotency_epoch: IdempotencyEpoch::ZERO,
        minimum_retry_epoch: IdempotencyEpoch::ZERO,
        materializations: Vec::new(),
        physical_artifacts: Vec::new(),
        artifact_cores: Vec::new(),
        migration_complements: Vec::new(),
        historical_epoch_anchors: BTreeMap::new(),
        committed_transactions: BTreeMap::from([(
            DurableTransactionKey::new(IdempotencyEpoch::ZERO, transaction_id),
            DurableCommittedTransaction::from_descriptor_intent(RevisionId::new(16), &intent),
        )]),
        semantic_modules: Vec::new(),
        next_revision_effect_id: 0,
        causal_coverage_root: Some(RevisionId::new(14)),
        revision_effects: BTreeMap::from([(
            RevisionEffectId(transaction_id.raw()),
            DurableRevisionEffectRecord {
                id: RevisionEffectId(transaction_id.raw()),
                prerequisites: BTreeSet::new(),
                transaction_epoch: IdempotencyEpoch::ZERO,
                transaction_id,
                change: crate::DurableRevisionChange::RelationData {
                    semantic_revision: kernel_types::SemanticRevision::new(
                        kernel_types::SchemaRevisionId::new(3),
                        kernel_types::SemanticEnvId::new(4),
                    ),
                    relation_mutations: vec![crate::DurableRelationMutation {
                        relation: SemanticId::new(10),
                        inserted: vec![vec![kernel_model::Value::I64(6)]],
                        removed: Vec::new(),
                        object_field_writes: Vec::new(),
                        authorization: crate::DurableRelationAuthorization::default(),
                    }],
                },
                intent,
                source_revision: RevisionId::new(14),
                target_revision: RevisionId::new(16),
            },
        )]),
        revision_effect_frontiers: BTreeMap::from([
            (RevisionId::new(14), BTreeSet::new()),
            (
                RevisionId::new(16),
                BTreeSet::from([RevisionEffectId(transaction_id.raw())]),
            ),
        ]),
        checkpoint_realization: None,
    };
    let bytes = encode(&metadata).unwrap();
    assert_eq!(decode(&bytes).unwrap(), metadata);
}

fn current_physical_artifact_specs(eq: SemanticId) -> Vec<DurablePhysicalArtifactSpec> {
    let mut specs = vec![
        DurablePhysicalArtifactSpec::RelationLayout {
            relation: SemanticId::new(1),
            layout_id: 0x1234,
            kind: DurableRelationLayoutKind::TypedColumnar,
        },
        DurablePhysicalArtifactSpec::I64Index {
            relation: SemanticId::new(1),
            key_column: 0,
            equivalence: eq,
            advisor_managed: false,
        },
        DurablePhysicalArtifactSpec::ObservableAtom {
            relation: SemanticId::new(1),
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: eq,
            }],
            advisor_managed: true,
        },
        DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            relation: SemanticId::new(2),
            key_parts: vec![DurableSemanticKeyPart {
                column: 1,
                equivalence: eq,
            }],
            advisor_managed: false,
        },
        DurablePhysicalArtifactSpec::SemanticStatistics {
            relation: SemanticId::new(2),
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: eq,
            }],
            advisor_managed: true,
        },
        DurablePhysicalArtifactSpec::ObservableAtom {
            relation: SemanticId::new(3),
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: eq,
            }],
            advisor_managed: false,
        },
    ];
    specs.sort();
    specs
}

#[test]
fn pre_release_physical_artifact_recipe_tags_are_rejected() {
    for old_tag in [1_u16, 2_u16] {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&old_tag.to_le_bytes());
        push_len(&mut bytes, 0).unwrap();
        assert_eq!(
            decode_physical_artifact_specs(&mut Cursor::new(&bytes)),
            Err("unsupported physical artifact recipe tag")
        );
    }
}

#[test]
fn physical_artifact_recipe_collapse_preserves_manual_pin() {
    let eq = SemanticId::new(78);
    let mut bytes = Vec::new();
    encode_physical_artifact_specs(
        &mut bytes,
        &[
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation: SemanticId::new(77),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: true,
            },
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation: SemanticId::new(77),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: false,
            },
        ],
    )
    .unwrap();
    let mut cursor = Cursor::new(&bytes);
    let decoded = decode_physical_artifact_specs(&mut cursor).unwrap();
    cursor.finish().unwrap();
    assert_eq!(
        decoded,
        vec![DurablePhysicalArtifactSpec::ObservableAtom {
            relation: SemanticId::new(77),
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: eq,
            }],
            advisor_managed: false,
        }]
    );
}

#[test]
fn i64_recipe_collapse_preserves_manual_pin() {
    let relation = SemanticId::new(79);
    let equivalence = SemanticId::new(80);
    let mut bytes = Vec::new();
    encode_physical_artifact_specs(
        &mut bytes,
        &[
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column: 2,
                equivalence,
                advisor_managed: true,
            },
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column: 2,
                equivalence,
                advisor_managed: false,
            },
        ],
    )
    .unwrap();
    let decoded = decode_physical_artifact_specs(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(
        decoded,
        vec![DurablePhysicalArtifactSpec::I64Index {
            relation,
            key_column: 2,
            equivalence,
            advisor_managed: false,
        }]
    );
}

#[test]
fn pre_release_metadata_v4_is_rejected() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    encode_materialization_specs(&mut bytes, &[]).unwrap();
    push_len(&mut bytes, 0).unwrap();
    push_len(&mut bytes, 0).unwrap();
    assert_eq!(
        decode(&bytes),
        Err("unsupported pre-release durable metadata format")
    );
}

#[test]
fn pre_release_metadata_v6_is_rejected() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&6_u16.to_le_bytes());
    encode_materialization_specs(&mut bytes, &[]).unwrap();
    encode_physical_artifact_specs(&mut bytes, &[]).unwrap();
    push_len(&mut bytes, 0).unwrap();
    push_len(&mut bytes, 0).unwrap();
    assert_eq!(
        decode(&bytes),
        Err("unsupported pre-release durable metadata format")
    );
}

#[test]
fn physical_artifact_recipe_tag_is_fail_closed() {
    let mut bytes = Vec::new();
    encode_physical_artifact_specs(&mut bytes, &[]).unwrap();
    bytes[..2].copy_from_slice(&(PHYSICAL_ARTIFACT_RECIPE_TAG + 1).to_le_bytes());
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(
        decode_physical_artifact_specs(&mut cursor),
        Err("unsupported physical artifact recipe tag")
    );
}

#[test]
fn metadata_decoder_accepts_bounded_reader_source_without_slice_materialization() {
    let metadata = DurableStoreMetadata::default();
    let bytes = encode(&metadata).unwrap();
    let mut reader = std::io::Cursor::new(bytes.as_slice());
    let decoded = decode_from_reader(&mut reader, bytes.len() as u64).unwrap();
    assert_eq!(decoded, metadata);
    assert_eq!(reader.position(), bytes.len() as u64);
}
