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
/// This is deliberately not a second relational query AST. Each variant is a compact persisted exact measure law maintained by validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderedStatisticSelector {
    FromStart(u64),
    FromEnd(u64),
    LowerQuantile { numerator: u64, denominator: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactAggregateMeasureExpr {
    Count {
        relation: SemanticId,
        predicate: SemanticRuleExpr,
    },
    F64Sum {
        relation: SemanticId,
        column: SemanticId,
        predicate: SemanticRuleExpr,
    },
    OrderedStatistic {
        relation: SemanticId,
        column: SemanticId,
        predicate: SemanticRuleExpr,
        ordering: SemanticId,
        selector: OrderedStatisticSelector,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderedStatisticBound {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    Text(String),
    LiveEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
    HistoricalEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactAggregateRange {
    Count {
        min: u64,
        max: Option<u64>,
    },
    F64Sum {
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    },
    OrderedStatistic {
        min: Option<OrderedStatisticBound>,
        max: Option<OrderedStatisticBound>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactMeasureConstraint {
    Range {
        measure: ExactAggregateMeasureExpr,
        range: ExactAggregateRange,
    },
    Compare {
        left: ExactAggregateMeasureExpr,
        right: ExactAggregateMeasureExpr,
        comparison: RuleOrderComparison,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRuleExpr {
    /// Exact maintained measure constraint without a grouping domain.
    ///
    /// Validation compiles this persisted vocabulary into specialized count-range, exact-f64-sum
    /// range, or homogeneous measure-comparison witnesses before any row-delta maintenance.
    RelationExactMeasure { constraint: ExactMeasureConstraint },
    /// Exact measure law over one live Γ-keyed grouped domain.
    ///
    /// The persisted representation owns grouping exactly once. Validation compiles the typed
    /// constraint into a specialized count-range, f64-sum-range or homogeneous comparison hot
    /// path; no dynamic grouped-law dispatch survives into row-delta maintenance.
    RelationGroupedExactMeasure {
        group_columns: Vec<SemanticId>,
        group_equivalences: Vec<SemanticId>,
        constraint: ExactMeasureConstraint,
    },
}

use crate::{FieldRule, ScalarType, TypeExpr};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuleValueExpr {
    Input,
    Field(SemanticId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuleOrderComparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    Equivalent {
        left: RuleValueExpr,
        right: RuleValueExpr,
        equivalence: SemanticId,
    },
    Ordered {
        left: RuleValueExpr,
        right: RuleValueExpr,
        ordering: SemanticId,
        comparison: RuleOrderComparison,
    },
}

impl SemanticRuleExpr {
    #[must_use]
    pub fn negate(self) -> Self {
        Self::Not(Box::new(self))
    }
}

fn push_canonical_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u64::try_from(len).unwrap_or(u64::MAX).to_le_bytes());
}

fn push_canonical_text(out: &mut Vec<u8>, text: &str) {
    push_canonical_len(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

fn encode_canonical_rule_value(out: &mut Vec<u8>, value: &RuleValueExpr) {
    match value {
        RuleValueExpr::Input => out.push(0),
        RuleValueExpr::Field(field) => {
            out.push(1);
            out.extend_from_slice(&field.raw().to_le_bytes());
        }
    }
}

fn encode_canonical_text_pattern(out: &mut Vec<u8>, pattern: &TextPattern) {
    match pattern {
        TextPattern::Never => out.push(0),
        TextPattern::Empty => out.push(1),
        TextPattern::Literal(text) => {
            out.push(2);
            push_canonical_text(out, text);
        }
        TextPattern::AnyScalar => out.push(3),
        TextPattern::Concat(parts) => {
            out.push(4);
            push_canonical_len(out, parts.len());
            for part in parts {
                encode_canonical_text_pattern(out, part);
            }
        }
        TextPattern::Alternate(parts) => {
            out.push(5);
            let mut frames = parts
                .iter()
                .map(|part| {
                    let mut frame = Vec::new();
                    encode_canonical_text_pattern(&mut frame, part);
                    frame
                })
                .collect::<Vec<_>>();
            frames.sort();
            frames.dedup();
            push_canonical_len(out, frames.len());
            for frame in frames {
                push_canonical_len(out, frame.len());
                out.extend_from_slice(&frame);
            }
        }
        TextPattern::ZeroOrMore(pattern) => {
            out.push(6);
            encode_canonical_text_pattern(out, pattern);
        }
    }
}

fn encode_canonical_semantic_rule(out: &mut Vec<u8>, expression: &SemanticRuleExpr) {
    match expression {
        SemanticRuleExpr::True => out.push(0),
        SemanticRuleExpr::False => out.push(1),
        SemanticRuleExpr::And(rules) | SemanticRuleExpr::Or(rules) => {
            out.push(if matches!(expression, SemanticRuleExpr::And(_)) {
                2
            } else {
                3
            });
            let mut frames = rules
                .iter()
                .map(canonical_semantic_rule_bytes)
                .collect::<Vec<_>>();
            frames.sort();
            frames.dedup();
            push_canonical_len(out, frames.len());
            for frame in frames {
                push_canonical_len(out, frame.len());
                out.extend_from_slice(&frame);
            }
        }
        SemanticRuleExpr::Not(rule) => {
            out.push(4);
            encode_canonical_semantic_rule(out, rule);
        }
        SemanticRuleExpr::I64Range { value, min, max } => {
            out.push(5);
            encode_canonical_rule_value(out, value);
            match min {
                None => out.push(0),
                Some(value) => {
                    out.push(1);
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
            match max {
                None => out.push(0),
                Some(value) => {
                    out.push(1);
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        SemanticRuleExpr::TextLength { value, min, max } => {
            out.push(6);
            encode_canonical_rule_value(out, value);
            out.extend_from_slice(&u64::try_from(*min).unwrap_or(u64::MAX).to_le_bytes());
            match max {
                None => out.push(0),
                Some(value) => {
                    out.push(1);
                    out.extend_from_slice(&u64::try_from(*value).unwrap_or(u64::MAX).to_le_bytes());
                }
            }
        }
        SemanticRuleExpr::TextOneOf { value, allowed } => {
            out.push(7);
            encode_canonical_rule_value(out, value);
            push_canonical_len(out, allowed.len());
            for text in allowed {
                push_canonical_text(out, text);
            }
        }
        SemanticRuleExpr::TextMatches { value, pattern } => {
            out.push(8);
            encode_canonical_rule_value(out, value);
            encode_canonical_text_pattern(out, pattern);
        }
        SemanticRuleExpr::Equivalent {
            left,
            right,
            equivalence,
        } => {
            out.push(9);
            encode_canonical_rule_value(out, left);
            encode_canonical_rule_value(out, right);
            out.extend_from_slice(&equivalence.raw().to_le_bytes());
        }
        SemanticRuleExpr::Ordered {
            left,
            right,
            ordering,
            comparison,
        } => {
            out.push(10);
            encode_canonical_rule_value(out, left);
            encode_canonical_rule_value(out, right);
            out.extend_from_slice(&ordering.raw().to_le_bytes());
            out.push(match comparison {
                RuleOrderComparison::Less => 0,
                RuleOrderComparison::LessOrEqual => 1,
                RuleOrderComparison::Greater => 2,
                RuleOrderComparison::GreaterOrEqual => 3,
            });
        }
    }
}

/// Canonical semantic-identity frame for one deterministic rule expression.
///
/// This is not the checkpoint codec. It deliberately normalizes commutative/idempotent
/// `And`, `Or`, and pattern `Alternate` nodes so retry/intent identity depends on the
/// semantic predicate rather than frontend construction order.
#[must_use]
pub fn canonical_semantic_rule_bytes(expression: &SemanticRuleExpr) -> Vec<u8> {
    let mut out = Vec::new();
    encode_canonical_semantic_rule(&mut out, expression);
    out
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
            Self::Equivalent { left, right, .. } | Self::Ordered { left, right, .. } => {
                let left = resolve(left)?;
                let right = resolve(right)?;
                if left == right {
                    Ok(())
                } else {
                    Err(SemanticRuleTypeError::TypeMismatch)
                }
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
