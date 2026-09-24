use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::IdentityTransport;
use kernel_model::{DatabaseState, FiniteModel};
use kernel_query::{AggregateSpec, ExactQuery, QueryError, QueryTypeError, RelExpr, RelQueryError};
use kernel_schema::SemanticContext;
use kernel_semantics::{SemanticError, SemanticRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    SourceSemantics(SemanticError),
    TargetSemantics(SemanticError),
    NotDefinitionallyEquivalent,
    UnsupportedStructuralChange,
    UnknownSourceField(kernel_types::SemanticId),
    UnknownTargetField(kernel_types::SemanticId),
    DuplicateTargetField(kernel_types::SemanticId),
    OwnerTypeMismatch(kernel_types::SemanticId),
    TransformType(QueryTypeError),
    TransformExecution(QueryError),
    InvalidTarget(kernel_validation::ValidationError),
    SourceRevisionMismatch,
    InvalidRevision(kernel_revision::RevisionError),
    SemanticEnvironmentChangeRequiresTransport,
    SemanticContractChanged(kernel_types::SemanticId),
    NoSemanticLawChange,
    UnknownTargetRelation(kernel_types::SemanticId),
    DuplicateTargetRelation(kernel_types::SemanticId),
    RelationTransformType(RelQueryError),
    RelationTransformExecution(RelQueryError),
    NotConservativeSemanticExtension,
    IdentitySourceCoverageMismatch,
}

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

