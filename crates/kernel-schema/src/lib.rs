use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId, SemanticRevision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Entity,
    Value,
    Field,
    Relation,
    Capability,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub id: SemanticId,
    pub kind: SymbolKind,
    pub presentation_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeVar(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarType {
    Unit,
    Bool,
    I64,
    F64,
    Text,
    LiveEntityRef(SemanticId),
    HistoricalEntityId(SemanticId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    Scalar(ScalarType),
    Product(BTreeMap<SemanticId, Self>),
    Sum(BTreeMap<SemanticId, Self>),
    Option(Box<Self>),
    Set {
        element: Box<Self>,
        equivalence: SemanticId,
    },
    Bag {
        element: Box<Self>,
        equivalence: SemanticId,
    },
    Seq(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
        key_equivalence: SemanticId,
    },
    Var(TypeVar),
    Mu {
        binder: TypeVar,
        body: Box<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    FreeVariable(TypeVar),
    UnguardedRecursion(TypeVar),
}

impl TypeExpr {
    pub fn validate(&self) -> Result<(), TypeError> {
        self.validate_inner(&BTreeSet::new(), &BTreeSet::new())
    }

    fn validate_inner(
        &self,
        bound: &BTreeSet<TypeVar>,
        guarded: &BTreeSet<TypeVar>,
    ) -> Result<(), TypeError> {
        match self {
            Self::Scalar(_) => Ok(()),
            Self::Var(var) => {
                if !bound.contains(var) {
                    return Err(TypeError::FreeVariable(*var));
                }
                if !guarded.contains(var) {
                    return Err(TypeError::UnguardedRecursion(*var));
                }
                Ok(())
            }
            Self::Mu { binder, body } => {
                let mut next_bound = bound.clone();
                next_bound.insert(*binder);
                body.validate_inner(&next_bound, guarded)
            }
            Self::Product(fields) | Self::Sum(fields) => fields
                .values()
                .try_for_each(|child| child.validate_under_constructor(bound, guarded)),
            Self::Option(child) | Self::Seq(child) => {
                child.validate_under_constructor(bound, guarded)
            }
            Self::Set { element, .. } | Self::Bag { element, .. } => {
                element.validate_under_constructor(bound, guarded)
            }
            Self::Map { key, value, .. } => {
                key.validate_under_constructor(bound, guarded)?;
                value.validate_under_constructor(bound, guarded)
            }
        }
    }

    fn collect_semantic_dependencies(&self, out: &mut BTreeSet<SemanticId>) {
        match self {
            Self::Scalar(_) | Self::Var(_) => {}
            Self::Product(fields) | Self::Sum(fields) => {
                for child in fields.values() {
                    child.collect_semantic_dependencies(out);
                }
            }
            Self::Option(child) | Self::Seq(child) | Self::Mu { body: child, .. } => {
                child.collect_semantic_dependencies(out);
            }
            Self::Set {
                element,
                equivalence,
            }
            | Self::Bag {
                element,
                equivalence,
            } => {
                out.insert(*equivalence);
                element.collect_semantic_dependencies(out);
            }
            Self::Map {
                key,
                value,
                key_equivalence,
            } => {
                out.insert(*key_equivalence);
                key.collect_semantic_dependencies(out);
                value.collect_semantic_dependencies(out);
            }
        }
    }

    fn validate_under_constructor(
        &self,
        bound: &BTreeSet<TypeVar>,
        guarded: &BTreeSet<TypeVar>,
    ) -> Result<(), TypeError> {
        let mut next_guarded = guarded.clone();
        next_guarded.extend(bound.iter().copied());
        self.validate_inner(bound, &next_guarded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDef {
    pub id: SemanticId,
    pub required_fields: BTreeMap<SemanticId, TypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    pub id: SemanticId,
    pub owner: SemanticId,
    pub value: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationSemantics {
    Set {
        column_equivalences: Vec<SemanticId>,
    },
    Bag {
        column_equivalences: Vec<SemanticId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDef {
    pub id: SemanticId,
    pub columns: Vec<TypeExpr>,
    pub semantics: RelationSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralEquivalenceDef {
    Mu {
        body: SemanticId,
    },
    Var {
        binder: SemanticId,
    },
    Product {
        fields: BTreeMap<SemanticId, SemanticId>,
    },
    Option {
        inner: SemanticId,
    },
    Sum {
        variants: BTreeMap<SemanticId, SemanticId>,
    },
    Set {
        element: SemanticId,
    },
    Bag {
        element: SemanticId,
    },
    Seq {
        element: SemanticId,
    },
    Map {
        key: SemanticId,
        value: SemanticId,
    },
}

/// Compositional semantic total-preorder definition for non-primitive values.
///
/// Product field order and Sum variant rank are explicit. Semantic ordering
/// must never inherit host map iteration order, enum discriminants, or
/// incidental `SemanticId` allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralOrderingDef {
    Mu {
        body: SemanticId,
    },
    Var {
        binder: SemanticId,
    },
    Product {
        fields: Vec<(SemanticId, SemanticId)>,
    },
    Option {
        inner: SemanticId,
        none_first: bool,
    },
    Sum {
        variants: Vec<(SemanticId, SemanticId)>,
    },
    Set {
        element: SemanticId,
    },
    Bag {
        element: SemanticId,
    },
    Seq {
        element: SemanticId,
    },
    Map {
        key: SemanticId,
        value: SemanticId,
    },
}

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
    inclusion_parents: BTreeMap<SemanticId, BTreeSet<SemanticId>>,
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
            inclusion_parents: BTreeMap::new(),
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
            self.inclusion_parents
                .entry(subtype)
                .or_default()
                .insert(supertype);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_subtype(&self, subtype: SemanticId, supertype: SemanticId) -> bool {
        if subtype == supertype {
            return true;
        }
        let mut frontier = vec![subtype];
        let mut seen = BTreeSet::new();
        while let Some(current) = frontier.pop() {
            if !seen.insert(current) {
                continue;
            }
            let Some(parents) = self.inclusion_parents.get(&current) else {
                continue;
            };
            for &parent in parents {
                if parent == supertype {
                    return true;
                }
                frontier.push(parent);
            }
        }
        false
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleDigest(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticEnvironment {
    pub revision: SemanticEnvId,
    modules: BTreeMap<SemanticId, ModuleDigest>,
}

impl SemanticEnvironment {
    #[must_use]
    pub fn new(revision: SemanticEnvId) -> Self {
        Self {
            revision,
            modules: BTreeMap::new(),
        }
    }

    pub fn pin_module(&mut self, module: SemanticId, digest: ModuleDigest) {
        self.modules.insert(module, digest);
    }

    #[must_use]
    pub fn definitionally_equivalent(&self, other: &Self) -> bool {
        self.modules == other.modules
    }

    #[must_use]
    pub fn module(&self, module: SemanticId) -> Option<ModuleDigest> {
        self.modules.get(&module).copied()
    }

    #[must_use]
    pub fn has_module(&self, module: SemanticId) -> bool {
        self.modules.contains_key(&module)
    }

    pub fn modules(&self) -> impl Iterator<Item = (SemanticId, ModuleDigest)> + '_ {
        self.modules.iter().map(|(&id, &digest)| (id, digest))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticContext {
    pub schema: Schema,
    pub environment: SemanticEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    MissingSemanticModule(SemanticId),
}

impl SemanticContext {
    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        SemanticRevision::new(self.schema.revision, self.environment.revision)
    }

    #[must_use]
    pub fn definitionally_equivalent(&self, other: &Self) -> bool {
        self.schema.definitionally_equivalent(&other.schema)
            && self
                .environment
                .definitionally_equivalent(&other.environment)
    }

    pub fn validate(&self) -> Result<(), ContextError> {
        for dependency in self.schema.semantic_dependencies() {
            if !self.environment.has_module(dependency) {
                return Err(ContextError::MissingSemanticModule(dependency));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_preserves_semantic_identity() {
        let id = SemanticId::new(41);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define(Symbol {
                id,
                kind: SymbolKind::Field,
                presentation_name: "name".into(),
            })
            .unwrap();

        schema.rename(id, "display_name").unwrap();

        let symbol = schema.symbol(id).unwrap();
        assert_eq!(symbol.id, id);
        assert_eq!(symbol.presentation_name, "display_name");
    }

    #[test]
    fn semantic_environment_is_explicitly_versioned() {
        let module = SemanticId::new(7);
        let digest = ModuleDigest([3; 32]);
        let mut env = SemanticEnvironment::new(SemanticEnvId::new(5));
        env.pin_module(module, digest);
        assert_eq!(env.module(module), Some(digest));
    }

    #[test]
    fn recursive_document_type_is_guarded_and_valid() {
        let x = TypeVar(0);
        let json = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (SemanticId::new(1), TypeExpr::Scalar(ScalarType::I64)),
                (
                    SemanticId::new(2),
                    TypeExpr::Seq(Box::new(TypeExpr::Var(x))),
                ),
            ]))),
        };
        assert_eq!(json.validate(), Ok(()));
    }

    #[test]
    fn naked_recursive_variable_is_rejected() {
        let x = TypeVar(0);
        let invalid = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Var(x)),
        };
        assert_eq!(invalid.validate(), Err(TypeError::UnguardedRecursion(x)));
    }

    #[test]
    fn free_type_variable_is_rejected() {
        let free = TypeExpr::Var(TypeVar(99));
        assert_eq!(free.validate(), Err(TypeError::FreeVariable(TypeVar(99))));
    }

    #[test]
    fn open_capability_is_not_closed_sum() {
        let capability_id = SemanticId::new(100);
        let field_id = SemanticId::new(101);
        let implementation = SemanticId::new(200);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability_id,
                required_fields: BTreeMap::from([(field_id, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(implementation, capability_id).unwrap();

        assert!(schema.capability(capability_id).is_some());
        assert!(schema.has_direct_inclusion(implementation, capability_id));
    }

    #[test]
    fn subtype_diamond_is_coherent_but_cycles_are_rejected() {
        let bottom = SemanticId::new(1);
        let left = SemanticId::new(2);
        let right = SemanticId::new(3);
        let top = SemanticId::new(4);
        let mut schema = Schema::new(SchemaRevisionId::new(1));

        schema.include(bottom, left).unwrap();
        schema.include(bottom, right).unwrap();
        schema.include(left, top).unwrap();
        schema.include(right, top).unwrap();

        assert!(schema.is_subtype(bottom, top));
        assert_eq!(
            schema.include(top, bottom),
            Err(SchemaError::SubtypeCycle {
                subtype: top,
                supertype: bottom,
            })
        );
    }

    #[test]
    fn context_requires_all_semantic_modules_referenced_by_schema() {
        let equality = SemanticId::new(700);
        let type_id = SemanticId::new(701);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_type(
                type_id,
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: equality,
                },
            )
            .unwrap();
        let mut context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        assert_eq!(
            context.validate(),
            Err(ContextError::MissingSemanticModule(equality))
        );
        context
            .environment
            .pin_module(equality, ModuleDigest([9; 32]));
        assert_eq!(context.validate(), Ok(()));
    }
    #[test]
    fn set_relation_requires_one_equivalence_per_column() {
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        assert_eq!(
            schema.define_relation(RelationDef {
                id: SemanticId::new(10),
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![SemanticId::new(20)],
                },
            }),
            Err(SchemaError::RelationEquivalenceArityMismatch)
        );
    }

    #[test]
    fn subtype_adjacency_matches_direct_relation_scan_reference() {
        fn reference(schema: &Schema, subtype: SemanticId, supertype: SemanticId) -> bool {
            if subtype == supertype {
                return true;
            }
            let mut frontier = vec![subtype];
            let mut seen = BTreeSet::new();
            while let Some(current) = frontier.pop() {
                if !seen.insert(current) {
                    continue;
                }
                for &(child, parent) in &schema.inclusions {
                    if child == current {
                        if parent == supertype {
                            return true;
                        }
                        frontier.push(parent);
                    }
                }
            }
            false
        }

        let ids = (0_u64..40)
            .map(|raw| SemanticId::new(u128::from(90_000_u64 + raw)))
            .collect::<Vec<_>>();
        let mut schema = Schema::new(SchemaRevisionId::new(90_000));
        for left in 0..ids.len() {
            for right in left + 1..ids.len() {
                if (left * 11 + right * 7) % 13 == 0 {
                    schema.include(ids[left], ids[right]).unwrap();
                    schema.include(ids[left], ids[right]).unwrap();
                }
            }
        }
        for &subtype in &ids {
            for &supertype in &ids {
                assert_eq!(
                    schema.is_subtype(subtype, supertype),
                    reference(&schema, subtype, supertype)
                );
            }
        }
    }
}
