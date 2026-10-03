use kernel_semantics::SemanticRegistry;
use kernel_types::{RevisionId, SchemaRevisionId};

use super::DurableRevisionStore;
use super::causal_ledger;
use super::migration_history;
use super::semantic_deployment::install_semantic_module_packages;
use crate::checkpoint;
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{
    DurableMigrationComplement, DurableRelationMutation, DurableRelationResolution,
    DurableRelationRewriteIntent, DurableRevisionChange, DurableTransactionIntent,
};
use crate::runtime::DurabilityError;

fn validate_relation_prepare_intent(
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    relation_mutations: &[DurableRelationMutation],
    rewrite_intents: Option<&[DurableRelationRewriteIntent]>,
) -> Result<(), DurabilityError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation intent is paired with a non-delta change",
        });
    };
    let rewrite_shape_matches = rewrite_intents.is_none_or(|intents| {
        relation_mutations.len() == intents.len()
            && relation_mutations
                .iter()
                .zip(intents)
                .all(|(mutation, intent)| mutation.relation == intent.relation)
    });
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || relation_mutations != change_mutations
        || !rewrite_shape_matches
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation intent does not match descriptor delta",
        });
    }
    Ok(())
}

fn validate_full_revision_prepare_intent(
    descriptor: &DurableRevisionDescriptor,
    target_revision: RevisionId,
    encoded_target_revision: &[u8],
    registry: &SemanticRegistry,
) -> Result<(), DurabilityError> {
    if target_revision != descriptor.target_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction intent target does not match descriptor target",
        });
    }
    let target = checkpoint::decode_revision(encoded_target_revision, registry)?;
    if target.id() != descriptor.target_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction intent revision payload does not match target id",
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct SchemaMigrationPrepare<'a> {
    expected_source_schema: Option<SchemaRevisionId>,
    source_revision: RevisionId,
    target_revision: RevisionId,
    program: &'a kernel_transport::SchemaMigrationProgram,
    migration_complement: &'a DurableMigrationComplement,
    semantic_modules: &'a [kernel_semantics::BuiltinSemanticModuleSpec],
}

fn validate_schema_migration_prepare_intent(
    store: &DurableRevisionStore,
    registry: &mut SemanticRegistry,
    descriptor: &DurableRevisionDescriptor,
    prepare: SchemaMigrationPrepare<'_>,
) -> Result<(), DurabilityError> {
    let SchemaMigrationPrepare {
        expected_source_schema,
        source_revision,
        target_revision,
        program,
        migration_complement,
        semantic_modules,
    } = prepare;
    if source_revision != descriptor.source_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration source revision does not match descriptor",
        });
    }
    if target_revision != descriptor.target_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration target revision does not match descriptor",
        });
    }
    let DurableRevisionChange::SchemaMigration {
        program: change_program,
    } = &descriptor.change
    else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration intent is paired with a non-migration change",
        });
    };
    if program != change_program {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration intent program does not match descriptor change",
        });
    }
    install_semantic_module_packages(registry, semantic_modules)?;
    registry
        .validate_context(program.target())
        .map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration target semantic context is invalid",
        })?;
    migration_complement
        .validate()
        .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
    if migration_complement.target_schema != program.target().schema.revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement target schema does not match target revision",
        });
    }
    if expected_source_schema.is_none() {
        if let Some(&position) =
            store
                .migration_complement_index
                .get(&migration_history::migration_complement_key(
                    migration_complement,
                ))
        {
            if migration_history::migration_step_replay_compatible(
                &store.migration_complements[position],
                migration_complement,
            ) {
                return Ok(());
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "schema migration complement conflicts with staged durable authority",
            });
        }
        return migration_history::validate_migration_complement_append(
            &store.migration_complements,
            &store.migration_complement_index,
            store.checkpoint.semantic_revision().schema,
            migration_complement,
        );
    }
    if let Some(&position) =
        store
            .migration_complement_index
            .get(&migration_history::migration_complement_key(
                migration_complement,
            ))
    {
        if migration_history::migration_step_replay_compatible(
            &store.migration_complements[position],
            migration_complement,
        ) {
            return Ok(());
        }
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration complement conflicts with staged durable authority",
        });
    }
    if migration_complement.source_schema != expected_source_schema.expect("checked above") {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement source schema does not match durable chain",
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RelationResolutionPrepare<'a> {
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    resolution: &'a DurableRelationResolution,
    semantic_modules: &'a [kernel_semantics::BuiltinSemanticModuleSpec],
}

