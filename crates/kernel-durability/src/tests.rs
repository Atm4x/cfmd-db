use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;

use super::*;
use crate::binary_codec::{
    Cursor, MAX_COLLECTION_LEN, encode_rows, push_len, push_u16, push_u32, push_u64, push_u128,
};
use crate::wal_frame::{RecordKind, encode_frame};
use crate::wal_payload::{decode_prepare_payload, encode_prepare_payload};
use kernel_lens::{ComplementCapsule, ComplementRetention, LensSpecId, SemanticManifestId};
use kernel_model::{DatabaseState, Value};
use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
use kernel_semantics::SemanticRegistry;
use kernel_types::{
    ClientTransactionId, EntityId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId,
};

fn descriptor(source: u64, target: u64, value: Value) -> DurableRevisionDescriptor {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(7)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(9)),
    };
    let target_revision = kernel_revision::Revision::build(
        RevisionId::new(target),
        &context,
        &registry,
        DatabaseState::default(),
    )
    .unwrap();
    DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(u128::from(target)),
        RevisionId::new(source),
        &target_revision,
        target_revision.semantic_revision(),
        vec![DurableRelationMutation {
            relation: SemanticId::new(11),
            inserted: vec![vec![value]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap()
}

#[test]
fn released_format_matrix_is_v1_read_write_only() {
    assert_eq!(FORMAT_VERSION, 1);
    assert_eq!(
        FORMAT_COMPATIBILITY,
        [DurableFormatCompatibility {
            version: 1,
            readable: true,
            writable: true,
        }]
    );
    assert_eq!(durable_format_support(1), DurableFormatSupport::ReadWrite);
    assert_eq!(
        durable_format_support(0),
        DurableFormatSupport::UnsupportedOlder
    );
    assert_eq!(
        durable_format_support(2),
        DurableFormatSupport::UnsupportedNewer
    );
}

#[test]
fn pre_release_checkpoint_codec_versions_are_not_compatibility_surface() {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(70)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(90)),
    };
    let revision = kernel_revision::Revision::build(
        RevisionId::new(1),
        &context,
        &registry,
        DatabaseState::default(),
    )
    .unwrap();
    let mut bytes = crate::checkpoint::encode_revision(&revision).unwrap();
    bytes[..2].copy_from_slice(&6_u16.to_le_bytes());
    assert!(matches!(
        crate::checkpoint::decode_revision(&bytes, &registry),
        Err(DurabilityError::Corruption {
            reason: "unsupported checkpoint codec version",
            ..
        })
    ));
}

#[test]
fn client_intent_identity_survives_revision_recertification() {
    let first = descriptor(1, 2, Value::I64(7));
    let rebased = descriptor(9, 10, Value::I64(7));
    let different = descriptor(9, 10, Value::I64(8));

    assert_ne!(first.intent, rebased.intent);
    assert!(first.intent.same_client_intent(&rebased.intent));
    assert!(!first.intent.same_client_intent(&different.intent));
}

#[test]
fn crc32c_known_vector() {
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
}

#[test]
fn hostile_declared_collection_length_does_not_drive_preallocation_past_payload() {
    let mut bytes = Vec::new();
    push_u32(&mut bytes, u32::try_from(MAX_COLLECTION_LEN).unwrap());
    bytes.push(0);
    let mut cursor = Cursor::new(&bytes);
    let count = cursor.len().unwrap();
    assert_eq!(count, MAX_COLLECTION_LEN);
    assert_eq!(cursor.bounded_capacity(count), 1);
}

