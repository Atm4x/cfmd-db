use std::collections::{BTreeMap, BTreeSet};

use kernel_model::Value;
use kernel_query::{
    AggregateSpec, CanonicalRowKey, CertifiedCanonicalRowKey, OrderDirection, RelExpr,
    RelQueryError, RelType, RelationOccurrenceCertificate, RelationRowCanonicalizer,
    canonical_row_key,
};
use kernel_schema::{RelationSemantics, SemanticContext};
use kernel_semantics::{CanonicalEqKey, SemanticRegistry};
use kernel_types::SemanticId;

use crate::{
    FactorizedRealizationRoot, PhysicalAtomId, PhysicalAtomStore, PreparedFactorizedRelation,
    RealizationError,
};

/// Storage-neutral one-shot source boundary for global relational preparation.
///
/// Implementations yield one transient semantic row at a time. They never own
/// maintained query state and never require a full source `FiniteModel`.
pub trait RelExecutionSource {
    fn visit_relation_rows(
        &self,
        relation: SemanticId,
        visit: &mut dyn FnMut(Vec<Value>) -> Result<(), RealizationError>,
    ) -> Result<(), RealizationError>;

    fn visit_relation_column(
        &self,
        relation: SemanticId,
        ordinal: usize,
        visit: &mut dyn FnMut(Value),
    ) -> Result<(), RealizationError>;

    fn relation_arity(&self, relation: SemanticId) -> Result<usize, RealizationError>;

    fn relation_dependencies(
        &self,
        relation: SemanticId,
    ) -> Result<BTreeSet<PhysicalAtomId>, RealizationError>;
}

struct FactorizedRelExecutionSource<'a> {
    root: &'a FactorizedRealizationRoot,
    atoms: &'a PhysicalAtomStore,
}

impl RelExecutionSource for FactorizedRelExecutionSource<'_> {
    fn visit_relation_rows(
        &self,
        relation: SemanticId,
        visit: &mut dyn FnMut(Vec<Value>) -> Result<(), RealizationError>,
    ) -> Result<(), RealizationError> {
        self.root
            .visit_execution_relation_rows(self.atoms, relation, visit)
    }

    fn visit_relation_column(
        &self,
        relation: SemanticId,
        ordinal: usize,
        visit: &mut dyn FnMut(Value),
    ) -> Result<(), RealizationError> {
        self.root
            .visit_execution_relation_column(self.atoms, relation, ordinal, visit)
    }

    fn relation_arity(&self, relation: SemanticId) -> Result<usize, RealizationError> {
        self.root.execution_relation_arity(relation)
    }

    fn relation_dependencies(
        &self,
        relation: SemanticId,
    ) -> Result<BTreeSet<PhysicalAtomId>, RealizationError> {
        self.root.execution_relation_dependencies(relation)
    }
}

/// One-shot target boundary. The sink is columnar authority; rows are only
/// transient operator values and are never staged as a complete target table.
pub trait RelExecutionSink {
    fn push_row(&mut self, row: Vec<Value>) -> Result<(), RealizationError>;
    fn push_column_value(&mut self, ordinal: usize, value: Value) -> Result<(), RealizationError>;
}

struct ColumnarRelExecutionSink {
    columns: Vec<Vec<Value>>,
}

impl ColumnarRelExecutionSink {
    fn new(arity: usize) -> Self {
        Self {
            columns: (0..arity).map(|_| Vec::new()).collect(),
        }
    }

    fn into_columns(self) -> Vec<Vec<Value>> {
        self.columns
    }
}

impl RelExecutionSink for ColumnarRelExecutionSink {
    fn push_row(&mut self, row: Vec<Value>) -> Result<(), RealizationError> {
        if row.len() != self.columns.len() {
            return Err(RealizationError::OneShotRelationArityMismatch);
        }
        for (column, value) in self.columns.iter_mut().zip(row) {
            column.push(value);
        }
        Ok(())
    }

    fn push_column_value(&mut self, ordinal: usize, value: Value) -> Result<(), RealizationError> {
        self.columns
            .get_mut(ordinal)
            .ok_or(RealizationError::OneShotRelationArityMismatch)?
            .push(value);
        Ok(())
    }
}

fn equivalences(relation_type: &RelType) -> &[SemanticId] {
    match &relation_type.semantics {
        RelationSemantics::Set {
            column_equivalences,
        }
        | RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    }
}