fn validate_relation_resolution_prepare_intent(
    store: &DurableRevisionStore,
    registry: &mut SemanticRegistry,
    descriptor: &DurableRevisionDescriptor,
    prepare: RelationResolutionPrepare<'_>,
) -> Result<(), DurabilityError> {
    let RelationResolutionPrepare {
        source_revision,
        target_revision,
        semantic_revision,
        resolution,
        semantic_modules,
    } = prepare;
    validate_relation_prepare_intent(
        descriptor,
        source_revision,
        target_revision,
        semantic_revision,
        &resolution.relation_mutations,
        Some(&resolution.rewrite_intents),
    )?;
    if resolution.causal_parents.len() < 2
        || resolution
            .causal_parents
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || resolution
            .causal_parents
            .binary_search(&source_revision)
            .is_err()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation resolution causal parents are not canonical",
        });
    }
    let _ = causal_ledger::causal_prerequisites_for_descriptor(
        &store.revision_effect_frontiers,
        descriptor,
    )?;
    install_semantic_module_packages(registry, semantic_modules)?;
    Ok(())
}

fn validate_relation_rewrite_prepare_intent(
    registry: &mut SemanticRegistry,
    descriptor: &DurableRevisionDescriptor,
    intent: &DurableTransactionIntent,
) -> Result<(), DurabilityError> {
    let DurableTransactionIntent::RelationRewrite {
        source_revision,
        target_revision,
        semantic_revision,
        relation_mutations,
        rewrite_intents,
        semantic_modules,
    } = intent
    else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "rewrite prepare helper received non-rewrite intent",
        });
    };
    validate_relation_prepare_intent(
        descriptor,
        *source_revision,
        *target_revision,
        *semantic_revision,
        relation_mutations,
        Some(rewrite_intents),
    )?;
    install_semantic_module_packages(registry, semantic_modules)?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // Single protocol validation decision tree over one bound PREPARE intent.
pub(super) fn validate_bound_prepare_intent(
    store: &DurableRevisionStore,
    registry: &mut SemanticRegistry,
    descriptor: &DurableRevisionDescriptor,
    expected_migration_source_schema: Option<SchemaRevisionId>,
) -> Result<(), DurabilityError> {
    match &descriptor.intent {
        DurableTransactionIntent::RelationResolution {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => {
            let resolution = DurableRelationResolution {
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
                causal_parents: causal_parents.clone(),
            };
            validate_relation_resolution_prepare_intent(
                store,
                registry,
                descriptor,
                RelationResolutionPrepare {
                    source_revision: *source_revision,
                    target_revision: *target_revision,
                    semantic_revision: *semantic_revision,
                    resolution: &resolution,
                    semantic_modules,
                },
            )
        }
        intent @ DurableTransactionIntent::RelationRewrite { .. } => {
            validate_relation_rewrite_prepare_intent(registry, descriptor, intent)
        }
        DurableTransactionIntent::RelationData {
            source_revision,
            target_revision,
            semantic_modules,
            ..
        } => {
            let DurableRevisionChange::RelationData { .. } = &descriptor.change else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "relation client intent is paired with a non-relation realized change",
                });
            };
            if *source_revision != descriptor.source_revision
                || *target_revision != descriptor.target_revision
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "relation intent provenance does not match descriptor",
                });
            }
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::MixedRevision {
            source_revision,
            target_revision,
            semantic_modules,
            ..
        } => {
            let DurableRevisionChange::MixedRevision { .. } = &descriptor.change else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "mixed client intent is paired with a non-mixed realized change",
                });
            };
            if *source_revision != descriptor.source_revision
                || *target_revision != descriptor.target_revision
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "mixed intent provenance does not match descriptor",
                });
            }
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::FullRevision {
            target_revision,
            encoded_target_revision,
            semantic_modules,
            ..
        } => {
            install_semantic_module_packages(registry, semantic_modules)?;
            validate_full_revision_prepare_intent(
                descriptor,
                *target_revision,
                encoded_target_revision,
                registry,
            )
        }
        DurableTransactionIntent::SchemaMigration {
            source_revision,
            target_revision,
            program,
            migration_complement,
            semantic_modules,
        } => validate_schema_migration_prepare_intent(
            store,
            registry,
            descriptor,
            SchemaMigrationPrepare {
                expected_source_schema: expected_migration_source_schema,
                source_revision: *source_revision,
                target_revision: *target_revision,
                program,
                migration_complement,
                semantic_modules,
            },
        ),
    }
}
