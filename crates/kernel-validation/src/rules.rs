use kernel_model::Value;
use std::collections::{BTreeMap, BTreeSet};

use kernel_schema::{FieldRule, RuleValueExpr, SemanticContext, SemanticRuleExpr, TextPattern};
use kernel_types::SemanticId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEvaluationError {
    TypeMismatch,
    MissingValue,
}

#[derive(Debug)]
pub enum CompiledSemanticRuleExpr {
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
        pattern: CompiledTextPattern,
    },
}

impl CompiledSemanticRuleExpr {
    #[must_use]
    pub fn compile(rule: &SemanticRuleExpr) -> Self {
        match rule {
            SemanticRuleExpr::True => Self::True,
            SemanticRuleExpr::False => Self::False,
            SemanticRuleExpr::And(rules) => Self::And(rules.iter().map(Self::compile).collect()),
            SemanticRuleExpr::Or(rules) => Self::Or(rules.iter().map(Self::compile).collect()),
            SemanticRuleExpr::Not(rule) => Self::Not(Box::new(Self::compile(rule))),
            SemanticRuleExpr::I64Range { value, min, max } => Self::I64Range {
                value: value.clone(),
                min: *min,
                max: *max,
            },
            SemanticRuleExpr::TextLength { value, min, max } => Self::TextLength {
                value: value.clone(),
                min: *min,
                max: *max,
            },
            SemanticRuleExpr::TextOneOf { value, allowed } => Self::TextOneOf {
                value: value.clone(),
                allowed: allowed.clone(),
            },
            SemanticRuleExpr::TextMatches { value, pattern } => Self::TextMatches {
                value: value.clone(),
                pattern: CompiledTextPattern::compile(pattern),
            },
        }
    }

    pub fn matches<'a>(
        &self,
        resolve: &impl Fn(&RuleValueExpr) -> Option<&'a Value>,
    ) -> Result<bool, RuleEvaluationError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::And(rules) => {
                for rule in rules {
                    if !rule.matches(resolve)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(rules) => {
                for rule in rules {
                    if rule.matches(resolve)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(rule) => Ok(!rule.matches(resolve)?),
            Self::I64Range { value, min, max } => {
                let Value::I64(value) = resolve(value).ok_or(RuleEvaluationError::MissingValue)?
                else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                Ok(min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max))
            }
            Self::TextLength { value, min, max } => {
                let Value::Text(value) = resolve(value).ok_or(RuleEvaluationError::MissingValue)?
                else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                let len = value.chars().count();
                Ok(len >= *min && max.is_none_or(|max| len <= max))
            }
            Self::TextOneOf { value, allowed } => {
                let Value::Text(value) = resolve(value).ok_or(RuleEvaluationError::MissingValue)?
                else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                Ok(allowed.contains(value))
            }
            Self::TextMatches { value, pattern } => {
                let Value::Text(value) = resolve(value).ok_or(RuleEvaluationError::MissingValue)?
                else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                Ok(pattern.is_match(value))
            }
        }
    }
}

#[derive(Debug)]
pub struct CompiledFieldRule(CompiledSemanticRuleExpr);

impl CompiledFieldRule {
    #[must_use]
    pub fn compile(rule: &FieldRule) -> Self {
        Self(CompiledSemanticRuleExpr::compile(&rule.expression()))
    }

    pub fn matches(&self, value: &Value) -> Result<bool, RuleEvaluationError> {
        self.0.matches(&|coordinate| match coordinate {
            RuleValueExpr::Input => Some(value),
            RuleValueExpr::Field(_) => None,
        })
    }
}

#[derive(Debug, Default)]
pub struct CompiledRulePlan {
    fields: BTreeMap<SemanticId, Vec<CompiledFieldRule>>,
    relation_columns: BTreeMap<(SemanticId, SemanticId), Vec<CompiledFieldRule>>,
    entities: BTreeMap<SemanticId, Vec<CompiledSemanticRuleExpr>>,
}

impl CompiledRulePlan {
    #[must_use]
    pub fn compile(context: &SemanticContext) -> Self {
        let mut plan = Self::default();
        for (field, rule) in context.schema.all_field_rules() {
            plan.fields
                .entry(field)
                .or_default()
                .push(CompiledFieldRule::compile(rule));
        }
        for (target, rule) in context.schema.all_relation_column_rules() {
            plan.relation_columns
                .entry(target)
                .or_default()
                .push(CompiledFieldRule::compile(rule));
        }
        for (owner, rule) in context.schema.all_entity_rules() {
            plan.entities
                .entry(owner)
                .or_default()
                .push(CompiledSemanticRuleExpr::compile(rule));
        }
        plan
    }

