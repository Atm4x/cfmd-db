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

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
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
    let DurableTransactionIntent::RelationRewriteExact {
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
        DurableTransactionIntent::RelationResolutionExact {
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
        intent @ DurableTransactionIntent::RelationRewriteExact { .. } => {
            validate_relation_rewrite_prepare_intent(registry, descriptor, intent)
        }
        DurableTransactionIntent::RelationDataExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            semantic_modules,
        } => {
            validate_relation_prepare_intent(
                descriptor,
                *source_revision,
                *target_revision,
                *semantic_revision,
                relation_mutations,
                None,
            )?;
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::RelationDataResidualExact {
            source_revision,
            target_revision,
            semantic_revision,
            client_mutations: _,
            realized_mutations,
            semantic_modules,
        } => {
            let DurableRevisionChange::RelationData {
                semantic_revision: change_semantics,
                relation_mutations: change_mutations,
            } = &descriptor.change
            else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "residual relation intent is paired with a non-relation change",
                });
            };
            if *source_revision != descriptor.source_revision
                || *target_revision != descriptor.target_revision
                || *semantic_revision != *change_semantics
                || realized_mutations != change_mutations
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "residual relation realization does not match descriptor delta",
                });
            }
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::MixedRevisionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            model_delta,
            model_complement: _,
            semantic_modules,
        } => {
            let DurableRevisionChange::MixedRevision {
                semantic_revision: change_semantics,
                relation_mutations: change_mutations,
                model_delta: change_model_delta,
            } = &descriptor.change
            else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "mixed intent is paired with a non-mixed change",
                });
            };
            if *source_revision != descriptor.source_revision
                || *target_revision != descriptor.target_revision
                || *semantic_revision != *change_semantics
                || relation_mutations != change_mutations
                || model_delta != change_model_delta
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "mixed intent does not match descriptor delta",
                });
            }
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::MixedRevisionResidualExact {
            source_revision,
            target_revision,
            semantic_revision,
            client_relation_mutations: _,
            client_model_delta: _,
            realized_relation_mutations,
            realized_model_delta,
            realized_model_complement: _,
            semantic_modules,
        } => {
            let DurableRevisionChange::MixedRevision {
                semantic_revision: change_semantics,
                relation_mutations: change_mutations,
                model_delta: change_model_delta,
            } = &descriptor.change
            else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "residual mixed intent is paired with a non-mixed change",
                });
            };
            if *source_revision != descriptor.source_revision
                || *target_revision != descriptor.target_revision
                || *semantic_revision != *change_semantics
                || realized_relation_mutations != change_mutations
                || realized_model_delta != change_model_delta
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "residual mixed realization does not match descriptor delta",
                });
            }
            install_semantic_module_packages(registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::Exact {
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
        DurableTransactionIntent::SchemaMigrationExact {
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
        DurableTransactionIntent::LegacyTargetOnly { .. } => Err(DurabilityError::Protocol {
            offset: 0,
            reason: "new durable prepare does not carry exact transaction intent",
        }),
    }
}
