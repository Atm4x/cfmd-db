use super::{PhysicalExecutionError, RelQueryError, SemanticId, Value};

// HOSTILE[P178][ACTIVE][CLEAN]: Γ-bound row identity/equality is shared semantic capability,
// not execution-engine representation.
pub(super) fn canonical_semantic_row_key(
    row: &[Value],
    equivalences: &[SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
    if row.len() != equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch.into());
    }
    row.iter()
        .zip(equivalences)
        .map(|(value, equivalence)| {
            registry
                .canonical_equivalence_key(context, *equivalence, value)
                .map_err(PhysicalExecutionError::from)
        })
        .collect()
}

pub(super) fn semantic_rows_equal(
    left: &[Value],
    right: &[Value],
    equivalences: &[SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, PhysicalExecutionError> {
    if left.len() != right.len() || left.len() != equivalences.len() {
        return Ok(false);
    }
    for ((left, right), equivalence) in left.iter().zip(right).zip(equivalences) {
        if !registry.equivalent(context, *equivalence, left, right)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn relation_value_from_rows(
    mut rows: Vec<kernel_query::Row>,
    result_type: &super::RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<super::RelationValue, PhysicalExecutionError> {
    match &result_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => Ok(super::RelationValue::Bag(rows)),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => {
            let mut seen =
                std::collections::BTreeSet::<Vec<kernel_semantics::CanonicalEqKey>>::new();
            let mut unique = Vec::with_capacity(rows.len());
            for row in rows.drain(..) {
                let key = canonical_semantic_row_key(&row, column_equivalences, context, registry)?;
                if seen.insert(key) {
                    unique.push(row);
                }
            }
            Ok(super::RelationValue::Set {
                rows: unique,
                column_equivalences: column_equivalences.clone(),
            })
        }
    }
}

pub(super) fn relation_value_matches_revision_relation(
    value: &super::RelationValue,
    revision: &kernel_revision::Revision,
    relation: SemanticId,
) -> bool {
    let Some(expected_rows) = revision.state().model.relations.get(&relation) else {
        return false;
    };
    let Some(definition) = revision.semantic_context().schema.relation(relation) else {
        return false;
    };
    match (&definition.semantics, value) {
        (kernel_schema::RelationSemantics::Bag { .. }, super::RelationValue::Bag(rows)) => {
            rows.as_slice() == expected_rows.as_slice()
        }
        (
            kernel_schema::RelationSemantics::Set {
                column_equivalences: expected_equivalences,
            },
            super::RelationValue::Set {
                rows,
                column_equivalences,
            },
        ) => {
            rows.as_slice() == expected_rows.as_slice()
                && column_equivalences == expected_equivalences
        }
        _ => false,
    }
}
