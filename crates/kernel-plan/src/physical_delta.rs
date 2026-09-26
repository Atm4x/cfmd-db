use super::PhysicalRowId;

// HOSTILE[P186][ACTIVE][CLEAN]: relation-delta maintenance is shared by storage indexes and
// multiway quotient maintenance; the change vocabulary is neutral and owns no storage state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhysicalRelationDelta {
    pub(crate) removed: Vec<(PhysicalRowId, kernel_query::Row)>,
    pub(crate) inserted: Vec<(PhysicalRowId, kernel_query::Row)>,
}
