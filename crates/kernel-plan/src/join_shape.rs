use super::{LayoutBinding, PhysicalExecutionError, PhysicalStore, Plan, SemanticId};
use crate::join_access::JoinKeySpec;
use crate::native_relation::native_column_count;
use crate::plan::transparent_direct_scan_relation;

// HOSTILE[P182][ACTIVE][CLEAN]: direct-join shape recognition is neutral plan inspection.
// Execution consumes normalized left/right keys; storage/advisor sees only scan identities
// plus right-side semantic key parts.
pub(super) struct DirectJoinShape {
    pub(super) left_scan: (SemanticId, LayoutBinding),
    pub(super) right_scan: (SemanticId, LayoutBinding),
    pub(super) keys: Vec<JoinKeySpec>,
}

pub(super) type DirectJoinAdviceSummary = (
    (SemanticId, LayoutBinding),
    (SemanticId, LayoutBinding),
    Vec<(usize, SemanticId)>,
);

pub(super) fn direct_join_shape(
    plan: &Plan,
    store: &PhysicalStore,
) -> Result<Option<DirectJoinShape>, PhysicalExecutionError> {
    let mut cursor = plan;
    let mut extra_predicates = Vec::new();
    while let Plan::FilterEqColumns {
        input,
        left_column,
        right_column,
        equivalence,
    } = cursor
    {
        extra_predicates.push((*left_column, *right_column, *equivalence));
        cursor = input;
    }
    let Plan::JoinEq {
        left,
        right,
        left_column,
        right_column,
        equivalence,
    } = cursor
    else {
        return Ok(None);
    };
    let Some(left_scan) = transparent_direct_scan_relation(left, store) else {
        return Ok(None);
    };
    let Some(right_scan) = transparent_direct_scan_relation(right, store) else {
        return Ok(None);
    };

    let mut keys = Vec::with_capacity(extra_predicates.len().saturating_add(1));
    keys.push(JoinKeySpec {
        left_column: *left_column,
        right_column: *right_column,
        equivalence: *equivalence,
    });
    if !extra_predicates.is_empty() {
        let left_installed = store.installed(left_scan.0, left_scan.1)?;
        let right_installed = store.installed(right_scan.0, right_scan.1)?;
        let left_width = native_column_count(&left_installed.data);
        let right_width = native_column_count(&right_installed.data);
        let Some(total_width) = left_width.checked_add(right_width) else {
            return Ok(None);
        };
        for (first_column, second_column, equivalence) in extra_predicates {
            if first_column >= total_width || second_column >= total_width {
                return Ok(None);
            }
            let key = match (first_column < left_width, second_column < left_width) {
                (true, false) => JoinKeySpec {
                    left_column: first_column,
                    right_column: second_column - left_width,
                    equivalence,
                },
                (false, true) => JoinKeySpec {
                    left_column: second_column,
                    right_column: first_column - left_width,
                    equivalence,
                },
                (true, true) | (false, false) => return Ok(None),
            };
            if key.right_column >= right_width {
                return Ok(None);
            }
            keys.push(key);
        }
    }

    Ok(Some(DirectJoinShape {
        left_scan,
        right_scan,
        keys,
    }))
}

pub(super) fn direct_join_advice_summary(
    plan: &Plan,
    store: &PhysicalStore,
) -> Result<Option<DirectJoinAdviceSummary>, PhysicalExecutionError> {
    let Some(shape) = direct_join_shape(plan, store)? else {
        return Ok(None);
    };
    Ok(Some((
        shape.left_scan,
        shape.right_scan,
        shape
            .keys
            .into_iter()
            .map(|key| (key.right_column, key.equivalence))
            .collect(),
    )))
}
