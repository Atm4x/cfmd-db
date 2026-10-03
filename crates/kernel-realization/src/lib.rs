use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

use kernel_persistent::{PersistentOrdMap, PersistentOrdMapStorageProbe};

use kernel_lifecycle::LifecycleGraph;
use kernel_model::{DatabaseState, Value};
use kernel_query::{ExactQuery, QueryError};
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;
use kernel_transport::{MigrationRelationRewrite, SchemaMigrationProgram, TransportError};
use kernel_types::{EntityId, SemanticId};

mod factorized;
mod one_shot;
pub use factorized::{
    CarrierSegmentCoordinate, DirectFactorizedFieldRoot, DirectFactorizedRealizationRoot,
    DirectFactorizedRelationRoot, FactorizedFieldExpr, FactorizedFieldNativeChunk,
    FactorizedFieldRule, FactorizedRealizationRoot, FactorizedRelationColumnExpr,
    FactorizedRelationRule, FieldColumnSegment, PreparedFactorizedRelation, RelationColumnSegment,
    RelationCompactionDecision, RelationCompactionWorkloadCost, RelationDeltaOverlayStats,
    compose_schema_migration_factorized, compose_schema_migration_factorized_with_prepared,
    prepare_general_relation_factorized, realize_database_state_factorized,
    relation_compaction_decision,
};
pub use one_shot::{RelExecutionSink, RelExecutionSource, evaluate_relation_expr_factorized};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAtomId(u128);