fn check_rel_impact_identity_context_law(
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BijectiveIdentityRevisionTransport {
    source: SemanticContext,
    target: SemanticContext,
    identity: IdentityTransport,
}

impl BijectiveIdentityRevisionTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        identity: IdentityTransport,
    ) -> Result<Self, TransportError> {
        if !registry
            .contexts_semantically_equivalent(source, target)
            .map_err(TransportError::TargetSemantics)?
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            identity,
        })
    }

    fn transport_state(&self, source: &DatabaseState) -> Result<DatabaseState, TransportError> {
        transport_database_state(&self.identity, source)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state())?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn identity(&self) -> &IdentityTransport {
        &self.identity
    }

    pub fn transport_lifecycle_intent(
        &self,
        intent: &kernel_lifecycle::LifecycleIntent,
    ) -> Result<kernel_lifecycle::LifecycleIntent, TransportError> {
        transport_lifecycle_intent(&self.identity, intent)
    }

    pub fn transport_lifecycle_graph(
        &self,
        graph: &kernel_lifecycle::LifecycleGraph,
    ) -> Result<kernel_lifecycle::LifecycleGraph, TransportError> {
        transport_lifecycle_graph(&self.identity, graph)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConservativeSemanticEnvironmentExtension {
    source: SemanticContext,
    target: SemanticContext,
}

impl ConservativeSemanticEnvironmentExtension {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        if !registry
            .context_conservatively_extends(source, target)
            .map_err(TransportError::TargetSemantics)?
        {
            return Err(TransportError::NotConservativeSemanticExtension);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivalentSemanticEnvironmentTransport {
    source: SemanticContext,
    target: SemanticContext,
}

impl EquivalentSemanticEnvironmentTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }
        let source_modules: BTreeMap<_, _> = source.environment.modules().collect();
        let target_modules: BTreeMap<_, _> = target.environment.modules().collect();
        if source_modules.keys().collect::<BTreeSet<_>>()
            != target_modules.keys().collect::<BTreeSet<_>>()
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        for (dependency, source_digest) in source_modules {
            let target_digest = target_modules[&dependency];
            if !registry
                .equivalent_implementation_contract(source_digest, target_digest)
                .map_err(TransportError::TargetSemantics)?
            {
                return Err(TransportError::SemanticContractChanged(dependency));
            }
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticLawMigration {
    source: SemanticContext,
    target: SemanticContext,
}

impl SemanticLawMigration {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }
        let mut changed = false;
        let source_modules: BTreeMap<_, _> = source.environment.modules().collect();
        let target_modules: BTreeMap<_, _> = target.environment.modules().collect();
        if source_modules.keys().collect::<BTreeSet<_>>()
            != target_modules.keys().collect::<BTreeSet<_>>()
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        for (dependency, source_digest) in source_modules {
            let target_digest = target_modules[&dependency];
            changed |= !registry
                .equivalent_implementation_contract(source_digest, target_digest)
                .map_err(TransportError::TargetSemantics)?;
        }
        if !changed {
            return Err(TransportError::NoSemanticLawChange);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionalTransport {
    source: SemanticContext,
    target: SemanticContext,
}

impl DefinitionalTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.definitionally_equivalent(target) {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state());
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn transport_state(&self, state: &DatabaseState) -> DatabaseState {
        state.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRewrite {
    pub source_field: kernel_types::SemanticId,
    pub target_field: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRewrite {
    pub target_relation: kernel_types::SemanticId,
    pub transform: RelExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedRelationTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites: Vec<RelationRewrite>,
    passthrough: BTreeSet<kernel_types::SemanticId>,
}

impl TypedRelationTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        rewrites: Vec<RelationRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source
            .environment
            .definitionally_equivalent(&target.environment)
        {
            return Err(TransportError::SemanticEnvironmentChangeRequiresTransport);
        }
        if !source
            .schema
            .relation_transport_base_equivalent(&target.schema)
        {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut by_target = BTreeMap::new();
        for rewrite in rewrites {
            let target_relation = rewrite.target_relation;
            if by_target.insert(target_relation, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetRelation(target_relation));
            }
        }

        let mut passthrough = BTreeSet::new();
        for target_relation in target.schema.relations() {
            if source.schema.relation(target_relation.id) == Some(target_relation) {
                passthrough.insert(target_relation.id);
                continue;
            }
            let rewrite = by_target
                .get(&target_relation.id)
                .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
            let prepared = rewrite
                .transform
                .prepare(source, registry)
                .map_err(TransportError::RelationTransformType)?;
            if prepared.result_type().columns != target_relation.columns
                || prepared.result_type().semantics != target_relation.semantics
            {
                return Err(TransportError::RelationTransformType(
                    RelQueryError::TypeMismatch,
                ));
            }
        }

        let known_targets: BTreeSet<_> = target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect();
        if let Some(unknown) = by_target
            .keys()
            .find(|relation| !known_targets.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            rewrites: by_target.into_values().collect(),
            passthrough,
        })
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut model = FiniteModel {
            carriers: source_state.model.carriers.clone(),
            fields: source_state.model.fields.clone(),
            relations: kernel_model::RelationStore::default(),
        };
        for relation in &self.passthrough {
            if let Some(rows) = source_state.model.relations.get(relation) {
                model.relations.insert(*relation, rows.clone());
            }
        }
        for rewrite in &self.rewrites {
            let prepared = rewrite
                .transform
                .prepare(&self.source, registry)
                .map_err(TransportError::RelationTransformType)?;
            let result = prepared
                .evaluate(&source_state.model, &self.source, registry)
                .map_err(TransportError::RelationTransformExecution)?;
            model
                .relations
                .insert(rewrite.target_relation, result.rows().to_vec());
        }
        let state = DatabaseState {
            model,
            lifecycle: source_state.lifecycle.clone(),
        };
        kernel_validation::validate_state(&self.target, registry, &state)
            .map_err(TransportError::InvalidTarget)?;
        Ok(state)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state(), registry)?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedFieldTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites: Vec<FieldRewrite>,
    passthrough: BTreeSet<kernel_types::SemanticId>,
}

impl TypedFieldTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        rewrites: Vec<FieldRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source
            .environment
            .definitionally_equivalent(&target.environment)
        {
            return Err(TransportError::SemanticEnvironmentChangeRequiresTransport);
        }
        if !source
            .schema
            .field_transport_base_equivalent(&target.schema)
        {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut by_target = BTreeMap::new();
        for rewrite in rewrites {
            let target_field = rewrite.target_field;
            if by_target.insert(target_field, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetField(target_field));
            }
        }

        let mut passthrough = BTreeSet::new();
        for target_field in target.schema.fields() {
            if let Some(source_field) = source.schema.field(target_field.id)
                && source_field == target_field
            {
                passthrough.insert(target_field.id);
                continue;
            }
            let rewrite = by_target
                .get(&target_field.id)
                .ok_or(TransportError::UnknownTargetField(target_field.id))?;
            let source_field = source
                .schema
                .field(rewrite.source_field)
                .ok_or(TransportError::UnknownSourceField(rewrite.source_field))?;
            if source_field.owner != target_field.owner {
                return Err(TransportError::OwnerTypeMismatch(target_field.id));
            }
            let result = rewrite
                .transform
                .typecheck(&source_field.value)
                .map_err(TransportError::TransformType)?;
            if result != target_field.value {
                return Err(TransportError::TransformType(QueryTypeError::TypeMismatch));
            }
        }

        let known_targets: BTreeSet<_> = target.schema.fields().map(|field| field.id).collect();
        if let Some(unknown) = by_target
            .keys()
            .find(|field| !known_targets.contains(field))
        {
            return Err(TransportError::UnknownTargetField(*unknown));
        }

        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            rewrites: by_target.into_values().collect(),
            passthrough,
        })
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut model = FiniteModel {
            carriers: source_state.model.carriers.clone(),
            fields: BTreeMap::new().into(),
            relations: source_state.model.relations.clone(),
        };

        for (&(field, entity), value) in &source_state.model.fields {
            if self.passthrough.contains(&field) {
                model.fields.insert((field, entity), value.clone());
            }
        }
        for rewrite in &self.rewrites {
            for (&(field, entity), value) in &source_state.model.fields {
                if field != rewrite.source_field {
                    continue;
                }
                let transformed = rewrite
                    .transform
                    .evaluate(value)
                    .map_err(TransportError::TransformExecution)?;
                model
                    .fields
                    .insert((rewrite.target_field, entity), transformed);
            }
        }
        let state = DatabaseState {
            model,
            lifecycle: source_state.lifecycle.clone(),
        };
        kernel_validation::validate_state(&self.target, registry, &state)
            .map_err(TransportError::InvalidTarget)?;
        Ok(state)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state(), registry)?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[cfg(test)]
