use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{SchemaRevisionId, SemanticId};

use crate::{
    CapabilityDef, ExactAggregateMeasureExpr, FieldDef, FieldRule, ModelRuleExpr,
    OwnedRelationshipDef, PermissionCoordinate, RelationDef, RelationSemantics, RuleValueExpr,
    SchemaAccess, SemanticRuleExpr, SemanticRuleTypeError, StructuralEquivalenceDef,
    StructuralOrderingDef, SubtypeClosure, Symbol, TypeError, TypeExpr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub revision: SchemaRevisionId,
    symbols: BTreeMap<SemanticId, Symbol>,
    types: BTreeMap<SemanticId, TypeExpr>,
    capabilities: BTreeMap<SemanticId, CapabilityDef>,
    access: SchemaAccess,
    fields: BTreeMap<SemanticId, FieldDef>,
    field_rules: BTreeMap<SemanticId, Vec<FieldRule>>,
    relation_column_rules: BTreeMap<(SemanticId, SemanticId), Vec<FieldRule>>,
    entity_rules: BTreeMap<SemanticId, Vec<SemanticRuleExpr>>,
    model_rules: Vec<ModelRuleExpr>,
    relation_column_ids: BTreeMap<SemanticId, Vec<SemanticId>>,
    relations: BTreeMap<SemanticId, RelationDef>,
    owned_relationships: BTreeMap<SemanticId, OwnedRelationshipDef>,
    structural_equivalences: BTreeMap<SemanticId, StructuralEquivalenceDef>,
    structural_orderings: BTreeMap<SemanticId, StructuralOrderingDef>,
    inclusions: BTreeSet<(SemanticId, SemanticId)>,
    subtype_closure: SubtypeClosure,
}

fn validate_field_rule_type(rule: &FieldRule, ty: &TypeExpr) -> Result<(), SchemaError> {
    match rule.expression().validate_for_input(ty) {
        Ok(()) => Ok(()),
        Err(crate::SemanticRuleTypeError::InvalidBounds) => {
            Err(SchemaError::InvalidFieldRuleBounds)
        }
        Err(
            crate::SemanticRuleTypeError::TypeMismatch
            | crate::SemanticRuleTypeError::UnknownField(_)
            | crate::SemanticRuleTypeError::FieldOutsideOwner { .. },
        ) => Err(SchemaError::FieldRuleTypeMismatch),
    }
}

fn validate_permission_coordinate(
    schema: &Schema,
    permission: PermissionCoordinate,
) -> Result<(), SchemaError> {
    let validate_relation = |relation: SemanticId| {
        schema
            .relations
            .contains_key(&relation)
            .then_some(())
            .ok_or(SchemaError::UnknownAuthorizationRelation(relation))
    };
    let validate_field = |relation: SemanticId, column: SemanticId| {
        validate_relation(relation)?;
        let columns = schema
            .relation_column_ids
            .get(&relation)
            .ok_or(SchemaError::UnknownAuthorizationRelation(relation))?;
        columns
            .contains(&column)
            .then_some(())
            .ok_or(SchemaError::UnknownAuthorizationColumn { relation, column })
    };
    match permission {
        PermissionCoordinate::ReadRelation { relation }
        | PermissionCoordinate::WriteRelation { relation }
        | PermissionCoordinate::CreateObject { relation }
        | PermissionCoordinate::DeleteObject { relation }
        | PermissionCoordinate::AttachRelationship { relation }
        | PermissionCoordinate::DetachRelationship { relation }
        | PermissionCoordinate::MoveRelationship { relation } => validate_relation(relation),
        PermissionCoordinate::ReadField { relation, column }
        | PermissionCoordinate::WriteField { relation, column } => validate_field(relation, column),
        PermissionCoordinate::ModelRead
        | PermissionCoordinate::HistoricalRead
        | PermissionCoordinate::HistoryRead
        | PermissionCoordinate::Watch
        | PermissionCoordinate::WriteCarrierPresence { .. }
        | PermissionCoordinate::WriteCarrierMember { .. }
        | PermissionCoordinate::WriteLifecycleEntity { .. }
        | PermissionCoordinate::WriteLifecycleRoot { .. }
        | PermissionCoordinate::WriteKeepsAlivePresence { .. }
        | PermissionCoordinate::WriteKeepsAliveEdge { .. } => Ok(()),
    }
}

