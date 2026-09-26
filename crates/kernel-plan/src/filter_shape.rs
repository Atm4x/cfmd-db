// HOSTILE[P184][ACTIVE][CLEAN]: direct FilterEqConst shape recognition is neutral planning
// capability shared by semantic-index advice and persisted-filter execution. Predicate order
// is preserved outer-to-inner because execution revalidation accounting depends on it.
use super::{LayoutBinding, Plan, SemanticId, Value};

pub(super) type DirectFilterConstraintRef<'a> = (usize, SemanticId, &'a Value);

pub(super) fn collect_direct_filter_chain(
    plan: &Plan,
) -> Option<(
    SemanticId,
    LayoutBinding,
    Vec<DirectFilterConstraintRef<'_>>,
)> {
    let mut predicates = Vec::new();
    let mut current = plan;
    loop {
        match current {
            Plan::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => {
                predicates.push((*column, *equivalence, value));
                current = input;
            }
            Plan::Scan { relation, layout } => return Some((*relation, *layout, predicates)),
            _ => return None,
        }
    }
}
