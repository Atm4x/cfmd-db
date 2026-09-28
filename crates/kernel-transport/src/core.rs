use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::IdentityTransport;
use kernel_model::{DatabaseState, FiniteModel};
use kernel_query::{AggregateSpec, RelExpr};
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;

use crate::TransportError;

pub fn transport_value(
    identity: &IdentityTransport,
    value: &kernel_model::Value,
) -> Result<kernel_model::Value, TransportError> {
    use kernel_model::Value;
    match value {
        Value::LiveEntityRef { entity_type, id } => Ok(Value::LiveEntityRef {
            entity_type: *entity_type,
            id: identity
                .transport(*id)
                .ok_or(TransportError::IdentitySourceCoverageMismatch)?,
        }),
        Value::HistoricalEntityId { entity_type, id } => Ok(Value::HistoricalEntityId {
            entity_type: *entity_type,
            id: identity
                .transport(*id)
                .ok_or(TransportError::IdentitySourceCoverageMismatch)?,
        }),
        Value::Product(fields) => Ok(Value::Product(
            fields
                .iter()
                .map(|(&field, value)| transport_value(identity, value).map(|value| (field, value)))
                .collect::<Result<_, _>>()?,
        )),
        Value::Option(value) => Ok(Value::Option(
            value
                .as_deref()
                .map(|value| transport_value(identity, value).map(Box::new))
                .transpose()?,
        )),
        Value::Variant { tag, value } => Ok(Value::Variant {
            tag: *tag,
            value: Box::new(transport_value(identity, value)?),
        }),
        Value::Seq(values) => Ok(Value::Seq(
            values
                .iter()
                .map(|value| transport_value(identity, value))
                .collect::<Result<_, _>>()?,
        )),
        Value::Set {
            equivalence,
            elements,
        } => Ok(Value::Set {
            equivalence: *equivalence,
            elements: elements
                .iter()
                .map(|value| transport_value(identity, value))
                .collect::<Result<_, _>>()?,
        }),
        Value::Bag {
            equivalence,
            entries,
        } => Ok(Value::Bag {
            equivalence: *equivalence,
            entries: entries
                .iter()
                .map(|(value, count)| transport_value(identity, value).map(|value| (value, *count)))
                .collect::<Result<_, _>>()?,
        }),
        Value::Map {
            key_equivalence,
            entries,
        } => Ok(Value::Map {
            key_equivalence: *key_equivalence,
            entries: entries
                .iter()
                .map(|(key, value)| {
                    Ok((
                        transport_value(identity, key)?,
                        transport_value(identity, value)?,
                    ))
                })
                .collect::<Result<_, TransportError>>()?,
        }),
        Value::Unit => Ok(Value::Unit),
        Value::Bool(value) => Ok(Value::Bool(*value)),
        Value::I64(value) => Ok(Value::I64(*value)),
        Value::F64Bits(value) => Ok(Value::F64Bits(*value)),
        Value::Text(value) => Ok(Value::Text(value.clone())),
    }
}

pub fn transport_value_change(
    identity: &IdentityTransport,
    change: &kernel_change::Change<kernel_model::Value>,
) -> Result<kernel_change::Change<kernel_model::Value>, TransportError> {
    Ok(match change {
        kernel_change::Change::NoChange => kernel_change::Change::NoChange,
        kernel_change::Change::Replace(value) => {
            kernel_change::Change::Replace(transport_value(identity, value)?)
        }
        kernel_change::Change::Fine(fine) => {
            kernel_change::Change::Fine(kernel_change::FineChange::new(
                fine.kind(),
                transport_value(identity, fine.endpoint())?,
            ))
        }
    })
}

fn transport_expr(
    identity: &IdentityTransport,
    expr: &kernel_query::Expr,
) -> Result<kernel_query::Expr, TransportError> {
    use kernel_query::Expr;
    Ok(match expr {
        Expr::Input => Expr::Input,
        Expr::Const(value) => Expr::Const(transport_value(identity, value)?),
        Expr::TypedConst { value, ty } => Expr::TypedConst {
            value: transport_value(identity, value)?,
            ty: ty.clone(),
        },
        Expr::ProductField { input, field } => Expr::ProductField {
            input: Box::new(transport_expr(identity, input)?),
            field: *field,
        },
        Expr::SeqLength(input) => Expr::SeqLength(Box::new(transport_expr(identity, input)?)),
        Expr::SeqSumI64(input) => Expr::SeqSumI64(Box::new(transport_expr(identity, input)?)),
        Expr::AddI64(left, right) => Expr::AddI64(
            Box::new(transport_expr(identity, left)?),
            Box::new(transport_expr(identity, right)?),
        ),
        Expr::If {
            condition,
            when_true,
            when_false,
        } => Expr::If {
            condition: Box::new(transport_expr(identity, condition)?),
            when_true: Box::new(transport_expr(identity, when_true)?),
            when_false: Box::new(transport_expr(identity, when_false)?),
        },
    })
}