    #[must_use]
    pub fn field_rules(&self, field: SemanticId) -> &[CompiledFieldRule] {
        self.fields
            .get(&field)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn relation_column_rules(
        &self,
        context: &SemanticContext,
        relation: SemanticId,
        ordinal: usize,
    ) -> &[CompiledFieldRule] {
        let Some(column) = context.schema.relation_column_id(relation, ordinal) else {
            return &[];
        };
        self.relation_columns
            .get(&(relation, column))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn entity_rules(&self, owner: SemanticId) -> &[CompiledSemanticRuleExpr] {
        self.entities
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn entity_rule_owners(&self) -> impl Iterator<Item = SemanticId> + '_ {
        self.entities.keys().copied()
    }
}

pub fn field_rule_matches(rule: &FieldRule, value: &Value) -> Result<bool, RuleEvaluationError> {
    CompiledFieldRule::compile(rule).matches(value)
}

pub fn semantic_rule_matches(
    rule: &SemanticRuleExpr,
    input: &Value,
) -> Result<bool, RuleEvaluationError> {
    CompiledSemanticRuleExpr::compile(rule).matches(&|coordinate| match coordinate {
        RuleValueExpr::Input => Some(input),
        RuleValueExpr::Field(_) => None,
    })
}

pub fn relation_row_rule_matches(
    rule: &SemanticRuleExpr,
    relation: kernel_types::SemanticId,
    row: &[Value],
    context: &SemanticContext,
) -> Result<bool, RuleEvaluationError> {
    CompiledSemanticRuleExpr::compile(rule).matches(&|coordinate| match coordinate {
        RuleValueExpr::Input => None,
        RuleValueExpr::Field(field) => context
            .schema
            .relation_column_ordinal(relation, *field)
            .and_then(|column| row.get(column)),
    })
}

pub fn entity_rule_matches(
    rule: &SemanticRuleExpr,
    entity: kernel_types::EntityId,
    state: &kernel_model::DatabaseState,
) -> Result<bool, RuleEvaluationError> {
    CompiledSemanticRuleExpr::compile(rule).matches(&|coordinate| match coordinate {
        RuleValueExpr::Input => None,
        RuleValueExpr::Field(field) => state.model.fields.get(&(*field, entity)),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Epsilon(usize),
    Scalar(char, usize),
    AnyScalar(usize),
}

#[derive(Debug, Default)]
struct NfaState {
    edges: Vec<Edge>,
}

fn state(states: &mut Vec<NfaState>) -> usize {
    let id = states.len();
    states.push(NfaState::default());
    id
}

fn compile_pattern(pattern: &TextPattern, states: &mut Vec<NfaState>) -> (usize, usize) {
    match pattern {
        TextPattern::Never => (state(states), state(states)),
        TextPattern::Empty => {
            let start = state(states);
            let end = state(states);
            states[start].edges.push(Edge::Epsilon(end));
            (start, end)
        }
        TextPattern::Literal(literal) => {
            let start = state(states);
            let mut cursor = start;
            for scalar in literal.chars() {
                let next = state(states);
                states[cursor].edges.push(Edge::Scalar(scalar, next));
                cursor = next;
            }
            (start, cursor)
        }
        TextPattern::AnyScalar => {
            let start = state(states);
            let end = state(states);
            states[start].edges.push(Edge::AnyScalar(end));
            (start, end)
        }
        TextPattern::Concat(parts) => {
            if parts.is_empty() {
                return compile_pattern(&TextPattern::Empty, states);
            }
            let mut parts = parts.iter();
            let (start, mut end) = compile_pattern(parts.next().expect("non-empty concat"), states);
            for part in parts {
                let (next_start, next_end) = compile_pattern(part, states);
                states[end].edges.push(Edge::Epsilon(next_start));
                end = next_end;
            }
            (start, end)
        }
        TextPattern::Alternate(parts) => {
            if parts.is_empty() {
                return compile_pattern(&TextPattern::Never, states);
            }
            let start = state(states);
            let end = state(states);
            for part in parts {
                let (branch_start, branch_end) = compile_pattern(part, states);
                states[start].edges.push(Edge::Epsilon(branch_start));
                states[branch_end].edges.push(Edge::Epsilon(end));
            }
            (start, end)
        }
        TextPattern::ZeroOrMore(pattern) => {
            let start = state(states);
            let end = state(states);
            let (body_start, body_end) = compile_pattern(pattern, states);
            states[start].edges.push(Edge::Epsilon(end));
            states[start].edges.push(Edge::Epsilon(body_start));
            states[body_end].edges.push(Edge::Epsilon(body_start));
            states[body_end].edges.push(Edge::Epsilon(end));
            (start, end)
        }
    }
}

fn epsilon_closure_into(
    states: &[NfaState],
    seeds: &[usize],
    marks: &mut [u32],
    epoch: u32,
    out: &mut Vec<usize>,
    stack: &mut Vec<usize>,
) {
    out.clear();
    stack.clear();
    stack.extend_from_slice(seeds);
    while let Some(current) = stack.pop() {
        if marks[current] == epoch {
            continue;
        }
        marks[current] = epoch;
        out.push(current);
        for edge in &states[current].edges {
            if let Edge::Epsilon(next) = *edge {
                stack.push(next);
            }
        }
    }
}

#[derive(Debug)]
pub struct CompiledTextPattern {
    states: Vec<NfaState>,
    start: usize,
    accept: usize,
}

impl CompiledTextPattern {
    #[must_use]
    pub fn compile(pattern: &TextPattern) -> Self {
        let mut states = Vec::new();
        let (start, accept) = compile_pattern(pattern, &mut states);
        Self {
            states,
            start,
            accept,
        }
    }

    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        let mut marks = vec![0_u32; self.states.len()];
        let mut epoch = 1_u32;
        let mut active = Vec::with_capacity(self.states.len());
        let mut next_active = Vec::with_capacity(self.states.len());
        let mut seeds = Vec::with_capacity(self.states.len());
        let mut stack = Vec::with_capacity(self.states.len());
        epsilon_closure_into(
            &self.states,
            &[self.start],
            &mut marks,
            epoch,
            &mut active,
            &mut stack,
        );

        for scalar in text.chars() {
            seeds.clear();
            for &current in &active {
                for edge in &self.states[current].edges {
                    match *edge {
                        Edge::Scalar(expected, target) if expected == scalar => seeds.push(target),
                        Edge::AnyScalar(target) => seeds.push(target),
                        Edge::Epsilon(_) | Edge::Scalar(_, _) => {}
                    }
                }
            }
            if seeds.is_empty() {
                return false;
            }

            epoch = epoch.wrapping_add(1);
            if epoch == 0 {
                marks.fill(0);
                epoch = 1;
            }
            epsilon_closure_into(
                &self.states,
                &seeds,
                &mut marks,
                epoch,
                &mut next_active,
                &mut stack,
            );
            std::mem::swap(&mut active, &mut next_active);
        }

        active.contains(&self.accept)
    }
}

#[must_use]
pub fn text_pattern_matches(pattern: &TextPattern, text: &str) -> bool {
    CompiledTextPattern::compile(pattern).is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_pattern_algebra_matches_unicode_scalars() {
        let pattern = TextPattern::concat([
            TextPattern::literal("cfmd-"),
            TextPattern::zero_or_more(TextPattern::AnyScalar),
            TextPattern::literal("✓"),
        ]);
        assert!(text_pattern_matches(&pattern, "cfmd-fast✓"));
        assert!(text_pattern_matches(&pattern, "cfmd-✓"));
        assert!(!text_pattern_matches(&pattern, "cfmd-fast"));
    }

    #[test]
    fn alternate_and_star_are_regular_not_backtracking() {
        let ambiguous = TextPattern::zero_or_more(TextPattern::alternate([
            TextPattern::literal("a"),
            TextPattern::literal("aa"),
        ]));
        let hostile = "a".repeat(20_000);
        assert!(text_pattern_matches(&ambiguous, &hostile));
    }

    #[test]
    fn semantic_rule_composition_has_one_evaluator() {
        let rule = SemanticRuleExpr::And(vec![
            SemanticRuleExpr::TextLength {
                value: RuleValueExpr::Input,
                min: 3,
                max: Some(8),
            },
            SemanticRuleExpr::TextMatches {
                value: RuleValueExpr::Input,
                pattern: TextPattern::concat([
                    TextPattern::literal("db"),
                    TextPattern::zero_or_more(TextPattern::AnyScalar),
                ]),
            },
        ]);
        assert_eq!(
            semantic_rule_matches(&rule, &Value::Text("db42".into())),
            Ok(true)
        );
        assert_eq!(
            semantic_rule_matches(&rule, &Value::Text("xx42".into())),
            Ok(false)
        );
    }
}