impl PhysicalAtomId {
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalCodec {
    Value,
    EntitySet,
    EntityOrder,
    RelationRows,
    FieldColumnSegment,
    RelationColumnSegment,
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhysicalAtomPayload {
    Value(Value),
    EntitySet(BTreeSet<EntityId>),
    EntityOrder(Vec<EntityId>),
    RelationRows(Vec<Vec<Value>>),
    FieldColumnSegment(FieldColumnSegment),
    RelationColumnSegment(RelationColumnSegment),
    Lifecycle(LifecycleGraph),
}

impl PhysicalAtomPayload {
    #[must_use]
    pub const fn codec(&self) -> PhysicalCodec {
        match self {
            Self::Value(_) => PhysicalCodec::Value,
            Self::EntitySet(_) => PhysicalCodec::EntitySet,
            Self::EntityOrder(_) => PhysicalCodec::EntityOrder,
            Self::RelationRows(_) => PhysicalCodec::RelationRows,
            Self::FieldColumnSegment(_) => PhysicalCodec::FieldColumnSegment,
            Self::RelationColumnSegment(_) => PhysicalCodec::RelationColumnSegment,
            Self::Lifecycle(_) => PhysicalCodec::Lifecycle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalAtom {
    codec: PhysicalCodec,
    payload: PhysicalAtomPayload,
}

impl PhysicalAtom {
    #[must_use]
    pub const fn codec(&self) -> PhysicalCodec {
        self.codec
    }

    #[must_use]
    pub const fn payload(&self) -> &PhysicalAtomPayload {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhysicalAtomStore {
    atoms: PersistentOrdMap<PhysicalAtomId, Arc<PhysicalAtom>>,
    next_id: u128,
}

#[derive(Debug)]
pub struct PhysicalAtomStoreProbe {
    map_nodes: PersistentOrdMapStorageProbe<PhysicalAtomId, Arc<PhysicalAtom>>,
    atoms: Vec<Weak<PhysicalAtom>>,
}

impl PhysicalAtomStoreProbe {
    #[must_use]
    pub fn map_nodes(&self) -> usize {
        self.map_nodes.total_nodes()
    }

    #[must_use]
    pub fn live_map_nodes(&self) -> usize {
        self.map_nodes.live_nodes()
    }

    #[must_use]
    pub fn atom_allocations(&self) -> usize {
        self.atoms.len()
    }

    #[must_use]
    pub fn live_atom_allocations(&self) -> usize {
        self.atoms
            .iter()
            .filter(|atom| atom.strong_count() != 0)
            .count()
    }
}

impl PhysicalAtomStore {
    pub fn insert(&mut self, payload: PhysicalAtomPayload) -> PhysicalAtomId {
        let id = PhysicalAtomId::new(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("physical atom id exhausted");
        let codec = payload.codec();
        self.atoms
            .insert(id, Arc::new(PhysicalAtom { codec, payload }));
        id
    }

    #[must_use]
    pub fn get(&self, id: PhysicalAtomId) -> Option<&PhysicalAtom> {
        self.atoms.get(&id).map(Arc::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = (PhysicalAtomId, &PhysicalAtom)> {
        self.atoms.iter().map(|(id, atom)| (*id, atom.as_ref()))
    }

    pub fn from_exact_atoms(
        atoms: impl IntoIterator<Item = (PhysicalAtomId, PhysicalAtomPayload)>,
    ) -> Result<Self, RealizationError> {
        let mut entries = atoms
            .into_iter()
            .map(|(id, payload)| {
                let codec = payload.codec();
                (id, Arc::new(PhysicalAtom { codec, payload }))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(id, _)| *id);
        for pair in entries.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(RealizationError::DuplicatePhysicalAtom(pair[0].0));
            }
        }
        let next_id = entries.last().map_or(Ok(0), |(id, _)| {
            id.raw()
                .checked_add(1)
                .ok_or(RealizationError::PhysicalAtomIdExhausted)
        })?;
        let atoms = PersistentOrdMap::from_sorted_unique_owned(entries)
            .expect("sorted unique physical atom entries");
        Ok(Self { atoms, next_id })
    }

    pub fn merge_exact_from(&mut self, other: &Self) -> Result<(), RealizationError> {
        for (id, atom) in &other.atoms {
            match self.atoms.get(id) {
                Some(existing) if existing.as_ref() != atom.as_ref() => {
                    return Err(RealizationError::ConflictingPhysicalAtom(*id));
                }
                Some(_) => {}
                None => {
                    self.atoms.insert(*id, Arc::clone(atom));
                    self.next_id = self.next_id.max(
                        id.raw()
                            .checked_add(1)
                            .ok_or(RealizationError::PhysicalAtomIdExhausted)?,
                    );
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    pub fn retain(&mut self, reachable: &BTreeSet<PhysicalAtomId>) {
        self.atoms.retain(|id, _| reachable.contains(id));
    }

    #[must_use]
    pub fn shares_storage_root_with(&self, other: &Self) -> bool {
        self.atoms.shares_root_with(&other.atoms)
    }

    #[must_use]
    pub fn shares_atom_allocation_with(&self, other: &Self, id: PhysicalAtomId) -> bool {
        match (self.atoms.get(&id), other.atoms.get(&id)) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    #[must_use]
    pub fn unique_storage_probe_against(&self, other: &Self) -> PhysicalAtomStoreProbe {
        PhysicalAtomStoreProbe {
            map_nodes: self.atoms.unique_storage_probe_against(&other.atoms),
            atoms: self
                .atoms
                .iter()
                .filter_map(|(id, atom)| match other.atoms.get(id) {
                    Some(other_atom) if Arc::ptr_eq(atom, other_atom) => None,
                    _ => Some(Arc::downgrade(atom)),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRealizationExpr {
    Direct(PhysicalAtomId),
    Constant(Value),
    Product(BTreeMap<SemanticId, Self>),
    Transform {
        source: Box<Self>,
        query: ExactQuery,
    },
}

impl ValueRealizationExpr {
    pub fn evaluate(&self, atoms: &PhysicalAtomStore) -> Result<Value, RealizationError> {
        match self {
            Self::Direct(atom) => match atoms
                .get(*atom)
                .ok_or(RealizationError::MissingAtom(*atom))?
                .payload()
            {
                PhysicalAtomPayload::Value(value) => Ok(value.clone()),
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::Value,
                    actual: payload.codec(),
                }),
            },
            Self::Constant(value) => Ok(value.clone()),
            Self::Product(fields) => fields
                .iter()
                .map(|(&field, expr)| expr.evaluate(atoms).map(|value| (field, value)))
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map(Value::Product),
            Self::Transform { source, query } => query
                .evaluate(&source.evaluate(atoms)?)
                .map_err(RealizationError::Query),
        }
    }

    fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        match self {
            Self::Direct(atom) => {
                into.insert(*atom);
            }
            Self::Constant(_) => {}
            Self::Product(fields) => {
                for expr in fields.values() {
                    expr.collect_dependencies(into);
                }
            }
            Self::Transform { source, .. } => source.collect_dependencies(into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowColumnRealizationExpr {
    source_columns: Vec<(SemanticId, usize)>,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationRealizationExpr {
    Direct(PhysicalAtomId),
    RowTransform {
        source: Box<Self>,
        columns: Vec<RowColumnRealizationExpr>,
    },
}

impl RelationRealizationExpr {
    pub fn evaluate(&self, atoms: &PhysicalAtomStore) -> Result<Vec<Vec<Value>>, RealizationError> {
        match self {
            Self::Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::RelationRows(rows) => Ok(rows.clone()),
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::RelationRows,
                    actual: payload.codec(),
                }),
            },
            Self::RowTransform { source, columns } => {
                let source_rows = source.evaluate(atoms)?;
                let mut target_rows = Vec::with_capacity(source_rows.len());
                for source_row in source_rows {
                    let mut target_row = Vec::with_capacity(columns.len());
                    for column in columns {
                        let mut input = BTreeMap::new();
                        for (source_column_id, source_ordinal) in &column.source_columns {
                            let value = source_row.get(*source_ordinal).ok_or(
                                RealizationError::MissingRelationColumn {
                                    column: *source_column_id,
                                    ordinal: *source_ordinal,
                                },
                            )?;
                            input.insert(*source_column_id, value.clone());
                        }
                        target_row.push(
                            column
                                .transform
                                .evaluate(&Value::Product(input))
                                .map_err(RealizationError::Query)?,
                        );
                    }
                    target_rows.push(target_row);
                }
                Ok(target_rows)
            }
        }
    }

    pub(crate) fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        match self {
            Self::Direct(atom) => {
                into.insert(*atom);
            }
            Self::RowTransform { source, .. } => source.collect_dependencies(into),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RealizationCoordinate {
    Lifecycle,
    Carrier(SemanticId),
    Field { field: SemanticId, entity: EntityId },
    Relation(SemanticId),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RealizationDependencyGraph {
    by_coordinate: BTreeMap<RealizationCoordinate, BTreeSet<PhysicalAtomId>>,
}

impl RealizationDependencyGraph {
    #[must_use]
    pub fn dependencies(
        &self,
        coordinate: RealizationCoordinate,
    ) -> Option<&BTreeSet<PhysicalAtomId>> {
        self.by_coordinate.get(&coordinate)
    }

    #[must_use]
    pub fn reachable_atoms(&self) -> BTreeSet<PhysicalAtomId> {
        self.by_coordinate
            .values()
            .flat_map(|dependencies| dependencies.iter().copied())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizationRoot {
    lifecycle: PhysicalAtomId,
    carriers: BTreeMap<SemanticId, PhysicalAtomId>,
    fields: BTreeMap<(SemanticId, EntityId), ValueRealizationExpr>,
    relations: BTreeMap<SemanticId, RelationRealizationExpr>,
}

impl RealizationRoot {
    #[must_use]
    pub const fn lifecycle_atom(&self) -> PhysicalAtomId {
        self.lifecycle
    }

    #[must_use]
    pub const fn fields(&self) -> &BTreeMap<(SemanticId, EntityId), ValueRealizationExpr> {
        &self.fields
    }

    pub fn set_field_expr(
        &mut self,
        field: SemanticId,
        entity: EntityId,
        expr: ValueRealizationExpr,
    ) {
        self.fields.insert((field, entity), expr);
    }

    pub fn evaluate(&self, atoms: &PhysicalAtomStore) -> Result<DatabaseState, RealizationError> {
        let lifecycle = match atom_payload(atoms, self.lifecycle)? {
            PhysicalAtomPayload::Lifecycle(graph) => graph.clone(),
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: self.lifecycle,
                    expected: PhysicalCodec::Lifecycle,
                    actual: payload.codec(),
                });
            }
        };

        let mut state = DatabaseState::default();
        state.lifecycle = lifecycle.into();

        for (&semantic, &atom) in &self.carriers {
            match atom_payload(atoms, atom)? {
                PhysicalAtomPayload::EntitySet(entities) => {
                    state.model.carriers.insert(semantic, entities.clone());
                }
                payload => {
                    return Err(RealizationError::CodecMismatch {
                        atom,
                        expected: PhysicalCodec::EntitySet,
                        actual: payload.codec(),
                    });
                }
            }
        }
        for (&coordinate, expr) in &self.fields {
            state.model.fields.insert(coordinate, expr.evaluate(atoms)?);
        }
        for (&semantic, expr) in &self.relations {
            state
                .model
                .relations
                .insert(semantic, expr.evaluate(atoms)?);
        }
        Ok(state)
    }

    #[must_use]
    pub fn dependency_graph(&self) -> RealizationDependencyGraph {
        let mut by_coordinate = BTreeMap::new();
        by_coordinate.insert(
            RealizationCoordinate::Lifecycle,
            BTreeSet::from([self.lifecycle]),
        );
        for (&semantic, &atom) in &self.carriers {
            by_coordinate.insert(
                RealizationCoordinate::Carrier(semantic),
                BTreeSet::from([atom]),
            );
        }
        for (&(field, entity), expr) in &self.fields {
            let mut dependencies = BTreeSet::new();
            expr.collect_dependencies(&mut dependencies);
            by_coordinate.insert(RealizationCoordinate::Field { field, entity }, dependencies);
        }
        for (&semantic, expr) in &self.relations {
            let mut dependencies = BTreeSet::new();
            expr.collect_dependencies(&mut dependencies);
            by_coordinate.insert(RealizationCoordinate::Relation(semantic), dependencies);
        }
        RealizationDependencyGraph { by_coordinate }
    }

    #[must_use]
    pub fn dependencies(&self) -> BTreeSet<PhysicalAtomId> {
        self.dependency_graph().reachable_atoms()
    }

    pub fn materialize_field(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        field: SemanticId,
        entity: EntityId,
    ) -> Result<PhysicalAtomId, RealizationError> {
        let expr = self
            .fields
            .get(&(field, entity))
            .ok_or(RealizationError::MissingField { field, entity })?;
        let value = expr.evaluate(atoms)?;
        let atom = atoms.insert(PhysicalAtomPayload::Value(value));
        self.fields
            .insert((field, entity), ValueRealizationExpr::Direct(atom));
        Ok(atom)
    }

    pub fn materialize_relation(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
    ) -> Result<PhysicalAtomId, RealizationError> {
        let expr = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingMigrationRelation(relation))?;
        let rows = expr.evaluate(atoms)?;
        let atom = atoms.insert(PhysicalAtomPayload::RelationRows(rows));
        self.relations
            .insert(relation, RelationRealizationExpr::Direct(atom));
        Ok(atom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizationCertificate {
    dependencies: BTreeSet<PhysicalAtomId>,
}

impl RealizationCertificate {
    #[must_use]
    pub const fn dependencies(&self) -> &BTreeSet<PhysicalAtomId> {
        &self.dependencies
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealizationEquivalenceCertificate {
    before_dependencies: BTreeSet<PhysicalAtomId>,
    after_dependencies: BTreeSet<PhysicalAtomId>,
}

impl RealizationEquivalenceCertificate {
    #[must_use]
    pub const fn before_dependencies(&self) -> &BTreeSet<PhysicalAtomId> {
        &self.before_dependencies
    }

    #[must_use]
    pub const fn after_dependencies(&self) -> &BTreeSet<PhysicalAtomId> {
        &self.after_dependencies
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealizationError {
    DuplicatePhysicalAtom(PhysicalAtomId),
    ConflictingPhysicalAtom(PhysicalAtomId),
    PhysicalAtomIdExhausted,
    NonDirectDurableRealization,
    MissingAtom(PhysicalAtomId),
    CodecMismatch {
        atom: PhysicalAtomId,
        expected: PhysicalCodec,
        actual: PhysicalCodec,
    },
    MissingField {
        field: SemanticId,
        entity: EntityId,
    },
    MissingFactorizedField(SemanticId),
    MissingFactorizedCarrier(SemanticId),
    MissingFactorizedCarrierEntity {
        carrier: PhysicalAtomId,
        entity: EntityId,
    },
    MissingFactorizedFieldValue {
        atom: PhysicalAtomId,
        entity: EntityId,
    },
    MissingFactorizedRelation(SemanticId),
    MissingFactorizedRelationColumn {
        relation: SemanticId,
        column: SemanticId,
    },
    MissingFactorizedRelationRow {
        atom: PhysicalAtomId,
        row: usize,
    },
    FactorizedRelationRowOutOfBounds {
        row: usize,
        row_count: usize,
    },
    InvalidRealizationChunkSize,
    RealizationChunkShapeMismatch,
    FactorizedRelationColumnArityMismatch(SemanticId),
    DuplicateFieldColumnEntity(EntityId),
    UnsortedFieldColumnEntities,
    Query(QueryError),
    Migration(TransportError),
    MissingMigrationField {
        field: SemanticId,
        entity: EntityId,
    },
    MissingMigrationRelation(SemanticId),
    MissingRelationColumn {
        column: SemanticId,
        ordinal: usize,
    },
    RelationQuery(kernel_query::RelQueryError),
    GeneralRelationRewriteRequiresPreparedRealization(SemanticId),
    PreparedRelationMismatch(SemanticId),
    RelationDeltaOverlayUnavailable(SemanticId),
    CompactionCostOverflow,
    OneShotRelationArityMismatch,
    SemanticMismatch,
}

pub fn realize_database_state(state: &DatabaseState) -> (PhysicalAtomStore, RealizationRoot) {
    let mut atoms = PhysicalAtomStore::default();
    let lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle((*state.lifecycle).clone()));

    let carriers = state
        .model
        .carriers
        .iter()
        .map(|(&semantic, entities)| {
            let atom = atoms.insert(PhysicalAtomPayload::EntitySet(entities.clone()));
            (semantic, atom)
        })
        .collect();

    let fields = state
        .model
        .fields
        .iter()
        .map(|(&coordinate, value)| {
            let atom = atoms.insert(PhysicalAtomPayload::Value(value.clone()));
            (coordinate, ValueRealizationExpr::Direct(atom))
        })
        .collect();

    let relations = state
        .model
        .relations
        .iter()
        .map(|(&semantic, rows)| {
            let atom = atoms.insert(PhysicalAtomPayload::RelationRows(rows.to_vec()));
            (semantic, RelationRealizationExpr::Direct(atom))
        })
        .collect();

    (
        atoms,
        RealizationRoot {
            lifecycle,
            carriers,
            fields,
            relations,
        },
    )
}

pub fn compose_schema_migration(
    atoms: &PhysicalAtomStore,
    source_root: &RealizationRoot,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    program: &SchemaMigrationProgram,
) -> Result<RealizationRoot, RealizationError> {
    let verified = program
        .verify(source_context, registry)
        .map_err(RealizationError::Migration)?;
    let target_context = verified.target();

    let mut fields = BTreeMap::new();
    let rewrites = program
        .field_rewrites()
        .iter()
        .map(|rewrite| (rewrite.target_field, rewrite))
        .collect::<BTreeMap<_, _>>();

    for target_field in target_context.schema.fields() {
        if source_context.schema.field(target_field.id) == Some(target_field) {
            for (&(field, entity), expr) in &source_root.fields {
                if field == target_field.id {
                    fields.insert((field, entity), expr.clone());
                }
            }
            continue;
        }

        let rewrite = rewrites
            .get(&target_field.id)
            .ok_or(RealizationError::Migration(
                TransportError::UnknownTargetField(target_field.id),
            ))?;
        let carrier_atom = source_root
            .carriers
            .get(&target_field.owner)
            .copied()
            .ok_or(RealizationError::Migration(
                TransportError::MigrationFieldOwnerMismatch(target_field.id),
            ))?;
        let entities = match atom_payload(atoms, carrier_atom)? {
            PhysicalAtomPayload::EntitySet(entities) => entities,
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: carrier_atom,
                    expected: PhysicalCodec::EntitySet,
                    actual: payload.codec(),
                });
            }
        };
        for &entity in entities {
            let product = rewrite
                .source_fields
                .iter()
                .map(|&source_field| {
                    source_root
                        .fields
                        .get(&(source_field, entity))
                        .cloned()
                        .map(|expr| (source_field, expr))
                        .ok_or(RealizationError::MissingMigrationField {
                            field: source_field,
                            entity,
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            fields.insert(
                (target_field.id, entity),
                ValueRealizationExpr::Transform {
                    source: Box::new(ValueRealizationExpr::Product(product)),
                    query: rewrite.transform.clone(),
                },
            );
        }
    }

    let relation_rewrites = program
        .relation_rewrites()
        .iter()
        .map(|rewrite| {
            let target = match rewrite {
                MigrationRelationRewrite::Query(rewrite) => rewrite.target_relation,
                MigrationRelationRewrite::Rows(rewrite) => rewrite.target_relation,
            };
            (target, rewrite)
        })
        .collect::<BTreeMap<_, _>>();
    let mut relations = BTreeMap::new();
    for target_relation in target_context.schema.relations() {
        if source_context.schema.relation(target_relation.id) == Some(target_relation) {
            let expr = source_root
                .relations
                .get(&target_relation.id)
                .cloned()
                .ok_or(RealizationError::MissingMigrationRelation(
                    target_relation.id,
                ))?;
            relations.insert(target_relation.id, expr);
            continue;
        }
        let rewrite = relation_rewrites.get(&target_relation.id).ok_or(
            RealizationError::MissingMigrationRelation(target_relation.id),
        )?;
        match rewrite {
            MigrationRelationRewrite::Query(_) => {
                return Err(
                    RealizationError::GeneralRelationRewriteRequiresPreparedRealization(
                        target_relation.id,
                    ),
                );
            }
            MigrationRelationRewrite::Rows(rewrite) => {
                let source = source_root
                    .relations
                    .get(&rewrite.source_relation)
                    .cloned()
                    .ok_or(RealizationError::MissingMigrationRelation(
                        rewrite.source_relation,
                    ))?;
                let target_column_ids = target_context
                    .schema
                    .relation_column_ids(target_relation.id)
                    .ok_or(RealizationError::MissingMigrationRelation(
                        target_relation.id,
                    ))?;
                let by_target = rewrite
                    .columns
                    .iter()
                    .map(|column| (column.target_column, column))
                    .collect::<BTreeMap<_, _>>();
                let mut columns = Vec::with_capacity(target_column_ids.len());
                for target_column_id in target_column_ids {
                    let column = by_target.get(target_column_id).ok_or(
                        RealizationError::MissingMigrationRelation(target_relation.id),
                    )?;
                    let source_columns = column
                        .source_columns
                        .iter()
                        .map(|&source_column_id| {
                            source_context
                                .schema
                                .relation_column_ordinal(rewrite.source_relation, source_column_id)
                                .map(|ordinal| (source_column_id, ordinal))
                                .ok_or(RealizationError::MissingRelationColumn {
                                    column: source_column_id,
                                    ordinal: usize::MAX,
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    columns.push(RowColumnRealizationExpr {
                        source_columns,
                        transform: column.transform.clone(),
                    });
                }
                relations.insert(
                    target_relation.id,
                    RelationRealizationExpr::RowTransform {
                        source: Box::new(source),
                        columns,
                    },
                );
            }
        }
    }

    Ok(RealizationRoot {
        lifecycle: source_root.lifecycle,
        carriers: source_root.carriers.clone(),
        fields,
        relations,
    })
}

pub fn certify_realization(
    atoms: &PhysicalAtomStore,
    root: &RealizationRoot,
    expected: &DatabaseState,
) -> Result<RealizationCertificate, RealizationError> {
    if root.evaluate(atoms)? != *expected {
        return Err(RealizationError::SemanticMismatch);
    }
    Ok(RealizationCertificate {
        dependencies: root.dependencies(),
    })
}

pub fn certify_equivalent_realizations(
    before_atoms: &PhysicalAtomStore,
    before: &RealizationRoot,
    after_atoms: &PhysicalAtomStore,
    after: &RealizationRoot,
) -> Result<RealizationEquivalenceCertificate, RealizationError> {
    if before.evaluate(before_atoms)? != after.evaluate(after_atoms)? {
        return Err(RealizationError::SemanticMismatch);
    }
    Ok(RealizationEquivalenceCertificate {
        before_dependencies: before.dependencies(),
        after_dependencies: after.dependencies(),
    })
}

#[must_use]
pub fn reachable_atoms<'a>(
    roots: impl IntoIterator<Item = &'a RealizationRoot>,
) -> BTreeSet<PhysicalAtomId> {
    roots
        .into_iter()
        .flat_map(RealizationRoot::dependencies)
        .collect()
}

fn atom_payload(
    atoms: &PhysicalAtomStore,
    atom: PhysicalAtomId,
) -> Result<&PhysicalAtomPayload, RealizationError> {
    atoms
        .get(atom)
        .map(PhysicalAtom::payload)
        .ok_or(RealizationError::MissingAtom(atom))
}

#[cfg(test)]
mod tests;
