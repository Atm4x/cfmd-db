use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::{DenseEntityIds, DenseEntitySet, DenseIdentityError};
use kernel_model::{DatabaseState, FiniteModel, Value};
use kernel_schema::{RelationSemantics, ScalarType, SemanticContext, TypeExpr, TypeVar};
use kernel_semantics::{SemanticError, SemanticRegistry};
use kernel_types::{EntityId, SemanticId};
use kernel_violation::{ViolationMeasure, ViolationMeasureError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnknownField(SemanticId),
    UnknownRelation(SemanticId),
    FieldOwnerMismatch {
        field: SemanticId,
        entity: EntityId,
    },
    CapabilityRequiredFieldUndefined {
        capability: SemanticId,
        field: SemanticId,
    },
    CapabilityRequiredFieldContractMismatch {
        capability: SemanticId,
        field: SemanticId,
    },
    MissingCapabilityRequiredField {
        capability: SemanticId,
        field: SemanticId,
        entity: EntityId,
    },
    RelationArityMismatch {
        relation: SemanticId,
        expected: usize,
        actual: usize,
    },
    TypeMismatch,
    ProductShapeMismatch,
    UnknownVariant(SemanticId),
    EquivalenceMismatch {
        expected: SemanticId,
        actual: SemanticId,
    },
    UnboundRecursiveVariable(TypeVar),
    Semantic(SemanticError),
    DenseIdentity(DenseIdentityError),
    ViolationMeasure(ViolationMeasureError),
}

impl From<SemanticError> for ValidationError {
    fn from(value: SemanticError) -> Self {
        Self::Semantic(value)
    }
}

impl From<DenseIdentityError> for ValidationError {
    fn from(value: DenseIdentityError) -> Self {
        Self::DenseIdentity(value)
    }
}

