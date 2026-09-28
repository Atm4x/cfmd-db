use std::collections::{BTreeMap, BTreeSet};

use kernel_types::SemanticId;

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

    pub(super) fn collect_semantic_dependencies(&self, out: &mut BTreeSet<SemanticId>) {
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
