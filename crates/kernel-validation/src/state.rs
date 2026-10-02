use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::DenseEntityIds;
use kernel_model::{DatabaseState, FiniteModel, Value};
use kernel_schema::{RelationSemantics, ScalarType, SemanticContext, TypeExpr, TypeVar};
use kernel_semantics::{SemanticError, SemanticRegistry};
use kernel_types::SemanticId;

use crate::{DenseTypeExtents, ValidationError, relation_uniqueness_violation_measure};

pub fn validate_state(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
) -> Result<(), ValidationError> {
    let entities = state
        .model
        .carriers
        .values()
        .flat_map(|carrier| carrier.iter().copied())
        .collect::<BTreeSet<_>>();
    let ids = DenseEntityIds::compile(&entities)?;
    validate_state_with_ids(context, registry, state, &ids)
}

pub fn validate_state_with_ids(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    ids: &DenseEntityIds,
) -> Result<(), ValidationError> {
    let entity_types = DenseTypeExtents::compile_with_ids(&state.model, &context.schema, ids);
    validate_state_with_extents(context, registry, state, &entity_types)
}

pub fn validate_state_with_extents(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
) -> Result<(), ValidationError> {
    let compiled_rules = crate::CompiledRulePlan::compile(context);
    validate_capability_required_fields(context, registry, state, entity_types)?;

    for (&(field_id, owner), value) in &state.model.fields {
        let field = context
            .schema
            .field(field_id)
            .ok_or(ValidationError::UnknownField(field_id))?;
        if !entity_types.contains(owner, field.owner) {
            return Err(ValidationError::FieldOwnerMismatch {
                field: field_id,
                entity: owner,
            });
        }
        validate_value(
            value,
            &field.value,
            &state.model,
            context,
            registry,
            entity_types,
            &BTreeMap::new(),
        )?;
        for (rule_index, rule) in compiled_rules.field_rules(field_id).iter().enumerate() {
            if !rule.matches(value).map_err(|_| {
                ValidationError::FieldRuleTypeMismatch { field: field_id }
            })? {
                return Err(ValidationError::FieldRuleViolation {
                    field: field_id,
                    entity: owner,
                    rule_index,
                });
            }
        }
    }

    for owner in compiled_rules.entity_rule_owners() {
        for entity in entity_types.entities(owner) {
            for (rule_index, rule) in compiled_rules.entity_rules(owner).iter().enumerate() {
                let matches = rule.matches(&|coordinate| match coordinate {
                    kernel_schema::RuleValueExpr::Input => None,
                    kernel_schema::RuleValueExpr::Field(field) => state.model.fields.get(&(*field, entity)),
                }).unwrap_or(false);
                if !matches {
                    return Err(ValidationError::EntityRuleViolation { owner, entity, rule_index });
                }
            }
        }
    }

    for (&relation_id, tuples) in &state.model.relations {
        let relation = context
            .schema
            .relation(relation_id)
            .ok_or(ValidationError::UnknownRelation(relation_id))?;
        for (row_index, tuple) in tuples.iter().enumerate() {
            if tuple.len() != relation.columns.len() {
                return Err(ValidationError::RelationArityMismatch {
                    relation: relation_id,
                    expected: relation.columns.len(),
                    actual: tuple.len(),
                });
            }
            for (column, (value, expected)) in tuple.iter().zip(&relation.columns).enumerate() {
                validate_value(
                    value,
                    expected,
                    &state.model,
                    context,
                    registry,
                    entity_types,
                    &BTreeMap::new(),
                )?;
                for (rule_index, rule) in compiled_rules
                    .relation_column_rules(context, relation_id, column)
                    .iter()
                    .enumerate()
                {
                    if !rule.matches(value).map_err(|_|
                        ValidationError::FieldRuleTypeMismatch { field: relation_id },
                    )? {
                        return Err(ValidationError::RelationColumnRuleViolation {
                            relation: relation_id,
                            row: row_index,
                            column,
                            rule_index,
                        });
                    }
                }
            }
        }
        if let RelationSemantics::Set {
            column_equivalences,
        } = &relation.semantics
        {
            for (column, equivalence) in relation.columns.iter().zip(column_equivalences) {
                validate_equivalence_type(*equivalence, column, context, registry)?;
            }
            ensure_relation_rows_unique(&tuples.materialize_owned(), column_equivalences, context, registry)?;
        }
    }

    registry.validate_model(context, &state.model)?;
    Ok(())
}

