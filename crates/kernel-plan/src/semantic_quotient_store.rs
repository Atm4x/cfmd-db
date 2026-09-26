use crate::{
    LayoutBinding, PhysicalExecutionError, PhysicalRowId, SemanticId, SemanticIndexBinding, Value,
};

// HOSTILE[P190][ACTIVE][CLEAN]: semantic-quotient construction/maintenance consumes a narrow
// read capability instead of PhysicalStore representation. Storage implements this view; the QCN
// domain can therefore be moved independently without acquiring storage internals.
pub(super) trait SemanticQuotientStoreView {
    fn semantic_quotient_logical_row_handles(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError>;

    fn semantic_quotient_row(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        row_id: PhysicalRowId,
    ) -> Result<kernel_query::Row, PhysicalExecutionError>;

    fn semantic_quotient_value(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        row_id: PhysicalRowId,
        column: usize,
    ) -> Result<Value, PhysicalExecutionError>;

    fn semantic_quotient_single_key(
        &self,
        binding: &SemanticIndexBinding,
        row_id: PhysicalRowId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError>;

    fn has_semantic_quotient_capability(
        &self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError>;
}
