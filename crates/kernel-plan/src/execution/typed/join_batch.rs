#[derive(Debug, Clone, Copy)]
struct JoinBatchColumn {
    side: usize,
    column: usize,
}

#[derive(Debug)]
struct JoinBatchPredicate {
    column: JoinBatchColumn,
    expected: i64,
}

#[derive(Debug)]
struct IndexedJoinBatchProgram<'a> {
    left: &'a InstalledRelation,
    right: &'a InstalledRelation,
    left_columns: Vec<NativeI64ColumnView<'a>>,
    right_columns: Vec<NativeI64ColumnView<'a>>,
    left_key_column: usize,
    right_key_column: usize,
    equivalence: SemanticId,
    right_relation: SemanticId,
    right_layout: LayoutBinding,
    columns: Vec<JoinBatchColumn>,
    predicates: Vec<JoinBatchPredicate>,
    downstream_ops: usize,
    projected: bool,
}

enum JoinBatchIndex<'a> {
    Persisted(I64IndexCapability<'a>),
    Ephemeral(BTreeMap<i64, Vec<usize>>),
}

fn select_join_batch_index<'a>(
    program: &IndexedJoinBatchProgram<'a>,
    store: &'a PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<JoinBatchIndex<'a>>, PhysicalExecutionError> {
    let left_keys = program
        .left_columns
        .get(program.left_key_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let right_keys = program
        .right_columns
        .get(program.right_key_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let decision = right_scan_join_access_decision(
        RightJoinAccessRequest {
            left_rows: left_keys.len(),
            right_relation: program.right_relation,
            right_layout: program.right_layout,
            right_column: program.right_key_column,
            equivalence: program.equivalence,
            allow_ephemeral: true,
        },
        store,
        context,
        registry,
    )?;
    match decision.family {
        JoinAccessFamily::PersistedI64 => {
            let index = store
                .i64_index_capability(I64IndexBinding {
                    relation: program.right_relation,
                    layout: program.right_layout,
                    key_column: program.right_key_column,
                    equivalence: program.equivalence,
                })
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
            Ok(Some(JoinBatchIndex::Persisted(index)))
        }
        JoinAccessFamily::EphemeralI64 => {
            let mut index = BTreeMap::<i64, Vec<usize>>::new();
            for position in program.right.scan_positions() {
                index
                    .entry(right_keys.value(position))
                    .or_default()
                    .push(position);
            }
            stats.ephemeral_index_builds = stats.ephemeral_index_builds.saturating_add(1);
            stats.scanned_rows = stats.scanned_rows.saturating_add(right_keys.len());
            stats.values_read = stats.values_read.saturating_add(right_keys.len());
            Ok(Some(JoinBatchIndex::Ephemeral(index)))
        }
        JoinAccessFamily::FullScan | JoinAccessFamily::PersistedSemantic => Ok(None),
    }
}

fn try_execute_indexed_join_batch_chain(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let Some(program) = build_indexed_join_batch_program(plan, store, context, registry)? else {
        return Ok(None);
    };
    if program.downstream_ops == 0 {
        return Ok(None);
    }
    let left_keys = program
        .left_columns
        .get(program.left_key_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let mut stats = ExecutionStats::default();
    let Some(index) = select_join_batch_index(&program, store, context, registry, &mut stats)?
    else {
        return Ok(None);
    };

    let mut out = Vec::new();
    for left_position in program.left.scan_positions() {
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(1);
        let key = left_keys[left_position];
        match &index {
            JoinBatchIndex::Persisted(index) => {
                if let Some(right_row_ids) = index.probe(key) {
                    for right_row_id in right_row_ids {
                        let right_position = program
                            .right
                            .position(right_row_id)
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                        execute_join_batch_candidate(
                            &program,
                            left_position,
                            right_position,
                            &mut out,
                            &mut stats,
                        )?;
                    }
                }
            }
            JoinBatchIndex::Ephemeral(ephemeral) => {
                if let Some(right_positions) = ephemeral.get(&key) {
                    for &right_position in right_positions {
                        execute_join_batch_candidate(
                            &program,
                            left_position,
                            right_position,
                            &mut out,
                            &mut stats,
                        )?;
                    }
                }
            }
        }
    }
    stats.typed_batch_chain_hits = stats.typed_batch_chain_hits.saturating_add(1);
    if program.projected {
        stats.fused_join_project_hits = stats.fused_join_project_hits.saturating_add(1);
    }
    Ok(Some((out, stats)))
}

fn execute_join_batch_candidate(
    program: &IndexedJoinBatchProgram<'_>,
    left_position: usize,
    right_position: usize,
    out: &mut Vec<kernel_query::Row>,
    stats: &mut ExecutionStats,
) -> Result<(), PhysicalExecutionError> {
    for predicate in &program.predicates {
        stats.values_read = stats.values_read.saturating_add(1);
        let candidate =
            join_batch_i64_value(program, predicate.column, left_position, right_position)?;
        if candidate != predicate.expected {
            return Ok(());
        }
    }
    let mut row = Vec::with_capacity(program.columns.len());
    for &column in &program.columns {
        let value = join_batch_i64_value(program, column, left_position, right_position)?;
        stats.values_read = stats.values_read.saturating_add(1);
        row.push(Value::I64(value));
    }
    out.push(row);
    Ok(())
}

fn join_batch_i64_value(
    program: &IndexedJoinBatchProgram<'_>,
    column: JoinBatchColumn,
    left_position: usize,
    right_position: usize,
) -> Result<i64, PhysicalExecutionError> {
    let (columns, position) = if column.side == 0 {
        (&program.left_columns, left_position)
    } else {
        (&program.right_columns, right_position)
    };
    columns
        .get(column.column)
        .and_then(|values| values.get(position))
        .copied()
        .ok_or(RelQueryError::ColumnOutOfBounds.into())
}

fn build_indexed_join_batch_program<'a>(
    plan: &Plan,
    store: &'a PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<IndexedJoinBatchProgram<'a>>, PhysicalExecutionError> {
    match plan {
        Plan::Project { input, columns } => {
            let Some(mut program) =
                build_indexed_join_batch_program(input, store, context, registry)?
            else {
                return Ok(None);
            };
            let mut projected = Vec::with_capacity(columns.len());
            for &column in columns {
                projected.push(
                    *program
                        .columns
                        .get(column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                );
            }
            program.columns = projected;
            program.downstream_ops = program.downstream_ops.saturating_add(1);
            program.projected = true;
            Ok(Some(program))
        }
        Plan::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            let Some(mut program) =
                build_indexed_join_batch_program(input, store, context, registry)?
            else {
                return Ok(None);
            };
            if *equivalence != program.equivalence {
                let Some(resolved) =
                    registry.resolve_primitive_equivalence(context, *equivalence)?
                else {
                    return Ok(None);
                };
                if !matches!(
                    resolved.bind_right(value),
                    Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
                ) {
                    return Ok(None);
                }
            }
            let expected = match value {
                Value::I64(value) => *value,
                _ => return Ok(None),
            };
            let column = *program
                .columns
                .get(*column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            program
                .predicates
                .push(JoinBatchPredicate { column, expected });
            program.downstream_ops = program.downstream_ops.saturating_add(1);
            Ok(Some(program))
        }
        Plan::PromoteToBag(input) => {
            let Some(mut program) =
                build_indexed_join_batch_program(input, store, context, registry)?
            else {
                return Ok(None);
            };
            program.downstream_ops = program.downstream_ops.saturating_add(1);
            Ok(Some(program))
        }
        Plan::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => build_indexed_join_batch_base(
            left,
            right,
            *left_column,
            *right_column,
            *equivalence,
            store,
            context,
            registry,
        ),
        Plan::Scan { .. }
        | Plan::FilterEqColumns { .. }
        | Plan::Difference { .. }
        | Plan::AntiJoin { .. }
        | Plan::Distinct { .. }
        | Plan::Group { .. }
        | Plan::TopKWithTies { .. } => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_indexed_join_batch_base<'a>(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &'a PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<IndexedJoinBatchProgram<'a>>, PhysicalExecutionError> {
    let Some((left_relation, left_layout)) = transparent_direct_scan_relation(left, store) else {
        return Ok(None);
    };
    let Some((right_relation, right_layout)) = transparent_direct_scan_relation(right, store)
    else {
        return Ok(None);
    };
    let Some(resolved) = registry.resolve_primitive_equivalence(context, equivalence)? else {
        return Ok(None);
    };
    if !matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
    ) {
        return Ok(None);
    }
    let left_installed = store.installed(left_relation, left_layout)?;
    let right_installed = store.installed(right_relation, right_layout)?;
    validate_indexable_i64_relation(context, left_relation, &left_installed.data)?;
    validate_indexable_i64_relation(context, right_relation, &right_installed.data)?;
    let Some(left_columns) = native_all_i64_columns(&left_installed.data) else {
        return Ok(None);
    };
    let Some(right_columns) = native_all_i64_columns(&right_installed.data) else {
        return Ok(None);
    };
    if left_column >= left_columns.len() || right_column >= right_columns.len() {
        return Err(RelQueryError::ColumnOutOfBounds.into());
    }
    let columns = (0..left_columns.len())
        .map(|column| JoinBatchColumn { side: 0, column })
        .chain((0..right_columns.len()).map(|column| JoinBatchColumn { side: 1, column }))
        .collect();
    Ok(Some(IndexedJoinBatchProgram {
        left: left_installed,
        right: right_installed,
        left_columns,
        right_columns,
        left_key_column: left_column,
        right_key_column: right_column,
        equivalence,
        right_relation,
        right_layout,
        columns,
        predicates: Vec::new(),
        downstream_ops: 0,
        projected: false,
    }))
}

