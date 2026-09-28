use kernel_lens::{RelColumnOrigin, RelRewriteLiftError};
use kernel_model::Value;
use kernel_query::{RelType, RelationValue, Row};
use std::collections::BTreeMap;

/// Compiled owner-coordinate projection authority.
///
/// A chain of relational `Project` stages is compositionally reduced once to
/// the owner columns visible at the final view boundary. Runtime projection,
/// bijective inversion and lossy hidden-column reconstruction all consume this
/// same coordinate map rather than replaying the stage chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedProjectionPath {
    owner_arity: usize,
    visible_columns: Vec<usize>,
}

impl PreparedProjectionPath {
    /// Normalize a plain owner projection directly from the final origin map.
    ///
    /// `RelWritableViewPlan::output_origins` already records each final view
    /// column in owner coordinates.  Replaying the historical `Project` stage
    /// chain is therefore unnecessary once writability compilation has pinned
    /// this map.  This constructor makes that normalized coordinate authority
    /// explicit and rejects any non-owner/duplicate/out-of-range origin.
    pub(crate) fn from_output_origins(
        origins: &[RelColumnOrigin],
        owner_relation: kernel_types::SemanticId,
        owner_arity: usize,
    ) -> Result<Self, RelRewriteLiftError> {
        let mut seen = vec![false; owner_arity];
        let mut visible_columns = Vec::with_capacity(origins.len());
        for origin in origins {
            if origin.relation != owner_relation
                || origin.column >= owner_arity
                || seen[origin.column]
            {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            seen[origin.column] = true;
            visible_columns.push(origin.column);
        }
        Ok(Self {
            owner_arity,
            visible_columns,
        })
    }

    pub(crate) fn from_stage_columns(
        stages: &[Vec<usize>],
        owner_arity: usize,
    ) -> Result<Self, RelRewriteLiftError> {
        let mut visible_columns = (0..owner_arity).collect::<Vec<_>>();
        for columns in stages {
            visible_columns = compose_columns(&visible_columns, columns)?;
        }
        Ok(Self {
            owner_arity,
            visible_columns,
        })
    }

    pub(crate) fn visible_columns(&self) -> &[usize] {
        &self.visible_columns
    }

    pub(crate) fn hidden_columns(&self) -> Vec<usize> {
        let mut visible = vec![false; self.owner_arity];
        for &column in &self.visible_columns {
            visible[column] = true;
        }
        visible
            .into_iter()
            .enumerate()
            .filter_map(|(column, is_visible)| (!is_visible).then_some(column))
            .collect()
    }

    pub(crate) fn project_row(&self, row: &[Value]) -> Result<Row, RelRewriteLiftError> {
        self.visible_columns
            .iter()
            .map(|&column| {
                row.get(column)
                    .cloned()
                    .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
            })
            .collect()
    }

    pub(crate) fn is_bijective(&self) -> bool {
        if self.visible_columns.len() != self.owner_arity {
            return false;
        }
        let mut seen = vec![false; self.owner_arity];
        for &column in &self.visible_columns {
            if column >= self.owner_arity || seen[column] {
                return false;
            }
            seen[column] = true;
        }
        true
    }

    pub(crate) fn invert_bijective_row(
        &self,
        projected_row: &[Value],
    ) -> Result<Row, RelRewriteLiftError> {
        if !self.is_bijective() || projected_row.len() != self.visible_columns.len() {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        }
        let mut owner_row = vec![None; self.owner_arity];
        for (&owner_column, value) in self.visible_columns.iter().zip(projected_row) {
            owner_row[owner_column] = Some(value.clone());
        }
        owner_row
            .into_iter()
            .map(|value| value.ok_or(RelRewriteLiftError::CandidateGenerationUnsupported))
            .collect()
    }

    pub(crate) fn invert_bijective_value(
        &self,
        value: &RelationValue,
        owner_type: &RelType,
    ) -> Result<RelationValue, RelRewriteLiftError> {
        let rows = value
            .rows()
            .iter()
            .map(|row| self.invert_bijective_row(row))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match &owner_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        })
    }

    pub(crate) fn inflate_with_hidden(
        &self,
        projected_row: &[Value],
        hidden_values: &BTreeMap<usize, Value>,
    ) -> Result<Row, RelRewriteLiftError> {
        if projected_row.len() != self.visible_columns.len() {
            return Err(RelRewriteLiftError::RequestedViewInadmissible);
        }
        let mut visible = vec![false; self.owner_arity];
        let mut owner_row = vec![None; self.owner_arity];
        for (&owner_column, value) in self.visible_columns.iter().zip(projected_row) {
            visible[owner_column] = true;
            owner_row[owner_column] = Some(value.clone());
        }
        for &column in hidden_values.keys() {
            if column >= self.owner_arity {
                return Err(RelRewriteLiftError::ProjectConstructorColumnOutOfBounds {
                    column,
                    arity: self.owner_arity,
                });
            }
            if visible[column] {
                return Err(
                    RelRewriteLiftError::ProjectConstructorOverridesVisibleColumn { column },
                );
            }
        }
        for (column, slot) in owner_row.iter_mut().enumerate() {
            if slot.is_some() {
                continue;
            }
            let Some(value) = hidden_values.get(&column) else {
                return Err(RelRewriteLiftError::ProjectConstructorMissingHiddenColumn { column });
            };
            *slot = Some(value.clone());
        }
        owner_row
            .into_iter()
            .map(|value| value.ok_or(RelRewriteLiftError::CandidateGenerationUnsupported))
            .collect()
    }
}