mod tests {
    use kernel_schema::{
        FieldDef, ScalarType, Schema, SemanticEnvironment, Symbol, SymbolKind, TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry, TokenizerModule};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn set_relation_context(
        env_revision: u64,
        equality_digest: kernel_schema::ModuleDigest,
    ) -> SemanticContext {
        let relation = SemanticId::new(5000);
        let equality = SemanticId::new(5001);
        let mut schema = Schema::new(SchemaRevisionId::new(5000));
        schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: kernel_schema::RelationSemantics::Set {
                    column_equivalences: vec![equality],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
        environment.pin_module(equality, equality_digest);
        SemanticContext {
            schema,
            environment,
        }
    }

    fn context(
        schema_revision: u64,
        env_revision: u64,
        name: &str,
    ) -> (SemanticContext, SemanticRegistry) {
        let entity = SemanticId::new(1);
        let field = SemanticId::new(2);
        let equality = SemanticId::new(3);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut schema = Schema::new(SchemaRevisionId::new(schema_revision));
        schema
            .define(Symbol {
                id: field,
                kind: SymbolKind::Field,
                presentation_name: name.into(),
            })
            .unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: entity,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
        environment.pin_module(equality, digest);
        (
            SemanticContext {
                schema,
                environment,
            },
            registry,
        )
    }

    #[test]
    fn rename_and_revision_bump_transport_without_data_migration() {
        let (source, registry) = context(1, 1, "name");
        let (target, _) = context(2, 2, "display_name");
        let transport = DefinitionalTransport::verify(&source, &target, &registry).unwrap();
        assert_eq!(
            transport.transport_state(&DatabaseState::default()),
            DatabaseState::default()
        );
    }

    #[test]
    fn semantic_type_change_is_not_definitional_transport() {
        let (source, registry) = context(1, 1, "name");
        let (mut target, _) = context(2, 2, "name");
        target
            .schema
            .define_field(FieldDef {
                id: SemanticId::new(9),
                owner: SemanticId::new(1),
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        assert_eq!(
            DefinitionalTransport::verify(&source, &target, &registry),
            Err(TransportError::NotDefinitionallyEquivalent)
        );
    }
    #[test]
    fn typed_field_transport_reuses_exact_query_ir_and_rebuilds_trusted_revision() {
        let entity_type = SemanticId::new(20);
        let source_field = SemanticId::new(21);
        let target_field = SemanticId::new(22);
        let entity = kernel_types::EntityId::new(1);
        let registry = SemanticRegistry::default();

        let mut source_schema = Schema::new(SchemaRevisionId::new(10));
        source_schema
            .define_field(FieldDef {
                id: source_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let source_context = SemanticContext {
            schema: source_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(10)),
        };

        let mut target_schema = Schema::new(SchemaRevisionId::new(11));
        target_schema
            .define_field(FieldDef {
                id: target_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let target_context = SemanticContext {
            schema: target_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(11)),
        };

        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([entity]));
        state
            .model
            .fields
            .insert((source_field, entity), kernel_model::Value::I64(41));
        let source_revision = kernel_revision::Revision::build(
            kernel_types::RevisionId::new(1),
            &source_context,
            &registry,
            state,
        )
        .unwrap();
        let transform = ExactQuery::new(kernel_query::Expr::AddI64(
            Box::new(kernel_query::Expr::Input),
            Box::new(kernel_query::Expr::Const(kernel_model::Value::I64(1))),
        ));
        let transport = TypedFieldTransport::verify(
            &source_context,
            &target_context,
            &registry,
            vec![FieldRewrite {
                source_field,
                target_field,
                transform,
            }],
        )
        .unwrap();
        let target_revision = transport
            .transport_revision(
                &source_revision,
                kernel_types::RevisionId::new(2),
                &registry,
            )
            .unwrap();
        assert_eq!(
            target_revision
                .state()
                .model
                .fields
                .get(&(target_field, entity)),
            Some(&kernel_model::Value::I64(42))
        );
        assert!(
            !target_revision
                .state()
                .model
                .fields
                .contains_key(&(source_field, entity))
        );
    }

    #[test]
    fn implementation_upgrade_with_same_contract_is_identity_transport() {
        let mut registry = SemanticRegistry::default();
        let old = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let new = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let source = set_relation_context(1, old);
        let target = set_relation_context(2, new);

        let transport =
            EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry).unwrap();
        let source_revision = kernel_revision::Revision::build(
            kernel_types::RevisionId::new(1),
            &source,
            &registry,
            DatabaseState::default(),
        )
        .unwrap();
        let target_revision = transport
            .transport_revision(
                &source_revision,
                kernel_types::RevisionId::new(2),
                &registry,
            )
            .unwrap();

        assert_eq!(source_revision.state(), target_revision.state());
        assert_ne!(source.environment, target.environment);
    }

    #[test]
    fn changed_law_is_not_misclassified_as_implementation_upgrade() {
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let ci =
            registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 1);
        let source = set_relation_context(1, exact);
        let target = set_relation_context(2, ci);

        assert_eq!(
            EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry),
            Err(TransportError::SemanticContractChanged(SemanticId::new(
                5001
            )))
        );
        assert!(SemanticLawMigration::verify(&source, &target, &registry).is_ok());
    }

    #[test]
    fn environment_transport_compares_query_visible_modules_even_when_schema_does_not_use_them() {
        let query_equality = SemanticId::new(5050);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let schema = Schema::new(SchemaRevisionId::new(5050));
        let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        source_environment.pin_module(query_equality, exact);
        let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(2));
        target_environment.pin_module(query_equality, ci);
        let source = SemanticContext {
            schema: schema.clone(),
            environment: source_environment,
        };
        let target = SemanticContext {
            schema,
            environment: target_environment,
        };

        assert_eq!(
            EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry),
            Err(TransportError::SemanticContractChanged(query_equality))
        );
        assert!(SemanticLawMigration::verify(&source, &target, &registry).is_ok());
    }

    #[test]
    fn adding_query_visible_module_is_conservative_but_removing_it_is_not() {
        let extra = SemanticId::new(5060);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence(EquivalenceModule::TextExact);
        let schema = Schema::new(SchemaRevisionId::new(5060));
        let source = SemanticContext {
            schema: schema.clone(),
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(2));
        target_environment.pin_module(extra, exact);
        let target = SemanticContext {
            schema,
            environment: target_environment,
        };

        assert!(
            ConservativeSemanticEnvironmentExtension::verify(&source, &target, &registry).is_ok()
        );
        assert_eq!(
            ConservativeSemanticEnvironmentExtension::verify(&target, &source, &registry),
            Err(TransportError::NotConservativeSemanticExtension)
        );
    }

    #[test]
    fn relational_impact_is_preserved_by_identity_like_semantic_context_transports() {
        use kernel_change::Change;
        use kernel_model::Value;
        use kernel_query::RelExpr;

        let relation = SemanticId::new(5000);
        let extra_module = SemanticId::new(5070);
        let mut registry = SemanticRegistry::default();
        let exact_v1 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let exact_v2 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let tokenizer = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);

        let source = set_relation_context(1, exact_v1);
        let same_contract = set_relation_context(2, exact_v2);
        let equivalent =
            EquivalentSemanticEnvironmentTransport::verify(&source, &same_contract, &registry)
                .unwrap();

        let mut definitionally_equivalent = source.clone();
        definitionally_equivalent.schema.revision = SchemaRevisionId::new(5001);
        let definitional =
            DefinitionalTransport::verify(&source, &definitionally_equivalent, &registry).unwrap();

        let mut extended = source.clone();
        extended.environment.revision = SemanticEnvId::new(3);
        extended.environment.pin_module(extra_module, tokenizer);
        let conservative =
            ConservativeSemanticEnvironmentExtension::verify(&source, &extended, &registry)
                .unwrap();

        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into())]]);
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into())]]);
        let change = Change::Replace(next);
        let query = RelExpr::Scan(relation);

        assert!(
            equivalent
                .check_rel_impact_law(&query, &old, &change, &registry)
                .unwrap()
        );
        assert!(
            definitional
                .check_rel_impact_law(&query, &old, &change, &registry)
                .unwrap()
        );
        assert!(
            conservative
                .check_rel_impact_law(&query, &old, &change, &registry)
                .unwrap()
        );
    }

    #[test]
    fn genuine_semantic_law_migration_has_no_generic_relational_impact_invariance() {
        use kernel_change::Change;
        use kernel_model::Value;
        use kernel_query::{Impact, RelExpr, rel_impact_by_recompute};

        let relation = SemanticId::new(5000);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let source = set_relation_context(1, exact);
        let target = set_relation_context(2, ci);
        SemanticLawMigration::verify(&source, &target, &registry).unwrap();

        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into())]]);
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into())]]);
        let change = Change::Replace(next);
        let query = RelExpr::Scan(relation);

        assert_eq!(
            rel_impact_by_recompute(&query, &old, &change, &source, &registry),
            Impact::Changed
        );
        assert_eq!(
            rel_impact_by_recompute(&query, &old, &change, &target, &registry),
            Impact::Unaffected
        );
    }

    #[test]
    fn law_migration_revalidates_target_instead_of_silently_collapsing_set_rows() {
        let relation = SemanticId::new(5000);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let ci =
            registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 1);
        let source = set_relation_context(1, exact);
        let target = set_relation_context(2, ci);
        let migration = SemanticLawMigration::verify(&source, &target, &registry).unwrap();
        let mut state = DatabaseState::default();
        state.model.relations.insert(
            relation,
            vec![
                vec![kernel_model::Value::Text("A".into())],
                vec![kernel_model::Value::Text("a".into())],
            ],
        );
        let source_revision = kernel_revision::Revision::build(
            kernel_types::RevisionId::new(1),
            &source,
            &registry,
            state,
        )
        .unwrap();

        assert!(matches!(
            migration.transport_revision(
                &source_revision,
                kernel_types::RevisionId::new(2),
                &registry,
            ),
            Err(TransportError::InvalidRevision(
                kernel_revision::RevisionError::InvalidTypedModel(
                    kernel_validation::ValidationError::Semantic(
                        kernel_semantics::SemanticError::DuplicateRelationRow
                    )
                )
            ))
        ));
    }

    #[test]
    fn typed_relation_transport_reuses_relational_query_ir() {
        let source_relation = SemanticId::new(6000);
        let target_relation = SemanticId::new(6001);
        let i64_eq = SemanticId::new(6002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(60));
        environment.pin_module(i64_eq, digest);

        let mut source_schema = Schema::new(SchemaRevisionId::new(60));
        source_schema
            .define_relation(kernel_schema::RelationDef {
                id: source_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let source = SemanticContext {
            schema: source_schema,
            environment: environment.clone(),
        };

        let mut target_schema = Schema::new(SchemaRevisionId::new(61));
        target_schema
            .define_relation(kernel_schema::RelationDef {
                id: target_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let target = SemanticContext {
            schema: target_schema,
            environment,
        };

        let transport = TypedRelationTransport::verify(
            &source,
            &target,
            &registry,
            vec![RelationRewrite {
                target_relation,
                transform: RelExpr::Project {
                    input: Box::new(RelExpr::Scan(source_relation)),
                    columns: vec![0],
                },
            }],
        )
        .unwrap();
        let mut state = DatabaseState::default();
        state.model.relations.insert(
            source_relation,
            vec![
                vec![kernel_model::Value::I64(1)],
                vec![kernel_model::Value::I64(2)],
            ],
        );
        let source_revision = kernel_revision::Revision::build(
            kernel_types::RevisionId::new(1),
            &source,
            &registry,
            state,
        )
        .unwrap();
        let target_revision = transport
            .transport_revision(
                &source_revision,
                kernel_types::RevisionId::new(2),
                &registry,
            )
            .unwrap();

        assert_eq!(
            target_revision
                .state()
                .model
                .relations
                .get(&target_relation),
            Some(&vec![
                vec![kernel_model::Value::I64(1)],
                vec![kernel_model::Value::I64(2)],
            ])
        );
        assert!(
            !target_revision
                .state()
                .model
                .relations
                .contains_key(&source_relation)
        );
    }

    #[test]
    fn typed_relation_transport_checks_full_set_bag_semantics() {
        let source_relation = SemanticId::new(6100);
        let target_relation = SemanticId::new(6101);
        let text_eq = SemanticId::new(6102);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(61));
        environment.pin_module(text_eq, digest);

        let mut source_schema = Schema::new(SchemaRevisionId::new(61));
        source_schema
            .define_relation(kernel_schema::RelationDef {
                id: source_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let source = SemanticContext {
            schema: source_schema,
            environment: environment.clone(),
        };
        let mut target_schema = Schema::new(SchemaRevisionId::new(62));
        target_schema
            .define_relation(kernel_schema::RelationDef {
                id: target_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: kernel_schema::RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let target = SemanticContext {
            schema: target_schema,
            environment,
        };

        assert_eq!(
            TypedRelationTransport::verify(
                &source,
                &target,
                &registry,
                vec![RelationRewrite {
                    target_relation,
                    transform: RelExpr::Scan(source_relation),
                }],
            ),
            Err(TransportError::RelationTransformType(
                RelQueryError::TypeMismatch
            ))
        );
    }

    fn identity_transport_context(
        person: SemanticId,
        friend: SemanticId,
        relation: SemanticId,
        env_revision: u64,
        registry: &mut SemanticRegistry,
    ) -> SemanticContext {
        let live_eq = SemanticId::new(person.raw() + 10_000);
        let historical_eq = SemanticId::new(person.raw() + 20_000);
        let live_digest =
            registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let historical_digest =
            registry.install_equivalence(EquivalenceModule::HistoricalEntityIdExact(person));
        let mut schema = Schema::new(SchemaRevisionId::new(70));
        schema
            .define_field(FieldDef {
                id: friend,
                owner: person,
                value: TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
            })
            .unwrap();
        schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
                    TypeExpr::Scalar(ScalarType::HistoricalEntityId(person)),
                ],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![live_eq, historical_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
        environment.pin_module(live_eq, live_digest);
        environment.pin_module(historical_eq, historical_digest);
        SemanticContext {
            schema,
            environment,
        }
    }

    fn identity_transport_state(
        person: SemanticId,
        friend: SemanticId,
        relation: SemanticId,
    ) -> DatabaseState {
        let mut state = DatabaseState::default();
        state.lifecycle.entities = BTreeSet::from([
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(2),
        ]);
        state.lifecycle.roots.insert(kernel_types::EntityId::new(1));
        state.lifecycle.keeps_alive.insert(
            kernel_types::EntityId::new(1),
            BTreeSet::from([kernel_types::EntityId::new(2)]),
        );
        state
            .model
            .carriers
            .insert(person, state.lifecycle.entities.clone());
        state.model.fields.insert(
            (friend, kernel_types::EntityId::new(1)),
            kernel_model::Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(2),
            },
        );
        state.model.relations.insert(
            relation,
            vec![vec![
                kernel_model::Value::LiveEntityRef {
                    entity_type: person,
                    id: kernel_types::EntityId::new(1),
                },
                kernel_model::Value::HistoricalEntityId {
                    entity_type: person,
                    id: kernel_types::EntityId::new(2),
                },
            ]],
        );
        state
    }

    #[test]
    fn bijective_identity_transport_rewrites_lifecycle_carriers_and_nested_references_coherently() {
        let person = SemanticId::new(7000);
        let friend = SemanticId::new(7001);
        let relation = SemanticId::new(7002);
        let mut registry = SemanticRegistry::default();
        let source = identity_transport_context(person, friend, relation, 70, &mut registry);
        let target = identity_transport_context(person, friend, relation, 71, &mut registry);
        let old = BTreeSet::from([
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(2),
        ]);
        let new = BTreeSet::from([
            kernel_types::EntityId::new(11),
            kernel_types::EntityId::new(12),
        ]);
        let identity = kernel_identity::IdentityTransport::new(
            &old,
            &new,
            BTreeMap::from([
                (
                    kernel_types::EntityId::new(1),
                    kernel_types::EntityId::new(11),
                ),
                (
                    kernel_types::EntityId::new(2),
                    kernel_types::EntityId::new(12),
                ),
            ]),
        )
        .unwrap();
        let inverse = identity.inverse();
        let forward =
            BijectiveIdentityRevisionTransport::verify(&source, &target, &registry, identity)
                .unwrap();
        let backward =
            BijectiveIdentityRevisionTransport::verify(&target, &source, &registry, inverse)
                .unwrap();
        let source_revision = kernel_revision::Revision::build(
            kernel_types::RevisionId::new(1),
            &source,
            &registry,
            identity_transport_state(person, friend, relation),
        )
        .unwrap();
        let mapped = forward
            .transport_revision(
                &source_revision,
                kernel_types::RevisionId::new(2),
                &registry,
            )
            .unwrap();
        assert_eq!(mapped.state().lifecycle.entities, new);
        assert_eq!(
            mapped
                .state()
                .model
                .fields
                .get(&(friend, kernel_types::EntityId::new(11))),
            Some(&kernel_model::Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(12),
            })
        );
        assert!(matches!(
            &mapped.state().model.relations[&relation][0][1],
            kernel_model::Value::HistoricalEntityId { id, .. }
                if *id == kernel_types::EntityId::new(12)
        ));
        let round_trip = backward
            .transport_revision(&mapped, kernel_types::RevisionId::new(3), &registry)
            .unwrap();
        assert_eq!(round_trip.state(), source_revision.state());
    }

    #[test]
    fn typed_transport_rejects_transform_whose_output_type_disagrees_with_target() {
        let entity_type = SemanticId::new(30);
        let source_field = SemanticId::new(31);
        let target_field = SemanticId::new(32);
        let registry = SemanticRegistry::default();
        let mut source_schema = Schema::new(SchemaRevisionId::new(20));
        source_schema
            .define_field(FieldDef {
                id: source_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let mut target_schema = Schema::new(SchemaRevisionId::new(21));
        target_schema
            .define_field(FieldDef {
                id: target_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        let source = SemanticContext {
            schema: source_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(20)),
        };
        let target = SemanticContext {
            schema: target_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(21)),
        };
        let error = TypedFieldTransport::verify(
            &source,
            &target,
            &registry,
            vec![FieldRewrite {
                source_field,
                target_field,
                transform: ExactQuery::new(kernel_query::Expr::Input),
            }],
        )
        .unwrap_err();
        assert_eq!(
            error,
            TransportError::TransformType(QueryTypeError::TypeMismatch)
        );
    }
    #[test]
    fn field_transport_cannot_silently_reinterpret_semantic_environment() {
        let entity_type = SemanticId::new(40);
        let field = SemanticId::new(41);
        let eq = SemanticId::new(42);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(30));
        schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: eq,
                },
            })
            .unwrap();
        let mut source_env = SemanticEnvironment::new(SemanticEnvId::new(30));
        source_env.pin_module(eq, exact);
        let mut target_env = SemanticEnvironment::new(SemanticEnvId::new(31));
        target_env.pin_module(eq, ci);
        let source = SemanticContext {
            schema: schema.clone(),
            environment: source_env,
        };
        let target = SemanticContext {
            schema,
            environment: target_env,
        };
        assert_eq!(
            TypedFieldTransport::verify(&source, &target, &registry, vec![]),
            Err(TransportError::SemanticEnvironmentChangeRequiresTransport)
        );
    }
    #[test]
    fn impact_commutes_with_bijective_identity_transport() {
        use kernel_change::Change;
        use kernel_model::Value;
        use kernel_query::{ExactQuery, Expr};
        use std::collections::{BTreeMap, BTreeSet};

        let entity_type = SemanticId::new(9900);
        let source_ids = BTreeSet::from([
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(2),
        ]);
        let target_ids = BTreeSet::from([
            kernel_types::EntityId::new(11),
            kernel_types::EntityId::new(12),
        ]);
        let identity = IdentityTransport::new(
            &source_ids,
            &target_ids,
            BTreeMap::from([
                (
                    kernel_types::EntityId::new(1),
                    kernel_types::EntityId::new(11),
                ),
                (
                    kernel_types::EntityId::new(2),
                    kernel_types::EntityId::new(12),
                ),
            ]),
        )
        .unwrap();
        let old = Value::LiveEntityRef {
            entity_type,
            id: kernel_types::EntityId::new(1),
        };
        let change = Change::Replace(Value::LiveEntityRef {
            entity_type,
            id: kernel_types::EntityId::new(2),
        });
        let query = ExactQuery::new(Expr::Input);

        assert!(check_impact_transport_law(&identity, &query, &old, &change).unwrap());
    }

    #[test]
    fn relational_impact_commutes_with_bijective_identity_transport() {
        use kernel_change::Change;
        use kernel_model::Value;
        use kernel_query::RelExpr;
        use kernel_schema::{RelationDef, RelationSemantics};
        use std::collections::{BTreeMap, BTreeSet};

        let person = SemanticId::new(9910);
        let relation = SemanticId::new(9911);
        let equality = SemanticId::new(9912);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9910));
        environment.pin_module(equality, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9910));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equality],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };

        let source_ids = BTreeSet::from([
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(2),
        ]);
        let target_ids = BTreeSet::from([
            kernel_types::EntityId::new(11),
            kernel_types::EntityId::new(12),
        ]);
        let identity = IdentityTransport::new(
            &source_ids,
            &target_ids,
            BTreeMap::from([
                (
                    kernel_types::EntityId::new(1),
                    kernel_types::EntityId::new(11),
                ),
                (
                    kernel_types::EntityId::new(2),
                    kernel_types::EntityId::new(12),
                ),
            ]),
        )
        .unwrap();

        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![vec![Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(1),
            }]],
        );
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![vec![Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(2),
            }]],
        );
        let change = Change::Replace(next);
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(1),
            },
            equivalence: equality,
        };

        assert!(
            check_rel_impact_transport_law(&identity, &query, &old, &change, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn retention_domain_mapping_does_not_resurrect_non_live_ids() {
        use std::collections::{BTreeMap, BTreeSet};

        let source_ids = BTreeSet::from([
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(2),
        ]);
        let target_ids = BTreeSet::from([
            kernel_types::EntityId::new(11),
            kernel_types::EntityId::new(12),
        ]);
        let identity = IdentityTransport::new(
            &source_ids,
            &target_ids,
            BTreeMap::from([
                (
                    kernel_types::EntityId::new(1),
                    kernel_types::EntityId::new(11),
                ),
                (
                    kernel_types::EntityId::new(2),
                    kernel_types::EntityId::new(12),
                ),
            ]),
        )
        .unwrap();
        let mut state = DatabaseState::default();
        state
            .lifecycle
            .entities
            .insert(kernel_types::EntityId::new(1));
        state.lifecycle.roots.insert(kernel_types::EntityId::new(1));

        let transported = transport_database_state(&identity, &state).unwrap();
        assert_eq!(
            transported.lifecycle.entities,
            BTreeSet::from([kernel_types::EntityId::new(11)])
        );
        assert!(
            !transported
                .lifecycle
                .entities
                .contains(&kernel_types::EntityId::new(12))
        );
    }
    #[test]
    fn historical_ids_must_be_covered_by_identity_retention_domain() {
        use std::collections::{BTreeMap, BTreeSet};

        let entity_type = SemanticId::new(9950);
        let identity = IdentityTransport::new(
            &BTreeSet::from([kernel_types::EntityId::new(1)]),
            &BTreeSet::from([kernel_types::EntityId::new(11)]),
            BTreeMap::from([(
                kernel_types::EntityId::new(1),
                kernel_types::EntityId::new(11),
            )]),
        )
        .unwrap();
        let historical = kernel_model::Value::HistoricalEntityId {
            entity_type,
            id: kernel_types::EntityId::new(2),
        };
        assert_eq!(
            transport_value(&identity, &historical),
            Err(TransportError::IdentitySourceCoverageMismatch)
        );
    }
    #[test]
    fn semantic_environment_transport_is_generic_across_tokenizer_modules() {
        let tokenizer = SemanticId::new(9960);
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 1);
        let v2 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 2);
        let changed = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
        let make_context = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(tokenizer, digest);
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(9960)),
                environment,
            }
        };
        let source = make_context(1, v1);
        let same_contract = make_context(2, v2);
        let new_contract = make_context(3, changed);

        assert!(
            EquivalentSemanticEnvironmentTransport::verify(&source, &same_contract, &registry)
                .is_ok()
        );
        assert_eq!(
            EquivalentSemanticEnvironmentTransport::verify(&source, &new_contract, &registry),
            Err(TransportError::SemanticContractChanged(tokenizer))
        );
        assert!(SemanticLawMigration::verify(&source, &new_contract, &registry).is_ok());
    }

    #[test]
    fn semantic_environment_transport_is_generic_across_ordering_modules() {
        let ordering = SemanticId::new(9970);
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_ordering_revision(OrderingModule::TextBinary, 1);
        let v2 = registry.install_ordering_revision(OrderingModule::TextBinary, 2);
        let changed = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);

        let context_with = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(ordering, digest);
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(9970)),
                environment,
            }
        };
        let source = context_with(1, v1);
        let upgraded = context_with(2, v2);
        let changed_law = context_with(3, changed);

        EquivalentSemanticEnvironmentTransport::verify(&source, &upgraded, &registry).unwrap();
        assert_eq!(
            EquivalentSemanticEnvironmentTransport::verify(&source, &changed_law, &registry),
            Err(TransportError::SemanticContractChanged(ordering))
        );
    }
}
