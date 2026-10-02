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
            let relation_definition_unchanged =
                source.schema.relation(target_relation.id) == Some(target_relation);
            let relation_column_identity_unchanged =
                source.schema.relation_column_ids(target_relation.id)
                    == target.schema.relation_column_ids(target_relation.id);
            if relation_definition_unchanged && relation_column_identity_unchanged {
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
                model.relations.insert_shared(*relation, rows.clone());
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

/// One target field in a schema migration.
///
/// `source_fields` are assembled into a product keyed by their semantic IDs and
/// passed to `transform`. An empty list supplies an empty product, which makes
/// constant/default field creation explicit. Multiple targets may read the same
/// source coordinates (split), and one target may read multiple source
/// coordinates (merge). Source fields not referenced by the target schema are
/// naturally dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFieldRewrite {
    pub source_fields: Vec<kernel_types::SemanticId>,
    pub target_field: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedMigrationFieldRewrite {
    owner: kernel_types::SemanticId,
    source_fields: Vec<kernel_types::SemanticId>,
    target_field: kernel_types::SemanticId,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationColumnRewrite {
    pub source_columns: Vec<kernel_types::SemanticId>,
    pub target_column: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRowRewrite {
    pub source_relation: kernel_types::SemanticId,
    pub target_relation: kernel_types::SemanticId,
    pub columns: Vec<MigrationColumnRewrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationRewrite {
    Query(RelationRewrite),
    Rows(MigrationRowRewrite),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedMigrationColumnRewrite {
    source_columns: Vec<(kernel_types::SemanticId, usize)>,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations to this representation."
)]
enum PreparedMigrationRelationRewrite {
    Query {
        rewrite: PreparedRelationRewrite,
        source_relations: BTreeSet<kernel_types::SemanticId>,
    },
    Rows {
        source_relation: kernel_types::SemanticId,
        target_relation: kernel_types::SemanticId,
        columns: Vec<PreparedMigrationColumnRewrite>,
    },
}

/// One independently materializable target-relation slice of a verified
/// schema migration.
///
/// The coordinate is derived from the verified forward program; it is not a
/// second migration-progress journal. A storage layer can therefore retain a
/// source-epoch physical slice until this target relation is materialized,
/// then advance only that coordinate to native target representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationSlice {
    /// Source and target relation definitions are identical. The same logical
    /// rows are valid under the target schema without rewriting bytes/values.
    Passthrough { relation: kernel_types::SemanticId },
    /// Every target row depends only on one source row from one relation.
    /// This is the preferred streaming/background rewrite primitive.
    RowLocal {
        source_relation: kernel_types::SemanticId,
        target_relation: kernel_types::SemanticId,
    },
    /// General deterministic relational rewrite. The exact source dependency
    /// set is explicit so a mixed-representation scheduler never needs to
    /// route through an opaque generic fallback.
    Query {
        target_relation: kernel_types::SemanticId,
        source_relations: BTreeSet<kernel_types::SemanticId>,
    },
}

impl MigrationRelationSlice {
    #[must_use]
    pub const fn target_relation(&self) -> kernel_types::SemanticId {
        match self {
            Self::Passthrough { relation } => *relation,
            Self::RowLocal {
                target_relation, ..
            }
            | Self::Query {
                target_relation, ..
            } => *target_relation,
        }
    }

    #[must_use]
    pub fn source_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        match self {
            Self::Passthrough { relation } => BTreeSet::from([*relation]),
            Self::RowLocal {
                source_relation, ..
            } => BTreeSet::from([*source_relation]),
            Self::Query {
                source_relations, ..
            } => source_relations.clone(),
        }
    }
}

#[must_use]
pub const fn migration_column_input_id(
    column: kernel_types::SemanticId,
) -> kernel_types::SemanticId {
    column
}

/// Canonical serializable description of one deterministic schema migration.
///
/// The program carries only target semantics and deterministic rewrites. The
/// source world is supplied by the causal source revision when the program is
/// verified/replayed, so durable migration identity never needs a second full
/// target-state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMigrationProgram {
    target: SemanticContext,
    field_rewrites: Vec<MigrationFieldRewrite>,
    relation_rewrites: Vec<MigrationRelationRewrite>,
}

impl SchemaMigrationProgram {
    #[must_use]
    pub fn new(
        target: SemanticContext,
        field_rewrites: Vec<MigrationFieldRewrite>,
        relation_rewrites: Vec<MigrationRelationRewrite>,
    ) -> Self {
        Self {
            target,
            field_rewrites,
            relation_rewrites,
        }
    }

    #[must_use]
    pub const fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn field_rewrites(&self) -> &[MigrationFieldRewrite] {
        &self.field_rewrites
    }

    #[must_use]
    pub fn relation_rewrites(&self) -> &[MigrationRelationRewrite] {
        &self.relation_rewrites
    }

    pub fn verify(
        &self,
        source: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<SchemaMigrationTransport, TransportError> {
        SchemaMigrationTransport::verify(
            source,
            &self.target,
            registry,
            self.field_rewrites.clone(),
            self.relation_rewrites.clone(),
        )
    }
}

/// Verified one-step structural data migration between two schema revisions.
///
/// Unlike `TypedFieldTransport`/`TypedRelationTransport`, this transport may
/// change fields and relations in the same atomic step. The migration is still
/// deliberately conservative about entity/type/lifecycle structure: those
/// coordinates must remain definitionally equal. Frontends can therefore
/// compile rename, conversion, split, merge, create/default and drop operations
/// into one deterministic kernel object without embedding host callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMigrationTransport {
    source: SemanticContext,
    target: SemanticContext,
    field_rewrites: Vec<PreparedMigrationFieldRewrite>,
    field_passthrough: BTreeSet<kernel_types::SemanticId>,
    relation_rewrites: Vec<PreparedMigrationRelationRewrite>,
    relation_passthrough: BTreeSet<kernel_types::SemanticId>,
}

impl SchemaMigrationTransport {
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        field_rewrites: Vec<MigrationFieldRewrite>,
        relation_rewrites: Vec<MigrationRelationRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.migration_base_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut fields_by_target = BTreeMap::new();
        for rewrite in field_rewrites {
            let target = rewrite.target_field;
            if fields_by_target.insert(target, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetField(target));
            }
        }

        let mut field_passthrough = BTreeSet::new();
        let mut prepared_fields = Vec::new();
        for target_field in target.schema.fields() {
            if source.schema.field(target_field.id) == Some(target_field) {
                if fields_by_target.contains_key(&target_field.id) {
                    return Err(TransportError::RewriteTargetsPassthroughField(
                        target_field.id,
                    ));
                }
                field_passthrough.insert(target_field.id);
                continue;
            }

            let rewrite = fields_by_target
                .get(&target_field.id)
                .ok_or(TransportError::UnknownTargetField(target_field.id))?;
            let mut source_types = BTreeMap::new();
            for source_field_id in &rewrite.source_fields {
                let source_field = source
                    .schema
                    .field(*source_field_id)
                    .ok_or(TransportError::UnknownSourceField(*source_field_id))?;
                if source_field.owner != target_field.owner {
                    return Err(TransportError::MigrationFieldOwnerMismatch(
                        *source_field_id,
                    ));
                }
                if source_types
                    .insert(*source_field_id, source_field.value.clone())
                    .is_some()
                {
                    return Err(TransportError::DuplicateMigrationSourceField(
                        *source_field_id,
                    ));
                }
            }
            let input = kernel_schema::TypeExpr::Product(source_types);
            let result = rewrite
                .transform
                .typecheck(&input)
                .map_err(TransportError::TransformType)?;
            if result != target_field.value {
                return Err(TransportError::TransformType(QueryTypeError::TypeMismatch));
            }
            prepared_fields.push(PreparedMigrationFieldRewrite {
                owner: target_field.owner,
                source_fields: rewrite.source_fields.clone(),
                target_field: target_field.id,
                transform: rewrite.transform.clone(),
            });
        }
        let known_target_fields: BTreeSet<_> =
            target.schema.fields().map(|field| field.id).collect();
        if let Some(unknown) = fields_by_target
            .keys()
            .find(|field| !known_target_fields.contains(field))
        {
            return Err(TransportError::UnknownTargetField(*unknown));
        }

        let mut relations_by_target = BTreeMap::new();
        for rewrite in relation_rewrites {
            let target = match &rewrite {
                MigrationRelationRewrite::Query(rewrite) => rewrite.target_relation,
                MigrationRelationRewrite::Rows(rewrite) => rewrite.target_relation,
            };
            if relations_by_target.insert(target, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetRelation(target));
            }
        }
        let mut relation_passthrough = BTreeSet::new();
        let mut prepared_relations = Vec::new();
        for target_relation in target.schema.relations() {
            let relation_definition_unchanged =
                source.schema.relation(target_relation.id) == Some(target_relation);
            let relation_column_identity_unchanged =
                source.schema.relation_column_ids(target_relation.id)
                    == target.schema.relation_column_ids(target_relation.id);
            if relation_definition_unchanged && relation_column_identity_unchanged {
                if relations_by_target.contains_key(&target_relation.id) {
                    return Err(TransportError::RewriteTargetsPassthroughRelation(
                        target_relation.id,
                    ));
                }
                relation_passthrough.insert(target_relation.id);
                continue;
            }
            let rewrite = relations_by_target
                .get(&target_relation.id)
                .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
            match rewrite {
                MigrationRelationRewrite::Query(rewrite) => {
                    let source_relations = rewrite.transform.scan_relations();
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
                    prepared_relations.push(PreparedMigrationRelationRewrite::Query {
                        rewrite: PreparedRelationRewrite {
                            target_relation: target_relation.id,
                            transform: prepared,
                        },
                        source_relations,
                    });
                }
                MigrationRelationRewrite::Rows(rewrite) => {
                    let source_relation = source.schema.relation(rewrite.source_relation).ok_or(
                        TransportError::UnknownSourceRelation(rewrite.source_relation),
                    )?;
                    let target_column_ids =
                        target
                            .schema
                            .relation_column_ids(target_relation.id)
                            .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
                    let mut by_target_column = BTreeMap::new();
                    for column in &rewrite.columns {
                        if target
                            .schema
                            .relation_column_ordinal(target_relation.id, column.target_column)
                            .is_none()
                        {
                            return Err(TransportError::MissingTargetMigrationColumn(
                                target_relation.id,
                                column.target_column,
                            ));
                        }
                        if by_target_column
                            .insert(column.target_column, column)
                            .is_some()
                        {
                            return Err(TransportError::DuplicateTargetMigrationColumn(
                                target_relation.id,
                                column.target_column,
                            ));
                        }
                    }
                    let mut prepared_columns = Vec::with_capacity(target_relation.columns.len());
                    for (target_ordinal, target_column_id) in
                        target_column_ids.iter().copied().enumerate()
                    {
                        let column = by_target_column.get(&target_column_id).ok_or(
                            TransportError::MissingTargetMigrationColumn(
                                target_relation.id,
                                target_column_id,
                            ),
                        )?;
                        let mut input_fields = BTreeMap::new();
                        let mut source_columns = Vec::with_capacity(column.source_columns.len());
                        for source_column_id in &column.source_columns {
                            let source_ordinal = source
                                .schema
                                .relation_column_ordinal(rewrite.source_relation, *source_column_id)
                                .ok_or(TransportError::UnknownSourceMigrationColumn(
                                    rewrite.source_relation,
                                    *source_column_id,
                                ))?;
                            let source_ty = &source_relation.columns[source_ordinal];
                            input_fields.insert(*source_column_id, source_ty.clone());
                            source_columns.push((*source_column_id, source_ordinal));
                        }
                        let result = column
                            .transform
                            .typecheck(&kernel_schema::TypeExpr::Product(input_fields))
                            .map_err(TransportError::TransformType)?;
                        if result != target_relation.columns[target_ordinal] {
                            return Err(TransportError::TransformType(
                                QueryTypeError::TypeMismatch,
                            ));
                        }
                        prepared_columns.push(PreparedMigrationColumnRewrite {
                            source_columns,
                            transform: column.transform.clone(),
                        });
                    }
                    prepared_relations.push(PreparedMigrationRelationRewrite::Rows {
                        source_relation: rewrite.source_relation,
                        target_relation: rewrite.target_relation,
                        columns: prepared_columns,
                    });
                }
            }
        }
        let known_target_relations: BTreeSet<_> = target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect();
        if let Some(unknown) = relations_by_target
            .keys()
            .find(|relation| !known_target_relations.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            field_rewrites: prepared_fields,
            field_passthrough,
            relation_rewrites: prepared_relations,
            relation_passthrough,
        })
    }

    /// Returns the independently materializable physical coordinate for one
    /// target relation.
    ///
    /// The result is derived entirely from the verified migration program and
    /// therefore survives recovery/replay without storing mutable progress in
    /// a second journal. `None` means the requested relation is not part of the
    /// target schema.
    #[must_use]
    pub fn relation_slice(
        &self,
        target_relation: kernel_types::SemanticId,
    ) -> Option<MigrationRelationSlice> {
        if self.relation_passthrough.contains(&target_relation) {
            return Some(MigrationRelationSlice::Passthrough {
                relation: target_relation,
            });
        }
        self.relation_rewrites
            .iter()
            .find_map(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Query {
                    rewrite,
                    source_relations,
                } if rewrite.target_relation == target_relation => {
                    Some(MigrationRelationSlice::Query {
                        target_relation,
                        source_relations: source_relations.clone(),
                    })
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation,
                    target_relation: rewrite_target,
                    ..
                } if *rewrite_target == target_relation => Some(MigrationRelationSlice::RowLocal {
                    source_relation: *source_relation,
                    target_relation,
                }),
                _ => None,
            })
    }

    /// Computes the exact source-epoch relation set that must remain readable
    /// while the supplied target relations are already present natively.
    ///
    /// `native_target_relations` is intentionally an input from physical
    /// authority rather than state owned by the migration calculus. This keeps
    /// migration progress derivable from actual stored coordinates and avoids
    /// a second mutable progress journal.
    pub fn required_source_relations(
        &self,
        native_target_relations: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<BTreeSet<kernel_types::SemanticId>, TransportError> {
        let target_relations = self
            .target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = native_target_relations
            .iter()
            .find(|relation| !target_relations.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        let mut required = BTreeSet::new();
        for target_relation in target_relations {
            if native_target_relations.contains(&target_relation) {
                continue;
            }
            let slice = self
                .relation_slice(target_relation)
                .ok_or(TransportError::UnknownTargetRelation(target_relation))?;
            required.extend(slice.source_relations());
        }
        Ok(required)
    }

    /// Deterministically materializes exactly one target relation from the
    /// source-epoch logical state.
    ///
    /// This is deliberately narrower than `transport_revision`: it does not
    /// claim that the whole target world has been materialized or globally
    /// validated. It is the physical A→B slice primitive used by mixed-epoch
    /// storage/background rewrite. Full target validation remains at the
    /// semantic cutover boundary.
    pub fn materialize_relation_slice(
        &self,
        source_state: &DatabaseState,
        target_relation: kernel_types::SemanticId,
        registry: &SemanticRegistry,
    ) -> Result<Vec<kernel_query::Row>, TransportError> {
        if self.relation_passthrough.contains(&target_relation) {
            return Ok(source_state
                .model
                .relations
                .materialize_owned(&target_relation)
                .unwrap_or_default());
        }

        let rewrite = self
            .relation_rewrites
            .iter()
            .find(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    rewrite.target_relation == target_relation
                }
                PreparedMigrationRelationRewrite::Rows {
                    target_relation: rewrite_target,
                    ..
                } => *rewrite_target == target_relation,
            })
            .ok_or(TransportError::UnknownTargetRelation(target_relation))?;

        match rewrite {
            PreparedMigrationRelationRewrite::Query { rewrite, .. } => rewrite
                .transform
                .evaluate(&source_state.model, &self.source, registry)
                .map(|value| value.rows().to_vec())
                .map_err(TransportError::RelationTransformExecution),
            PreparedMigrationRelationRewrite::Rows {
                source_relation,
                target_relation: _,
                columns,
            } => {
                let source_rows = source_state
                    .model
                    .relations
                    .materialize_owned(source_relation)
                    .unwrap_or_default();
                let mut target_rows = Vec::with_capacity(source_rows.len());
                for source_row in source_rows {
                    let mut target_row = Vec::with_capacity(columns.len());
                    for column in columns {
                        let mut input = BTreeMap::new();
                        for (source_column_id, source_ordinal) in &column.source_columns {
                            let value = source_row.get(*source_ordinal).ok_or(
                                TransportError::UnknownSourceMigrationColumn(
                                    *source_relation,
                                    *source_column_id,
                                ),
                            )?;
                            input.insert(*source_column_id, value.clone());
                        }
                        target_row.push(
                            column
                                .transform
                                .evaluate(&kernel_model::Value::Product(input))
                                .map_err(TransportError::TransformExecution)?,
                        );
                    }
                    target_rows.push(target_row);
                }
                Ok(target_rows)
            }
        }
    }

    /// Transforms one physical source row for a row-local migration slice.
    ///
    /// This is the bounded-memory primitive for background physical rewrite:
    /// callers may scan/chunk source storage and publish target-native rows
    /// without ever constructing the whole relation in memory. General query
    /// slices are rejected explicitly because their semantics may depend on
    /// multiple relations or global relational operators.
    pub fn transform_row_local_slice(
        &self,
        target_relation: kernel_types::SemanticId,
        source_row: &kernel_query::Row,
    ) -> Result<kernel_query::Row, TransportError> {
        let rewrite = self
            .relation_rewrites
            .iter()
            .find(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Rows {
                    target_relation: rewrite_target,
                    ..
                } => *rewrite_target == target_relation,
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    rewrite.target_relation == target_relation
                }
            })
            .ok_or(TransportError::UnknownTargetRelation(target_relation))?;

        let PreparedMigrationRelationRewrite::Rows {
            source_relation,
            columns,
            ..
        } = rewrite
        else {
            return Err(TransportError::MigrationSliceNotRowLocal(target_relation));
        };

        let mut target_row = Vec::with_capacity(columns.len());
        for column in columns {
            let mut input = BTreeMap::new();
            for (source_column_id, source_ordinal) in &column.source_columns {
                let value = source_row.get(*source_ordinal).ok_or(
                    TransportError::UnknownSourceMigrationColumn(
                        *source_relation,
                        *source_column_id,
                    ),
                )?;
                input.insert(*source_column_id, value.clone());
            }
            target_row.push(
                column
                    .transform
                    .evaluate(&kernel_model::Value::Product(input))
                    .map_err(TransportError::TransformExecution)?,
            );
        }
        Ok(target_row)
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut fields = BTreeMap::new();
        for (&(field, entity), value) in &source_state.model.fields {
            if self.field_passthrough.contains(&field) {
                fields.insert((field, entity), value.clone());
            }
        }
        for rewrite in &self.field_rewrites {
            let entities = source_state
                .model
                .carriers
                .get(&rewrite.owner)
                .cloned()
                .unwrap_or_default();
            for entity in entities {
                let mut input_fields = BTreeMap::new();
                for source_field in &rewrite.source_fields {
                    let value = source_state
                        .model
                        .fields
                        .get(&(*source_field, entity))
                        .ok_or(TransportError::MissingMigrationSourceValue(*source_field))?;
                    input_fields.insert(*source_field, value.clone());
                }
                let transformed = rewrite
                    .transform
                    .evaluate(&kernel_model::Value::Product(input_fields))
                    .map_err(TransportError::TransformExecution)?;
                fields.insert((rewrite.target_field, entity), transformed);
            }
        }

        let mut relations = kernel_model::RelationStore::default();
        for relation in &self.relation_passthrough {
            if let Some(rows) = source_state.model.relations.get(relation) {
                relations.insert_shared(*relation, rows.clone());
            }
        }
        for rewrite in &self.relation_rewrites {
            match rewrite {
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    let result = rewrite
                        .transform
                        .evaluate(&source_state.model, &self.source, registry)
                        .map_err(TransportError::RelationTransformExecution)?;
                    relations.insert(rewrite.target_relation, result.rows().to_vec());
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation,
                    target_relation,
                    columns,
                } => {
                    let source_rows = source_state
                        .model
                        .relations
                        .materialize_owned(source_relation)
                        .unwrap_or_default();
                    let mut target_rows = Vec::with_capacity(source_rows.len());
                    for source_row in source_rows {
                        let mut target_row = Vec::with_capacity(columns.len());
                        for column in columns {
                            let mut input = BTreeMap::new();
                            for (source_column_id, source_ordinal) in &column.source_columns {
                                let value = source_row.get(*source_ordinal).ok_or(
                                    TransportError::UnknownSourceMigrationColumn(
                                        *source_relation,
                                        *source_column_id,
                                    ),
                                )?;
                                input.insert(*source_column_id, value.clone());
                            }
                            target_row.push(
                                column
                                    .transform
                                    .evaluate(&kernel_model::Value::Product(input))
                                    .map_err(TransportError::TransformExecution)?,
                            );
                        }
                        target_rows.push(target_row);
                    }
                    relations.insert(*target_relation, target_rows);
                }
            }
        }

        let state = DatabaseState {
            model: FiniteModel {
                carriers: source_state.model.carriers.clone(),
                fields: fields.into(),
                relations,
            },
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