fn validate_capability_required_fields(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
) -> Result<(), ValidationError> {
    for capability in context.schema.capabilities() {
        for (&field_id, required_type) in &capability.required_fields {
            let field = context.schema.field(field_id).ok_or(
                ValidationError::CapabilityRequiredFieldUndefined {
                    capability: capability.id,
                    field: field_id,
                },
            )?;
            if &field.value != required_type
                || !context.schema.is_subtype(capability.id, field.owner)
            {
                return Err(ValidationError::CapabilityRequiredFieldContractMismatch {
                    capability: capability.id,
                    field: field_id,
                });
            }

            for entity in entity_types.entities(capability.id) {
                let value = state.model.fields.get(&(field_id, entity)).ok_or(
                    ValidationError::MissingCapabilityRequiredField {
                        capability: capability.id,
                        field: field_id,
                        entity,
                    },
                )?;
                validate_value(
                    value,
                    required_type,
                    &state.model,
                    context,
                    registry,
                    entity_types,
                    &BTreeMap::new(),
                )?;
            }
        }
    }
    Ok(())
}

pub fn validate_relations_with_extents(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    state: &DatabaseState,
    entity_types: &DenseTypeExtents,
    relation_ids: &BTreeSet<SemanticId>,
) -> Result<(), ValidationError> {
    let compiled_rules = crate::CompiledRulePlan::compile(context);
    let mut semantic_subset = FiniteModel::default();
    for &relation_id in relation_ids {
        let relation = context
            .schema
            .relation(relation_id)
            .ok_or(ValidationError::UnknownRelation(relation_id))?;
        let tuples_owned = state
            .model
            .relations
            .materialize_owned(&relation_id)
            .unwrap_or_default();
        let tuples = tuples_owned.as_slice();
        for (row_index, tuple) in tuples.iter().enumerate() {
            if tuple.len() != relation.columns.len() {
                return Err(ValidationError::RelationArityMismatch {
                    relation: relation_id,
                    expected: relation.columns.len(),
                    actual: tuple.len(),
                });
            }
            for (column, (value, expected)) in tuple.iter().zip(&relation.columns).enumerate() {
                validate_value(
                    value,
                    expected,
                    &state.model,
                    context,
                    registry,
                    entity_types,
                    &BTreeMap::new(),
                )?;
                for (rule_index, rule) in compiled_rules
                    .relation_column_rules(context, relation_id, column)
                    .iter()
                    .enumerate()
                {
                    if !rule.matches(value).map_err(|_|
                        ValidationError::FieldRuleTypeMismatch { field: relation_id },
                    )? {
                        return Err(ValidationError::RelationColumnRuleViolation {
                            relation: relation_id,
                            row: row_index,
                            column,
                            rule_index,
                        });
                    }
                }
            }
        }
        if let RelationSemantics::Set {
            column_equivalences,
        } = &relation.semantics
        {
            for (column, equivalence) in relation.columns.iter().zip(column_equivalences) {
                validate_equivalence_type(*equivalence, column, context, registry)?;
            }
            ensure_relation_rows_unique(tuples, column_equivalences, context, registry)?;
        }
        if state.model.relations.get_shared(&relation_id).is_some() {
            semantic_subset.relations.insert(relation_id, tuples_owned);
        }
    }
    registry.validate_model(context, &semantic_subset)?;
    Ok(())
}

