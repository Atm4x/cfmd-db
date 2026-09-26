use super::execution::test_support::{
    execute_bound_typed_filter_for_test, execute_fused_filter_project_scan_for_test,
    join_batch_selection_is_ephemeral_for_test, merge_execution_stats_for_test,
    select_top_k_i64_positions_for_test, select_top_k_semantic_positions_for_test,
    typed_column_matches_bound_for_test,
};
use super::join_access::test_support::{
    JoinAccessKind, JoinAccessProbe, observe_right_join_access_for_test,
};
use super::multiway::test_support::{
    estimated_quotient_build_work_for_test, execute_multiway_join_candidate_for_test,
    execute_order_preserving_quotient_join_for_test, multiway_right_access_estimate_for_test,
    optimize_contiguous_multiway_join_for_test,
};
use super::native_relation::{
    materialize_native_row, native_row_count, push_native_row, remove_native_row,
};
use super::recovery::recover_runtime_bundle_with_policy_and_cores;
use super::storage_impl::UnifiedObservableAdvisorInputs;
use super::storage_impl::test_support::{
    I64IndexStateTestExt as _, PhysicalStoreTestExt as _, SemanticIndexStateTestExt as _,
    build_i64_index_state_for_test, native_semantic_column_work_units,
    semantic_index_estimated_retained_bytes,
};

// Mechanical Pass167 test decomposition: textual includes preserve one shared test namespace.
include!("test_parts/segment_01.rs");
include!("test_parts/segment_02.rs");
include!("test_parts/segment_03.rs");
include!("test_parts/segment_04.rs");
include!("test_parts/segment_05.rs");
include!("test_parts/segment_06.rs");
include!("test_parts/segment_07.rs");
