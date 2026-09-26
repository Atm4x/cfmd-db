use super::{
    Arc, BTreeMap, DurabilityError, DurableArtifactCore, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableRelationMutation, DurableRevisionChange,
    DurableSemanticKeyPart, LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation,
    PhysicalExecutionError, PhysicalRecoveryPolicy, PhysicalRecoveryReport, PhysicalStore,
    RecoveryScan, RelExpr, RelQueryError, RelationDelta, RevisionId, RuntimeMaterializationSpec,
    RuntimeRecoveryError, RuntimeRevisionBundle, SemanticId, SemanticIndexBinding,
    SemanticIndexKeyPart, Value, VecDeque,
};
use crate::semantic_key::{
    ResolvedSemanticIndexKeyPart, resolve_semantic_key_binding, resolved_semantic_index_row_key,
};
use crate::semantic_rows::{canonical_semantic_row_key, semantic_rows_equal};
use crate::storage_impl::PhysicalRecoveryInputs;

/// Replays the committed logical WAL tail over one exact durable base
/// revision. Incremental relation-data records preserve one pinned semantic
/// context; full-revision records atomically replace `(S, Γ, M)` and are the
/// correctness-first durable path for schema/semantic/lifecycle migrations.
pub fn replay_durable_revisions(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<kernel_revision::Revision, RuntimeRecoveryError> {
    if scan.base_revision() != base_revision.id() {
        return Err(RuntimeRecoveryError::BaseRevisionMismatch);
    }
    let mut current = base_revision.clone();
    for committed in scan.committed() {
        let descriptor = &committed.descriptor;
        if descriptor.source_revision != current.id() {
            return Err(RuntimeRecoveryError::BaseRevisionMismatch);
        }
        match &descriptor.change {
            DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            } => {
                if *semantic_revision != current.semantic_revision() {
                    return Err(RuntimeRecoveryError::SemanticRevisionMismatch);
                }

                let mut candidate = current.relation_update_candidate();
                for mutation in relation_mutations {
                    let relation = RelExpr::Scan(mutation.relation);
                    let result_type = relation
                        .typecheck(current.semantic_context(), registry)
                        .map_err(PhysicalExecutionError::from)?;
                    let old = relation
                        .evaluate(
                            &candidate.state().model,
                            current.semantic_context(),
                            registry,
                        )
                        .map_err(PhysicalExecutionError::from)?;
                    let delta = RelationDelta {
                        inserted: mutation.inserted.clone(),
                        removed: mutation.removed.clone(),
                        result_type,
                    };
                    let next = delta
                        .apply_to_value(old, current.semantic_context(), registry)
                        .map_err(PhysicalExecutionError::from)?;
                    candidate.replace_relation_rows(mutation.relation, next.into_rows());
                }
                current = candidate.build(descriptor.target_revision, registry)?;
            }
            DurableRevisionChange::FullRevision { .. }
            | DurableRevisionChange::FullRevisionAndMaterializations { .. } => {
                current = descriptor.decode_full_revision(registry)?.ok_or(
                    RuntimeRecoveryError::Durability(DurabilityError::Protocol {
                        offset: 0,
                        reason: "full revision record missing full revision payload",
                    }),
                )?;
            }
        }
    }
    Ok(current)
}

