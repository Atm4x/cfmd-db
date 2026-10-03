use std::collections::{BTreeMap, BTreeSet};

use crate::{NativeRelation, PhysicalExecutionError};

/// Bounded-memory physical reader for one verified row-local migration slice.
///
/// The cursor is deliberately layout-independent on the source side: row-store,
/// columnar and typed-columnar representations are decoded one row at a time,
/// then transformed through the verified kernel migration program. It owns no
/// migration progress; a durability layer can derive progress from whichever
/// target-native physical coordinates it has actually published.
pub struct RowLocalMigrationCursor<'a> {
    migration: &'a kernel_transport::SchemaMigrationTransport,
    target_relation: kernel_types::SemanticId,
    source: &'a NativeRelation,
    next_row: usize,
    row_count: usize,
}

impl<'a> RowLocalMigrationCursor<'a> {
    pub fn new(
        migration: &'a kernel_transport::SchemaMigrationTransport,
        target_relation: kernel_types::SemanticId,
        source: &'a NativeRelation,
    ) -> Result<Self, PhysicalExecutionError> {
        match migration.relation_slice(target_relation) {
            Some(kernel_transport::MigrationRelationSlice::RowLocal { .. }) => {}
            Some(_) => {
                return Err(PhysicalExecutionError::MigrationTransport(
                    kernel_transport::TransportError::MigrationSliceNotRowLocal(target_relation),
                ));
            }
            None => {
                return Err(PhysicalExecutionError::MigrationTransport(
                    kernel_transport::TransportError::UnknownTargetRelation(target_relation),
                ));
            }
        }
        Ok(Self {
            migration,
            target_relation,
            source,
            next_row: 0,
            row_count: crate::native_relation::native_row_count(source),
        })
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.row_count - self.next_row
    }
}

impl Iterator for RowLocalMigrationCursor<'_> {
    type Item = Result<kernel_query::Row, PhysicalExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.row_count {
            return None;
        }
        let row_index = self.next_row;
        self.next_row += 1;
        Some(
            crate::native_relation::materialize_native_row(self.source, row_index).and_then(
                |row| {
                    self.migration
                        .transform_row_local_slice(self.target_relation, &row)
                        .map_err(PhysicalExecutionError::MigrationTransport)
                },
            ),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining();
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for RowLocalMigrationCursor<'_> {}

struct NativeRelationCursor<'a> {
    relation: &'a NativeRelation,
    next_row: usize,
    row_count: usize,
}

impl<'a> NativeRelationCursor<'a> {
    fn new(relation: &'a NativeRelation) -> Self {
        Self {
            relation,
            next_row: 0,
            row_count: crate::native_relation::native_row_count(relation),
        }
    }
}

impl Iterator for NativeRelationCursor<'_> {
    type Item = Result<kernel_query::Row, PhysicalExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.row_count {
            return None;
        }
        let row_index = self.next_row;
        self.next_row += 1;
        Some(crate::native_relation::materialize_native_row(
            self.relation,
            row_index,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.row_count - self.next_row;
        (remaining, Some(remaining))
    }
}

/// Physical coordinate of one target relation in a mixed migration view.
///
/// This is derived, never persisted as a second progress journal. `NativeTarget`
/// means target-epoch bytes/values are authoritative for the relation.
/// `ForwardFromSource` means the relation is still represented by the exact
/// source-epoch slice named by the verified migration program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedMigrationRelationCoordinate {
    NativeTarget { relation: kernel_types::SemanticId },
    ForwardFromSource(kernel_transport::MigrationRelationSlice),
}

