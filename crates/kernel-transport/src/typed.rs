use std::collections::{BTreeMap, BTreeSet};

use kernel_model::{DatabaseState, FiniteModel};
use kernel_query::{ExactQuery, QueryTypeError, RelExpr, RelQueryError};
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;

use crate::TransportError;

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
struct PreparedRelationRewrite {
    target_relation: kernel_types::SemanticId,
    transform: kernel_query::PreparedRelExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedRelationTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites: Vec<PreparedRelationRewrite>,
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
        let mut prepared_rewrites = Vec::new();
        for target_relation in target.schema.relations() {
            if source.schema.relation(target_relation.id) == Some(target_relation) {
                if by_target.contains_key(&target_relation.id) {
                    return Err(TransportError::RewriteTargetsPassthroughRelation(
                        target_relation.id,
                    ));
                }
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
            prepared_rewrites.push(PreparedRelationRewrite {
                target_relation: target_relation.id,
                transform: prepared,
            });
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
            rewrites: prepared_rewrites,
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
            let result = rewrite
                .transform
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
struct PreparedFieldRewrite {
    target_field: kernel_types::SemanticId,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedFieldTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites_by_source: BTreeMap<kernel_types::SemanticId, Vec<PreparedFieldRewrite>>,
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
        let mut rewrites_by_source = BTreeMap::<_, Vec<_>>::new();
        for target_field in target.schema.fields() {
            if let Some(source_field) = source.schema.field(target_field.id)
                && source_field == target_field
            {
                if by_target.contains_key(&target_field.id) {
                    return Err(TransportError::RewriteTargetsPassthroughField(
                        target_field.id,
                    ));
                }
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
            rewrites_by_source
                .entry(rewrite.source_field)
                .or_default()
                .push(PreparedFieldRewrite {
                    target_field: target_field.id,
                    transform: rewrite.transform.clone(),
                });
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
            rewrites_by_source,
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
            if let Some(rewrites) = self.rewrites_by_source.get(&field) {
                for rewrite in rewrites {
                    let transformed = rewrite
                        .transform
                        .evaluate(value)
                        .map_err(TransportError::TransformExecution)?;
                    model
                        .fields
                        .insert((rewrite.target_field, entity), transformed);
                }
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
