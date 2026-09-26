use super::{
    Arc, BTreeMap, BTreeSet, EqClassId, ExecutionStats, LayoutBinding, PhysicalExecutionError,
    PhysicalRowId, PhysicalStore, Plan, RelQueryError, RevisionObservableId,
    SemanticAccessCostModel, SemanticId, SemanticIndexBinding,
};
use crate::execution::{execute_join_plan, filter_rows_columns};
use crate::join_access::{
    JoinAccessDecision, JoinAccessFamily, RightJoinAccessRequest, right_scan_join_access_decision,
};
use crate::native_relation::{materialize_native_row, native_column_count, native_row_count};
#[cfg(test)]
use crate::semantic_quotient_physical::test_support::cyclic_prefix_fixture_for_test;
#[cfg(test)]
use crate::semantic_quotient_physical::{
    MaterializedSemanticQuotientSupportState, build_semantic_quotient_support_state,
};
use crate::semantic_quotient_physical::{
    PreparedSemanticQuotientBuildContext, SemanticQuotientConstraint, SemanticQuotientEndpoint,
    SemanticQuotientPhysicalLeaf, SemanticQuotientSupportBinding,
    build_semantic_quotient_constraints_for_prepared_specs, mask_intersect_in_place,
    masks_intersect, quotient_leaf, quotient_support_masks,
};
use crate::semantic_quotient_store::SemanticQuotientStoreView;
include!("multiway/frontend.rs");
include!("multiway/semantic_quotient.rs");
include!("multiway/anchor_pullback.rs");
include!("multiway/cyclic.rs");

// HOSTILE[P177][TEST-ONLY][CLEAN]: cross-owner integration tests use a purpose-built facade;
// multiway execution/costing representations remain owner-private.
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
