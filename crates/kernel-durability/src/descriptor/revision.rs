use kernel_change::RevisionEffectId;
use kernel_types::{ClientTransactionId, RevisionId, SemanticRevision};

use crate::checkpoint;
use crate::domain::{
    DurableMigrationComplement, DurableRelationMutation, DurableRelationResolution,
    DurableRelationRewriteIntent, DurableRevisionChange, DurableTransactionIntent,
    IdempotencyEpoch,
};
use crate::runtime::{CodecError, DurabilityError};

use super::artifacts::DurableMaterializationSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRevisionDescriptor {
    pub idempotency_epoch: IdempotencyEpoch,
    pub revision_effect_id: Option<RevisionEffectId>,
    pub transaction_id: ClientTransactionId,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub intent: DurableTransactionIntent,
    pub change: DurableRevisionChange,
}

impl DurableRevisionDescriptor {
    pub fn relation_data(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_data(
                source_revision,
                target,
                semantic_revision,
                relation_mutations.clone(),
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn relation_rewrites(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_rewrites(
                source_revision,
                target,
                semantic_revision,
                relation_mutations.clone(),
                rewrite_intents,
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn relation_resolution(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        resolution: DurableRelationResolution,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let relation_mutations = resolution.relation_mutations.clone();
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_resolution(
                source_revision,
                target,
                semantic_revision,
                resolution,
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn full_revision(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::revision(target, registry)?,
            change: DurableRevisionChange::FullRevision {
                encoded_target_revision: checkpoint::encode_revision(target)?,
            },
        })
    }

    pub fn schema_migration(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        migration_complement: DurableMigrationComplement,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let intent = DurableTransactionIntent::schema_migration(
            source_revision,
            target,
            migration_complement,
            registry,
        )?;
        let DurableTransactionIntent::SchemaMigrationExact {
            encoded_target_revision,
            ..
        } = &intent
        else {
            unreachable!("schema migration intent has exact target bytes")
        };
        let encoded_target_revision = encoded_target_revision.clone();
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent,
            change: DurableRevisionChange::FullRevision {
                encoded_target_revision,
            },
        })
    }

    pub fn full_revision_and_materializations(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        materializations: &[DurableMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let intent = DurableTransactionIntent::revision_and_materializations(
            target,
            materializations,
            registry,
        )?;
        let DurableTransactionIntent::Exact {
            encoded_target_revision,
            materializations: Some(materializations),
            ..
        } = intent.clone()
        else {
            unreachable!("combined durable intent is exact and carries materializations")
        };
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent,
            change: DurableRevisionChange::FullRevisionAndMaterializations {
                encoded_target_revision,
                materializations,
            },
        })
    }

    pub fn decode_full_revision(
        &self,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<kernel_revision::Revision>, DurabilityError> {
        match &self.change {
            DurableRevisionChange::RelationData { .. } => Ok(None),
            DurableRevisionChange::FullRevision {
                encoded_target_revision,
            }
            | DurableRevisionChange::FullRevisionAndMaterializations {
                encoded_target_revision,
                ..
            } => {
                let revision = checkpoint::decode_revision(encoded_target_revision, registry)?;
                if revision.id() != self.target_revision {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "full revision payload target id mismatch",
                    });
                }
                Ok(Some(revision))
            }
        }
    }
}
