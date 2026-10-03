use std::collections::BTreeSet;

use crate::{FieldId, Query, RelationId, Schema, Type, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationValueExpr {
    Field(FieldId),
    Column(usize),
    Constant {
        value: Value,
        ty: Type,
    },
    AddI64(Box<Self>, Box<Self>),
    I64ToF64(Box<Self>),
    SeqLength(Box<Self>),
    SeqSumI64(Box<Self>),
    If {
        condition: Box<Self>,
        when_true: Box<Self>,
        when_false: Box<Self>,
    },
}

impl MigrationValueExpr {
    pub(crate) fn source_fields(&self, output: &mut BTreeSet<FieldId>) {
        match self {
            Self::Field(field) => {
                output.insert(*field);
            }
            Self::Column(_) | Self::Constant { .. } => {}
            Self::AddI64(left, right) => {
                left.source_fields(output);
                right.source_fields(output);
            }
            Self::I64ToF64(source) | Self::SeqLength(source) | Self::SeqSumI64(source) => {
                source.source_fields(output);
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                condition.source_fields(output);
                when_true.source_fields(output);
                when_false.source_fields(output);
            }
        }
    }

    pub(crate) fn to_kernel(&self) -> kernel_query::Expr {
        self.to_kernel_with_columns(None)
    }

    pub(crate) fn to_kernel_for_relation(
        &self,
        column_ids: &[kernel_types::SemanticId],
    ) -> kernel_query::Expr {
        self.to_kernel_with_columns(Some(column_ids))
    }

    fn to_kernel_with_columns(
        &self,
        column_ids: Option<&[kernel_types::SemanticId]>,
    ) -> kernel_query::Expr {
        match self {
            Self::Field(field) => kernel_query::Expr::ProductField {
                input: Box::new(kernel_query::Expr::Input),
                field: (*field).into(),
            },
            Self::Column(column) => {
                let field = column_ids
                    .and_then(|ids| ids.get(*column).copied())
                    .unwrap_or_else(|| kernel_types::SemanticId::new((*column as u128) + 1));
                kernel_query::Expr::ProductField {
                    input: Box::new(kernel_query::Expr::Input),
                    field,
                }
            }
            Self::Constant { value, ty } => kernel_query::Expr::TypedConst {
                value: value.clone().into(),
                ty: crate::schema::type_to_kernel(ty),
            },
            Self::AddI64(left, right) => kernel_query::Expr::AddI64(
                Box::new(left.to_kernel_with_columns(column_ids)),
                Box::new(right.to_kernel_with_columns(column_ids)),
            ),
            Self::I64ToF64(source) => {
                kernel_query::Expr::I64ToF64(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::SeqLength(source) => {
                kernel_query::Expr::SeqLength(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::SeqSumI64(source) => {
                kernel_query::Expr::SeqSumI64(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => kernel_query::Expr::If {
                condition: Box::new(condition.to_kernel_with_columns(column_ids)),
                when_true: Box::new(when_true.to_kernel_with_columns(column_ids)),
                when_false: Box::new(when_false.to_kernel_with_columns(column_ids)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFieldRule {
    pub target: FieldId,
    pub value: MigrationValueExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationColumnRule {
    pub source_columns: Vec<usize>,
    pub target_column: usize,
    pub value: MigrationValueExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationRule {
    Query {
        target: RelationId,
        query: Query,
    },
    Rows {
        source: RelationId,
        target: RelationId,
        columns: Vec<MigrationColumnRule>,
    },
}

#[derive(Debug, Clone)]
pub struct MigrationModel {
    id: u128,
    target: Schema,
    fields: Vec<MigrationFieldRule>,
    relations: Vec<MigrationRelationRule>,
}

impl MigrationModel {
    #[must_use]
    pub fn new(id: u128, target: Schema) -> Self {
        Self {
            id,
            target,
            fields: Vec::new(),
            relations: Vec::new(),
        }
    }

    #[must_use]
    pub fn field(mut self, rule: MigrationFieldRule) -> Self {
        self.fields.push(rule);
        self
    }

    #[must_use]
    pub fn relation(mut self, rule: MigrationRelationRule) -> Self {
        self.relations.push(rule);
        self
    }

    pub(crate) const fn id(&self) -> u128 {
        self.id
    }
    pub(crate) fn target(&self) -> Schema {
        self.target.clone()
    }
    pub(crate) fn field_rules(&self) -> &[MigrationFieldRule] {
        &self.fields
    }
    pub(crate) fn relation_rules(&self) -> &[MigrationRelationRule] {
        &self.relations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationHistoryPolicy {
    /// Explicitly accepts that this migration cannot be inverted from local history.
    Forget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, Schema, Storage, TransactionId};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn runtime_publishes_kernel_verified_migration_model() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::create(&path, source).unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        let model = MigrationModel::new(380_381, target);
        let outcome = db
            .migrate(
                &model,
                TransactionId::new(380_001),
                MigrationHistoryPolicy::Forget,
            )
            .unwrap();
        assert!(matches!(outcome, crate::CommitOutcome::Committed { .. }));
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let history = db.history().unwrap();
        let migration = history.latest().expect("migration history event");
        assert_eq!(migration.kind(), crate::HistoryEffectKind::SchemaMigration);
        assert_eq!(
            migration.reversibility(),
            crate::HistoryReversibility::NonPlanTransition
        );
        let semantic_change = migration
            .semantic_change()
            .expect("semantic migration boundary");
        assert_eq!(semantic_change.source_schema(), 380);
        assert_eq!(semantic_change.target_schema(), 381);
        assert_eq!(semantic_change.migration_spec(), 380_381);
        assert_eq!(
            semantic_change.historical_authority(),
            crate::HistoryBoundaryAuthority::ExplicitlyForgotten
        );
        assert!(
            !semantic_change
                .historical_authority()
                .retains_history_authority()
        );
        drop(db);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap().schema_revision(), 381);
        drop(reopened);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn historical_at_crosses_migration_in_active_single_file_epoch() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-single-file-migration-history-{}-{}.cfmd",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::builder(&path).schema(source).create().unwrap();
        let source_revision = db.current_revision().unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        db.migrate(
            &MigrationModel::new(380_382, target),
            TransactionId::new(380_003),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let historical = db.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(db);

        let reopened = Database::open(&path).unwrap();
        let historical = reopened.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(reopened);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn historical_at_crosses_migration_through_source_epoch_anchor() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::builder(&path)
            .storage(Storage::Directory)
            .schema(source)
            .create()
            .unwrap();
        let source_revision = db.current_revision().unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        db.migrate(
            &MigrationModel::new(380_381, target),
            TransactionId::new(380_002),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let historical = db.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(db);

        let reopened = Database::builder(&path)
            .storage(Storage::Directory)
            .open()
            .unwrap();
        let historical = reopened.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(reopened);
        let _ = fs::remove_dir_all(&path);
    }
}
