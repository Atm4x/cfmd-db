use super::{
    AggregateSpec, BTreeMap, BTreeSet, ExecutionStats, I64IndexBinding, InstalledRelation,
    LayoutBinding, NativeColumn, NativeRelation, OrderDirection, PersistentPhysicalVec,
    PhysicalExecutionError, PhysicalStore, Plan, RelQueryError, SemanticAccessCostModel,
    SemanticAccessPath, SemanticId, SemanticIndexBinding, Value, algebraic_native,
};
use crate::filter_shape::collect_direct_filter_chain;
use crate::join_access::{
    JoinAccessFamily, JoinKeySpec, RightJoinAccessRequest, direct_join_access_decision,
    right_scan_join_access_decision,
};
use crate::join_shape::direct_join_shape;
use crate::native_relation::{
    NativeI64ColumnView, append_i64_row_values, append_native_row_values, materialize_native_row,
    native_all_i64_columns, native_column_count, native_i64_column, native_row_count,
    validate_i64_columnar_schema, validate_indexable_i64_relation, validate_typed_columnar_schema,
};
use crate::plan::transparent_direct_scan_relation;
use crate::semantic_key::ResolvedSemanticIndexKeyPart;
use crate::semantic_rows::{canonical_semantic_row_key, relation_value_from_rows};
use crate::storage_impl::{I64IndexCapability, SemanticFiberCapability, SemanticFiberProbeScratch};
include!("execution/typed.rs");
include!("execution/filter.rs");
include!("execution/fallback.rs");
include!("execution/join.rs");
include!("execution/row_ops.rs");
include!("execution/helpers.rs");

#[cfg(test)]
pub(crate) mod test_support;
