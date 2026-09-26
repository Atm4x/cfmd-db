struct RightScanJoinInputs<'a> {
    left_rows: &'a [kernel_query::Row],
    right: &'a InstalledRelation,
    right_relation: SemanticId,
    right_layout: LayoutBinding,
    key: JoinKeySpec,
}

fn try_join_rows_with_persisted_i64(
    inputs: &RightScanJoinInputs<'_>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    store: &PhysicalStore,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some(resolved) = resolved else {
        return Ok(None);
    };
    if !matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
    ) {
        return Ok(None);
    }
    let Some(index) = store.i64_index_capability(I64IndexBinding {
        relation: inputs.right_relation,
        layout: inputs.right_layout,
        key_column: inputs.key.right_column,
        equivalence: inputs.key.equivalence,
    }) else {
        return Ok(None);
    };

    let mut out = Vec::new();
    for left_row in inputs.left_rows {
        let Some(Value::I64(key)) = left_row.get(inputs.key.left_column) else {
            return Err(RelQueryError::ColumnOutOfBounds.into());
        };
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(1);
        if let Some(right_ids) = index.probe(*key) {
            for right_id in right_ids {
                let position = inputs
                    .right
                    .position(right_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let right_row = materialize_native_row(&inputs.right.data, position)?;
                let mut row = left_row.clone();
                row.extend(right_row);
                stats.values_read = stats.values_read.saturating_add(row.len());
                out.push(row);
            }
        }
    }
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(out))
}

fn try_join_rows_with_persisted_semantic(
    inputs: &RightScanJoinInputs<'_>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let binding = SemanticIndexBinding::single(
        inputs.right_relation,
        inputs.right_layout,
        inputs.key.right_column,
        inputs.key.equivalence,
    );
    let Some(index) = store.semantic_fiber_capability(&binding, context, registry)? else {
        return Ok(None);
    };
    let Some(resolved) = resolved else {
        return Ok(None);
    };

    let mut out = Vec::new();
    let mut probe_scratch = SemanticFiberProbeScratch::default();
    for left_row in inputs.left_rows {
        let left_key = left_row
            .get(inputs.key.left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(1);
        if let Some(right_ids) = index.probe_row_columns_with_scratch(
            left_row,
            std::iter::once(inputs.key.left_column),
            context,
            registry,
            &mut probe_scratch,
        )? {
            for right_id in right_ids {
                let position = inputs
                    .right
                    .position(right_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let right_row = materialize_native_row(&inputs.right.data, position)?;
                let right_key = right_row
                    .get(inputs.key.right_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                if !resolved.equivalent(left_key, right_key)? {
                    return Err(RelQueryError::InconsistentIncrementalDelta.into());
                }
                let mut row = left_row.clone();
                row.extend(right_row);
                stats.values_read = stats
                    .values_read
                    .saturating_add(row.len().saturating_add(1));
                out.push(row);
            }
        }
    }
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(out))
}

fn join_rows_with_ephemeral_i64(
    inputs: &RightScanJoinInputs<'_>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some(resolved) = resolved else {
        return Ok(None);
    };
    if !matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
    ) {
        return Ok(None);
    }
    let Some(right_keys) = native_i64_column(&inputs.right.data, inputs.key.right_column) else {
        return Ok(None);
    };
    let mut index = BTreeMap::<i64, Vec<usize>>::new();
    for position in inputs.right.scan_positions() {
        index
            .entry(right_keys.value(position))
            .or_default()
            .push(position);
    }
    stats.ephemeral_index_builds = stats.ephemeral_index_builds.saturating_add(1);
    let mut out = Vec::new();
    for left_row in inputs.left_rows {
        let Some(Value::I64(key)) = left_row.get(inputs.key.left_column) else {
            return Err(RelQueryError::ColumnOutOfBounds.into());
        };
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(1);
        if let Some(right_positions) = index.get(key) {
            for &position in right_positions {
                let right_row = materialize_native_row(&inputs.right.data, position)?;
                let mut row = left_row.clone();
                row.extend(right_row);
                stats.values_read = stats.values_read.saturating_add(row.len());
                out.push(row);
            }
        }
    }
    Ok(Some(out))
}

fn try_execute_indexed_right_scan_join_from_rows(
    left_rows: &[kernel_query::Row],
    right: &Plan,
    key: JoinKeySpec,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some((right_relation, right_layout)) = transparent_direct_scan_relation(right, store)
    else {
        return Ok(None);
    };
    let right_installed = store.installed(right_relation, right_layout)?;
    let resolved = registry.resolve_primitive_equivalence(context, key.equivalence)?;
    let inputs = RightScanJoinInputs {
        left_rows,
        right: right_installed,
        right_relation,
        right_layout,
        key,
    };
    let decision = right_scan_join_access_decision(
        RightJoinAccessRequest {
            left_rows: left_rows.len(),
            right_relation,
            right_layout,
            right_column: key.right_column,
            equivalence: key.equivalence,
            allow_ephemeral: true,
        },
        store,
        context,
        registry,
    )?;
    match decision.family {
        JoinAccessFamily::FullScan => Ok(None),
        JoinAccessFamily::PersistedI64 => {
            try_join_rows_with_persisted_i64(&inputs, resolved.as_ref(), store, stats)
        }
        JoinAccessFamily::PersistedSemantic => try_join_rows_with_persisted_semantic(
            &inputs,
            resolved.as_ref(),
            store,
            context,
            registry,
            stats,
        ),
        JoinAccessFamily::EphemeralI64 => {
            join_rows_with_ephemeral_i64(&inputs, resolved.as_ref(), stats)
        }
    }
}

