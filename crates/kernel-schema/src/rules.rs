use std::collections::BTreeSet;

use kernel_types::SemanticId;

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