fn canonical_key(
    row: &Vec<Value>,
    relation_type: &RelType,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<CanonicalRowKey, RealizationError> {
    canonical_row_key(row, equivalences(relation_type), context, registry)
        .map_err(RealizationError::RelationQuery)
}

fn can_emit_certified_output(
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<bool, RealizationError> {
    match expr {
        RelExpr::Union { .. } => Ok(matches!(
            expr.typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?
                .semantics,
            RelationSemantics::Set { .. }
        )),
        RelExpr::Difference { .. } | RelExpr::Distinct { .. } => Ok(true),
        RelExpr::Project { .. } => Ok(matches!(
            expr.typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?
                .semantics,
            RelationSemantics::Set { .. }
        )),
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterOrderConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. } => {
            can_emit_certified_output(input, context, registry)
        }
        RelExpr::AntiJoin { left, .. } => can_emit_certified_output(left, context, registry),
        _ => Ok(false),
    }
}

/// Evaluates the current relational IR directly against a factorized physical
/// realization without materializing a full logical `DatabaseState`.
///
/// The result rows are caller-owned query output. Intermediate memory is only
/// the inherent state of the selected one-shot operators.
pub fn evaluate_relation_expr_factorized(
    root: &FactorizedRealizationRoot,
    atoms: &PhysicalAtomStore,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    expr: &RelExpr,
) -> Result<Vec<Vec<Value>>, RealizationError> {
    let source = FactorizedRelExecutionSource { root, atoms };
    let mut rows = Vec::new();
    execute_expr(&source, expr, context, registry, &mut |row| {
        rows.push(row);
        Ok(())
    })?;
    Ok(rows)
}

fn execute_expr(
    source: &dyn RelExecutionSource,
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    emit: &mut dyn FnMut(Vec<Value>) -> Result<(), RealizationError>,
) -> Result<(), RealizationError> {
    match expr {
        RelExpr::Scan(relation) => source.visit_relation_rows(*relation, emit),
        RelExpr::Union { left, right } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            match result_type.semantics {
                RelationSemantics::Bag { .. } => {
                    execute_expr(source, left, context, registry, emit)?;
                    execute_expr(source, right, context, registry, emit)
                }
                RelationSemantics::Set { .. } => {
                    let mut seen = BTreeSet::<CanonicalRowKey>::new();
                    let mut accept = |row: Vec<Value>| {
                        let key = canonical_key(&row, &result_type, context, registry)?;
                        if seen.insert(key) {
                            emit(row)?;
                        }
                        Ok(())
                    };
                    execute_expr(source, left, context, registry, &mut accept)?;
                    execute_expr(source, right, context, registry, &mut accept)
                }
            }
        }
        RelExpr::Difference { left, right } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            match result_type.semantics {
                RelationSemantics::Bag { .. } => {
                    let mut blocked = BTreeMap::<CanonicalRowKey, usize>::new();
                    execute_expr(source, right, context, registry, &mut |row| {
                        let key = canonical_key(&row, &result_type, context, registry)?;
                        *blocked.entry(key).or_default() += 1;
                        Ok(())
                    })?;
                    execute_expr(source, left, context, registry, &mut |row| {
                        let key = canonical_key(&row, &result_type, context, registry)?;
                        if let Some(count) = blocked.get_mut(&key)
                            && *count > 0
                        {
                            *count -= 1;
                            return Ok(());
                        }
                        emit(row)
                    })
                }
                RelationSemantics::Set { .. } => {
                    let mut blocked = BTreeSet::<CanonicalRowKey>::new();
                    execute_expr(source, right, context, registry, &mut |row| {
                        blocked.insert(canonical_key(&row, &result_type, context, registry)?);
                        Ok(())
                    })?;
                    execute_expr(source, left, context, registry, &mut |row| {
                        let key = canonical_key(&row, &result_type, context, registry)?;
                        if !blocked.contains(&key) {
                            emit(row)?;
                        }
                        Ok(())
                    })
                }
            }
        }
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            let mut blocked = BTreeSet::<CanonicalEqKey>::new();
            execute_expr(source, right, context, registry, &mut |row| {
                let value = row
                    .get(*right_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                blocked.insert(
                    registry
                        .canonical_equivalence_key(context, *equivalence, value)
                        .map_err(RelQueryError::from)
                        .map_err(RealizationError::RelationQuery)?,
                );
                Ok(())
            })?;
            execute_expr(source, left, context, registry, &mut |row| {
                let value = row
                    .get(*left_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                let key = registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?;
                if !blocked.contains(&key) {
                    emit(row)?;
                }
                Ok(())
            })
        }
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => execute_expr(source, input, context, registry, &mut |row| {
            let candidate = row.get(*column).ok_or(RealizationError::RelationQuery(
                RelQueryError::ColumnOutOfBounds,
            ))?;
            if registry
                .equivalent(context, *equivalence, candidate, value)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?
            {
                emit(row)?;
            }
            Ok(())
        }),
        RelExpr::FilterOrderConst {
            input,
            column,
            value,
            ordering,
            comparison,
        } => {
            let compiled = registry
                .compile_ordering(context, *ordering)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?;
            let threshold = compiled
                .canonical_key(value)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?;
            execute_expr(source, input, context, registry, &mut |row| {
                let candidate = row.get(*column).ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
                let order = compiled
                    .canonical_key(candidate)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?
                    .cmp(&threshold);
                let passes = match comparison {
                    kernel_query::OrderComparison::Less => order.is_lt(),
                    kernel_query::OrderComparison::LessOrEqual => order.is_le(),
                    kernel_query::OrderComparison::Greater => order.is_gt(),
                    kernel_query::OrderComparison::GreaterOrEqual => order.is_ge(),
                };
                if passes {
                    emit(row)?;
                }
                Ok(())
            })
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => execute_expr(source, input, context, registry, &mut |row| {
            let left = row
                .get(*left_column)
                .ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
            let right = row
                .get(*right_column)
                .ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
            if registry
                .equivalent(context, *equivalence, left, right)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?
            {
                emit(row)?;
            }
            Ok(())
        }),
        RelExpr::Project { input, columns } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            match result_type.semantics {
                RelationSemantics::Bag { .. } => {
                    execute_expr(source, input, context, registry, &mut |row| {
                        let projected = columns
                            .iter()
                            .map(|ordinal| {
                                row.get(*ordinal)
                                    .cloned()
                                    .ok_or(RealizationError::RelationQuery(
                                        RelQueryError::ColumnOutOfBounds,
                                    ))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        emit(projected)
                    })
                }
                RelationSemantics::Set { .. } => {
                    let mut seen = BTreeSet::<CanonicalRowKey>::new();
                    execute_expr(source, input, context, registry, &mut |row| {
                        let projected = columns
                            .iter()
                            .map(|ordinal| {
                                row.get(*ordinal)
                                    .cloned()
                                    .ok_or(RealizationError::RelationQuery(
                                        RelQueryError::ColumnOutOfBounds,
                                    ))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let key = canonical_key(&projected, &result_type, context, registry)?;
                        if seen.insert(key) {
                            emit(projected)?;
                        }
                        Ok(())
                    })
                }
            }
        }
        RelExpr::Distinct { input, .. } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let mut seen = BTreeSet::<CanonicalRowKey>::new();
            execute_expr(source, input, context, registry, &mut |row| {
                let key = canonical_key(&row, &result_type, context, registry)?;
                if seen.insert(key) {
                    emit(row)?;
                }
                Ok(())
            })
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            let mut right_buckets = BTreeMap::<CanonicalEqKey, Vec<Vec<Value>>>::new();
            execute_expr(source, right, context, registry, &mut |row| {
                let value = row
                    .get(*right_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                let key = registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?;
                right_buckets.entry(key).or_default().push(row);
                Ok(())
            })?;
            execute_expr(source, left, context, registry, &mut |left_row| {
                let value = left_row
                    .get(*left_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                let key = registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?;
                if let Some(matches) = right_buckets.get(&key) {
                    for right_row in matches {
                        let mut joined = Vec::with_capacity(left_row.len() + right_row.len());
                        joined.extend(left_row.iter().cloned());
                        joined.extend(right_row.iter().cloned());
                        emit(joined)?;
                    }
                }
                Ok(())
            })
        }
        RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => {
            enum State {
                Count(kernel_aggregate::ExactCount),
                ExactF64Sum(kernel_aggregate::ExactF64Sum),
            }

            let mut groups = Vec::<(Vec<Value>, State)>::new();
            let mut lookup = BTreeMap::<CanonicalRowKey, usize>::new();
            execute_expr(source, input, context, registry, &mut |row| {
                let key = group_columns
                    .iter()
                    .map(|column| {
                        row.get(*column)
                            .cloned()
                            .ok_or(RealizationError::RelationQuery(
                                RelQueryError::ColumnOutOfBounds,
                            ))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let canonical = canonical_row_key(&key, group_equivalences, context, registry)
                    .map_err(RealizationError::RelationQuery)?;
                let index = if let Some(index) = lookup.get(&canonical).copied() {
                    index
                } else {
                    let state = match aggregate {
                        AggregateSpec::Count { .. } => {
                            State::Count(kernel_aggregate::ExactCount::default())
                        }
                        AggregateSpec::ExactF64Sum { .. } => {
                            State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
                        }
                    };
                    let index = groups.len();
                    groups.push((key, state));
                    lookup.insert(canonical, index);
                    index
                };
                match (&mut groups[index].1, aggregate) {
                    (State::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
                    (State::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { value_column, .. }) => {
                        let value =
                            row.get(*value_column)
                                .ok_or(RealizationError::RelationQuery(
                                    RelQueryError::ColumnOutOfBounds,
                                ))?;
                        let Value::F64Bits(bits) = value else {
                            return Err(RealizationError::RelationQuery(
                                RelQueryError::TypeMismatch,
                            ));
                        };
                        sum.add(f64::from_bits(*bits))
                            .map_err(RelQueryError::from)
                            .map_err(RealizationError::RelationQuery)?;
                    }
                    _ => unreachable!("aggregate state is created from the same AggregateSpec"),
                }
                Ok(())
            })?;
            if groups.is_empty() && group_columns.is_empty() {
                groups.push((
                    Vec::new(),
                    match aggregate {
                        AggregateSpec::Count { .. } => {
                            State::Count(kernel_aggregate::ExactCount::default())
                        }
                        AggregateSpec::ExactF64Sum { .. } => {
                            State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
                        }
                    },
                ));
            }
            for (mut key, state) in groups {
                let aggregate_value = match state {
                    State::Count(count) => Value::I64(
                        count
                            .finish_i64()
                            .map_err(RelQueryError::from)
                            .map_err(RealizationError::RelationQuery)?,
                    ),
                    State::ExactF64Sum(sum) => Value::F64Bits(sum.finish().to_bits()),
                };
                key.push(aggregate_value);
                emit(key)?;
            }
            Ok(())
        }
        RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => {
            let compiled = registry
                .compile_ordering(context, *ordering)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?;
            let mut buckets =
                BTreeMap::<kernel_semantics::CanonicalOrderKey, Vec<Vec<Value>>>::new();
            let mut retained = 0usize;
            execute_expr(source, input, context, registry, &mut |row| {
                if *k == 0 {
                    return Ok(());
                }
                let value = row.get(*column).ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
                let key = compiled
                    .canonical_key(value)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?;
                buckets.entry(key).or_default().push(row);
                retained = retained
                    .checked_add(1)
                    .ok_or(RealizationError::CompactionCostOverflow)?;
                while retained > *k {
                    let worst = match direction {
                        OrderDirection::Ascending => buckets.last_key_value(),
                        OrderDirection::Descending => buckets.first_key_value(),
                    }
                    .map(|(key, rows)| (key.clone(), rows.len()))
                    .expect("retained row implies a top-k bucket");
                    if retained.saturating_sub(worst.1) < *k {
                        break;
                    }
                    buckets.remove(&worst.0);
                    retained -= worst.1;
                }
                Ok(())
            })?;
            match direction {
                OrderDirection::Ascending => {
                    for rows in buckets.into_values() {
                        for row in rows {
                            emit(row)?;
                        }
                    }
                }
                OrderDirection::Descending => {
                    for (_, rows) in buckets.into_iter().rev() {
                        for row in rows {
                            emit(row)?;
                        }
                    }
                }
            }
            Ok(())
        }
        RelExpr::PromoteToBag(input) => execute_expr(source, input, context, registry, emit),
    }
}

/// Executes the top-level operators whose semantic law already computes the
/// complete Γ row key. The same sealed evidence is returned with each emitted
/// row so the physical witness can adopt it instead of canonicalizing output a
/// second time. `Ok(false)` means this expression has no such lowering; the
/// caller may use the ordinary one-shot executor, never the legacy evaluator.
fn execute_certified_output(
    source: &dyn RelExecutionSource,
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    emit: &mut dyn FnMut(Vec<Value>, CertifiedCanonicalRowKey) -> Result<(), RealizationError>,
) -> Result<bool, RealizationError> {
    match expr {
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => execute_certified_output(source, input, context, registry, &mut |row, evidence| {
            let candidate = row.get(*column).ok_or(RealizationError::RelationQuery(
                RelQueryError::ColumnOutOfBounds,
            ))?;
            if registry
                .equivalent(context, *equivalence, candidate, value)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?
            {
                emit(row, evidence)?;
            }
            Ok(())
        }),
        RelExpr::FilterOrderConst {
            input,
            column,
            value,
            ordering,
            comparison,
        } => {
            let compiled = registry
                .compile_ordering(context, *ordering)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?;
            let threshold = compiled
                .canonical_key(value)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?;
            execute_certified_output(source, input, context, registry, &mut |row, evidence| {
                let candidate = row.get(*column).ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
                let order = compiled
                    .canonical_key(candidate)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?
                    .cmp(&threshold);
                let passes = match comparison {
                    kernel_query::OrderComparison::Less => order.is_lt(),
                    kernel_query::OrderComparison::LessOrEqual => order.is_le(),
                    kernel_query::OrderComparison::Greater => order.is_gt(),
                    kernel_query::OrderComparison::GreaterOrEqual => order.is_ge(),
                };
                if passes {
                    emit(row, evidence)?;
                }
                Ok(())
            })
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => execute_certified_output(source, input, context, registry, &mut |row, evidence| {
            let left = row
                .get(*left_column)
                .ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
            let right = row
                .get(*right_column)
                .ok_or(RealizationError::RelationQuery(
                    RelQueryError::ColumnOutOfBounds,
                ))?;
            if registry
                .equivalent(context, *equivalence, left, right)
                .map_err(RelQueryError::from)
                .map_err(RealizationError::RelationQuery)?
            {
                emit(row, evidence)?;
            }
            Ok(())
        }),
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            if !can_emit_certified_output(left, context, registry)? {
                return Ok(false);
            }
            let mut blocked = BTreeSet::<CanonicalEqKey>::new();
            execute_expr(source, right, context, registry, &mut |row| {
                let value = row
                    .get(*right_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                blocked.insert(
                    registry
                        .canonical_equivalence_key(context, *equivalence, value)
                        .map_err(RelQueryError::from)
                        .map_err(RealizationError::RelationQuery)?,
                );
                Ok(())
            })?;
            execute_certified_output(source, left, context, registry, &mut |row, evidence| {
                let value = row
                    .get(*left_column)
                    .ok_or(RealizationError::RelationQuery(
                        RelQueryError::ColumnOutOfBounds,
                    ))?;
                let key = registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(RelQueryError::from)
                    .map_err(RealizationError::RelationQuery)?;
                if !blocked.contains(&key) {
                    emit(row, evidence)?;
                }
                Ok(())
            })
        }
        RelExpr::Union { left, right } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            if !matches!(result_type.semantics, RelationSemantics::Set { .. }) {
                return Ok(false);
            }
            let canonicalizer = RelationRowCanonicalizer::compile(result_type, context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let mut seen = BTreeSet::<CanonicalRowKey>::new();
            let mut accept = |row: Vec<Value>| {
                let evidence = canonicalizer
                    .certify_row(&row)
                    .map_err(RealizationError::RelationQuery)?;
                if seen.insert(evidence.canonical_key().clone()) {
                    emit(row, evidence)?;
                }
                Ok(())
            };
            execute_expr(source, left, context, registry, &mut accept)?;
            execute_expr(source, right, context, registry, &mut accept)?;
            Ok(true)
        }
        RelExpr::Difference { left, right } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let canonicalizer =
                RelationRowCanonicalizer::compile(result_type.clone(), context, registry)
                    .map_err(RealizationError::RelationQuery)?;
            match result_type.semantics {
                RelationSemantics::Bag { .. } => {
                    let mut blocked = BTreeMap::<CanonicalRowKey, usize>::new();
                    execute_expr(source, right, context, registry, &mut |row| {
                        let evidence = canonicalizer
                            .certify_row(&row)
                            .map_err(RealizationError::RelationQuery)?;
                        *blocked.entry(evidence.canonical_key().clone()).or_default() += 1;
                        Ok(())
                    })?;
                    execute_expr(source, left, context, registry, &mut |row| {
                        let evidence = canonicalizer
                            .certify_row(&row)
                            .map_err(RealizationError::RelationQuery)?;
                        if let Some(count) = blocked.get_mut(evidence.canonical_key())
                            && *count > 0
                        {
                            *count -= 1;
                            return Ok(());
                        }
                        emit(row, evidence)
                    })?;
                }
                RelationSemantics::Set { .. } => {
                    let mut blocked = BTreeSet::<CanonicalRowKey>::new();
                    execute_expr(source, right, context, registry, &mut |row| {
                        let evidence = canonicalizer
                            .certify_row(&row)
                            .map_err(RealizationError::RelationQuery)?;
                        blocked.insert(evidence.canonical_key().clone());
                        Ok(())
                    })?;
                    execute_expr(source, left, context, registry, &mut |row| {
                        let evidence = canonicalizer
                            .certify_row(&row)
                            .map_err(RealizationError::RelationQuery)?;
                        if !blocked.contains(evidence.canonical_key()) {
                            emit(row, evidence)?;
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(true)
        }
        RelExpr::Project { input, columns } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            if !matches!(result_type.semantics, RelationSemantics::Set { .. }) {
                return Ok(false);
            }
            let canonicalizer = RelationRowCanonicalizer::compile(result_type, context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let mut seen = BTreeSet::<CanonicalRowKey>::new();
            execute_expr(source, input, context, registry, &mut |row| {
                let projected = columns
                    .iter()
                    .map(|ordinal| {
                        row.get(*ordinal)
                            .cloned()
                            .ok_or(RealizationError::RelationQuery(
                                RelQueryError::ColumnOutOfBounds,
                            ))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let evidence = canonicalizer
                    .certify_row(&projected)
                    .map_err(RealizationError::RelationQuery)?;
                if seen.insert(evidence.canonical_key().clone()) {
                    emit(projected, evidence)?;
                }
                Ok(())
            })?;
            Ok(true)
        }
        RelExpr::Distinct { input, .. } => {
            let result_type = expr
                .typecheck(context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let canonicalizer = RelationRowCanonicalizer::compile(result_type, context, registry)
                .map_err(RealizationError::RelationQuery)?;
            let mut seen = BTreeSet::<CanonicalRowKey>::new();
            execute_expr(source, input, context, registry, &mut |row| {
                let evidence = canonicalizer
                    .certify_row(&row)
                    .map_err(RealizationError::RelationQuery)?;
                if seen.insert(evidence.canonical_key().clone()) {
                    emit(row, evidence)?;
                }
                Ok(())
            })?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn execute_direct_bag_union_columnar(
    source: &dyn RelExecutionSource,
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    sink: &mut dyn RelExecutionSink,
) -> Result<bool, RealizationError> {
    let RelExpr::Union { left, right } = expr else {
        return Ok(false);
    };
    let (RelExpr::Scan(left_relation), RelExpr::Scan(right_relation)) =
        (left.as_ref(), right.as_ref())
    else {
        return Ok(false);
    };
    let result_type = expr
        .typecheck(context, registry)
        .map_err(RealizationError::RelationQuery)?;
    if !matches!(result_type.semantics, RelationSemantics::Bag { .. }) {
        return Ok(false);
    }
    let arity = result_type.columns.len();
    if source.relation_arity(*left_relation)? != arity
        || source.relation_arity(*right_relation)? != arity
    {
        return Err(RealizationError::OneShotRelationArityMismatch);
    }
    for ordinal in 0..arity {
        for relation in [*left_relation, *right_relation] {
            let mut push = |value| {
                sink.push_column_value(ordinal, value)
                    .expect("validated one-shot sink arity")
            };
            source.visit_relation_column(relation, ordinal, &mut push)?;
        }
    }
    Ok(true)
}

pub(crate) fn prepare_one_shot_relation(
    source_root: &FactorizedRealizationRoot,
    atoms: &mut PhysicalAtomStore,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    target_context: &SemanticContext,
    target_relation: SemanticId,
    expr: &RelExpr,
) -> Result<PreparedFactorizedRelation, RealizationError> {
    let source = FactorizedRelExecutionSource {
        root: source_root,
        atoms,
    };
    let source_relations = expr.scan_relations();
    let mut source_atoms = BTreeSet::new();
    for relation in &source_relations {
        source_atoms.extend(source.relation_dependencies(*relation)?);
    }
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(target_context, registry)
        .map_err(RealizationError::RelationQuery)?;
    let column_order = target_context
        .schema
        .relation_column_ids(target_relation)
        .ok_or(RealizationError::MissingFactorizedRelation(target_relation))?
        .to_vec();
    let mut sink = ColumnarRelExecutionSink::new(column_order.len());
    let mut certified_keys = Vec::<CertifiedCanonicalRowKey>::new();
    let certified_output = execute_certified_output(
        &source,
        expr,
        source_context,
        registry,
        &mut |row, evidence| {
            sink.push_row(row)?;
            certified_keys.push(evidence);
            Ok(())
        },
    )?;
    if !certified_output
        && !execute_direct_bag_union_columnar(&source, expr, source_context, registry, &mut sink)?
    {
        execute_expr(&source, expr, source_context, registry, &mut |row| {
            sink.push_row(row)
        })?;
    }
    let occurrence_certificate = if certified_output && !certified_keys.is_empty() {
        Some(
            RelationOccurrenceCertificate::from_dense_certified_keys(certified_keys)
                .map_err(RealizationError::RelationQuery)?,
        )
    } else {
        None
    };
    PreparedFactorizedRelation::from_execution_columns(
        atoms,
        target_relation,
        source_relations,
        source_atoms,
        column_order,
        sink.into_columns(),
        result_type,
        target_context,
        registry,
        occurrence_certificate,
    )
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use kernel_query::RelationBaseWitness;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, OrderingModule};
    use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId};

    struct MockSource {
        rows: BTreeMap<SemanticId, Vec<Vec<Value>>>,
    }

    impl RelExecutionSource for MockSource {
        fn visit_relation_rows(
            &self,
            relation: SemanticId,
            visit: &mut dyn FnMut(Vec<Value>) -> Result<(), RealizationError>,
        ) -> Result<(), RealizationError> {
            for row in self.rows.get(&relation).into_iter().flatten() {
                visit(row.clone())?;
            }
            Ok(())
        }

        fn visit_relation_column(
            &self,
            relation: SemanticId,
            ordinal: usize,
            visit: &mut dyn FnMut(Value),
        ) -> Result<(), RealizationError> {
            for row in self.rows.get(&relation).into_iter().flatten() {
                visit(
                    row.get(ordinal)
                        .cloned()
                        .ok_or(RealizationError::OneShotRelationArityMismatch)?,
                );
            }
            Ok(())
        }

        fn relation_arity(&self, relation: SemanticId) -> Result<usize, RealizationError> {
            Ok(self
                .rows
                .get(&relation)
                .and_then(|rows| rows.first())
                .map_or(1, Vec::len))
        }

        fn relation_dependencies(
            &self,
            _relation: SemanticId,
        ) -> Result<BTreeSet<PhysicalAtomId>, RealizationError> {
            Ok(BTreeSet::new())
        }
    }

    fn context(
        set: bool,
        arity: usize,
    ) -> (
        SemanticContext,
        SemanticRegistry,
        SemanticId,
        SemanticId,
        SemanticId,
    ) {
        let left = SemanticId::new(91_000);
        let right = SemanticId::new(91_001);
        let eq = SemanticId::new(91_002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let semantics = if set {
            RelationSemantics::Set {
                column_equivalences: vec![eq; arity],
            }
        } else {
            RelationSemantics::Bag {
                column_equivalences: vec![eq; arity],
            }
        };
        let mut schema = Schema::new(SchemaRevisionId::new(910));
        for relation in [left, right] {
            schema
                .define_relation_with_column_ids(
                    RelationDef {
                        id: relation,
                        columns: vec![TypeExpr::Scalar(ScalarType::I64); arity],
                        semantics: semantics.clone(),
                    },
                    (0..arity)
                        .map(|i| SemanticId::new(91_100_u128 + i as u128))
                        .collect(),
                )
                .unwrap();
        }
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(910));
        environment.pin_module(eq, digest);
        (
            SemanticContext {
                schema,
                environment,
            },
            registry,
            left,
            right,
            eq,
        )
    }

    fn run(
        source: &MockSource,
        expr: &RelExpr,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Vec<Vec<Value>>, RealizationError> {
        let mut rows = Vec::new();
        execute_expr(source, expr, context, registry, &mut |row| {
            rows.push(row);
            Ok(())
        })?;
        Ok(rows)
    }

    #[test]
    fn factorized_public_evaluator_runs_rel_expr_without_database_state_materialization() {
        let (context, registry, left, right, _eq) = context(false, 1);
        let mut state = kernel_model::DatabaseState::default();
        state.model.relations.insert(
            left,
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)],
            ],
        );
        state
            .model
            .relations
            .insert(right, vec![vec![Value::I64(2)]]);
        let (atoms, root) = crate::realize_database_state_factorized(&state, &context).unwrap();
        let expr = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };

        let rows =
            evaluate_relation_expr_factorized(&root, &atoms, &context, &registry, &expr).unwrap();
        assert_eq!(rows, vec![vec![Value::I64(1)], vec![Value::I64(3)]]);
    }

    #[test]
    fn set_union_project_and_distinct_quotient_in_gamma_space() {
        let (context, registry, left, right, eq) = context(true, 1);
        let source = MockSource {
            rows: BTreeMap::from([
                (left, vec![vec![Value::I64(1)], vec![Value::I64(2)]]),
                (right, vec![vec![Value::I64(2)], vec![Value::I64(3)]]),
            ]),
        };
        let expr = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Union {
                    left: Box::new(RelExpr::Scan(left)),
                    right: Box::new(RelExpr::Scan(right)),
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![eq],
        };
        assert_eq!(
            run(&source, &expr, &context, &registry).unwrap(),
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)]
            ]
        );
    }

    #[test]
    fn nested_set_filter_union_project_distinct_join_composes_without_fallback() {
        let (context, registry, left, right, eq) = context(true, 1);
        let source = MockSource {
            rows: BTreeMap::from([
                (
                    left,
                    vec![
                        vec![Value::I64(1)],
                        vec![Value::I64(2)],
                        vec![Value::I64(4)],
                    ],
                ),
                (
                    right,
                    vec![
                        vec![Value::I64(2)],
                        vec![Value::I64(3)],
                        vec![Value::I64(4)],
                    ],
                ),
            ]),
        };
        let left_set = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Union {
                    left: Box::new(RelExpr::FilterEqConst {
                        input: Box::new(RelExpr::Scan(left)),
                        column: 0,
                        value: Value::I64(2),
                        equivalence: eq,
                    }),
                    right: Box::new(RelExpr::Scan(right)),
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![eq],
        };
        let expr = RelExpr::JoinEq {
            left: Box::new(left_set),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        assert_eq!(
            run(&source, &expr, &context, &registry).unwrap(),
            vec![
                vec![Value::I64(2), Value::I64(2)],
                vec![Value::I64(3), Value::I64(3)],
                vec![Value::I64(4), Value::I64(4)],
            ]
        );
    }

    #[test]
    fn group_count_owns_only_group_and_exact_aggregate_state() {
        let (context, registry, left, _right, eq) = context(false, 1);
        let source = MockSource {
            rows: BTreeMap::from([(
                left,
                vec![
                    vec![Value::I64(2)],
                    vec![Value::I64(1)],
                    vec![Value::I64(2)],
                ],
            )]),
        };
        let expr = RelExpr::Group {
            input: Box::new(RelExpr::Scan(left)),
            group_columns: vec![0],
            group_equivalences: vec![eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: eq,
            },
        };
        assert_eq!(
            run(&source, &expr, &context, &registry).unwrap(),
            vec![
                vec![Value::I64(2), Value::I64(2)],
                vec![Value::I64(1), Value::I64(1)],
            ]
        );
    }

    #[test]
    fn certified_gamma_survives_filter_and_antijoin_without_recanonicalization() {
        let (context, registry, left, right, eq) = context(true, 1);
        let source = MockSource {
            rows: BTreeMap::from([
                (
                    left,
                    vec![
                        vec![Value::I64(1)],
                        vec![Value::I64(2)],
                        vec![Value::I64(3)],
                    ],
                ),
                (right, vec![vec![Value::I64(3)]]),
            ]),
        };
        let expr = RelExpr::AntiJoin {
            left: Box::new(RelExpr::FilterOrderConst {
                input: Box::new(RelExpr::Distinct {
                    input: Box::new(RelExpr::Scan(left)),
                    column_equivalences: vec![eq],
                }),
                column: 0,
                value: Value::I64(1),
                ordering: {
                    // Reuse a separately pinned ordering authority below.
                    SemanticId::new(91_003)
                },
                comparison: kernel_query::OrderComparison::GreaterOrEqual,
            }),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };

        let mut context = context;
        let mut registry = registry;
        let ordering = SemanticId::new(91_003);
        let digest = registry.install_ordering(OrderingModule::I64Ascending);
        context.environment.pin_module(ordering, digest);
        let mut rows = Vec::new();
        let mut evidence = Vec::new();
        assert!(
            execute_certified_output(
                &source,
                &expr,
                &context,
                &registry,
                &mut |row, certified| {
                    rows.push(row);
                    evidence.push(certified);
                    Ok(())
                },
            )
            .unwrap()
        );
        assert_eq!(rows, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
        assert_eq!(evidence.len(), 2);
        RelationOccurrenceCertificate::from_dense_certified_keys(evidence).unwrap();
    }

    #[test]
    fn top_k_with_ties_keeps_only_boundary_candidates_and_orders_output() {
        let (mut context, mut registry, left, _right, _eq) = context(false, 1);
        let ordering = SemanticId::new(91_003);
        let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
        context.environment.pin_module(ordering, ordering_digest);
        let source = MockSource {
            rows: BTreeMap::from([(
                left,
                vec![
                    vec![Value::I64(4)],
                    vec![Value::I64(2)],
                    vec![Value::I64(1)],
                    vec![Value::I64(2)],
                    vec![Value::I64(9)],
                ],
            )]),
        };
        let expr = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(left)),
            column: 0,
            ordering,
            direction: kernel_query::OrderDirection::Ascending,
            k: 2,
        };
        assert_eq!(
            run(&source, &expr, &context, &registry).unwrap(),
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(2)],
            ]
        );
    }

    #[test]
    #[ignore = "manual release-mode one-shot Join hostile benchmark"]
    fn one_shot_join_scale_benchmark() {
        use std::time::Instant;
        let (context, registry, left, right, eq) = context(false, 1);
        let rows = 100_000usize;
        let left_rows = (0..rows)
            .map(|i| vec![Value::I64(i as i64)])
            .collect::<Vec<_>>();
        let right_rows = (0..rows)
            .map(|i| vec![Value::I64(i as i64)])
            .collect::<Vec<_>>();
        let source = MockSource {
            rows: BTreeMap::from([(left, left_rows.clone()), (right, right_rows.clone())]),
        };
        let mut model = kernel_model::FiniteModel::default();
        model.relations.insert(left, left_rows);
        model.relations.insert(right, right_rows);
        let expr = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        let expected = expr
            .evaluate(&model, &context, &registry)
            .unwrap()
            .into_rows();
        let mut generic_ms = Vec::new();
        let mut one_shot_ms = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let generic = expr
                .evaluate(&model, &context, &registry)
                .unwrap()
                .into_rows();
            generic_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(generic, expected);
            let started = Instant::now();
            let streamed = run(&source, &expr, &context, &registry).unwrap();
            one_shot_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(streamed, expected);
        }
        eprintln!("one-shot Join 100k generic_ms={generic_ms:?} one_shot_ms={one_shot_ms:?}");
    }

    #[test]
    fn join_owns_only_one_side_fiber_index_and_streams_left() {
        let (context, registry, left, right, eq) = context(false, 1);
        let source = MockSource {
            rows: BTreeMap::from([
                (
                    left,
                    vec![
                        vec![Value::I64(1)],
                        vec![Value::I64(2)],
                        vec![Value::I64(1)],
                    ],
                ),
                (right, vec![vec![Value::I64(1)], vec![Value::I64(3)]]),
            ]),
        };
        let expr = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        assert_eq!(
            run(&source, &expr, &context, &registry).unwrap(),
            vec![
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(1), Value::I64(1)],
            ]
        );
    }

    #[test]
    fn certified_gamma_rows_adopt_occurrence_root_without_recanonicalization() {
        let (context, registry, left, _right, _eq) = context(true, 1);
        let result_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let canonicalizer =
            RelationRowCanonicalizer::compile(result_type.clone(), &context, &registry).unwrap();
        let certified = [1_i64, 2, 3]
            .into_iter()
            .map(|value| canonicalizer.certify_row(&vec![Value::I64(value)]).unwrap())
            .collect::<Vec<_>>();
        let certificate =
            RelationOccurrenceCertificate::from_dense_certified_keys(certified).unwrap();
        let probe = certificate.clone();
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            left,
            certificate,
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        assert!(probe.shares_occurrence_root_with(&witness));

        let other =
            RelationRowCanonicalizer::compile(witness.result_type().clone(), &context, &registry)
                .unwrap();
        let mixed = vec![
            canonicalizer.certify_row(&vec![Value::I64(4)]).unwrap(),
            other.certify_row(&vec![Value::I64(5)]).unwrap(),
        ];
        assert!(matches!(
            RelationOccurrenceCertificate::from_dense_certified_keys(mixed),
            Err(RelQueryError::StructuralRewriteBaseMismatch)
        ));
    }
}