#[test]
fn value_codec_roundtrips_all_shapes() {
    let values = vec![
        Value::Unit,
        Value::Bool(true),
        Value::I64(-7),
        Value::F64Bits(f64::NAN.to_bits()),
        Value::Text("Aßz".into()),
        Value::LiveEntityRef {
            entity_type: SemanticId::new(2),
            id: EntityId::new(3),
        },
        Value::HistoricalEntityId {
            entity_type: SemanticId::new(4),
            id: EntityId::new(5),
        },
        Value::Product(BTreeMap::from([(
            SemanticId::new(6),
            Value::Option(Some(Box::new(Value::I64(8)))),
        )])),
        Value::Variant {
            tag: SemanticId::new(9),
            value: Box::new(Value::Seq(vec![Value::Bool(false)])),
        },
        Value::Set {
            equivalence: SemanticId::new(10),
            elements: vec![Value::Text("x".into())],
        },
        Value::Bag {
            equivalence: SemanticId::new(12),
            entries: vec![(Value::I64(4), 3)],
        },
        Value::Map {
            key_equivalence: SemanticId::new(13),
            entries: vec![(Value::Text("k".into()), Value::I64(1))],
        },
    ];
    for value in values {
        let descriptor = descriptor(1, 2, value);
        let payload = encode_prepare_payload(&descriptor).unwrap();
        assert_eq!(
            decode_prepare_payload(RevisionId::new(2), &payload).unwrap(),
            descriptor
        );
    }
}

