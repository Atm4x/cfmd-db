use std::collections::{BTreeMap, BTreeSet};

use crate::{
    EquivalenceId, FieldId, OrderingId, RelationColumnId, RelationId, TypeId, VariantTagId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Unit,
    Bool,
    I64,
    F64,
    Text,
    LiveEntityRef(TypeId),
    HistoricalEntityRef(TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextPattern {
    Never,
    Empty,
    Literal(String),
    AnyScalar,
    Concat(Vec<Self>),
    Alternate(Vec<Self>),
    ZeroOrMore(Box<Self>),
}

impl TextPattern {
    #[must_use]
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }
    #[must_use]
    pub fn concat(parts: impl IntoIterator<Item = Self>) -> Self {
        Self::Concat(parts.into_iter().collect())
    }
    #[must_use]
    pub fn alternate(parts: impl IntoIterator<Item = Self>) -> Self {
        Self::Alternate(parts.into_iter().collect())
    }
    #[must_use]
    pub fn zero_or_more(pattern: Self) -> Self {
        Self::ZeroOrMore(Box::new(pattern))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleValueExpr {
    Input,
    Field(FieldId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticRuleExpr {
    True,
    False,
    And(Vec<Self>),
    Or(Vec<Self>),
    Not(Box<Self>),
    I64Range {
        value: RuleValueExpr,
        min: Option<i64>,
        max: Option<i64>,
    },
    TextLength {
        value: RuleValueExpr,
        min: usize,
        max: Option<usize>,
    },
    TextOneOf {
        value: RuleValueExpr,
        allowed: BTreeSet<String>,
    },
    TextMatches {
        value: RuleValueExpr,
        pattern: TextPattern,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldRule {
    I64Range { min: Option<i64>, max: Option<i64> },
    TextLength { min: usize, max: Option<usize> },
    TextOneOf(BTreeSet<String>),
    TextMatches(TextPattern),
    Expr(SemanticRuleExpr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Type {
    Scalar(ScalarType),
    Product(BTreeMap<FieldId, Self>),
    Sum(BTreeMap<VariantTagId, Self>),
    Option(Box<Self>),
    Set {
        element: Box<Self>,
        equivalence: EquivalenceId,
    },
    Bag {
        element: Box<Self>,
        equivalence: EquivalenceId,
    },
    Seq(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
        key_equivalence: EquivalenceId,
    },
    Var(u32),
    Recursive {
        binder: u32,
        body: Box<Self>,
    },
}

impl Type {
    #[must_use]
    pub const fn unit() -> Self {
        Self::Scalar(ScalarType::Unit)
    }
    #[must_use]
    pub const fn bool() -> Self {
        Self::Scalar(ScalarType::Bool)
    }
    #[must_use]
    pub const fn i64() -> Self {
        Self::Scalar(ScalarType::I64)
    }
    #[must_use]
    pub const fn f64() -> Self {
        Self::Scalar(ScalarType::F64)
    }
    #[must_use]
    pub const fn text() -> Self {
        Self::Scalar(ScalarType::Text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveEquivalence {
    UnitExact,
    BoolExact,
    I64Exact,
    F64Bitwise,
    TextExact,
    TextAsciiCaseInsensitive,
    LiveEntityIdExact(TypeId),
    HistoricalEntityIdExact(TypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralEquivalence {
    Option { inner: EquivalenceId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveOrdering {
    UnitExact,
    BoolAscending,
    I64Ascending,
    F64Total,
    TextBinary,
    TextAsciiCaseInsensitive,
    TextAsciiCaseInsensitiveThenBinary,
    LiveEntityIdAscending(TypeId),
    HistoricalEntityIdAscending(TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationSemantics {
    Bag {
        column_equivalences: Vec<EquivalenceId>,
    },
    Set {
        column_equivalences: Vec<EquivalenceId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationSchema {
    id: RelationId,
    column_ids: Vec<RelationColumnId>,
    columns: Vec<Type>,
    semantics: RelationSemantics,
}

impl RelationSchema {
    #[must_use]
    pub fn bag(
        id: RelationId,
        columns: impl Into<Vec<Type>>,
        column_equivalences: impl Into<Vec<EquivalenceId>>,
    ) -> Self {
        let columns = columns.into();
        let column_ids = (0..columns.len())
            .map(|ordinal| RelationColumnId::new((ordinal as u128) + 1))
            .collect();
        Self {
            id,
            column_ids,
            columns,
            semantics: RelationSemantics::Bag {
                column_equivalences: column_equivalences.into(),
            },
        }
    }

    #[must_use]
    pub fn set(
        id: RelationId,
        columns: impl Into<Vec<Type>>,
        column_equivalences: impl Into<Vec<EquivalenceId>>,
    ) -> Self {
        let columns = columns.into();
        let column_ids = (0..columns.len())
            .map(|ordinal| RelationColumnId::new((ordinal as u128) + 1))
            .collect();
        Self {
            id,
            column_ids,
            columns,
            semantics: RelationSemantics::Set {
                column_equivalences: column_equivalences.into(),
            },
        }
    }

    #[must_use]
    pub fn bag_with_column_ids(
        id: RelationId,
        columns: impl Into<Vec<(RelationColumnId, Type)>>,
        column_equivalences: impl Into<Vec<EquivalenceId>>,
    ) -> Self {
        let columns = columns.into();
        Self {
            id,
            column_ids: columns.iter().map(|(id, _)| *id).collect(),
            columns: columns.into_iter().map(|(_, ty)| ty).collect(),
            semantics: RelationSemantics::Bag {
                column_equivalences: column_equivalences.into(),
            },
        }
    }

    #[must_use]
    pub fn set_with_column_ids(
        id: RelationId,
        columns: impl Into<Vec<(RelationColumnId, Type)>>,
        column_equivalences: impl Into<Vec<EquivalenceId>>,
    ) -> Self {
        let columns = columns.into();
        Self {
            id,
            column_ids: columns.iter().map(|(id, _)| *id).collect(),
            columns: columns.into_iter().map(|(_, ty)| ty).collect(),
            semantics: RelationSemantics::Set {
                column_equivalences: column_equivalences.into(),
            },
        }
    }

    #[must_use]
    pub const fn id(&self) -> RelationId {
        self.id
    }
    #[must_use]
    pub fn column_ids(&self) -> &[RelationColumnId] {
        &self.column_ids
    }
    #[must_use]
    pub fn columns(&self) -> &[Type] {
        &self.columns
    }
    #[must_use]
    pub const fn semantics(&self) -> &RelationSemantics {
        &self.semantics
    }
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub(crate) revision: u64,
    pub(crate) environment_revision: u64,
    pub(crate) equivalences: BTreeMap<EquivalenceId, PrimitiveEquivalence>,
    pub(crate) structural_equivalences: BTreeMap<EquivalenceId, StructuralEquivalence>,
    pub(crate) orderings: BTreeMap<OrderingId, PrimitiveOrdering>,
    pub(crate) relations: BTreeMap<RelationId, RelationSchema>,
    pub(crate) entity_fields: BTreeMap<FieldId, (TypeId, Type)>,
    pub(crate) field_rules: BTreeMap<FieldId, Vec<FieldRule>>,
    pub(crate) relation_column_rules: BTreeMap<(RelationId, usize), Vec<FieldRule>>,
    pub(crate) entity_rules: BTreeMap<TypeId, Vec<SemanticRuleExpr>>,
}

impl Schema {
    #[must_use]
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::new()
    }

    pub fn relations(&self) -> impl Iterator<Item = &RelationSchema> {
        self.relations.values()
    }
}

#[derive(Debug, Clone)]
pub struct SchemaBuilder {
    schema_revision: u64,
    environment_revision: u64,
    equivalences: BTreeMap<EquivalenceId, PrimitiveEquivalence>,
    structural_equivalences: BTreeMap<EquivalenceId, StructuralEquivalence>,
    orderings: BTreeMap<OrderingId, PrimitiveOrdering>,
    relations: BTreeMap<RelationId, RelationSchema>,
    entity_types: BTreeSet<TypeId>,
    entity_fields: BTreeMap<FieldId, (TypeId, Type)>,
    field_rules: BTreeMap<FieldId, Vec<FieldRule>>,
    relation_column_rules: BTreeMap<(RelationId, usize), Vec<FieldRule>>,
    entity_rules: BTreeMap<TypeId, Vec<SemanticRuleExpr>>,
    duplicates: BTreeSet<u128>,
    invalid_schema: Vec<String>,
    required_relations: BTreeMap<RelationId, String>,
}

impl Default for SchemaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_revision: 1,
            environment_revision: 1,
            equivalences: BTreeMap::from([(
                crate::object::count_equivalence_id(),
                PrimitiveEquivalence::I64Exact,
            )]),
            structural_equivalences: BTreeMap::new(),
            orderings: BTreeMap::from([
                (
                    crate::object::count_ordering_id(),
                    PrimitiveOrdering::I64Ascending,
                ),
                (
                    crate::object::exact_f64_sum_ordering_id(),
                    PrimitiveOrdering::F64Total,
                ),
            ]),
            relations: BTreeMap::new(),
            entity_types: BTreeSet::new(),
            entity_fields: BTreeMap::new(),
            field_rules: BTreeMap::new(),
            relation_column_rules: BTreeMap::new(),
            entity_rules: BTreeMap::new(),
            duplicates: BTreeSet::new(),
            invalid_schema: Vec::new(),
            required_relations: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn object<E: crate::Object>(self) -> Self {
        crate::object::register_object::<E>(self)
    }

    #[must_use]
    pub const fn revisions(mut self, schema_revision: u64, environment_revision: u64) -> Self {
        self.schema_revision = schema_revision;
        self.environment_revision = environment_revision;
        self
    }

    #[must_use]
    pub fn equivalence(mut self, id: EquivalenceId, module: PrimitiveEquivalence) -> Self {
        if self.equivalences.insert(id, module).is_some() {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[must_use]
    pub fn structural_equivalence(
        mut self,
        id: EquivalenceId,
        definition: StructuralEquivalence,
    ) -> Self {
        if self
            .structural_equivalences
            .insert(id, definition)
            .is_some()
        {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[must_use]
    pub fn ordering(mut self, id: OrderingId, module: PrimitiveOrdering) -> Self {
        if self.orderings.insert(id, module).is_some() {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[must_use]
    pub fn relation(mut self, relation: RelationSchema) -> Self {
        let id = relation.id();
        if self.relations.insert(id, relation).is_some() {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[must_use]
    pub fn field_rule(mut self, field: FieldId, rule: FieldRule) -> Self {
        self.field_rules.entry(field).or_default().push(rule);
        self
    }

    #[must_use]
    pub fn entity_rule(mut self, owner: TypeId, rule: SemanticRuleExpr) -> Self {
        self.entity_rules.entry(owner).or_default().push(rule);
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __relation_column_rule(
        mut self,
        relation: RelationId,
        column: usize,
        rule: FieldRule,
    ) -> Self {
        self.relation_column_rules
            .entry((relation, column))
            .or_default()
            .push(rule);
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __entity_type(mut self, id: TypeId) -> Self {
        if !self.entity_types.insert(id) {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __entity_field(mut self, id: FieldId, owner: TypeId, ty: Type) -> Self {
        if self.entity_fields.insert(id, (owner, ty)).is_some() {
            self.duplicates.insert(id.raw());
        }
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __invalid_schema(mut self, message: String) -> Self {
        self.invalid_schema.push(message);
        self
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __require_relation(mut self, relation: RelationId, message: String) -> Self {
        self.required_relations.entry(relation).or_insert(message);
        self
    }

    pub fn build(self) -> crate::Result<Schema> {
        if let Some(message) = self.invalid_schema.first() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                message.clone(),
            ));
        }
        for (relation, message) in &self.required_relations {
            if !self.relations.contains_key(relation) {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    message.clone(),
                ));
            }
        }
        for field in self.field_rules.keys() {
            if !self.entity_fields.contains_key(field) {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    format!("field rule references unknown field {}", field.raw()),
                ));
            }
        }
        for owner in self.entity_rules.keys() {
            if !self.entity_types.contains(owner) {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    format!("entity rule references unknown entity type {}", owner.raw()),
                ));
            }
        }
        if let Some(id) = self.duplicates.first() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!("duplicate semantic id {id}"),
            ));
        }
        let mut seen = BTreeSet::new();
        for raw in self
            .equivalences
            .keys()
            .map(|id| id.raw())
            .chain(self.structural_equivalences.keys().map(|id| id.raw()))
            .chain(self.orderings.keys().map(|id| id.raw()))
            .chain(self.relations.keys().map(|id| id.raw()))
            .chain(self.entity_types.iter().map(|id| id.raw()))
            .chain(self.entity_fields.keys().map(|id| id.raw()))
        {
            if !seen.insert(raw) {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    format!("semantic id {raw} is reused across schema namespaces"),
                ));
            }
        }
        Ok(Schema {
            revision: self.schema_revision,
            environment_revision: self.environment_revision,
            equivalences: self.equivalences,
            structural_equivalences: self.structural_equivalences,
            orderings: self.orderings,
            relations: self.relations,
            entity_fields: self.entity_fields,
            field_rules: self.field_rules,
            relation_column_rules: self.relation_column_rules,
            entity_rules: self.entity_rules,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaView {
    revision: u64,
    relations: Vec<RelationSchema>,
}

impl SchemaView {
    pub(crate) fn from_kernel(schema: &kernel_schema::Schema) -> Self {
        Self {
            revision: schema.revision.raw(),
            relations: schema
                .relations()
                .map(|relation| relation_from_kernel(schema, relation))
                .collect(),
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn relations(&self) -> &[RelationSchema] {
        &self.relations
    }
    #[must_use]
    pub fn relation(&self, id: RelationId) -> Option<&RelationSchema> {
        self.relations.iter().find(|relation| relation.id == id)
    }
}

pub(crate) fn text_pattern_to_kernel(pattern: TextPattern) -> kernel_schema::TextPattern {
    match pattern {
        TextPattern::Never => kernel_schema::TextPattern::Never,
        TextPattern::Empty => kernel_schema::TextPattern::Empty,
        TextPattern::Literal(value) => kernel_schema::TextPattern::Literal(value),
        TextPattern::AnyScalar => kernel_schema::TextPattern::AnyScalar,
        TextPattern::Concat(parts) => kernel_schema::TextPattern::Concat(
            parts.into_iter().map(text_pattern_to_kernel).collect(),
        ),
        TextPattern::Alternate(parts) => kernel_schema::TextPattern::Alternate(
            parts.into_iter().map(text_pattern_to_kernel).collect(),
        ),
        TextPattern::ZeroOrMore(inner) => {
            kernel_schema::TextPattern::ZeroOrMore(Box::new(text_pattern_to_kernel(*inner)))
        }
    }
}

pub(crate) fn semantic_rule_to_kernel(rule: SemanticRuleExpr) -> kernel_schema::SemanticRuleExpr {
    match rule {
        SemanticRuleExpr::True => kernel_schema::SemanticRuleExpr::True,
        SemanticRuleExpr::False => kernel_schema::SemanticRuleExpr::False,
        SemanticRuleExpr::And(rules) => kernel_schema::SemanticRuleExpr::And(
            rules.into_iter().map(semantic_rule_to_kernel).collect(),
        ),
        SemanticRuleExpr::Or(rules) => kernel_schema::SemanticRuleExpr::Or(
            rules.into_iter().map(semantic_rule_to_kernel).collect(),
        ),
        SemanticRuleExpr::Not(rule) => {
            kernel_schema::SemanticRuleExpr::Not(Box::new(semantic_rule_to_kernel(*rule)))
        }
        SemanticRuleExpr::I64Range { value, min, max } => {
            kernel_schema::SemanticRuleExpr::I64Range {
                value: rule_value_to_kernel(&value),
                min,
                max,
            }
        }
        SemanticRuleExpr::TextLength { value, min, max } => {
            kernel_schema::SemanticRuleExpr::TextLength {
                value: rule_value_to_kernel(&value),
                min,
                max,
            }
        }
        SemanticRuleExpr::TextOneOf { value, allowed } => {
            kernel_schema::SemanticRuleExpr::TextOneOf {
                value: rule_value_to_kernel(&value),
                allowed,
            }
        }
        SemanticRuleExpr::TextMatches { value, pattern } => {
            kernel_schema::SemanticRuleExpr::TextMatches {
                value: rule_value_to_kernel(&value),
                pattern: text_pattern_to_kernel(pattern),
            }
        }
    }
}

fn rule_value_to_kernel(value: &RuleValueExpr) -> kernel_schema::RuleValueExpr {
    match value {
        RuleValueExpr::Input => kernel_schema::RuleValueExpr::Input,
        RuleValueExpr::Field(field) => kernel_schema::RuleValueExpr::Field((*field).into()),
    }
}

pub(crate) fn type_to_kernel(value: &Type) -> kernel_schema::TypeExpr {
    match value {
        Type::Scalar(value) => kernel_schema::TypeExpr::Scalar(match value {
            ScalarType::Unit => kernel_schema::ScalarType::Unit,
            ScalarType::Bool => kernel_schema::ScalarType::Bool,
            ScalarType::I64 => kernel_schema::ScalarType::I64,
            ScalarType::F64 => kernel_schema::ScalarType::F64,
            ScalarType::Text => kernel_schema::ScalarType::Text,
            ScalarType::LiveEntityRef(id) => kernel_schema::ScalarType::LiveEntityRef((*id).into()),
            ScalarType::HistoricalEntityRef(id) => {
                kernel_schema::ScalarType::HistoricalEntityId((*id).into())
            }
        }),
        Type::Product(fields) => kernel_schema::TypeExpr::Product(
            fields
                .iter()
                .map(|(id, value)| ((*id).into(), type_to_kernel(value)))
                .collect(),
        ),
        Type::Sum(variants) => kernel_schema::TypeExpr::Sum(
            variants
                .iter()
                .map(|(id, value)| ((*id).into(), type_to_kernel(value)))
                .collect(),
        ),
        Type::Option(value) => kernel_schema::TypeExpr::Option(Box::new(type_to_kernel(value))),
        Type::Set {
            element,
            equivalence,
        } => kernel_schema::TypeExpr::Set {
            element: Box::new(type_to_kernel(element)),
            equivalence: (*equivalence).into(),
        },
        Type::Bag {
            element,
            equivalence,
        } => kernel_schema::TypeExpr::Bag {
            element: Box::new(type_to_kernel(element)),
            equivalence: (*equivalence).into(),
        },
        Type::Seq(value) => kernel_schema::TypeExpr::Seq(Box::new(type_to_kernel(value))),
        Type::Map {
            key,
            value,
            key_equivalence,
        } => kernel_schema::TypeExpr::Map {
            key: Box::new(type_to_kernel(key)),
            value: Box::new(type_to_kernel(value)),
            key_equivalence: (*key_equivalence).into(),
        },
        Type::Var(var) => kernel_schema::TypeExpr::Var(kernel_schema::TypeVar(*var)),
        Type::Recursive { binder, body } => kernel_schema::TypeExpr::Mu {
            binder: kernel_schema::TypeVar(*binder),
            body: Box::new(type_to_kernel(body)),
        },
    }
}

fn type_from_kernel(value: &kernel_schema::TypeExpr) -> Type {
    match value {
        kernel_schema::TypeExpr::Scalar(value) => Type::Scalar(match value {
            kernel_schema::ScalarType::Unit => ScalarType::Unit,
            kernel_schema::ScalarType::Bool => ScalarType::Bool,
            kernel_schema::ScalarType::I64 => ScalarType::I64,
            kernel_schema::ScalarType::F64 => ScalarType::F64,
            kernel_schema::ScalarType::Text => ScalarType::Text,
            kernel_schema::ScalarType::LiveEntityRef(id) => {
                ScalarType::LiveEntityRef(TypeId::new(id.raw()))
            }
            kernel_schema::ScalarType::HistoricalEntityId(id) => {
                ScalarType::HistoricalEntityRef(TypeId::new(id.raw()))
            }
        }),
        kernel_schema::TypeExpr::Product(fields) => Type::Product(
            fields
                .iter()
                .map(|(id, value)| (FieldId::new(id.raw()), type_from_kernel(value)))
                .collect(),
        ),
        kernel_schema::TypeExpr::Sum(variants) => Type::Sum(
            variants
                .iter()
                .map(|(id, value)| (VariantTagId::new(id.raw()), type_from_kernel(value)))
                .collect(),
        ),
        kernel_schema::TypeExpr::Option(value) => Type::Option(Box::new(type_from_kernel(value))),
        kernel_schema::TypeExpr::Set {
            element,
            equivalence,
        } => Type::Set {
            element: Box::new(type_from_kernel(element)),
            equivalence: EquivalenceId::new(equivalence.raw()),
        },
        kernel_schema::TypeExpr::Bag {
            element,
            equivalence,
        } => Type::Bag {
            element: Box::new(type_from_kernel(element)),
            equivalence: EquivalenceId::new(equivalence.raw()),
        },
        kernel_schema::TypeExpr::Seq(value) => Type::Seq(Box::new(type_from_kernel(value))),
        kernel_schema::TypeExpr::Map {
            key,
            value,
            key_equivalence,
        } => Type::Map {
            key: Box::new(type_from_kernel(key)),
            value: Box::new(type_from_kernel(value)),
            key_equivalence: EquivalenceId::new(key_equivalence.raw()),
        },
        kernel_schema::TypeExpr::Var(var) => Type::Var(var.0),
        kernel_schema::TypeExpr::Mu { binder, body } => Type::Recursive {
            binder: binder.0,
            body: Box::new(type_from_kernel(body)),
        },
    }
}

fn relation_from_kernel(
    schema: &kernel_schema::Schema,
    value: &kernel_schema::RelationDef,
) -> RelationSchema {
    let semantics = match &value.semantics {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => RelationSemantics::Bag {
            column_equivalences: column_equivalences
                .iter()
                .map(|id| EquivalenceId::new(id.raw()))
                .collect(),
        },
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationSemantics::Set {
            column_equivalences: column_equivalences
                .iter()
                .map(|id| EquivalenceId::new(id.raw()))
                .collect(),
        },
    };
    RelationSchema {
        id: RelationId::new(value.id.raw()),
        column_ids: schema
            .relation_column_ids(value.id)
            .unwrap_or_default()
            .iter()
            .map(|id| RelationColumnId::new(id.raw()))
            .collect(),
        columns: value.columns.iter().map(type_from_kernel).collect(),
        semantics,
    }
}
