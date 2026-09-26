#[derive(Debug)]
struct OwnedTypedBatch {
    columns: Vec<NativeColumn>,
    positions: Vec<usize>,
    stats: ExecutionStats,
}

impl OwnedTypedBatch {
    fn dense(
        columns: Vec<NativeColumn>,
        stats: ExecutionStats,
    ) -> Result<Self, PhysicalExecutionError> {
        let len = columns.first().map_or(0, NativeColumn::len);
        if columns.iter().any(|column| column.len() != len) {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        Ok(Self {
            columns,
            positions: (0..len).collect(),
            stats,
        })
    }

    fn project(&mut self, projection: &[usize]) -> Result<(), PhysicalExecutionError> {
        let mut columns = Vec::with_capacity(projection.len());
        for &column in projection {
            columns.push(
                self.columns
                    .get(column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)?,
            );
        }
        self.columns = columns;
        Ok(())
    }

    fn filter(
        &mut self,
        column: usize,
        value: &Value,
        equivalence: SemanticId,
        env: &StatefulBatchEnv<'_>,
    ) -> Result<(), PhysicalExecutionError> {
        let physical = self
            .columns
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let Some(resolved) = env
            .registry
            .resolve_primitive_equivalence(env.context, equivalence)?
        else {
            return Err(PhysicalExecutionError::UnsupportedPhysicalPlan);
        };
        let bound = resolved.bind_right(value)?;
        if !typed_column_bound_compatible(physical, &bound) {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        let mut selected = Vec::with_capacity(self.positions.len());
        for &position in &self.positions {
            self.stats.values_read = self.stats.values_read.saturating_add(1);
            if typed_column_matches_bound(physical, position, &bound)? {
                selected.push(position);
            }
        }
        self.positions = selected;
        Ok(())
    }

    fn materialize(mut self) -> (Vec<kernel_query::Row>, ExecutionStats) {
        let mut rows = Vec::with_capacity(self.positions.len());
        for position in self.positions {
            let mut row = Vec::with_capacity(self.columns.len());
            for column in &self.columns {
                row.push(column.value_at(position));
            }
            self.stats.values_read = self.stats.values_read.saturating_add(self.columns.len());
            rows.push(row);
        }
        self.stats.typed_stateful_producer_hits =
            self.stats.typed_stateful_producer_hits.saturating_add(1);
        (rows, self.stats)
    }
}

fn try_execute_typed_stateful_producer_chain(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    if !matches!(
        plan,
        Plan::Project { .. }
            | Plan::FilterEqConst { .. }
            | Plan::FilterEqColumns { .. }
            | Plan::Difference { .. }
            | Plan::AntiJoin { .. }
            | Plan::PromoteToBag(_)
    ) {
        return Ok(None);
    }
    let env = StatefulBatchEnv { context, registry };
    let Some(batch) = try_produce_typed_stateful_batch(plan, store, &env)? else {
        return Ok(None);
    };
    Ok(Some(batch.materialize()))
}

fn try_produce_typed_stateful_batch(
    plan: &Plan,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    match plan {
        Plan::Project { input, columns } => {
            let Some(mut batch) = try_produce_typed_stateful_batch(input, store, env)? else {
                return Ok(None);
            };
            batch.project(columns)?;
            Ok(Some(batch))
        }
        Plan::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            let Some(mut batch) = try_produce_typed_stateful_batch(input, store, env)? else {
                return Ok(None);
            };
            batch.filter(*column, value, *equivalence, env)?;
            Ok(Some(batch))
        }
        Plan::PromoteToBag(input) => try_produce_typed_stateful_batch(input, store, env),
        Plan::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => produce_typed_filter_columns(
            input,
            *left_column,
            *right_column,
            *equivalence,
            store,
            env,
        ),
        Plan::Difference { left, right } => produce_typed_difference(left, right, store, env),
        Plan::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => produce_typed_anti_join(
            left,
            right,
            *left_column,
            *right_column,
            *equivalence,
            store,
            env,
        ),
        Plan::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => produce_typed_group(
            input,
            GroupBatchSpec {
                group_columns,
                group_equivalences,
                aggregate,
            },
            store,
            env,
        ),
        Plan::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => produce_typed_top_k(
            input,
            TopKBatchSpec {
                column: *column,
                ordering: *ordering,
                direction: *direction,
                k: *k,
            },
            store,
            env,
        ),
        Plan::Scan { .. } | Plan::Distinct { .. } | Plan::JoinEq { .. } => Ok(None),
    }
}

fn resolve_typed_canonical_keyer(
    equivalence: SemanticId,
    env: &StatefulBatchEnv<'_>,
) -> Result<ResolvedSemanticIndexKeyPart, PhysicalExecutionError> {
    if let Some(module) = env
        .registry
        .resolve_primitive_equivalence(env.context, equivalence)?
    {
        Ok(ResolvedSemanticIndexKeyPart::Primitive(module))
    } else {
        Ok(ResolvedSemanticIndexKeyPart::Structural(
            env.registry.compile_equivalence(env.context, equivalence)?,
        ))
    }
}

fn typed_column_canonical_key_at(
    column: &NativeColumn,
    position: usize,
    keyer: &ResolvedSemanticIndexKeyPart,
) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
    match keyer {
        ResolvedSemanticIndexKeyPart::Primitive(module) => module
            .canonical_key(&column.value_at(position))
            .map_err(Into::into),
        ResolvedSemanticIndexKeyPart::Structural(compiled) => match column {
            NativeColumn::Algebraic(column) => column.canonical_key_at_compiled(position, compiled),
            _ => compiled
                .canonical_key(&column.value_at(position))
                .map_err(Into::into),
        },
    }
}