#[test]
fn relation_data_intent_and_prepare_scale_with_delta_not_target_snapshot() {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(70)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(80)),
    };
    let mut state = DatabaseState::default();
    for raw in 1..=5_000_u128 {
        let entity = EntityId::new(raw);
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
    }
    let target =
        kernel_revision::Revision::build(RevisionId::new(2), &context, &registry, state).unwrap();
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(0x700),
        RevisionId::new(1),
        &target,
        target.semantic_revision(),
        vec![DurableRelationMutation {
            relation: SemanticId::new(11),
            inserted: vec![vec![Value::I64(7)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let full_revision = checkpoint::encode_revision(&target).unwrap();
    let prepare = encode_prepare_payload(&descriptor).unwrap();
    let metadata_bytes = metadata::encode(&metadata::DurableStoreMetadata {
        external_freshness: None,
        current_idempotency_epoch: IdempotencyEpoch::ZERO,
        minimum_retry_epoch: IdempotencyEpoch::ZERO,
        materializations: Vec::new(),
        physical_artifacts: Vec::new(),
        artifact_cores: Vec::new(),
        migration_complements: Vec::new(),
        historical_epoch_anchors: BTreeMap::new(),
        committed_transactions: BTreeMap::from([(
            DurableTransactionKey::new(IdempotencyEpoch::ZERO, descriptor.transaction_id),
            DurableCommittedTransaction::from_descriptor_intent(
                descriptor.target_revision,
                &descriptor.intent,
            ),
        )]),
        semantic_modules: Vec::new(),
        next_revision_effect_id: 0,
        causal_coverage_root: None,
        revision_effects: BTreeMap::new(),
        revision_effect_frontiers: BTreeMap::new(),
        checkpoint_realization: None,
    })
    .unwrap();

    assert!(matches!(
        descriptor.intent,
        DurableTransactionIntent::RelationData { .. }
    ));
    assert!(
        prepare.len() * 100 < full_revision.len(),
        "delta prepare={} full revision={}",
        prepare.len(),
        full_revision.len()
    );
    assert!(
        metadata_bytes.len() * 100 < full_revision.len(),
        "delta ledger={} full revision={}",
        metadata_bytes.len(),
        full_revision.len()
    );
}

#[test]
fn schema_migration_prepare_carries_program_not_target_snapshot() {
    let registry = SemanticRegistry::default();
    let target_context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(381)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
    };
    let mut state = DatabaseState::default();
    for raw in 1..=5_000_u128 {
        let entity = EntityId::new(raw);
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
    }
    let target =
        kernel_revision::Revision::build(RevisionId::new(2), &target_context, &registry, state)
            .unwrap();
    let program =
        kernel_transport::SchemaMigrationProgram::new(target_context, Vec::new(), Vec::new());
    let complement = DurableMigrationComplement::from_capsule(
        ComplementCapsule {
            source_schema: SchemaRevisionId::new(380),
            target_schema: SchemaRevisionId::new(381),
            lens_spec: LensSpecId(SemanticId::new(380_381)),
            semantic_pins: SemanticManifestId(SemanticId::new(0xCF_4D38_0381)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        ComplementRetention::Forget,
    );
    let descriptor = DurableRevisionDescriptor::schema_migration_program(
        ClientTransactionId::new(0x380_381),
        RevisionId::new(1),
        target.id(),
        program,
        complement,
        &registry,
    )
    .unwrap();
    let prepare = encode_prepare_payload(&descriptor).unwrap();
    let full_revision = checkpoint::encode_revision(&target).unwrap();
    let decoded = decode_prepare_payload(target.id(), &prepare).unwrap();

    assert_eq!(decoded, descriptor);
    assert!(matches!(
        descriptor.change,
        DurableRevisionChange::SchemaMigration { .. }
    ));
    assert!(
        prepare.len() * 100 < full_revision.len(),
        "migration program prepare={} full target={}",
        prepare.len(),
        full_revision.len()
    );
}

#[test]
fn mixed_revision_intent_roundtrips_compactly_and_reconstructs_exact_target() {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(170)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(180)),
    };
    let mut source_state = DatabaseState::default();
    for raw in 1..=5_000_u128 {
        let entity = EntityId::new(raw);
        source_state.lifecycle.entities.insert(entity);
        source_state.lifecycle.roots.insert(entity);
    }
    let source =
        kernel_revision::Revision::build(RevisionId::new(1), &context, &registry, source_state)
            .unwrap();
    let mut target_state = source.state().clone();
    let added = EntityId::new(5_001);
    target_state.lifecycle.entities.insert(added);
    target_state.lifecycle.roots.insert(added);
    let target =
        kernel_revision::Revision::build(RevisionId::new(2), &context, &registry, target_state)
            .unwrap();

    let model_delta = DurableModelDelta::between(source.state(), target.state());
    let model_complement = DurableModelDelta::between(target.state(), source.state());
    let descriptor = DurableRevisionDescriptor::mixed_revision(
        ClientTransactionId::new(0x1700),
        source.id(),
        &target,
        target.semantic_revision(),
        Vec::new(),
        model_delta.clone(),
        model_complement,
        &registry,
    )
    .unwrap();
    let prepare = encode_prepare_payload(&descriptor).unwrap();
    assert_eq!(
        decode_prepare_payload(target.id(), &prepare).unwrap(),
        descriptor
    );

    let mut reconstructed_state = source.state().clone();
    model_delta.apply_to(&mut reconstructed_state);
    let reconstructed = kernel_revision::Revision::build(
        target.id(),
        source.semantic_context(),
        &registry,
        reconstructed_state,
    )
    .unwrap();
    assert_eq!(reconstructed, target);

    let full_revision = checkpoint::encode_revision(&target).unwrap();
    assert!(
        prepare.len() * 50 < full_revision.len(),
        "mixed prepare={} full revision={}",
        prepare.len(),
        full_revision.len()
    );

    let metadata_value = metadata::DurableStoreMetadata {
        external_freshness: None,
        current_idempotency_epoch: IdempotencyEpoch::ZERO,
        minimum_retry_epoch: IdempotencyEpoch::ZERO,
        materializations: Vec::new(),
        physical_artifacts: Vec::new(),
        artifact_cores: Vec::new(),
        migration_complements: Vec::new(),
        historical_epoch_anchors: BTreeMap::new(),
        committed_transactions: BTreeMap::from([(
            DurableTransactionKey::new(IdempotencyEpoch::ZERO, descriptor.transaction_id),
            DurableCommittedTransaction::from_descriptor_intent(
                descriptor.target_revision,
                &descriptor.intent,
            ),
        )]),
        semantic_modules: Vec::new(),
        next_revision_effect_id: 0,
        causal_coverage_root: None,
        revision_effects: BTreeMap::new(),
        revision_effect_frontiers: BTreeMap::new(),
        checkpoint_realization: None,
    };
    let encoded_metadata = metadata::encode(&metadata_value).unwrap();
    assert_eq!(metadata::decode(&encoded_metadata).unwrap(), metadata_value);
}

#[test]
fn relation_rewrite_prepare_roundtrips_and_old_relation_data_format_is_rejected() {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(71)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(81)),
    };
    let target = kernel_revision::Revision::build(
        RevisionId::new(2),
        &context,
        &registry,
        DatabaseState::default(),
    )
    .unwrap();
    let mutation = DurableRelationMutation {
        relation: SemanticId::new(11),
        inserted: vec![vec![Value::I64(7)]],
        removed: Vec::new(),
        object_field_writes: Vec::new(),
        authorization: crate::DurableRelationAuthorization::default(),
    };
    let rewrite = DurableRevisionDescriptor::relation_rewrites(
        ClientTransactionId::new(0x701),
        RevisionId::new(1),
        &target,
        target.semantic_revision(),
        vec![mutation.clone()],
        vec![DurableRelationRewriteIntent {
            relation: mutation.relation,
            rewrite_spec: SemanticId::new(0xAA),
            law_set: SemanticId::new(0xBB),
        }],
        &registry,
    )
    .unwrap();
    let payload = encode_prepare_payload(&rewrite).unwrap();
    assert_eq!(
        decode_prepare_payload(target.id(), &payload).unwrap(),
        rewrite
    );

    let resolution = DurableRevisionDescriptor::relation_resolution(
        ClientTransactionId::new(0x703),
        RevisionId::new(1),
        &target,
        target.semantic_revision(),
        DurableRelationResolution {
            relation_mutations: vec![DurableRelationMutation {
                relation: SemanticId::new(11),
                inserted: vec![vec![Value::I64(8)]],
                removed: Vec::new(),
                object_field_writes: Vec::new(),
                authorization: crate::DurableRelationAuthorization::default(),
            }],
            rewrite_intents: vec![DurableRelationRewriteIntent {
                relation: SemanticId::new(11),
                rewrite_spec: SemanticId::new(0xCC),
                law_set: SemanticId::new(0xDD),
            }],
            causal_parents: vec![RevisionId::new(1), RevisionId::new(7)],
        },
        &registry,
    )
    .unwrap();
    let payload = encode_prepare_payload(&resolution).unwrap();
    assert_eq!(
        decode_prepare_payload(target.id(), &payload).unwrap(),
        resolution
    );
}

