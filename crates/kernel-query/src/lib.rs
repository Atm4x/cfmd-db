use std::collections::{BTreeMap, BTreeSet};
mod access;
mod blocker;
mod composition;
mod delta_abi;
mod delta_kernels;
mod delta_materialization;
mod differential_api;
mod differential_program;
mod exact_measure;
mod execgraph;
mod group;
mod join;
mod linear_island;
mod maintained_delta_kernels;
mod maintained_plan;
mod prepared_rel;
mod projection;
mod quotient;
mod recursive_query;
mod rel_eval;
mod rel_model;
mod relation_oracles;
mod relation_state;
mod relational_operators;
mod scalar_query;
mod topk;
pub use access::RelReadFootprint;
pub use delta_abi::{
    AdaptiveDelta, BinaryDeltaKernel, CompactDelta, CompiledDeltaEdgeIdentity, DeltaSink,
    DeltaView, ExactDelta, ExactDeltaSink, ExactDeltaView, ExactWeighted, InlineDelta,
    PlannedDeltaEffect, RelationDeltaView, UnaryDeltaKernel, ValidatedTransitionFrame, Weighted,
};

// Crate-internal exact row carrier shared by maintained execution owners.
type MaintainedDelta = ExactDelta<Row>;
use delta_kernels::{
    rel_delta_distinct, rel_delta_filter, rel_delta_filter_columns, rel_delta_filter_order_const,
    rel_delta_project_bag, rel_delta_project_set, rel_delta_scan,
};
#[cfg(test)]
use delta_materialization::{
    exact_delta_from_legacy, exact_delta_to_legacy_checked, materialize_delta_view,
    relation_delta_materialization_count, reset_relation_delta_materialization_count,
};
use delta_materialization::{materialize_exact_delta_view, materialize_exact_quotient_delta_view};
pub use differential_api::{
    rel_delta_by_recompute, rel_delta_optimized, relation_deltas_semantically_equivalent,
};
#[cfg(test)]
use exact_measure::exact_integer_to_i64;
use exact_measure::{apply_exact_to_natural, exact_natural_difference};
pub use execgraph::{
    ExecutionInputSlot, NodeId, NodeInbox, PreparedRelGraph, UnifiedTransitionProgram,
    UnifiedTransitionScratch,
};
pub use linear_island::{
    BarrierKernelClass, CompiledDeltaProgram, LinearIslandNormalForm, LinearIslandPredicate,
};
#[cfg(test)]
use maintained_delta_kernels::project_bag_delta_view;
pub use maintained_plan::MaterializedRelPlanState;
use projection::{project_row, project_rows};
use relation_oracles::{
    canonical_row_multiset_counts, rows_as_multisets_equivalent, rows_semantically_equal,
    unmatched_semantic_rows,
};
pub use relation_oracles::{
    check_derivative_law, rel_derivative_by_recompute, rel_impact_by_recompute,
};
#[cfg(test)]
use relation_oracles::{
    relation_values_semantically_equivalent, rows_as_multisets_equivalent_by_matching,
    unmatched_semantic_rows_by_matching,
};
pub use relational_operators::{
    anti_join_relation_values, difference_relation_values, union_relation_values,
};
use relational_operators::{
    distinct_rows, distinct_rows_with_canonical_keys, group_relation_value, query_types_compatible,
    relation_column_equivalence, validate_query_equivalence, value_shape_matches_type,
};
pub use scalar_query::{
    ExactQuery, Expr, Impact, QueryError, QueryResult, QueryTypeError, derivative_by_recompute,
    derivative_seq_splice, impact_by_recompute,
};

pub use composition::MaterializedJoinGroupTopKState;

use blocker::{BlockerBuildSpec, MaintainedBlockerKind, MaterializedBlockerDeltaState};
pub use differential_program::{
    RelDifferentialClass, RelDifferentialProgram, RelDifferentialStateRequirement,
    RelObservationGuard, RelObservationKey,
};
pub use group::MaterializedGroupDeltaState;
pub use join::MaterializedJoinDeltaState;
use kernel_change::{
    Change, FineChange, FineChangeKind, PreparedRewrite, PreparedStructuralRewrite,
    RewriteActionLaw, RewriteEffect, RewriteFootprint, RewriteSpec, SemanticWriteCoordinate,
    StructuralRewriteEffect,
};
use kernel_model::Value;
pub use prepared_rel::PreparedRelExpr;
use quotient::CanonicalRowPositionIndex;
pub use quotient::{CanonicalRowKey, canonical_row_key};
pub use recursive_query::{
    CompactRecursiveBag, FixpointCall, PositiveRecursiveRowAtom, PositiveRecursiveRowRule,
};
use rel_eval::collect_rel_source_relations;
pub use rel_model::{
    AggregateSpec, OrderComparison, OrderDirection, RelExpr, RelQueryError, RelQueryResult,
    RelType, RelationValue, Row,
};
use rel_model::{relation_column_equivalences, relation_value_from_rows};
#[cfg(test)]
use relation_state::apply_relation_delta_to_value;
use relation_state::validate_exact_delta_view_rows;
pub use relation_state::{
    CertifiedCanonicalRowKey, MaterializedRelDeltaState, MaterializedSetSupportState,
    PreparedRelationRewrite, RelationBaseWitness, RelationDelta, RelationOccurrenceCertificate,
    RelationRowCanonicalizer, RelationScanOccurrenceSeed, RelationWitnessStorageProbe,
    RelationWitnessStorageStats, StorageResolvedRelationDelta,
};
pub use topk::MaterializedTopKDeltaState;

// Relational tests remain mechanically included until their owner boundaries are cleaned.
include!("parts/relational_tests.rs");