fn ensure_relation_rows_unique(
    rows: &[Vec<Value>],
    column_equivalences: &[SemanticId],
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<(), ValidationError> {
    if !relation_uniqueness_violation_measure(rows, column_equivalences, context, registry)?
        .is_zero()
    {
        return Err(ValidationError::Semantic(
            SemanticError::DuplicateRelationRow,
        ));
    }
    Ok(())
}

pub(crate) fn validate_value(
    value: &Value,
    expected: &TypeExpr,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    entity_types: &DenseTypeExtents,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<(), ValidationError> {
    match expected {
        TypeExpr::Scalar(scalar) => validate_scalar(value, scalar, &context.schema, entity_types),
        TypeExpr::Product(fields) => validate_product(
            value,
            fields,
            model,
            context,
            registry,
            entity_types,
            recursive,
        ),
        TypeExpr::Sum(variants) => {
            let Value::Variant { tag, value } = value else {
                return Err(ValidationError::TypeMismatch);
            };
            let variant = variants
                .get(tag)
                .ok_or(ValidationError::UnknownVariant(*tag))?;
            validate_value(
                value,
                variant,
                model,
                context,
                registry,
                entity_types,
                recursive,
            )
        }
        TypeExpr::Option(inner) => {
            let Value::Option(value) = value else {
                return Err(ValidationError::TypeMismatch);
            };
            if let Some(value) = value.as_deref() {
                validate_value(
                    value,
                    inner,
                    model,
                    context,
                    registry,
                    entity_types,
                    recursive,
                )?;
            }
            Ok(())
        }
        TypeExpr::Set { .. } | TypeExpr::Bag { .. } => validate_unary_collection(
            value,
            expected,
            model,
            context,
            registry,
            entity_types,
            recursive,
        ),
        TypeExpr::Seq(element) => {
            let Value::Seq(values) = value else {
                return Err(ValidationError::TypeMismatch);
            };
            for value in values {
                validate_value(
                    value,
                    element,
                    model,
                    context,
                    registry,
                    entity_types,
                    recursive,
                )?;
            }
            Ok(())
        }
        TypeExpr::Map { .. } => validate_map(
            value,
            expected,
            model,
            context,
            registry,
            entity_types,
            recursive,
        ),
        TypeExpr::Var(var) => {
            let ty = recursive
                .get(var)
                .ok_or(ValidationError::UnboundRecursiveVariable(*var))?;
            validate_value(value, ty, model, context, registry, entity_types, recursive)
        }
        TypeExpr::Mu { binder, body } => {
            let mut next = recursive.clone();
            next.insert(*binder, expected.clone());
            validate_value(value, body, model, context, registry, entity_types, &next)
        }
    }
}

fn validate_unary_collection(
    value: &Value,
    expected: &TypeExpr,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    entity_types: &DenseTypeExtents,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<(), ValidationError> {
    let (element, equivalence, values): (&TypeExpr, SemanticId, Vec<&Value>) =
        match (value, expected) {
            (
                Value::Set {
                    equivalence: actual,
                    elements,
                },
                TypeExpr::Set {
                    element,
                    equivalence,
                },
            ) => {
                ensure_equivalence(*equivalence, *actual)?;
                (element, *equivalence, elements.iter().collect())
            }
            (
                Value::Bag {
                    equivalence: actual,
                    entries,
                },
                TypeExpr::Bag {
                    element,
                    equivalence,
                },
            ) => {
                ensure_equivalence(*equivalence, *actual)?;
                (
                    element,
                    *equivalence,
                    entries.iter().map(|(value, _)| value).collect(),
                )
            }
            _ => return Err(ValidationError::TypeMismatch),
        };
    validate_equivalence_type(equivalence, element, context, registry)?;
    for value in values {
        validate_value(
            value,
            element,
            model,
            context,
            registry,
            entity_types,
            recursive,
        )?;
    }
    Ok(())
}

fn validate_map(
    value: &Value,
    expected: &TypeExpr,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    entity_types: &DenseTypeExtents,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<(), ValidationError> {
    let (
        Value::Map {
            key_equivalence: actual,
            entries,
        },
        TypeExpr::Map {
            key,
            value: mapped,
            key_equivalence,
        },
    ) = (value, expected)
    else {
        return Err(ValidationError::TypeMismatch);
    };
    ensure_equivalence(*key_equivalence, *actual)?;
    validate_equivalence_type(*key_equivalence, key, context, registry)?;
    for (entry_key, entry_value) in entries {
        validate_value(
            entry_key,
            key,
            model,
            context,
            registry,
            entity_types,
            recursive,
        )?;
        validate_value(
            entry_value,
            mapped,
            model,
            context,
            registry,
            entity_types,
            recursive,
        )?;
    }
    Ok(())
}

fn validate_product(
    value: &Value,
    fields: &BTreeMap<SemanticId, TypeExpr>,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    entity_types: &DenseTypeExtents,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<(), ValidationError> {
    let Value::Product(values) = value else {
        return Err(ValidationError::TypeMismatch);
    };
    if values.keys().copied().collect::<BTreeSet<_>>()
        != fields.keys().copied().collect::<BTreeSet<_>>()
    {
        return Err(ValidationError::ProductShapeMismatch);
    }
    for (field, ty) in fields {
        validate_value(
            &values[field],
            ty,
            model,
            context,
            registry,
            entity_types,
            recursive,
        )?;
    }
    Ok(())
}

fn validate_scalar(
    value: &Value,
    expected: &ScalarType,
    schema: &kernel_schema::Schema,
    entity_types: &DenseTypeExtents,
) -> Result<(), ValidationError> {
    let valid = match (value, expected) {
        (Value::Unit, ScalarType::Unit)
        | (Value::Bool(_), ScalarType::Bool)
        | (Value::I64(_), ScalarType::I64)
        | (Value::F64Bits(_), ScalarType::F64)
        | (Value::Text(_), ScalarType::Text) => true,
        (Value::HistoricalEntityId { entity_type, .. }, ScalarType::HistoricalEntityId(target)) => {
            entity_type == target || schema.is_subtype(*entity_type, *target)
        }
        (Value::LiveEntityRef { entity_type, id }, ScalarType::LiveEntityRef(target)) => {
            (entity_type == target || schema.is_subtype(*entity_type, *target))
                && entity_types.contains(*id, *target)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ValidationError::TypeMismatch)
    }
}

fn validate_equivalence_type(
    equivalence: SemanticId,
    ty: &TypeExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<(), ValidationError> {
    let expected = kernel_semantics::domain_for_type(ty).ok_or(ValidationError::TypeMismatch)?;
    let actual = registry.equivalence_domain(context, equivalence)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ValidationError::Semantic(
            SemanticError::EquivalenceDomainMismatch {
                equivalence,
                expected,
                actual,
            },
        ))
    }
}

fn ensure_equivalence(expected: SemanticId, actual: SemanticId) -> Result<(), ValidationError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ValidationError::EquivalenceMismatch { expected, actual })
    }
}