impl From<ViolationMeasureError> for ValidationError {
    fn from(value: ViolationMeasureError) -> Self {
        Self::ViolationMeasure(value)
    }
}

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
    let mut measure = ViolationMeasure::new();

    for relation in context.schema.relations() {
        let relation_measure = relation_dynamic_violation_measure(
            context,
            registry,
            state,
            entity_types,
            relation.id,
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
    }
    for capability in context.schema.capabilities() {
        for &field_id in capability.required_fields.keys() {
            for (&actual_type, entities) in &state.model.carriers {
                if !context.schema.is_subtype(actual_type, capability.id) {
                    continue;
                }
                for &entity in entities {
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
    let relation = context
        .schema
        .relation(relation_id)
        .ok_or(ValidationError::UnknownRelation(relation_id))?;
    let rows = state
        .model
        .relations
        .get(&relation_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut measure = ViolationMeasure::new();

    if let RelationSemantics::Set {
        column_equivalences,
    } = &relation.semantics
    {
        let uniqueness =
            relation_uniqueness_violation_measure(rows, column_equivalences, context, registry)?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenseTypeExtents {
    ids: DenseEntityIds,
    extents: BTreeMap<SemanticId, DenseEntitySet>,
}

impl DenseTypeExtents {
    pub fn compile(
        model: &FiniteModel,
        schema: &kernel_schema::Schema,
    ) -> Result<Self, DenseIdentityError> {
        let entities = model
            .carriers
            .values()
            .flat_map(|carrier| carrier.iter().copied())
            .collect::<BTreeSet<_>>();
        let ids = DenseEntityIds::compile(&entities)?;
        Ok(Self::compile_with_ids(model, schema, &ids))
    }

    #[must_use]
    pub fn compile_with_ids(
        model: &FiniteModel,
        schema: &kernel_schema::Schema,
        ids: &DenseEntityIds,
    ) -> Self {
        let mut targets = model.carriers.keys().copied().collect::<BTreeSet<_>>();
        targets.extend(schema.type_definitions().map(|(id, _)| id));
        targets.extend(schema.capabilities().map(|capability| capability.id));
        for (subtype, supertype) in schema.inclusions() {
            targets.insert(subtype);
            targets.insert(supertype);
        }

        let mut extents = targets
            .iter()
            .copied()
            .map(|target| (target, DenseEntitySet::with_capacity(ids.len())))
            .collect::<BTreeMap<_, _>>();

        for (&actual, carrier) in &model.carriers {
            let matching_targets = targets
                .iter()
                .copied()
                .filter(|&target| schema.is_subtype(actual, target))
                .collect::<Vec<_>>();
            for &entity in carrier {
                let Some(local) = ids.local(entity) else {
                    continue;
                };
                for target in &matching_targets {
                    extents
                        .get_mut(target)
                        .expect("target extent was initialized")
                        .insert(local);
                }
            }
        }

        Self {
            ids: ids.clone(),
            extents,
        }
    }

    #[must_use]
    pub fn contains(&self, entity: EntityId, expected: SemanticId) -> bool {
        self.ids.local(entity).is_some_and(|local| {
            self.extents
                .get(&expected)
                .is_some_and(|extent| extent.contains(local))
        })
    }

    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.ids.len()
    }
}

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
    }

    for (&relation_id, tuples) in &state.model.relations {
        let relation = context
            .schema
            .relation(relation_id)
            .ok_or(ValidationError::UnknownRelation(relation_id))?;
        for tuple in tuples {
            if tuple.len() != relation.columns.len() {
                return Err(ValidationError::RelationArityMismatch {
                    relation: relation_id,
                    expected: relation.columns.len(),
                    actual: tuple.len(),
                });
            }
            for (value, expected) in tuple.iter().zip(&relation.columns) {
                validate_value(
                    value,
                    expected,
                    &state.model,
                    context,
                    registry,
                    entity_types,
                    &BTreeMap::new(),
                )?;
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
            if &field.value != required_type {
                return Err(ValidationError::CapabilityRequiredFieldContractMismatch {
                    capability: capability.id,
                    field: field_id,
                });
            }

            for (&actual_type, entities) in &state.model.carriers {
                if !context.schema.is_subtype(actual_type, capability.id) {
                    continue;
                }
                if !context.schema.is_subtype(actual_type, field.owner) {
                    return Err(ValidationError::CapabilityRequiredFieldContractMismatch {
                        capability: capability.id,
                        field: field_id,
                    });
                }
                for &entity in entities {
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
    let mut semantic_subset = FiniteModel::default();
    for &relation_id in relation_ids {
        let relation = context
            .schema
            .relation(relation_id)
            .ok_or(ValidationError::UnknownRelation(relation_id))?;
        let tuples = state
            .model
            .relations
            .get(&relation_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for tuple in tuples {
            if tuple.len() != relation.columns.len() {
                return Err(ValidationError::RelationArityMismatch {
                    relation: relation_id,
                    expected: relation.columns.len(),
                    actual: tuple.len(),
                });
            }
            for (value, expected) in tuple.iter().zip(&relation.columns) {
                validate_value(
                    value,
                    expected,
                    &state.model,
                    context,
                    registry,
                    entity_types,
                    &BTreeMap::new(),
                )?;
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
        if let Some(rows) = state.model.relations.get(&relation_id) {
            semantic_subset.relations.insert(relation_id, rows.clone());
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

fn validate_value(
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel_schema::{CapabilityDef, FieldDef, RelationDef, Schema, SemanticEnvironment};
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    use super::*;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    fn fixture() -> (SemanticContext, SemanticRegistry, DatabaseState) {
        let person = SemanticId::new(1);
        let name = SemanticId::new(2);
        let relation = SemanticId::new(3);
        let text_eq = SemanticId::new(4);
        let entity_eq = SemanticId::new(5);
        let set_eq = SemanticId::new(6);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let entity_digest =
            registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_field(FieldDef {
                id: name,
                owner: person,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
                    TypeExpr::Set {
                        element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                        equivalence: text_eq,
                    },
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq, set_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        environment.pin_module(entity_eq, entity_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(10));
        state.lifecycle.roots.insert(id(10));
        state
            .model
            .carriers
            .insert(person, BTreeSet::from([id(10)]));
        state
            .model
            .fields
            .insert((name, id(10)), Value::Text("Ada".into()));
        state.model.relations.insert(
            relation,
            vec![vec![
                Value::LiveEntityRef {
                    entity_type: person,
                    id: id(10),
                },
                Value::Set {
                    equivalence: text_eq,
                    elements: vec![Value::Text("db".into())],
                },
            ]],
        );
        (context, registry, state)
    }

    #[test]
    fn well_typed_model_validates() {
        let (context, registry, state) = fixture();
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert!(
            dynamic_violation_measure(&context, &registry, &state, &extents)
                .unwrap()
                .is_zero()
        );
    }

    #[test]
    fn dynamic_violation_measure_exposes_missing_live_reference_witness() {
        let (context, registry, mut state) = fixture();
        let relation = SemanticId::new(3);
        let person = SemanticId::new(1);
        let missing = id(999);
        state.model.relations.get_mut(&relation).unwrap()[0][0] = Value::LiveEntityRef {
            entity_type: person,
            id: missing,
        };
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        let relation_measure =
            relation_dynamic_violation_measure(&context, &registry, &state, &extents, relation)
                .unwrap();
        assert_eq!(relation_measure, measure);
        assert_eq!(measure.witness_count(), 1);
        assert_eq!(
            measure.mass(&DynamicViolationWitness::MissingLiveReference {
                location: DynamicViolationLocation::RelationCell {
                    relation,
                    row: 0,
                    column: 0,
                },
                target_type: person,
                target: missing,
            }),
            1
        );
    }

    #[test]
    fn wrong_field_type_is_rejected() {
        let (context, registry, mut state) = fixture();
        state
            .model
            .fields
            .insert((SemanticId::new(2), id(10)), Value::I64(99));
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::TypeMismatch)
        );
    }

    #[test]
    fn product_shape_is_keyed_by_semantic_field_id() {
        let product_type = TypeExpr::Product(BTreeMap::from([
            (SemanticId::new(20), TypeExpr::Scalar(ScalarType::I64)),
            (SemanticId::new(21), TypeExpr::Scalar(ScalarType::Text)),
        ]));
        let value = Value::Product(BTreeMap::from([
            (SemanticId::new(20), Value::I64(1)),
            (SemanticId::new(22), Value::Text("wrong field".into())),
        ]));
        let (context, registry, state) = fixture();
        let entity_types = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert_eq!(
            validate_value(
                &value,
                &product_type,
                &state.model,
                &context,
                &registry,
                &entity_types,
                &BTreeMap::new()
            ),
            Err(ValidationError::ProductShapeMismatch)
        );
    }
    #[test]
    fn equality_domain_is_checked_even_for_singleton_collection() {
        let set_field = SemanticId::new(50);
        let text_eq = SemanticId::new(51);
        let mut registry = SemanticRegistry::default();
        let wrong_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_field(FieldDef {
                id: set_field,
                owner: SemanticId::new(1),
                value: TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: text_eq,
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, wrong_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(10));
        state.lifecycle.roots.insert(id(10));
        state
            .model
            .carriers
            .insert(SemanticId::new(1), BTreeSet::from([id(10)]));
        state.model.fields.insert(
            (set_field, id(10)),
            Value::Set {
                equivalence: text_eq,
                elements: vec![Value::Text("only".into())],
            },
        );
        assert!(matches!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::EquivalenceDomainMismatch { .. }
            ))
        ));
    }

    #[test]
    fn set_relation_rejects_semantically_duplicate_rows() {
        let relation = SemanticId::new(60);
        let text_eq = SemanticId::new(61);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into())],
                vec![Value::Text("alpha".into())],
            ],
        );
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::DuplicateRelationRow
            ))
        );

        let rows = state.model.relations.get(&relation).unwrap();
        let measure =
            relation_uniqueness_violation_measure(rows, &[text_eq], &context, &registry).unwrap();
        assert_eq!(measure.witness_count(), 1);
        assert_eq!(measure.iter().next().map(|(_, mass)| mass), Some(1));
    }

    #[test]
    fn relation_uniqueness_violation_measure_is_zero_for_distinct_gamma_classes() {
        let text_eq = SemanticId::new(62);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        };
        let rows = vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("Beta".into())],
        ];
        assert!(
            relation_uniqueness_violation_measure(&rows, &[text_eq], &context, &registry)
                .unwrap()
                .is_zero()
        );
    }
    #[test]
    fn unit_is_a_real_schema_type_not_an_untyped_runtime_sentinel() {
        let field = SemanticId::new(70);
        let owner_type = SemanticId::new(71);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_field(FieldDef {
                id: field,
                owner: owner_type,
                value: TypeExpr::Scalar(ScalarType::Unit),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state
            .model
            .carriers
            .insert(owner_type, BTreeSet::from([id(1)]));
        state.model.fields.insert((field, id(1)), Value::Unit);
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }
    #[test]
    fn structural_product_equivalence_validates_composite_set_keys() {
        let owner_type = SemanticId::new(80);
        let field = SemanticId::new(81);
        let product_eq = SemanticId::new(82);
        let text_eq = SemanticId::new(83);
        let i64_eq = SemanticId::new(84);
        let name_field = SemanticId::new(85);
        let age_field = SemanticId::new(86);
        let product_type = TypeExpr::Product(BTreeMap::from([
            (name_field, TypeExpr::Scalar(ScalarType::Text)),
            (age_field, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(name_field, text_eq), (age_field, i64_eq)]),
                },
            )
            .unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: owner_type,
                value: TypeExpr::Set {
                    element: Box::new(product_type),
                    equivalence: product_eq,
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state
            .model
            .carriers
            .insert(owner_type, BTreeSet::from([id(1)]));
        let key = |name: &str| {
            Value::Product(BTreeMap::from([
                (name_field, Value::Text(name.into())),
                (age_field, Value::I64(30)),
            ]))
        };
        state.model.fields.insert(
            (field, id(1)),
            Value::Set {
                equivalence: product_eq,
                elements: vec![key("ALICE"), key("alice")],
            },
        );
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::DuplicateSetElement {
                    equivalence: product_eq
                }
            ))
        );
    }
    #[test]
    fn empty_set_relation_still_rejects_wrong_column_equivalence_domain() {
        let relation = SemanticId::new(950);
        let wrong_eq = SemanticId::new(951);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let mut environment = kernel_schema::SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(wrong_eq, digest);
        let mut schema = kernel_schema::Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![wrong_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        assert!(matches!(
            registry.validate_context(&context),
            Err(SemanticError::EquivalenceDomainMismatch { .. })
        ));
    }

    #[test]
    fn dense_type_extents_match_subtype_membership_and_overlap() {
        let concrete = SemanticId::new(980);
        let secondary = SemanticId::new(981);
        let parent = SemanticId::new(982);
        let unrelated = SemanticId::new(983);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema.include(concrete, parent).unwrap();

        let mut model = FiniteModel::default();
        model
            .carriers
            .insert(concrete, BTreeSet::from([id(1), id(2)]));
        model
            .carriers
            .insert(secondary, BTreeSet::from([id(2), id(3)]));
        model.carriers.insert(unrelated, BTreeSet::from([id(4)]));

        let extents = DenseTypeExtents::compile(&model, &schema).unwrap();
        assert_eq!(extents.entity_count(), 4);
        assert!(extents.contains(id(1), concrete));
        assert!(extents.contains(id(1), parent));
        assert!(extents.contains(id(2), concrete));
        assert!(extents.contains(id(2), secondary));
        assert!(extents.contains(id(3), secondary));
        assert!(!extents.contains(id(3), parent));
        assert!(extents.contains(id(4), unrelated));
        assert!(!extents.contains(id(4), parent));
    }

    #[test]
    fn validation_accepts_subtype_owner_through_dense_extent() {
        let concrete = SemanticId::new(990);
        let parent = SemanticId::new(991);
        let field = SemanticId::new(992);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema.include(concrete, parent).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: parent,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state.model.fields.insert((field, id(1)), Value::I64(7));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }

    #[test]
    fn capability_required_fields_are_enforced_for_every_member() {
        let concrete = SemanticId::new(1_100);
        let capability = SemanticId::new(1_101);
        let field = SemanticId::new(1_102);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: capability,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1), id(2)]));
        state.lifecycle.entities.extend([id(1), id(2)]);
        state.lifecycle.roots.extend([id(1), id(2)]);
        state
            .model
            .fields
            .insert((field, id(1)), Value::Text("present".into()));

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::MissingCapabilityRequiredField {
                capability,
                field,
                entity: id(2),
            })
        );

        state
            .model
            .fields
            .insert((field, id(2)), Value::Text("present too".into()));
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }

    #[test]
    fn capability_required_field_contract_must_match_schema_field() {
        let concrete = SemanticId::new(1_110);
        let capability = SemanticId::new(1_111);
        let field = SemanticId::new(1_112);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: capability,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state.model.fields.insert((field, id(1)), Value::I64(7));

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::CapabilityRequiredFieldContractMismatch { capability, field })
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_dense_type_extent_membership_against_carrier_scan() {
        use std::hint::black_box;
        use std::time::Instant;

        let parent = SemanticId::new(10_000);
        let carrier_count = 128_u128;
        let per_carrier = 256_u128;
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        let mut model = FiniteModel::default();
        let mut probes = Vec::new();
        for carrier_index in 0..carrier_count {
            let actual = SemanticId::new(20_000 + carrier_index);
            schema.include(actual, parent).unwrap();
            let start = carrier_index * per_carrier + 1;
            let entities = (start..start + per_carrier)
                .map(id)
                .collect::<BTreeSet<_>>();
            probes.extend(entities.iter().copied());
            model.carriers.insert(actual, entities);
        }
        let compile_start = Instant::now();
        let dense = DenseTypeExtents::compile(&model, &schema).unwrap();
        let compile_ns = compile_start.elapsed().as_nanos();

        let start = Instant::now();
        let mut baseline_hits = 0_usize;
        for _ in 0..4 {
            for &entity in &probes {
                let hit = model.carriers.iter().any(|(&actual, entities)| {
                    entities.contains(&entity) && schema.is_subtype(actual, parent)
                });
                baseline_hits += usize::from(black_box(hit));
            }
        }
        let baseline_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut dense_hits = 0_usize;
        for _ in 0..4 {
            for &entity in &probes {
                dense_hits += usize::from(black_box(dense.contains(entity, parent)));
            }
        }
        let dense_ns = start.elapsed().as_nanos();
        assert_eq!(baseline_hits, dense_hits);
        let ratio_milli = baseline_ns.saturating_mul(1_000) / dense_ns.max(1);
        println!(
            "compile_ns={compile_ns} baseline_ns={baseline_ns} dense_ns={dense_ns} ratio_milli={ratio_milli} probes={}",
            probes.len()
        );
    }
}
