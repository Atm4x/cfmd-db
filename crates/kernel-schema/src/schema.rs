use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{SchemaRevisionId, SemanticId};

use crate::{
    CapabilityDef, FieldDef, RelationDef, RelationSemantics, StructuralEquivalenceDef,
    StructuralOrderingDef, SubtypeClosure, Symbol, TypeError, TypeExpr,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub revision: SchemaRevisionId,
    symbols: BTreeMap<SemanticId, Symbol>,
    types: BTreeMap<SemanticId, TypeExpr>,
    capabilities: BTreeMap<SemanticId, CapabilityDef>,
    fields: BTreeMap<SemanticId, FieldDef>,
    relations: BTreeMap<SemanticId, RelationDef>,
    structural_equivalences: BTreeMap<SemanticId, StructuralEquivalenceDef>,
    structural_orderings: BTreeMap<SemanticId, StructuralOrderingDef>,
    inclusions: BTreeSet<(SemanticId, SemanticId)>,
    subtype_closure: SubtypeClosure,
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

    pub fn define_relation(&mut self, relation: RelationDef) -> Result<(), SchemaError> {
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
        if self.relations.insert(relation.id, relation).is_some() {
            return Err(SchemaError::DuplicateRelation);
        }
        Ok(())
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