fn compose_columns(
    current_owner_columns: &[usize],
    next_columns: &[usize],
) -> Result<Vec<usize>, RelRewriteLiftError> {
    next_columns
        .iter()
        .map(|&column| {
            current_owner_columns
                .get(column)
                .copied()
                .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_types::{RevisionObservableId, SemanticId};

    #[test]
    fn final_origins_are_already_normalized_owner_coordinates() {
        let owner = SemanticId::new(10);
        let path = PreparedProjectionPath::from_output_origins(
            &[
                RelColumnOrigin {
                    relation: owner,
                    column: 2,
                    observable: RevisionObservableId::new(102),
                },
                RelColumnOrigin {
                    relation: owner,
                    column: 0,
                    observable: RevisionObservableId::new(100),
                },
            ],
            owner,
            4,
        )
        .unwrap();

        assert_eq!(path.visible_columns(), &[2, 0]);
        assert_eq!(path.hidden_columns(), vec![1, 3]);
    }

    #[test]
    fn origin_normalization_rejects_foreign_or_duplicate_owner_coordinates() {
        let owner = SemanticId::new(10);
        let foreign = SemanticId::new(11);
        let duplicate = [
            RelColumnOrigin {
                relation: owner,
                column: 1,
                observable: RevisionObservableId::new(101),
            },
            RelColumnOrigin {
                relation: owner,
                column: 1,
                observable: RevisionObservableId::new(201),
            },
        ];
        assert_eq!(
            PreparedProjectionPath::from_output_origins(&duplicate, owner, 3),
            Err(RelRewriteLiftError::CandidateGenerationUnsupported)
        );
        assert_eq!(
            PreparedProjectionPath::from_output_origins(
                &[RelColumnOrigin {
                    relation: foreign,
                    column: 0,
                    observable: RevisionObservableId::new(300),
                }],
                owner,
                3,
            ),
            Err(RelRewriteLiftError::CandidateGenerationUnsupported)
        );
    }

    #[test]
    fn nested_projection_compiles_to_direct_owner_coordinates() {
        let path =
            PreparedProjectionPath::from_stage_columns(&[vec![4, 1, 3, 0], vec![2, 0]], 5).unwrap();

        assert_eq!(path.visible_columns(), &[3, 4]);
        assert_eq!(
            path.project_row(&[
                Value::I64(10),
                Value::I64(11),
                Value::I64(12),
                Value::I64(13),
                Value::I64(14),
            ]),
            Ok(vec![Value::I64(13), Value::I64(14)])
        );
        assert_eq!(path.hidden_columns(), vec![0, 1, 2]);
    }

    #[test]
    fn composed_permutation_inverts_without_replaying_stages() {
        let path =
            PreparedProjectionPath::from_stage_columns(&[vec![2, 0, 1], vec![2, 0, 1]], 3).unwrap();

        assert!(path.is_bijective());
        assert_eq!(path.visible_columns(), &[1, 2, 0]);
        assert_eq!(
            path.invert_bijective_row(&[Value::I64(8), Value::I64(9), Value::I64(7)]),
            Ok(vec![Value::I64(7), Value::I64(8), Value::I64(9)])
        );
    }

    #[test]
    fn hidden_inflation_uses_the_same_compiled_coordinate_authority() {
        let path = PreparedProjectionPath::from_stage_columns(&[vec![2, 0]], 4).unwrap();
        let hidden = BTreeMap::from([(1, Value::Text("h1".into())), (3, Value::Text("h3".into()))]);

        assert_eq!(
            path.inflate_with_hidden(
                &[Value::Text("v2".into()), Value::Text("v0".into())],
                &hidden,
            ),
            Ok(vec![
                Value::Text("v0".into()),
                Value::Text("h1".into()),
                Value::Text("v2".into()),
                Value::Text("h3".into()),
            ])
        );
    }
}
