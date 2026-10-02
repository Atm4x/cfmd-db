use std::collections::{BTreeMap, BTreeSet};

use kernel_types::SemanticId;

use crate::TypeExpr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDef {
    pub id: SemanticId,
    pub required_fields: BTreeMap<SemanticId, TypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldRule {
    I64Range { min: Option<i64>, max: Option<i64> },
    TextLength { min: usize, max: Option<usize> },
    TextOneOf(BTreeSet<String>),
    TextMatches(crate::TextPattern),
    Expr(crate::SemanticRuleExpr),
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