pub fn transport_exact_query(
    identity: &IdentityTransport,
    query: &kernel_query::ExactQuery,
) -> Result<kernel_query::ExactQuery, TransportError> {
    Ok(kernel_query::ExactQuery::new(transport_expr(
        identity,
        query.root(),
    )?))
}

pub fn check_impact_transport_law(
    identity: &IdentityTransport,
    query: &kernel_query::ExactQuery,
    old: &kernel_model::Value,
    change: &kernel_change::Change<kernel_model::Value>,
) -> Result<bool, TransportError> {
    let transported_query = transport_exact_query(identity, query)?;
    let transported_old = transport_value(identity, old)?;
    let transported_change = transport_value_change(identity, change)?;
    Ok(kernel_query::impact_by_recompute(query, old, change)
        == kernel_query::impact_by_recompute(
            &transported_query,
            &transported_old,
            &transported_change,
        ))
}

pub fn transport_finite_model(
    identity: &IdentityTransport,
    source: &FiniteModel,
) -> Result<FiniteModel, TransportError> {
    let map_id = |id| {
        identity
            .transport(id)
            .ok_or(TransportError::IdentitySourceCoverageMismatch)
    };
    let mut model = FiniteModel::default();
    for (&carrier, entities) in &source.carriers {
        model.carriers.insert(
            carrier,
            entities
                .iter()
                .copied()
                .map(map_id)
                .collect::<Result<BTreeSet<_>, _>>()?,
        );
    }
    for (&(field, owner), value) in &source.fields {
        model
            .fields
            .insert((field, map_id(owner)?), transport_value(identity, value)?);
    }
    for (&relation, rows) in &source.relations {
        model.relations.insert(
            relation,
            rows.iter()
                .map(|row| {
                    row.iter()
                        .map(|value| transport_value(identity, value))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(model)
}

pub fn transport_model_change(
    identity: &IdentityTransport,
    change: &kernel_change::Change<FiniteModel>,
) -> Result<kernel_change::Change<FiniteModel>, TransportError> {
    Ok(match change {
        kernel_change::Change::NoChange => kernel_change::Change::NoChange,
        kernel_change::Change::Replace(model) => {
            kernel_change::Change::Replace(transport_finite_model(identity, model)?)
        }
        kernel_change::Change::Fine(fine) => {
            kernel_change::Change::Fine(kernel_change::FineChange::new(
                fine.kind(),
                transport_finite_model(identity, fine.endpoint())?,
            ))
        }
    })
}

pub fn transport_rel_expr(
    identity: &IdentityTransport,
    expr: &RelExpr,
) -> Result<RelExpr, TransportError> {
    Ok(match expr {
        RelExpr::Scan(relation) => RelExpr::Scan(*relation),
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => RelExpr::FilterEqConst {
            input: Box::new(transport_rel_expr(identity, input)?),
            column: *column,
            value: transport_value(identity, value)?,
            equivalence: *equivalence,
        },
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => RelExpr::FilterEqColumns {
            input: Box::new(transport_rel_expr(identity, input)?),
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
        },
        RelExpr::Project { input, columns } => RelExpr::Project {
            input: Box::new(transport_rel_expr(identity, input)?),
            columns: columns.clone(),
        },
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => RelExpr::JoinEq {
            left: Box::new(transport_rel_expr(identity, left)?),
            right: Box::new(transport_rel_expr(identity, right)?),
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
        },
        RelExpr::Difference { left, right } => RelExpr::Difference {
            left: Box::new(transport_rel_expr(identity, left)?),
            right: Box::new(transport_rel_expr(identity, right)?),
        },
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => RelExpr::AntiJoin {
            left: Box::new(transport_rel_expr(identity, left)?),
            right: Box::new(transport_rel_expr(identity, right)?),
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
        },
        RelExpr::Distinct {
            input,
            column_equivalences,
        } => RelExpr::Distinct {
            input: Box::new(transport_rel_expr(identity, input)?),
            column_equivalences: column_equivalences.clone(),
        },
        RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => transport_group_expr(
            identity,
            input,
            group_columns,
            group_equivalences,
            aggregate,
        )?,
        RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => RelExpr::TopKWithTies {
            input: Box::new(transport_rel_expr(identity, input)?),
            column: *column,
            ordering: *ordering,
            direction: *direction,
            k: *k,
        },
        RelExpr::PromoteToBag(input) => {
            RelExpr::PromoteToBag(Box::new(transport_rel_expr(identity, input)?))
        }
    })
}

fn transport_group_expr(
    identity: &IdentityTransport,
    input: &RelExpr,
    group_columns: &[usize],
    group_equivalences: &[kernel_types::SemanticId],
    aggregate: &AggregateSpec,
) -> Result<RelExpr, TransportError> {
    let aggregate = match aggregate {
        AggregateSpec::Count { result_equivalence } => AggregateSpec::Count {
            result_equivalence: *result_equivalence,
        },
        AggregateSpec::ExactF64Sum {
            value_column,
            result_equivalence,
        } => AggregateSpec::ExactF64Sum {
            value_column: *value_column,
            result_equivalence: *result_equivalence,
        },
    };
    Ok(RelExpr::Group {
        input: Box::new(transport_rel_expr(identity, input)?),
        group_columns: group_columns.to_vec(),
        group_equivalences: group_equivalences.to_vec(),
        aggregate,
    })
}

pub fn check_rel_impact_transport_law(
    identity: &IdentityTransport,
    query: &RelExpr,
    old: &FiniteModel,
    change: &kernel_change::Change<FiniteModel>,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<bool, TransportError> {
    let transported_query = transport_rel_expr(identity, query)?;
    let transported_old = transport_finite_model(identity, old)?;
    let transported_change = transport_model_change(identity, change)?;
    Ok(
        kernel_query::rel_impact_by_recompute(query, old, change, context, registry)
            == kernel_query::rel_impact_by_recompute(
                &transported_query,
                &transported_old,
                &transported_change,
                context,
                registry,
            ),
    )
}

pub(crate) fn check_rel_impact_identity_context_law(
    query: &RelExpr,
    old: &FiniteModel,
    change: &kernel_change::Change<FiniteModel>,
    source: &SemanticContext,
    target: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<bool, TransportError> {
    query
        .prepare(source, registry)
        .map_err(TransportError::RelationTransformType)?;
    query
        .prepare(target, registry)
        .map_err(TransportError::RelationTransformType)?;
    Ok(
        kernel_query::rel_impact_by_recompute(query, old, change, source, registry)
            == kernel_query::rel_impact_by_recompute(query, old, change, target, registry),
    )
}

pub fn transport_database_state(
    identity: &IdentityTransport,
    source: &DatabaseState,
) -> Result<DatabaseState, TransportError> {
    if !source.lifecycle.entities.is_subset(&identity.source_ids()) {
        return Err(TransportError::IdentitySourceCoverageMismatch);
    }
    let lifecycle = transport_lifecycle_graph(identity, &source.lifecycle)?;
    let model = transport_finite_model(identity, &source.model)?;
    Ok(DatabaseState {
        model,
        lifecycle: lifecycle.into(),
    })
}

pub fn transport_lifecycle_intent(
    identity: &IdentityTransport,
    intent: &kernel_lifecycle::LifecycleIntent,
) -> Result<kernel_lifecycle::LifecycleIntent, TransportError> {
    use kernel_lifecycle::{LifecycleFact, LifecycleIntent};

    let map_id = |id| {
        identity
            .transport(id)
            .ok_or(TransportError::IdentitySourceCoverageMismatch)
    };
    let mut transported = LifecycleIntent::default();
    for (fact, &present) in intent.edits() {
        let fact = match fact {
            LifecycleFact::Root(entity) => LifecycleFact::Root(map_id(*entity)?),
            LifecycleFact::KeepsAlive { parent, child } => LifecycleFact::KeepsAlive {
                parent: map_id(*parent)?,
                child: map_id(*child)?,
            },
        };
        transported.set(fact, present);
    }
    Ok(transported)
}

pub fn transport_lifecycle_graph(
    identity: &IdentityTransport,
    graph: &kernel_lifecycle::LifecycleGraph,
) -> Result<kernel_lifecycle::LifecycleGraph, TransportError> {
    let source_ids = identity.source_ids();
    if !graph.entities.is_subset(&source_ids) {
        return Err(TransportError::IdentitySourceCoverageMismatch);
    }
    let map_id = |id| {
        identity
            .transport(id)
            .ok_or(TransportError::IdentitySourceCoverageMismatch)
    };
    let mut transported = kernel_lifecycle::LifecycleGraph {
        entities: graph
            .entities
            .iter()
            .copied()
            .map(map_id)
            .collect::<Result<_, _>>()?,
        roots: graph
            .roots
            .iter()
            .copied()
            .map(map_id)
            .collect::<Result<_, _>>()?,
        keeps_alive: BTreeMap::new(),
    };
    for (&parent, children) in &graph.keeps_alive {
        let parent = map_id(parent)?;
        let children = children
            .iter()
            .copied()
            .map(map_id)
            .collect::<Result<BTreeSet<_>, _>>()?;
        transported.keeps_alive.insert(parent, children);
    }
    Ok(transported)
}
