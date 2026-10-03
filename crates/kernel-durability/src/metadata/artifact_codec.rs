use kernel_lens::{ArchiveProofId, ComplementRetention, LensSpecId, SemanticManifestId};
use kernel_types::{MaterializationId, RevisionId, SemanticId};

use crate::binary_codec::{
    BinarySource, encode_value, push_bytes, push_len, push_u32, push_u64, push_u128,
};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableSemanticKeyPart, PHYSICAL_ARTIFACT_RECIPE_TAG,
    canonical_physical_artifact_specs,
};
use crate::domain::DurableMigrationComplement;
use crate::runtime::CodecError;

use super::query_codec::{decode_rel_expr, encode_rel_expr};

pub(crate) fn encode_migration_complements(
    out: &mut impl crate::binary_codec::BinarySink,
    complements: &[DurableMigrationComplement],
) -> Result<(), CodecError> {
    push_len(out, complements.len())?;
    let mut previous_target = None;
    for complement in complements {
        complement
            .validate()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if previous_target.is_some_and(|target| target != complement.source_schema) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous_target = Some(complement.target_schema);
        push_u64(out, complement.source_schema.raw());
        push_u64(out, complement.target_schema.raw());
        push_u128(out, complement.lens_spec.0.raw());
        push_u128(out, complement.semantic_pins.0.raw());
        push_u32(out, complement.encoding_version);
        match complement.retention {
            ComplementRetention::Forever => out.push(0),
            ComplementRetention::UntilRevision(revision) => {
                out.push(1);
                push_u64(out, revision.raw());
            }
            ComplementRetention::UntilEpoch(epoch) => {
                out.push(2);
                push_u64(out, epoch);
            }
            ComplementRetention::ExternalArchive(proof) => {
                out.push(3);
                push_u128(out, proof.0.raw());
            }
            ComplementRetention::Forget => out.push(4),
        }
        out.push(u8::from(complement.released));
        match &complement.local_complement {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                encode_value(out, value, 0)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn decode_migration_complements(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurableMigrationComplement>, &'static str> {
    let count = cursor.len()?;
    let mut complements = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous_target = None;
    for _ in 0..count {
        let source_schema = kernel_types::SchemaRevisionId::new(cursor.u64()?);
        let target_schema = kernel_types::SchemaRevisionId::new(cursor.u64()?);
        if previous_target.is_some_and(|target| target != source_schema) {
            return Err("migration complement chain is discontinuous");
        }
        previous_target = Some(target_schema);
        let lens_spec = LensSpecId(SemanticId::new(cursor.u128()?));
        let semantic_pins = SemanticManifestId(SemanticId::new(cursor.u128()?));
        let encoding_version = cursor.u32()?;
        let retention = match cursor.u8()? {
            0 => ComplementRetention::Forever,
            1 => ComplementRetention::UntilRevision(RevisionId::new(cursor.u64()?)),
            2 => ComplementRetention::UntilEpoch(cursor.u64()?),
            3 => ComplementRetention::ExternalArchive(ArchiveProofId(SemanticId::new(
                cursor.u128()?,
            ))),
            4 => ComplementRetention::Forget,
            _ => return Err("invalid migration complement retention tag"),
        };
        let released = match cursor.u8()? {
            0 => false,
            1 => true,
            _ => return Err("invalid migration complement release tag"),
        };
        let local_complement = match cursor.u8()? {
            0 => None,
            1 => Some(cursor.value(0)?),
            _ => return Err("invalid migration complement payload tag"),
        };
        let complement = DurableMigrationComplement {
            source_schema,
            target_schema,
            lens_spec,
            semantic_pins,
            encoding_version,
            retention,
            local_complement,
            released,
        };
        complement.validate()?;
        complements.push(complement);
    }
    Ok(complements)
}

pub(crate) fn encode_materialization_specs(
    out: &mut impl crate::binary_codec::BinarySink,
    specs: &[DurableMaterializationSpec],
) -> Result<(), CodecError> {
    push_len(out, specs.len())?;
    let mut previous = None;
    for spec in specs {
        if previous.is_some_and(|id: MaterializationId| id >= spec.id) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(spec.id);
        push_u128(out, spec.id.raw());
        encode_rel_expr(out, &spec.query, 0)?;
    }
    Ok(())
}

pub(crate) fn decode_materialization_specs(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurableMaterializationSpec>, &'static str> {
    let materialization_count = cursor.len()?;
    let mut materializations = Vec::with_capacity(cursor.bounded_capacity(materialization_count));
    let mut previous = None;
    for _ in 0..materialization_count {
        let id = MaterializationId::new(cursor.u128()?);
        if previous.is_some_and(|prior: MaterializationId| prior >= id) {
            return Err("materialization ids are not strictly sorted and unique");
        }
        previous = Some(id);
        materializations.push(DurableMaterializationSpec {
            id,
            query: decode_rel_expr(cursor, 0)?,
        });
    }
    Ok(materializations)
}

pub(super) fn encode_artifact_cores(
    out: &mut impl crate::binary_codec::BinarySink,
    cores: &[DurableArtifactCore],
) -> Result<(), CodecError> {
    let mut cores = cores.to_vec();
    cores.sort();
    cores.dedup();
    push_len(out, cores.len())?;
    for core in cores {
        match core {
            DurableArtifactCore::ObservableAtom {
                source_revision,
                relation,
                key_parts,
                encoded_keys_by_ordinal,
            } => {
                out.push(0);
                push_u64(out, source_revision.raw());
                push_u128(out, relation.raw());
                push_len(out, key_parts.len())?;
                for part in key_parts {
                    push_len(out, part.column)?;
                    push_u128(out, part.equivalence.raw());
                }
                push_len(out, encoded_keys_by_ordinal.len())?;
                for tuple in encoded_keys_by_ordinal {
                    kernel_semantics::decode_canonical_eq_key_tuple(&tuple)
                        .map_err(|_| CodecError::CollectionTooLarge)?;
                    push_bytes(out, &tuple)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn decode_artifact_cores(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurableArtifactCore>, &'static str> {
    let count = cursor.len()?;
    let mut cores = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous = None;
    for _ in 0..count {
        let core = match cursor.u8()? {
            0 => {
                let source_revision = RevisionId::new(cursor.u64()?);
                let relation = SemanticId::new(cursor.u128()?);
                let key_count = cursor.len()?;
                let mut key_parts = Vec::with_capacity(cursor.bounded_capacity(key_count));
                for _ in 0..key_count {
                    key_parts.push(DurableSemanticKeyPart {
                        column: cursor.len()?,
                        equivalence: SemanticId::new(cursor.u128()?),
                    });
                }
                let row_count = cursor.len()?;
                let mut encoded_keys_by_ordinal =
                    Vec::with_capacity(cursor.bounded_capacity(row_count));
                for _ in 0..row_count {
                    let len = cursor.len()?;
                    let tuple = cursor.take_owned(len)?;
                    let keys = kernel_semantics::decode_canonical_eq_key_tuple(&tuple)
                        .map_err(|_| "invalid canonical observable artifact core")?;
                    if keys.len() != key_parts.len() {
                        return Err("observable artifact core key arity mismatch");
                    }
                    encoded_keys_by_ordinal.push(tuple);
                }
                DurableArtifactCore::ObservableAtom {
                    source_revision,
                    relation,
                    key_parts,
                    encoded_keys_by_ordinal,
                }
            }
            _ => return Err("unknown durable artifact core tag"),
        };
        if previous.as_ref().is_some_and(|prior| prior >= &core) {
            return Err("durable artifact cores are not strictly sorted and unique");
        }
        previous = Some(core.clone());
        cores.push(core);
    }
    Ok(cores)
}

pub(super) fn encode_physical_artifact_specs(
    out: &mut impl crate::binary_codec::BinarySink,
    specs: &[DurablePhysicalArtifactSpec],
) -> Result<(), CodecError> {
    out.extend_from_slice(&PHYSICAL_ARTIFACT_RECIPE_TAG.to_le_bytes());
    let specs = canonical_physical_artifact_specs(specs);
    push_len(out, specs.len())?;
    for spec in specs {
        match spec {
            DurablePhysicalArtifactSpec::RelationLayout {
                relation,
                layout_id,
                kind,
            } => {
                out.push(0);
                push_u128(out, relation.raw());
                push_u128(out, layout_id);
                out.push(match kind {
                    DurableRelationLayoutKind::RowStore => 0,
                    DurableRelationLayoutKind::ValueColumnar => 1,
                    DurableRelationLayoutKind::I64Columnar => 2,
                    DurableRelationLayoutKind::TypedColumnar => 3,
                });
            }
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column,
                equivalence,
                advisor_managed,
            } => {
                out.push(1);
                push_u128(out, relation.raw());
                push_u64(
                    out,
                    u64::try_from(key_column).map_err(|_| CodecError::CollectionTooLarge)?,
                );
                push_u128(out, equivalence.raw());
                out.push(u8::from(advisor_managed));
            }
            DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(2);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
            DurablePhysicalArtifactSpec::SemanticStatistics {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(3);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(4);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
        }
    }
    Ok(())
}

pub(super) fn encode_semantic_artifact_key(
    out: &mut impl crate::binary_codec::BinarySink,
    relation: SemanticId,
    key_parts: &[DurableSemanticKeyPart],
    advisor_managed: bool,
) -> Result<(), CodecError> {
    push_u128(out, relation.raw());
    push_len(out, key_parts.len())?;
    for part in key_parts {
        push_u64(
            out,
            u64::try_from(part.column).map_err(|_| CodecError::CollectionTooLarge)?,
        );
        push_u128(out, part.equivalence.raw());
    }
    out.push(u8::from(advisor_managed));
    Ok(())
}

pub(super) fn decode_physical_artifact_specs(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurablePhysicalArtifactSpec>, &'static str> {
    let recipe_tag = cursor.u16()?;
    if recipe_tag != PHYSICAL_ARTIFACT_RECIPE_TAG {
        return Err("unsupported physical artifact recipe tag");
    }
    let count = cursor.len()?;
    let mut specs = Vec::with_capacity(cursor.bounded_capacity(count));
    for _ in 0..count {
        let spec = match cursor.u8()? {
            0 => {
                let relation = SemanticId::new(cursor.u128()?);
                let layout_id = cursor.u128()?;
                let kind = match cursor.u8()? {
                    0 => DurableRelationLayoutKind::RowStore,
                    1 => DurableRelationLayoutKind::ValueColumnar,
                    2 => DurableRelationLayoutKind::I64Columnar,
                    3 => DurableRelationLayoutKind::TypedColumnar,
                    _ => return Err("invalid durable relation layout kind"),
                };
                DurablePhysicalArtifactSpec::RelationLayout {
                    relation,
                    layout_id,
                    kind,
                }
            }
            1 => {
                let relation = SemanticId::new(cursor.u128()?);
                let key_column = usize::try_from(cursor.u64()?)
                    .map_err(|_| "physical artifact column overflow")?;
                let equivalence = SemanticId::new(cursor.u128()?);
                let advisor_managed = match cursor.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err("invalid physical artifact ownership tag"),
                };
                DurablePhysicalArtifactSpec::I64Index {
                    relation,
                    key_column,
                    equivalence,
                    advisor_managed,
                }
            }
            2 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            3 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::SemanticStatistics {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            4 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::ObservableAtom {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            _ => return Err("invalid physical artifact spec tag"),
        };
        specs.push(spec);
    }
    if specs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("physical artifact specs are not strictly sorted and unique");
    }
    Ok(specs)
}

fn decode_semantic_artifact_key(
    cursor: &mut impl BinarySource,
) -> Result<(SemanticId, Vec<DurableSemanticKeyPart>, bool), &'static str> {
    let relation = SemanticId::new(cursor.u128()?);
    let count = cursor.len()?;
    if count == 0 {
        return Err("physical artifact semantic key is empty");
    }
    let mut key_parts = Vec::with_capacity(cursor.bounded_capacity(count));
    for _ in 0..count {
        let column =
            usize::try_from(cursor.u64()?).map_err(|_| "physical artifact column overflow")?;
        let equivalence = SemanticId::new(cursor.u128()?);
        key_parts.push(DurableSemanticKeyPart {
            column,
            equivalence,
        });
    }
    let advisor_managed = match cursor.u8()? {
        0 => false,
        1 => true,
        _ => return Err("invalid physical artifact ownership tag"),
    };
    Ok((relation, key_parts, advisor_managed))
}
