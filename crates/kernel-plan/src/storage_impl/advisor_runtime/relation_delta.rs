fn full_row_occurrence_binding(
    relation: SemanticId,
    layout: LayoutBinding,
    context: &kernel_schema::SemanticContext,
) -> Result<SemanticIndexBinding, PhysicalExecutionError> {
    let definition = context
        .schema
        .relation(relation)
        .ok_or(RelQueryError::UnknownRelation(relation))?;
    let equivalences = match &definition.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    if equivalences.len() != definition.columns.len() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    Ok(SemanticIndexBinding {
        relation,
        layout,
        key_parts: equivalences
            .iter()
            .copied()
            .enumerate()
            .map(|(column, equivalence)| SemanticIndexKeyPart {
                column,
                equivalence,
            })
            .collect(),
    })
}

fn plan_installed_relation_delta(
    relation: &InstalledRelation,
    candidate_index: Option<&MaterializedI64IndexState>,
    occurrence_atom: Option<&MaterializedObservableAtomState>,
    relation_id: SemanticId,
    delta: &RelationDelta,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<PhysicalRelationDelta, PhysicalExecutionError> {
    let definition = context
        .schema
        .relation(relation_id)
        .ok_or(RelQueryError::UnknownRelation(relation_id))?;
    let expected_type = RelType {
        columns: definition.columns.clone(),
        semantics: definition.semantics.clone(),
    };
    if delta.result_type != expected_type {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    let equivalences = match &expected_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };

    let mut physical = PhysicalRelationDelta {
        removed: Vec::with_capacity(delta.removed.len()),
        inserted: Vec::with_capacity(delta.inserted.len()),
    };
    let mut reserved = BTreeSet::new();
    let mut occurrence_probe_classes = Vec::new();
    // The full-row observable atom is a maintained Γ-quotient occurrence directory.
    // It makes both single and batched removals class-local after one warm-up build;
    // reserved handles preserve the exact first-unused occurrence law for Bag rows.
    for removed in &delta.removed {
        let found = if let Some(atom) = occurrence_atom {
            atom.probe_row_fiber_with_scratch(
                removed,
                context,
                registry,
                &mut occurrence_probe_classes,
            )?
                .and_then(|ids| ids.into_iter().copied().find(|row_id| !reserved.contains(row_id)))
        } else {
            find_removed_row_id(
                relation,
                candidate_index,
                removed,
                &reserved,
                equivalences,
                context,
                registry,
            )?
        };
        let row_id = found.ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if relation
            .slots
            .get(row_id.slot)
            .is_some_and(|slot| slot.generation == u64::MAX)
        {
            return Err(PhysicalExecutionError::HandleGenerationExhausted);
        }
        reserved.insert(row_id);
        physical.removed.push((row_id, removed.clone()));
    }
    let removed_ids: Vec<_> = physical.removed.iter().map(|(row_id, _)| *row_id).collect();
    let planned_ids = relation.planned_insert_ids(&removed_ids, delta.inserted.len())?;
    for (row_id, inserted) in planned_ids.into_iter().zip(&delta.inserted) {
        validate_native_row(&relation.data, inserted)?;
        physical.inserted.push((row_id, inserted.clone()));
    }
    Ok(physical)
}

fn find_removed_row_id(
    relation: &InstalledRelation,
    candidate_index: Option<&MaterializedI64IndexState>,
    removed: &kernel_query::Row,
    reserved: &BTreeSet<PhysicalRowId>,
    equivalences: &[SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<PhysicalRowId>, PhysicalExecutionError> {
    if let Some(index) = candidate_index
        && let Some(Value::I64(key)) = removed.get(index.binding.key_column)
        && let Some(candidates) = index.probe(*key)
    {
        for row_id in candidates {
            if reserved.contains(row_id) {
                continue;
            }
            let Some(position) = relation.position(*row_id) else {
                continue;
            };
            let existing = materialize_native_row(&relation.data, position)?;
            if semantic_rows_equal(&existing, removed, equivalences, context, registry)? {
                return Ok(Some(*row_id));
            }
        }
        return Ok(None);
    }
    for position in 0..native_row_count(&relation.data) {
        let row_id = relation.row_id_at(position)?;
        if reserved.contains(&row_id) {
            continue;
        }
        let existing = materialize_native_row(&relation.data, position)?;
        if semantic_rows_equal(&existing, removed, equivalences, context, registry)? {
            return Ok(Some(row_id));
        }
    }
    Ok(None)
}

fn apply_planned_relation_delta(
    relation: &mut InstalledRelation,
    delta: &PhysicalRelationDelta,
) -> Result<(), PhysicalExecutionError> {
    // HOSTILE[P193][ACTIVE][CLEAN-ASYMPTOTIC]: InstalledRelation owns the choice between
    // constant-time primitive removals and one survivor rebuild for nested algebraic batches.
    relation.remove_rows(
        &delta
            .removed
            .iter()
            .map(|(row_id, _)| *row_id)
            .collect::<Vec<_>>(),
    )?;
    for (expected_id, row) in &delta.inserted {
        let inserted = relation.push_row(row)?;
        if inserted != *expected_id {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
    }
    Ok(())
}


fn native_semantic_column_work_units(
    data: &NativeRelation,
    column: usize,
) -> Result<usize, PhysicalExecutionError> {
    match data {
        NativeRelation::RowStore(rows) => rows.iter().try_fold(0_usize, |work, row| {
            let value = row
                .get(column)
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            Ok(work.saturating_add(
                kernel_semantics::semantic_value_work_estimate(value).total_units(),
            ))
        }),
        NativeRelation::Columnar { columns, .. } => {
            let values = columns
                .get(column)
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            Ok(values.iter().fold(0_usize, |work, value| {
                work.saturating_add(
                    kernel_semantics::semantic_value_work_estimate(value).total_units(),
                )
            }))
        }
        NativeRelation::I64Columnar { columns, row_count } => {
            columns
                .get(column)
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            Ok(*row_count)
        }
        NativeRelation::TypedColumnar { columns, row_count } => {
            let column = columns
                .get(column)
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            (0..*row_count).try_fold(0_usize, |work, row_index| {
                Ok(work.saturating_add(column.semantic_work_units_at(row_index)?))
            })
        }
    }
}