#[test]
fn full_revision_prepare_payload_roundtrips_and_revalidates_target() {
    let registry = SemanticRegistry::default();
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(50)),
        environment: SemanticEnvironment::new(SemanticEnvId::new(60)),
    };
    let target = kernel_revision::Revision::build(
        RevisionId::new(2),
        &context,
        &registry,
        DatabaseState::default(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::full_revision(
        ClientTransactionId::new(900),
        RevisionId::new(1),
        &target,
        &registry,
    )
    .unwrap();
    let payload = encode_prepare_payload(&descriptor).unwrap();
    let decoded = decode_prepare_payload(RevisionId::new(2), &payload).unwrap();
    assert_eq!(decoded, descriptor);
    assert_eq!(
        decoded.decode_full_revision(&registry).unwrap(),
        Some(target)
    );
}

#[test]
fn pre_release_mutation_format_v2_is_rejected() {
    let expected = descriptor(7, 8, Value::I64(9));
    let DurableRevisionChange::RelationData {
        semantic_revision,
        relation_mutations,
    } = &expected.change
    else {
        unreachable!();
    };
    let mut payload = Vec::new();
    push_u16(&mut payload, 2);
    push_u128(&mut payload, expected.transaction_id.raw());
    push_u64(&mut payload, expected.source_revision.raw());
    push_u64(&mut payload, semantic_revision.schema.raw());
    push_u64(&mut payload, semantic_revision.environment.raw());
    push_len(&mut payload, relation_mutations.len()).unwrap();
    for mutation in relation_mutations {
        push_u128(&mut payload, mutation.relation.raw());
        encode_rows(&mut payload, &mutation.inserted).unwrap();
        encode_rows(&mut payload, &mutation.removed).unwrap();
    }
    assert_eq!(
        decode_prepare_payload(expected.target_revision, &payload),
        Err("unsupported pre-release mutation payload format")
    );
}

#[test]
fn every_prefix_exposes_only_durable_commits() {
    let mut wal = SimulatedRevisionWal::new();
    let d1 = descriptor(10, 20, Value::I64(1));
    let p1 = wal.durably_prepare(&d1).unwrap();
    wal.durably_commit(p1).unwrap();
    let first_end = wal.crash_image().len();
    let d2 = descriptor(20, 30, Value::I64(2));
    let p2 = wal.durably_prepare(&d2).unwrap();
    wal.durably_commit(p2).unwrap();
    let second_end = wal.crash_image().len();
    let bytes = wal.crash_image().to_vec();
    for cut in 0..=bytes.len() {
        let scan = scan_wal(&bytes[..cut], RevisionId::new(10)).unwrap();
        let expected = if cut >= second_end {
            RevisionId::new(30)
        } else if cut >= first_end {
            RevisionId::new(20)
        } else {
            RevisionId::new(10)
        };
        assert_eq!(scan.durable_revision(), expected, "cut={cut}");
    }
}

#[test]
fn durable_prepare_without_commit_is_not_visible() {
    let mut wal = SimulatedRevisionWal::new();
    wal.durably_prepare(&descriptor(1, 2, Value::I64(4)))
        .unwrap();
    let scan = scan_wal(wal.crash_image(), RevisionId::new(1)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(1));
}

#[test]
fn every_single_bit_corruption_of_committed_stream_is_rejected() {
    let mut wal = SimulatedRevisionWal::new();
    let prepared = wal
        .durably_prepare(&descriptor(1, 2, Value::Text("payload".into())))
        .unwrap();
    wal.durably_commit(prepared).unwrap();
    let bytes = wal.crash_image();
    for byte in 0..bytes.len() {
        for bit in 0..8 {
            let mut damaged = bytes.to_vec();
            damaged[byte] ^= 1_u8 << bit;
            assert!(
                scan_wal(&damaged, RevisionId::new(1)).is_err(),
                "accepted byte={byte} bit={bit}"
            );
        }
    }
}

#[test]
fn scanner_rejects_commit_whose_source_is_not_durable_head() {
    let mut wal = SimulatedRevisionWal::new();
    let prepared = wal
        .durably_prepare(&descriptor(99, 100, Value::I64(1)))
        .unwrap();
    wal.durably_commit(prepared).unwrap();
    assert!(matches!(
        scan_wal(wal.crash_image(), RevisionId::new(1)),
        Err(DurabilityError::Protocol { .. })
    ));
}

#[test]
fn duplicate_identical_prepare_and_commit_are_idempotent() {
    let mut wal = SimulatedRevisionWal::new();
    let descriptor = descriptor(1, 2, Value::I64(7));
    let first = wal.durably_prepare(&descriptor).unwrap();
    let _duplicate_prepare = wal.durably_prepare(&descriptor).unwrap();
    wal.durably_commit(first).unwrap();
    wal.durably_commit(first).unwrap();

    let scan = scan_wal(wal.crash_image(), RevisionId::new(1)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(2));
    assert_eq!(scan.committed().len(), 1);
}

#[test]
fn scanner_rejects_same_target_transaction_reuse_with_different_delta() {
    let mut wal = SimulatedRevisionWal::new();
    let first = descriptor(1, 2, Value::I64(7));
    let mut second = descriptor(1, 2, Value::I64(8));
    second.transaction_id = first.transaction_id;
    let prepared = wal.durably_prepare(&first).unwrap();
    wal.durably_commit(prepared).unwrap();
    wal.durably_prepare(&second).unwrap();

    assert!(matches!(
        scan_wal(wal.crash_image(), RevisionId::new(1)),
        Err(DurabilityError::Protocol {
            reason: "transaction id already committed to another exact intent",
            ..
        })
    ));
}

#[test]
fn scanner_rejects_transaction_id_reuse_for_different_prepare() {
    let mut wal = SimulatedRevisionWal::new();
    let first = descriptor(1, 2, Value::I64(7));
    let mut second = descriptor(1, 3, Value::I64(8));
    second.transaction_id = first.transaction_id;
    wal.durably_prepare(&first).unwrap();
    wal.durably_prepare(&second).unwrap();

    assert!(matches!(
        scan_wal(wal.crash_image(), RevisionId::new(1)),
        Err(DurabilityError::Protocol {
            reason: "transaction id reused by conflicting prepare",
            ..
        })
    ));
}

#[test]
fn scanner_rejects_conflicting_duplicate_prepare() {
    let mut wal = SimulatedRevisionWal::new();
    wal.durably_prepare(&descriptor(1, 2, Value::I64(7)))
        .unwrap();
    wal.durably_prepare(&descriptor(1, 2, Value::I64(8)))
        .unwrap();

    assert!(matches!(
        scan_wal(wal.crash_image(), RevisionId::new(1)),
        Err(DurabilityError::Protocol {
            reason: "conflicting duplicate prepare",
            ..
        })
    ));
}

#[test]
fn scanner_rejects_conflicting_duplicate_commit() {
    let mut wal = SimulatedRevisionWal::new();
    let descriptor = descriptor(1, 2, Value::I64(7));
    let first = wal.durably_prepare(&descriptor).unwrap();
    let second = wal.durably_prepare(&descriptor).unwrap();
    wal.durably_commit(first).unwrap();
    wal.durably_commit(second).unwrap();

    assert!(matches!(
        scan_wal(wal.crash_image(), RevisionId::new(1)),
        Err(DurabilityError::Protocol {
            reason: "conflicting duplicate commit",
            ..
        })
    ));
}

#[test]
fn scanner_distinguishes_short_garbage_tail_from_hidden_full_frame() {
    let mut wal = SimulatedRevisionWal::new();
    let prepared = wal
        .durably_prepare(&descriptor(1, 2, Value::I64(7)))
        .unwrap();
    wal.durably_commit(prepared).unwrap();

    let mut short_garbage = wal.crash_image().to_vec();
    short_garbage.extend_from_slice(b"junk");
    let scan = scan_wal(&short_garbage, RevisionId::new(1)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(2));
    assert!(matches!(scan.tail_status(), TailStatus::Garbage { .. }));

    let mut full_garbage = wal.crash_image().to_vec();
    full_garbage.extend(std::iter::repeat_n(0xA5, HEADER_LEN));
    assert!(matches!(
        scan_wal(&full_garbage, RevisionId::new(1)),
        Err(DurabilityError::Corruption {
            reason: "non-frame bytes large enough to hide a complete frame",
            ..
        })
    ));
}

#[test]
fn scanner_rejects_non_monotone_lsn_even_with_valid_checksums() {
    let descriptor = descriptor(1, 2, Value::I64(7));
    let payload = encode_prepare_payload(&descriptor).unwrap();
    let frame = encode_frame(
        2,
        RecordKind::PrepareRevision,
        descriptor.target_revision,
        &payload,
    )
    .unwrap();

    assert!(matches!(
        scan_wal(&frame.bytes, RevisionId::new(1)),
        Err(DurabilityError::Protocol {
            reason: "non-monotone or gapped LSN",
            ..
        })
    ));
}

#[test]
fn create_never_truncates_an_existing_wal() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cfmd-pass32-create-{unique}.wal"));
    std::fs::write(&path, b"existing wal bytes").unwrap();

    assert!(FileRevisionWal::create(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing wal bytes");
    let _ = std::fs::remove_file(path);
}

#[test]
fn file_open_recovered_truncates_safe_torn_tail_and_continues_lsn() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cfmd-pass32-{unique}.wal"));
    let mut wal = FileRevisionWal::create(&path).unwrap();
    let prepared = wal
        .durably_prepare(&descriptor(1, 2, Value::I64(5)))
        .unwrap();
    wal.durably_commit(prepared).unwrap();
    drop(wal);
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&MAGIC[..2]).unwrap();
        file.sync_all().unwrap();
    }
    let (mut reopened, scan) = FileRevisionWal::open_recovered(&path, RevisionId::new(1)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(2));
    let prepared = reopened
        .durably_prepare(&descriptor(2, 7, Value::I64(6)))
        .unwrap();
    let receipt = reopened.durably_commit(prepared).unwrap();
    assert_eq!(receipt.commit_lsn(), 4);
    drop(reopened);
    let (_, final_scan) = FileRevisionWal::open_recovered(&path, RevisionId::new(1)).unwrap();
    assert_eq!(final_scan.durable_revision(), RevisionId::new(7));
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod historical_lens_registry_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn step(
        source: u64,
        target: u64,
        lens: u128,
        manifest: u128,
        complement: Value,
    ) -> DurableMigrationComplement {
        DurableMigrationComplement::from_capsule(
            ComplementCapsule {
                source_schema: kernel_types::SchemaRevisionId::new(source),
                target_schema: kernel_types::SchemaRevisionId::new(target),
                lens_spec: LensSpecId(SemanticId::new(lens)),
                semantic_pins: SemanticManifestId(SemanticId::new(manifest)),
                encoding_version: 1,
                complement,
            },
            ComplementRetention::Forever,
        )
    }

    #[test]
    fn historical_value_restore_runs_exact_lens_chain_in_reverse() {
        let outer = SemanticId::new(81_001);
        let inner = SemanticId::new(81_002);
        let mut chain = LocalHistoricalComplementChain::default();
        chain.push(step(
            1,
            2,
            81_101,
            81_201,
            Value::Product(BTreeMap::from([(outer, Value::I64(10))])),
        ));
        chain.push(step(
            2,
            3,
            81_102,
            81_202,
            Value::Product(BTreeMap::from([(inner, Value::I64(20))])),
        ));
        let mut registry = HistoricalLensRegistry::default();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_101)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_201)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::ProductField {
                    field: SemanticId::new(81_011),
                },
            )
            .unwrap();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_102)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_202)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::ProductField {
                    field: SemanticId::new(81_012),
                },
            )
            .unwrap();

        let restored = chain.restore_value(&Value::I64(99), &registry).unwrap();
        let Value::Product(first) = restored else {
            panic!("first migration must reconstruct product")
        };
        assert_eq!(first[&outer], Value::I64(10));
        let Value::Product(second) = &first[&SemanticId::new(81_011)] else {
            panic!("second migration must reconstruct nested product")
        };
        assert_eq!(second[&inner], Value::I64(20));
        assert_eq!(second[&SemanticId::new(81_012)], Value::I64(99));
    }

    #[test]
    fn historical_restore_requires_exact_manifest_and_encoding_binding() {
        let mut chain = LocalHistoricalComplementChain::default();
        chain.push(step(1, 2, 81_301, 81_401, Value::Unit));
        let mut registry = HistoricalLensRegistry::default();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_301)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_999)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::Identity,
            )
            .unwrap();
        assert!(matches!(
            chain.restore_value(&Value::I64(1), &registry),
            Err(HistoricalRestoreError::MissingImplementation(_))
        ));
    }
}
