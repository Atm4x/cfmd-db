use kernel_model::Value;
use std::collections::{BTreeMap, BTreeSet};

use kernel_schema::{
    FieldRule, FiniteF64, ModelRuleExpr, RuleValueExpr, SemanticContext, SemanticRuleExpr,
    TextPattern,
};
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRuleDependency {
    relation: SemanticId,
    columns: BTreeSet<SemanticId>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRuleWitness {
    RelationCardinality { count: u64 },
    RelationExists { matching: u64 },
    RelationAll { violating: u64 },
    RelationExactF64SumRange { sum: kernel_aggregate::ExactF64Sum },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelRuleWitnessState {
    witnesses: Vec<ModelRuleWitness>,
}

impl ModelRuleWitnessState {
    pub fn build(
        context: &SemanticContext,
        state: &kernel_model::DatabaseState,
    ) -> Result<Self, RuleEvaluationError> {
        let plan = CompiledRulePlan::compile(context);
        let witnesses = plan
            .model_rules()
            .iter()
            .map(|rule| rule.build_witness(state))
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
            rule.apply_delta(&mut next.witnesses[rule_index], removed, inserted)?;
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
    RelationCardinality {
        dependency: ModelRuleDependency,
        min: u64,
        max: Option<u64>,
    },
    RelationExists {
        dependency: ModelRuleDependency,
        predicate: CompiledRelationPredicate,
    },
    RelationAll {
        dependency: ModelRuleDependency,
        predicate: CompiledRelationPredicate,
    },
    RelationExactF64SumRange {
        dependency: ModelRuleDependency,
        column_ordinal: usize,
        min: Option<FiniteF64>,
        max: Option<FiniteF64>,
    },
}

impl CompiledModelRule {
    #[must_use]
    pub fn compile(rule: &ModelRuleExpr, context: &SemanticContext) -> Self {
        match rule {
            ModelRuleExpr::RelationCardinality { relation, min, max } => {
                Self::RelationCardinality {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns: BTreeSet::new(),
                    },
                    min: *min,
                    max: *max,
                }
            }
            ModelRuleExpr::RelationExists {
                relation,
                predicate,
            } => {
                let predicate = CompiledRelationPredicate::compile(*relation, predicate, context);
                let columns = predicate.columns().collect();
                Self::RelationExists {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    predicate,
                }
            }
            ModelRuleExpr::RelationAll {
                relation,
                predicate,
            } => {
                let predicate = CompiledRelationPredicate::compile(*relation, predicate, context);
                let columns = predicate.columns().collect();
                Self::RelationAll {
                    dependency: ModelRuleDependency {
                        relation: *relation,
                        columns,
                    },
                    predicate,
                }
            }
            ModelRuleExpr::RelationExactF64SumRange {
                relation,
                column,
                min,
                max,
            } => Self::RelationExactF64SumRange {
                dependency: ModelRuleDependency {
                    relation: *relation,
                    columns: BTreeSet::from([*column]),
                },
                column_ordinal: context
                    .schema
                    .relation_column_ordinal(*relation, *column)
                    .expect("schema-validated aggregate column exists"),
                min: *min,
                max: *max,
            },
        }
    }

    #[must_use]
    pub const fn dependency(&self) -> &ModelRuleDependency {
        match self {
            Self::RelationCardinality { dependency, .. }
            | Self::RelationExists { dependency, .. }
            | Self::RelationAll { dependency, .. }
            | Self::RelationExactF64SumRange { dependency, .. } => dependency,
        }
    }

    #[must_use]
    pub const fn dependency_relation(&self) -> SemanticId {
        self.dependency().relation
    }

    pub fn build_witness(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<ModelRuleWitness, RuleEvaluationError> {
        match self {
            Self::RelationCardinality { dependency, .. } => {
                let count = state
                    .model
                    .relations
                    .get_shared(&dependency.relation)
                    .map_or(0, kernel_model::SharedRelationRows::len);
                Ok(ModelRuleWitness::RelationCardinality {
                    count: u64::try_from(count).map_err(|_| RuleEvaluationError::TypeMismatch)?,
                })
            }
            Self::RelationExists {
                dependency,
                predicate,
            } => {
                let mut matching = 0u64;
                if let Some(rows) = state.model.relations.get_shared(&dependency.relation) {
                    for row in rows {
                        if predicate.matches(row)? {
                            matching = matching
                                .checked_add(1)
                                .ok_or(RuleEvaluationError::TypeMismatch)?;
                        }
                    }
                }
                Ok(ModelRuleWitness::RelationExists { matching })
            }
            Self::RelationAll {
                dependency,
                predicate,
            } => {
                let mut violating = 0u64;
                if let Some(rows) = state.model.relations.get_shared(&dependency.relation) {
                    for row in rows {
                        if !predicate.matches(row)? {
                            violating = violating
                                .checked_add(1)
                                .ok_or(RuleEvaluationError::TypeMismatch)?;
                        }
                    }
                }
                Ok(ModelRuleWitness::RelationAll { violating })
            }
            Self::RelationExactF64SumRange {
                dependency,
                column_ordinal,
                ..
            } => Ok(ModelRuleWitness::RelationExactF64SumRange {
                sum: Self::exact_f64_sum(state, dependency.relation, *column_ordinal)?,
            }),
        }
    }

    pub fn apply_delta(
        &self,
        witness: &mut ModelRuleWitness,
        removed: &[Vec<Value>],
        inserted: &[Vec<Value>],
    ) -> Result<(), RuleEvaluationError> {
        match (self, witness) {
            (Self::RelationCardinality { .. }, ModelRuleWitness::RelationCardinality { count }) => {
                let removed =
                    u64::try_from(removed.len()).map_err(|_| RuleEvaluationError::TypeMismatch)?;
                let inserted =
                    u64::try_from(inserted.len()).map_err(|_| RuleEvaluationError::TypeMismatch)?;
                *count = count
                    .checked_sub(removed)
                    .and_then(|value| value.checked_add(inserted))
                    .ok_or(RuleEvaluationError::TypeMismatch)?;
            }
            (
                Self::RelationExists { predicate, .. },
                ModelRuleWitness::RelationExists { matching },
            ) => {
                for row in removed {
                    if predicate.matches(row)? {
                        *matching = matching
                            .checked_sub(1)
                            .ok_or(RuleEvaluationError::TypeMismatch)?;
                    }
                }
                for row in inserted {
                    if predicate.matches(row)? {
                        *matching = matching
                            .checked_add(1)
                            .ok_or(RuleEvaluationError::TypeMismatch)?;
                    }
                }
            }
            (Self::RelationAll { predicate, .. }, ModelRuleWitness::RelationAll { violating }) => {
                for row in removed {
                    if !predicate.matches(row)? {
                        *violating = violating
                            .checked_sub(1)
                            .ok_or(RuleEvaluationError::TypeMismatch)?;
                    }
                }
                for row in inserted {
                    if !predicate.matches(row)? {
                        *violating = violating
                            .checked_add(1)
                            .ok_or(RuleEvaluationError::TypeMismatch)?;
                    }
                }
            }
            (
                Self::RelationExactF64SumRange { column_ordinal, .. },
                ModelRuleWitness::RelationExactF64SumRange { sum },
            ) => {
                for row in removed {
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.remove(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
                for row in inserted {
                    let Some(Value::F64Bits(bits)) = row.get(*column_ordinal) else {
                        return Err(RuleEvaluationError::TypeMismatch);
                    };
                    sum.add(f64::from_bits(*bits))
                        .map_err(|_| RuleEvaluationError::TypeMismatch)?;
                }
            }
            _ => return Err(RuleEvaluationError::TypeMismatch),
        }
        Ok(())
    }

    pub fn witness_violation_mass(
        &self,
        witness: &ModelRuleWitness,
    ) -> Result<u64, RuleEvaluationError> {
        match (self, witness) {
            (
                Self::RelationCardinality { min, max, .. },
                ModelRuleWitness::RelationCardinality { count },
            ) => {
                if *count < *min {
                    Ok(*min - *count)
                } else if let Some(max) = max {
                    Ok(count.saturating_sub(*max))
                } else {
                    Ok(0)
                }
            }
            (Self::RelationExists { .. }, ModelRuleWitness::RelationExists { matching }) => {
                Ok(u64::from(*matching == 0))
            }
            (Self::RelationAll { .. }, ModelRuleWitness::RelationAll { violating }) => {
                Ok(*violating)
            }
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
            _ => Err(RuleEvaluationError::TypeMismatch),
        }
    }

    fn exact_f64_sum(
        state: &kernel_model::DatabaseState,
        relation: SemanticId,
        column_ordinal: usize,
    ) -> Result<kernel_aggregate::ExactF64Sum, RuleEvaluationError> {
        let mut sum = kernel_aggregate::ExactF64Sum::default();
        if let Some(rows) = state.model.relations.get_shared(&relation) {
            for row in rows {
                let Some(Value::F64Bits(bits)) = row.get(column_ordinal) else {
                    return Err(RuleEvaluationError::TypeMismatch);
                };
                sum.add(f64::from_bits(*bits))
                    .map_err(|_| RuleEvaluationError::TypeMismatch)?;
            }
        }
        Ok(sum)
    }

    pub fn is_satisfied(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<bool, RuleEvaluationError> {
        match self {
            Self::RelationCardinality {
                dependency,
                min,
                max,
            } => {
                let count = state
                    .model
                    .relations
                    .get_shared(&dependency.relation)
                    .map_or(0, kernel_model::SharedRelationRows::len);
                let count = u64::try_from(count).map_err(|_| RuleEvaluationError::TypeMismatch)?;
                Ok(count >= *min && max.is_none_or(|max| count <= max))
            }
            Self::RelationExists {
                dependency,
                predicate,
            } => {
                let Some(rows) = state.model.relations.get_shared(&dependency.relation) else {
                    return Ok(false);
                };
                for row in rows {
                    if predicate.matches(row)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::RelationAll {
                dependency,
                predicate,
            } => {
                let Some(rows) = state.model.relations.get_shared(&dependency.relation) else {
                    return Ok(true);
                };
                for row in rows {
                    if !predicate.matches(row)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::RelationExactF64SumRange {
                dependency,
                column_ordinal,
                min,
                max,
            } => {
                use std::cmp::Ordering;
                let sum = Self::exact_f64_sum(state, dependency.relation, *column_ordinal)?;
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
        }
    }

    pub fn violation_mass(
        &self,
        state: &kernel_model::DatabaseState,
    ) -> Result<u64, RuleEvaluationError> {
        match self {
            Self::RelationCardinality {
                dependency,
                min,
                max,
            } => {
                let count = state
                    .model
                    .relations
                    .get_shared(&dependency.relation)
                    .map_or(0, kernel_model::SharedRelationRows::len);
                let count = u64::try_from(count).map_err(|_| RuleEvaluationError::TypeMismatch)?;
                if count < *min {
                    Ok(*min - count)
                } else if let Some(max) = max {
                    Ok(count.saturating_sub(*max))
                } else {
                    Ok(0)
                }
            }
            Self::RelationExists {
                dependency,
                predicate,
            } => {
                let Some(rows) = state.model.relations.get_shared(&dependency.relation) else {
                    return Ok(1);
                };
                for row in rows {
                    if predicate.matches(row)? {
                        return Ok(0);
                    }
                }
                Ok(1)
            }
            Self::RelationAll {
                dependency,
                predicate,
            } => {
                let Some(rows) = state.model.relations.get_shared(&dependency.relation) else {
                    return Ok(0);
                };
                let mut mass = 0u64;
                for row in rows {
                    if !predicate.matches(row)? {
                        mass = mass
                            .checked_add(1)
                            .ok_or(RuleEvaluationError::TypeMismatch)?;
                    }
                }
                Ok(mass)
            }
            Self::RelationExactF64SumRange { .. } => Ok(u64::from(!self.is_satisfied(state)?)),
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
            plan.model_rules_by_relation
                .entry(rule.dependency_relation())
                .or_default()
                .push(rule_index);
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
            .filter(move |(_, rule)| footprint.affects(rule.dependency()))
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