/// Cutover-time certificate that a validated target Revision is exactly the
/// deterministic image of one validated source Revision under this migration.
///
/// Global invariants are discharged once at semantic cutover by the ordinary
/// `Revision` constructor. Mixed physical publication is subsequently allowed
/// only when every installed source/target region is Γ-canonically equal to
/// the certified relation base for its epoch; changing representation therefore
/// cannot change the already validated logical world.
#[derive(Debug, Clone)]
pub struct MixedMigrationCutoverCertificate {
    head: kernel_revision::RevisionSemanticHead,
    source_revision: kernel_types::RevisionId,
    migration: kernel_transport::SchemaMigrationTransport,
    source_bases: BTreeMap<kernel_types::SemanticId, crate::RelationBaseWitness>,
    target_bases: BTreeMap<kernel_types::SemanticId, crate::RelationBaseWitness>,
}

impl MixedMigrationCutoverCertificate {
    pub fn certify(
        source: &kernel_revision::Revision,
        target: &kernel_revision::Revision,
        migration: &kernel_transport::SchemaMigrationTransport,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        if source.semantic_context() != migration.source()
            || target.semantic_context() != migration.target()
        {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        let realized = migration
            .transport_revision(source, target.id(), registry)
            .map_err(PhysicalExecutionError::MigrationTransport)?;
        if &realized != target {
            return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
        }
        let source_bases = Self::relation_bases(source, registry)?;
        let target_bases = Self::relation_bases(target, registry)?;
        Ok(Self {
            head: target.semantic_head(),
            source_revision: source.id(),
            migration: migration.clone(),
            source_bases,
            target_bases,
        })
    }

    fn relation_bases(
        revision: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<
        BTreeMap<kernel_types::SemanticId, crate::RelationBaseWitness>,
        PhysicalExecutionError,
    > {
        let mut bases = BTreeMap::new();
        for relation in revision.semantic_context().schema.relations() {
            let rows = revision
                .state()
                .model
                .relations
                .materialize_owned(&relation.id)
                .unwrap_or_default();
            let result_type = kernel_query::RelExpr::Scan(relation.id)
                .typecheck(revision.semantic_context(), registry)?;
            bases.insert(
                relation.id,
                crate::RelationBaseWitness::build(
                    revision.id(),
                    relation.id,
                    &rows,
                    result_type,
                    revision.semantic_context(),
                    registry,
                )?,
            );
        }
        Ok(bases)
    }

    fn require_native_relation(
        &self,
        relation: kernel_types::SemanticId,
        native: &NativeRelation,
        source_epoch: bool,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let (revision, context, expected) = if source_epoch {
            (
                self.source_revision,
                self.migration.source(),
                &self.source_bases,
            )
        } else {
            (self.head.id(), self.migration.target(), &self.target_bases)
        };
        let rows = NativeRelationCursor::new(native).collect::<Result<Vec<_>, _>>()?;
        let result_type = kernel_query::RelExpr::Scan(relation).typecheck(context, registry)?;
        let actual = crate::RelationBaseWitness::build(
            revision,
            relation,
            &rows,
            result_type,
            context,
            registry,
        )?;
        if expected.get(&relation) != Some(&actual) {
            return Err(PhysicalExecutionError::LogicalPhysicalStateMismatch(
                relation,
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn semantic_head(&self) -> &kernel_revision::RevisionSemanticHead {
        &self.head
    }

    #[must_use]
    pub const fn source_revision(&self) -> kernel_types::RevisionId {
        self.source_revision
    }
}

/// Physical authority for one migration frontier backed by the real
/// `PhysicalStore`, not caller-owned relation maps.
///
/// Source and target layouts may coexist in the same store. A target relation
/// is native iff it has a target layout binding here; otherwise its unique
/// realization is derived from the certified migration slice. These bindings
/// are the physical progress coordinate; no second progress bitmap exists.
pub struct MixedMigrationPhysicalAuthority<'a> {
    store: &'a crate::PhysicalStore,
    source_layouts: BTreeMap<kernel_types::SemanticId, crate::LayoutBinding>,
    native_target_layouts: BTreeMap<kernel_types::SemanticId, crate::LayoutBinding>,
}

impl<'a> MixedMigrationPhysicalAuthority<'a> {
    pub fn certify(
        store: &'a crate::PhysicalStore,
        source_layouts: BTreeMap<kernel_types::SemanticId, crate::LayoutBinding>,
        native_target_layouts: BTreeMap<kernel_types::SemanticId, crate::LayoutBinding>,
        certificate: &MixedMigrationCutoverCertificate,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let source_schema_relations = certificate
            .migration
            .source()
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = source_layouts
            .keys()
            .find(|relation| !source_schema_relations.contains(relation))
        {
            return Err(PhysicalExecutionError::MigrationTransport(
                kernel_transport::TransportError::UnknownSourceRelation(*unknown),
            ));
        }
        let target_schema_relations = certificate
            .migration
            .target()
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = native_target_layouts
            .keys()
            .find(|relation| !target_schema_relations.contains(relation))
        {
            return Err(PhysicalExecutionError::MigrationTransport(
                kernel_transport::TransportError::UnknownTargetRelation(*unknown),
            ));
        }
        let native_target_ids = native_target_layouts
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        for relation in certificate
            .migration
            .required_source_relations(&native_target_ids)
            .map_err(PhysicalExecutionError::MigrationTransport)?
        {
            let layout = source_layouts.get(&relation).copied().ok_or(
                PhysicalExecutionError::MissingMigrationSourceRelation(relation),
            )?;
            certificate.require_native_relation(
                relation,
                store.native_relation_at(relation, layout)?,
                true,
                registry,
            )?;
        }
        for (&relation, &layout) in &native_target_layouts {
            certificate.require_native_relation(
                relation,
                store.native_relation_at(relation, layout)?,
                false,
                registry,
            )?;
        }
        Ok(Self {
            store,
            source_layouts,
            native_target_layouts,
        })
    }

    fn source_relation(
        &self,
        relation: kernel_types::SemanticId,
    ) -> Result<&NativeRelation, PhysicalExecutionError> {
        let layout = self.source_layouts.get(&relation).copied().ok_or(
            PhysicalExecutionError::MissingMigrationSourceRelation(relation),
        )?;
        self.store.native_relation_at(relation, layout)
    }

    fn target_relation(
        &self,
        relation: kernel_types::SemanticId,
    ) -> Result<&NativeRelation, PhysicalExecutionError> {
        let layout = self.native_target_layouts.get(&relation).copied().ok_or(
            PhysicalExecutionError::MissingMigrationTargetRelation(relation),
        )?;
        self.store.native_relation_at(relation, layout)
    }

    #[must_use]
    pub fn native_target_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        self.native_target_layouts.keys().copied().collect()
    }
}

/// Read-only semantic target view over a physically mixed migration frontier.
///
/// The target semantic context is singular and authoritative. Physical
/// coordinates come only from `MixedMigrationPhysicalAuthority`; there is no
/// caller-owned map and no read-time fallback policy. The cutover certificate
/// proves that both source-backed and native-target regions are representations
/// of the same already validated target Revision.
pub struct MixedMigrationRevisionView<'a> {
    certificate: &'a MixedMigrationCutoverCertificate,
    registry: &'a kernel_semantics::SemanticRegistry,
    physical: &'a MixedMigrationPhysicalAuthority<'a>,
    native_target_ids: BTreeSet<kernel_types::SemanticId>,
}

impl<'a> MixedMigrationRevisionView<'a> {
    pub fn new(
        certificate: &'a MixedMigrationCutoverCertificate,
        registry: &'a kernel_semantics::SemanticRegistry,
        physical: &'a MixedMigrationPhysicalAuthority<'a>,
    ) -> Result<Self, PhysicalExecutionError> {
        let native_target_ids = physical.native_target_relations();
        Ok(Self {
            certificate,
            registry,
            physical,
            native_target_ids,
        })
    }

    #[must_use]
    pub const fn target_revision(&self) -> kernel_types::RevisionId {
        self.certificate.head.id()
    }

    #[must_use]
    pub const fn semantic_head(&self) -> &kernel_revision::RevisionSemanticHead {
        &self.certificate.head
    }

    #[must_use]
    pub fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        self.certificate.head.semantic_context()
    }

    #[must_use]
    pub fn native_target_relations(&self) -> &BTreeSet<kernel_types::SemanticId> {
        &self.native_target_ids
    }

    pub fn required_source_relations(
        &self,
    ) -> Result<BTreeSet<kernel_types::SemanticId>, PhysicalExecutionError> {
        self.certificate
            .migration
            .required_source_relations(&self.native_target_ids)
            .map_err(PhysicalExecutionError::MigrationTransport)
    }

    pub fn relation_coordinate(
        &self,
        target_relation: kernel_types::SemanticId,
    ) -> Result<MixedMigrationRelationCoordinate, PhysicalExecutionError> {
        if self.native_target_ids.contains(&target_relation) {
            return Ok(MixedMigrationRelationCoordinate::NativeTarget {
                relation: target_relation,
            });
        }
        self.certificate
            .migration
            .relation_slice(target_relation)
            .map(MixedMigrationRelationCoordinate::ForwardFromSource)
            .ok_or({
                PhysicalExecutionError::MigrationTransport(
                    kernel_transport::TransportError::UnknownTargetRelation(target_relation),
                )
            })
    }

    pub fn relation_rows(
        &'a self,
        target_relation: kernel_types::SemanticId,
    ) -> Result<MixedMigrationRelationCursor<'a>, PhysicalExecutionError> {
        match self.relation_coordinate(target_relation)? {
            MixedMigrationRelationCoordinate::NativeTarget { relation } => Ok(
                MixedMigrationRelationCursor::native(self.physical.target_relation(relation)?),
            ),
            MixedMigrationRelationCoordinate::ForwardFromSource(slice) => match slice {
                kernel_transport::MigrationRelationSlice::Passthrough { relation } => Ok(
                    MixedMigrationRelationCursor::native(self.physical.source_relation(relation)?),
                ),
                kernel_transport::MigrationRelationSlice::RowLocal {
                    source_relation,
                    target_relation,
                } => Ok(MixedMigrationRelationCursor::row_local(
                    RowLocalMigrationCursor::new(
                        &self.certificate.migration,
                        target_relation,
                        self.physical.source_relation(source_relation)?,
                    )?,
                )),
                kernel_transport::MigrationRelationSlice::Query {
                    target_relation,
                    source_relations,
                } => {
                    let mut source_state = kernel_model::DatabaseState::default();
                    for source_relation in source_relations {
                        let rows = NativeRelationCursor::new(
                            self.physical.source_relation(source_relation)?,
                        )
                        .collect::<Result<Vec<_>, _>>()?;
                        source_state.model.relations.insert(source_relation, rows);
                    }
                    let rows = self
                        .certificate
                        .migration
                        .materialize_relation_slice(&source_state, target_relation, self.registry)
                        .map_err(PhysicalExecutionError::MigrationTransport)?;
                    Ok(MixedMigrationRelationCursor::materialized(rows))
                }
            },
        }
    }
}

pub struct MixedMigrationRelationCursor<'a> {
    inner: MixedMigrationRelationCursorInner<'a>,
}