/// Reconstructs a reader-visible runtime root from an exact base revision and
/// the committed logical WAL tail. Physical row handles, indexes and maintained
/// state are regenerated rather than recovered as authority.
// HOSTILE[P161][RECOVERY][KEEP]: full reconstruction boundary; not a normal hot-path fallback.
pub fn recover_runtime_bundle(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    materialization_specs: &[RuntimeMaterializationSpec],
    physical_artifact_specs: &[DurablePhysicalArtifactSpec],
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RuntimeRevisionBundle, RuntimeRecoveryError> {
    recover_runtime_bundle_with_policy(
        base_revision,
        scan,
        materialization_specs,
        physical_artifact_specs,
        PhysicalRecoveryPolicy::default(),
        registry,
    )
    .map(|(bundle, _)| bundle)
}

pub fn recover_runtime_bundle_with_policy(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    materialization_specs: &[RuntimeMaterializationSpec],
    physical_artifact_specs: &[DurablePhysicalArtifactSpec],
    physical_recovery_policy: PhysicalRecoveryPolicy,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RuntimeRevisionBundle, PhysicalRecoveryReport), RuntimeRecoveryError> {
    recover_runtime_bundle_with_policy_and_cores(
        base_revision,
        scan,
        materialization_specs,
        physical_artifact_specs,
        &[],
        physical_recovery_policy,
        registry,
    )
}

pub(super) fn recover_runtime_bundle_with_policy_and_cores(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    materialization_specs: &[RuntimeMaterializationSpec],
    physical_artifact_specs: &[DurablePhysicalArtifactSpec],
    artifact_cores: &[DurableArtifactCore],
    physical_recovery_policy: PhysicalRecoveryPolicy,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RuntimeRevisionBundle, PhysicalRecoveryReport), RuntimeRecoveryError> {
    let revision = replay_durable_revisions(base_revision, scan, registry)?;
    let mut effective_materializations = materialization_specs.to_vec();
    for committed in scan.committed() {
        if let DurableRevisionChange::FullRevisionAndMaterializations {
            materializations, ..
        } = &committed.descriptor.change
        {
            effective_materializations = materializations
                .iter()
                .map(|spec| RuntimeMaterializationSpec {
                    id: spec.id,
                    query: spec.query.clone(),
                })
                .collect();
        }
    }
    let replayed_artifact_cores =
        replay_durable_artifact_cores(base_revision, scan, artifact_cores, registry);
    let mut physical = PhysicalStore::default();
    let mut relation_layouts = BTreeMap::new();
    for relation in revision.semantic_context().schema.relations() {
        let rows = revision
            .state()
            .model
            .relations
            .get(&relation.id)
            .cloned()
            .unwrap_or_default();
        let recovered = recovered_relation_layout_recipe(relation.id, physical_artifact_specs)
            .and_then(|(layout, kind)| {
                recovered_native_relation(&revision, relation, &rows, kind)
                    .ok()
                    .map(|native| (layout, native))
            });
        let (layout, native) = recovered.unwrap_or_else(|| {
            (
                LayoutBinding::RECOVERY_ROW_STORE,
                NativeRelation::row_store(rows),
            )
        });
        physical.install(relation.id, layout, native)?;
        relation_layouts.insert(relation.id, layout);
    }
    let recovery_report = physical.restore_durable_physical_artifacts(&PhysicalRecoveryInputs {
        specs: physical_artifact_specs,
        artifact_cores: &replayed_artifact_cores,
        target_revision: revision.id(),
        relation_layouts: &relation_layouts,
        policy: physical_recovery_policy,
        telemetry: None,
        context: revision.semantic_context(),
        registry,
    });
    let bundle = RuntimeRevisionBundle::build(
        revision,
        physical,
        relation_layouts,
        &effective_materializations,
        registry,
    )?;
    Ok((bundle, recovery_report))
}

fn replay_durable_artifact_cores(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    cores: &[DurableArtifactCore],
    registry: &kernel_semantics::SemanticRegistry,
) -> Vec<DurableArtifactCore> {
    cores
        .iter()
        .filter_map(|core| replay_durable_artifact_core(base_revision, scan, core, registry).ok())
        .collect()
}

#[derive(Debug, Clone)]
struct ObservableCoreReplayState {
    rows: Vec<Option<Vec<Value>>>,
    encoded_keys_by_ordinal: Vec<Option<Vec<u8>>>,
    row_class_positions: BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, VecDeque<usize>>,
    binding: SemanticIndexBinding,
    resolved: Vec<ResolvedSemanticIndexKeyPart>,
    column_equivalences: Vec<SemanticId>,
    is_set: bool,
}

fn apply_relation_mutation_to_observable_core(
    state: &mut ObservableCoreReplayState,
    mutation: &DurableRelationMutation,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), PhysicalExecutionError> {
    for removed in &mutation.removed {
        let class =
            canonical_semantic_row_key(removed, &state.column_equivalences, context, registry)?;
        let positions = state
            .row_class_positions
            .get_mut(&class)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let index = positions
            .pop_front()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if positions.is_empty() {
            state.row_class_positions.remove(&class);
        }
        let existing = state
            .rows
            .get_mut(index)
            .and_then(Option::take)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if !semantic_rows_equal(
            &existing,
            removed,
            &state.column_equivalences,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        state
            .encoded_keys_by_ordinal
            .get_mut(index)
            .and_then(Option::take)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
    }
    for inserted in &mutation.inserted {
        let class =
            canonical_semantic_row_key(inserted, &state.column_equivalences, context, registry)?;
        if state.is_set
            && state
                .row_class_positions
                .get(&class)
                .is_some_and(|positions| !positions.is_empty())
        {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        let keys = resolved_semantic_index_row_key(&state.binding, &state.resolved, inserted)?;
        let index = state.rows.len();
        state.rows.push(Some(inserted.clone()));
        state
            .encoded_keys_by_ordinal
            .push(Some(kernel_semantics::encode_canonical_eq_key_tuple(&keys)));
        state
            .row_class_positions
            .entry(class)
            .or_default()
            .push_back(index);
    }
    Ok(())
}

fn replay_durable_artifact_core(
    base_revision: &kernel_revision::Revision,
    scan: &RecoveryScan,
    core: &DurableArtifactCore,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<DurableArtifactCore, PhysicalExecutionError> {
    let DurableArtifactCore::ObservableAtom {
        source_revision,
        relation,
        key_parts,
        encoded_keys_by_ordinal,
    } = core;
    if *source_revision != base_revision.id() {
        return Err(PhysicalExecutionError::RevisionBindingMismatch);
    }
    let context = base_revision.semantic_context();
    let definition = context
        .schema
        .relation(*relation)
        .ok_or(RelQueryError::UnknownRelation(*relation))?;
    let (column_equivalences, is_set) = match &definition.semantics {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => (column_equivalences.clone(), false),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => (column_equivalences.clone(), true),
    };
    let binding =
        recovered_semantic_index_binding(*relation, LayoutBinding::RECOVERY_ROW_STORE, key_parts)?;
    let (resolved, _) = resolve_semantic_key_binding(&binding, context, registry)?;
    let rows = base_revision
        .state()
        .model
        .relations
        .get(relation)
        .cloned()
        .unwrap_or_default();
    if rows.len() != encoded_keys_by_ordinal.len() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    for tuple in encoded_keys_by_ordinal {
        let keys = kernel_semantics::decode_canonical_eq_key_tuple(tuple)
            .map_err(|_| PhysicalExecutionError::PhysicalTypeMismatch)?;
        if keys.len() != key_parts.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
    }
    let mut row_class_positions =
        BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, VecDeque<usize>>::new();
    for (index, row) in rows.iter().enumerate() {
        let class = canonical_semantic_row_key(row, &column_equivalences, context, registry)?;
        row_class_positions
            .entry(class)
            .or_default()
            .push_back(index);
    }
    let mut state = ObservableCoreReplayState {
        rows: rows.into_iter().map(Some).collect(),
        encoded_keys_by_ordinal: encoded_keys_by_ordinal.iter().cloned().map(Some).collect(),
        row_class_positions,
        binding,
        resolved,
        column_equivalences,
        is_set,
    };
    let mut current_revision = base_revision.id();
    for committed in scan.committed() {
        if committed.descriptor.source_revision != current_revision {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        let DurableRevisionChange::RelationData {
            semantic_revision,
            relation_mutations,
        } = &committed.descriptor.change
        else {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        };
        if *semantic_revision != base_revision.semantic_revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        for mutation in relation_mutations
            .iter()
            .filter(|mutation| mutation.relation == *relation)
        {
            apply_relation_mutation_to_observable_core(&mut state, mutation, context, registry)?;
        }
        current_revision = committed.descriptor.target_revision;
    }
    Ok(DurableArtifactCore::ObservableAtom {
        source_revision: current_revision,
        relation: *relation,
        key_parts: key_parts.clone(),
        encoded_keys_by_ordinal: state
            .encoded_keys_by_ordinal
            .into_iter()
            .flatten()
            .collect(),
    })
}

pub(super) fn matching_durable_artifact_core<'a>(
    spec: &DurablePhysicalArtifactSpec,
    cores: &'a [DurableArtifactCore],
    target_revision: RevisionId,
) -> Option<&'a DurableArtifactCore> {
    let DurablePhysicalArtifactSpec::ObservableAtom {
        relation,
        key_parts,
        ..
    } = spec
    else {
        return None;
    };
    cores.iter().find(|core| {
        matches!(
            core,
            DurableArtifactCore::ObservableAtom {
                source_revision,
                relation: core_relation,
                key_parts: core_key_parts,
                ..
            } if *source_revision == target_revision
                && core_relation == relation
                && core_key_parts == key_parts
        )
    })
}

pub(super) fn durable_semantic_key_parts(
    binding: &SemanticIndexBinding,
) -> Vec<DurableSemanticKeyPart> {
    binding
        .key_parts
        .iter()
        .map(|part| DurableSemanticKeyPart {
            column: part.column,
            equivalence: part.equivalence,
        })
        .collect()
}

pub(super) fn durable_physical_artifact_is_advisor_managed(
    spec: &DurablePhysicalArtifactSpec,
) -> bool {
    match spec {
        DurablePhysicalArtifactSpec::RelationLayout { .. } => false,
        DurablePhysicalArtifactSpec::I64Index {
            advisor_managed, ..
        }
        | DurablePhysicalArtifactSpec::SemanticIndex {
            advisor_managed, ..
        }
        | DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            advisor_managed, ..
        }
        | DurablePhysicalArtifactSpec::SemanticStatistics {
            advisor_managed, ..
        }
        | DurablePhysicalArtifactSpec::ObservableAtom {
            advisor_managed, ..
        } => *advisor_managed,
    }
}

pub(super) fn recovered_semantic_index_binding(
    relation: SemanticId,
    layout: LayoutBinding,
    key_parts: &[DurableSemanticKeyPart],
) -> Result<SemanticIndexBinding, PhysicalExecutionError> {
    if key_parts.is_empty() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    Ok(SemanticIndexBinding {
        relation,
        layout,
        key_parts: key_parts
            .iter()
            .map(|part| SemanticIndexKeyPart {
                column: part.column,
                equivalence: part.equivalence,
            })
            .collect(),
    })
}

fn recovered_relation_layout_recipe(
    relation: SemanticId,
    specs: &[DurablePhysicalArtifactSpec],
) -> Option<(LayoutBinding, DurableRelationLayoutKind)> {
    let mut recovered = None;
    for spec in specs {
        let DurablePhysicalArtifactSpec::RelationLayout {
            relation: candidate,
            layout_id,
            kind,
        } = spec
        else {
            continue;
        };
        if *candidate != relation {
            continue;
        }
        let family = match kind {
            DurableRelationLayoutKind::RowStore => LayoutFamily::RowStore,
            DurableRelationLayoutKind::ValueColumnar
            | DurableRelationLayoutKind::I64Columnar
            | DurableRelationLayoutKind::TypedColumnar => LayoutFamily::Columnar,
        };
        let candidate = (
            LayoutBinding {
                id: LayoutId(*layout_id),
                family,
            },
            *kind,
        );
        if recovered.is_some_and(|prior| prior != candidate) {
            return None;
        }
        recovered = Some(candidate);
    }
    recovered
}

fn transpose_value_rows(
    rows: &[kernel_query::Row],
    column_count: usize,
) -> Result<Vec<Vec<Value>>, PhysicalExecutionError> {
    let mut columns = (0..column_count)
        .map(|_| Vec::with_capacity(rows.len()))
        .collect::<Vec<_>>();
    for row in rows {
        if row.len() != column_count {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        for (column, value) in columns.iter_mut().zip(row) {
            column.push(value.clone());
        }
    }
    Ok(columns)
}

fn recovered_native_relation(
    revision: &kernel_revision::Revision,
    relation: &kernel_schema::RelationDef,
    rows: &[kernel_query::Row],
    kind: DurableRelationLayoutKind,
) -> Result<NativeRelation, PhysicalExecutionError> {
    match kind {
        DurableRelationLayoutKind::RowStore => Ok(NativeRelation::row_store(rows.to_vec())),
        DurableRelationLayoutKind::ValueColumnar => {
            NativeRelation::columnar(transpose_value_rows(rows, relation.columns.len())?)
        }
        DurableRelationLayoutKind::I64Columnar => {
            let values = transpose_value_rows(rows, relation.columns.len())?;
            let columns = values
                .into_iter()
                .map(|column| {
                    column
                        .into_iter()
                        .map(|value| match value {
                            Value::I64(value) => Ok(value),
                            _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            NativeRelation::i64_columnar(columns)
        }
        DurableRelationLayoutKind::TypedColumnar => {
            use kernel_schema::{ScalarType, TypeExpr};
            if rows.iter().any(|row| row.len() != relation.columns.len()) {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            let dense = revision.dense_entity_ids();
            let mut columns = Vec::with_capacity(relation.columns.len());
            for (column_index, ty) in relation.columns.iter().enumerate() {
                let values = rows
                    .iter()
                    .map(|row| row[column_index].clone())
                    .collect::<Vec<_>>();
                let column = match ty {
                    TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type)) => {
                        let external = values
                            .into_iter()
                            .map(|value| match value {
                                Value::LiveEntityRef {
                                    entity_type: value_type,
                                    id,
                                } if value_type == *entity_type => Ok(id),
                                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        NativeColumn::dense_live_entity_ids(
                            *entity_type,
                            Arc::clone(&dense),
                            external,
                        )?
                    }
                    _ => NativeColumn::from_typed_values(&values, ty)?,
                };
                columns.push(column);
            }
            NativeRelation::typed_columnar(columns)
        }
    }
}
