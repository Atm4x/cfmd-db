use std::collections::BTreeSet;

use kernel_types::SemanticId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FiniteF64(u64);

impl FiniteF64 {
    #[must_use]
    pub fn new(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let normalized = if value == 0.0 { 0.0 } else { value };
        Some(Self(normalized.to_bits()))
    }

    #[must_use]
    pub fn from_bits(bits: u64) -> Option<Self> {
        Self::new(f64::from_bits(bits))
    }

    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// Database-wide semantic assertions whose truth depends on relation-level observations.
///
/// This is deliberately not a second relational query AST. Each variant is a compact persisted
/// law that validation lowers into the existing query algebra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRuleExpr {
    /// Exact COUNT over the semantic relation extent.
    RelationCardinality {
        relation: SemanticId,
        min: u64,
        max: Option<u64>,
    },
    /// At least one semantic row must satisfy `predicate`.
    RelationExists {
        relation: SemanticId,
        predicate: SemanticRuleExpr,
    },
    /// Every semantic row must satisfy `predicate` (vacuously true on an empty relation).
    RelationAll {
        relation: SemanticId,
        predicate: SemanticRuleExpr,
    },
    /// Exact sum of one finite-f64 relation column constrained by finite bounds.
    RelationExactF64SumRange {
        relation: SemanticId,
        column: SemanticId,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    },
}

use crate::{FieldRule, ScalarType, TypeExpr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleValueExpr {
    Input,
    Field(SemanticId),
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

impl FieldRule {
    #[must_use]
    pub fn expression(&self) -> SemanticRuleExpr {
        match self {
            Self::I64Range { min, max } => SemanticRuleExpr::I64Range {
                value: RuleValueExpr::Input,
                min: *min,
                max: *max,
            },
            Self::TextLength { min, max } => SemanticRuleExpr::TextLength {
                value: RuleValueExpr::Input,
                min: *min,
                max: *max,
            },
            Self::TextOneOf(allowed) => SemanticRuleExpr::TextOneOf {
                value: RuleValueExpr::Input,
                allowed: allowed.clone(),
            },
            Self::TextMatches(pattern) => SemanticRuleExpr::TextMatches {
                value: RuleValueExpr::Input,
                pattern: pattern.clone(),
            },
            Self::Expr(expression) => expression.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticRuleTypeError {
    TypeMismatch,
    InvalidBounds,
    UnknownField(SemanticId),
    FieldOutsideOwner {
        field: SemanticId,
        owner: SemanticId,
    },
}

impl SemanticRuleExpr {
    pub fn validate_for_input(&self, input: &TypeExpr) -> Result<(), SemanticRuleTypeError> {
        self.validate_values(&mut |value| match value {
            RuleValueExpr::Input => Ok(input.clone()),
            RuleValueExpr::Field(field) => Err(SemanticRuleTypeError::UnknownField(*field)),
        })
    }

    pub(crate) fn validate_values(
        &self,
        resolve: &mut impl FnMut(&RuleValueExpr) -> Result<TypeExpr, SemanticRuleTypeError>,
    ) -> Result<(), SemanticRuleTypeError> {
        match self {
            Self::True | Self::False => Ok(()),
            Self::And(rules) | Self::Or(rules) => {
                for rule in rules {
                    rule.validate_values(resolve)?;
                }
                Ok(())
            }
            Self::Not(rule) => rule.validate_values(resolve),
            Self::I64Range { value, min, max } => {
                require_type(&resolve(value)?, ScalarType::I64)?;
                if min.zip(*max).is_some_and(|(min, max)| min > max) {
                    return Err(SemanticRuleTypeError::InvalidBounds);
                }
                Ok(())
            }
            Self::TextLength { value, min, max } => {
                require_type(&resolve(value)?, ScalarType::Text)?;
                if max.is_some_and(|max| *min > max) {
                    return Err(SemanticRuleTypeError::InvalidBounds);
                }
                Ok(())
            }
            Self::TextOneOf { value, .. } | Self::TextMatches { value, .. } => {
                require_type(&resolve(value)?, ScalarType::Text)
            }
        }
    }
}

fn require_type(input: &TypeExpr, expected: ScalarType) -> Result<(), SemanticRuleTypeError> {
    if *input == TypeExpr::Scalar(expected) {
        Ok(())
    } else {
        Err(SemanticRuleTypeError::TypeMismatch)
    }
}
