use kernel_model::Value;
use std::collections::{BTreeMap, BTreeSet};

use kernel_schema::{
    ExactAggregateMeasureExpr, ExactAggregateRange, ExactMeasureConstraint, FieldRule, FiniteF64,
    ModelRuleExpr, OrderedStatisticBound, OrderedStatisticSelector, RuleOrderComparison,
    RuleValueExpr, SemanticContext, SemanticRuleExpr, TextPattern,
};
use kernel_types::SemanticId;

fn exact_aggregate_measure_relation(measure: &ExactAggregateMeasureExpr) -> SemanticId {
    match measure {
        ExactAggregateMeasureExpr::Count { relation, .. }
        | ExactAggregateMeasureExpr::F64Sum { relation, .. }
        | ExactAggregateMeasureExpr::OrderedStatistic { relation, .. } => *relation,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEvaluationError {
    TypeMismatch,
    MissingValue,
    Semantic,
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
            SemanticRuleExpr::Equivalent {
                left,
                right,
                equivalence,
            } => Self::Equivalent {
                left: left.clone(),
                right: right.clone(),
                equivalence: *equivalence,
            },
            SemanticRuleExpr::Ordered {
                left,
                right,
                ordering,
                comparison,
            } => Self::Ordered {
                left: left.clone(),
                right: right.clone(),
                ordering: *ordering,
                comparison: *comparison,
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
            Self::Equivalent { .. } | Self::Ordered { .. } => Err(RuleEvaluationError::Semantic),
        }
    }

    pub fn matches_semantic<'a>(
        &self,
        resolve: &impl Fn(&RuleValueExpr) -> Option<&'a Value>,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RuleEvaluationError> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::And(rules) => {
                for rule in rules {
                    if !rule.matches_semantic(resolve, context, registry)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(rules) => {
                for rule in rules {
                    if rule.matches_semantic(resolve, context, registry)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(rule) => Ok(!rule.matches_semantic(resolve, context, registry)?),
            Self::Equivalent {
                left,
                right,
                equivalence,
            } => {
                let left = resolve(left).ok_or(RuleEvaluationError::MissingValue)?;
                let right = resolve(right).ok_or(RuleEvaluationError::MissingValue)?;
                registry
                    .equivalent(context, *equivalence, left, right)
                    .map_err(|_| RuleEvaluationError::Semantic)
            }
            Self::Ordered {
                left,
                right,
                ordering,
                comparison,
            } => {
                let left = resolve(left).ok_or(RuleEvaluationError::MissingValue)?;
                let right = resolve(right).ok_or(RuleEvaluationError::MissingValue)?;
                let order = registry
                    .compare(context, *ordering, left, right)
                    .map_err(|_| RuleEvaluationError::Semantic)?;
                Ok(match comparison {
                    RuleOrderComparison::Less => order.is_lt(),
                    RuleOrderComparison::LessOrEqual => order.is_le(),
                    RuleOrderComparison::Greater => order.is_gt(),
                    RuleOrderComparison::GreaterOrEqual => order.is_ge(),
                })
            }
            _ => self.matches(resolve),
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

#[derive(Debug)]
pub struct CompiledRelationPredicate {
    expression: CompiledSemanticRuleExpr,
    columns: BTreeMap<SemanticId, usize>,
}

impl CompiledRelationPredicate {
    fn compile(relation: SemanticId, rule: &SemanticRuleExpr, context: &SemanticContext) -> Self {
        let mut fields = BTreeSet::new();
        collect_rule_fields(rule, &mut fields);
        let columns = fields
            .into_iter()
            .map(|field| {
                let ordinal = context
                    .schema
                    .relation_column_ordinal(relation, field)
                    .expect("schema-validated model rule references an existing relation column");
                (field, ordinal)
            })
            .collect();
        Self {
            expression: CompiledSemanticRuleExpr::compile(rule),
            columns,
        }
    }

    pub(crate) fn matches(&self, row: &[Value]) -> Result<bool, RuleEvaluationError> {
        self.expression.matches(&|coordinate| match coordinate {
            RuleValueExpr::Input => None,
            RuleValueExpr::Field(field) => self
                .columns
                .get(field)
                .and_then(|ordinal| row.get(*ordinal)),
        })
    }

    pub(crate) fn matches_semantic(
        &self,
        row: &[Value],
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RuleEvaluationError> {
        self.expression.matches_semantic(
            &|coordinate| match coordinate {
                RuleValueExpr::Input => None,
                RuleValueExpr::Field(field) => self
                    .columns
                    .get(field)
                    .and_then(|ordinal| row.get(*ordinal)),
            },
            context,
            registry,
        )
    }

    fn columns(&self) -> impl Iterator<Item = SemanticId> + '_ {
        self.columns.keys().copied()
    }
}

fn collect_rule_fields(rule: &SemanticRuleExpr, fields: &mut BTreeSet<SemanticId>) {
    match rule {
        SemanticRuleExpr::True | SemanticRuleExpr::False => {}
        SemanticRuleExpr::And(parts) | SemanticRuleExpr::Or(parts) => {
            for part in parts {
                collect_rule_fields(part, fields);
            }
        }
        SemanticRuleExpr::Not(part) => collect_rule_fields(part, fields),
        SemanticRuleExpr::I64Range { value, .. }
        | SemanticRuleExpr::TextLength { value, .. }
        | SemanticRuleExpr::TextOneOf { value, .. }
        | SemanticRuleExpr::TextMatches { value, .. } => {
            if let RuleValueExpr::Field(field) = value {
                fields.insert(*field);
            }
        }
        SemanticRuleExpr::Equivalent { left, right, .. }
        | SemanticRuleExpr::Ordered { left, right, .. } => {
            for value in [left, right] {
                if let RuleValueExpr::Field(field) = value {
                    fields.insert(*field);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRuleDependency {
    relation: SemanticId,
    columns: BTreeSet<SemanticId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactAggregateWitness {
    Count(kernel_aggregate::ExactCount),
    F64Sum(kernel_aggregate::ExactF64Sum),
    OrderedStatistic {
        values: kernel_aggregate::ExactOrderedMultiset<kernel_semantics::CanonicalOrderKey>,
        selector: OrderedStatisticSelector,
    },
}

type CanonicalGroupKey = Vec<kernel_semantics::CanonicalEqKey>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedExactMeasureBucket {
    members: kernel_aggregate::ExactCount,
    selected: ExactAggregateWitness,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupedExactMeasureWitness {
    groups: BTreeMap<CanonicalGroupKey, GroupedExactMeasureBucket>,
    violation_mass: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedExactMeasureProductBucket {
    members: kernel_aggregate::ExactCount,
    left: ExactAggregateWitness,
    right: ExactAggregateWitness,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupedExactMeasureProductWitness {
    groups: BTreeMap<CanonicalGroupKey, GroupedExactMeasureProductBucket>,
    violation_mass: u64,
}

#[derive(Debug)]
pub enum CompiledExactAggregateMeasure {
    Count {
        dependency: ModelRuleDependency,
        predicate: Option<CompiledRelationPredicate>,
    },
    F64Sum {
        dependency: ModelRuleDependency,
        column_ordinal: usize,
        predicate: Option<CompiledRelationPredicate>,
    },
    OrderedStatistic {
        dependency: ModelRuleDependency,
        column_ordinal: usize,
        predicate: Option<CompiledRelationPredicate>,
        ordering: SemanticId,
        selector: OrderedStatisticSelector,
    },
}

impl ModelRuleDependency {
    #[must_use]
    pub const fn relation(&self) -> SemanticId {
        self.relation
    }
    #[must_use]
    pub const fn columns(&self) -> &BTreeSet<SemanticId> {
        &self.columns
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationMutationFootprint {
    membership_changed: bool,
    columns: Option<BTreeSet<SemanticId>>,
}

impl RelationMutationFootprint {
    #[must_use]
    pub fn full() -> Self {
        Self {
            membership_changed: true,
            columns: None,
        }
    }

    #[must_use]
    pub fn fields(columns: impl IntoIterator<Item = SemanticId>) -> Self {
        Self {
            membership_changed: false,
            columns: Some(columns.into_iter().collect()),
        }
    }

    #[must_use]
    pub const fn membership_changed(&self) -> bool {
        self.membership_changed
    }

    #[must_use]
    pub fn columns(&self) -> Option<&BTreeSet<SemanticId>> {
        self.columns.as_ref()
    }

    #[must_use]
    pub fn affects(&self, dependency: &ModelRuleDependency) -> bool {
        if self.membership_changed {
            return true;
        }
        match &self.columns {
            None => true,
            Some(columns) => !columns.is_disjoint(&dependency.columns),
        }
    }
}

impl CompiledExactAggregateMeasure {
    #[must_use]
    pub fn compile(measure: &ExactAggregateMeasureExpr, context: &SemanticContext) -> Self {
        match measure {
            ExactAggregateMeasureExpr::Count {
                relation,
                predicate,
            } => {
                let predicate = if matches!(predicate, SemanticRuleExpr::True) {
                    None
                } else {
                    Some(CompiledRelationPredicate::compile(
                        *relation, predicate, context,
                    ))
                };
                let columns = predicate
                    .iter()
                    .flat_map(CompiledRelationPredicate::columns)
                    .collect();
                Self::Count {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    predicate,
                }
            }
            ExactAggregateMeasureExpr::F64Sum {
                relation,
                column,
                predicate,
            } => {
                let predicate = if matches!(predicate, SemanticRuleExpr::True) {
                    None
                } else {
                    Some(CompiledRelationPredicate::compile(
                        *relation, predicate, context,
                    ))
                };
                let mut columns = predicate
                    .iter()
                    .flat_map(CompiledRelationPredicate::columns)
                    .collect::<BTreeSet<_>>();
                columns.insert(*column);
                Self::F64Sum {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    column_ordinal: context
                        .schema
                        .relation_column_ordinal(*relation, *column)
                        .expect("schema-validated aggregate column exists"),
                    predicate,
                }
            }
            ExactAggregateMeasureExpr::OrderedStatistic {
                relation,
                column,
                predicate,
                ordering,
                selector,
            } => {
                let predicate = if matches!(predicate, SemanticRuleExpr::True) {
                    None
                } else {
                    Some(CompiledRelationPredicate::compile(
                        *relation, predicate, context,
                    ))
                };
                let mut columns = predicate
                    .iter()
                    .flat_map(CompiledRelationPredicate::columns)
                    .collect::<BTreeSet<_>>();
                columns.insert(*column);
                Self::OrderedStatistic {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    column_ordinal: context
                        .schema
                        .relation_column_ordinal(*relation, *column)
                        .expect("schema-validated extremum column exists"),
                    predicate,
                    ordering: *ordering,
                    selector: *selector,
                }
            }
        }
    }

    #[must_use]
    pub const fn dependency(&self) -> &ModelRuleDependency {
        match self {
            Self::Count { dependency, .. }
            | Self::F64Sum { dependency, .. }
            | Self::OrderedStatistic { dependency, .. } => dependency,
        }
    }

    #[must_use]
    pub fn zero_witness(&self) -> ExactAggregateWitness {
        match self {
            Self::Count { .. } => {
                ExactAggregateWitness::Count(kernel_aggregate::ExactCount::default())
            }
            Self::F64Sum { .. } => {
                ExactAggregateWitness::F64Sum(kernel_aggregate::ExactF64Sum::default())
            }
            Self::OrderedStatistic { selector, .. } => ExactAggregateWitness::OrderedStatistic {
                values: kernel_aggregate::ExactOrderedMultiset::default(),
                selector: *selector,
            },
        }
    }

    pub fn build_witness(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<ExactAggregateWitness, RuleEvaluationError> {
        match self {
            Self::Count {
                dependency,
                predicate,
            } => Ok(ExactAggregateWitness::Count(
                CompiledModelRule::exact_count_semantic(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    predicate.as_ref(),
                )?,
            )),
            Self::F64Sum {
                dependency,
                column_ordinal,
                predicate,
            } => Ok(ExactAggregateWitness::F64Sum(
                CompiledModelRule::exact_f64_sum_semantic(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    *column_ordinal,
                    predicate.as_ref(),
                )?,
            )),
            Self::OrderedStatistic {
                dependency,
                column_ordinal,
                predicate,
                ordering,
                selector,
            } => {
                let mut values = kernel_aggregate::ExactOrderedMultiset::default();
                if let Some(rows) = state.model.relations.get(&dependency.relation) {
                    for row in rows {
                        if !CompiledModelRule::aggregate_row_selected_semantic(
                            predicate.as_ref(),
                            row,
                            context,
                            registry,
                        )? {
                            continue;
                        }
                        let value = row
                            .get(*column_ordinal)
                            .ok_or(RuleEvaluationError::MissingValue)?;
                        let key = registry
                            .canonical_order_key(context, *ordering, value)
                            .map_err(|_| RuleEvaluationError::Semantic)?;
                        values.add_one(key);
                    }
                }
                Ok(ExactAggregateWitness::OrderedStatistic {
                    values,
                    selector: *selector,
                })
            }
        }
    }

    pub fn build_witness_unchecked(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<ExactAggregateWitness, RuleEvaluationError> {
        match self {
            Self::Count {
                dependency,
                predicate,
            } => Ok(ExactAggregateWitness::Count(
                CompiledModelRule::exact_count(state, dependency.relation, predicate.as_ref())?,
            )),
            Self::F64Sum {
                dependency,
                column_ordinal,
                predicate,
            } => Ok(ExactAggregateWitness::F64Sum(
                CompiledModelRule::exact_f64_sum(
                    state,
                    dependency.relation,
                    *column_ordinal,
                    predicate.as_ref(),
                )?,
            )),
            Self::OrderedStatistic { .. } => Err(RuleEvaluationError::Semantic),
        }
    }

    pub fn apply_delta(
        &self,
        relation: SemanticId,
        witness: &mut ExactAggregateWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if self.dependency().relation != relation {
            return Ok(());
        }
        match (self, &mut *witness) {
            (Self::Count { predicate, .. }, ExactAggregateWitness::Count(count)) => {
                for row in removed {
                    if CompiledModelRule::aggregate_row_selected(predicate.as_ref(), row)? {
                        count
                            .remove_one()
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
                for row in inserted {
                    if CompiledModelRule::aggregate_row_selected(predicate.as_ref(), row)? {
                        count.add_one();
                    }
                }
            }
            (
                Self::F64Sum {
                    column_ordinal,
                    predicate,
                    ..
                },
                ExactAggregateWitness::F64Sum(sum),
            ) => {
                for row in removed {
                    if !CompiledModelRule::aggregate_row_selected(predicate.as_ref(), row)? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.remove(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
                for row in inserted {
                    if !CompiledModelRule::aggregate_row_selected(predicate.as_ref(), row)? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.add(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
            }
            (Self::OrderedStatistic { .. }, ExactAggregateWitness::OrderedStatistic { .. }) => {
                return Err(RuleEvaluationError::Semantic);
            }
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }

    pub fn apply_row_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        witness: &mut ExactAggregateWitness,
        row: &[Value],
        insert: bool,
    ) -> Result<(), RuleEvaluationError> {
        match (self, &mut *witness) {
            (Self::Count { predicate, .. }, ExactAggregateWitness::Count(count)) => {
                if CompiledModelRule::aggregate_row_selected_semantic(
                    predicate.as_ref(),
                    row,
                    context,
                    registry,
                )? {
                    if insert {
                        count.add_one();
                    } else {
                        count
                            .remove_one()
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
            }
            (
                Self::F64Sum {
                    column_ordinal,
                    predicate,
                    ..
                },
                ExactAggregateWitness::F64Sum(sum),
            ) => {
                if CompiledModelRule::aggregate_row_selected_semantic(
                    predicate.as_ref(),
                    row,
                    context,
                    registry,
                )? {
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    if insert {
                        sum.add(f64::from_bits(*bits))
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    } else {
                        sum.remove(f64::from_bits(*bits))
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
            }
            (
                Self::OrderedStatistic {
                    column_ordinal,
                    predicate,
                    ordering,
                    ..
                },
                ExactAggregateWitness::OrderedStatistic { values, .. },
            ) => {
                if CompiledModelRule::aggregate_row_selected_semantic(
                    predicate.as_ref(),
                    row,
                    context,
                    registry,
                )? {
                    let value = row
                        .get(*column_ordinal)
                        .ok_or(RuleEvaluationError::MissingValue)?;
                    let key = registry
                        .canonical_order_key(context, *ordering, value)
                        .map_err(|_| RuleEvaluationError::Semantic)?;
                    if insert {
                        values.add_one(key);
                    } else {
                        values
                            .remove_one(&key)
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
            }
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }

    pub fn apply_delta_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        relation: SemanticId,
        witness: &mut ExactAggregateWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if self.dependency().relation != relation {
            return Ok(());
        }
        match (self, &mut *witness) {
            (Self::Count { predicate, .. }, ExactAggregateWitness::Count(count)) => {
                for row in removed {
                    if CompiledModelRule::aggregate_row_selected_semantic(
                        predicate.as_ref(),
                        row,
                        context,
                        registry,
                    )? {
                        count
                            .remove_one()
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
                for row in inserted {
                    if CompiledModelRule::aggregate_row_selected_semantic(
                        predicate.as_ref(),
                        row,
                        context,
                        registry,
                    )? {
                        count.add_one();
                    }
                }
            }
            (
                Self::F64Sum {
                    column_ordinal,
                    predicate,
                    ..
                },
                ExactAggregateWitness::F64Sum(sum),
            ) => {
                for row in removed {
                    if !CompiledModelRule::aggregate_row_selected_semantic(
                        predicate.as_ref(),
                        row,
                        context,
                        registry,
                    )? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.remove(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
                for row in inserted {
                    if !CompiledModelRule::aggregate_row_selected_semantic(
                        predicate.as_ref(),
                        row,
                        context,
                        registry,
                    )? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.add(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
            }
            (Self::OrderedStatistic { .. }, ExactAggregateWitness::OrderedStatistic { .. }) => {
                for row in removed {
                    self.apply_row_semantic(context, registry, witness, row, false)?;
                }
                for row in inserted {
                    self.apply_row_semantic(context, registry, witness, row, true)?;
                }
            }
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRuleWitness {
    RelationExactCountRange {
        count: kernel_aggregate::ExactCount,
    },
    RelationExactF64SumRange {
        sum: kernel_aggregate::ExactF64Sum,
    },
    RelationExactOrderedStatisticRange {
        measure: ExactAggregateWitness,
        min: Option<kernel_semantics::CanonicalOrderKey>,
        max: Option<kernel_semantics::CanonicalOrderKey>,
    },
    ExactAggregateCompare {
        left: ExactAggregateWitness,
        right: ExactAggregateWitness,
    },
    RelationGroupedExactCountRange {
        witness: GroupedExactMeasureWitness,
    },
    RelationGroupedExactF64SumRange {
        witness: GroupedExactMeasureWitness,
    },
    RelationGroupedExactOrderedStatisticRange {
        witness: GroupedExactMeasureWitness,
        min: Option<kernel_semantics::CanonicalOrderKey>,
        max: Option<kernel_semantics::CanonicalOrderKey>,
    },
    RelationGroupedExactAggregateCompare {
        witness: GroupedExactMeasureProductWitness,
    },
}

impl ModelRuleWitness {
    fn violation_mass_value(&self) -> Result<u64, RuleEvaluationError> {
        match self {
            Self::RelationGroupedExactCountRange { witness }
            | Self::RelationGroupedExactF64SumRange { witness }
            | Self::RelationGroupedExactOrderedStatisticRange { witness, .. } => {
                Ok(witness.violation_mass)
            }
            Self::RelationGroupedExactAggregateCompare { witness } => Ok(witness.violation_mass),
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelRuleWitnessState {
    witnesses: Vec<ModelRuleWitness>,
}

impl ModelRuleWitnessState {
    pub fn build(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<Self, RuleEvaluationError> {
        let plan = CompiledRulePlan::compile(context);
        let witnesses = plan
            .model_rules()
            .iter()
            .map(|rule| rule.build_witness(context, registry, state))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { witnesses })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.witnesses.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.witnesses.is_empty()
    }

    pub fn apply_relation_delta(
        &self,
        context: &SemanticContext,
        relation: SemanticId,
        footprint: &RelationMutationFootprint,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<Self, RuleEvaluationError> {
        let plan = CompiledRulePlan::compile(context);
        if plan.model_rules().len() != self.witnesses.len() {
            return Err(RuleEvaluationError::TypeMismatch);
        }
        let mut next = self.clone();
        for (rule_index, rule) in plan.model_rules_for_mutation(relation, footprint) {
            rule.apply_delta(relation, &mut next.witnesses[rule_index], removed, inserted)?;
        }
        Ok(next)
    }

    pub fn apply_relation_delta_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        relation: SemanticId,
        footprint: &RelationMutationFootprint,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<Self, RuleEvaluationError> {
        let plan = CompiledRulePlan::compile(context);
        if plan.model_rules().len() != self.witnesses.len() {
            return Err(RuleEvaluationError::TypeMismatch);
        }
        let mut next = self.clone();
        for (rule_index, rule) in plan.model_rules_for_mutation(relation, footprint) {
            rule.apply_delta_semantic(
                context,
                registry,
                relation,
                &mut next.witnesses[rule_index],
                removed,
                inserted,
            )?;
        }
        Ok(next)
    }

    pub fn violation_mass(
        &self,
        context: &SemanticContext,
        rule_index: usize,
    ) -> Result<u64, RuleEvaluationError> {
        let plan = CompiledRulePlan::compile(context);
        let rule = plan
            .model_rules()
            .get(rule_index)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        let witness = self
            .witnesses
            .get(rule_index)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        rule.witness_violation_mass(witness)
    }

    pub fn is_satisfied(
        &self,
        context: &SemanticContext,
        rule_index: usize,
    ) -> Result<bool, RuleEvaluationError> {
        Ok(self.violation_mass(context, rule_index)? == 0)
    }
}

#[derive(Debug)]
pub enum CompiledModelRule {
    RelationExactCountRange {
        dependency: ModelRuleDependency,
        predicate: Option<CompiledRelationPredicate>,
        min: u64,
        max: Option<u64>,
    },
    RelationExactF64SumRange {
        dependency: ModelRuleDependency,
        column_ordinal: usize,
        predicate: Option<CompiledRelationPredicate>,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    },
    RelationExactOrderedStatisticRange {
        measure: CompiledExactAggregateMeasure,
        min: Option<OrderedStatisticBound>,
        max: Option<OrderedStatisticBound>,
    },
    ExactAggregateCompare {
        left: CompiledExactAggregateMeasure,
        right: CompiledExactAggregateMeasure,
        comparison: RuleOrderComparison,
    },
    RelationGroupedExactCountRange {
        dependency: ModelRuleDependency,
        group_column_ordinals: Vec<usize>,
        group_equivalences: Vec<SemanticId>,
        predicate: Option<CompiledRelationPredicate>,
        min: u64,
        max: Option<u64>,
    },
    RelationGroupedExactF64SumRange {
        dependency: ModelRuleDependency,
        group_column_ordinals: Vec<usize>,
        group_equivalences: Vec<SemanticId>,
        column_ordinal: usize,
        predicate: Option<CompiledRelationPredicate>,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    },
    RelationGroupedExactOrderedStatisticRange {
        dependency: ModelRuleDependency,
        group_column_ordinals: Vec<usize>,
        group_equivalences: Vec<SemanticId>,
        measure: CompiledExactAggregateMeasure,
        min: Option<OrderedStatisticBound>,
        max: Option<OrderedStatisticBound>,
    },
    RelationGroupedExactAggregateCompare {
        dependency: ModelRuleDependency,
        group_column_ordinals: Vec<usize>,
        group_equivalences: Vec<SemanticId>,
        left: CompiledExactAggregateMeasure,
        right: CompiledExactAggregateMeasure,
        comparison: RuleOrderComparison,
    },
}

impl CompiledModelRule {
    #[must_use]
    pub fn compile(rule: &ModelRuleExpr, context: &SemanticContext) -> Self {
        match rule {
            ModelRuleExpr::RelationExactMeasure { constraint } => {
                Self::compile_exact_measure_constraint(context, constraint)
            }
            ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns,
                group_equivalences,
                constraint,
            } => Self::compile_grouped_exact_measure(
                context,
                group_columns,
                group_equivalences,
                constraint,
            ),
        }
    }

    fn compile_exact_measure_constraint(
        context: &SemanticContext,
        constraint: &ExactMeasureConstraint,
    ) -> Self {
        match constraint {
            ExactMeasureConstraint::Range {
                measure:
                    ExactAggregateMeasureExpr::Count {
                        relation,
                        predicate,
                    },
                range: ExactAggregateRange::Count { min, max },
            } => {
                let predicate = if matches!(predicate, SemanticRuleExpr::True) {
                    None
                } else {
                    Some(CompiledRelationPredicate::compile(
                        *relation, predicate, context,
                    ))
                };
                let columns = predicate
                    .iter()
                    .flat_map(CompiledRelationPredicate::columns)
                    .collect();
                Self::RelationExactCountRange {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    predicate,
                    min: *min,
                    max: *max,
                }
            }
            ExactMeasureConstraint::Range {
                measure:
                    ExactAggregateMeasureExpr::F64Sum {
                        relation,
                        column,
                        predicate,
                    },
                range: ExactAggregateRange::F64Sum { min, max },
            } => Self::compile_exact_f64_sum_range(
                context, *relation, *column, predicate, *min, *max,
            ),
            ExactMeasureConstraint::Range {
                measure: measure @ ExactAggregateMeasureExpr::OrderedStatistic { .. },
                range: ExactAggregateRange::OrderedStatistic { min, max },
            } => Self::RelationExactOrderedStatisticRange {
                measure: CompiledExactAggregateMeasure::compile(measure, context),
                min: min.clone(),
                max: max.clone(),
            },
            ExactMeasureConstraint::Compare {
                left,
                right,
                comparison,
            } => Self::ExactAggregateCompare {
                left: CompiledExactAggregateMeasure::compile(left, context),
                right: CompiledExactAggregateMeasure::compile(right, context),
                comparison: *comparison,
            },
            ExactMeasureConstraint::Range { .. } => {
                unreachable!("schema validation rejects mismatched exact-measure range codomains")
            }
        }
    }

    fn compile_grouped_exact_measure(
        context: &SemanticContext,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
        constraint: &ExactMeasureConstraint,
    ) -> Self {
        match constraint {
            ExactMeasureConstraint::Range {
                measure:
                    ExactAggregateMeasureExpr::Count {
                        relation,
                        predicate,
                    },
                range: ExactAggregateRange::Count { min, max },
            } => Self::compile_grouped_exact_count_range(
                context,
                *relation,
                group_columns,
                group_equivalences,
                predicate,
                *min,
                *max,
            ),
            ExactMeasureConstraint::Range {
                measure:
                    ExactAggregateMeasureExpr::F64Sum {
                        relation,
                        column,
                        predicate,
                    },
                range: ExactAggregateRange::F64Sum { min, max },
            } => Self::compile_grouped_exact_f64_sum_range(
                context,
                *relation,
                group_columns,
                group_equivalences,
                *column,
                predicate,
                *min,
                *max,
            ),
            ExactMeasureConstraint::Range {
                measure: measure @ ExactAggregateMeasureExpr::OrderedStatistic { relation, .. },
                range: ExactAggregateRange::OrderedStatistic { min, max },
            } => Self::compile_grouped_exact_ordered_statistic_range(
                context,
                *relation,
                group_columns,
                group_equivalences,
                measure,
                min.clone(),
                max.clone(),
            ),
            ExactMeasureConstraint::Compare {
                left,
                right,
                comparison,
            } => Self::compile_grouped_exact_aggregate_compare(
                context,
                exact_aggregate_measure_relation(left),
                group_columns,
                group_equivalences,
                left,
                right,
                *comparison,
            ),
            ExactMeasureConstraint::Range { .. } => {
                unreachable!("schema validation rejects mismatched grouped range codomains")
            }
        }
    }

    fn compile_grouped_exact_ordered_statistic_range(
        context: &SemanticContext,
        relation: SemanticId,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
        measure: &ExactAggregateMeasureExpr,
        min: Option<OrderedStatisticBound>,
        max: Option<OrderedStatisticBound>,
    ) -> Self {
        let measure = CompiledExactAggregateMeasure::compile(measure, context);
        let mut columns = measure.dependency().columns.clone();
        columns.extend(group_columns.iter().copied());
        Self::RelationGroupedExactOrderedStatisticRange {
            dependency: ModelRuleDependency { relation, columns },
            group_column_ordinals: group_columns
                .iter()
                .map(|group_column| {
                    context
                        .schema
                        .relation_column_ordinal(relation, *group_column)
                        .expect("schema-validated group column exists")
                })
                .collect(),
            group_equivalences: group_equivalences.to_vec(),
            measure,
            min,
            max,
        }
    }

    fn compile_exact_f64_sum_range(
        context: &SemanticContext,
        relation: SemanticId,
        column: SemanticId,
        predicate: &SemanticRuleExpr,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    ) -> Self {
        let predicate = if matches!(predicate, SemanticRuleExpr::True) {
            None
        } else {
            Some(CompiledRelationPredicate::compile(
                relation, predicate, context,
            ))
        };
        let mut columns = predicate
            .iter()
            .flat_map(CompiledRelationPredicate::columns)
            .collect::<BTreeSet<_>>();
        columns.insert(column);
        Self::RelationExactF64SumRange {
            dependency: ModelRuleDependency { relation, columns },
            column_ordinal: context
                .schema
                .relation_column_ordinal(relation, column)
                .expect("schema-validated aggregate column exists"),
            predicate,
            min,
            max,
        }
    }

    fn compile_grouped_exact_count_range(
        context: &SemanticContext,
        relation: SemanticId,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
        predicate: &SemanticRuleExpr,
        min: u64,
        max: Option<u64>,
    ) -> Self {
        let predicate = if matches!(predicate, SemanticRuleExpr::True) {
            None
        } else {
            Some(CompiledRelationPredicate::compile(
                relation, predicate, context,
            ))
        };
        let mut columns = predicate
            .iter()
            .flat_map(CompiledRelationPredicate::columns)
            .collect::<BTreeSet<_>>();
        columns.extend(group_columns.iter().copied());
        Self::RelationGroupedExactCountRange {
            dependency: ModelRuleDependency { relation, columns },
            group_column_ordinals: group_columns
                .iter()
                .map(|column| {
                    context
                        .schema
                        .relation_column_ordinal(relation, *column)
                        .expect("schema-validated group column exists")
                })
                .collect(),
            group_equivalences: group_equivalences.to_vec(),
            predicate,
            min,
            max,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_grouped_exact_f64_sum_range(
        context: &SemanticContext,
        relation: SemanticId,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
        column: SemanticId,
        predicate: &SemanticRuleExpr,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    ) -> Self {
        let predicate = if matches!(predicate, SemanticRuleExpr::True) {
            None
        } else {
            Some(CompiledRelationPredicate::compile(
                relation, predicate, context,
            ))
        };
        let mut columns = predicate
            .iter()
            .flat_map(CompiledRelationPredicate::columns)
            .collect::<BTreeSet<_>>();
        columns.extend(group_columns.iter().copied());
        columns.insert(column);
        Self::RelationGroupedExactF64SumRange {
            dependency: ModelRuleDependency { relation, columns },
            group_column_ordinals: group_columns
                .iter()
                .map(|group_column| {
                    context
                        .schema
                        .relation_column_ordinal(relation, *group_column)
                        .expect("schema-validated group column exists")
                })
                .collect(),
            group_equivalences: group_equivalences.to_vec(),
            column_ordinal: context
                .schema
                .relation_column_ordinal(relation, column)
                .expect("schema-validated grouped aggregate column exists"),
            predicate,
            min,
            max,
        }
    }

    fn compile_grouped_exact_aggregate_compare(
        context: &SemanticContext,
        relation: SemanticId,
        group_columns: &[SemanticId],
        group_equivalences: &[SemanticId],
        left: &ExactAggregateMeasureExpr,
        right: &ExactAggregateMeasureExpr,
        comparison: RuleOrderComparison,
    ) -> Self {
        let left = CompiledExactAggregateMeasure::compile(left, context);
        let right = CompiledExactAggregateMeasure::compile(right, context);
        let mut columns = group_columns.iter().copied().collect::<BTreeSet<_>>();
        columns.extend(left.dependency().columns.iter().copied());
        columns.extend(right.dependency().columns.iter().copied());
        Self::RelationGroupedExactAggregateCompare {
            dependency: ModelRuleDependency { relation, columns },
            group_column_ordinals: group_columns
                .iter()
                .map(|column| {
                    context
                        .schema
                        .relation_column_ordinal(relation, *column)
                        .expect("schema-validated group column exists")
                })
                .collect(),
            group_equivalences: group_equivalences.to_vec(),
            left,
            right,
            comparison,
        }
    }

    #[must_use]
    pub fn dependency_relations(&self) -> Vec<SemanticId> {
        match self {
            Self::RelationExactCountRange { dependency, .. }
            | Self::RelationExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactCountRange { dependency, .. }
            | Self::RelationGroupedExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactOrderedStatisticRange { dependency, .. }
            | Self::RelationGroupedExactAggregateCompare { dependency, .. } => {
                vec![dependency.relation]
            }
            Self::RelationExactOrderedStatisticRange { measure, .. } => {
                vec![measure.dependency().relation]
            }
            Self::ExactAggregateCompare { left, right, .. } => {
                let left = left.dependency().relation;
                let right = right.dependency().relation;
                if left == right {
                    vec![left]
                } else {
                    vec![left, right]
                }
            }
        }
    }

    #[must_use]
    pub fn dependencies(&self) -> Vec<&ModelRuleDependency> {
        match self {
            Self::RelationExactCountRange { dependency, .. }
            | Self::RelationExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactCountRange { dependency, .. }
            | Self::RelationGroupedExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactOrderedStatisticRange { dependency, .. }
            | Self::RelationGroupedExactAggregateCompare { dependency, .. } => vec![dependency],
            Self::RelationExactOrderedStatisticRange { measure, .. } => vec![measure.dependency()],
            Self::ExactAggregateCompare { left, right, .. } => {
                if left.dependency().relation == right.dependency().relation
                    && left.dependency().columns == right.dependency().columns
                {
                    vec![left.dependency()]
                } else {
                    vec![left.dependency(), right.dependency()]
                }
            }
        }
    }

    #[must_use]
    pub fn affected_by(&self, relation: SemanticId, footprint: &RelationMutationFootprint) -> bool {
        match self {
            Self::RelationExactCountRange { dependency, .. }
            | Self::RelationExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactCountRange { dependency, .. }
            | Self::RelationGroupedExactF64SumRange { dependency, .. }
            | Self::RelationGroupedExactOrderedStatisticRange { dependency, .. }
            | Self::RelationGroupedExactAggregateCompare { dependency, .. } => {
                dependency.relation == relation && footprint.affects(dependency)
            }
            Self::RelationExactOrderedStatisticRange { measure, .. } => {
                measure.dependency().relation == relation && footprint.affects(measure.dependency())
            }
            Self::ExactAggregateCompare { left, right, .. } => {
                [left.dependency(), right.dependency()]
                    .into_iter()
                    .any(|dependency| {
                        dependency.relation == relation && footprint.affects(dependency)
                    })
            }
        }
    }

    fn build_ordered_statistic_range_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        measure: &CompiledExactAggregateMeasure,
        min: Option<&OrderedStatisticBound>,
        max: Option<&OrderedStatisticBound>,
    ) -> Result<ModelRuleWitness, RuleEvaluationError> {
        let ordering = match measure {
            CompiledExactAggregateMeasure::OrderedStatistic { ordering, .. } => *ordering,
            _ => return Err(RuleEvaluationError::TypeMismatch),
        };
        let (min, max) =
            Self::canonicalize_ordered_statistic_range(context, registry, ordering, min, max)?;
        Ok(ModelRuleWitness::RelationExactOrderedStatisticRange {
            measure: measure.build_witness(context, registry, state)?,
            min,
            max,
        })
    }

    fn build_grouped_witness(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<ModelRuleWitness, RuleEvaluationError> {
        match self {
            Self::RelationGroupedExactCountRange {
                dependency,
                group_column_ordinals,
                group_equivalences,
                predicate,
                min,
                max,
            } => Ok(ModelRuleWitness::RelationGroupedExactCountRange {
                witness: Self::build_grouped_exact_count_witness(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    group_column_ordinals,
                    group_equivalences,
                    predicate.as_ref(),
                    *min,
                    *max,
                )?,
            }),
            Self::RelationGroupedExactF64SumRange {
                dependency,
                group_column_ordinals,
                group_equivalences,
                column_ordinal,
                predicate,
                min,
                max,
            } => Ok(ModelRuleWitness::RelationGroupedExactF64SumRange {
                witness: Self::build_grouped_exact_f64_sum_witness(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    group_column_ordinals,
                    group_equivalences,
                    *column_ordinal,
                    predicate.as_ref(),
                    *min,
                    *max,
                )?,
            }),
            Self::RelationGroupedExactOrderedStatisticRange {
                dependency,
                group_column_ordinals,
                group_equivalences,
                measure,
                min,
                max,
            } => Self::build_grouped_ordered_statistic_rule_witness(
                context,
                registry,
                state,
                dependency.relation,
                group_column_ordinals,
                group_equivalences,
                measure,
                min.as_ref(),
                max.as_ref(),
            ),
            Self::RelationGroupedExactAggregateCompare {
                dependency,
                group_column_ordinals,
                group_equivalences,
                left,
                right,
                comparison,
            } => Ok(ModelRuleWitness::RelationGroupedExactAggregateCompare {
                witness: Self::build_grouped_exact_aggregate_compare_witness(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    group_column_ordinals,
                    group_equivalences,
                    left,
                    right,
                    *comparison,
                )?,
            }),
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_grouped_ordered_statistic_rule_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        measure: &CompiledExactAggregateMeasure,
        min: Option<&OrderedStatisticBound>,
        max: Option<&OrderedStatisticBound>,
    ) -> Result<ModelRuleWitness, RuleEvaluationError> {
        let ordering = match measure {
            CompiledExactAggregateMeasure::OrderedStatistic { ordering, .. } => *ordering,
            _ => return Err(RuleEvaluationError::TypeMismatch),
        };
        let (min, max) =
            Self::canonicalize_ordered_statistic_range(context, registry, ordering, min, max)?;
        Ok(
            ModelRuleWitness::RelationGroupedExactOrderedStatisticRange {
                witness: Self::build_grouped_exact_ordered_statistic_witness(
                    context,
                    registry,
                    state,
                    relation,
                    group_column_ordinals,
                    group_equivalences,
                    measure,
                    min.as_ref(),
                    max.as_ref(),
                )?,
                min,
                max,
            },
        )
    }

    pub fn build_witness(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<ModelRuleWitness, RuleEvaluationError> {
        match self {
            Self::RelationExactCountRange {
                dependency,
                predicate,
                ..
            } => Ok(ModelRuleWitness::RelationExactCountRange {
                count: Self::exact_count_semantic(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    predicate.as_ref(),
                )?,
            }),
            Self::RelationExactF64SumRange {
                dependency,
                column_ordinal,
                predicate,
                ..
            } => Ok(ModelRuleWitness::RelationExactF64SumRange {
                sum: Self::exact_f64_sum_semantic(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    *column_ordinal,
                    predicate.as_ref(),
                )?,
            }),
            Self::RelationExactOrderedStatisticRange { measure, min, max } => {
                Self::build_ordered_statistic_range_witness(
                    context,
                    registry,
                    state,
                    measure,
                    min.as_ref(),
                    max.as_ref(),
                )
            }
            Self::ExactAggregateCompare { left, right, .. } => {
                Ok(ModelRuleWitness::ExactAggregateCompare {
                    left: left.build_witness(context, registry, state)?,
                    right: right.build_witness(context, registry, state)?,
                })
            }
            Self::RelationGroupedExactCountRange { .. }
            | Self::RelationGroupedExactF64SumRange { .. }
            | Self::RelationGroupedExactOrderedStatisticRange { .. }
            | Self::RelationGroupedExactAggregateCompare { .. } => {
                self.build_grouped_witness(context, registry, state)
            }
        }
    }

    pub fn apply_delta(
        &self,
        relation: SemanticId,
        witness: &mut ModelRuleWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        match (self, &mut *witness) {
            (
                Self::RelationExactCountRange { predicate, .. },
                ModelRuleWitness::RelationExactCountRange { count },
            ) => {
                for row in removed {
                    if Self::aggregate_row_selected(predicate.as_ref(), row)? {
                        count
                            .remove_one()
                            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                    }
                }
                for row in inserted {
                    if Self::aggregate_row_selected(predicate.as_ref(), row)? {
                        count.add_one();
                    }
                }
            }
            (
                Self::RelationExactF64SumRange {
                    column_ordinal,
                    predicate,
                    ..
                },
                ModelRuleWitness::RelationExactF64SumRange { sum },
            ) => {
                for row in removed {
                    if !Self::aggregate_row_selected(predicate.as_ref(), row)? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.remove(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
                for row in inserted {
                    if !Self::aggregate_row_selected(predicate.as_ref(), row)? {
                        continue;
                    }
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.add(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
            }
            (
                Self::ExactAggregateCompare { left, right, .. },
                ModelRuleWitness::ExactAggregateCompare {
                    left: left_witness,
                    right: right_witness,
                },
            ) => {
                left.apply_delta(relation, left_witness, removed, inserted)?;
                right.apply_delta(relation, right_witness, removed, inserted)?;
            }
            (
                Self::RelationExactOrderedStatisticRange { .. }
                | Self::RelationGroupedExactCountRange { .. }
                | Self::RelationGroupedExactF64SumRange { .. }
                | Self::RelationGroupedExactOrderedStatisticRange { .. }
                | Self::RelationGroupedExactAggregateCompare { .. },
                _,
            ) => {
                return Err(RuleEvaluationError::Semantic);
            }
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }

    pub fn apply_delta_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        relation: SemanticId,
        witness: &mut ModelRuleWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        match (self, &mut *witness) {
            (
                Self::RelationExactCountRange { predicate, .. },
                ModelRuleWitness::RelationExactCountRange { count },
            ) => Self::apply_exact_count_delta_semantic(
                context,
                registry,
                predicate.as_ref(),
                count,
                removed,
                inserted,
            )?,
            (
                Self::RelationExactF64SumRange {
                    column_ordinal,
                    predicate,
                    ..
                },
                ModelRuleWitness::RelationExactF64SumRange { sum },
            ) => Self::apply_exact_f64_sum_delta_semantic(
                context,
                registry,
                predicate.as_ref(),
                *column_ordinal,
                sum,
                removed,
                inserted,
            )?,
            (
                Self::RelationExactOrderedStatisticRange { measure, .. },
                ModelRuleWitness::RelationExactOrderedStatisticRange {
                    measure: measure_witness,
                    ..
                },
            ) => measure.apply_delta_semantic(
                context,
                registry,
                relation,
                measure_witness,
                removed,
                inserted,
            )?,
            (
                Self::ExactAggregateCompare { left, right, .. },
                ModelRuleWitness::ExactAggregateCompare {
                    left: left_witness,
                    right: right_witness,
                },
            ) => Self::apply_aggregate_compare_delta_semantic(
                context,
                registry,
                relation,
                left,
                right,
                left_witness,
                right_witness,
                removed,
                inserted,
            )?,
            (
                Self::RelationGroupedExactCountRange { .. }
                | Self::RelationGroupedExactF64SumRange { .. }
                | Self::RelationGroupedExactOrderedStatisticRange { .. }
                | Self::RelationGroupedExactAggregateCompare { .. },
                _,
            ) => self.apply_grouped_delta_semantic(
                context, registry, relation, witness, removed, inserted,
            )?,
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }

    fn apply_grouped_delta_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        relation: SemanticId,
        witness: &mut ModelRuleWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        match (self, &mut *witness) {
            (
                Self::RelationGroupedExactCountRange {
                    dependency,
                    group_column_ordinals,
                    group_equivalences,
                    predicate,
                    min,
                    max,
                },
                ModelRuleWitness::RelationGroupedExactCountRange { witness },
            ) => Self::apply_grouped_exact_count_delta_semantic(
                context,
                registry,
                dependency.relation,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                predicate.as_ref(),
                *min,
                *max,
                removed,
                inserted,
            ),
            (
                Self::RelationGroupedExactF64SumRange {
                    dependency,
                    group_column_ordinals,
                    group_equivalences,
                    column_ordinal,
                    predicate,
                    min,
                    max,
                },
                ModelRuleWitness::RelationGroupedExactF64SumRange { witness },
            ) => Self::apply_grouped_exact_f64_sum_delta_semantic(
                context,
                registry,
                dependency.relation,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                *column_ordinal,
                predicate.as_ref(),
                *min,
                *max,
                removed,
                inserted,
            ),
            (
                Self::RelationGroupedExactOrderedStatisticRange {
                    dependency,
                    group_column_ordinals,
                    group_equivalences,
                    measure,
                    ..
                },
                ModelRuleWitness::RelationGroupedExactOrderedStatisticRange { witness, min, max },
            ) => Self::apply_grouped_exact_ordered_statistic_delta_semantic(
                context,
                registry,
                dependency.relation,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                measure,
                min.as_ref(),
                max.as_ref(),
                removed,
                inserted,
            ),
            (
                Self::RelationGroupedExactAggregateCompare {
                    dependency,
                    group_column_ordinals,
                    group_equivalences,
                    left,
                    right,
                    comparison,
                },
                ModelRuleWitness::RelationGroupedExactAggregateCompare { witness },
            ) => Self::apply_grouped_exact_aggregate_compare_delta_semantic(
                context,
                registry,
                dependency.relation,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                left,
                right,
                *comparison,
                removed,
                inserted,
            ),
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }

    fn apply_exact_count_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        predicate: Option<&CompiledRelationPredicate>,
        count: &mut kernel_aggregate::ExactCount,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        for row in removed {
            if Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                count
                    .remove_one()
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }
        for row in inserted {
            if Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                count.add_one();
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_exact_f64_sum_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        predicate: Option<&CompiledRelationPredicate>,
        column_ordinal: usize,
        sum: &mut kernel_aggregate::ExactF64Sum,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        for row in removed {
            if !Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                continue;
            }
            let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                return Err(RuleEvaluationError::TypeMismatch);
            };
            sum.remove(f64::from_bits(*bits))
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        }
        for row in inserted {
            if !Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                continue;
            }
            let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                return Err(RuleEvaluationError::TypeMismatch);
            };
            sum.add(f64::from_bits(*bits))
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_aggregate_compare_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        relation: SemanticId,
        left: &CompiledExactAggregateMeasure,
        right: &CompiledExactAggregateMeasure,
        left_witness: &mut ExactAggregateWitness,
        right_witness: &mut ExactAggregateWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        left.apply_delta_semantic(context, registry, relation, left_witness, removed, inserted)?;
        right.apply_delta_semantic(
            context,
            registry,
            relation,
            right_witness,
            removed,
            inserted,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_grouped_exact_count_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        dependency_relation: SemanticId,
        relation: SemanticId,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        predicate: Option<&CompiledRelationPredicate>,
        min: u64,
        max: Option<u64>,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if dependency_relation != relation {
            return Ok(());
        }
        for row in removed {
            Self::update_grouped_exact_count_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                predicate,
                min,
                max,
                row,
                false,
            )?;
        }
        for row in inserted {
            Self::update_grouped_exact_count_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                predicate,
                min,
                max,
                row,
                true,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_grouped_exact_f64_sum_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        dependency_relation: SemanticId,
        relation: SemanticId,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        column_ordinal: usize,
        predicate: Option<&CompiledRelationPredicate>,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if dependency_relation != relation {
            return Ok(());
        }
        for row in removed {
            Self::update_grouped_exact_f64_sum_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                column_ordinal,
                predicate,
                min,
                max,
                row,
                false,
            )?;
        }
        for row in inserted {
            Self::update_grouped_exact_f64_sum_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                column_ordinal,
                predicate,
                min,
                max,
                row,
                true,
            )?;
        }
        Ok(())
    }

    pub fn witness_violation_mass(
        &self,
        witness: &ModelRuleWitness,
    ) -> Result<u64, RuleEvaluationError> {
        match (self, witness) {
            (
                Self::RelationExactCountRange { min, max, .. },
                ModelRuleWitness::RelationExactCountRange { count },
            ) => Self::count_violation_mass(count, *min, *max),
            (
                Self::RelationExactF64SumRange { min, max, .. },
                ModelRuleWitness::RelationExactF64SumRange { sum },
            ) => {
                use std::cmp::Ordering;
                if let Some(min) = min
                    && sum
                        .cmp_f64_exact(min.value())
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?
                        == Ordering::Less
                {
                    return Ok(1);
                }
                if let Some(max) = max
                    && sum
                        .cmp_f64_exact(max.value())
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?
                        == Ordering::Greater
                {
                    return Ok(1);
                }
                Ok(0)
            }
            (
                Self::RelationExactOrderedStatisticRange { .. },
                ModelRuleWitness::RelationExactOrderedStatisticRange { measure, min, max },
            ) => Self::ordered_statistic_range_violation_mass(measure, min.as_ref(), max.as_ref()),
            (
                Self::ExactAggregateCompare { comparison, .. },
                ModelRuleWitness::ExactAggregateCompare { left, right },
            ) => Self::aggregate_compare_violation_mass(left, right, *comparison),
            (
                Self::RelationGroupedExactCountRange { .. },
                ModelRuleWitness::RelationGroupedExactCountRange { witness },
            )
            | (
                Self::RelationGroupedExactF64SumRange { .. },
                ModelRuleWitness::RelationGroupedExactF64SumRange { witness },
            )
            | (
                Self::RelationGroupedExactOrderedStatisticRange { .. },
                ModelRuleWitness::RelationGroupedExactOrderedStatisticRange { witness, .. },
            ) => Ok(witness.violation_mass),
            (
                Self::RelationGroupedExactAggregateCompare { .. },
                ModelRuleWitness::RelationGroupedExactAggregateCompare { witness },
            ) => Ok(witness.violation_mass),
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }

    fn grouped_key_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        row: &[Value],
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
    ) -> Result<CanonicalGroupKey, RuleEvaluationError> {
        group_column_ordinals
            .iter()
            .zip(group_equivalences)
            .map(|(ordinal, equivalence)| {
                let value = row.get(*ordinal).ok_or(RuleEvaluationError::MissingValue)?;
                registry
                    .canonical_equivalence_key(context, *equivalence, value)
                    .map_err(|_| RuleEvaluationError::Semantic)
            })
            .collect()
    }

    fn grouped_count_bucket_violation_mass(
        bucket: &GroupedExactMeasureBucket,
        min: u64,
        max: Option<u64>,
    ) -> Result<u64, RuleEvaluationError> {
        if bucket.members.is_zero() {
            return Ok(0);
        }
        let ExactAggregateWitness::Count(selected) = &bucket.selected else {
            return Err(RuleEvaluationError::TypeMismatch);
        };
        Self::count_violation_mass(selected, min, max)
    }

    fn grouped_f64_sum_bucket_violation_mass(
        bucket: &GroupedExactMeasureBucket,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    ) -> Result<u64, RuleEvaluationError> {
        use std::cmp::Ordering;
        if bucket.members.is_zero() {
            return Ok(0);
        }
        let ExactAggregateWitness::F64Sum(sum) = &bucket.selected else {
            return Err(RuleEvaluationError::TypeMismatch);
        };
        if let Some(min) = min
            && sum
                .cmp_f64_exact(min.value())
                .map_err(|_| RuleEvaluationError::TypeMismatch)?
                == Ordering::Less
        {
            return Ok(1);
        }
        if let Some(max) = max
            && sum
                .cmp_f64_exact(max.value())
                .map_err(|_| RuleEvaluationError::TypeMismatch)?
                == Ordering::Greater
        {
            return Ok(1);
        }
        Ok(0)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_grouped_exact_count_row(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        predicate: Option<&CompiledRelationPredicate>,
        min: u64,
        max: Option<u64>,
        row: &[Value],
        insert: bool,
    ) -> Result<(), RuleEvaluationError> {
        let key = Self::grouped_key_semantic(
            context,
            registry,
            row,
            group_column_ordinals,
            group_equivalences,
        )?;
        let selected = Self::aggregate_row_selected_semantic(predicate, row, context, registry)?;
        let bucket =
            witness
                .groups
                .entry(key.clone())
                .or_insert_with(|| GroupedExactMeasureBucket {
                    members: kernel_aggregate::ExactCount::default(),
                    selected: ExactAggregateWitness::Count(kernel_aggregate::ExactCount::default()),
                });
        let old_violation = Self::grouped_count_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_sub(old_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;

        if insert {
            bucket.members.add_one();
            if selected {
                let ExactAggregateWitness::Count(count) = &mut bucket.selected else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                count.add_one();
            }
        } else {
            bucket
                .members
                .remove_one()
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            if selected {
                let ExactAggregateWitness::Count(count) = &mut bucket.selected else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                count
                    .remove_one()
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }

        let remove_bucket = bucket.members.is_zero();
        let new_violation = Self::grouped_count_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_add(new_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        if remove_bucket {
            witness.groups.remove(&key);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_grouped_exact_count_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        predicate: Option<&CompiledRelationPredicate>,
        min: u64,
        max: Option<u64>,
    ) -> Result<GroupedExactMeasureWitness, RuleEvaluationError> {
        let mut witness = GroupedExactMeasureWitness::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                Self::update_grouped_exact_count_row(
                    context,
                    registry,
                    &mut witness,
                    group_column_ordinals,
                    group_equivalences,
                    predicate,
                    min,
                    max,
                    row,
                    true,
                )?;
            }
        }
        Ok(witness)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_grouped_exact_f64_sum_row(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        column_ordinal: usize,
        predicate: Option<&CompiledRelationPredicate>,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
        row: &[Value],
        insert: bool,
    ) -> Result<(), RuleEvaluationError> {
        let key = Self::grouped_key_semantic(
            context,
            registry,
            row,
            group_column_ordinals,
            group_equivalences,
        )?;
        let selected = Self::aggregate_row_selected_semantic(predicate, row, context, registry)?;
        let value = if selected {
            let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                return Err(RuleEvaluationError::TypeMismatch);
            };
            Some(f64::from_bits(*bits))
        } else {
            None
        };
        let bucket =
            witness
                .groups
                .entry(key.clone())
                .or_insert_with(|| GroupedExactMeasureBucket {
                    members: kernel_aggregate::ExactCount::default(),
                    selected: ExactAggregateWitness::F64Sum(
                        kernel_aggregate::ExactF64Sum::default(),
                    ),
                });
        let old_violation = Self::grouped_f64_sum_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_sub(old_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;

        if insert {
            bucket.members.add_one();
        } else {
            bucket
                .members
                .remove_one()
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        }
        if let Some(value) = value {
            let ExactAggregateWitness::F64Sum(sum) = &mut bucket.selected else {
                return Err(RuleEvaluationError::TypeMismatch);
            };
            if insert {
                sum.add(value)
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            } else {
                sum.remove(value)
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }

        let remove_bucket = bucket.members.is_zero();
        let new_violation = Self::grouped_f64_sum_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_add(new_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        if remove_bucket {
            witness.groups.remove(&key);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_grouped_exact_f64_sum_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        column_ordinal: usize,
        predicate: Option<&CompiledRelationPredicate>,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    ) -> Result<GroupedExactMeasureWitness, RuleEvaluationError> {
        let mut witness = GroupedExactMeasureWitness::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                Self::update_grouped_exact_f64_sum_row(
                    context,
                    registry,
                    &mut witness,
                    group_column_ordinals,
                    group_equivalences,
                    column_ordinal,
                    predicate,
                    min,
                    max,
                    row,
                    true,
                )?;
            }
        }
        Ok(witness)
    }

    fn grouped_ordered_statistic_bucket_violation_mass(
        bucket: &GroupedExactMeasureBucket,
        min: Option<&kernel_semantics::CanonicalOrderKey>,
        max: Option<&kernel_semantics::CanonicalOrderKey>,
    ) -> Result<u64, RuleEvaluationError> {
        if bucket.members.is_zero() {
            return Ok(0);
        }
        Self::ordered_statistic_range_violation_mass(&bucket.selected, min, max)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_grouped_exact_ordered_statistic_row(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        measure: &CompiledExactAggregateMeasure,
        min: Option<&kernel_semantics::CanonicalOrderKey>,
        max: Option<&kernel_semantics::CanonicalOrderKey>,
        row: &[Value],
        insert: bool,
    ) -> Result<(), RuleEvaluationError> {
        let key = Self::grouped_key_semantic(
            context,
            registry,
            row,
            group_column_ordinals,
            group_equivalences,
        )?;
        let bucket =
            witness
                .groups
                .entry(key.clone())
                .or_insert_with(|| GroupedExactMeasureBucket {
                    members: kernel_aggregate::ExactCount::default(),
                    selected: measure.zero_witness(),
                });
        let old_violation =
            Self::grouped_ordered_statistic_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_sub(old_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;

        if insert {
            bucket.members.add_one();
        } else {
            bucket
                .members
                .remove_one()
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        }
        measure.apply_row_semantic(context, registry, &mut bucket.selected, row, insert)?;

        let remove_bucket = bucket.members.is_zero();
        let new_violation =
            Self::grouped_ordered_statistic_bucket_violation_mass(bucket, min, max)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_add(new_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        if remove_bucket {
            witness.groups.remove(&key);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_grouped_exact_ordered_statistic_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        measure: &CompiledExactAggregateMeasure,
        min: Option<&kernel_semantics::CanonicalOrderKey>,
        max: Option<&kernel_semantics::CanonicalOrderKey>,
    ) -> Result<GroupedExactMeasureWitness, RuleEvaluationError> {
        let mut witness = GroupedExactMeasureWitness::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                Self::update_grouped_exact_ordered_statistic_row(
                    context,
                    registry,
                    &mut witness,
                    group_column_ordinals,
                    group_equivalences,
                    measure,
                    min,
                    max,
                    row,
                    true,
                )?;
            }
        }
        Ok(witness)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_grouped_exact_ordered_statistic_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        dependency_relation: SemanticId,
        relation: SemanticId,
        witness: &mut GroupedExactMeasureWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        measure: &CompiledExactAggregateMeasure,
        min: Option<&kernel_semantics::CanonicalOrderKey>,
        max: Option<&kernel_semantics::CanonicalOrderKey>,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if dependency_relation != relation {
            return Ok(());
        }
        for row in removed {
            Self::update_grouped_exact_ordered_statistic_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                measure,
                min,
                max,
                row,
                false,
            )?;
        }
        for row in inserted {
            Self::update_grouped_exact_ordered_statistic_row(
                context,
                registry,
                witness,
                group_column_ordinals,
                group_equivalences,
                measure,
                min,
                max,
                row,
                true,
            )?;
        }
        Ok(())
    }

    fn grouped_aggregate_compare_bucket_violation_mass(
        bucket: &GroupedExactMeasureProductBucket,
        comparison: RuleOrderComparison,
    ) -> Result<u64, RuleEvaluationError> {
        if bucket.members.is_zero() {
            return Ok(0);
        }
        Self::aggregate_compare_violation_mass(&bucket.left, &bucket.right, comparison)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_grouped_exact_aggregate_compare_row(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        _relation: SemanticId,
        witness: &mut GroupedExactMeasureProductWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        left: &CompiledExactAggregateMeasure,
        right: &CompiledExactAggregateMeasure,
        comparison: RuleOrderComparison,
        row: &[Value],
        insert: bool,
    ) -> Result<(), RuleEvaluationError> {
        let key = Self::grouped_key_semantic(
            context,
            registry,
            row,
            group_column_ordinals,
            group_equivalences,
        )?;
        let bucket =
            witness
                .groups
                .entry(key.clone())
                .or_insert_with(|| GroupedExactMeasureProductBucket {
                    members: kernel_aggregate::ExactCount::default(),
                    left: left.zero_witness(),
                    right: right.zero_witness(),
                });
        let old_violation =
            Self::grouped_aggregate_compare_bucket_violation_mass(bucket, comparison)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_sub(old_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;

        if insert {
            bucket.members.add_one();
            left.apply_row_semantic(context, registry, &mut bucket.left, row, true)?;
            right.apply_row_semantic(context, registry, &mut bucket.right, row, true)?;
        } else {
            left.apply_row_semantic(context, registry, &mut bucket.left, row, false)?;
            right.apply_row_semantic(context, registry, &mut bucket.right, row, false)?;
            bucket
                .members
                .remove_one()
                .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        }

        let remove_bucket = bucket.members.is_zero();
        let new_violation =
            Self::grouped_aggregate_compare_bucket_violation_mass(bucket, comparison)?;
        witness.violation_mass = witness
            .violation_mass
            .checked_add(new_violation)
            .ok_or(RuleEvaluationError::TypeMismatch)?;
        if remove_bucket {
            witness.groups.remove(&key);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_grouped_exact_aggregate_compare_witness(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        left: &CompiledExactAggregateMeasure,
        right: &CompiledExactAggregateMeasure,
        comparison: RuleOrderComparison,
    ) -> Result<GroupedExactMeasureProductWitness, RuleEvaluationError> {
        let mut witness = GroupedExactMeasureProductWitness::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                Self::update_grouped_exact_aggregate_compare_row(
                    context,
                    registry,
                    relation,
                    &mut witness,
                    group_column_ordinals,
                    group_equivalences,
                    left,
                    right,
                    comparison,
                    row,
                    true,
                )?;
            }
        }
        Ok(witness)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_grouped_exact_aggregate_compare_delta_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        dependency_relation: SemanticId,
        relation: SemanticId,
        witness: &mut GroupedExactMeasureProductWitness,
        group_column_ordinals: &[usize],
        group_equivalences: &[SemanticId],
        left: &CompiledExactAggregateMeasure,
        right: &CompiledExactAggregateMeasure,
        comparison: RuleOrderComparison,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        if dependency_relation != relation {
            return Ok(());
        }
        for row in removed {
            Self::update_grouped_exact_aggregate_compare_row(
                context,
                registry,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                left,
                right,
                comparison,
                row,
                false,
            )?;
        }
        for row in inserted {
            Self::update_grouped_exact_aggregate_compare_row(
                context,
                registry,
                relation,
                witness,
                group_column_ordinals,
                group_equivalences,
                left,
                right,
                comparison,
                row,
                true,
            )?;
        }
        Ok(())
    }

    fn select_ordered_statistic(
        values: &kernel_aggregate::ExactOrderedMultiset<kernel_semantics::CanonicalOrderKey>,
        selector: OrderedStatisticSelector,
    ) -> Option<&kernel_semantics::CanonicalOrderKey> {
        match selector {
            OrderedStatisticSelector::FromStart(rank) => values.select_from_start(rank),
            OrderedStatisticSelector::FromEnd(rank) => values.select_from_end(rank),
            OrderedStatisticSelector::LowerQuantile {
                numerator,
                denominator,
            } => values.select_lower_quantile(numerator, denominator),
        }
    }

    fn ordered_statistic_bound_value(bound: &OrderedStatisticBound) -> Value {
        match bound {
            OrderedStatisticBound::Unit => Value::Unit,
            OrderedStatisticBound::Bool(value) => Value::Bool(*value),
            OrderedStatisticBound::I64(value) => Value::I64(*value),
            OrderedStatisticBound::F64Bits(value) => Value::F64Bits(*value),
            OrderedStatisticBound::Text(value) => Value::Text(value.clone()),
            OrderedStatisticBound::LiveEntityId { entity_type, id } => Value::LiveEntityRef {
                entity_type: *entity_type,
                id: *id,
            },
            OrderedStatisticBound::HistoricalEntityId { entity_type, id } => {
                Value::HistoricalEntityId {
                    entity_type: *entity_type,
                    id: *id,
                }
            }
        }
    }

    fn canonicalize_ordered_statistic_bound(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        ordering: SemanticId,
        bound: &OrderedStatisticBound,
    ) -> Result<kernel_semantics::CanonicalOrderKey, RuleEvaluationError> {
        registry
            .canonical_order_key(
                context,
                ordering,
                &Self::ordered_statistic_bound_value(bound),
            )
            .map_err(|_| RuleEvaluationError::Semantic)
    }

    fn canonicalize_ordered_statistic_range(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        ordering: SemanticId,
        min: Option<&OrderedStatisticBound>,
        max: Option<&OrderedStatisticBound>,
    ) -> Result<
        (
            Option<kernel_semantics::CanonicalOrderKey>,
            Option<kernel_semantics::CanonicalOrderKey>,
        ),
        RuleEvaluationError,
    > {
        let min = min
            .map(|bound| {
                Self::canonicalize_ordered_statistic_bound(context, registry, ordering, bound)
            })
            .transpose()?;
        let max = max
            .map(|bound| {
                Self::canonicalize_ordered_statistic_bound(context, registry, ordering, bound)
            })
            .transpose()?;
        if min
            .as_ref()
            .zip(max.as_ref())
            .is_some_and(|(min, max)| min > max)
        {
            return Err(RuleEvaluationError::TypeMismatch);
        }
        Ok((min, max))
    }

    fn ordered_statistic_range_violation_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        measure: &CompiledExactAggregateMeasure,
        min: Option<&OrderedStatisticBound>,
        max: Option<&OrderedStatisticBound>,
    ) -> Result<u64, RuleEvaluationError> {
        let ordering = match measure {
            CompiledExactAggregateMeasure::OrderedStatistic { ordering, .. } => *ordering,
            _ => return Err(RuleEvaluationError::TypeMismatch),
        };
        let witness = measure.build_witness(context, registry, state)?;
        let (min, max) =
            Self::canonicalize_ordered_statistic_range(context, registry, ordering, min, max)?;
        Self::ordered_statistic_range_violation_mass(&witness, min.as_ref(), max.as_ref())
    }

    fn ordered_statistic_range_violation_mass(
        witness: &ExactAggregateWitness,
        min: Option<&kernel_semantics::CanonicalOrderKey>,
        max: Option<&kernel_semantics::CanonicalOrderKey>,
    ) -> Result<u64, RuleEvaluationError> {
        let ExactAggregateWitness::OrderedStatistic { values, selector } = witness else {
            return Err(RuleEvaluationError::TypeMismatch);
        };
        let selected = Self::select_ordered_statistic(values, *selector);
        let Some(selected) = selected else {
            return Ok(0);
        };
        if min.is_some_and(|min| selected < min) || max.is_some_and(|max| selected > max) {
            return Ok(1);
        }
        Ok(0)
    }

    fn aggregate_compare_violation_mass(
        left: &ExactAggregateWitness,
        right: &ExactAggregateWitness,
        comparison: RuleOrderComparison,
    ) -> Result<u64, RuleEvaluationError> {
        use std::cmp::Ordering;

        match (left, right) {
            (ExactAggregateWitness::Count(left), ExactAggregateWitness::Count(right)) => {
                let left = left
                    .finish_u64()
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                let right = right
                    .finish_u64()
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                Ok(match comparison {
                    RuleOrderComparison::Less => {
                        if left < right {
                            0
                        } else {
                            left.saturating_sub(right).saturating_add(1)
                        }
                    }
                    RuleOrderComparison::LessOrEqual => left.saturating_sub(right),
                    RuleOrderComparison::Greater => {
                        if left > right {
                            0
                        } else {
                            right.saturating_sub(left).saturating_add(1)
                        }
                    }
                    RuleOrderComparison::GreaterOrEqual => right.saturating_sub(left),
                })
            }
            (ExactAggregateWitness::F64Sum(left), ExactAggregateWitness::F64Sum(right)) => {
                let ordering = left.cmp_exact(right);
                let satisfied = match comparison {
                    RuleOrderComparison::Less => ordering == Ordering::Less,
                    RuleOrderComparison::LessOrEqual => ordering != Ordering::Greater,
                    RuleOrderComparison::Greater => ordering == Ordering::Greater,
                    RuleOrderComparison::GreaterOrEqual => ordering != Ordering::Less,
                };
                Ok(u64::from(!satisfied))
            }
            (
                ExactAggregateWitness::OrderedStatistic {
                    values: left,
                    selector: left_selector,
                },
                ExactAggregateWitness::OrderedStatistic {
                    values: right,
                    selector: right_selector,
                },
            ) => {
                let left = Self::select_ordered_statistic(left, *left_selector);
                let right = Self::select_ordered_statistic(right, *right_selector);
                let (left, right) = match (left, right) {
                    (None, None) => return Ok(0),
                    (Some(left), Some(right)) => (left, right),
                    (None, Some(_)) | (Some(_), None) => return Ok(1),
                };
                let ordering = left.cmp(right);
                let satisfied = match comparison {
                    RuleOrderComparison::Less => ordering == Ordering::Less,
                    RuleOrderComparison::LessOrEqual => ordering != Ordering::Greater,
                    RuleOrderComparison::Greater => ordering == Ordering::Greater,
                    RuleOrderComparison::GreaterOrEqual => ordering != Ordering::Less,
                };
                Ok(u64::from(!satisfied))
            }
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }

    fn count_violation_mass(
        count: &kernel_aggregate::ExactCount,
        min: u64,
        max: Option<u64>,
    ) -> Result<u64, RuleEvaluationError> {
        let count = count
            .finish_u64()
            .map_err(|_| RuleEvaluationError::TypeMismatch)?;
        if count < min {
            Ok(min - count)
        } else if let Some(max) = max {
            Ok(count.saturating_sub(max))
        } else {
            Ok(0)
        }
    }

    fn exact_count(
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        predicate: Option<&CompiledRelationPredicate>,
    ) -> Result<kernel_aggregate::ExactCount, RuleEvaluationError> {
        let mut count = kernel_aggregate::ExactCount::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                if Self::aggregate_row_selected(predicate, row)? {
                    count.add_one();
                }
            }
        }
        Ok(count)
    }

    fn exact_count_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        predicate: Option<&CompiledRelationPredicate>,
    ) -> Result<kernel_aggregate::ExactCount, RuleEvaluationError> {
        let mut count = kernel_aggregate::ExactCount::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                if Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                    count.add_one();
                }
            }
        }
        Ok(count)
    }

    fn exact_f64_sum(
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        column_ordinal: usize,
        predicate: Option<&CompiledRelationPredicate>,
    ) -> Result<kernel_aggregate::ExactF64Sum, RuleEvaluationError> {
        let mut sum = kernel_aggregate::ExactF64Sum::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                if !Self::aggregate_row_selected(predicate, row)? {
                    continue;
                }
                let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                sum.add(f64::from_bits(*bits))
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }
        Ok(sum)
    }

    fn exact_f64_sum_semantic(
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        column_ordinal: usize,
        predicate: Option<&CompiledRelationPredicate>,
    ) -> Result<kernel_aggregate::ExactF64Sum, RuleEvaluationError> {
        let mut sum = kernel_aggregate::ExactF64Sum::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                if !Self::aggregate_row_selected_semantic(predicate, row, context, registry)? {
                    continue;
                }
                let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                sum.add(f64::from_bits(*bits))
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }
        Ok(sum)
    }

    fn aggregate_row_selected(
        predicate: Option<&CompiledRelationPredicate>,
        row: &[Value],
    ) -> Result<bool, RuleEvaluationError> {
        match predicate {
            None => Ok(true),
            Some(predicate) => predicate.matches(row),
        }
    }

    fn aggregate_row_selected_semantic(
        predicate: Option<&CompiledRelationPredicate>,
        row: &[Value],
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RuleEvaluationError> {
        match predicate {
            None => Ok(true),
            Some(predicate) => predicate.matches_semantic(row, context, registry),
        }
    }

    pub fn is_satisfied(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<bool, RuleEvaluationError> {
        Ok(self.violation_mass(state)? == 0)
    }

    pub fn is_satisfied_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<bool, RuleEvaluationError> {
        Ok(self.violation_mass_semantic(context, registry, state)? == 0)
    }

    pub fn violation_mass_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<u64, RuleEvaluationError> {
        match self {
            Self::RelationExactCountRange {
                dependency,
                predicate,
                min,
                max,
            } => {
                let count = Self::exact_count_semantic(
                    context,
                    registry,
                    state,
                    dependency.relation,
                    predicate.as_ref(),
                )?;
                Self::count_violation_mass(&count, *min, *max)
            }
            Self::RelationExactF64SumRange { .. } => Ok(u64::from(
                !self.is_satisfied_semantic_sum(context, registry, state)?,
            )),
            Self::RelationExactOrderedStatisticRange { measure, min, max } => {
                Self::ordered_statistic_range_violation_semantic(
                    context,
                    registry,
                    state,
                    measure,
                    min.as_ref(),
                    max.as_ref(),
                )
            }
            Self::ExactAggregateCompare {
                left,
                right,
                comparison,
            } => {
                let left = left.build_witness(context, registry, state)?;
                let right = right.build_witness(context, registry, state)?;
                Self::aggregate_compare_violation_mass(&left, &right, *comparison)
            }
            Self::RelationGroupedExactCountRange { .. }
            | Self::RelationGroupedExactF64SumRange { .. }
            | Self::RelationGroupedExactOrderedStatisticRange { .. }
            | Self::RelationGroupedExactAggregateCompare { .. } => {
                self.grouped_violation_mass_semantic(context, registry, state)
            }
        }
    }

    fn grouped_violation_mass_semantic(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<u64, RuleEvaluationError> {
        self.build_grouped_witness(context, registry, state)?
            .violation_mass_value()
    }

    fn is_satisfied_semantic_sum(
        &self,
        context: &SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        state: &kernel_model::DatabaseState,
    ) -> Result<bool, RuleEvaluationError> {
        use std::cmp::Ordering;

        let Self::RelationExactF64SumRange {
            dependency,
            column_ordinal,
            predicate,
            min,
            max,
        } = self
        else {
            return Err(RuleEvaluationError::TypeMismatch);
        };
        let sum = Self::exact_f64_sum_semantic(
            context,
            registry,
            state,
            dependency.relation,
            *column_ordinal,
            predicate.as_ref(),
        )?;
        if let Some(min) = min
            && sum
                .cmp_f64_exact(min.value())
                .map_err(|_| RuleEvaluationError::TypeMismatch)?
                == Ordering::Less
        {
            return Ok(false);
        }
        if let Some(max) = max
            && sum
                .cmp_f64_exact(max.value())
                .map_err(|_| RuleEvaluationError::TypeMismatch)?
                == Ordering::Greater
        {
            return Ok(false);
        }
        Ok(true)
    }

    pub fn violation_mass(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<u64, RuleEvaluationError> {
        match self {
            Self::RelationExactCountRange {
                dependency,
                predicate,
                min,
                max,
            } => {
                let count = Self::exact_count(state, dependency.relation, predicate.as_ref())?;
                Self::count_violation_mass(&count, *min, *max)
            }
            Self::RelationExactF64SumRange {
                dependency,
                column_ordinal,
                predicate,
                min,
                max,
            } => {
                use std::cmp::Ordering;
                let sum = Self::exact_f64_sum(
                    state,
                    dependency.relation,
                    *column_ordinal,
                    predicate.as_ref(),
                )?;
                if let Some(min) = min
                    && sum
                        .cmp_f64_exact(min.value())
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?
                        == Ordering::Less
                {
                    return Ok(1);
                }
                if let Some(max) = max
                    && sum
                        .cmp_f64_exact(max.value())
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?
                        == Ordering::Greater
                {
                    return Ok(1);
                }
                Ok(0)
            }
            Self::RelationExactOrderedStatisticRange { .. } => Err(RuleEvaluationError::Semantic),
            Self::ExactAggregateCompare {
                left,
                right,
                comparison,
            } => {
                let left = left.build_witness_unchecked(state)?;
                let right = right.build_witness_unchecked(state)?;
                Self::aggregate_compare_violation_mass(&left, &right, *comparison)
            }
            Self::RelationGroupedExactCountRange { .. }
            | Self::RelationGroupedExactF64SumRange { .. }
            | Self::RelationGroupedExactOrderedStatisticRange { .. }
            | Self::RelationGroupedExactAggregateCompare { .. } => {
                Err(RuleEvaluationError::Semantic)
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct CompiledRulePlan {
    field_rules: BTreeMap<SemanticId, Vec<CompiledFieldRule>>,
    relation_column_rules: BTreeMap<(SemanticId, SemanticId), Vec<CompiledFieldRule>>,
    entity_rules: BTreeMap<SemanticId, Vec<CompiledSemanticRuleExpr>>,
    model_rules: Vec<CompiledModelRule>,
    model_rules_by_relation: BTreeMap<SemanticId, Vec<usize>>,
}

impl CompiledRulePlan {
    #[must_use]
    pub fn compile(context: &SemanticContext) -> Self {
        let mut plan = Self::default();
        for (field, rule) in context.schema.all_field_rules() {
            plan.field_rules
                .entry(field)
                .or_default()
                .push(CompiledFieldRule::compile(rule));
        }
        for (target, rule) in context.schema.all_relation_column_rules() {
            plan.relation_column_rules
                .entry(target)
                .or_default()
                .push(CompiledFieldRule::compile(rule));
        }
        for (owner, rule) in context.schema.all_entity_rules() {
            plan.entity_rules
                .entry(owner)
                .or_default()
                .push(CompiledSemanticRuleExpr::compile(rule));
        }
        plan.model_rules.extend(
            context
                .schema
                .model_rules()
                .iter()
                .map(|rule| CompiledModelRule::compile(rule, context)),
        );
        for (rule_index, rule) in plan.model_rules.iter().enumerate() {
            for relation in rule.dependency_relations() {
                plan.model_rules_by_relation
                    .entry(relation)
                    .or_default()
                    .push(rule_index);
            }
        }
        plan
    }

    #[must_use]
    pub fn field_rules(&self, field: SemanticId) -> &[CompiledFieldRule] {
        self.field_rules
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
        self.relation_column_rules
            .get(&(relation, column))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn entity_rules(&self, owner: SemanticId) -> &[CompiledSemanticRuleExpr] {
        self.entity_rules
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn entity_rule_owners(&self) -> impl Iterator<Item = SemanticId> + '_ {
        self.entity_rules.keys().copied()
    }

    #[must_use]
    pub fn model_rules(&self) -> &[CompiledModelRule] {
        &self.model_rules
    }

    pub fn model_rules_for_relation(
        &self,
        relation: SemanticId,
    ) -> impl Iterator<Item = (usize, &CompiledModelRule)> {
        self.model_rules_by_relation
            .get(&relation)
            .into_iter()
            .flatten()
            .map(|&rule_index| (rule_index, &self.model_rules[rule_index]))
    }

    pub fn model_rules_for_mutation<'a>(
        &'a self,
        relation: SemanticId,
        footprint: &'a RelationMutationFootprint,
    ) -> impl Iterator<Item = (usize, &'a CompiledModelRule)> + 'a {
        self.model_rules_for_relation(relation)
            .filter(move |(_, rule)| rule.affected_by(relation, footprint))
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

pub fn semantic_rule_matches_fields(
    rule: &SemanticRuleExpr,
    fields: &std::collections::BTreeMap<kernel_types::SemanticId, Value>,
) -> Result<bool, RuleEvaluationError> {
    CompiledSemanticRuleExpr::compile(rule).matches(&|coordinate| match coordinate {
        RuleValueExpr::Input => None,
        RuleValueExpr::Field(field) => fields.get(field),
    })
}

pub fn relation_row_rule_matches(
    rule: &SemanticRuleExpr,
    relation: kernel_types::SemanticId,
    row: &[Value],
    context: &SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RuleEvaluationError> {
    CompiledSemanticRuleExpr::compile(rule).matches_semantic(
        &|coordinate| match coordinate {
            RuleValueExpr::Input => None,
            RuleValueExpr::Field(field) => context
                .schema
                .relation_column_ordinal(relation, *field)
                .and_then(|column| row.get(column)),
        },
        context,
        registry,
    )
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
