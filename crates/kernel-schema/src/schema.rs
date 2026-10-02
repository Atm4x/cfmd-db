use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{SchemaRevisionId, SemanticId};

use crate::{
    CapabilityDef, FieldDef, FieldRule, RelationDef, RelationSemantics, RuleValueExpr,
    SemanticRuleExpr, SemanticRuleTypeError, StructuralEquivalenceDef, StructuralOrderingDef,
    SubtypeClosure, Symbol, TypeError, TypeExpr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub revision: SchemaRevisionId,
    symbols: BTreeMap<SemanticId, Symbol>,
    types: BTreeMap<SemanticId, TypeExpr>,
    capabilities: BTreeMap<SemanticId, CapabilityDef>,
    fields: BTreeMap<SemanticId, FieldDef>,
    field_rules: BTreeMap<SemanticId, Vec<FieldRule>>,
    relation_column_rules: BTreeMap<(SemanticId, SemanticId), Vec<FieldRule>>,
    entity_rules: BTreeMap<SemanticId, Vec<SemanticRuleExpr>>,
    relation_column_ids: BTreeMap<SemanticId, Vec<SemanticId>>,
    relations: BTreeMap<SemanticId, RelationDef>,
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

impl Schema {
    #[must_use]
    pub fn new(revision: SchemaRevisionId) -> Self {
        Self {
            revision,
            symbols: BTreeMap::new(),
            types: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            fields: BTreeMap::new(),
            field_rules: BTreeMap::new(),
            relation_column_rules: BTreeMap::new(),
            entity_rules: BTreeMap::new(),
            relation_column_ids: BTreeMap::new(),
            relations: BTreeMap::new(),
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
            && self.structural_equivalences == other.structural_equivalences
            && self.structural_orderings == other.structural_orderings
            && self.inclusions == other.inclusions
    }

    #[must_use]
    pub fn relation_transport_base_equivalent(&self, other: &Self) -> bool {
        self.types == other.types
            && self.capabilities == other.capabilities
            && self.fields == other.fields
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
            && self.fields == other.fields
            && self.field_rules == other.field_rules
            && self.relation_column_rules == other.relation_column_rules
            && self.relation_column_ids == other.relation_column_ids
            && self.entity_rules == other.entity_rules
            && self.relations == other.relations
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
    EntityRuleTypeMismatch,
    DuplicateRelation,
    DuplicateStructuralEquivalence,
    DuplicateStructuralOrdering,
    RelationEquivalenceArityMismatch,
    SubtypeCycle {
        subtype: SemanticId,
        supertype: SemanticId,
    },
    InvalidType(TypeError),
}
