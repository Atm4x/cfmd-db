use std::collections::BTreeMap;

use kernel_model::{DatabaseState, Value};
use kernel_schema::{RelationSemantics, SemanticContext};
use kernel_semantics::SemanticRegistry;
use kernel_types::{EntityId, SemanticId};
use kernel_violation::{ViolationMeasure, ViolationMeasureError};

use crate::{DenseTypeExtents, ValidationError};
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationUniquenessWitness {
    pub row_key: Vec<kernel_semantics::CanonicalEqKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DynamicViolationLocation {
    Field {
        field: SemanticId,
        entity: EntityId,
    },
    RelationCell {
        relation: SemanticId,
        row: usize,
        column: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DynamicViolationWitness {
    RelationUniqueness {
        relation: SemanticId,
        row_key: Vec<kernel_semantics::CanonicalEqKey>,
    },
    MissingLiveReference {
        location: DynamicViolationLocation,
        target_type: SemanticId,
        target: EntityId,
    },
    MissingCapabilityRequiredField {
        capability: SemanticId,
        field: SemanticId,
        entity: EntityId,
    },
    FieldRule {
        field: SemanticId,
        entity: EntityId,
        rule_index: usize,
    },
    RelationColumnRule {
        relation: SemanticId,
        row: usize,
        column: usize,
        rule_index: usize,
    },
    EntityRule {
        owner: SemanticId,
        entity: EntityId,
        rule_index: usize,
    },
}

fn add_missing_live_reference_violations(
    value: &Value,
    location: &DynamicViolationLocation,
    entity_types: &DenseTypeExtents,
    multiplier: u64,
    measure: &mut ViolationMeasure<DynamicViolationWitness>,
) -> Result<(), ValidationError> {
    match value {
        Value::LiveEntityRef { entity_type, id } => {
            if !entity_types.contains(*id, *entity_type) {
                measure.add(
                    DynamicViolationWitness::MissingLiveReference {
                        location: location.clone(),
                        target_type: *entity_type,
                        target: *id,
                    },
                    multiplier,
                )?;
            }
        }
        Value::Product(values) => {
            for value in values.values() {
                add_missing_live_reference_violations(
                    value,
                    location,
                    entity_types,
                    multiplier,
                    measure,
                )?;
            }
        }
        Value::Seq(values)
        | Value::Set {
            elements: values, ..
        } => {
            for value in values {
                add_missing_live_reference_violations(
                    value,
                    location,
                    entity_types,
                    multiplier,
                    measure,
                )?;
            }
        }
        Value::Variant { value, .. } => add_missing_live_reference_violations(
            value,
            location,
            entity_types,
            multiplier,
            measure,
        )?,
        Value::Option(value) => {
            if let Some(value) = value.as_deref() {
                add_missing_live_reference_violations(
                    value,
                    location,
                    entity_types,
                    multiplier,
                    measure,
                )?;
            }
        }
        Value::Bag { entries, .. } => {
            for (value, multiplicity) in entries {
                let nested = multiplier
                    .checked_mul(*multiplicity)
                    .ok_or(ViolationMeasureError::MultiplicityOverflow)?;
                add_missing_live_reference_violations(
                    value,
                    location,
                    entity_types,
                    nested,
                    measure,
                )?;
            }
        }
        Value::Map { entries, .. } => {
            for (key, value) in entries {
                add_missing_live_reference_violations(
                    key,
                    location,
                    entity_types,
                    multiplier,
                    measure,
                )?;
                add_missing_live_reference_violations(
                    value,
                    location,
                    entity_types,
                    multiplier,
                    measure,
                )?;
            }
        }
        Value::Unit
        | Value::Bool(_)
        | Value::I64(_)
        | Value::F64Bits(_)
        | Value::Text(_)
        | Value::HistoricalEntityId { .. } => {}
    }
    Ok(())
}

/// Exact finite non-negative measure for currently modeled dynamic invariants.
///
/// Structural type/schema errors remain ordinary validation errors. This
/// measure captures invariants whose failure has stable finite witnesses and
/// can therefore be maintained incrementally by Γ-DTC/VMF later without
/// changing the publication contract.
pub fn dynamic_violation_measure(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
) -> Result<ViolationMeasure<DynamicViolationWitness>, ValidationError> {
    let compiled_rules = crate::CompiledRulePlan::compile(context);
    let mut measure = ViolationMeasure::new();

    for relation in context.schema.relations() {
        let relation_measure = relation_dynamic_violation_measure_with_plan(
            context,
            registry,
            state,
            entity_types,
            relation.id,
            &compiled_rules,
        )?;
        for (witness, mass) in relation_measure.iter() {
            measure.add(witness.clone(), mass)?;
        }
    }

    for (&(field, entity), value) in &state.model.fields {
        add_missing_live_reference_violations(
            value,
            &DynamicViolationLocation::Field { field, entity },
            entity_types,
            1,
            &mut measure,
        )?;
        for (rule_index, rule) in compiled_rules.field_rules(field).iter().enumerate() {
            if !rule.matches(value).unwrap_or(false) {
                measure.add(
                    DynamicViolationWitness::FieldRule {
                        field,
                        entity,
                        rule_index,
                    },
                    1,
                )?;
            }
        }
    }
    for owner in compiled_rules.entity_rule_owners() {
        for entity in entity_types.entities(owner) {
            for (rule_index, rule) in compiled_rules.entity_rules(owner).iter().enumerate() {
                let matches = rule
                    .matches(&|coordinate| match coordinate {
                        kernel_schema::RuleValueExpr::Input => None,
                        kernel_schema::RuleValueExpr::Field(field) => {
                            state.model.fields.get(&(*field, entity))
                        }
                    })
                    .unwrap_or(false);
                if !matches {
                    measure.add(
                        DynamicViolationWitness::EntityRule {
                            owner,
                            entity,
                            rule_index,
                        },
                        1,
                    )?;
                }
            }
        }
    }

    for capability in context.schema.capabilities() {
        for &field_id in capability.required_fields.keys() {
            for entity in entity_types.entities(capability.id) {
                if !state.model.fields.contains_key(&(field_id, entity)) {
                    measure.add(
                        DynamicViolationWitness::MissingCapabilityRequiredField {
                            capability: capability.id,
                            field: field_id,
                            entity,
                        },
                        1,
                    )?;
                }
            }
        }
    }

    Ok(measure)
}

/// Exact finite violation measure whose witnesses are local to one relation.
///
/// This is the Γ-VMF derivative boundary for relation-data-only transitions:
/// when carriers, fields, lifecycle and Γ are unchanged, only relation-local
/// uniqueness and relation-cell live-reference witnesses can change.
pub fn relation_dynamic_violation_measure(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
    relation_id: SemanticId,
) -> Result<ViolationMeasure<DynamicViolationWitness>, ValidationError> {
    let compiled_rules = crate::CompiledRulePlan::compile(context);
    relation_dynamic_violation_measure_with_plan(
        context,
        registry,
        state,
        entity_types,
        relation_id,
        &compiled_rules,
    )
}

fn relation_dynamic_violation_measure_with_plan(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
    relation_id: SemanticId,
    compiled_rules: &crate::CompiledRulePlan,
) -> Result<ViolationMeasure<DynamicViolationWitness>, ValidationError> {
    let relation = context
        .schema
        .relation(relation_id)
        .ok_or(ValidationError::UnknownRelation(relation_id))?;
    let rows = state
        .model
        .relations
        .materialize_owned(&relation_id)
        .unwrap_or_default();
    let mut measure = ViolationMeasure::new();

    if let RelationSemantics::Set {
        column_equivalences,
    } = &relation.semantics
    {
        let uniqueness =
            relation_uniqueness_violation_measure(&rows, column_equivalences, context, registry)?;
        for (witness, mass) in uniqueness.iter() {
            measure.add(
                DynamicViolationWitness::RelationUniqueness {
                    relation: relation_id,
                    row_key: witness.row_key.clone(),
                },
                mass,
            )?;
        }
    }

    for (row_index, row) in rows.iter().enumerate() {
        for (column, value) in row.iter().enumerate() {
            add_missing_live_reference_violations(
                value,
                &DynamicViolationLocation::RelationCell {
                    relation: relation_id,
                    row: row_index,
                    column,
                },
                entity_types,
                1,
                &mut measure,
            )?;
            for (rule_index, rule) in compiled_rules
                .relation_column_rules(context, relation_id, column)
                .iter()
                .enumerate()
            {
                if !rule.matches(value).unwrap_or(false) {
                    measure.add(
                        DynamicViolationWitness::RelationColumnRule {
                            relation: relation_id,
                            row: row_index,
                            column,
                            rule_index,
                        },
                        1,
                    )?;
                }
            }
        }
    }

    Ok(measure)
}

/// Builds the exact non-negative uniqueness-violation measure for a Set relation.
///
/// Every semantic row class with mass `m > 1` contributes mass `m - 1`. Hence
/// the measure is zero iff relation rows are unique under the pinned Γ equality.
pub fn relation_uniqueness_violation_measure(
    rows: &[Vec<Value>],
    column_equivalences: &[SemanticId],
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<ViolationMeasure<RelationUniquenessWitness>, ValidationError> {
    let mut counts = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, u64>::new();
    for row in rows {
        if row.len() != column_equivalences.len() {
            return Err(ValidationError::TypeMismatch);
        }
        let key = row
            .iter()
            .zip(column_equivalences)
            .map(|(value, equivalence)| {
                registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(ValidationError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = counts.entry(key).or_insert(0);
        *count = count
            .checked_add(1)
            .ok_or(ViolationMeasureError::MultiplicityOverflow)?;
    }

    let mut measure = ViolationMeasure::new();
    for (row_key, count) in counts {
        if count > 1 {
            measure.add(RelationUniquenessWitness { row_key }, count - 1)?;
        }
    }
    Ok(measure)
}