fn typed_selection_column<'a>(
    selection: &'a TypedBatchSelection<'_>,
    logical_column: usize,
) -> Result<&'a NativeColumn, PhysicalExecutionError> {
    let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data else {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    };
    let physical_column = *selection
        .program
        .columns
        .get(logical_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    columns
        .get(physical_column)
        .ok_or(PhysicalExecutionError::ColumnShapeMismatch)
}

fn typed_selection_row_key(
    selection: &TypedBatchSelection<'_>,
    position: usize,
    keyers: &[ResolvedSemanticIndexKeyPart],
) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
    if keyers.len() != selection.program.columns.len() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    selection
        .program
        .columns
        .iter()
        .zip(keyers)
        .map(|(&physical_column, keyer)| {
            let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data
            else {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            };
            let column = columns
                .get(physical_column)
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            typed_column_canonical_key_at(column, position, keyer)
        })
        .collect()
}

fn typed_selection_into_owned(
    selection: TypedBatchSelection<'_>,
) -> Result<OwnedTypedBatch, PhysicalExecutionError> {
    let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data else {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    };
    let projected = selection
        .program
        .columns
        .iter()
        .map(|&physical_column| {
            columns
                .get(physical_column)
                .cloned()
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OwnedTypedBatch {
        columns: projected,
        positions: selection.positions,
        stats: selection.stats,
    })
}

// HOSTILE[P164][ACTIVE][CLEAN]: same-row Γ equality stays column-native and preserves
// physical positions for downstream Project/Group/TopK instead of materializing input rows.
fn produce_typed_filter_columns(
    input: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    let Some(mut selection) =
        execute_typed_batch_selection(input, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let keyer = resolve_typed_canonical_keyer(equivalence, env)?;
    let mut kept = Vec::with_capacity(selection.positions.len());
    for &position in &selection.positions {
        let left_key = typed_column_canonical_key_at(
            typed_selection_column(&selection, left_column)?,
            position,
            &keyer,
        )?;
        let right_key = typed_column_canonical_key_at(
            typed_selection_column(&selection, right_column)?,
            position,
            &keyer,
        )?;
        selection.stats.values_read = selection.stats.values_read.saturating_add(2);
        if left_key == right_key {
            kept.push(position);
        }
    }
    selection.positions = kept;
    typed_selection_into_owned(selection).map(Some)
}

// HOSTILE[P164][ACTIVE][CLEAN:P163.F]: typed Difference performs Γ-canonical monus/set
// subtraction over native columns and physical positions; it never reconstructs both input row vectors.
fn produce_typed_difference(
    left: &Plan,
    right: &Plan,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    let Some(mut left_selection) =
        execute_typed_batch_selection(left, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let Some(mut right_selection) =
        execute_typed_batch_selection(right, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let left_type = left
        .to_logical_expr()
        .typecheck(env.context, env.registry)?;
    let right_type = right
        .to_logical_expr()
        .typecheck(env.context, env.registry)?;
    if left_type != right_type {
        return Err(RelQueryError::TypeMismatch.into());
    }
    let equivalences = match &left_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    let keyers = equivalences
        .iter()
        .copied()
        .map(|equivalence| resolve_typed_canonical_keyer(equivalence, env))
        .collect::<Result<Vec<_>, _>>()?;
    let mut blockers = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
    for &position in &right_selection.positions {
        let key = typed_selection_row_key(&right_selection, position, &keyers)?;
        right_selection.stats.values_read = right_selection
            .stats
            .values_read
            .saturating_add(keyers.len());
        *blockers.entry(key).or_default() += 1;
    }
    let is_set = matches!(
        left_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    let mut kept = Vec::with_capacity(left_selection.positions.len());
    for &position in &left_selection.positions {
        let key = typed_selection_row_key(&left_selection, position, &keyers)?;
        left_selection.stats.values_read = left_selection
            .stats
            .values_read
            .saturating_add(keyers.len());
        let blocked = if is_set {
            blockers.contains_key(&key)
        } else if let Some(count) = blockers.get_mut(&key)
            && *count > 0
        {
            *count -= 1;
            true
        } else {
            false
        };
        if !blocked {
            kept.push(position);
        }
    }
    left_selection.positions = kept;
    merge_execution_stats(&mut left_selection.stats, right_selection.stats);
    typed_selection_into_owned(left_selection).map(Some)
}

// HOSTILE[P164][ACTIVE][CLEAN:P163.F]: typed AntiJoin stores only right Γ-canonical blocker
// keys and preserves left physical positions until the surviving output is actually consumed.
fn produce_typed_anti_join(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    let Some(mut left_selection) =
        execute_typed_batch_selection(left, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let Some(mut right_selection) =
        execute_typed_batch_selection(right, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let keyer = resolve_typed_canonical_keyer(equivalence, env)?;
    let mut blocked = BTreeSet::new();
    for &position in &right_selection.positions {
        let key = typed_column_canonical_key_at(
            typed_selection_column(&right_selection, right_column)?,
            position,
            &keyer,
        )?;
        blocked.insert(key);
        right_selection.stats.values_read = right_selection.stats.values_read.saturating_add(1);
    }
    let mut kept = Vec::with_capacity(left_selection.positions.len());
    for &position in &left_selection.positions {
        let key = typed_column_canonical_key_at(
            typed_selection_column(&left_selection, left_column)?,
            position,
            &keyer,
        )?;
        left_selection.stats.values_read = left_selection.stats.values_read.saturating_add(1);
        if !blocked.contains(&key) {
            kept.push(position);
        }
    }
    left_selection.positions = kept;
    merge_execution_stats(&mut left_selection.stats, right_selection.stats);
    typed_selection_into_owned(left_selection).map(Some)
}

// HOSTILE[P181][ACTIVE][CLEAN]: stateful Group producer family is isolated from filter/difference/anti-join state.
include!("stateful_group.rs");

// HOSTILE[P181][ACTIVE][CLEAN]: stateful TopK producer family is isolated from the general producer recursion.
include!("stateful_top_k.rs");