enum MixedMigrationRelationCursorInner<'a> {
    Native(NativeRelationCursor<'a>),
    RowLocal(RowLocalMigrationCursor<'a>),
    Materialized(std::vec::IntoIter<kernel_query::Row>),
}

impl<'a> MixedMigrationRelationCursor<'a> {
    fn native(relation: &'a NativeRelation) -> Self {
        Self {
            inner: MixedMigrationRelationCursorInner::Native(NativeRelationCursor::new(relation)),
        }
    }

    fn row_local(cursor: RowLocalMigrationCursor<'a>) -> Self {
        Self {
            inner: MixedMigrationRelationCursorInner::RowLocal(cursor),
        }
    }

    fn materialized(rows: Vec<kernel_query::Row>) -> Self {
        Self {
            inner: MixedMigrationRelationCursorInner::Materialized(rows.into_iter()),
        }
    }
}

impl Iterator for MixedMigrationRelationCursor<'_> {
    type Item = Result<kernel_query::Row, PhysicalExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            MixedMigrationRelationCursorInner::Native(cursor) => cursor.next(),
            MixedMigrationRelationCursorInner::RowLocal(cursor) => cursor.next(),
            MixedMigrationRelationCursorInner::Materialized(cursor) => cursor.next().map(Ok),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.inner {
            MixedMigrationRelationCursorInner::Native(cursor) => cursor.size_hint(),
            MixedMigrationRelationCursorInner::RowLocal(cursor) => cursor.size_hint(),
            MixedMigrationRelationCursorInner::Materialized(cursor) => cursor.size_hint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use kernel_query::ExactQuery;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_transport::{
        MigrationColumnRewrite, MigrationRelationRewrite, MigrationRowRewrite,
        SchemaMigrationTransport, migration_column_input_id,
    };
    use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    #[allow(
        clippy::similar_names,
        reason = "Names distinguish related before and after states."
    )]
    fn row_local_fixture() -> (
        SemanticRegistry,
        SchemaMigrationTransport,
        SemanticId,
        SemanticId,
    ) {
        let migrated = SemanticId::new(90_000);
        let passthrough = SemanticId::new(90_010);
        let eq_i64 = SemanticId::new(90_001);
        let eq_f64 = SemanticId::new(90_002);
        let mut registry = SemanticRegistry::default();
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);

        let mut source_schema = Schema::new(SchemaRevisionId::new(900));
        source_schema
            .define_relation(RelationDef {
                id: migrated,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_i64],
                },
            })
            .unwrap();
        source_schema
            .define_relation(RelationDef {
                id: passthrough,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_i64],
                },
            })
            .unwrap();
        let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(900));
        source_environment.pin_module(eq_i64, i64_digest);
        source_environment.pin_module(eq_f64, f64_digest);
        let source_column = source_schema.relation_column_ids(migrated).unwrap()[0];
        let source = SemanticContext {
            schema: source_schema,
            environment: source_environment,
        };

        let mut target_schema = Schema::new(SchemaRevisionId::new(901));
        target_schema
            .define_relation(RelationDef {
                id: migrated,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_f64],
                },
            })
            .unwrap();
        target_schema
            .define_relation(RelationDef {
                id: passthrough,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_i64],
                },
            })
            .unwrap();
        let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(900));
        target_environment.pin_module(eq_i64, i64_digest);
        target_environment.pin_module(eq_f64, f64_digest);
        let target_column = target_schema.relation_column_ids(migrated).unwrap()[0];
        let target = SemanticContext {
            schema: target_schema,
            environment: target_environment,
        };

        let input = migration_column_input_id(source_column);
        let migration = SchemaMigrationTransport::verify(
            &source,
            &target,
            &registry,
            vec![],
            vec![MigrationRelationRewrite::Rows(MigrationRowRewrite {
                source_relation: migrated,
                target_relation: migrated,
                columns: vec![MigrationColumnRewrite {
                    source_columns: vec![source_column],
                    target_column,
                    transform: ExactQuery::new(kernel_query::Expr::I64ToF64(Box::new(
                        kernel_query::Expr::ProductField {
                            input: Box::new(kernel_query::Expr::Input),
                            field: input,
                        },
                    ))),
                }],
            })],
        )
        .unwrap();
        (registry, migration, migrated, passthrough)
    }

    #[test]
    fn row_local_migration_cursor_streams_columnar_source_without_full_relation_materialization() {
        let (registry, migration, relation, _) = row_local_fixture();
        let _ = registry;
        let source_physical = NativeRelation::i64_columnar(vec![vec![3, 5, 8]]).unwrap();
        let cursor = RowLocalMigrationCursor::new(&migration, relation, &source_physical).unwrap();
        assert_eq!(cursor.remaining(), 3);
        assert_eq!(
            cursor.collect::<Result<Vec<_>, _>>().unwrap(),
            vec![
                vec![kernel_model::Value::F64Bits(3.0_f64.to_bits())],
                vec![kernel_model::Value::F64Bits(5.0_f64.to_bits())],
                vec![kernel_model::Value::F64Bits(8.0_f64.to_bits())],
            ]
        );
    }

    fn certified_world(
        registry: &SemanticRegistry,
        migration: &SchemaMigrationTransport,
        migrated: SemanticId,
        passthrough: SemanticId,
    ) -> (
        kernel_revision::Revision,
        kernel_revision::Revision,
        MixedMigrationCutoverCertificate,
    ) {
        let mut state = kernel_model::DatabaseState::default();
        state.model.relations.insert(
            migrated,
            vec![
                vec![kernel_model::Value::I64(3)],
                vec![kernel_model::Value::I64(5)],
                vec![kernel_model::Value::I64(8)],
            ],
        );
        state.model.relations.insert(
            passthrough,
            vec![
                vec![kernel_model::Value::I64(11)],
                vec![kernel_model::Value::I64(13)],
            ],
        );
        let source = kernel_revision::Revision::build(
            RevisionId::new(900),
            migration.source(),
            registry,
            state,
        )
        .unwrap();
        let target = migration
            .transport_revision(&source, RevisionId::new(901), registry)
            .unwrap();
        let certificate =
            MixedMigrationCutoverCertificate::certify(&source, &target, migration, registry)
                .unwrap();
        (source, target, certificate)
    }

    fn columnar_layout(id: u128) -> crate::LayoutBinding {
        crate::LayoutBinding {
            id: crate::LayoutId(id),
            family: crate::LayoutFamily::Columnar,
        }
    }

    #[test]
    fn mixed_revision_view_uses_real_physical_store_and_certified_coordinates() {
        let (registry, migration, migrated, passthrough) = row_local_fixture();
        let (_source, _target, certificate) =
            certified_world(&registry, &migration, migrated, passthrough);
        let source_migrated = columnar_layout(100);
        let target_passthrough = columnar_layout(200);
        let mut physical = crate::PhysicalStore::default();
        physical
            .install(
                migrated,
                source_migrated,
                NativeRelation::i64_columnar(vec![vec![3, 5, 8]]).unwrap(),
            )
            .unwrap();
        physical
            .install(
                passthrough,
                target_passthrough,
                NativeRelation::i64_columnar(vec![vec![11, 13]]).unwrap(),
            )
            .unwrap();
        let authority = MixedMigrationPhysicalAuthority::certify(
            &physical,
            BTreeMap::from([(migrated, source_migrated)]),
            BTreeMap::from([(passthrough, target_passthrough)]),
            &certificate,
            &registry,
        )
        .unwrap();
        let view = MixedMigrationRevisionView::new(&certificate, &registry, &authority).unwrap();

        assert_eq!(view.semantic_context(), migration.target());
        assert_eq!(
            view.relation_coordinate(migrated).unwrap(),
            MixedMigrationRelationCoordinate::ForwardFromSource(
                kernel_transport::MigrationRelationSlice::RowLocal {
                    source_relation: migrated,
                    target_relation: migrated,
                }
            )
        );
        assert_eq!(
            view.relation_coordinate(passthrough).unwrap(),
            MixedMigrationRelationCoordinate::NativeTarget {
                relation: passthrough
            }
        );
        assert_eq!(
            view.required_source_relations().unwrap(),
            BTreeSet::from([migrated])
        );
        assert_eq!(
            view.relation_rows(migrated)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![
                vec![kernel_model::Value::F64Bits(3.0_f64.to_bits())],
                vec![kernel_model::Value::F64Bits(5.0_f64.to_bits())],
                vec![kernel_model::Value::F64Bits(8.0_f64.to_bits())],
            ]
        );
    }

    #[test]
    fn cutover_certificate_rejects_target_not_equal_to_verified_program_image() {
        let (registry, migration, migrated, passthrough) = row_local_fixture();
        let (source, _, _) = certified_world(&registry, &migration, migrated, passthrough);
        let mut wrong_state = kernel_model::DatabaseState::default();
        wrong_state.model.relations.insert(
            migrated,
            vec![vec![kernel_model::Value::F64Bits(99.0_f64.to_bits())]],
        );
        wrong_state.model.relations.insert(
            passthrough,
            vec![
                vec![kernel_model::Value::I64(11)],
                vec![kernel_model::Value::I64(13)],
            ],
        );
        let wrong_target = kernel_revision::Revision::build(
            RevisionId::new(901),
            migration.target(),
            &registry,
            wrong_state,
        )
        .unwrap();
        let err = MixedMigrationCutoverCertificate::certify(
            &source,
            &wrong_target,
            &migration,
            &registry,
        )
        .expect_err("certificate must bind exact program image");
        assert_eq!(err, PhysicalExecutionError::LogicalRevisionMutationMismatch);
    }

    #[test]
    fn mixed_revision_view_rejects_physical_target_not_equal_to_certified_base() {
        let (registry, migration, migrated, passthrough) = row_local_fixture();
        let (_source, _target, certificate) =
            certified_world(&registry, &migration, migrated, passthrough);
        let source_migrated = columnar_layout(100);
        let target_passthrough = columnar_layout(200);
        let mut physical = crate::PhysicalStore::default();
        physical
            .install(
                migrated,
                source_migrated,
                NativeRelation::i64_columnar(vec![vec![3, 5, 8]]).unwrap(),
            )
            .unwrap();
        physical
            .install(
                passthrough,
                target_passthrough,
                NativeRelation::i64_columnar(vec![vec![999]]).unwrap(),
            )
            .unwrap();
        let err = MixedMigrationPhysicalAuthority::certify(
            &physical,
            BTreeMap::from([(migrated, source_migrated)]),
            BTreeMap::from([(passthrough, target_passthrough)]),
            &certificate,
            &registry,
        )
        .err()
        .expect("native target bytes must match cutover certificate");
        assert_eq!(
            err,
            PhysicalExecutionError::LogicalPhysicalStateMismatch(passthrough)
        );
    }

    #[test]
    fn source_retention_frontier_is_derived_from_real_native_target_layouts() {
        let (registry, migration, migrated, passthrough) = row_local_fixture();
        let (_source, _target, certificate) =
            certified_world(&registry, &migration, migrated, passthrough);
        let target_migrated = columnar_layout(300);
        let target_passthrough = columnar_layout(301);
        let mut physical = crate::PhysicalStore::default();
        physical
            .install(
                migrated,
                target_migrated,
                NativeRelation::typed_from_rows(
                    &[
                        vec![kernel_model::Value::F64Bits(3.0_f64.to_bits())],
                        vec![kernel_model::Value::F64Bits(5.0_f64.to_bits())],
                        vec![kernel_model::Value::F64Bits(8.0_f64.to_bits())],
                    ],
                    &[TypeExpr::Scalar(ScalarType::F64)],
                )
                .unwrap(),
            )
            .unwrap();
        physical
            .install(
                passthrough,
                target_passthrough,
                NativeRelation::i64_columnar(vec![vec![11, 13]]).unwrap(),
            )
            .unwrap();
        let authority = MixedMigrationPhysicalAuthority::certify(
            &physical,
            BTreeMap::new(),
            BTreeMap::from([
                (migrated, target_migrated),
                (passthrough, target_passthrough),
            ]),
            &certificate,
            &registry,
        )
        .unwrap();
        let view = MixedMigrationRevisionView::new(&certificate, &registry, &authority).unwrap();
        assert!(view.required_source_relations().unwrap().is_empty());
    }
}