fn visit_access_role(
    role: SemanticId,
    policy: &SchemaAccess,
    visiting: &mut BTreeSet<SemanticId>,
    visited: &mut BTreeSet<SemanticId>,
) -> Result<(), SchemaError> {
    if visited.contains(&role) {
        return Ok(());
    }
    if !visiting.insert(role) {
        return Err(SchemaError::AccessRoleCycle(role));
    }
    let definition = policy
        .roles
        .get(&role)
        .ok_or(SchemaError::UnknownAccessRole(role))?;
    for included in &definition.includes {
        visit_access_role(*included, policy, visiting, visited)?;
    }
    visiting.remove(&role);
    visited.insert(role);
    Ok(())
}

fn validate_schema_access(schema: &Schema, policy: &SchemaAccess) -> Result<(), SchemaError> {
    for capability in policy.capabilities.values() {
        for permission in &capability.permissions {
            validate_permission_coordinate(schema, *permission)?;
        }
    }
    for role in policy.roles.values() {
        for capability in &role.capabilities {
            if !policy.capabilities.contains_key(capability) {
                return Err(SchemaError::UnknownAccessCapability(*capability));
            }
        }
        for included in &role.includes {
            if !policy.roles.contains_key(included) {
                return Err(SchemaError::UnknownAccessRole(*included));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for role in policy.roles.keys().copied() {
        visit_access_role(role, policy, &mut visiting, &mut visited)?;
    }
    Ok(())
}

impl Schema {
    #[must_use]
    pub fn new(revision: SchemaRevisionId) -> Self {
        Self {
            revision,
            symbols: BTreeMap::new(),
            types: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            access: SchemaAccess::default(),
            fields: BTreeMap::new(),
            field_rules: BTreeMap::new(),
            relation_column_rules: BTreeMap::new(),
            entity_rules: BTreeMap::new(),
            model_rules: Vec::new(),
            relation_column_ids: BTreeMap::new(),
            relations: BTreeMap::new(),
            owned_relationships: BTreeMap::new(),
            structural_equivalences: BTreeMap::new(),
            structural_orderings: BTreeMap::new(),
            inclusions: BTreeSet::new(),
            subtype_closure: SubtypeClosure::default(),
        }
    }

    pub fn define(&mut self, symbol: Symbol) -> Result<(), SchemaError> {
        if self.symbols.insert(symbol.id, symbol).is_some() {
            return Err(SchemaError::DuplicateSemanticId);
        }
        Ok(())
    }

    pub fn define_type(&mut self, id: SemanticId, definition: TypeExpr) -> Result<(), SchemaError> {
        definition.validate().map_err(SchemaError::InvalidType)?;
        if self.types.insert(id, definition).is_some() {
            return Err(SchemaError::DuplicateTypeDefinition);
        }
        Ok(())
    }

    pub fn define_field(&mut self, field: FieldDef) -> Result<(), SchemaError> {
        field.value.validate().map_err(SchemaError::InvalidType)?;
        if self.fields.insert(field.id, field).is_some() {
            return Err(SchemaError::DuplicateField);
        }
        Ok(())
    }

    pub fn add_field_rule(
        &mut self,
        field: SemanticId,
        rule: FieldRule,
    ) -> Result<(), SchemaError> {
        let definition = self
            .fields
            .get(&field)
            .ok_or(SchemaError::UnknownFieldForRule(field))?;
        validate_field_rule_type(&rule, &definition.value)?;
        self.field_rules.entry(field).or_default().push(rule);
        Ok(())
    }

    pub fn validate_entity_rule(
        &self,
        owner: SemanticId,
        rule: &SemanticRuleExpr,
    ) -> Result<(), SchemaError> {
        rule.validate_values(&mut |value| match value {
            RuleValueExpr::Input => Err(SemanticRuleTypeError::TypeMismatch),
            RuleValueExpr::Field(field_id) => {
                let field = self
                    .fields
                    .get(field_id)
                    .ok_or(SemanticRuleTypeError::UnknownField(*field_id))?;
                if !self.is_subtype(owner, field.owner) {
                    return Err(SemanticRuleTypeError::FieldOutsideOwner {
                        field: *field_id,
                        owner,
                    });
                }
                Ok(field.value.clone())
            }
        })
        .map_err(|error| match error {
            SemanticRuleTypeError::InvalidBounds => SchemaError::InvalidFieldRuleBounds,
            SemanticRuleTypeError::UnknownField(field) => SchemaError::UnknownFieldForRule(field),
            SemanticRuleTypeError::TypeMismatch
            | SemanticRuleTypeError::FieldOutsideOwner { .. } => {
                SchemaError::EntityRuleTypeMismatch
            }
        })
    }

    pub fn add_entity_rule(
        &mut self,
        owner: SemanticId,
        rule: SemanticRuleExpr,
    ) -> Result<(), SchemaError> {
        self.validate_entity_rule(owner, &rule)?;
        self.entity_rules.entry(owner).or_default().push(rule);
        Ok(())
    }

    #[must_use]
    pub fn entity_rules(&self, owner: SemanticId) -> &[SemanticRuleExpr] {
        self.entity_rules
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn all_entity_rules(&self) -> impl Iterator<Item = (SemanticId, &SemanticRuleExpr)> {
        self.entity_rules
            .iter()
            .flat_map(|(&owner, rules)| rules.iter().map(move |rule| (owner, rule)))
    }

    pub fn add_model_rule(&mut self, rule: ModelRuleExpr) -> Result<(), SchemaError> {
        match &rule {
            ModelRuleExpr::RelationExactMeasure { constraint } => {
                self.validate_exact_measure_constraint(constraint)?;
            }
            ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns,
                group_equivalences,
                constraint,
            } => {
                self.validate_exact_measure_constraint(constraint)?;
                let relation = Self::grouped_exact_measure_relation(constraint)?;
                self.validate_group_coordinates(relation, group_columns, group_equivalences)?;
            }
        }
        self.model_rules.push(rule);
        Ok(())
    }

    fn validate_exact_measure_constraint(
        &self,
        constraint: &crate::ExactMeasureConstraint,
    ) -> Result<(), SchemaError> {
        use crate::{ExactAggregateRange, ExactMeasureConstraint};

        match constraint {
            ExactMeasureConstraint::Range { measure, range } => {
                self.validate_exact_aggregate_measure(measure)?;
                match (measure, range) {
                    (
                        ExactAggregateMeasureExpr::Count { .. },
                        ExactAggregateRange::Count { min, max },
                    ) => {
                        if max.is_some_and(|max| max < *min) {
                            return Err(SchemaError::InvalidFieldRuleBounds);
                        }
                    }
                    (
                        ExactAggregateMeasureExpr::F64Sum { .. },
                        ExactAggregateRange::F64Sum { min, max },
                    ) => {
                        if min
                            .zip(*max)
                            .is_some_and(|(min, max)| min.value() > max.value())
                        {
                            return Err(SchemaError::InvalidFieldRuleBounds);
                        }
                    }
                    (
                        ExactAggregateMeasureExpr::OrderedStatistic {
                            relation, column, ..
                        },
                        ExactAggregateRange::OrderedStatistic { min, max },
                    ) => {
                        let ty = self.relation_column_type(*relation, *column).ok_or(
                            SchemaError::UnknownRelationColumnForRule {
                                relation: *relation,
                                column: *column,
                            },
                        )?;
                        if min
                            .iter()
                            .chain(max.iter())
                            .any(|bound| !Self::ordered_statistic_bound_matches_type(bound, ty))
                        {
                            return Err(SchemaError::EntityRuleTypeMismatch);
                        }
                    }
                    _ => return Err(SchemaError::EntityRuleTypeMismatch),
                }
            }
            ExactMeasureConstraint::Compare { left, right, .. } => {
                self.validate_exact_aggregate_pair(left, right)?;
            }
        }
        Ok(())
    }

    fn ordered_statistic_bound_matches_type(
        bound: &crate::OrderedStatisticBound,
        ty: &TypeExpr,
    ) -> bool {
        match (bound, ty) {
            (crate::OrderedStatisticBound::Unit, TypeExpr::Scalar(crate::ScalarType::Unit))
            | (crate::OrderedStatisticBound::Bool(_), TypeExpr::Scalar(crate::ScalarType::Bool))
            | (crate::OrderedStatisticBound::I64(_), TypeExpr::Scalar(crate::ScalarType::I64))
            | (
                crate::OrderedStatisticBound::F64Bits(_),
                TypeExpr::Scalar(crate::ScalarType::F64),
            )
            | (crate::OrderedStatisticBound::Text(_), TypeExpr::Scalar(crate::ScalarType::Text)) => {
                true
            }
            (
                crate::OrderedStatisticBound::LiveEntityId { entity_type, .. },
                TypeExpr::Scalar(crate::ScalarType::LiveEntityRef(expected)),
            )
            | (
                crate::OrderedStatisticBound::HistoricalEntityId { entity_type, .. },
                TypeExpr::Scalar(crate::ScalarType::HistoricalEntityId(expected)),
            ) => entity_type == expected,
            _ => false,
        }
    }

    fn grouped_exact_measure_relation(
        constraint: &crate::ExactMeasureConstraint,
    ) -> Result<SemanticId, SchemaError> {
        let relation = match constraint {
            crate::ExactMeasureConstraint::Range { measure, .. } => {
                Self::exact_aggregate_relation(measure)
            }
            crate::ExactMeasureConstraint::Compare { left, right, .. } => {
                let relation = Self::exact_aggregate_relation(left);
                if Self::exact_aggregate_relation(right) != relation {
                    return Err(SchemaError::EntityRuleTypeMismatch);
                }
                relation
            }
        };
        Ok(relation)
    }

    fn exact_aggregate_relation(measure: &ExactAggregateMeasureExpr) -> SemanticId {
        match measure {
            ExactAggregateMeasureExpr::Count { relation, .. }
            | ExactAggregateMeasureExpr::F64Sum { relation, .. }
            | ExactAggregateMeasureExpr::OrderedStatistic { relation, .. } => *relation,
        }
    }

    fn validate_exact_aggregate_pair(
        &self,
        left: &ExactAggregateMeasureExpr,
        right: &ExactAggregateMeasureExpr,
    ) -> Result<(), SchemaError> {
        self.validate_exact_aggregate_measure(left)?;
        self.validate_exact_aggregate_measure(right)?;
        let homogeneous = match (left, right) {
            (ExactAggregateMeasureExpr::Count { .. }, ExactAggregateMeasureExpr::Count { .. })
            | (
                ExactAggregateMeasureExpr::F64Sum { .. },
                ExactAggregateMeasureExpr::F64Sum { .. },
            ) => true,
            (
                ExactAggregateMeasureExpr::OrderedStatistic {
                    ordering: left_ordering,
                    relation: left_relation,
                    column: left_column,
                    ..
                },
                ExactAggregateMeasureExpr::OrderedStatistic {
                    ordering: right_ordering,
                    relation: right_relation,
                    column: right_column,
                    ..
                },
            ) => {
                left_ordering == right_ordering
                    && self.relation_column_type(*left_relation, *left_column)
                        == self.relation_column_type(*right_relation, *right_column)
            }
            _ => false,
        };
        if !homogeneous {
            return Err(SchemaError::EntityRuleTypeMismatch);
        }
        Ok(())
    }

    fn validate_group_coordinates(
        &self,
        relation: SemanticId,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
    ) -> Result<(), SchemaError> {
        let Some(definition) = self.relation(relation) else {
            return Err(SchemaError::UnknownRelationForRule(relation));
        };
        if group_columns.is_empty() || group_columns.len() != group_equivalences.len() {
            return Err(SchemaError::RelationEquivalenceArityMismatch);
        }
        let declared_equivalences = match &definition.semantics {
            crate::RelationSemantics::Set {
                column_equivalences,
            }
            | crate::RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        for (column, equivalence) in group_columns.iter().zip(group_equivalences) {
            let Some(ordinal) = self.relation_column_ordinal(relation, *column) else {
                return Err(SchemaError::UnknownRelationColumnForRule {
                    relation,
                    column: *column,
                });
            };
            if declared_equivalences.get(ordinal) != Some(equivalence) {
                return Err(SchemaError::EntityRuleTypeMismatch);
            }
        }
        Ok(())
    }

    fn validate_exact_aggregate_measure(
        &self,
        measure: &ExactAggregateMeasureExpr,
    ) -> Result<(), SchemaError> {
        match measure {
            ExactAggregateMeasureExpr::Count {
                relation,
                predicate,
            } => {
                if self.relation(*relation).is_none() {
                    return Err(SchemaError::UnknownRelationForRule(*relation));
                }
                self.validate_relation_row_rule(*relation, predicate)
            }
            ExactAggregateMeasureExpr::F64Sum {
                relation,
                column,
                predicate,
            } => {
                if self.relation(*relation).is_none() {
                    return Err(SchemaError::UnknownRelationForRule(*relation));
                }
                if self.relation_column_type(*relation, *column)
                    != Some(&TypeExpr::Scalar(crate::ScalarType::F64))
                {
                    return Err(SchemaError::EntityRuleTypeMismatch);
                }
                self.validate_relation_row_rule(*relation, predicate)
            }
            ExactAggregateMeasureExpr::OrderedStatistic {
                relation,
                column,
                predicate,
                selector,
                ..
            } => {
                if self.relation(*relation).is_none() {
                    return Err(SchemaError::UnknownRelationForRule(*relation));
                }
                if self.relation_column_type(*relation, *column).is_none() {
                    return Err(SchemaError::UnknownRelationColumnForRule {
                        relation: *relation,
                        column: *column,
                    });
                }
                if matches!(
                    selector,
                    crate::OrderedStatisticSelector::LowerQuantile { numerator, denominator }
                        if *denominator == 0 || *numerator > *denominator
                ) {
                    return Err(SchemaError::InvalidExactOrderStatistic);
                }
                self.validate_relation_row_rule(*relation, predicate)
            }
        }
    }

    #[must_use]
    pub fn model_rules(&self) -> &[ModelRuleExpr] {
        &self.model_rules
    }

    pub fn validate_relation_row_rule(
        &self,
        relation: SemanticId,
        rule: &SemanticRuleExpr,
    ) -> Result<(), SchemaError> {
        rule.validate_values(&mut |value| match value {
            RuleValueExpr::Input => Err(SemanticRuleTypeError::TypeMismatch),
            RuleValueExpr::Field(column) => self
                .relation_column_type(relation, *column)
                .cloned()
                .ok_or(SemanticRuleTypeError::UnknownField(*column)),
        })
        .map_err(|error| match error {
            SemanticRuleTypeError::InvalidBounds => SchemaError::InvalidFieldRuleBounds,
            SemanticRuleTypeError::UnknownField(field) => SchemaError::UnknownFieldForRule(field),
            SemanticRuleTypeError::TypeMismatch
            | SemanticRuleTypeError::FieldOutsideOwner { .. } => {
                SchemaError::EntityRuleTypeMismatch
            }
        })
    }

    pub fn add_relation_column_rule(
        &mut self,
        relation: SemanticId,
        column: SemanticId,
        rule: FieldRule,
    ) -> Result<(), SchemaError> {
        let ty = self
            .relation_column_type(relation, column)
            .ok_or(SchemaError::UnknownRelationColumnForRule { relation, column })?;
        validate_field_rule_type(&rule, ty)?;
        self.relation_column_rules
            .entry((relation, column))
            .or_default()
            .push(rule);
        Ok(())
    }

    /// Defines a relation with explicit stable semantic identities for its columns.
    ///
    /// Column order remains a physical/query lowering coordinate. `column_ids` are
    /// the schema identity and survive reorder/representation changes.
    pub fn define_relation_with_column_ids(
        &mut self,
        relation: RelationDef,
        column_ids: Vec<SemanticId>,
    ) -> Result<(), SchemaError> {
        for column in &relation.columns {
            column.validate().map_err(SchemaError::InvalidType)?;
        }
        let column_equivalences = match &relation.semantics {
            RelationSemantics::Set {
                column_equivalences,
            }
            | RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        if column_equivalences.len() != relation.columns.len() {
            return Err(SchemaError::RelationEquivalenceArityMismatch);
        }
        if column_ids.len() != relation.columns.len() {
            return Err(SchemaError::RelationColumnIdentityArityMismatch);
        }
        let unique = column_ids.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != column_ids.len() {
            return Err(SchemaError::DuplicateRelationColumnIdentity);
        }
        let id = relation.id;
        if self.relations.contains_key(&id) {
            return Err(SchemaError::DuplicateRelation);
        }
        self.relation_column_ids.insert(id, column_ids);
        self.relations.insert(id, relation);
        Ok(())
    }

    /// Convenience definition for relations that do not yet provide explicit
    /// semantic column identities. The generated identities are deterministic
    /// within the relation. Schema-evolution code should use
    /// `define_relation_with_column_ids` so identity is independent of ordinal.
    pub fn define_relation(&mut self, relation: RelationDef) -> Result<(), SchemaError> {
        let column_ids = (0..relation.columns.len())
            .map(|column| SemanticId::new((column as u128) + 1))
            .collect();
        self.define_relation_with_column_ids(relation, column_ids)
    }

    pub fn define_owned_relationship(
        &mut self,
        definition: OwnedRelationshipDef,
    ) -> Result<(), SchemaError> {
        if !self.relations.contains_key(&definition.relation) {
            return Err(SchemaError::UnknownOwnedRelationshipRelation(
                definition.relation,
            ));
        }
        if !self.relations.contains_key(&definition.target_relation) {
            return Err(SchemaError::UnknownOwnedRelationshipTarget(
                definition.target_relation,
            ));
        }
        if self
            .owned_relationships
            .insert(definition.relation, definition)
            .is_some()
        {
            return Err(SchemaError::DuplicateOwnedRelationship);
        }
        Ok(())
    }

    #[must_use]
    pub fn owned_relationship(&self, relation: SemanticId) -> Option<&OwnedRelationshipDef> {
        self.owned_relationships.get(&relation)
    }

    pub fn owned_relationships(&self) -> impl Iterator<Item = &OwnedRelationshipDef> {
        self.owned_relationships.values()
    }

    pub fn set_schema_access(&mut self, policy: SchemaAccess) -> Result<(), SchemaError> {
        validate_schema_access(self, &policy)?;
        self.access = policy;
        Ok(())
    }

    #[must_use]
    pub const fn schema_access(&self) -> &SchemaAccess {
        &self.access
    }

    pub fn resolve_access_roles(
        &self,
        roles: impl IntoIterator<Item = SemanticId>,
    ) -> Result<BTreeSet<PermissionCoordinate>, SchemaError> {
        let mut permissions = BTreeSet::new();
        let mut pending = roles.into_iter().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(role) = pending.pop() {
            if !visited.insert(role) {
                continue;
            }
            let definition = self
                .access
                .roles
                .get(&role)
                .ok_or(SchemaError::UnknownAccessRole(role))?;
            for capability in &definition.capabilities {
                let definition = self
                    .access
                    .capabilities
                    .get(capability)
                    .ok_or(SchemaError::UnknownAccessCapability(*capability))?;
                permissions.extend(definition.permissions.iter().copied());
            }
            pending.extend(definition.includes.iter().copied());
        }
        Ok(permissions)
    }

    pub fn define_capability(&mut self, capability: CapabilityDef) -> Result<(), SchemaError> {
        for field_type in capability.required_fields.values() {
            field_type.validate().map_err(SchemaError::InvalidType)?;
        }
        if self
            .capabilities
            .insert(capability.id, capability)
            .is_some()
        {
            return Err(SchemaError::DuplicateCapability);
        }
        Ok(())
    }

    pub fn include(
        &mut self,
        subtype: SemanticId,
        supertype: SemanticId,
    ) -> Result<(), SchemaError> {
        if subtype == supertype || self.is_subtype(supertype, subtype) {
            return Err(SchemaError::SubtypeCycle { subtype, supertype });
        }
        if self.inclusions.insert((subtype, supertype)) {
            self.subtype_closure.include(subtype, supertype);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_subtype(&self, subtype: SemanticId, supertype: SemanticId) -> bool {
        self.subtype_closure.is_subtype(subtype, supertype)
    }

    #[must_use]
    pub const fn subtype_closure(&self) -> &SubtypeClosure {
        &self.subtype_closure
    }

    pub fn rename(
        &mut self,
        id: SemanticId,
        new_name: impl Into<String>,
    ) -> Result<(), SchemaError> {
        let symbol = self
            .symbols
            .get_mut(&id)
            .ok_or(SchemaError::UnknownSemanticId)?;
        symbol.presentation_name = new_name.into();
        Ok(())
    }

    #[must_use]
    pub fn symbol(&self, id: SemanticId) -> Option<&Symbol> {
        self.symbols.get(&id)
    }

    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }

    #[must_use]
    pub fn type_definition(&self, id: SemanticId) -> Option<&TypeExpr> {
        self.types.get(&id)
    }

    pub fn type_definitions(&self) -> impl Iterator<Item = (SemanticId, &TypeExpr)> {
        self.types.iter().map(|(&id, definition)| (id, definition))
    }

    #[must_use]
    pub fn capability(&self, id: SemanticId) -> Option<&CapabilityDef> {
        self.capabilities.get(&id)
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilityDef> {
        self.capabilities.values()
    }

    #[must_use]
    pub fn field(&self, id: SemanticId) -> Option<&FieldDef> {
        self.fields.get(&id)
    }

    pub fn fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.values()
    }

    #[must_use]
    pub fn field_rules(&self, field: SemanticId) -> &[FieldRule] {
        self.field_rules
            .get(&field)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn all_field_rules(&self) -> impl Iterator<Item = (SemanticId, &FieldRule)> {
        self.field_rules
            .iter()
            .flat_map(|(&field, rules)| rules.iter().map(move |rule| (field, rule)))
    }

    #[must_use]
    pub fn relation_column_ids(&self, relation: SemanticId) -> Option<&[SemanticId]> {
        self.relation_column_ids.get(&relation).map(Vec::as_slice)
    }

    #[must_use]
    pub fn relation_column_id(&self, relation: SemanticId, ordinal: usize) -> Option<SemanticId> {
        self.relation_column_ids(relation)?.get(ordinal).copied()
    }

    #[must_use]
    pub fn relation_column_ordinal(
        &self,
        relation: SemanticId,
        column: SemanticId,
    ) -> Option<usize> {
        self.relation_column_ids(relation)?
            .iter()
            .position(|candidate| *candidate == column)
    }

    #[must_use]
    pub fn relation_column_type(
        &self,
        relation: SemanticId,
        column: SemanticId,
    ) -> Option<&TypeExpr> {
        let ordinal = self.relation_column_ordinal(relation, column)?;
        self.relation(relation)?.columns.get(ordinal)
    }

    #[must_use]
    pub fn relation_column_rules_by_id(
        &self,
        relation: SemanticId,
        column: SemanticId,
    ) -> &[FieldRule] {
        self.relation_column_rules
            .get(&(relation, column))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn relation_column_rules(&self, relation: SemanticId, ordinal: usize) -> &[FieldRule] {
        self.relation_column_id(relation, ordinal)
            .map(|column| self.relation_column_rules_by_id(relation, column))
            .unwrap_or_default()
    }

    pub fn all_relation_column_rules(
        &self,
    ) -> impl Iterator<Item = ((SemanticId, SemanticId), &FieldRule)> {
        self.relation_column_rules
            .iter()
            .flat_map(|(&target, rules)| rules.iter().map(move |rule| (target, rule)))
    }

    #[must_use]
    pub fn relation(&self, id: SemanticId) -> Option<&RelationDef> {
        self.relations.get(&id)
    }

    pub fn relations(&self) -> impl Iterator<Item = &RelationDef> {
        self.relations.values()
    }

    pub fn define_structural_equivalence(
        &mut self,
        id: SemanticId,
        definition: StructuralEquivalenceDef,
    ) -> Result<(), SchemaError> {
        if self
            .structural_equivalences
            .insert(id, definition)
            .is_some()
        {
            return Err(SchemaError::DuplicateStructuralEquivalence);
        }
        Ok(())
    }

    #[must_use]
    pub fn structural_equivalence(&self, id: SemanticId) -> Option<&StructuralEquivalenceDef> {
        self.structural_equivalences.get(&id)
    }

    pub fn structural_equivalences(
        &self,
    ) -> impl Iterator<Item = (SemanticId, &StructuralEquivalenceDef)> {
        self.structural_equivalences
            .iter()
            .map(|(id, definition)| (*id, definition))
    }

    pub fn define_structural_ordering(
        &mut self,
        id: SemanticId,
        definition: StructuralOrderingDef,
    ) -> Result<(), SchemaError> {
        if self.structural_orderings.insert(id, definition).is_some() {
            return Err(SchemaError::DuplicateStructuralOrdering);
        }
        Ok(())
    }

    #[must_use]
    pub fn structural_ordering(&self, id: SemanticId) -> Option<&StructuralOrderingDef> {
        self.structural_orderings.get(&id)
    }

    pub fn structural_orderings(
        &self,
    ) -> impl Iterator<Item = (SemanticId, &StructuralOrderingDef)> {
        self.structural_orderings
            .iter()
            .map(|(id, definition)| (*id, definition))
    }

    #[must_use]
    pub fn has_direct_inclusion(&self, subtype: SemanticId, supertype: SemanticId) -> bool {
        self.inclusions.contains(&(subtype, supertype))
    }

    pub fn inclusions(&self) -> impl Iterator<Item = (SemanticId, SemanticId)> + '_ {
        self.inclusions.iter().copied()
    }

    #[must_use]
    pub fn semantic_dependencies(&self) -> BTreeSet<SemanticId> {
        let mut dependencies = BTreeSet::new();
        for definition in self.types.values() {
            definition.collect_semantic_dependencies(&mut dependencies);
        }
        for capability in self.capabilities.values() {
            for field_type in capability.required_fields.values() {
                field_type.collect_semantic_dependencies(&mut dependencies);
            }
        }
        for field in self.fields.values() {
            field.value.collect_semantic_dependencies(&mut dependencies);
        }
        for relation in self.relations.values() {
            for column in &relation.columns {
                column.collect_semantic_dependencies(&mut dependencies);
            }
            let column_equivalences = match &relation.semantics {
                RelationSemantics::Set {
                    column_equivalences,
                }
                | RelationSemantics::Bag {
                    column_equivalences,
                } => column_equivalences,
            };
            dependencies.extend(column_equivalences.iter().copied());
        }
        let mut pending: Vec<_> = dependencies.iter().copied().collect();
        let mut visited = BTreeSet::new();
        while let Some(dependency) = pending.pop() {
            if !visited.insert(dependency) {
                continue;
            }
            if let Some(definition) = self.structural_equivalences.get(&dependency) {
                dependencies.remove(&dependency);
                let children: Vec<_> = match definition {
                    StructuralEquivalenceDef::Mu { body } => vec![*body],
                    StructuralEquivalenceDef::Var { binder } => vec![*binder],
                    StructuralEquivalenceDef::Product { fields } => {
                        fields.values().copied().collect()
                    }
                    StructuralEquivalenceDef::Option { inner } => vec![*inner],
                    StructuralEquivalenceDef::Sum { variants } => {
                        variants.values().copied().collect()
                    }
                    StructuralEquivalenceDef::Set { element }
                    | StructuralEquivalenceDef::Bag { element }
                    | StructuralEquivalenceDef::Seq { element } => vec![*element],
                    StructuralEquivalenceDef::Map { key, value } => vec![*key, *value],
                };
                for child in children {
                    dependencies.insert(child);
                    pending.push(child);
                }
            }
        }
        for derived in self.structural_equivalences.keys() {
            dependencies.remove(derived);
        }
        dependencies
    }

    #[must_use]
    pub fn field_transport_base_equivalent(&self, other: &Self) -> bool {
        self.types == other.types
            && self.capabilities == other.capabilities
            && self.relations == other.relations
            && self.owned_relationships == other.owned_relationships
            && self.structural_equivalences == other.structural_equivalences
            && self.structural_orderings == other.structural_orderings
            && self.inclusions == other.inclusions
    }

    #[must_use]
    pub fn relation_transport_base_equivalent(&self, other: &Self) -> bool {
        self.types == other.types
            && self.capabilities == other.capabilities
            && self.access == other.access
            && self.fields == other.fields
            && self.owned_relationships == other.owned_relationships
            && self.structural_equivalences == other.structural_equivalences
            && self.structural_orderings == other.structural_orderings
            && self.inclusions == other.inclusions
    }

    /// Returns whether two schema revisions have the same non-data-bearing
    /// structural foundation and therefore may be connected by one explicit
    /// data migration. Fields, field rules, relations, relation-column rules,
    /// presentation symbols and the schema revision itself are intentionally
    /// excluded: those are the coordinates a migration is allowed to replace.
    #[must_use]
    pub fn migration_base_equivalent(&self, other: &Self) -> bool {
        self.types == other.types
            && self.capabilities == other.capabilities
            && self.structural_equivalences == other.structural_equivalences
            && self.structural_orderings == other.structural_orderings
            && self.inclusions == other.inclusions
    }

    #[must_use]
    pub fn definitionally_equivalent(&self, other: &Self) -> bool {
        let symbols_match = self.symbols.len() == other.symbols.len()
            && self.symbols.iter().all(|(id, symbol)| {
                other
                    .symbols
                    .get(id)
                    .is_some_and(|other| symbol.kind == other.kind)
            });
        symbols_match
            && self.types == other.types
            && self.capabilities == other.capabilities
            && self.access == other.access
            && self.fields == other.fields
            && self.field_rules == other.field_rules
            && self.relation_column_rules == other.relation_column_rules
            && self.relation_column_ids == other.relation_column_ids
            && self.entity_rules == other.entity_rules
            && self.relations == other.relations
            && self.owned_relationships == other.owned_relationships
            && self.structural_equivalences == other.structural_equivalences
            && self.structural_orderings == other.structural_orderings
            && self.inclusions == other.inclusions
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    DuplicateSemanticId,
    UnknownSemanticId,
    DuplicateTypeDefinition,
    DuplicateCapability,
    UnknownAccessCapability(SemanticId),
    UnknownAccessRole(SemanticId),
    UnknownAuthorizationRelation(SemanticId),
    UnknownAuthorizationColumn {
        relation: SemanticId,
        column: SemanticId,
    },
    AccessRoleCycle(SemanticId),
    DuplicateField,
    UnknownFieldForRule(SemanticId),
    UnknownRelationForRule(SemanticId),
    UnknownRelationColumnForRule {
        relation: SemanticId,
        column: SemanticId,
    },
    RelationColumnIdentityArityMismatch,
    DuplicateRelationColumnIdentity,
    FieldRuleTypeMismatch,
    InvalidFieldRuleBounds,
    InvalidExactOrderStatistic,
    EntityRuleTypeMismatch,
    DuplicateRelation,
    DuplicateOwnedRelationship,
    UnknownOwnedRelationshipRelation(SemanticId),
    UnknownOwnedRelationshipTarget(SemanticId),
    DuplicateStructuralEquivalence,
    DuplicateStructuralOrdering,
    RelationEquivalenceArityMismatch,
    SubtypeCycle {
        subtype: SemanticId,
        supertype: SemanticId,
    },
    InvalidType(TypeError),
}
