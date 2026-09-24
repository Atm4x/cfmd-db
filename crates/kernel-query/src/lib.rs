use std::collections::{BTreeMap, BTreeSet};
mod delta_abi;
mod execgraph;
mod linear_island;
mod sealed_group_v3;
mod topk_i64;
pub use delta_abi::{
    AdaptiveDelta, BinaryDeltaKernel, CompactDelta, CompiledDeltaEdgeIdentity, DeltaSink,
    DeltaView, InlineDelta, PlannedDeltaEffect, RelationDeltaView, UnaryDeltaKernel,
    ValidatedTransitionFrame, Weighted,
};
pub use execgraph::{
    ExecutionInputSlot, NodeId, NodeInbox, PreparedRelGraph, UnifiedTransitionProgram,
    UnifiedTransitionScratch,
};
pub use linear_island::{
    BarrierKernelClass, CompiledDeltaProgram, LinearIslandNormalForm, LinearIslandPredicate,
};

use std::sync::Arc;

use kernel_change::{
    Change, FineChange, FineChangeKind, PreparedRewrite, RewriteEffect, RewriteSpec,
    SeqChangeError, SeqSplice,
};
use kernel_model::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impact {
    Unaffected,
    Changed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    TypeMismatch,
    IndexOutOfBounds,
    ArithmeticOverflow,
    LengthOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryTypeError {
    TypeMismatch,
    UnknownField(kernel_types::SemanticId),
    AmbiguousConstantType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Input,
    Const(Value),
    TypedConst {
        value: Value,
        ty: kernel_schema::TypeExpr,
    },
    ProductField {
        input: Box<Self>,
        field: kernel_types::SemanticId,
    },
    SeqLength(Box<Self>),
    SeqSumI64(Box<Self>),
    AddI64(Box<Self>, Box<Self>),
    If {
        condition: Box<Self>,
        when_true: Box<Self>,
        when_false: Box<Self>,
    },
}

enum EvalValue<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl EvalValue<'_> {
    fn as_ref(&self) -> &Value {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }

    fn into_owned(self) -> Value {
        match self {
            Self::Borrowed(value) => value.clone(),
            Self::Owned(value) => value,
        }
    }
}

impl Expr {
    pub fn evaluate(&self, input: &Value) -> Result<Value, QueryError> {
        Ok(self.evaluate_internal(input)?.into_owned())
    }

    fn evaluate_internal<'a>(&self, input: &'a Value) -> Result<EvalValue<'a>, QueryError> {
        match self {
            Self::Input => Ok(EvalValue::Borrowed(input)),
            Self::Const(value) | Self::TypedConst { value, .. } => {
                Ok(EvalValue::Owned(value.clone()))
            }
            Self::ProductField {
                input: source,
                field,
            } => match source.evaluate_internal(input)? {
                EvalValue::Borrowed(Value::Product(fields)) => fields
                    .get(field)
                    .map(EvalValue::Borrowed)
                    .ok_or(QueryError::IndexOutOfBounds),
                EvalValue::Owned(Value::Product(mut fields)) => fields
                    .remove(field)
                    .map(EvalValue::Owned)
                    .ok_or(QueryError::IndexOutOfBounds),
                EvalValue::Borrowed(_) | EvalValue::Owned(_) => Err(QueryError::TypeMismatch),
            },
            Self::SeqLength(source) => {
                let source = source.evaluate_internal(input)?;
                let Value::Seq(values) = source.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let len = i64::try_from(values.len()).map_err(|_| QueryError::LengthOverflow)?;
                Ok(EvalValue::Owned(Value::I64(len)))
            }
            Self::SeqSumI64(source) => {
                let source = source.evaluate_internal(input)?;
                let Value::Seq(values) = source.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let mut sum = 0_i64;
                for value in values {
                    let Value::I64(value) = value else {
                        return Err(QueryError::TypeMismatch);
                    };
                    sum = sum
                        .checked_add(*value)
                        .ok_or(QueryError::ArithmeticOverflow)?;
                }
                Ok(EvalValue::Owned(Value::I64(sum)))
            }
            Self::AddI64(left, right) => {
                let left = left.evaluate_internal(input)?;
                let right = right.evaluate_internal(input)?;
                let Value::I64(left) = left.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let Value::I64(right) = right.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                left.checked_add(*right)
                    .map(|value| EvalValue::Owned(Value::I64(value)))
                    .ok_or(QueryError::ArithmeticOverflow)
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                let condition = condition.evaluate_internal(input)?;
                let Value::Bool(condition) = condition.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                if *condition {
                    when_true.evaluate_internal(input)
                } else {
                    when_false.evaluate_internal(input)
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactQuery {
    root: Expr,
}

impl ExactQuery {
    #[must_use]
    pub const fn new(root: Expr) -> Self {
        Self { root }
    }

    #[must_use]
    pub const fn root(&self) -> &Expr {
        &self.root
    }

    pub fn evaluate(&self, input: &Value) -> Result<Value, QueryError> {
        self.root.evaluate(input)
    }

    pub fn typecheck(
        &self,
        input: &kernel_schema::TypeExpr,
    ) -> Result<kernel_schema::TypeExpr, QueryTypeError> {
        self.root.typecheck(input)
    }
}

impl Expr {
    pub fn typecheck(
        &self,
        input_type: &kernel_schema::TypeExpr,
    ) -> Result<kernel_schema::TypeExpr, QueryTypeError> {
        use kernel_schema::{ScalarType, TypeExpr};
        match self {
            Self::Input => Ok(input_type.clone()),
            Self::Const(value) => {
                infer_value_type(value).ok_or(QueryTypeError::AmbiguousConstantType)
            }
            Self::TypedConst { value, ty } => {
                if value_shape_matches_type(value, ty) {
                    Ok(ty.clone())
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::ProductField { input, field } => {
                let TypeExpr::Product(fields) = input.typecheck(input_type)? else {
                    return Err(QueryTypeError::TypeMismatch);
                };
                fields
                    .get(field)
                    .cloned()
                    .ok_or(QueryTypeError::UnknownField(*field))
            }
            Self::SeqLength(source) => {
                if matches!(source.typecheck(input_type)?, TypeExpr::Seq(_)) {
                    Ok(TypeExpr::Scalar(ScalarType::I64))
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::SeqSumI64(source) => {
                let TypeExpr::Seq(element) = source.typecheck(input_type)? else {
                    return Err(QueryTypeError::TypeMismatch);
                };
                if *element == TypeExpr::Scalar(ScalarType::I64) {
                    Ok(TypeExpr::Scalar(ScalarType::I64))
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::AddI64(left, right) => {
                let expected = TypeExpr::Scalar(ScalarType::I64);
                if left.typecheck(input_type)? == expected
                    && right.typecheck(input_type)? == expected
                {
                    Ok(expected)
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                if condition.typecheck(input_type)? != TypeExpr::Scalar(ScalarType::Bool) {
                    return Err(QueryTypeError::TypeMismatch);
                }
                let left = when_true.typecheck(input_type)?;
                let right = when_false.typecheck(input_type)?;
                if left == right {
                    Ok(left)
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
        }
    }
}

fn infer_value_type(value: &Value) -> Option<kernel_schema::TypeExpr> {
    use kernel_schema::{ScalarType, TypeExpr};
    match value {
        Value::Unit => Some(TypeExpr::Scalar(ScalarType::Unit)),
        Value::Bool(_) => Some(TypeExpr::Scalar(ScalarType::Bool)),
        Value::I64(_) => Some(TypeExpr::Scalar(ScalarType::I64)),
        Value::F64Bits(_) => Some(TypeExpr::Scalar(ScalarType::F64)),
        Value::Text(_) => Some(TypeExpr::Scalar(ScalarType::Text)),
        Value::LiveEntityRef { entity_type, .. } => {
            Some(TypeExpr::Scalar(ScalarType::LiveEntityRef(*entity_type)))
        }
        Value::HistoricalEntityId { entity_type, .. } => Some(TypeExpr::Scalar(
            ScalarType::HistoricalEntityId(*entity_type),
        )),
        Value::Product(fields) => fields
            .iter()
            .map(|(field, value)| infer_value_type(value).map(|ty| (*field, ty)))
            .collect::<Option<std::collections::BTreeMap<_, _>>>()
            .map(TypeExpr::Product),
        Value::Seq(values) if !values.is_empty() => {
            let first = infer_value_type(&values[0])?;
            if values
                .iter()
                .all(|value| infer_value_type(value).as_ref() == Some(&first))
            {
                Some(TypeExpr::Seq(Box::new(first)))
            } else {
                None
            }
        }
        Value::Option(_)
        | Value::Variant { .. }
        | Value::Seq(_)
        | Value::Set { .. }
        | Value::Bag { .. }
        | Value::Map { .. } => None,
    }
}

pub type QueryResult = Result<Value, QueryError>;

#[must_use]
pub fn derivative_by_recompute(
    query: &ExactQuery,
    old: &Value,
    change: &Change<Value>,
) -> Change<QueryResult> {
    let old_output = query.evaluate(old);
    let new_input = change.apply(old);
    let new_output = query.evaluate(&new_input);

    if old_output == new_output {
        Change::NoChange
    } else {
        Change::Replace(new_output)
    }
}

pub fn derivative_seq_splice(
    query: &ExactQuery,
    old: &Value,
    splice: &SeqSplice<Value>,
) -> Result<Option<Change<QueryResult>>, SeqChangeError> {
    let Expr::SeqLength(source) = &query.root else {
        return Ok(None);
    };
    if !matches!(source.as_ref(), Expr::Input) {
        return Ok(None);
    }
    let Value::Seq(values) = old else {
        return Ok(None);
    };

    let next = splice.apply(values)?;
    let old_len = i64::try_from(values.len()).map_err(|_| SeqChangeError::DeleteOutOfBounds);
    let next_len = i64::try_from(next.len()).map_err(|_| SeqChangeError::DeleteOutOfBounds);
    let (Ok(old_len), Ok(next_len)) = (old_len, next_len) else {
        return Ok(None);
    };
    if old_len == next_len {
        Ok(Some(Change::NoChange))
    } else {
        Ok(Some(Change::Replace(Ok(Value::I64(next_len)))))
    }
}

#[must_use]
pub fn impact_by_recompute(query: &ExactQuery, old: &Value, change: &Change<Value>) -> Impact {
    let old_output = query.evaluate(old);
    let new_input = change.apply(old);
    let new_output = query.evaluate(&new_input);
    if old_output == new_output {
        Impact::Unaffected
    } else {
        Impact::Changed
    }
}

pub type RelQueryResult = Result<RelationValue, RelQueryError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDelta {
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
    pub result_type: RelType,
}

/// One relation Rewrite intent whose derived effect is pinned to an exact
/// Γ-validated `RelationDelta`. The delta remains explicit because it is the
/// compact structural effect consumed by DTC/maintained plans, while the
/// prepared Rewrite preserves family/law identity and explicit user inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelationRewrite<I = Value> {
    pub delta: RelationDelta,
    pub rewrite: PreparedRewrite<RelationValue, I>,
}

/// Cross-crate row-identity evidence for a relation delta.
///
/// This value is deliberately **not** a semantic certificate or publication
/// capability: callers may construct it, and every maintained leaf validates
/// the supplied handles against its current storage-handle snapshot before it
/// can produce a detached candidate. Authoritative publication remains owned
/// by `kernel-plan::RuntimeRevisionCell`.
///
/// The row payload is carried for operator semantics, while handles let `Scan`
/// update its local snapshot without repeating semantic membership search over
/// the whole base relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageResolvedRelationDelta {
    relation: kernel_types::SemanticId,
    delta: RelationDelta,
    removed_handles: Vec<kernel_types::StableRowHandle>,
    inserted_handles: Vec<kernel_types::StableRowHandle>,
}

impl StorageResolvedRelationDelta {
    #[must_use]
    pub fn from_parts(
        relation: kernel_types::SemanticId,
        delta: RelationDelta,
        removed_handles: Vec<kernel_types::StableRowHandle>,
        inserted_handles: Vec<kernel_types::StableRowHandle>,
    ) -> Self {
        Self {
            relation,
            delta,
            removed_handles,
            inserted_handles,
        }
    }

    #[must_use]
    pub fn relation(&self) -> kernel_types::SemanticId {
        self.relation
    }

    #[must_use]
    pub fn delta(&self) -> &RelationDelta {
        &self.delta
    }
}

impl RelationDelta {
    #[must_use]
    pub fn as_delta_view(&self) -> RelationDeltaView<'_> {
        RelationDeltaView {
            removed: &self.removed,
            inserted: &self.inserted,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }

    pub fn apply_to_value(
        &self,
        old: RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        apply_relation_delta_to_value(old, self, context, registry)
    }

    pub fn between_values(
        old: &RelationValue,
        next: &RelationValue,
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        relation_delta_between_values(old, next, result_type, context, registry)
    }

    /// Compiles this Γ-validated relation delta into one intent-bearing
    /// prepared Rewrite. The exact endpoint is derived through the same pinned
    /// semantic relation-delta application used by maintained queries; callers
    /// cannot substitute a different endpoint while retaining the delta intent.
    pub fn prepare_rewrite<I>(
        &self,
        old: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        spec: &RewriteSpec,
        explicit_inputs: Vec<I>,
    ) -> Result<PreparedRewrite<RelationValue, I>, RelQueryError> {
        let endpoint = self.apply_to_value(old.clone(), context, registry)?;
        Ok(spec.prepare(
            explicit_inputs,
            RewriteEffect::Fine(FineChange::new(FineChangeKind::Relation, endpoint)),
        ))
    }

    pub fn prepare_relation_rewrite<I>(
        &self,
        old: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        spec: &RewriteSpec,
        explicit_inputs: Vec<I>,
    ) -> Result<PreparedRelationRewrite<I>, RelQueryError> {
        Ok(PreparedRelationRewrite {
            delta: self.clone(),
            rewrite: self.prepare_rewrite(old, context, registry, spec, explicit_inputs)?,
        })
    }
}

type CanonicalRowKey = Vec<kernel_semantics::CanonicalEqKey>;
type SupportLookup = BTreeMap<CanonicalRowKey, usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetSupportPatchEntry {
    key: CanonicalRowKey,
    representative: Row,
    after: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetSupportPatch {
    entries: Vec<SetSupportPatchEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedSetSupportState {
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    supports: Vec<(Row, i64)>,
    support_lookup: SupportLookup,
}

impl MaterializedSetSupportState {
    pub fn build(
        rows: &[Row],
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if !matches!(
            result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) {
            return Err(RelQueryError::TypeMismatch);
        }
        Self::validate_rows(rows, &result_type, context, registry)?;
        let (supports, support_lookup) = canonical_row_supports(
            rows,
            relation_column_equivalences(&result_type),
            context,
            registry,
        )?;
        Ok(Self {
            result_type,
            semantic_context: context.clone(),
            supports,
            support_lookup,
        })
    }

    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
    }

    pub fn support_count(
        &self,
        row: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<i64, RelQueryError> {
        self.check_context(context)?;
        Self::validate_rows(
            std::slice::from_ref(row),
            &self.result_type,
            context,
            registry,
        )?;
        let key = canonical_row_key(
            row,
            relation_column_equivalences(&self.result_type),
            context,
            registry,
        )?;
        Ok(self
            .support_lookup
            .get(&key)
            .map_or(0, |index| self.supports[*index].1))
    }

    fn output_value(&self) -> RelationValue {
        let rows = self
            .supports
            .iter()
            .filter(|(_, count)| *count > 0)
            .map(|(row, _)| row.clone())
            .collect();
        relation_value_from_rows(rows, &self.result_type)
    }

    pub fn apply_rows_delta(
        &mut self,
        inserted_rows: Vec<Row>,
        removed_rows: Vec<Row>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let delta = RelationDelta {
            inserted: inserted_rows,
            removed: removed_rows,
            result_type: self.result_type.clone(),
        };
        let planned = self.plan_delta_view(&delta.as_delta_view(), context, registry)?;
        let effect = materialize_delta_view(&planned.effect, self.result_type.clone())?;
        self.commit_support_patch(planned.patch);
        Ok(effect)
    }

    fn plan_delta_view<D: DeltaView<Row>>(
        &self,
        delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<SetSupportPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        self.check_context(context)?;
        let column_equivalences = relation_column_equivalences(&self.result_type);
        let mut changes = Vec::<SupportDeltaPlan>::new();
        let mut change_lookup = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
        let mut visit_error = None;
        delta.visit(|weight, row| {
            if weight == 0 || visit_error.is_some() {
                return;
            }
            if let Err(error) = Self::validate_rows(
                std::slice::from_ref(row),
                &self.result_type,
                context,
                registry,
            ) {
                visit_error = Some(error);
                return;
            }
            let key = match canonical_row_key(row, column_equivalences, context, registry) {
                Ok(key) => key,
                Err(error) => {
                    visit_error = Some(error);
                    return;
                }
            };
            let change_index = if let Some(index) = change_lookup.get(&key).copied() {
                index
            } else {
                let index = changes.len();
                change_lookup.insert(key.clone(), index);
                changes.push(SupportDeltaPlan {
                    key,
                    representative: row.clone(),
                    removals: 0,
                    insertions: 0,
                });
                index
            };
            if weight < 0 {
                let removals = weight
                    .checked_neg()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta);
                match removals.and_then(|amount| {
                    changes[change_index]
                        .removals
                        .checked_add(amount)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)
                }) {
                    Ok(total) => changes[change_index].removals = total,
                    Err(error) => visit_error = Some(error),
                }
            } else {
                match changes[change_index].insertions.checked_add(weight) {
                    Some(total) => changes[change_index].insertions = total,
                    None => visit_error = Some(RelQueryError::InconsistentIncrementalDelta),
                }
            }
        });
        if let Some(error) = visit_error {
            return Err(error);
        }

        let mut patch_entries = Vec::with_capacity(changes.len());
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        for change in changes {
            let before = self
                .support_lookup
                .get(&change.key)
                .map_or(0, |index| self.supports[*index].1);
            if change.removals > before {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let after = before
                .checked_sub(change.removals)
                .and_then(|value| value.checked_add(change.insertions))
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if before == 0 && after > 0 {
                effect.push_weighted(1, change.representative.clone());
            } else if before > 0 && after == 0 {
                effect.push_weighted(-1, change.representative.clone());
            }
            patch_entries.push(SetSupportPatchEntry {
                key: change.key,
                representative: change.representative,
                after,
            });
        }
        Ok(PlannedDeltaEffect {
            patch: SetSupportPatch {
                entries: patch_entries,
            },
            effect,
        })
    }

    fn commit_support_patch(&mut self, patch: SetSupportPatch) {
        for entry in patch.entries {
            if let Some(index) = self.support_lookup.get(&entry.key).copied() {
                self.supports[index].1 = entry.after;
            } else if entry.after > 0 {
                let index = self.supports.len();
                self.supports
                    .push((entry.representative.clone(), entry.after));
                self.support_lookup.insert(entry.key, index);
            }
        }
    }

    fn validate_rows(
        rows: &[Row],
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let equivalences = relation_column_equivalences(result_type);
        if equivalences.len() != result_type.columns.len() {
            return Err(RelQueryError::TypeMismatch);
        }
        for (column_type, equivalence) in result_type.columns.iter().zip(equivalences) {
            validate_query_equivalence(*equivalence, column_type, context, registry)?;
        }
        Self::validate_row_shapes(rows, result_type)
    }

    fn validate_row_shapes(rows: &[Row], result_type: &RelType) -> Result<(), RelQueryError> {
        for row in rows {
            if row.len() != result_type.columns.len()
                || !row
                    .iter()
                    .zip(&result_type.columns)
                    .all(|(value, ty)| value_shape_matches_type(value, ty))
            {
                return Err(RelQueryError::TypeMismatch);
            }
        }
        Ok(())
    }

    fn check_context(&self, context: &kernel_schema::SemanticContext) -> Result<(), RelQueryError> {
        if context == &self.semantic_context {
            Ok(())
        } else {
            Err(RelQueryError::SemanticRevisionMismatch)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaterializedRelDeltaOperator {
    ProjectSet { input: RelExpr, columns: Vec<usize> },
    Distinct { input: RelExpr },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedRelDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    operator: MaterializedRelDeltaOperator,
    supports: MaterializedSetSupportState,
}

impl MaterializedRelDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        match query {
            RelExpr::Project { input, columns } => {
                let input_type = input.typecheck(context, registry)?;
                if !matches!(
                    input_type.semantics,
                    kernel_schema::RelationSemantics::Set { .. }
                ) {
                    return Ok(None);
                }
                let result_type = query.typecheck(context, registry)?;
                let old_rows =
                    project_rows(input.evaluate(old, context, registry)?.into_rows(), columns)?;
                let supports =
                    MaterializedSetSupportState::build(&old_rows, result_type, context, registry)?;
                Ok(Some(Self {
                    query: query.clone(),
                    semantic_context: context.clone(),
                    operator: MaterializedRelDeltaOperator::ProjectSet {
                        input: input.as_ref().clone(),
                        columns: columns.clone(),
                    },
                    supports,
                }))
            }
            RelExpr::Distinct { input, .. } => {
                let result_type = query.typecheck(context, registry)?;
                let old_rows = input.evaluate(old, context, registry)?.into_rows();
                let supports =
                    MaterializedSetSupportState::build(&old_rows, result_type, context, registry)?;
                Ok(Some(Self {
                    query: query.clone(),
                    semantic_context: context.clone(),
                    operator: MaterializedRelDeltaOperator::Distinct {
                        input: input.as_ref().clone(),
                    },
                    supports,
                }))
            }
            _ => Ok(None),
        }
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn support_state(&self) -> &MaterializedSetSupportState {
        &self.supports
    }

    pub fn apply_model_change(
        &mut self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let (input, projected_columns) = match &self.operator {
            MaterializedRelDeltaOperator::ProjectSet { input, columns } => {
                (input, Some(columns.as_slice()))
            }
            MaterializedRelDeltaOperator::Distinct { input } => (input, None),
        };
        let input_delta = rel_delta_optimized_inner(input, old, change, context, registry)?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let (inserted, removed) = if let Some(columns) = projected_columns {
            (
                project_rows(input_delta.inserted, columns)?,
                project_rows(input_delta.removed, columns)?,
            )
        } else {
            (input_delta.inserted, input_delta.removed)
        };
        self.supports
            .apply_rows_delta(inserted, removed, context, registry)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct IndexedRowId(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedJoinSide {
    rows: BTreeMap<IndexedRowId, Row>,
    index:
        kernel_semantic_index::SemanticBucketIndex<kernel_semantics::CanonicalEqKey, IndexedRowId>,
    next_id: u64,
}

#[derive(Debug)]
struct IndexedRelationMutationPlan {
    remove_ids: Vec<IndexedRowId>,
    inserted: Vec<(IndexedRowId, kernel_semantics::CanonicalEqKey, Row)>,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticIndexedJoinStorage {
    encoder: kernel_semantics::ResolvedPrimitiveEquivalence,
    left: IndexedJoinSide,
    right: IndexedJoinSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralIndexedJoinSide {
    rows: BTreeMap<IndexedRowId, Row>,
    keys: BTreeMap<IndexedRowId, kernel_semantics::CanonicalEqKey>,
    buckets: BTreeMap<kernel_semantics::CanonicalEqKey, Vec<IndexedRowId>>,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralIndexedJoinStorage {
    equivalence: kernel_types::SemanticId,
    left: StructuralIndexedJoinSide,
    right: StructuralIndexedJoinSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedJoinStorage {
    I64 {
        left: BTreeMap<i64, Vec<Row>>,
        right: BTreeMap<i64, Vec<Row>>,
    },
    SemanticIndexed(Box<SemanticIndexedJoinStorage>),
    StructuralIndexed(Box<StructuralIndexedJoinStorage>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationMutationPlan {
    remove_indices: Vec<usize>,
    inserted: Vec<Row>,
    canonical_keys: Option<CanonicalRelationMutationKeys>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalRelationMutationKeys {
    removed: Vec<CanonicalRowKey>,
    inserted: Vec<CanonicalRowKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalRowPositionIndex {
    by_key: BTreeMap<CanonicalRowKey, Vec<usize>>,
    by_position: Vec<CanonicalRowKey>,
}

/// R&D V3 scan-level commit patch.  Both semantic deltas and
/// storage-resolved deltas feed the same immutable maintained-plan planner;
/// only the leaf publication mechanism differs.
#[derive(Debug)]
enum MaintainedScanCommitPatch {
    Semantic(RelationMutationPlan),
    StorageResolved(StorageResolvedScanPatch),
}

#[derive(Debug)]
struct StorageResolvedScanPatch {
    removed_handles: Vec<kernel_types::StableRowHandle>,
    inserted: Vec<(kernel_types::StableRowHandle, Row)>,
}

#[derive(Debug)]
struct PlannedI64JoinSide {
    buckets: BTreeMap<i64, Vec<Row>>,
}

#[derive(Debug)]
enum JoinDeltaPatch {
    I64 {
        left: PlannedI64JoinSide,
        right: PlannedI64JoinSide,
    },
    SemanticIndexed {
        left: IndexedRelationMutationPlan,
        right: IndexedRelationMutationPlan,
    },
    StructuralIndexed {
        left: IndexedRelationMutationPlan,
        right: IndexedRelationMutationPlan,
    },
}

#[derive(Clone, Copy)]
struct JoinI64MaintenanceSpec<'a> {
    left_column: usize,
    right_column: usize,
    left_type: &'a RelType,
    right_type: &'a RelType,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Clone, Copy)]
struct GenericJoinMaintenanceSpec<'a> {
    left_column: usize,
    right_column: usize,
    left_type: &'a RelType,
    right_type: &'a RelType,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

struct NormalizedDeltaRows {
    removed: Vec<(usize, Row)>,
    inserted: Vec<(usize, Row)>,
}

fn normalized_delta_rows<D: DeltaView<Row>>(
    delta: &D,
) -> Result<NormalizedDeltaRows, RelQueryError> {
    let mut removed = Vec::new();
    let mut inserted = Vec::new();
    let mut error = None;
    delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let magnitude = if weight < 0 {
            let Some(value) = weight.checked_neg() else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
            value
        } else {
            weight
        };
        let Ok(magnitude) = usize::try_from(magnitude) else {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        };
        if weight < 0 {
            removed.push((magnitude, row.clone()));
        } else {
            inserted.push((magnitude, row.clone()));
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(NormalizedDeltaRows { removed, inserted })
}

impl IndexedJoinSide {
    fn build(
        rows: &[Row],
        column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveEquivalence,
        context: &kernel_schema::SemanticContext,
    ) -> Result<Self, RelQueryError> {
        let dependency = kernel_semantic_index::SemanticModuleBinding {
            semantic_id: encoder.equivalence(),
            module_digest: encoder.module_digest(),
        };
        let mut index = kernel_semantic_index::SemanticBucketIndex::new(
            kernel_semantic_index::SemanticIndexBinding::new(context, vec![dependency]),
        );
        let mut stored = BTreeMap::new();
        let mut next_id = 0_u64;
        for row in rows {
            let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            let key = encoder.canonical_key(value)?;
            let id = IndexedRowId(next_id);
            next_id = next_id
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
            let previous_key = index.insert(id, key);
            let previous_row = stored.insert(id, row.clone());
            debug_assert!(previous_key.is_none());
            debug_assert!(previous_row.is_none());
        }
        Ok(Self {
            rows: stored,
            index,
            next_id,
        })
    }

    fn plan_mutation_view<D: DeltaView<Row>>(
        &self,
        delta: &D,
        join_column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveEquivalence,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<IndexedRelationMutationPlan, RelQueryError> {
        let normalized = normalized_delta_rows(delta)?;
        let equivalences = relation_column_equivalences(ty);
        let mut used = BTreeSet::new();
        let mut remove_ids = Vec::new();
        for (magnitude, removed) in &normalized.removed {
            for _ in 0..*magnitude {
                let key = encoder.canonical_key(
                    removed
                        .get(join_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                )?;
                let bucket = self
                    .index
                    .bucket(&key)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let mut found = None;
                for id in bucket {
                    if used.contains(id) {
                        continue;
                    }
                    let candidate = self
                        .rows
                        .get(id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    if rows_semantically_equal(candidate, removed, equivalences, context, registry)?
                    {
                        found = Some(*id);
                        break;
                    }
                }
                let Some(id) = found else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                used.insert(id);
                remove_ids.push(id);
            }
        }

        let is_set = matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. });
        let mut inserted = Vec::new();
        let mut next_id = self.next_id;
        for (magnitude, row) in &normalized.inserted {
            for _ in 0..*magnitude {
                let key = encoder.canonical_key(
                    row.get(join_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                )?;
                if is_set {
                    if let Some(bucket) = self.index.bucket(&key) {
                        for id in bucket {
                            if used.contains(id) {
                                continue;
                            }
                            let candidate = self
                                .rows
                                .get(id)
                                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                            if rows_semantically_equal(
                                candidate,
                                row,
                                equivalences,
                                context,
                                registry,
                            )? {
                                return Err(RelQueryError::InconsistentIncrementalDelta);
                            }
                        }
                    }
                    for (_, previous_key, previous_row) in &inserted {
                        if previous_key == &key
                            && rows_semantically_equal(
                                previous_row,
                                row,
                                equivalences,
                                context,
                                registry,
                            )?
                        {
                            return Err(RelQueryError::InconsistentIncrementalDelta);
                        }
                    }
                }
                let id = IndexedRowId(next_id);
                next_id = next_id
                    .checked_add(1)
                    .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                inserted.push((id, key, row.clone()));
            }
        }
        Ok(IndexedRelationMutationPlan {
            remove_ids,
            inserted,
            next_id,
        })
    }

    fn commit_plan(&mut self, plan: IndexedRelationMutationPlan) {
        for id in plan.remove_ids {
            let removed_key = self.index.remove(&id);
            let removed_row = self.rows.remove(&id);
            debug_assert!(removed_key.is_some());
            debug_assert!(removed_row.is_some());
        }
        for (id, key, row) in plan.inserted {
            let previous_key = self.index.insert(id, key);
            let previous_row = self.rows.insert(id, row);
            debug_assert!(previous_key.is_none());
            debug_assert!(previous_row.is_none());
        }
        self.next_id = plan.next_id;
    }
}

impl StructuralIndexedJoinSide {
    fn build(
        rows: &[Row],
        column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let mut stored = BTreeMap::new();
        let mut keys = BTreeMap::new();
        let mut buckets = BTreeMap::<kernel_semantics::CanonicalEqKey, Vec<IndexedRowId>>::new();
        let mut next_id = 0_u64;
        for row in rows {
            let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            let key = registry.canonical_equivalence_key(context, equivalence, value)?;
            let id = IndexedRowId(next_id);
            next_id = next_id
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
            buckets.entry(key.clone()).or_default().push(id);
            keys.insert(id, key);
            stored.insert(id, row.clone());
        }
        Ok(Self {
            rows: stored,
            keys,
            buckets,
            next_id,
        })
    }

    fn canonical_key(
        row: &Row,
        column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_semantics::CanonicalEqKey, RelQueryError> {
        registry
            .canonical_equivalence_key(
                context,
                equivalence,
                row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?,
            )
            .map_err(Into::into)
    }

    fn bucket(&self, key: &kernel_semantics::CanonicalEqKey) -> Option<&[IndexedRowId]> {
        self.buckets.get(key).map(Vec::as_slice)
    }

    fn plan_mutation_view<D: DeltaView<Row>>(
        &self,
        delta: &D,
        join_column: usize,
        equivalence: kernel_types::SemanticId,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<IndexedRelationMutationPlan, RelQueryError> {
        let normalized = normalized_delta_rows(delta)?;
        let equivalences = relation_column_equivalences(ty);
        let mut used = BTreeSet::new();
        let mut remove_ids = Vec::new();
        for (magnitude, removed) in &normalized.removed {
            for _ in 0..*magnitude {
                let key =
                    Self::canonical_key(removed, join_column, equivalence, context, registry)?;
                let bucket = self
                    .bucket(&key)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let mut found = None;
                for id in bucket {
                    if used.contains(id) {
                        continue;
                    }
                    let candidate = self
                        .rows
                        .get(id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    if rows_semantically_equal(candidate, removed, equivalences, context, registry)?
                    {
                        found = Some(*id);
                        break;
                    }
                }
                let Some(id) = found else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                used.insert(id);
                remove_ids.push(id);
            }
        }

        let is_set = matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. });
        let mut inserted = Vec::new();
        let mut next_id = self.next_id;
        for (magnitude, row) in &normalized.inserted {
            for _ in 0..*magnitude {
                let key = Self::canonical_key(row, join_column, equivalence, context, registry)?;
                if is_set {
                    if let Some(bucket) = self.bucket(&key) {
                        for id in bucket {
                            if used.contains(id) {
                                continue;
                            }
                            let candidate = self
                                .rows
                                .get(id)
                                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                            if rows_semantically_equal(
                                candidate,
                                row,
                                equivalences,
                                context,
                                registry,
                            )? {
                                return Err(RelQueryError::InconsistentIncrementalDelta);
                            }
                        }
                    }
                    for (_, previous_key, previous_row) in &inserted {
                        if previous_key == &key
                            && rows_semantically_equal(
                                previous_row,
                                row,
                                equivalences,
                                context,
                                registry,
                            )?
                        {
                            return Err(RelQueryError::InconsistentIncrementalDelta);
                        }
                    }
                }
                let id = IndexedRowId(next_id);
                next_id = next_id
                    .checked_add(1)
                    .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                inserted.push((id, key, row.clone()));
            }
        }
        Ok(IndexedRelationMutationPlan {
            remove_ids,
            inserted,
            next_id,
        })
    }

    fn commit_plan(&mut self, plan: IndexedRelationMutationPlan) {
        for id in plan.remove_ids {
            let Some(key) = self.keys.remove(&id) else {
                debug_assert!(false, "planned structural index removal must have a key");
                continue;
            };
            if let Some(bucket) = self.buckets.get_mut(&key) {
                if let Some(position) = bucket.iter().position(|candidate| *candidate == id) {
                    bucket.remove(position);
                }
                if bucket.is_empty() {
                    self.buckets.remove(&key);
                }
            }
            self.rows.remove(&id);
        }
        for (id, key, row) in plan.inserted {
            self.buckets.entry(key.clone()).or_default().push(id);
            self.keys.insert(id, key);
            self.rows.insert(id, row);
        }
        self.next_id = plan.next_id;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedJoinDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    left_type: RelType,
    right_type: RelType,
    result_type: RelType,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    storage: MaintainedJoinStorage,
}

impl MaterializedJoinDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::JoinEq { left, right, .. } = query else {
            return Ok(None);
        };
        let left_value = left.evaluate(old, context, registry)?;
        let right_value = right.evaluate(old, context, registry)?;
        Self::build_from_input_values(query, &left_value, &right_value, context, registry)
    }

    fn build_from_input_values(
        query: &RelExpr,
        left_value: &RelationValue,
        right_value: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } = query
        else {
            return Ok(None);
        };
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        let result_type = query.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            left_value.rows(),
            &left_type,
            context,
            registry,
        )?;
        MaterializedSetSupportState::validate_rows(
            right_value.rows(),
            &right_type,
            context,
            registry,
        )?;
        let primitive_equivalence =
            registry.resolve_primitive_equivalence(context, *equivalence)?;
        let exact_i64 = matches!(
            left_type.columns.get(*left_column),
            Some(kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::I64
            ))
        ) && matches!(
            right_type.columns.get(*right_column),
            Some(kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::I64
            ))
        ) && matches!(
            primitive_equivalence.map(|resolved| resolved.bind_right(&Value::I64(0))),
            Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
        );
        let storage = if exact_i64 {
            MaintainedJoinStorage::I64 {
                left: Self::build_i64_buckets(left_value.rows(), *left_column)?,
                right: Self::build_i64_buckets(right_value.rows(), *right_column)?,
            }
        } else if let Some(encoder) = primitive_equivalence {
            MaintainedJoinStorage::SemanticIndexed(Box::new(SemanticIndexedJoinStorage {
                encoder,
                left: IndexedJoinSide::build(left_value.rows(), *left_column, encoder, context)?,
                right: IndexedJoinSide::build(right_value.rows(), *right_column, encoder, context)?,
            }))
        } else {
            MaintainedJoinStorage::StructuralIndexed(Box::new(StructuralIndexedJoinStorage {
                equivalence: *equivalence,
                left: StructuralIndexedJoinSide::build(
                    left_value.rows(),
                    *left_column,
                    *equivalence,
                    context,
                    registry,
                )?,
                right: StructuralIndexedJoinSide::build(
                    right_value.rows(),
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?,
            }))
        };
        Ok(Some(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            left_type,
            right_type,
            result_type,
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
            storage,
        }))
    }

    fn output_value(
        &self,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        match &self.storage {
            MaintainedJoinStorage::I64 { left, right } => {
                let mut rows = Vec::new();
                for (key, left_rows) in left {
                    let Some(right_rows) = right.get(key) else {
                        continue;
                    };
                    for left_row in left_rows {
                        for right_row in right_rows {
                            rows.push(Self::join_pair(left_row, right_row));
                        }
                    }
                }
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
            MaintainedJoinStorage::SemanticIndexed(storage) => {
                let mut rows = Vec::new();
                for (left_id, left_row) in &storage.left.rows {
                    let key = storage
                        .left
                        .index
                        .key_for(left_id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    let Some(right_ids) = storage.right.index.bucket(key) else {
                        continue;
                    };
                    for right_id in right_ids {
                        let right_row = storage
                            .right
                            .rows
                            .get(right_id)
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                        rows.push(Self::join_pair(left_row, right_row));
                    }
                }
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
            MaintainedJoinStorage::StructuralIndexed(storage) => {
                let mut rows = Vec::new();
                for (left_id, left_row) in &storage.left.rows {
                    let key = storage
                        .left
                        .keys
                        .get(left_id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    let Some(right_ids) = storage.right.bucket(key) else {
                        continue;
                    };
                    for right_id in right_ids {
                        let right_row = storage
                            .right
                            .rows
                            .get(right_id)
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                        rows.push(Self::join_pair(left_row, right_row));
                    }
                }
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
        }
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    pub fn apply_input_deltas(
        &mut self,
        left_delta: &RelationDelta,
        right_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if left_delta.result_type != self.left_type || right_delta.result_type != self.right_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned = self.plan_delta_views(
            &left_delta.as_delta_view(),
            &right_delta.as_delta_view(),
            context,
            registry,
        )?;
        let effect = materialize_delta_view(&planned.effect, self.result_type.clone())?;
        self.commit_join_patch(planned.patch);
        Ok(effect)
    }

    fn plan_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        Self::validate_delta_view_rows(left_delta, &self.left_type, context, registry)?;
        Self::validate_delta_view_rows(right_delta, &self.right_type, context, registry)?;

        match &self.storage {
            MaintainedJoinStorage::I64 { left, right } => {
                self.plan_i64_delta_views(left, right, left_delta, right_delta, context, registry)
            }
            MaintainedJoinStorage::SemanticIndexed(storage) => {
                self.plan_semantic_delta_views(storage, left_delta, right_delta, context, registry)
            }
            MaintainedJoinStorage::StructuralIndexed(storage) => self.plan_structural_delta_views(
                storage,
                left_delta,
                right_delta,
                context,
                registry,
            ),
        }
    }

    fn plan_i64_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        left: &BTreeMap<i64, Vec<Row>>,
        right: &BTreeMap<i64, Vec<Row>>,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let spec = JoinI64MaintenanceSpec {
            left_column: self.left_column,
            right_column: self.right_column,
            left_type: &self.left_type,
            right_type: &self.right_type,
            context,
            registry,
        };
        let planned_left = Self::plan_i64_side_view(
            left,
            left_delta,
            spec.left_column,
            spec.left_type,
            spec.context,
            spec.registry,
        )?;
        let planned_right = Self::plan_i64_side_view(
            right,
            right_delta,
            spec.right_column,
            spec.right_type,
            spec.context,
            spec.registry,
        )?;
        let effect = Self::plan_i64_join_effect(
            left,
            right,
            &planned_left.buckets,
            left_delta,
            right_delta,
            spec,
        )?;
        Ok(PlannedDeltaEffect {
            patch: JoinDeltaPatch::I64 {
                left: planned_left,
                right: planned_right,
            },
            effect,
        })
    }

    fn plan_semantic_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        storage: &SemanticIndexedJoinStorage,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let spec = GenericJoinMaintenanceSpec {
            left_column: self.left_column,
            right_column: self.right_column,
            left_type: &self.left_type,
            right_type: &self.right_type,
            context,
            registry,
        };
        let left_plan = storage.left.plan_mutation_view(
            left_delta,
            spec.left_column,
            storage.encoder,
            spec.left_type,
            spec.context,
            spec.registry,
        )?;
        let right_plan = storage.right.plan_mutation_view(
            right_delta,
            spec.right_column,
            storage.encoder,
            spec.right_type,
            spec.context,
            spec.registry,
        )?;
        let effect = Self::plan_semantic_join_effect(
            storage.encoder,
            &storage.left,
            &storage.right,
            &left_plan,
            left_delta,
            right_delta,
            spec,
        )?;
        Ok(PlannedDeltaEffect {
            patch: JoinDeltaPatch::SemanticIndexed {
                left: left_plan,
                right: right_plan,
            },
            effect,
        })
    }

    fn plan_structural_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        storage: &StructuralIndexedJoinStorage,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let spec = GenericJoinMaintenanceSpec {
            left_column: self.left_column,
            right_column: self.right_column,
            left_type: &self.left_type,
            right_type: &self.right_type,
            context,
            registry,
        };
        let left_plan = storage.left.plan_mutation_view(
            left_delta,
            spec.left_column,
            storage.equivalence,
            spec.left_type,
            spec.context,
            spec.registry,
        )?;
        let right_plan = storage.right.plan_mutation_view(
            right_delta,
            spec.right_column,
            storage.equivalence,
            spec.right_type,
            spec.context,
            spec.registry,
        )?;
        let effect = Self::plan_structural_join_effect(
            storage.equivalence,
            &storage.left,
            &storage.right,
            &left_plan,
            left_delta,
            right_delta,
            spec,
        )?;
        Ok(PlannedDeltaEffect {
            patch: JoinDeltaPatch::StructuralIndexed {
                left: left_plan,
                right: right_plan,
            },
            effect,
        })
    }

    fn validate_delta_view_rows<D: DeltaView<Row>>(
        delta: &D,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let mut error = None;
        delta.visit(|weight, row| {
            if weight == 0 || error.is_some() {
                return;
            }
            if weight == i64::MIN {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            }
            if let Err(next) = MaterializedSetSupportState::validate_rows(
                std::slice::from_ref(row),
                ty,
                context,
                registry,
            ) {
                error = Some(next);
            }
        });
        error.map_or(Ok(()), Err)
    }

    fn commit_join_patch(&mut self, patch: JoinDeltaPatch) {
        match (patch, &mut self.storage) {
            (
                JoinDeltaPatch::I64 { left, right },
                MaintainedJoinStorage::I64 {
                    left: left_state,
                    right: right_state,
                },
            ) => {
                Self::commit_i64_buckets(left_state, left.buckets);
                Self::commit_i64_buckets(right_state, right.buckets);
            }
            (
                JoinDeltaPatch::SemanticIndexed { left, right },
                MaintainedJoinStorage::SemanticIndexed(storage),
            ) => {
                storage.left.commit_plan(left);
                storage.right.commit_plan(right);
            }
            (
                JoinDeltaPatch::StructuralIndexed { left, right },
                MaintainedJoinStorage::StructuralIndexed(storage),
            ) => {
                storage.left.commit_plan(left);
                storage.right.commit_plan(right);
            }
            _ => unreachable!("Join patch/backend mismatch"),
        }
    }

    fn plan_semantic_join_effect<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        encoder: kernel_semantics::ResolvedPrimitiveEquivalence,
        left: &IndexedJoinSide,
        right: &IndexedJoinSide,
        left_plan: &IndexedRelationMutationPlan,
        left_delta: &LD,
        right_delta: &RD,
        spec: GenericJoinMaintenanceSpec<'_>,
    ) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut error = None;
        left_delta.visit(|weight, left_row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match left_row
                .get(spec.left_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)
                .and_then(|value| encoder.canonical_key(value).map_err(Into::into))
            {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            let Some(right_ids) = right.index.bucket(&key) else {
                return;
            };
            for right_id in right_ids {
                let Some(right_row) = right.rows.get(right_id) else {
                    error = Some(RelQueryError::InconsistentIncrementalDelta);
                    return;
                };
                effect.push_weighted(weight, Self::join_pair(left_row, right_row));
            }
        });
        if let Some(error) = error {
            return Err(error);
        }

        let removed_left = left_plan
            .remove_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        right_delta.visit(|weight, right_row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match right_row
                .get(spec.right_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)
                .and_then(|value| encoder.canonical_key(value).map_err(Into::into))
            {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            if let Some(left_ids) = left.index.bucket(&key) {
                for left_id in left_ids {
                    if removed_left.contains(left_id) {
                        continue;
                    }
                    let Some(left_row) = left.rows.get(left_id) else {
                        error = Some(RelQueryError::InconsistentIncrementalDelta);
                        return;
                    };
                    effect.push_weighted(weight, Self::join_pair(left_row, right_row));
                }
            }
            for (_, inserted_key, left_row) in &left_plan.inserted {
                if inserted_key == &key {
                    effect.push_weighted(weight, Self::join_pair(left_row, right_row));
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(effect)
    }

    fn plan_structural_join_effect<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        equivalence: kernel_types::SemanticId,
        left: &StructuralIndexedJoinSide,
        right: &StructuralIndexedJoinSide,
        left_plan: &IndexedRelationMutationPlan,
        left_delta: &LD,
        right_delta: &RD,
        spec: GenericJoinMaintenanceSpec<'_>,
    ) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut error = None;
        left_delta.visit(|weight, left_row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match StructuralIndexedJoinSide::canonical_key(
                left_row,
                spec.left_column,
                equivalence,
                spec.context,
                spec.registry,
            ) {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            let Some(right_ids) = right.bucket(&key) else {
                return;
            };
            for right_id in right_ids {
                let Some(right_row) = right.rows.get(right_id) else {
                    error = Some(RelQueryError::InconsistentIncrementalDelta);
                    return;
                };
                effect.push_weighted(weight, Self::join_pair(left_row, right_row));
            }
        });
        if let Some(error) = error {
            return Err(error);
        }

        let removed_left = left_plan
            .remove_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        right_delta.visit(|weight, right_row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match StructuralIndexedJoinSide::canonical_key(
                right_row,
                spec.right_column,
                equivalence,
                spec.context,
                spec.registry,
            ) {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            if let Some(left_ids) = left.bucket(&key) {
                for left_id in left_ids {
                    if removed_left.contains(left_id) {
                        continue;
                    }
                    let Some(left_row) = left.rows.get(left_id) else {
                        error = Some(RelQueryError::InconsistentIncrementalDelta);
                        return;
                    };
                    effect.push_weighted(weight, Self::join_pair(left_row, right_row));
                }
            }
            for (_, inserted_key, left_row) in &left_plan.inserted {
                if inserted_key == &key {
                    effect.push_weighted(weight, Self::join_pair(left_row, right_row));
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(effect)
    }

    fn plan_relation_mutation(
        value: &RelationValue,
        delta: &RelationDelta,
        ty: &RelType,
        canonical_lookup: Option<&CanonicalRowPositionIndex>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationMutationPlan, RelQueryError> {
        if let Some(lookup) = canonical_lookup {
            let equivalences = relation_column_equivalences(ty);
            let mut removed_per_key = BTreeMap::<CanonicalRowKey, usize>::new();
            let mut remove_indices = Vec::with_capacity(delta.removed.len());
            let mut removed_keys = Vec::with_capacity(delta.removed.len());
            for removed in &delta.removed {
                let Some(key) = try_canonical_row_key(removed, equivalences, context, registry)?
                else {
                    return Self::plan_relation_mutation_linear(
                        value, delta, ty, context, registry,
                    );
                };
                let used = removed_per_key.entry(key.clone()).or_default();
                let bucket = lookup
                    .by_key
                    .get(&key)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let index = bucket
                    .len()
                    .checked_sub(*used + 1)
                    .and_then(|position| bucket.get(position).copied())
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                *used += 1;
                remove_indices.push(index);
                removed_keys.push(key);
            }

            let mut inserted_keys = Vec::with_capacity(delta.inserted.len());
            for inserted in &delta.inserted {
                let Some(key) = try_canonical_row_key(inserted, equivalences, context, registry)?
                else {
                    return Self::plan_relation_mutation_linear(
                        value, delta, ty, context, registry,
                    );
                };
                if matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. }) {
                    let existing = lookup.by_key.get(&key).map_or(0, Vec::len);
                    let removing = removed_per_key.get(&key).copied().unwrap_or(0);
                    if existing > removing || inserted_keys.iter().any(|previous| previous == &key)
                    {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                }
                inserted_keys.push(key);
            }
            return Ok(RelationMutationPlan {
                remove_indices,
                inserted: delta.inserted.clone(),
                canonical_keys: Some(CanonicalRelationMutationKeys {
                    removed: removed_keys,
                    inserted: inserted_keys,
                }),
            });
        }
        Self::plan_relation_mutation_linear(value, delta, ty, context, registry)
    }

    fn plan_relation_mutation_linear(
        value: &RelationValue,
        delta: &RelationDelta,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationMutationPlan, RelQueryError> {
        let equivalences = relation_column_equivalences(ty);
        let mut used = vec![false; value.rows().len()];
        let mut remove_indices = Vec::with_capacity(delta.removed.len());
        for removed in &delta.removed {
            let mut found = None;
            for (index, candidate) in value.rows().iter().enumerate() {
                if !used[index]
                    && rows_semantically_equal(candidate, removed, equivalences, context, registry)?
                {
                    found = Some(index);
                    break;
                }
            }
            let Some(index) = found else {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            };
            used[index] = true;
            remove_indices.push(index);
        }
        if matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. }) {
            for (insert_index, inserted) in delta.inserted.iter().enumerate() {
                for (index, candidate) in value.rows().iter().enumerate() {
                    if !used[index]
                        && rows_semantically_equal(
                            candidate,
                            inserted,
                            equivalences,
                            context,
                            registry,
                        )?
                    {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                }
                for previous in &delta.inserted[..insert_index] {
                    if rows_semantically_equal(previous, inserted, equivalences, context, registry)?
                    {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                }
            }
        }
        Ok(RelationMutationPlan {
            remove_indices,
            inserted: delta.inserted.clone(),
            canonical_keys: None,
        })
    }

    fn commit_relation_mutation(
        value: &mut RelationValue,
        canonical_lookup: &mut Option<CanonicalRowPositionIndex>,
        mut plan: RelationMutationPlan,
    ) {
        if let (Some(lookup), Some(keys)) = (canonical_lookup.as_mut(), plan.canonical_keys.take())
        {
            let mut removals = plan
                .remove_indices
                .into_iter()
                .zip(keys.removed)
                .collect::<Vec<_>>();
            removals.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
            let rows = value.rows_mut();
            for (index, key) in removals {
                let last_index = rows.len() - 1;
                let bucket = lookup
                    .by_key
                    .get_mut(&key)
                    .expect("planned canonical Scan removal key must exist");
                let position = bucket
                    .iter()
                    .position(|candidate| *candidate == index)
                    .expect("planned canonical Scan removal index must exist");
                bucket.swap_remove(position);
                if bucket.is_empty() {
                    lookup.by_key.remove(&key);
                }
                if index != last_index {
                    rows.swap(index, last_index);
                    let moved_key = lookup.by_position[last_index].clone();
                    let moved_bucket = lookup
                        .by_key
                        .get_mut(&moved_key)
                        .expect("moved canonical Scan key must exist");
                    let moved_position = moved_bucket
                        .iter()
                        .position(|candidate| *candidate == last_index)
                        .expect("moved canonical Scan index must exist");
                    moved_bucket[moved_position] = index;
                    lookup.by_position.swap(index, last_index);
                }
                rows.pop();
                lookup.by_position.pop();
            }
            for (row, key) in plan.inserted.into_iter().zip(keys.inserted) {
                let index = rows.len();
                rows.push(row);
                lookup.by_position.push(key.clone());
                lookup.by_key.entry(key).or_default().push(index);
            }
            return;
        }

        plan.remove_indices
            .sort_unstable_by(|left, right| right.cmp(left));
        let rows = value.rows_mut();
        for index in plan.remove_indices {
            rows.remove(index);
        }
        rows.extend(plan.inserted);
        *canonical_lookup = None;
    }

    fn build_i64_buckets(
        rows: &[Row],
        column: usize,
    ) -> Result<BTreeMap<i64, Vec<Row>>, RelQueryError> {
        let mut buckets = BTreeMap::<i64, Vec<Row>>::new();
        for row in rows {
            let Value::I64(key) = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)? else {
                return Err(RelQueryError::TypeMismatch);
            };
            buckets.entry(*key).or_default().push(row.clone());
        }
        Ok(buckets)
    }

    fn join_pair(left: &Row, right: &Row) -> Row {
        let mut joined = Vec::with_capacity(left.len() + right.len());
        joined.extend(left.iter().cloned());
        joined.extend(right.iter().cloned());
        joined
    }

    fn plan_i64_side_view<D: DeltaView<Row>>(
        current: &BTreeMap<i64, Vec<Row>>,
        delta: &D,
        column: usize,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedI64JoinSide, RelQueryError> {
        type WeightedRowsByKey = BTreeMap<i64, (Vec<(usize, Row)>, Vec<(usize, Row)>)>;

        let normalized = normalized_delta_rows(delta)?;
        let mut changes = WeightedRowsByKey::new();
        for (magnitude, row) in normalized.removed {
            let Value::I64(key) = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)? else {
                return Err(RelQueryError::TypeMismatch);
            };
            changes.entry(*key).or_default().0.push((magnitude, row));
        }
        for (magnitude, row) in normalized.inserted {
            let Value::I64(key) = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)? else {
                return Err(RelQueryError::TypeMismatch);
            };
            changes.entry(*key).or_default().1.push((magnitude, row));
        }

        let equivalences = relation_column_equivalences(ty);
        let is_set = matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. });
        let mut planned = BTreeMap::new();
        for (key, (removed, inserted)) in changes {
            let mut bucket = current.get(&key).cloned().unwrap_or_default();
            for (magnitude, row) in removed {
                for _ in 0..magnitude {
                    let mut found = None;
                    for (index, candidate) in bucket.iter().enumerate() {
                        if rows_semantically_equal(
                            candidate,
                            &row,
                            equivalences,
                            context,
                            registry,
                        )? {
                            found = Some(index);
                            break;
                        }
                    }
                    let Some(index) = found else {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    };
                    bucket.remove(index);
                }
            }
            for (magnitude, row) in inserted {
                for _ in 0..magnitude {
                    if is_set {
                        for candidate in &bucket {
                            if rows_semantically_equal(
                                candidate,
                                &row,
                                equivalences,
                                context,
                                registry,
                            )? {
                                return Err(RelQueryError::InconsistentIncrementalDelta);
                            }
                        }
                    }
                    bucket.push(row.clone());
                }
            }
            planned.insert(key, bucket);
        }
        Ok(PlannedI64JoinSide { buckets: planned })
    }

    fn plan_i64_join_effect<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        left: &BTreeMap<i64, Vec<Row>>,
        right: &BTreeMap<i64, Vec<Row>>,
        planned_left: &BTreeMap<i64, Vec<Row>>,
        left_delta: &LD,
        right_delta: &RD,
        spec: JoinI64MaintenanceSpec<'_>,
    ) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut error = None;
        left_delta.visit(|weight, row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match row.get(spec.left_column) {
                Some(Value::I64(key)) => *key,
                Some(_) => {
                    error = Some(RelQueryError::TypeMismatch);
                    return;
                }
                None => {
                    error = Some(RelQueryError::ColumnOutOfBounds);
                    return;
                }
            };
            if let Some(right_bucket) = right.get(&key) {
                for right_row in right_bucket {
                    effect.push_weighted(weight, Self::join_pair(row, right_row));
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }

        right_delta.visit(|weight, row| {
            if weight == 0 || error.is_some() {
                return;
            }
            let key = match row.get(spec.right_column) {
                Some(Value::I64(key)) => *key,
                Some(_) => {
                    error = Some(RelQueryError::TypeMismatch);
                    return;
                }
                None => {
                    error = Some(RelQueryError::ColumnOutOfBounds);
                    return;
                }
            };
            let left_bucket = planned_left
                .get(&key)
                .or_else(|| left.get(&key))
                .map_or(&[][..], Vec::as_slice);
            for left_row in left_bucket {
                effect.push_weighted(weight, Self::join_pair(left_row, row));
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(effect)
    }

    fn commit_i64_buckets(target: &mut BTreeMap<i64, Vec<Row>>, planned: BTreeMap<i64, Vec<Row>>) {
        for (key, bucket) in planned {
            if bucket.is_empty() {
                target.remove(&key);
            } else {
                target.insert(key, bucket);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedJoinGroupTopKState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    join: MaterializedJoinDeltaState,
    group: MaterializedGroupDeltaState,
    top_k: MaterializedTopKDeltaState,
}

impl MaterializedJoinGroupTopKState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let RelExpr::TopKWithTies {
            input: group_query, ..
        } = query
        else {
            return Ok(None);
        };
        let RelExpr::Group {
            input: join_query, ..
        } = group_query.as_ref()
        else {
            return Ok(None);
        };
        if !matches!(join_query.as_ref(), RelExpr::JoinEq { .. }) {
            return Ok(None);
        }
        let Some(join) = MaterializedJoinDeltaState::build(join_query, old, context, registry)?
        else {
            return Ok(None);
        };
        let join_snapshot = join.output_value(context, registry)?;
        let Some(group) = MaterializedGroupDeltaState::build_from_input_value(
            group_query,
            join_snapshot,
            context,
            registry,
        )?
        else {
            return Ok(None);
        };
        let group_snapshot = group.output_value()?;
        let Some(top_k) = MaterializedTopKDeltaState::build_from_input_value(
            query,
            group_snapshot,
            context,
            registry,
        )?
        else {
            return Ok(None);
        };
        if !matches!(join.storage, MaintainedJoinStorage::I64 { .. })
            || !group.fast_i64_count
            || !matches!(
                top_k.storage,
                MaintainedTopKStorage::I64Scalar(_) | MaintainedTopKStorage::I64Rows(_)
            )
        {
            return Ok(None);
        }
        Ok(Some(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            join,
            group,
            top_k,
        }))
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    pub fn apply_join_input_deltas(
        &mut self,
        left_delta: &RelationDelta,
        right_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        // The owned tree is admitted only for the total exact-I64 fast fragment. Leaf deltas
        // are validated atomically by the Join state; downstream deltas are generated
        // internally and are compatible with the pinned Group/TopK states by construction.
        let join_delta =
            self.join
                .apply_input_deltas(left_delta, right_delta, context, registry)?;
        let group_delta = self
            .group
            .apply_input_delta(&join_delta, context, registry)?;
        self.top_k
            .apply_input_delta(&group_delta, context, registry)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaintainedBlockerKind {
    Difference,
    AntiJoin {
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DifferenceBlockerClass {
    left: Vec<Row>,
    right: Vec<Row>,
}

impl DifferenceBlockerClass {
    fn output_count(&self) -> usize {
        self.left.len().saturating_sub(self.right.len())
    }

    fn representative(&self) -> Option<&Row> {
        self.left.first()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AntiJoinBlockerClass {
    left: Vec<Row>,
    right_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedBlockerStorage {
    Difference {
        equivalences: Vec<kernel_types::SemanticId>,
        classes: BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
    },
    AntiJoin {
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        classes: BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedBlockerDeltaState {
    left_type: RelType,
    right_type: RelType,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    storage: MaintainedBlockerStorage,
}

#[derive(Debug)]
enum BlockerDeltaPatch {
    Difference(Vec<(CanonicalRowKey, DifferenceBlockerClass)>),
    AntiJoin(Vec<(kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass)>),
}

struct BlockerBuildSpec<'a> {
    kind: &'a MaintainedBlockerKind,
    left_type: RelType,
    right_type: RelType,
    result_type: RelType,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Clone, Copy)]
enum DifferenceBlockerSide {
    Left,
    Right,
}

type DifferenceBlockerWrites = Vec<(CanonicalRowKey, DifferenceBlockerClass)>;
type DifferenceBlockerEffect = (AdaptiveDelta<Row, 4>, DifferenceBlockerWrites);
type AntiJoinBlockerWrites = Vec<(kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass)>;
type AntiJoinBlockerEffect = (AdaptiveDelta<Row, 4>, AntiJoinBlockerWrites);
type AntiJoinChangeMap = BTreeMap<kernel_semantics::CanonicalEqKey, Vec<(usize, Row)>>;

struct AntiJoinLeftChanges {
    removed: AntiJoinChangeMap,
    inserted: AntiJoinChangeMap,
}

impl MaterializedBlockerDeltaState {
    fn build(
        left: &RelationValue,
        right: &RelationValue,
        spec: BlockerBuildSpec<'_>,
    ) -> Result<Self, RelQueryError> {
        let BlockerBuildSpec {
            kind,
            left_type,
            right_type,
            result_type,
            context,
            registry,
        } = spec;
        MaterializedSetSupportState::validate_rows(left.rows(), &left_type, context, registry)?;
        MaterializedSetSupportState::validate_rows(right.rows(), &right_type, context, registry)?;
        let storage = match kind {
            MaintainedBlockerKind::Difference => {
                let equivalences = relation_column_equivalences(&result_type).to_vec();
                let mut classes = BTreeMap::<CanonicalRowKey, DifferenceBlockerClass>::new();
                for row in left.rows() {
                    let key = canonical_row_key(row, &equivalences, context, registry)?;
                    classes
                        .entry(key)
                        .or_insert_with(|| DifferenceBlockerClass {
                            left: Vec::new(),
                            right: Vec::new(),
                        })
                        .left
                        .push(row.clone());
                }
                for row in right.rows() {
                    let key = canonical_row_key(row, &equivalences, context, registry)?;
                    classes
                        .entry(key)
                        .or_insert_with(|| DifferenceBlockerClass {
                            left: Vec::new(),
                            right: Vec::new(),
                        })
                        .right
                        .push(row.clone());
                }
                MaintainedBlockerStorage::Difference {
                    equivalences,
                    classes,
                }
            }
            MaintainedBlockerKind::AntiJoin {
                left_column,
                right_column,
                equivalence,
            } => {
                let mut classes =
                    BTreeMap::<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>::new();
                for row in left.rows() {
                    let key =
                        Self::anti_join_key(row, *left_column, *equivalence, context, registry)?;
                    classes
                        .entry(key)
                        .or_insert_with(|| AntiJoinBlockerClass {
                            left: Vec::new(),
                            right_count: 0,
                        })
                        .left
                        .push(row.clone());
                }
                for row in right.rows() {
                    let key =
                        Self::anti_join_key(row, *right_column, *equivalence, context, registry)?;
                    classes
                        .entry(key)
                        .or_insert_with(|| AntiJoinBlockerClass {
                            left: Vec::new(),
                            right_count: 0,
                        })
                        .right_count += 1;
                }
                MaintainedBlockerStorage::AntiJoin {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                    classes,
                }
            }
        };
        Ok(Self {
            left_type,
            right_type,
            result_type,
            semantic_context: context.clone(),
            storage,
        })
    }

    fn anti_join_key(
        row: &Row,
        column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_semantics::CanonicalEqKey, RelQueryError> {
        let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
        registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(RelQueryError::from)
    }

    fn output_value(&self) -> RelationValue {
        let mut rows = Vec::new();
        match &self.storage {
            MaintainedBlockerStorage::Difference { classes, .. } => {
                for class in classes.values() {
                    let count = class.output_count();
                    if count == 0 {
                        continue;
                    }
                    if let Some(row) = class.representative() {
                        rows.extend(std::iter::repeat_n(row.clone(), count));
                    }
                }
            }
            MaintainedBlockerStorage::AntiJoin { classes, .. } => {
                for class in classes.values() {
                    if class.right_count == 0 {
                        rows.extend(class.left.iter().cloned());
                    }
                }
            }
        }
        relation_value_from_rows(rows, &self.result_type)
    }

    fn plan_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        MaterializedJoinDeltaState::validate_delta_view_rows(
            left_delta,
            &self.left_type,
            context,
            registry,
        )?;
        MaterializedJoinDeltaState::validate_delta_view_rows(
            right_delta,
            &self.right_type,
            context,
            registry,
        )?;
        match &self.storage {
            MaintainedBlockerStorage::Difference {
                equivalences,
                classes,
            } => Self::plan_difference(
                equivalences,
                classes,
                left_delta,
                right_delta,
                context,
                registry,
            ),
            MaintainedBlockerStorage::AntiJoin {
                left_column,
                right_column,
                equivalence,
                classes,
            } => self.plan_anti_join(
                *left_column,
                *right_column,
                *equivalence,
                classes,
                left_delta,
                right_delta,
                context,
                registry,
            ),
        }
    }

    fn plan_difference<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        equivalences: &[kernel_types::SemanticId],
        classes: &BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let left = normalized_delta_rows(left_delta)?;
        let right = normalized_delta_rows(right_delta)?;
        let mut local = BTreeMap::<CanonicalRowKey, DifferenceBlockerClass>::new();
        Self::apply_difference_delta(
            &mut local,
            classes,
            equivalences,
            &left,
            DifferenceBlockerSide::Left,
            context,
            registry,
        )?;
        Self::apply_difference_delta(
            &mut local,
            classes,
            equivalences,
            &right,
            DifferenceBlockerSide::Right,
            context,
            registry,
        )?;
        let (effect, writes) = Self::difference_effect(classes, local)?;
        Ok(PlannedDeltaEffect {
            patch: BlockerDeltaPatch::Difference(writes),
            effect,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_difference_delta(
        local: &mut BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
        classes: &BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
        equivalences: &[kernel_types::SemanticId],
        delta: &NormalizedDeltaRows,
        side: DifferenceBlockerSide,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        for (magnitude, row) in &delta.removed {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes
                    .get(&key)
                    .cloned()
                    .unwrap_or(DifferenceBlockerClass {
                        left: Vec::new(),
                        right: Vec::new(),
                    })
            });
            let rows = match side {
                DifferenceBlockerSide::Left => &mut class.left,
                DifferenceBlockerSide::Right => &mut class.right,
            };
            if rows.len() < *magnitude {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            rows.truncate(rows.len() - *magnitude);
        }
        for (magnitude, row) in &delta.inserted {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes
                    .get(&key)
                    .cloned()
                    .unwrap_or(DifferenceBlockerClass {
                        left: Vec::new(),
                        right: Vec::new(),
                    })
            });
            let rows = match side {
                DifferenceBlockerSide::Left => &mut class.left,
                DifferenceBlockerSide::Right => &mut class.right,
            };
            rows.extend(std::iter::repeat_n(row.clone(), *magnitude));
        }
        Ok(())
    }

    fn difference_effect(
        classes: &BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
        local: BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
    ) -> Result<DifferenceBlockerEffect, RelQueryError> {
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut writes = Vec::with_capacity(local.len());
        for (key, after) in local {
            let before = classes
                .get(&key)
                .cloned()
                .unwrap_or(DifferenceBlockerClass {
                    left: Vec::new(),
                    right: Vec::new(),
                });
            let before_count = before.output_count();
            let after_count = after.output_count();
            if after_count > before_count {
                let row = after
                    .representative()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                effect.push_weighted(
                    i64::try_from(after_count - before_count)
                        .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?,
                    row.clone(),
                );
            } else if before_count > after_count {
                let row = before
                    .representative()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                effect.push_weighted(
                    -i64::try_from(before_count - after_count)
                        .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?,
                    row.clone(),
                );
            }
            writes.push((key, after));
        }
        Ok((effect, writes))
    }

    fn remove_anti_join_left_occurrence(
        rows: &mut Vec<Row>,
        target: &Row,
        equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        for (index, candidate) in rows.iter().enumerate() {
            if rows_semantically_equal(candidate, target, equivalences, context, registry)? {
                rows.remove(index);
                return Ok(());
            }
        }
        Err(RelQueryError::InconsistentIncrementalDelta)
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_anti_join<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        classes: &BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let left = normalized_delta_rows(left_delta)?;
        let right = normalized_delta_rows(right_delta)?;
        let mut local = BTreeMap::<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>::new();
        let changes = self.apply_anti_join_left_delta(
            &mut local,
            classes,
            &left,
            left_column,
            equivalence,
            context,
            registry,
        )?;
        Self::apply_anti_join_right_delta(
            &mut local,
            classes,
            &right,
            right_column,
            equivalence,
            context,
            registry,
        )?;
        let (effect, writes) = Self::anti_join_effect(classes, local, &changes)?;
        Ok(PlannedDeltaEffect {
            patch: BlockerDeltaPatch::AntiJoin(writes),
            effect,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_anti_join_left_delta(
        &self,
        local: &mut BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        classes: &BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        delta: &NormalizedDeltaRows,
        left_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<AntiJoinLeftChanges, RelQueryError> {
        let left_equivalences = relation_column_equivalences(&self.left_type);
        let mut removed = AntiJoinChangeMap::new();
        let mut inserted = AntiJoinChangeMap::new();
        for (magnitude, row) in &delta.removed {
            let key = Self::anti_join_key(row, left_column, equivalence, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes.get(&key).cloned().unwrap_or(AntiJoinBlockerClass {
                    left: Vec::new(),
                    right_count: 0,
                })
            });
            for _ in 0..*magnitude {
                Self::remove_anti_join_left_occurrence(
                    &mut class.left,
                    row,
                    left_equivalences,
                    context,
                    registry,
                )?;
            }
            removed
                .entry(key)
                .or_default()
                .push((*magnitude, row.clone()));
        }
        for (magnitude, row) in &delta.inserted {
            let key = Self::anti_join_key(row, left_column, equivalence, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes.get(&key).cloned().unwrap_or(AntiJoinBlockerClass {
                    left: Vec::new(),
                    right_count: 0,
                })
            });
            class
                .left
                .extend(std::iter::repeat_n(row.clone(), *magnitude));
            inserted
                .entry(key)
                .or_default()
                .push((*magnitude, row.clone()));
        }
        Ok(AntiJoinLeftChanges { removed, inserted })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_anti_join_right_delta(
        local: &mut BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        classes: &BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        delta: &NormalizedDeltaRows,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        for (magnitude, row) in &delta.removed {
            let key = Self::anti_join_key(row, right_column, equivalence, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes.get(&key).cloned().unwrap_or(AntiJoinBlockerClass {
                    left: Vec::new(),
                    right_count: 0,
                })
            });
            if class.right_count < *magnitude {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            class.right_count -= *magnitude;
        }
        for (magnitude, row) in &delta.inserted {
            let key = Self::anti_join_key(row, right_column, equivalence, context, registry)?;
            let class = local.entry(key.clone()).or_insert_with(|| {
                classes.get(&key).cloned().unwrap_or(AntiJoinBlockerClass {
                    left: Vec::new(),
                    right_count: 0,
                })
            });
            class.right_count = class
                .right_count
                .checked_add(*magnitude)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        }
        Ok(())
    }

    fn anti_join_effect(
        classes: &BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        local: BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        changes: &AntiJoinLeftChanges,
    ) -> Result<AntiJoinBlockerEffect, RelQueryError> {
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut writes = Vec::with_capacity(local.len());
        for (key, after) in local {
            let before = classes.get(&key).cloned().unwrap_or(AntiJoinBlockerClass {
                left: Vec::new(),
                right_count: 0,
            });
            match (before.right_count == 0, after.right_count == 0) {
                (true, true) => {
                    Self::append_anti_join_changes(&mut effect, changes.removed.get(&key), -1)?;
                    Self::append_anti_join_changes(&mut effect, changes.inserted.get(&key), 1)?;
                }
                (true, false) => {
                    for row in &before.left {
                        effect.push_weighted(-1, row.clone());
                    }
                }
                (false, true) => {
                    for row in &after.left {
                        effect.push_weighted(1, row.clone());
                    }
                }
                (false, false) => {}
            }
            writes.push((key, after));
        }
        Ok((effect, writes))
    }

    fn append_anti_join_changes(
        effect: &mut AdaptiveDelta<Row, 4>,
        changes: Option<&Vec<(usize, Row)>>,
        sign: i64,
    ) -> Result<(), RelQueryError> {
        let Some(changes) = changes else {
            return Ok(());
        };
        for (magnitude, row) in changes {
            let magnitude = i64::try_from(*magnitude)
                .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
            effect.push_weighted(
                sign.checked_mul(magnitude)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                row.clone(),
            );
        }
        Ok(())
    }

    fn commit_patch(&mut self, patch: BlockerDeltaPatch) {
        match (&mut self.storage, patch) {
            (
                MaintainedBlockerStorage::Difference { classes, .. },
                BlockerDeltaPatch::Difference(writes),
            ) => {
                for (key, class) in writes {
                    if class.left.is_empty() && class.right.is_empty() {
                        classes.remove(&key);
                    } else {
                        classes.insert(key, class);
                    }
                }
            }
            (
                MaintainedBlockerStorage::AntiJoin { classes, .. },
                BlockerDeltaPatch::AntiJoin(writes),
            ) => {
                for (key, class) in writes {
                    if class.left.is_empty() && class.right_count == 0 {
                        classes.remove(&key);
                    } else {
                        classes.insert(key, class);
                    }
                }
            }
            _ => unreachable!("blocker patch/storage mismatch"),
        }
    }
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedRelPlanNode {
    Scan {
        relation: kernel_types::SemanticId,
        value: RelationValue,
        handles: Option<MaintainedLeafHandles>,
        canonical_lookup: Option<CanonicalRowPositionIndex>,
    },
    Filter {
        input: Box<MaterializedRelPlanState>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterColumns {
        input: Box<MaterializedRelPlanState>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    ProjectBag {
        input: Box<MaterializedRelPlanState>,
        columns: Vec<usize>,
    },
    ProjectSet {
        input: Box<MaterializedRelPlanState>,
        columns: Vec<usize>,
        supports: MaterializedSetSupportState,
    },
    Distinct {
        input: Box<MaterializedRelPlanState>,
        supports: MaterializedSetSupportState,
    },
    PromoteToBag {
        input: Box<MaterializedRelPlanState>,
    },
    Blocker {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
        state: MaterializedBlockerDeltaState,
    },
    Join {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
        state: MaterializedJoinDeltaState,
    },
    Group {
        input: Box<MaterializedRelPlanState>,
        state: MaterializedGroupDeltaState,
    },
    TopK {
        input: Box<MaterializedRelPlanState>,
        state: MaterializedTopKDeltaState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatMaintainedRelPlanNode {
    result_type: RelType,
    kind: FlatMaintainedRelPlanNodeKind,
}

struct BuiltFlatMaintainedSubtree {
    id: NodeId,
    output: RelationValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlatMaintainedRelPlanNodeKind {
    Scan {
        relation: kernel_types::SemanticId,
        value: RelationValue,
        handles: Option<MaintainedLeafHandles>,
        canonical_lookup: Option<CanonicalRowPositionIndex>,
    },
    Filter {
        input: NodeId,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterColumns {
        input: NodeId,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    ProjectBag {
        input: NodeId,
        columns: Vec<usize>,
    },
    ProjectSet {
        input: NodeId,
        columns: Vec<usize>,
        supports: MaterializedSetSupportState,
    },
    Distinct {
        input: NodeId,
        supports: MaterializedSetSupportState,
    },
    PromoteToBag {
        input: NodeId,
    },
    Blocker {
        left: NodeId,
        right: NodeId,
        state: MaterializedBlockerDeltaState,
    },
    Join {
        left: NodeId,
        right: NodeId,
        state: MaterializedJoinDeltaState,
    },
    Group {
        input: NodeId,
        state: MaterializedGroupDeltaState,
    },
    TopK {
        input: NodeId,
        state: MaterializedTopKDeltaState,
    },
}

#[derive(Debug)]
enum MaintainedGroupCommitPatch {
    Sealed(sealed_group_v3::SealedGroupPatch),
}

#[derive(Debug)]
#[cfg(debug_assertions)]
enum MaintainedRelPlanPatch {
    Scan(Option<MaintainedScanCommitPatch>),
    Unary(Box<MaintainedRelPlanPatch>),
    SetUnary {
        input: Box<MaintainedRelPlanPatch>,
        patch: SetSupportPatch,
    },
    Blocker {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
        patch: BlockerDeltaPatch,
    },
    Join {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
        patch: JoinDeltaPatch,
    },
    Group {
        input: Box<MaintainedRelPlanPatch>,
        patch: MaintainedGroupCommitPatch,
    },
    TopK {
        input: Box<MaintainedRelPlanPatch>,
        patch: TopKDeltaPatch,
    },
}

#[derive(Debug)]
enum GraphNodePatch {
    Scan(MaintainedScanCommitPatch),
    SetSupport(SetSupportPatch),
    Blocker(BlockerDeltaPatch),
    Join(JoinDeltaPatch),
    Group(MaintainedGroupCommitPatch),
    TopK(TopKDeltaPatch),
}

#[derive(Debug)]
struct GraphPatchSet {
    nodes: Vec<Option<GraphNodePatch>>,
    root_effect: MaintainedDelta,
}

struct PlannedGraphNodeTransition {
    patch: Option<GraphNodePatch>,
    effect: MaintainedDelta,
}

#[derive(Debug)]
#[cfg(debug_assertions)]
struct PlannedMaintainedRelPlanTransition {
    patch: MaintainedRelPlanPatch,
    effect: MaintainedDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaintainedLeafLink {
    previous: Option<kernel_types::StableRowHandle>,
    next: Option<kernel_types::StableRowHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaintainedLeafHandles {
    dense_ids: Vec<kernel_types::StableRowHandle>,
    positions: BTreeMap<kernel_types::StableRowHandle, usize>,
    links: BTreeMap<kernel_types::StableRowHandle, MaintainedLeafLink>,
    logical_head: Option<kernel_types::StableRowHandle>,
    logical_tail: Option<kernel_types::StableRowHandle>,
}

impl MaintainedLeafHandles {
    fn new(ids: Vec<kernel_types::StableRowHandle>) -> Result<Self, RelQueryError> {
        let mut positions = BTreeMap::new();
        let mut links = BTreeMap::new();
        for (position, id) in ids.iter().copied().enumerate() {
            if positions.insert(id, position).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let link = MaintainedLeafLink {
                previous: position.checked_sub(1).map(|index| ids[index]),
                next: ids.get(position + 1).copied(),
            };
            links.insert(id, link);
        }
        Ok(Self {
            logical_head: ids.first().copied(),
            logical_tail: ids.last().copied(),
            dense_ids: ids,
            positions,
            links,
        })
    }

    fn row_for_handle<'a>(
        &self,
        value: &'a RelationValue,
        id: kernel_types::StableRowHandle,
    ) -> Result<&'a Row, RelQueryError> {
        let position = self
            .positions
            .get(&id)
            .copied()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        value
            .rows()
            .get(position)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn ordered_rows(&self, value: &RelationValue) -> Result<Vec<Row>, RelQueryError> {
        if value.rows().len() != self.dense_ids.len()
            || self.positions.len() != self.dense_ids.len()
            || self.links.len() != self.dense_ids.len()
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut rows = Vec::with_capacity(self.dense_ids.len());
        let mut current = self.logical_head;
        while let Some(id) = current {
            if rows.len() >= self.dense_ids.len() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            rows.push(self.row_for_handle(value, id)?.clone());
            current = self
                .links
                .get(&id)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                .next;
        }
        if rows.len() != self.dense_ids.len() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Ok(rows)
    }

    fn remove(
        &mut self,
        value: &mut RelationValue,
        id: kernel_types::StableRowHandle,
    ) -> Result<(), RelQueryError> {
        let position = self
            .positions
            .remove(&id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let link = self
            .links
            .remove(&id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;

        match link.previous {
            Some(previous) => {
                self.links
                    .get_mut(&previous)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                    .next = link.next;
            }
            None => self.logical_head = link.next,
        }
        match link.next {
            Some(next) => {
                self.links
                    .get_mut(&next)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                    .previous = link.previous;
            }
            None => self.logical_tail = link.previous,
        }

        let rows = value.rows_mut();
        rows.swap_remove(position);
        let removed_id = self.dense_ids.swap_remove(position);
        if removed_id != id {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        if position < self.dense_ids.len() {
            self.positions.insert(self.dense_ids[position], position);
        }
        Ok(())
    }

    fn insert(
        &mut self,
        value: &mut RelationValue,
        id: kernel_types::StableRowHandle,
        row: Row,
    ) -> Result<(), RelQueryError> {
        if self.positions.contains_key(&id) || self.links.contains_key(&id) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let rows = value.rows_mut();
        let position = rows.len();
        rows.push(row);
        self.dense_ids.push(id);
        self.positions.insert(id, position);
        self.links.insert(
            id,
            MaintainedLeafLink {
                previous: self.logical_tail,
                next: None,
            },
        );
        if let Some(previous) = self.logical_tail {
            self.links
                .get_mut(&previous)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                .next = Some(id);
        } else {
            self.logical_head = Some(id);
        }
        self.logical_tail = Some(id);
        Ok(())
    }
}

fn collect_flat_maintained_state_requirements(
    arena: &[Arc<FlatMaintainedRelPlanNode>],
    out: &mut BTreeSet<RelDifferentialStateRequirement>,
) {
    for node in arena {
        match node.kind {
            FlatMaintainedRelPlanNodeKind::Scan { .. }
            | FlatMaintainedRelPlanNodeKind::Filter { .. }
            | FlatMaintainedRelPlanNodeKind::FilterColumns { .. }
            | FlatMaintainedRelPlanNodeKind::ProjectBag { .. }
            | FlatMaintainedRelPlanNodeKind::PromoteToBag { .. } => {}
            FlatMaintainedRelPlanNodeKind::ProjectSet { .. }
            | FlatMaintainedRelPlanNodeKind::Distinct { .. } => {
                out.insert(RelDifferentialStateRequirement::SetSupport);
            }
            FlatMaintainedRelPlanNodeKind::Blocker { .. } => {
                out.insert(RelDifferentialStateRequirement::BlockerMass);
            }
            FlatMaintainedRelPlanNodeKind::Join { .. } => {
                out.insert(RelDifferentialStateRequirement::JoinFibers);
            }
            FlatMaintainedRelPlanNodeKind::Group { .. } => {
                out.insert(RelDifferentialStateRequirement::GroupAnnotations);
            }
            FlatMaintainedRelPlanNodeKind::TopK { .. } => {
                out.insert(RelDifferentialStateRequirement::OrderedCut);
            }
        }
    }
}

/// Flat `NodeId` owner for maintained relational execution state.
///
/// Construction may use a temporary bottom-up tree to reuse the operator builders, but
/// release states discard that tree after flattening. Runtime planning, validation,
/// output reconstruction, and commit address authoritative state only through the
/// compiled graph's stable postorder `NodeId` coordinates.
#[derive(Debug)]
pub struct MaterializedRelPlanState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    differential: Arc<RelDifferentialProgram>,
    result_type: RelType,
    #[cfg(debug_assertions)]
    node: Option<Arc<MaintainedRelPlanNode>>,
    arena: Arc<Vec<Arc<FlatMaintainedRelPlanNode>>>,
    execgraph_scratch: UnifiedTransitionScratch<MaintainedDelta>,
    transition_epoch: u64,
    revision: Option<kernel_types::RevisionId>,
}

type MaintainedDelta = AdaptiveDelta<Row, 4>;

impl Clone for MaterializedRelPlanState {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            semantic_context: self.semantic_context.clone(),
            differential: Arc::clone(&self.differential),
            result_type: self.result_type.clone(),
            #[cfg(debug_assertions)]
            node: self.node.as_ref().map(Arc::clone),
            arena: Arc::clone(&self.arena),
            execgraph_scratch: UnifiedTransitionScratch::default(),
            transition_epoch: self.transition_epoch,
            revision: self.revision,
        }
    }
}

impl PartialEq for MaterializedRelPlanState {
    fn eq(&self, other: &Self) -> bool {
        self.query == other.query
            && self.semantic_context == other.semantic_context
            && self.differential == other.differential
            && self.result_type == other.result_type
            && self.arena == other.arena
            && self.transition_epoch == other.transition_epoch
            && self.revision == other.revision
    }
}

impl Eq for MaterializedRelPlanState {}

impl MaterializedRelPlanState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let differential = Arc::new(RelDifferentialProgram::compile(query, context, registry)?);
        let graph = differential.physical_program().execution_graph();
        if !graph.has_typed_metadata() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let mut arena = Vec::with_capacity(graph.node_count());
        let built =
            Self::build_flat_subtree(query, old, context, registry, &differential, &mut arena)?;
        if built.id != graph.root() || arena.len() != graph.node_count() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let result_type = graph
            .result_type(built.id)
            .cloned()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut bound_requirements = BTreeSet::new();
        collect_flat_maintained_state_requirements(&arena, &mut bound_requirements);
        if bound_requirements != differential.state_requirements() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        let node = Some(Arc::new(Self::rehydrate_debug_tree(
            query,
            built.id,
            &arena,
            context,
            &differential,
        )?));
        Ok(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            differential,
            result_type,
            #[cfg(debug_assertions)]
            node,
            arena: Arc::new(arena),
            execgraph_scratch: UnifiedTransitionScratch::default(),
            transition_epoch: 0,
            revision: None,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn build_flat_subtree(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        differential: &Arc<RelDifferentialProgram>,
        out: &mut Vec<Arc<FlatMaintainedRelPlanNode>>,
    ) -> Result<BuiltFlatMaintainedSubtree, RelQueryError> {
        let graph = differential.physical_program().execution_graph();
        let (kind, output) = match query {
            RelExpr::Scan(relation) => {
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let value = relation_value_from_rows(
                    old.relations.get(relation).cloned().unwrap_or_default(),
                    result_type,
                );
                let canonical_lookup = canonical_row_position_index(
                    value.rows(),
                    relation_column_equivalences(result_type),
                    context,
                    registry,
                )?;
                (
                    FlatMaintainedRelPlanNodeKind::Scan {
                        relation: *relation,
                        value: value.clone(),
                        handles: None,
                        canonical_lookup,
                    },
                    value,
                )
            }
            RelExpr::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let input_type = graph
                    .result_type(input.id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let filtered = rel_delta_filter(
                    RelationDelta {
                        inserted: input.output.into_rows(),
                        removed: Vec::new(),
                        result_type: input_type,
                    },
                    *column,
                    value,
                    *equivalence,
                    context,
                    registry,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                (
                    FlatMaintainedRelPlanNodeKind::Filter {
                        input: input.id,
                        column: *column,
                        value: value.clone(),
                        equivalence: *equivalence,
                    },
                    relation_value_from_rows(filtered.inserted, result_type),
                )
            }
            RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let input_type = graph
                    .result_type(input.id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let filtered = rel_delta_filter_columns(
                    RelationDelta {
                        inserted: input.output.into_rows(),
                        removed: Vec::new(),
                        result_type: input_type,
                    },
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                (
                    FlatMaintainedRelPlanNodeKind::FilterColumns {
                        input: input.id,
                        left_column: *left_column,
                        right_column: *right_column,
                        equivalence: *equivalence,
                    },
                    relation_value_from_rows(filtered.inserted, result_type),
                )
            }
            RelExpr::Project { input, columns } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let rows = project_rows(input.output.into_rows(), columns)?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                if matches!(
                    graph
                        .result_type(input.id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                        .semantics,
                    kernel_schema::RelationSemantics::Set { .. }
                ) {
                    let supports =
                        MaterializedSetSupportState::build(&rows, result_type, context, registry)?;
                    let output = supports.output_value();
                    (
                        FlatMaintainedRelPlanNodeKind::ProjectSet {
                            input: input.id,
                            columns: columns.clone(),
                            supports,
                        },
                        output,
                    )
                } else {
                    let output = relation_value_from_rows(rows, &result_type);
                    (
                        FlatMaintainedRelPlanNodeKind::ProjectBag {
                            input: input.id,
                            columns: columns.clone(),
                        },
                        output,
                    )
                }
            }
            RelExpr::Difference { left, right } => {
                let left =
                    Self::build_flat_subtree(left, old, context, registry, differential, out)?;
                let right =
                    Self::build_flat_subtree(right, old, context, registry, differential, out)?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let state = MaterializedBlockerDeltaState::build(
                    &left.output,
                    &right.output,
                    BlockerBuildSpec {
                        kind: &MaintainedBlockerKind::Difference,
                        left_type: graph
                            .result_type(left.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        right_type: graph
                            .result_type(right.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        result_type,
                        context,
                        registry,
                    },
                )?;
                let output = state.output_value();
                (
                    FlatMaintainedRelPlanNodeKind::Blocker {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => {
                let left =
                    Self::build_flat_subtree(left, old, context, registry, differential, out)?;
                let right =
                    Self::build_flat_subtree(right, old, context, registry, differential, out)?;
                let id = out.len();
                let kind = MaintainedBlockerKind::AntiJoin {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                };
                let state = MaterializedBlockerDeltaState::build(
                    &left.output,
                    &right.output,
                    BlockerBuildSpec {
                        kind: &kind,
                        left_type: graph
                            .result_type(left.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        right_type: graph
                            .result_type(right.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        result_type: graph
                            .result_type(id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        context,
                        registry,
                    },
                )?;
                let output = state.output_value();
                (
                    FlatMaintainedRelPlanNodeKind::Blocker {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::Distinct { input, .. } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let supports = MaterializedSetSupportState::build(
                    input.output.rows(),
                    result_type,
                    context,
                    registry,
                )?;
                let output = supports.output_value();
                (
                    FlatMaintainedRelPlanNodeKind::Distinct {
                        input: input.id,
                        supports,
                    },
                    output,
                )
            }
            RelExpr::PromoteToBag(input) => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let output = RelationValue::Bag(input.output.into_rows());
                (
                    FlatMaintainedRelPlanNodeKind::PromoteToBag { input: input.id },
                    output,
                )
            }
            RelExpr::JoinEq { left, right, .. } => {
                let left =
                    Self::build_flat_subtree(left, old, context, registry, differential, out)?;
                let right =
                    Self::build_flat_subtree(right, old, context, registry, differential, out)?;
                let state = MaterializedJoinDeltaState::build_from_input_values(
                    query,
                    &left.output,
                    &right.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value(context, registry)?;
                (
                    FlatMaintainedRelPlanNodeKind::Join {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::Group { input, .. } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let state = MaterializedGroupDeltaState::build_from_input_value(
                    query,
                    input.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value()?;
                (
                    FlatMaintainedRelPlanNodeKind::Group {
                        input: input.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::TopKWithTies { input, .. } => {
                let input =
                    Self::build_flat_subtree(input, old, context, registry, differential, out)?;
                let state = MaterializedTopKDeltaState::build_from_input_value(
                    query,
                    input.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value();
                (
                    FlatMaintainedRelPlanNodeKind::TopK {
                        input: input.id,
                        state,
                    },
                    output,
                )
            }
        };
        let id = out.len();
        let result_type = graph
            .result_type(id)
            .cloned()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        out.push(Arc::new(FlatMaintainedRelPlanNode { result_type, kind }));
        Ok(BuiltFlatMaintainedSubtree { id, output })
    }

    #[cfg(debug_assertions)]
    #[allow(clippy::too_many_lines)]
    fn rehydrate_debug_tree(
        query: &RelExpr,
        node_id: NodeId,
        arena: &[Arc<FlatMaintainedRelPlanNode>],
        context: &kernel_schema::SemanticContext,
        differential: &Arc<RelDifferentialProgram>,
    ) -> Result<MaintainedRelPlanNode, RelQueryError> {
        let flat = arena
            .get(node_id)
            .map(Arc::as_ref)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let child_state =
            |child_query: &RelExpr, child_id: NodeId| -> Result<Box<Self>, RelQueryError> {
                let child = arena
                    .get(child_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok(Box::new(Self {
                    query: child_query.clone(),
                    semantic_context: context.clone(),
                    differential: Arc::clone(differential),
                    result_type: child.result_type.clone(),
                    node: Some(Arc::new(Self::rehydrate_debug_tree(
                        child_query,
                        child_id,
                        arena,
                        context,
                        differential,
                    )?)),
                    arena: Arc::new(Vec::new()),
                    execgraph_scratch: UnifiedTransitionScratch::default(),
                    transition_epoch: 0,
                    revision: None,
                }))
            };
        Ok(match (query, &flat.kind) {
            (
                RelExpr::Scan(relation),
                FlatMaintainedRelPlanNodeKind::Scan {
                    relation: flat_relation,
                    value,
                    handles,
                    canonical_lookup,
                },
            ) if relation == flat_relation => MaintainedRelPlanNode::Scan {
                relation: *relation,
                value: value.clone(),
                handles: handles.clone(),
                canonical_lookup: canonical_lookup.clone(),
            },
            (
                RelExpr::FilterEqConst {
                    input,
                    column,
                    value,
                    equivalence,
                },
                FlatMaintainedRelPlanNodeKind::Filter { input: id, .. },
            ) => MaintainedRelPlanNode::Filter {
                input: child_state(input, *id)?,
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            },
            (
                RelExpr::FilterEqColumns {
                    input,
                    left_column,
                    right_column,
                    equivalence,
                },
                FlatMaintainedRelPlanNodeKind::FilterColumns { input: id, .. },
            ) => MaintainedRelPlanNode::FilterColumns {
                input: child_state(input, *id)?,
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
            (
                RelExpr::Project { input, columns },
                FlatMaintainedRelPlanNodeKind::ProjectBag { input: id, .. },
            ) => MaintainedRelPlanNode::ProjectBag {
                input: child_state(input, *id)?,
                columns: columns.clone(),
            },
            (
                RelExpr::Project { input, columns },
                FlatMaintainedRelPlanNodeKind::ProjectSet {
                    input: id,
                    supports,
                    ..
                },
            ) => MaintainedRelPlanNode::ProjectSet {
                input: child_state(input, *id)?,
                columns: columns.clone(),
                supports: supports.clone(),
            },
            (
                RelExpr::Distinct { input, .. },
                FlatMaintainedRelPlanNodeKind::Distinct {
                    input: id,
                    supports,
                },
            ) => MaintainedRelPlanNode::Distinct {
                input: child_state(input, *id)?,
                supports: supports.clone(),
            },
            (
                RelExpr::PromoteToBag(input),
                FlatMaintainedRelPlanNodeKind::PromoteToBag { input: id },
            ) => MaintainedRelPlanNode::PromoteToBag {
                input: child_state(input, *id)?,
            },
            (
                RelExpr::Difference { left, right } | RelExpr::AntiJoin { left, right, .. },
                FlatMaintainedRelPlanNodeKind::Blocker {
                    left: left_id,
                    right: right_id,
                    state,
                },
            ) => MaintainedRelPlanNode::Blocker {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
                state: state.clone(),
            },
            (
                RelExpr::JoinEq { left, right, .. },
                FlatMaintainedRelPlanNodeKind::Join {
                    left: left_id,
                    right: right_id,
                    state,
                },
            ) => MaintainedRelPlanNode::Join {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
                state: state.clone(),
            },
            (
                RelExpr::Group { input, .. },
                FlatMaintainedRelPlanNodeKind::Group { input: id, state },
            ) => MaintainedRelPlanNode::Group {
                input: child_state(input, *id)?,
                state: state.clone(),
            },
            (
                RelExpr::TopKWithTies { input, .. },
                FlatMaintainedRelPlanNodeKind::TopK { input: id, state },
            ) => MaintainedRelPlanNode::TopK {
                input: child_state(input, *id)?,
                state: state.clone(),
            },
            _ => return Err(RelQueryError::InconsistentIncrementalDelta),
        })
    }

    /// Exact Γ-DTC program whose state requirements are bound by this maintained tree.
    ///
    /// This is reconstructible execution metadata, not semantic authority. Construction
    /// rejects any drift between the compiled differential requirements and the concrete
    /// maintained-state capabilities owned by the tree.
    #[must_use]
    pub fn differential(&self) -> &RelDifferentialProgram {
        self.differential.as_ref()
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn scan_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        self.arena
            .iter()
            .filter_map(|node| match &node.kind {
                FlatMaintainedRelPlanNodeKind::Scan { relation, .. } => Some(*relation),
                _ => None,
            })
            .collect()
    }

    pub fn output_value(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if self.arena.is_empty() {
            #[cfg(debug_assertions)]
            {
                return self.output_value_recursive(context, registry);
            }
            #[cfg(not(debug_assertions))]
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        let root = self
            .differential
            .physical_program()
            .execution_graph()
            .root();
        self.output_value_from_arena(root, context, registry)
    }

    fn output_value_from_arena(
        &self,
        node_id: NodeId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        let node = self
            .arena
            .get(node_id)
            .map(Arc::as_ref)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        match &node.kind {
            FlatMaintainedRelPlanNodeKind::Scan { value, handles, .. } => {
                let Some(handles) = handles else {
                    return Ok(value.clone());
                };
                Ok(relation_value_from_rows(
                    handles.ordered_rows(value)?,
                    &node.result_type,
                ))
            }
            FlatMaintainedRelPlanNodeKind::Filter {
                input,
                column,
                value,
                equivalence,
            } => {
                let input_node = self
                    .arena
                    .get(*input)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let input_value = self.output_value_from_arena(*input, context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input_node.result_type.clone(),
                };
                let filtered =
                    rel_delta_filter(input_delta, *column, value, *equivalence, context, registry)?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &node.result_type,
                ))
            }
            FlatMaintainedRelPlanNodeKind::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let input_node = self
                    .arena
                    .get(*input)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let input_value = self.output_value_from_arena(*input, context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input_node.result_type.clone(),
                };
                let filtered = rel_delta_filter_columns(
                    input_delta,
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &node.result_type,
                ))
            }
            FlatMaintainedRelPlanNodeKind::ProjectBag { input, columns } => {
                let rows = project_rows(
                    self.output_value_from_arena(*input, context, registry)?
                        .into_rows(),
                    columns,
                )?;
                Ok(relation_value_from_rows(rows, &node.result_type))
            }
            FlatMaintainedRelPlanNodeKind::ProjectSet { supports, .. }
            | FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => {
                Ok(supports.output_value())
            }
            FlatMaintainedRelPlanNodeKind::PromoteToBag { input } => Ok(RelationValue::Bag(
                self.output_value_from_arena(*input, context, registry)?
                    .into_rows(),
            )),
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => Ok(state.output_value()),
            FlatMaintainedRelPlanNodeKind::Join { state, .. } => {
                state.output_value(context, registry)
            }
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => state.output_value(),
            FlatMaintainedRelPlanNodeKind::TopK { state, .. } => Ok(state.output_value()),
        }
    }

    #[cfg(debug_assertions)]
    fn output_value_recursive(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        let node = self
            .node
            .as_deref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        match node {
            MaintainedRelPlanNode::Scan { value, handles, .. } => {
                let Some(handles) = handles else {
                    return Ok(value.clone());
                };
                Ok(relation_value_from_rows(
                    handles.ordered_rows(value)?,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::Filter {
                input,
                column,
                value,
                equivalence,
            } => {
                let input_value = input.output_value_recursive(context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input.result_type.clone(),
                };
                let filtered =
                    rel_delta_filter(input_delta, *column, value, *equivalence, context, registry)?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let input_value = input.output_value_recursive(context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input.result_type.clone(),
                };
                let filtered = rel_delta_filter_columns(
                    input_delta,
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::ProjectBag { input, columns } => {
                let rows = project_rows(
                    input.output_value_recursive(context, registry)?.into_rows(),
                    columns,
                )?;
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
            MaintainedRelPlanNode::ProjectSet { supports, .. }
            | MaintainedRelPlanNode::Distinct { supports, .. } => Ok(supports.output_value()),
            MaintainedRelPlanNode::PromoteToBag { input } => Ok(RelationValue::Bag(
                input.output_value_recursive(context, registry)?.into_rows(),
            )),
            MaintainedRelPlanNode::Blocker { state, .. } => Ok(state.output_value()),
            MaintainedRelPlanNode::Join { state, .. } => state.output_value(context, registry),
            MaintainedRelPlanNode::Group { state, .. } => state.output_value(),
            MaintainedRelPlanNode::TopK { state, .. } => Ok(state.output_value()),
        }
    }

    pub fn apply_relation_deltas(
        &mut self,
        deltas: &BTreeMap<kernel_types::SemanticId, RelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if self.revision.is_some() {
            return Err(RelQueryError::RevisionBoundMutationRequiresPreparedTransition);
        }
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let transition_program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for relation in deltas.keys() {
            if !transition_program.contains_source(*relation) {
                return Err(RelQueryError::UnknownRelation(*relation));
            }
        }
        #[cfg(debug_assertions)]
        let recursive_oracle = self.recursive_oracle_from_frames(
            self.validate_leaf_deltas(deltas, context, registry)?,
            context,
            registry,
        )?;
        let mut validated_frames = self.validate_leaf_deltas(deltas, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let planned =
            self.plan_relation_deltas_execgraph(&mut validated_frames, context, registry)?;
        let output = materialize_delta_view(&planned.root_effect, self.result_type.clone())?;
        #[cfg(debug_assertions)]
        if !relation_deltas_semantically_equivalent(
            &output,
            &recursive_oracle.1,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        self.commit_graph_patch_set(planned);
        #[cfg(debug_assertions)]
        if !relation_values_semantically_equivalent(
            &self.output_value(context, registry)?,
            &recursive_oracle
                .0
                .output_value_recursive(context, registry)?,
            &self.result_type,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        {
            self.node = recursive_oracle.0.node.as_ref().map(Arc::clone);
        }
        self.transition_epoch = next_epoch;
        Ok(output)
    }

    /// Attaches authoritative storage row identities to every `Scan` of one
    /// relation. Each handle is paired with the exact row payload in logical
    /// scan order. The binding is rejected unless it exactly matches the
    /// maintained Scan snapshot; semantic bag equality alone is insufficient.
    pub fn attach_storage_rows(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
    ) -> Result<(), RelQueryError> {
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let handles = self.validate_storage_rows_binding(relation, rows)?;
        #[cfg(debug_assertions)]
        self.attach_storage_rows_recursive(relation, rows)?;
        self.commit_storage_rows_binding(relation, &handles);
        self.transition_epoch = next_epoch;
        Ok(())
    }

    fn validate_storage_rows_binding(
        &self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
    ) -> Result<MaintainedLeafHandles, RelQueryError> {
        for node in self.arena.iter() {
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation: current,
                value,
                ..
            } = &node.kind
            else {
                continue;
            };
            if *current != relation {
                continue;
            }
            if value.rows().len() != rows.len()
                || value
                    .rows()
                    .iter()
                    .zip(rows.iter().map(|(_, row)| row))
                    .any(|(logical, physical)| logical != physical)
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        MaintainedLeafHandles::new(rows.iter().map(|(id, _)| *id).collect())
    }

    fn commit_storage_rows_binding(
        &mut self,
        relation: kernel_types::SemanticId,
        handles: &MaintainedLeafHandles,
    ) {
        let arena = Arc::make_mut(&mut self.arena);
        for node in arena {
            let node = Arc::make_mut(node);
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation: current,
                handles: target,
                ..
            } = &mut node.kind
            else {
                continue;
            };
            if *current == relation {
                *target = Some(handles.clone());
            }
        }
    }

    #[cfg(debug_assertions)]
    fn attach_storage_rows_recursive(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
    ) -> Result<(), RelQueryError> {
        match Arc::make_mut(
            self.node
                .as_mut()
                .expect("debug construction tree must exist while binding storage rows"),
        ) {
            MaintainedRelPlanNode::Scan {
                relation: current,
                value,
                handles,
                ..
            } => {
                if *current != relation {
                    return Ok(());
                }
                if value.rows().len() != rows.len()
                    || value
                        .rows()
                        .iter()
                        .zip(rows.iter().map(|(_, row)| row))
                        .any(|(logical, physical)| logical != physical)
                {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                *handles = Some(MaintainedLeafHandles::new(
                    rows.iter().map(|(id, _)| *id).collect(),
                )?);
                Ok(())
            }
            MaintainedRelPlanNode::Filter { input, .. }
            | MaintainedRelPlanNode::FilterColumns { input, .. }
            | MaintainedRelPlanNode::ProjectBag { input, .. }
            | MaintainedRelPlanNode::ProjectSet { input, .. }
            | MaintainedRelPlanNode::Distinct { input, .. }
            | MaintainedRelPlanNode::PromoteToBag { input }
            | MaintainedRelPlanNode::Group { input, .. }
            | MaintainedRelPlanNode::TopK { input, .. } => {
                input.attach_storage_rows_recursive(relation, rows)
            }
            MaintainedRelPlanNode::Blocker { left, right, .. }
            | MaintainedRelPlanNode::Join { left, right, .. } => {
                left.attach_storage_rows_recursive(relation, rows)?;
                right.attach_storage_rows_recursive(relation, rows)
            }
        }
    }

    /// Binds this maintained snapshot to the logical revision it represents.
    /// A bound state can advance only through a revision-aware prepared transition.
    pub fn bind_revision(
        &mut self,
        revision: kernel_types::RevisionId,
    ) -> Result<(), RelQueryError> {
        match self.revision {
            Some(current) if current == revision => return Ok(()),
            Some(_) => return Err(RelQueryError::RevisionBindingMismatch),
            None => {}
        }
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        self.revision = Some(revision);
        self.transition_epoch = next_epoch;
        Ok(())
    }

    #[must_use]
    pub const fn revision(&self) -> Option<kernel_types::RevisionId> {
        self.revision
    }

    #[must_use]
    pub const fn transition_epoch(&self) -> u64 {
        self.transition_epoch
    }

    /// Builds a detached candidate for a revision-bound maintained plan from
    /// storage-resolved row identities. This does not mutate or publish the
    /// source state; authoritative publication remains owned by `kernel-plan`.
    pub fn candidate_from_storage_resolved_deltas_for_revision(
        &self,
        source_revision: kernel_types::RevisionId,
        target_revision: kernel_types::RevisionId,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(Self, RelationDelta), RelQueryError> {
        if source_revision == target_revision {
            return Err(RelQueryError::InvalidRevisionTransition);
        }
        if self.revision != Some(source_revision) {
            return Err(RelQueryError::RevisionBindingMismatch);
        }
        let mut candidate = self.clone();
        let output_delta =
            candidate.apply_storage_resolved_deltas_in_place(deltas, context, registry)?;
        candidate.revision = Some(target_revision);
        Ok((candidate, output_delta))
    }

    /// Applies storage-resolved base-table deltas without repeating semantic
    /// membership search inside `Scan` nodes. Revision-bound states must use the
    /// joint prepared-transition path instead of this legacy convenience entrypoint.
    pub fn apply_storage_resolved_deltas(
        &mut self,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if self.revision.is_some() {
            return Err(RelQueryError::RevisionBoundMutationRequiresPreparedTransition);
        }
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let transition_program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for (relation, resolved) in deltas {
            if *relation != resolved.relation || !transition_program.contains_source(*relation) {
                return Err(RelQueryError::UnknownRelation(*relation));
            }
        }
        #[cfg(debug_assertions)]
        let recursive_oracle = self.recursive_oracle_from_frames(
            self.validate_resolved_leaf_frames(deltas, context, registry)?,
            context,
            registry,
        )?;
        let mut validated_frames = self.validate_resolved_leaf_frames(deltas, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let planned =
            self.plan_relation_deltas_execgraph(&mut validated_frames, context, registry)?;
        let output = materialize_delta_view(&planned.root_effect, self.result_type.clone())?;
        #[cfg(debug_assertions)]
        if !relation_deltas_semantically_equivalent(
            &output,
            &recursive_oracle.1,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        self.commit_graph_patch_set(planned);
        #[cfg(debug_assertions)]
        if !relation_values_semantically_equivalent(
            &self.output_value(context, registry)?,
            &recursive_oracle
                .0
                .output_value_recursive(context, registry)?,
            &self.result_type,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        {
            self.node = recursive_oracle.0.node.as_ref().map(Arc::clone);
        }
        self.transition_epoch = next_epoch;
        Ok(output)
    }

    /// Builds the exact same compiled-edge frame map as the semantic-delta
    /// planner, but seals Scan publication to already-resolved stable handles.
    /// All operator state above Scan therefore goes through one immutable
    /// plan/commit semantics regardless of the source delta representation.
    fn validate_resolved_leaf_frames(
        &self,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ValidatedLeafTransitionFrames, RelQueryError> {
        let mut frames = BTreeMap::new();
        let mut next_edge_ordinal = 0_u32;
        for node in self.arena.iter().map(Arc::as_ref) {
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation,
                value,
                handles,
                ..
            } = &node.kind
            else {
                continue;
            };
            let edge = CompiledDeltaEdgeIdentity::new(next_edge_ordinal, *relation);
            next_edge_ordinal = next_edge_ordinal
                .checked_add(1)
                .ok_or(RelQueryError::TransitionEpochExhausted)?;
            let Some(resolved) = deltas.get(relation) else {
                continue;
            };
            if resolved.delta.result_type != node.result_type
                || resolved.removed_handles.len() != resolved.delta.removed.len()
                || resolved.inserted_handles.len() != resolved.delta.inserted.len()
            {
                return Err(RelQueryError::TypeMismatch);
            }
            MaterializedSetSupportState::validate_rows(
                &resolved.delta.removed,
                &node.result_type,
                context,
                registry,
            )?;
            MaterializedSetSupportState::validate_rows(
                &resolved.delta.inserted,
                &node.result_type,
                context,
                registry,
            )?;
            let handles = handles
                .as_ref()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let mut seen = BTreeSet::new();
            for (id, row) in resolved
                .removed_handles
                .iter()
                .copied()
                .zip(&resolved.delta.removed)
            {
                if !seen.insert(id) || handles.row_for_handle(value, id)? != row {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            }
            for id in &resolved.inserted_handles {
                if handles.positions.contains_key(id) || !seen.insert(*id) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            }
            let patch = StorageResolvedScanPatch {
                removed_handles: resolved.removed_handles.clone(),
                inserted: resolved
                    .inserted_handles
                    .iter()
                    .copied()
                    .zip(resolved.delta.inserted.iter().cloned())
                    .collect(),
            };
            if frames
                .insert(
                    edge,
                    ValidatedTransitionFrame::new(
                        edge,
                        MaintainedScanCommitPatch::StorageResolved(patch),
                        resolved.delta.clone(),
                    ),
                )
                .is_some()
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        Ok(frames)
    }

    fn commit_storage_resolved_scan_patch(
        value: &mut RelationValue,
        handles: &mut Option<MaintainedLeafHandles>,
        patch: StorageResolvedScanPatch,
    ) {
        let handles = handles
            .as_mut()
            .expect("sealed storage-resolved Scan patch requires bound handles");
        for id in patch.removed_handles {
            handles
                .remove(value, id)
                .expect("validated storage-resolved Scan removal must commit");
        }
        for (id, row) in patch.inserted {
            handles
                .insert(value, id, row)
                .expect("validated storage-resolved Scan insertion must commit");
        }
    }

    fn apply_storage_resolved_deltas_in_place(
        &mut self,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let transition_program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for (relation, resolved) in deltas {
            if *relation != resolved.relation || !transition_program.contains_source(*relation) {
                return Err(RelQueryError::UnknownRelation(*relation));
            }
        }
        #[cfg(debug_assertions)]
        let recursive_oracle = self.recursive_oracle_from_frames(
            self.validate_resolved_leaf_frames(deltas, context, registry)?,
            context,
            registry,
        )?;
        let mut validated_frames = self.validate_resolved_leaf_frames(deltas, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let planned =
            self.plan_relation_deltas_execgraph(&mut validated_frames, context, registry)?;
        let output = materialize_delta_view(&planned.root_effect, self.result_type.clone())?;
        #[cfg(debug_assertions)]
        if !relation_deltas_semantically_equivalent(
            &output,
            &recursive_oracle.1,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        self.commit_graph_patch_set(planned);
        #[cfg(debug_assertions)]
        if !relation_values_semantically_equivalent(
            &self.output_value(context, registry)?,
            &recursive_oracle
                .0
                .output_value_recursive(context, registry)?,
            &self.result_type,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        {
            self.node = recursive_oracle.0.node.as_ref().map(Arc::clone);
        }
        self.transition_epoch = next_epoch;
        Ok(output)
    }

    fn validate_leaf_deltas(
        &self,
        deltas: &BTreeMap<kernel_types::SemanticId, RelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ValidatedLeafTransitionFrames, RelQueryError> {
        let mut frames = BTreeMap::new();
        let mut next_edge_ordinal = 0_u32;
        for node in self.arena.iter().map(Arc::as_ref) {
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation,
                value,
                canonical_lookup,
                ..
            } = &node.kind
            else {
                continue;
            };
            let edge = CompiledDeltaEdgeIdentity::new(next_edge_ordinal, *relation);
            next_edge_ordinal = next_edge_ordinal
                .checked_add(1)
                .ok_or(RelQueryError::TransitionEpochExhausted)?;
            let Some(delta) = deltas.get(relation) else {
                continue;
            };
            if delta.result_type != node.result_type {
                return Err(RelQueryError::TypeMismatch);
            }
            MaterializedSetSupportState::validate_rows(
                &delta.removed,
                &node.result_type,
                context,
                registry,
            )?;
            MaterializedSetSupportState::validate_rows(
                &delta.inserted,
                &node.result_type,
                context,
                registry,
            )?;
            let plan = MaterializedJoinDeltaState::plan_relation_mutation(
                value,
                delta,
                &node.result_type,
                canonical_lookup.as_ref(),
                context,
                registry,
            )?;
            if frames
                .insert(
                    edge,
                    ValidatedTransitionFrame::new(
                        edge,
                        MaintainedScanCommitPatch::Semantic(plan),
                        delta.clone(),
                    ),
                )
                .is_some()
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        Ok(frames)
    }

    fn plan_relation_deltas_execgraph(
        &mut self,
        validated_frames: &mut ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<GraphPatchSet, RelQueryError> {
        let mut scratch = std::mem::take(&mut self.execgraph_scratch);
        let result = self.plan_relation_deltas_execgraph_with_scratch(
            validated_frames,
            context,
            registry,
            &mut scratch,
        );
        if result.is_err() {
            scratch.reset();
        }
        self.execgraph_scratch = scratch;
        result
    }

    fn plan_relation_deltas_execgraph_with_scratch(
        &self,
        validated_frames: &mut ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut UnifiedTransitionScratch<MaintainedDelta>,
    ) -> Result<GraphPatchSet, RelQueryError> {
        let graph = self.differential.physical_program().execution_graph();
        let program = graph.transition_program();
        if self.arena.len() != graph.node_count() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut patches = (0..graph.node_count())
            .map(|_| None)
            .collect::<Vec<Option<GraphNodePatch>>>();
        scratch.ensure_nodes(graph.node_count());
        scratch.reset();
        let mut root_effect = MaintainedDelta::default();
        let mut next_edge_ordinal = 0_u32;

        for (node_id, state) in self.arena.iter().enumerate() {
            let FlatMaintainedRelPlanNodeKind::Scan { relation, .. } = &state.kind else {
                continue;
            };
            let edge = CompiledDeltaEdgeIdentity::new(next_edge_ordinal, *relation);
            next_edge_ordinal = next_edge_ordinal
                .checked_add(1)
                .ok_or(RelQueryError::TransitionEpochExhausted)?;
            let Some(frame) = validated_frames.remove(&edge) else {
                continue;
            };
            if frame.edge() != edge {
                scratch.reset();
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let (patch, delta) = frame.into_parts();
            if delta.result_type != state.result_type {
                scratch.reset();
                return Err(RelQueryError::TypeMismatch);
            }
            patches[node_id] = Some(GraphNodePatch::Scan(patch));
            let effect = maintained_delta_from_relation_delta(delta);
            if node_id == program.root() {
                root_effect = effect;
            } else if effect.support_len() != 0
                && let Err(error) = program.deliver_output(node_id, effect, scratch)
            {
                scratch.reset();
                return Err(error);
            }
        }

        if !validated_frames.is_empty() {
            scratch.reset();
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        while let Some((node_id, inbox)) = scratch.pop_next() {
            let state = self
                .arena
                .get(node_id)
                .map(Arc::as_ref)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let planned = match Self::plan_execgraph_node(state, inbox, context, registry) {
                Ok(planned) => planned,
                Err(error) => {
                    scratch.reset();
                    return Err(error);
                }
            };
            if patches[node_id].is_some() {
                scratch.reset();
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            patches[node_id] = planned.patch;
            if node_id == program.root() {
                root_effect = planned.effect;
            } else if planned.effect.support_len() != 0
                && let Err(error) = program.deliver_output(node_id, planned.effect, scratch)
            {
                scratch.reset();
                return Err(error);
            }
        }
        scratch.finish_success();
        Ok(GraphPatchSet {
            nodes: patches,
            root_effect,
        })
    }

    fn plan_execgraph_node(
        state: &FlatMaintainedRelPlanNode,
        mut inbox: NodeInbox<MaintainedDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        let empty = MaintainedDelta::default;
        match &state.kind {
            FlatMaintainedRelPlanNodeKind::Scan { .. } => {
                Err(RelQueryError::InconsistentIncrementalDelta)
            }
            FlatMaintainedRelPlanNodeKind::Filter {
                input: _,
                column,
                value,
                equivalence,
            } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: filter_delta_view(
                    &inbox.take_unary().unwrap_or_else(empty),
                    &state.result_type,
                    *column,
                    value,
                    *equivalence,
                    context,
                    registry,
                )?,
            }),
            FlatMaintainedRelPlanNodeKind::FilterColumns {
                input: _,
                left_column,
                right_column,
                equivalence,
            } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: filter_columns_delta_view(
                    &inbox.take_unary().unwrap_or_else(empty),
                    &state.result_type,
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?,
            }),
            FlatMaintainedRelPlanNodeKind::ProjectBag { columns, .. } => {
                Ok(PlannedGraphNodeTransition {
                    patch: None,
                    effect: project_bag_delta_view(
                        &inbox.take_unary().unwrap_or_else(empty),
                        columns,
                        &state.result_type,
                        context,
                        registry,
                    )?,
                })
            }
            FlatMaintainedRelPlanNodeKind::ProjectSet {
                columns, supports, ..
            } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let projected = project_delta_view(&input, columns)?;
                let planned = supports.plan_delta_view(&projected, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::SetSupport(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = supports.plan_delta_view(&input, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::SetSupport(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::PromoteToBag { .. } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: inbox.take_unary().unwrap_or_else(empty),
            }),
            FlatMaintainedRelPlanNodeKind::Blocker { .. }
            | FlatMaintainedRelPlanNodeKind::Join { .. }
            | FlatMaintainedRelPlanNodeKind::Group { .. }
            | FlatMaintainedRelPlanNodeKind::TopK { .. } => {
                Self::plan_execgraph_stateful_node(state, inbox, context, registry)
            }
        }
    }

    fn plan_execgraph_stateful_node(
        state: &FlatMaintainedRelPlanNode,
        mut inbox: NodeInbox<MaintainedDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        let empty = MaintainedDelta::default;
        match &state.kind {
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                let planned = state.plan_delta_views(&left, &right, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Blocker(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Join { state, .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                let planned = state.plan_delta_views(&left, &right, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Join(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = state.plan_delta_view(&input, context, registry)?;
                let patch =
                    sealed_group_v3::seal_group_patch(state, planned.patch, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Group(MaintainedGroupCommitPatch::Sealed(
                        patch,
                    ))),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::TopK { state, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = state.plan_delta_view(&input, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::TopK(planned.patch)),
                    effect: planned.effect,
                })
            }
            _ => Err(RelQueryError::InconsistentIncrementalDelta),
        }
    }

    fn commit_graph_patch_set(&mut self, mut patch_set: GraphPatchSet) {
        let arena = Arc::make_mut(&mut self.arena);
        debug_assert_eq!(arena.len(), patch_set.nodes.len());
        for (node_id, patch) in patch_set.nodes.iter_mut().enumerate() {
            let Some(patch) = patch.take() else {
                continue;
            };
            let node = Arc::make_mut(
                arena
                    .get_mut(node_id)
                    .expect("execgraph patch NodeId must exist in flat arena"),
            );
            match (&mut node.kind, patch) {
                (
                    FlatMaintainedRelPlanNodeKind::Scan {
                        value,
                        handles,
                        canonical_lookup,
                        ..
                    },
                    GraphNodePatch::Scan(patch),
                ) => match patch {
                    MaintainedScanCommitPatch::Semantic(plan) => {
                        MaterializedJoinDeltaState::commit_relation_mutation(
                            value,
                            canonical_lookup,
                            plan,
                        );
                    }
                    MaintainedScanCommitPatch::StorageResolved(plan) => {
                        Self::commit_storage_resolved_scan_patch(value, handles, plan);
                        *canonical_lookup = None;
                    }
                },
                (
                    FlatMaintainedRelPlanNodeKind::ProjectSet { supports, .. }
                    | FlatMaintainedRelPlanNodeKind::Distinct { supports, .. },
                    GraphNodePatch::SetSupport(patch),
                ) => supports.commit_support_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Blocker { state, .. },
                    GraphNodePatch::Blocker(patch),
                ) => state.commit_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Join { state, .. },
                    GraphNodePatch::Join(patch),
                ) => state.commit_join_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Group { state, .. },
                    GraphNodePatch::Group(patch),
                ) => {
                    let MaintainedGroupCommitPatch::Sealed(patch) = patch;
                    sealed_group_v3::commit_sealed_group_patch(state, patch);
                }
                (
                    FlatMaintainedRelPlanNodeKind::TopK { state, .. },
                    GraphNodePatch::TopK(patch),
                ) => state.commit_topk_patch(patch),
                _ => unreachable!("execgraph patch/flat-arena node mismatch"),
            }
        }
        debug_assert!(patch_set.nodes.iter().all(Option::is_none));
    }

    #[cfg(debug_assertions)]
    fn recursive_oracle_from_frames(
        &self,
        mut validated_frames: ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(Self, RelationDelta), RelQueryError> {
        let mut candidate = self.clone();
        let mut plan = RelationDeltaPlanContext {
            validated_frames: &mut validated_frames,
            next_edge_ordinal: 0,
            context,
            registry,
        };
        let planned = candidate.plan_relation_deltas_inner(&mut plan)?;
        if !validated_frames.is_empty() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let output =
            materialize_delta_view_uncounted(&planned.effect, candidate.result_type.clone())?;
        candidate.commit_relation_plan(planned.patch);
        Ok((candidate, output))
    }

    #[cfg(debug_assertions)]
    fn plan_relation_deltas_inner(
        &self,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        match self
            .node
            .as_deref()
            .expect("debug recursive oracle tree must exist")
        {
            MaintainedRelPlanNode::Scan { relation, .. } => {
                plan.plan_scan(*relation, &self.result_type)
            }
            MaintainedRelPlanNode::Filter {
                input,
                column,
                value,
                equivalence,
            } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                let effect = filter_delta_view(
                    &child.effect,
                    &input.result_type,
                    *column,
                    value,
                    *equivalence,
                    plan.context,
                    plan.registry,
                )?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect,
                })
            }
            MaintainedRelPlanNode::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                let effect = filter_columns_delta_view(
                    &child.effect,
                    &input.result_type,
                    *left_column,
                    *right_column,
                    *equivalence,
                    plan.context,
                    plan.registry,
                )?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect,
                })
            }
            MaintainedRelPlanNode::ProjectBag { input, columns } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                let effect = project_bag_delta_view(
                    &child.effect,
                    columns,
                    &self.result_type,
                    plan.context,
                    plan.registry,
                )?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect,
                })
            }
            MaintainedRelPlanNode::ProjectSet {
                input,
                columns,
                supports,
            } => Self::plan_project_set_transition(input, columns, supports, plan),
            MaintainedRelPlanNode::Distinct { input, supports } => {
                Self::plan_distinct_transition(input, supports, plan)
            }
            MaintainedRelPlanNode::PromoteToBag { input } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect: child.effect,
                })
            }
            MaintainedRelPlanNode::Blocker { left, right, state } => {
                Self::plan_blocker_transition(left, right, state, plan)
            }
            MaintainedRelPlanNode::Join { left, right, state } => {
                Self::plan_join_transition(left, right, state, plan)
            }
            MaintainedRelPlanNode::Group { input, state } => {
                Self::plan_group_transition(input, state, plan)
            }
            MaintainedRelPlanNode::TopK { input, state } => {
                Self::plan_top_k_transition(input, state, plan)
            }
        }
    }

    #[cfg(debug_assertions)]
    fn plan_project_set_transition(
        input: &MaterializedRelPlanState,
        columns: &[usize],
        supports: &MaterializedSetSupportState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let projected = project_delta_view(&child.effect, columns)?;
        let planned = supports.plan_delta_view(&projected, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::SetUnary {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_distinct_transition(
        input: &MaterializedRelPlanState,
        supports: &MaterializedSetSupportState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned = supports.plan_delta_view(&child.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::SetUnary {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_blocker_transition(
        left: &MaterializedRelPlanState,
        right: &MaterializedRelPlanState,
        state: &MaterializedBlockerDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let left = left.plan_relation_deltas_inner(plan)?;
        let right = right.plan_relation_deltas_inner(plan)?;
        let planned =
            state.plan_delta_views(&left.effect, &right.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Blocker {
                left: Box::new(left.patch),
                right: Box::new(right.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_join_transition(
        left: &MaterializedRelPlanState,
        right: &MaterializedRelPlanState,
        state: &MaterializedJoinDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let left = left.plan_relation_deltas_inner(plan)?;
        let right = right.plan_relation_deltas_inner(plan)?;
        let planned =
            state.plan_delta_views(&left.effect, &right.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Join {
                left: Box::new(left.patch),
                right: Box::new(right.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_group_transition(
        input: &MaterializedRelPlanState,
        state: &MaterializedGroupDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned = state.plan_delta_view(&child.effect, plan.context, plan.registry)?;
        let patch = MaintainedGroupCommitPatch::Sealed(sealed_group_v3::seal_group_patch(
            state,
            planned.patch,
            plan.context,
            plan.registry,
        )?);
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Group {
                input: Box::new(child.patch),
                patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_top_k_transition(
        input: &MaterializedRelPlanState,
        state: &MaterializedTopKDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned = state.plan_delta_view(&child.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::TopK {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn commit_relation_plan(&mut self, patch: MaintainedRelPlanPatch) {
        match (
            Arc::make_mut(
                self.node
                    .as_mut()
                    .expect("debug recursive oracle tree must exist"),
            ),
            patch,
        ) {
            (
                MaintainedRelPlanNode::Scan {
                    value,
                    handles,
                    canonical_lookup,
                    ..
                },
                MaintainedRelPlanPatch::Scan(Some(plan)),
            ) => match plan {
                MaintainedScanCommitPatch::Semantic(plan) => {
                    MaterializedJoinDeltaState::commit_relation_mutation(
                        value,
                        canonical_lookup,
                        plan,
                    );
                }
                MaintainedScanCommitPatch::StorageResolved(plan) => {
                    Self::commit_storage_resolved_scan_patch(value, handles, plan);
                    *canonical_lookup = None;
                }
            },
            (MaintainedRelPlanNode::Scan { .. }, MaintainedRelPlanPatch::Scan(None)) => {}
            (
                MaintainedRelPlanNode::Filter { input, .. }
                | MaintainedRelPlanNode::FilterColumns { input, .. }
                | MaintainedRelPlanNode::ProjectBag { input, .. }
                | MaintainedRelPlanNode::PromoteToBag { input },
                MaintainedRelPlanPatch::Unary(child),
            ) => input.commit_relation_plan(*child),
            (
                MaintainedRelPlanNode::ProjectSet {
                    input, supports, ..
                }
                | MaintainedRelPlanNode::Distinct { input, supports },
                MaintainedRelPlanPatch::SetUnary {
                    input: child,
                    patch,
                },
            ) => {
                supports.commit_support_patch(patch);
                input.commit_relation_plan(*child);
            }
            (
                MaintainedRelPlanNode::Blocker { left, right, state },
                MaintainedRelPlanPatch::Blocker {
                    left: left_patch,
                    right: right_patch,
                    patch,
                },
            ) => {
                state.commit_patch(patch);
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::Join { left, right, state },
                MaintainedRelPlanPatch::Join {
                    left: left_patch,
                    right: right_patch,
                    patch,
                },
            ) => {
                state.commit_join_patch(patch);
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::Group { input, state },
                MaintainedRelPlanPatch::Group {
                    input: child,
                    patch,
                },
            ) => {
                match patch {
                    MaintainedGroupCommitPatch::Sealed(patch) => {
                        sealed_group_v3::commit_sealed_group_patch(state, patch);
                    }
                }
                input.commit_relation_plan(*child);
            }
            (
                MaintainedRelPlanNode::TopK { input, state },
                MaintainedRelPlanPatch::TopK {
                    input: child,
                    patch,
                },
            ) => {
                state.commit_topk_patch(patch);
                input.commit_relation_plan(*child);
            }
            _ => unreachable!("maintained plan patch/node mismatch"),
        }
    }
}

type ValidatedLeafTransitionFrames = BTreeMap<
    CompiledDeltaEdgeIdentity,
    ValidatedTransitionFrame<MaintainedScanCommitPatch, RelationDelta>,
>;

#[cfg(debug_assertions)]
struct RelationDeltaPlanContext<'a> {
    validated_frames: &'a mut ValidatedLeafTransitionFrames,
    next_edge_ordinal: u32,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

#[cfg(debug_assertions)]
impl RelationDeltaPlanContext<'_> {
    fn next_edge(
        &mut self,
        relation: kernel_types::SemanticId,
    ) -> Result<CompiledDeltaEdgeIdentity, RelQueryError> {
        let edge = CompiledDeltaEdgeIdentity::new(self.next_edge_ordinal, relation);
        self.next_edge_ordinal = self
            .next_edge_ordinal
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        Ok(edge)
    }

    fn plan_scan(
        &mut self,
        relation: kernel_types::SemanticId,
        result_type: &RelType,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let edge = self.next_edge(relation)?;
        let Some(frame) = self.validated_frames.remove(&edge) else {
            return Ok(PlannedMaintainedRelPlanTransition {
                patch: MaintainedRelPlanPatch::Scan(None),
                effect: MaintainedDelta::default(),
            });
        };
        if frame.edge() != edge {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let (patch, delta) = frame.into_parts();
        if delta.result_type != *result_type {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Scan(Some(patch)),
            effect: maintained_delta_from_relation_delta(delta),
        })
    }
}

fn filter_delta_view(
    input_delta: &impl DeltaView<Row>,
    input_type: &RelType,
    column: usize,
    value: &Value,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let column_type = input_type
        .columns
        .get(column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    validate_query_equivalence(equivalence, column_type, context, registry)?;
    let input_equivalence = relation_column_equivalence(input_type, column)?;
    if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }
    if !value_shape_matches_type(value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }

    let mut output = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let Some(candidate) = row.get(column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        match registry.equivalent(context, equivalence, candidate, value) {
            Ok(true) => output.push_weighted(weight, row.clone()),
            Ok(false) => {}
            Err(cause) => error = Some(cause.into()),
        }
    });
    error.map_or(Ok(output), Err)
}

fn filter_columns_delta_view(
    input_delta: &impl DeltaView<Row>,
    input_type: &RelType,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let left_type = input_type
        .columns
        .get(left_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let right_type = input_type
        .columns
        .get(right_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    if !query_types_compatible(left_type, right_type, &context.schema) {
        return Err(RelQueryError::TypeMismatch);
    }
    validate_query_equivalence(equivalence, left_type, context, registry)?;
    validate_query_equivalence(equivalence, right_type, context, registry)?;
    let left_input_equivalence = relation_column_equivalence(input_type, left_column)?;
    let right_input_equivalence = relation_column_equivalence(input_type, right_column)?;
    if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
        || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
    {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }

    let mut output = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let Some(left) = row.get(left_column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        let Some(right) = row.get(right_column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        match registry.equivalent(context, equivalence, left, right) {
            Ok(true) => output.push_weighted(weight, row.clone()),
            Ok(false) => {}
            Err(cause) => error = Some(cause.into()),
        }
    });
    error.map_or(Ok(output), Err)
}

fn project_bag_delta_view(
    input_delta: &impl DeltaView<Row>,
    columns: &[usize],
    result_type: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let (inserted, removed) = collect_projected_signed_rows(input_delta, columns)?;
    let column_equivalences = relation_column_equivalences(result_type);
    let normalized_inserted =
        unmatched_semantic_rows(&inserted, &removed, column_equivalences, context, registry)?;
    let normalized_removed =
        unmatched_semantic_rows(&removed, &inserted, column_equivalences, context, registry)?;
    Ok(maintained_delta_from_rows(
        normalized_inserted,
        normalized_removed,
    ))
}

fn collect_projected_signed_rows(
    input_delta: &impl DeltaView<Row>,
    columns: &[usize],
) -> Result<(Vec<Row>, Vec<Row>), RelQueryError> {
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    let mut error = None;
    input_delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let magnitude = if weight < 0 {
            let Some(value) = weight.checked_neg() else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
            value
        } else {
            weight
        };
        let Ok(magnitude) = usize::try_from(magnitude) else {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        };
        let projected = match project_row(row, columns) {
            Ok(projected) => projected,
            Err(cause) => {
                error = Some(cause);
                return;
            }
        };
        let target = if weight < 0 {
            &mut removed
        } else {
            &mut inserted
        };
        if target.try_reserve(magnitude).is_err() {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        }
        target.extend(std::iter::repeat_n(projected, magnitude));
    });
    error.map_or(Ok((inserted, removed)), Err)
}

fn project_delta_view(
    input_delta: &impl DeltaView<Row>,
    columns: &[usize],
) -> Result<MaintainedDelta, RelQueryError> {
    let mut projected = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        match project_row(row, columns) {
            Ok(row) => projected.push_weighted(weight, row),
            Err(cause) => error = Some(cause),
        }
    });
    error.map_or(Ok(projected), Err)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaintainedGroupBucket {
    key: Row,
    count: kernel_aggregate::ExactCount,
    sum: kernel_aggregate::ExactF64Sum,
}

const DENSE_GROUP_MARGIN: usize = 64;
const DENSE_GROUP_MAX_SLOTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseWindowGroupCount {
    base: i64,
    counts: Vec<kernel_aggregate::ExactCount>,
}

impl DenseWindowGroupCount {
    fn try_build(groups: &[MaintainedGroupBucket]) -> Option<Self> {
        let mut keys = groups.iter().filter_map(|group| {
            let [Value::I64(key)] = group.key.as_slice() else {
                return None;
            };
            Some(*key)
        });
        let Some(first) = keys.next() else {
            return Some(Self {
                base: 0,
                counts: Vec::new(),
            });
        };
        let (mut min, mut max) = (first, first);
        for key in keys {
            min = min.min(key);
            max = max.max(key);
        }
        let margin = i64::try_from(DENSE_GROUP_MARGIN).ok()?;
        let lower = min.saturating_sub(margin);
        let upper = max.saturating_add(margin);
        let span = i128::from(upper) - i128::from(lower) + 1;
        let slots = usize::try_from(span).ok()?;
        if slots == 0 || slots > DENSE_GROUP_MAX_SLOTS {
            return None;
        }
        let mut dense = Self {
            base: lower,
            counts: vec![kernel_aggregate::ExactCount::default(); slots],
        };
        for group in groups {
            let [Value::I64(key)] = group.key.as_slice() else {
                return None;
            };
            let index = dense.index(*key)?;
            dense.counts[index] = group.count.clone();
        }
        Some(dense)
    }

    fn index(&self, key: i64) -> Option<usize> {
        let offset = i128::from(key) - i128::from(self.base);
        let index = usize::try_from(offset).ok()?;
        (index < self.counts.len()).then_some(index)
    }

    fn count(&self, key: i64) -> Option<&kernel_aggregate::ExactCount> {
        self.index(key).map(|index| &self.counts[index])
    }

    fn set(&mut self, key: i64, count: kernel_aggregate::ExactCount) -> bool {
        let Some(index) = self.index(key) else {
            return false;
        };
        self.counts[index] = count;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct I64CountGroupPatch {
    changes: Vec<(i64, kernel_aggregate::ExactCount)>,
    retain_dense: bool,
    dense_move: Option<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GenericGroupPatch {
    planned: Vec<(Row, Option<MaintainedGroupBucket>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupDeltaPatch {
    Generic(GenericGroupPatch),
    I64Count(I64CountGroupPatch),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedGroupDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    input: RelExpr,
    input_type: RelType,
    group_columns: Vec<usize>,
    group_equivalences: Vec<kernel_types::SemanticId>,
    aggregate: AggregateSpec,
    result_type: RelType,
    groups: Vec<MaintainedGroupBucket>,
    i64_lookup: Option<BTreeMap<i64, usize>>,
    semantic_lookup: Option<BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, usize>>,
    group_encoders: Option<Vec<kernel_semantics::ResolvedPrimitiveEquivalence>>,
    canonical_group_lookup: bool,
    fast_i64_count: bool,
    dense_i64_count: Option<DenseWindowGroupCount>,
}

impl MaterializedGroupDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let RelExpr::Group { input, .. } = query else {
            return Ok(None);
        };
        query.prepare(context, registry)?;
        let input_value = input.evaluate(old, context, registry)?;
        Self::build_from_input_value(query, input_value, context, registry)
    }

    fn build_from_input_value(
        query: &RelExpr,
        input_value: RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } = query
        else {
            return Ok(None);
        };
        let result_type = query.typecheck(context, registry)?;
        let input_type = input.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            input_value.rows(),
            &input_type,
            context,
            registry,
        )?;
        let i64_lookup = if group_columns.len() == 1
            && matches!(
                input_type.columns.get(group_columns[0]),
                Some(kernel_schema::TypeExpr::Scalar(
                    kernel_schema::ScalarType::I64
                ))
            )
            && matches!(
                registry
                    .resolve_primitive_equivalence(context, group_equivalences[0])?
                    .map(|resolved| resolved.bind_right(&Value::I64(0))),
                Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
            ) {
            Some(BTreeMap::new())
        } else {
            None
        };
        let group_encoders = if i64_lookup.is_none() && !group_columns.is_empty() {
            let mut encoders = Vec::with_capacity(group_equivalences.len());
            let mut supported = true;
            for equivalence in group_equivalences {
                let Some(encoder) =
                    registry.resolve_primitive_equivalence(context, *equivalence)?
                else {
                    supported = false;
                    break;
                };
                encoders.push(encoder);
            }
            supported.then_some(encoders)
        } else {
            None
        };
        let canonical_group_lookup = i64_lookup.is_none()
            && !group_columns.is_empty()
            && Self::canonical_group_lookup_supported(context, registry, group_equivalences)?;
        let semantic_lookup = canonical_group_lookup.then(BTreeMap::new);
        let fast_i64_count = i64_lookup.is_some()
            && matches!(aggregate, AggregateSpec::Count { .. })
            && matches!(
                aggregate,
                AggregateSpec::Count { result_equivalence }
                    if matches!(
                        registry
                            .resolve_primitive_equivalence(context, *result_equivalence)?
                            .map(|resolved| resolved.bind_right(&Value::I64(0))),
                        Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
                    )
            );
        let mut state = Self {
            query: query.clone(),
            semantic_context: context.clone(),
            input: input.as_ref().clone(),
            input_type,
            group_columns: group_columns.clone(),
            group_equivalences: group_equivalences.clone(),
            aggregate: aggregate.clone(),
            result_type,
            groups: Vec::new(),
            i64_lookup,
            semantic_lookup,
            group_encoders,
            canonical_group_lookup,
            fast_i64_count,
            dense_i64_count: None,
        };
        for row in input_value.into_rows() {
            state.insert_row(&row, context, registry)?;
        }
        if state.group_columns.is_empty() && state.groups.is_empty() {
            state.groups.push(Self::empty_bucket(Vec::new()));
        }
        if state.fast_i64_count {
            state.dense_i64_count = DenseWindowGroupCount::try_build(&state.groups);
        }
        Ok(Some(state))
    }

    fn canonical_group_lookup_supported(
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        group_equivalences: &[kernel_types::SemanticId],
    ) -> Result<bool, RelQueryError> {
        for equivalence in group_equivalences {
            if context
                .schema
                .structural_equivalence(*equivalence)
                .is_none()
                && registry
                    .resolve_primitive_equivalence(context, *equivalence)?
                    .is_none()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn output_value(&self) -> Result<RelationValue, RelQueryError> {
        let rows = self
            .groups
            .iter()
            .map(|group| self.output_for_bucket(Some(group)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(relation_value_from_rows(rows, &self.result_type))
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn apply_model_change(
        &mut self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let input_delta = rel_delta_optimized_inner(&self.input, old, change, context, registry)?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        self.apply_input_delta(&input_delta, context, registry)
    }

    pub fn apply_input_delta(
        &mut self,
        input_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if input_delta.result_type != self.input_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned = self.plan_delta_view(&input_delta.as_delta_view(), context, registry)?;
        let effect = materialize_delta_view(&planned.effect, self.result_type.clone())?;
        self.commit_group_patch(planned.patch, context, registry)?;
        Ok(effect)
    }

    fn plan_delta_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if self.fast_i64_count {
            let mut validation_error = None;
            input_delta.visit(|weight, row| {
                if weight == 0 || validation_error.is_some() {
                    return;
                }
                if let Err(error) = MaterializedSetSupportState::validate_row_shapes(
                    std::slice::from_ref(row),
                    &self.input_type,
                ) {
                    validation_error = Some(error);
                }
            });
            if let Some(error) = validation_error {
                return Err(error);
            }
            let planned = self.plan_i64_count_delta(input_delta)?;
            return Ok(PlannedDeltaEffect {
                patch: GroupDeltaPatch::I64Count(planned.patch),
                effect: planned.effect,
            });
        }
        self.plan_generic_delta_view(input_delta, context, registry)
    }

    fn plan_generic_delta_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let mut entries = Vec::<(i64, Row)>::with_capacity(input_delta.support_len());
        let mut visit_error = None;
        input_delta.visit(|weight, row| {
            if weight == 0 || visit_error.is_some() {
                return;
            }
            if let Err(error) = MaterializedSetSupportState::validate_rows(
                std::slice::from_ref(row),
                &self.input_type,
                context,
                registry,
            ) {
                visit_error = Some(error);
                return;
            }
            entries.push((weight, row.clone()));
        });
        if let Some(error) = visit_error {
            return Err(error);
        }

        let mut affected = Vec::<Row>::new();
        for (_, row) in &entries {
            let key = self.key_for_row(row)?;
            if !self.affected_contains_key(&affected, &key, context, registry)? {
                affected.push(key);
            }
        }
        if self.group_columns.is_empty() && affected.is_empty() {
            affected.push(Vec::new());
        }
        let before = affected
            .iter()
            .map(|key| self.output_for_key(key, context, registry))
            .collect::<Result<Vec<_>, _>>()?;
        let mut planned = Vec::with_capacity(affected.len());
        for key in &affected {
            let current = self
                .find_group(key, context, registry)?
                .map(|index| self.groups[index].clone());
            let mut next = current.unwrap_or_else(|| Self::empty_bucket(key.clone()));
            for (weight, row) in &entries {
                let row_key = self.key_for_row(row)?;
                if !self.keys_equal(key, &row_key, context, registry)? {
                    continue;
                }
                if *weight < 0 {
                    for _ in 0..weight.unsigned_abs() {
                        self.remove_from_bucket(&mut next, row)?;
                    }
                } else {
                    for _ in 0..u64::try_from(*weight)
                        .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?
                    {
                        self.add_to_bucket(&mut next, row)?;
                    }
                }
            }
            let next = if next.count.is_zero() && !self.group_columns.is_empty() {
                None
            } else {
                Some(next)
            };
            planned.push((key.clone(), next));
        }
        let after = planned
            .iter()
            .map(|(_, bucket)| self.output_for_bucket(bucket.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let equivalences = relation_column_equivalences(&self.result_type);
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        for (old_row, new_row) in before.into_iter().zip(after) {
            match (old_row, new_row) {
                (Some(old_row), Some(new_row)) => {
                    if !rows_semantically_equal(
                        &old_row,
                        &new_row,
                        equivalences,
                        context,
                        registry,
                    )? {
                        effect.push_weighted(-1, old_row);
                        effect.push_weighted(1, new_row);
                    }
                }
                (Some(old_row), None) => effect.push_weighted(-1, old_row),
                (None, Some(new_row)) => effect.push_weighted(1, new_row),
                (None, None) => {}
            }
        }
        Ok(PlannedDeltaEffect {
            patch: GroupDeltaPatch::Generic(GenericGroupPatch { planned }),
            effect,
        })
    }

    fn plan_i64_count_delta<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
    ) -> Result<PlannedDeltaEffect<I64CountGroupPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        if let Some(planned) = self.plan_small_i64_count_delta(input_delta)? {
            return Ok(planned);
        }
        let group_column = self.group_columns[0];
        let mut signed = BTreeMap::<i64, i128>::new();
        let mut error = None;
        input_delta.visit(|weight, row| {
            if error.is_some() || weight == 0 {
                return;
            }
            let Some(Value::I64(key)) = row.get(group_column) else {
                error = Some(RelQueryError::TypeMismatch);
                return;
            };
            let entry = signed.entry(*key).or_default();
            *entry = if let Some(next) = entry.checked_add(i128::from(weight)) {
                next
            } else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
        });
        if let Some(error) = error {
            return Err(error);
        }
        signed.retain(|_, weight| *weight != 0);

        let retain_dense = self
            .dense_i64_count
            .as_ref()
            .is_some_and(|dense| signed.keys().all(|key| dense.index(*key).is_some()));
        let lookup = self
            .i64_lookup
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut changes = Vec::with_capacity(signed.len());
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let dense_move = self.plan_dense_singleton_move(&signed, retain_dense)?;
        for (key, weight) in signed {
            let current = if retain_dense {
                self.dense_i64_count
                    .as_ref()
                    .and_then(|dense| dense.count(key))
                    .cloned()
                    .unwrap_or_default()
            } else {
                lookup
                    .get(&key)
                    .map(|index| self.groups[*index].count.clone())
                    .unwrap_or_default()
            };
            let before = (!current.is_zero())
                .then(|| current.finish_i64())
                .transpose()?;
            let mut next = current;
            if weight < 0 {
                for _ in 0..weight.unsigned_abs() {
                    next.remove_one()?;
                }
            } else {
                for _ in 0..u128::try_from(weight)
                    .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?
                {
                    next.add_one();
                }
            }
            let after = (!next.is_zero()).then(|| next.finish_i64()).transpose()?;
            if before != after {
                if let Some(value) = before {
                    effect.push_weighted(-1, vec![Value::I64(key), Value::I64(value)]);
                }
                if let Some(value) = after {
                    effect.push_weighted(1, vec![Value::I64(key), Value::I64(value)]);
                }
            }
            changes.push((key, next));
        }
        Ok(PlannedDeltaEffect {
            patch: I64CountGroupPatch {
                changes,
                retain_dense,
                dense_move,
            },
            effect,
        })
    }

    /// Common tiny-delta lowering for I64 Count groups.  This is deliberately
    /// only a representation optimization: unsupported shapes fall through to
    /// the general signed-map planner below, while the patch/effect contract is
    /// identical.
    #[allow(clippy::too_many_lines)]
    fn plan_small_i64_count_delta<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
    ) -> Result<Option<PlannedDeltaEffect<I64CountGroupPatch, AdaptiveDelta<Row, 4>>>, RelQueryError>
    {
        if input_delta.support_len() > 2 {
            return Ok(None);
        }
        let group_column = self.group_columns[0];
        let mut first = None::<(i64, i64)>;
        let mut second = None::<(i64, i64)>;
        let mut unsupported = false;
        let mut error = None;
        input_delta.visit(|weight, row| {
            if weight == 0 || unsupported || error.is_some() {
                return;
            }
            if weight != -1 && weight != 1 {
                unsupported = true;
                return;
            }
            let Some(Value::I64(key)) = row.get(group_column) else {
                error = Some(RelQueryError::TypeMismatch);
                return;
            };
            if first.is_none() {
                first = Some((weight, *key));
            } else if second.is_none() {
                second = Some((weight, *key));
            } else {
                unsupported = true;
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        if unsupported {
            return Ok(None);
        }

        let mut entries = [(0_i64, 0_i64); 2];
        let mut len = 0_usize;
        if let Some(entry) = first {
            entries[len] = entry;
            len += 1;
        }
        if let Some(entry) = second {
            if len == 1 && entries[0].1 == entry.1 {
                let combined = entries[0].0 + entry.0;
                if combined == 0 {
                    len = 0;
                } else {
                    // Same-key ±2 is uncommon and belongs to the general path.
                    return Ok(None);
                }
            } else {
                entries[len] = entry;
                len += 1;
            }
        }

        let retain_dense = self.dense_i64_count.as_ref().is_some_and(|dense| {
            entries[..len]
                .iter()
                .all(|(_, key)| dense.index(*key).is_some())
        });
        let lookup = self
            .i64_lookup
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut changes = Vec::with_capacity(len);
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        let mut dense_move = None;

        let mut source_is_one = false;
        let mut target_is_zero = false;
        let mut removed_key = None;
        let mut inserted_key = None;

        for &(weight, key) in &entries[..len] {
            let current = if retain_dense {
                self.dense_i64_count
                    .as_ref()
                    .and_then(|dense| dense.count(key))
                    .cloned()
                    .unwrap_or_default()
            } else {
                lookup
                    .get(&key)
                    .map(|index| self.groups[*index].count.clone())
                    .unwrap_or_default()
            };
            let before = (!current.is_zero())
                .then(|| current.finish_i64())
                .transpose()?;
            if weight == -1 {
                source_is_one = current.is_one();
                removed_key = Some(key);
            } else {
                target_is_zero = current.is_zero();
                inserted_key = Some(key);
            }
            let mut next = current;
            if weight == -1 {
                next.remove_one()?;
            } else {
                next.add_one();
            }
            let after = (!next.is_zero()).then(|| next.finish_i64()).transpose()?;
            if before != after {
                if let Some(value) = before {
                    effect.push_weighted(-1, vec![Value::I64(key), Value::I64(value)]);
                }
                if let Some(value) = after {
                    effect.push_weighted(1, vec![Value::I64(key), Value::I64(value)]);
                }
            }
            changes.push((key, next));
        }

        if retain_dense
            && len == 2
            && source_is_one
            && target_is_zero
            && removed_key != inserted_key
            && let (Some(removed), Some(inserted)) = (removed_key, inserted_key)
        {
            dense_move = Some((removed, inserted));
        }
        Ok(Some(PlannedDeltaEffect {
            patch: I64CountGroupPatch {
                changes,
                retain_dense,
                dense_move,
            },
            effect,
        }))
    }

    fn plan_dense_singleton_move(
        &self,
        signed: &BTreeMap<i64, i128>,
        retain_dense: bool,
    ) -> Result<Option<(i64, i64)>, RelQueryError> {
        if !retain_dense || signed.len() != 2 {
            return Ok(None);
        }
        let removed = signed
            .iter()
            .find_map(|(key, weight)| (*weight == -1).then_some(*key));
        let inserted = signed
            .iter()
            .find_map(|(key, weight)| (*weight == 1).then_some(*key));
        let (Some(removed), Some(inserted)) = (removed, inserted) else {
            return Ok(None);
        };
        if removed == inserted {
            return Ok(None);
        }
        let dense = self
            .dense_i64_count
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let source = dense
            .count(removed)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let target = dense
            .count(inserted)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        Ok((source.is_one() && target.is_zero()).then_some((removed, inserted)))
    }

    fn commit_group_patch(
        &mut self,
        patch: GroupDeltaPatch,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        match patch {
            GroupDeltaPatch::I64Count(patch) => self.commit_i64_count_patch(patch),
            GroupDeltaPatch::Generic(patch) => {
                self.commit_generic_group_patch(patch, context, registry)
            }
        }
    }

    fn commit_i64_count_patch(&mut self, patch: I64CountGroupPatch) -> Result<(), RelQueryError> {
        if !patch.retain_dense {
            self.dense_i64_count = None;
        }
        if let Some((removed, inserted)) = patch.dense_move {
            let dense = self
                .dense_i64_count
                .as_mut()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let removed_index = dense
                .index(removed)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let inserted_index = dense
                .index(inserted)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let moved = std::mem::take(&mut dense.counts[removed_index]);
            if !moved.is_one() || !dense.counts[inserted_index].is_zero() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            dense.counts[inserted_index] = moved;
        }
        for (key, count) in patch.changes {
            if let Some(dense) = &mut self.dense_i64_count
                && !patch
                    .dense_move
                    .is_some_and(|(removed, inserted)| key == removed || key == inserted)
                && !dense.set(key, count.clone())
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let current = self
                .i64_lookup
                .as_ref()
                .and_then(|lookup| lookup.get(&key).copied());
            match (current, count.is_zero()) {
                (Some(index), false) => self.groups[index].count = count,
                (Some(index), true) => self.remove_i64_group_at(index)?,
                (None, false) => {
                    let index = self.groups.len();
                    let mut bucket = Self::empty_bucket(vec![Value::I64(key)]);
                    bucket.count = count;
                    self.groups.push(bucket);
                    self.i64_lookup
                        .as_mut()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                        .insert(key, index);
                }
                (None, true) => {}
            }
        }
        Ok(())
    }

    fn remove_i64_group_at(&mut self, index: usize) -> Result<(), RelQueryError> {
        let [Value::I64(removed_key)] = self.groups[index].key.as_slice() else {
            return Err(RelQueryError::TypeMismatch);
        };
        self.i64_lookup
            .as_mut()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?
            .remove(removed_key);
        self.groups.swap_remove(index);
        if index < self.groups.len() {
            let [Value::I64(moved_key)] = self.groups[index].key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            self.i64_lookup
                .as_mut()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                .insert(*moved_key, index);
        }
        Ok(())
    }

    fn commit_generic_group_patch(
        &mut self,
        patch: GenericGroupPatch,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        for (key, next_bucket) in patch.planned {
            let current = self.find_group(&key, context, registry)?;
            match (current, next_bucket) {
                (Some(index), Some(bucket)) => self.groups[index] = bucket,
                (Some(index), None) => self.remove_group_at(index, registry)?,
                (None, Some(bucket)) => self.push_group_bucket(bucket, registry)?,
                (None, None) => {}
            }
        }
        if self.group_columns.is_empty() && self.groups.is_empty() {
            self.push_group_bucket(Self::empty_bucket(Vec::new()), registry)?;
        }
        Ok(())
    }

    fn empty_bucket(key: Row) -> MaintainedGroupBucket {
        MaintainedGroupBucket {
            key,
            count: kernel_aggregate::ExactCount::default(),
            sum: kernel_aggregate::ExactF64Sum::default(),
        }
    }

    fn key_for_row(&self, row: &Row) -> Result<Row, RelQueryError> {
        self.group_columns
            .iter()
            .map(|column| {
                row.get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect()
    }

    fn keys_equal(
        &self,
        left: &Row,
        right: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RelQueryError> {
        if self.semantic_lookup.is_some() {
            let left = self
                .canonical_group_key(left, registry)?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let right = self
                .canonical_group_key(right, registry)?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            return Ok(left == right);
        }
        rows_semantically_equal(left, right, &self.group_equivalences, context, registry)
    }

    fn canonical_group_key(
        &self,
        key: &Row,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Vec<kernel_semantics::CanonicalEqKey>>, RelQueryError> {
        if let Some(encoders) = &self.group_encoders {
            if encoders.len() != key.len() {
                return Err(RelQueryError::EquivalenceArityMismatch);
            }
            return encoders
                .iter()
                .zip(key)
                .map(|(encoder, value)| encoder.canonical_key(value).map_err(RelQueryError::from))
                .collect::<Result<Vec<_>, _>>()
                .map(Some);
        }
        if !self.canonical_group_lookup {
            return Ok(None);
        }
        if self.group_equivalences.len() != key.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        self.group_equivalences
            .iter()
            .zip(key)
            .map(|(equivalence, value)| {
                registry
                    .canonical_equivalence_key(&self.semantic_context, *equivalence, value)
                    .map_err(RelQueryError::from)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    fn find_group(
        &self,
        key: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<usize>, RelQueryError> {
        if let Some(lookup) = &self.i64_lookup {
            let [Value::I64(key)] = key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            return Ok(lookup.get(key).copied());
        }
        if let Some(lookup) = &self.semantic_lookup {
            let canonical = self
                .canonical_group_key(key, registry)?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            return Ok(lookup.get(&canonical).copied());
        }
        for (index, group) in self.groups.iter().enumerate() {
            if self.keys_equal(&group.key, key, context, registry)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn affected_contains_key(
        &self,
        affected: &[Row],
        key: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RelQueryError> {
        for existing in affected {
            if self.keys_equal(existing, key, context, registry)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn insert_row(
        &mut self,
        row: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let key = self.key_for_row(row)?;
        let index = if let Some(index) = self.find_group(&key, context, registry)? {
            index
        } else {
            self.push_group_bucket(Self::empty_bucket(key), registry)?;
            self.groups.len() - 1
        };
        let aggregate = self.aggregate.clone();
        Self::add_to_bucket_with(&aggregate, &mut self.groups[index], row)?;
        Ok(())
    }

    fn add_to_bucket(
        &self,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
    ) -> Result<(), RelQueryError> {
        Self::add_to_bucket_with(&self.aggregate, bucket, row)
    }

    fn add_to_bucket_with(
        aggregate: &AggregateSpec,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
    ) -> Result<(), RelQueryError> {
        bucket.count.add_one();
        if let AggregateSpec::ExactF64Sum { value_column, .. } = aggregate {
            let Value::F64Bits(bits) = row
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
            else {
                return Err(RelQueryError::TypeMismatch);
            };
            bucket.sum.add(f64::from_bits(*bits))?;
        }
        Ok(())
    }

    fn remove_from_bucket(
        &self,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
    ) -> Result<(), RelQueryError> {
        Self::remove_from_bucket_with(&self.aggregate, bucket, row)
    }

    fn remove_from_bucket_with(
        aggregate: &AggregateSpec,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
    ) -> Result<(), RelQueryError> {
        bucket.count.remove_one()?;
        if let AggregateSpec::ExactF64Sum { value_column, .. } = aggregate {
            let Value::F64Bits(bits) = row
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
            else {
                return Err(RelQueryError::TypeMismatch);
            };
            bucket.sum.remove(f64::from_bits(*bits))?;
        }
        Ok(())
    }

    fn push_group_bucket(
        &mut self,
        bucket: MaintainedGroupBucket,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let index = self.groups.len();
        if let Some(lookup) = &mut self.i64_lookup {
            let [Value::I64(key)] = bucket.key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            lookup.insert(*key, index);
        } else if self.semantic_lookup.is_some() {
            let canonical = self
                .canonical_group_key(&bucket.key, registry)?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if let Some(lookup) = &mut self.semantic_lookup {
                lookup.insert(canonical, index);
            }
        }
        self.groups.push(bucket);
        Ok(())
    }

    fn remove_group_at(
        &mut self,
        index: usize,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        if let Some(lookup) = &mut self.i64_lookup {
            let [Value::I64(removed_key)] = self.groups[index].key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            lookup.remove(removed_key);
        } else if self.semantic_lookup.is_some() {
            let canonical = self
                .canonical_group_key(&self.groups[index].key, registry)?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if let Some(lookup) = &mut self.semantic_lookup {
                lookup.remove(&canonical);
            }
        }
        self.groups.swap_remove(index);
        if index < self.groups.len() {
            if let Some(lookup) = &mut self.i64_lookup {
                let [Value::I64(moved_key)] = self.groups[index].key.as_slice() else {
                    return Err(RelQueryError::TypeMismatch);
                };
                lookup.insert(*moved_key, index);
            } else if self.semantic_lookup.is_some() {
                let canonical = self
                    .canonical_group_key(&self.groups[index].key, registry)?
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                if let Some(lookup) = &mut self.semantic_lookup {
                    lookup.insert(canonical, index);
                }
            }
        }
        Ok(())
    }

    fn output_for_key(
        &self,
        key: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Row>, RelQueryError> {
        let Some(index) = self.find_group(key, context, registry)? else {
            return Ok(None);
        };
        self.output_for_bucket(Some(&self.groups[index]))
    }

    fn output_for_bucket(
        &self,
        group: Option<&MaintainedGroupBucket>,
    ) -> Result<Option<Row>, RelQueryError> {
        let Some(group) = group else {
            return Ok(None);
        };
        let mut row = group.key.clone();
        let aggregate = match self.aggregate {
            AggregateSpec::Count { .. } => Value::I64(group.count.finish_i64()?),
            AggregateSpec::ExactF64Sum { .. } => Value::F64Bits(group.sum.finish().to_bits()),
        };
        row.push(aggregate);
        Ok(Some(row))
    }
}

pub fn relation_deltas_semantically_equivalent(
    left: &RelationDelta,
    right: &RelationDelta,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.result_type != right.result_type {
        return Ok(false);
    }
    let column_equivalences = relation_column_equivalences(&left.result_type);
    Ok(rows_as_multisets_equivalent(
        &left.inserted,
        &right.inserted,
        column_equivalences,
        context,
        registry,
    )? && rows_as_multisets_equivalent(
        &left.removed,
        &right.removed,
        column_equivalences,
        context,
        registry,
    )?)
}

pub fn rel_delta_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let prepared = query.prepare(context, registry)?;
    let old_output = prepared.evaluate(old, context, registry)?;
    let next = change.apply(old);
    let new_output = prepared.evaluate(&next, context, registry)?;
    let result_type = prepared.result_type().clone();
    let column_equivalences = match &result_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            new_output.rows(),
            old_output.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old_output.rows(),
            new_output.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

pub fn rel_delta_optimized(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    let program = RelDifferentialProgram::compile(query, context, registry)?;
    program.apply(old, change, context, registry).map(Some)
}

fn rel_delta_optimized_inner(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    match query {
        RelExpr::Scan(relation) => {
            rel_delta_scan(*relation, old, change, context, registry).map(Some)
        }
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            let Some(input_delta) =
                rel_delta_optimized_inner(input, old, change, context, registry)?
            else {
                return Ok(None);
            };
            rel_delta_filter(input_delta, *column, value, *equivalence, context, registry).map(Some)
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let Some(input_delta) =
                rel_delta_optimized_inner(input, old, change, context, registry)?
            else {
                return Ok(None);
            };
            rel_delta_filter_columns(
                input_delta,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            )
            .map(Some)
        }
        RelExpr::Project { input, columns } => {
            let Some(input_delta) =
                rel_delta_optimized_inner(input, old, change, context, registry)?
            else {
                return Ok(None);
            };
            if matches!(
                input_delta.result_type.semantics,
                kernel_schema::RelationSemantics::Set { .. }
            ) {
                return rel_delta_project_set(
                    input,
                    input_delta,
                    columns,
                    query,
                    old,
                    context,
                    registry,
                )
                .map(Some);
            }
            rel_delta_project_bag(input_delta, columns, query, context, registry).map(Some)
        }
        RelExpr::Distinct {
            input,
            column_equivalences: _,
        } => {
            let Some(input_delta) =
                rel_delta_optimized_inner(input, old, change, context, registry)?
            else {
                return Ok(None);
            };
            rel_delta_distinct(input, input_delta, query, old, context, registry).map(Some)
        }
        RelExpr::PromoteToBag(input) => {
            let Some(input_delta) =
                rel_delta_optimized_inner(input, old, change, context, registry)?
            else {
                return Ok(None);
            };
            let result_type = query.typecheck(context, registry)?;
            Ok(Some(RelationDelta {
                inserted: input_delta.inserted,
                removed: input_delta.removed,
                result_type,
            }))
        }
        RelExpr::JoinEq { .. } => rel_delta_join_optimized(query, old, change, context, registry),
        RelExpr::Difference { .. } | RelExpr::AntiJoin { .. } => {
            rel_delta_by_recompute(query, old, change, context, registry).map(Some)
        }
        RelExpr::TopKWithTies { .. } => {
            rel_delta_top_k_optimized(query, old, change, context, registry)
        }
        RelExpr::Group { .. } => rel_delta_group_optimized(query, old, change, context, registry),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedOrderedRows {
    rows: BTreeMap<IndexedRowId, Row>,
    index: kernel_semantic_index::SemanticBucketIndex<
        kernel_semantics::CanonicalOrderKey,
        IndexedRowId,
    >,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedOrderedMutationPlan {
    remove_ids: Vec<IndexedRowId>,
    inserted: Vec<(IndexedRowId, kernel_semantics::CanonicalOrderKey, Row)>,
    next_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedTopKStorage {
    I64Scalar(topk_i64::I64TopKState),
    I64Rows(BTreeMap<i64, Vec<Row>>),
    SemanticOrdered {
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        rows: Box<IndexedOrderedRows>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct I64RowsMutationPlan {
    changes: Vec<(i64, Option<Vec<Row>>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TopKDeltaPatch {
    I64Scalar(topk_i64::I64TopKPatch),
    I64Rows(I64RowsMutationPlan),
    SemanticOrdered(IndexedOrderedMutationPlan),
}

impl IndexedOrderedRows {
    fn build(
        rows: Vec<Row>,
        column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        context: &kernel_schema::SemanticContext,
    ) -> Result<Self, RelQueryError> {
        let dependency = kernel_semantic_index::SemanticModuleBinding {
            semantic_id: encoder.ordering(),
            module_digest: encoder.module_digest(),
        };
        let mut index = kernel_semantic_index::SemanticBucketIndex::new(
            kernel_semantic_index::SemanticIndexBinding::new(context, vec![dependency]),
        );
        let mut stored = BTreeMap::new();
        let mut next_id = 0_u64;
        for row in rows {
            let key =
                encoder.canonical_key(row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?)?;
            let id = IndexedRowId(next_id);
            next_id = next_id
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
            let previous_key = index.insert(id, key);
            let previous_row = stored.insert(id, row);
            debug_assert!(previous_key.is_none());
            debug_assert!(previous_row.is_none());
        }
        Ok(Self {
            rows: stored,
            index,
            next_id,
        })
    }

    fn plan_mutation_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        input_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<IndexedOrderedMutationPlan, RelQueryError> {
        let mut entries = Vec::<(i64, Row)>::with_capacity(input_delta.support_len());
        input_delta.visit(|weight, row| {
            if weight != 0 {
                entries.push((weight, row.clone()));
            }
        });
        self.plan_mutation_entries(&entries, column, encoder, input_type, context, registry)
    }

    fn plan_mutation_entries(
        &self,
        entries: &[(i64, Row)],
        column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        input_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<IndexedOrderedMutationPlan, RelQueryError> {
        let equivalences = relation_column_equivalences(input_type);
        let mut used = BTreeSet::new();
        let mut remove_ids = Vec::new();
        for (weight, removed) in entries.iter().filter(|(weight, _)| *weight < 0) {
            for _ in 0..weight.unsigned_abs() {
                let key = encoder.canonical_key(
                    removed
                        .get(column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                )?;
                let bucket = self
                    .index
                    .bucket(&key)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let mut found = None;
                for id in bucket {
                    if used.contains(id) {
                        continue;
                    }
                    let candidate = self
                        .rows
                        .get(id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    if rows_semantically_equal(candidate, removed, equivalences, context, registry)?
                    {
                        found = Some(*id);
                        break;
                    }
                }
                let Some(id) = found else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                used.insert(id);
                remove_ids.push(id);
            }
        }

        let is_set = matches!(
            input_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let mut inserted = Vec::new();
        let mut next_id = self.next_id;
        for (weight, row) in entries.iter().filter(|(weight, _)| *weight > 0) {
            let magnitude =
                u64::try_from(*weight).map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
            for _ in 0..magnitude {
                let key = encoder
                    .canonical_key(row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?)?;
                if is_set {
                    if let Some(bucket) = self.index.bucket(&key) {
                        for id in bucket {
                            if used.contains(id) {
                                continue;
                            }
                            let candidate = self
                                .rows
                                .get(id)
                                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                            if rows_semantically_equal(
                                candidate,
                                row,
                                equivalences,
                                context,
                                registry,
                            )? {
                                return Err(RelQueryError::InconsistentIncrementalDelta);
                            }
                        }
                    }
                    for (_, previous_key, previous_row) in &inserted {
                        if previous_key == &key
                            && rows_semantically_equal(
                                previous_row,
                                row,
                                equivalences,
                                context,
                                registry,
                            )?
                        {
                            return Err(RelQueryError::InconsistentIncrementalDelta);
                        }
                    }
                }
                let id = IndexedRowId(next_id);
                next_id = next_id
                    .checked_add(1)
                    .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                inserted.push((id, key, row.clone()));
            }
        }
        Ok(IndexedOrderedMutationPlan {
            remove_ids,
            inserted,
            next_id,
        })
    }

    fn commit_plan(&mut self, plan: IndexedOrderedMutationPlan) {
        for id in plan.remove_ids {
            let removed_key = self.index.remove(&id);
            let removed_row = self.rows.remove(&id);
            debug_assert!(removed_key.is_some());
            debug_assert!(removed_row.is_some());
        }
        for (id, key, row) in plan.inserted {
            let previous_key = self.index.insert(id, key);
            let previous_row = self.rows.insert(id, row);
            debug_assert!(previous_key.is_none());
            debug_assert!(previous_row.is_none());
        }
        self.next_id = plan.next_id;
    }

    fn output_rows(&self, direction: OrderDirection, k: usize) -> Vec<Row> {
        if k == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        match direction {
            OrderDirection::Ascending => {
                for (_, ids) in self.index.buckets() {
                    for id in ids {
                        out.push(self.rows[id].clone());
                    }
                    if out.len() >= k {
                        break;
                    }
                }
            }
            OrderDirection::Descending => {
                for (_, ids) in self.index.buckets().rev() {
                    for id in ids {
                        out.push(self.rows[id].clone());
                    }
                    if out.len() >= k {
                        break;
                    }
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedTopKDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    input: RelExpr,
    input_type: RelType,
    result_type: RelType,
    column: usize,
    ordering: kernel_types::SemanticId,
    direction: OrderDirection,
    k: usize,
    storage: MaintainedTopKStorage,
}

impl MaterializedTopKDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let RelExpr::TopKWithTies { input, .. } = query else {
            return Ok(None);
        };
        query.prepare(context, registry)?;
        let input_value = input.evaluate(old, context, registry)?;
        Self::build_from_input_value(query, input_value, context, registry)
    }

    fn build_from_input_value(
        query: &RelExpr,
        input_value: RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } = query
        else {
            return Ok(None);
        };
        let input_type = input.typecheck(context, registry)?;
        let result_type = query.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            input_value.rows(),
            &input_type,
            context,
            registry,
        )?;
        let use_i64 = matches!(
            registry.ordering_domain(context, *ordering)?,
            kernel_semantics::OrderingDomain::I64
        ) && matches!(
            input_type.columns.get(*column),
            Some(kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::I64
            ))
        );
        let rows = input_value.into_rows();
        let scalar_i64 = use_i64
            && input_type.columns.len() == 1
            && relation_column_equivalences(&input_type).len() == 1
            && matches!(
                registry
                    .resolve_primitive_equivalence(
                        context,
                        relation_column_equivalences(&input_type)[0],
                    )?
                    .map(|resolved| resolved.bind_right(&Value::I64(0))),
                Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
            );
        let storage = if scalar_i64 {
            let mut buckets = BTreeMap::<i64, usize>::new();
            for row in rows {
                let [Value::I64(key)] = row.as_slice() else {
                    return Err(RelQueryError::TypeMismatch);
                };
                *buckets.entry(*key).or_default() += 1;
            }
            let is_set = matches!(
                input_type.semantics,
                kernel_schema::RelationSemantics::Set { .. }
            );
            MaintainedTopKStorage::I64Scalar(
                topk_i64::I64TopKState::build(&buckets, *k, *direction, is_set)
                    .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?,
            )
        } else if use_i64 {
            let mut buckets = BTreeMap::<i64, Vec<Row>>::new();
            for row in rows {
                let Value::I64(key) = row.get(*column).ok_or(RelQueryError::ColumnOutOfBounds)?
                else {
                    return Err(RelQueryError::TypeMismatch);
                };
                buckets.entry(*key).or_default().push(row);
            }
            MaintainedTopKStorage::I64Rows(buckets)
        } else {
            let encoder = registry.resolve_primitive_ordering(context, *ordering)?;
            MaintainedTopKStorage::SemanticOrdered {
                encoder,
                rows: Box::new(IndexedOrderedRows::build(rows, *column, encoder, context)?),
            }
        };
        Ok(Some(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            input: input.as_ref().clone(),
            input_type,
            result_type,
            column: *column,
            ordering: *ordering,
            direction: *direction,
            k: *k,
            storage,
        }))
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        match &self.storage {
            MaintainedTopKStorage::I64Scalar(state) => state.total_rows(),
            MaintainedTopKStorage::I64Rows(buckets) => buckets.values().map(Vec::len).sum(),
            MaintainedTopKStorage::SemanticOrdered { rows, .. } => rows.index.len(),
        }
    }

    pub fn apply_model_change(
        &mut self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let input_delta = rel_delta_optimized_inner(&self.input, old, change, context, registry)?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        self.apply_input_delta(&input_delta, context, registry)
    }

    pub fn apply_input_delta(
        &mut self,
        input_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if input_delta.result_type != self.input_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned = self.plan_delta_view(&input_delta.as_delta_view(), context, registry)?;
        let effect = materialize_delta_view(&planned.effect, self.result_type.clone())?;
        self.commit_topk_patch(planned.patch);
        Ok(effect)
    }

    fn plan_delta_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<TopKDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let mut validation_error = None;
        input_delta.visit(|weight, row| {
            if weight == 0 || validation_error.is_some() {
                return;
            }
            if let Err(error) = MaterializedSetSupportState::validate_row_shapes(
                std::slice::from_ref(row),
                &self.input_type,
            ) {
                validation_error = Some(error);
            }
        });
        if let Some(error) = validation_error {
            return Err(error);
        }
        match &self.storage {
            MaintainedTopKStorage::I64Scalar(_) => {
                let planned = self.plan_i64_scalar_delta(input_delta)?;
                Ok(PlannedDeltaEffect {
                    patch: TopKDeltaPatch::I64Scalar(planned.patch),
                    effect: planned.effect,
                })
            }
            MaintainedTopKStorage::I64Rows(buckets) => {
                let planned = self.plan_i64_rows_delta(buckets, input_delta, context, registry)?;
                Ok(PlannedDeltaEffect {
                    patch: TopKDeltaPatch::I64Rows(planned.patch),
                    effect: planned.effect,
                })
            }
            MaintainedTopKStorage::SemanticOrdered { encoder, rows } => {
                let patch = rows.plan_mutation_view(
                    input_delta,
                    self.column,
                    *encoder,
                    &self.input_type,
                    context,
                    registry,
                )?;
                let before = rows.output_rows(self.direction, self.k);
                let after =
                    Self::semantic_output_rows_after_patch(rows, &patch, self.direction, self.k);
                let effect = adaptive_delta_between_rows(
                    &before,
                    &after,
                    &self.result_type,
                    context,
                    registry,
                )?;
                Ok(PlannedDeltaEffect {
                    patch: TopKDeltaPatch::SemanticOrdered(patch),
                    effect,
                })
            }
        }
    }

    /// Computes the selected WITH-TIES output after one already-validated
    /// semantic ordered mutation without cloning the complete ordered store.
    ///
    /// The mutation patch changes only the canonical-order buckets named by
    /// removed/inserted identities.  A merge of the immutable base bucket
    /// stream with those changed buckets is therefore sufficient to recover
    /// the exact post-state prefix.  Work is proportional to the visible
    /// prefix plus the delta, rather than to the complete relation.
    fn semantic_output_rows_after_patch(
        rows: &IndexedOrderedRows,
        patch: &IndexedOrderedMutationPlan,
        direction: OrderDirection,
        k: usize,
    ) -> Vec<Row> {
        if k == 0 {
            return Vec::new();
        }

        let removed = patch.remove_ids.iter().copied().collect::<BTreeSet<_>>();
        let mut inserted = BTreeMap::<&kernel_semantics::CanonicalOrderKey, Vec<&Row>>::new();
        for (_, key, row) in &patch.inserted {
            inserted.entry(key).or_default().push(row);
        }

        let mut output = Vec::new();
        let emit_base = |ids: &[IndexedRowId], output: &mut Vec<Row>| {
            for id in ids {
                if !removed.contains(id) {
                    output.push(rows.rows[id].clone());
                }
            }
        };
        let emit_inserted = |values: &[&Row], output: &mut Vec<Row>| {
            output.extend(values.iter().map(|row| (*row).clone()));
        };

        match direction {
            OrderDirection::Ascending => {
                let mut base = rows.index.buckets().peekable();
                let mut delta = inserted.iter().peekable();
                while output.len() < k {
                    let base_key = base.peek().map(|(key, _)| *key);
                    let delta_key = delta.peek().map(|(key, _)| **key);
                    match (base_key, delta_key) {
                        (Some(base_key), Some(delta_key)) => match base_key.cmp(delta_key) {
                            std::cmp::Ordering::Less => {
                                let (_, ids) = base.next().expect("peeked base bucket");
                                emit_base(ids, &mut output);
                            }
                            std::cmp::Ordering::Equal => {
                                let (_, ids) = base.next().expect("peeked base bucket");
                                let (_, values) = delta.next().expect("peeked delta bucket");
                                emit_base(ids, &mut output);
                                emit_inserted(values, &mut output);
                            }
                            std::cmp::Ordering::Greater => {
                                let (_, values) = delta.next().expect("peeked delta bucket");
                                emit_inserted(values, &mut output);
                            }
                        },
                        (Some(_), None) => {
                            let (_, ids) = base.next().expect("peeked base bucket");
                            emit_base(ids, &mut output);
                        }
                        (None, Some(_)) => {
                            let (_, values) = delta.next().expect("peeked delta bucket");
                            emit_inserted(values, &mut output);
                        }
                        (None, None) => break,
                    }
                }
            }
            OrderDirection::Descending => {
                let mut base = rows.index.buckets().rev().peekable();
                let mut delta = inserted.iter().rev().peekable();
                while output.len() < k {
                    let base_key = base.peek().map(|(key, _)| *key);
                    let delta_key = delta.peek().map(|(key, _)| **key);
                    match (base_key, delta_key) {
                        (Some(base_key), Some(delta_key)) => match base_key.cmp(delta_key) {
                            std::cmp::Ordering::Greater => {
                                let (_, ids) = base.next().expect("peeked base bucket");
                                emit_base(ids, &mut output);
                            }
                            std::cmp::Ordering::Equal => {
                                let (_, ids) = base.next().expect("peeked base bucket");
                                let (_, values) = delta.next().expect("peeked delta bucket");
                                emit_base(ids, &mut output);
                                emit_inserted(values, &mut output);
                            }
                            std::cmp::Ordering::Less => {
                                let (_, values) = delta.next().expect("peeked delta bucket");
                                emit_inserted(values, &mut output);
                            }
                        },
                        (Some(_), None) => {
                            let (_, ids) = base.next().expect("peeked base bucket");
                            emit_base(ids, &mut output);
                        }
                        (None, Some(_)) => {
                            let (_, values) = delta.next().expect("peeked delta bucket");
                            emit_inserted(values, &mut output);
                        }
                        (None, None) => break,
                    }
                }
            }
        }
        output
    }

    fn commit_topk_patch(&mut self, patch: TopKDeltaPatch) {
        match (patch, &mut self.storage) {
            (TopKDeltaPatch::I64Scalar(patch), MaintainedTopKStorage::I64Scalar(state)) => {
                state.commit_patch(patch);
            }
            (TopKDeltaPatch::I64Rows(plan), MaintainedTopKStorage::I64Rows(buckets)) => {
                for (key, bucket) in plan.changes {
                    if let Some(bucket) = bucket {
                        buckets.insert(key, bucket);
                    } else {
                        buckets.remove(&key);
                    }
                }
            }
            (
                TopKDeltaPatch::SemanticOrdered(plan),
                MaintainedTopKStorage::SemanticOrdered { rows, .. },
            ) => rows.commit_plan(plan),
            _ => unreachable!("TopK patch/backend mismatch"),
        }
    }

    fn plan_i64_scalar_delta<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
    ) -> Result<PlannedDeltaEffect<topk_i64::I64TopKPatch, AdaptiveDelta<Row, 4>>, RelQueryError>
    {
        let MaintainedTopKStorage::I64Scalar(state) = &self.storage else {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        };
        let mut changes = BTreeMap::<i64, i64>::new();
        let mut error = None;
        input_delta.visit(|weight, row| {
            if error.is_some() || weight == 0 {
                return;
            }
            let [Value::I64(key)] = row.as_slice() else {
                error = Some(RelQueryError::TypeMismatch);
                return;
            };
            let entry = changes.entry(*key).or_default();
            *entry = if let Some(next) = entry.checked_add(weight) {
                next
            } else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
        });
        if let Some(error) = error {
            return Err(error);
        }
        changes.retain(|_, weight| *weight != 0);
        let plan = state
            .plan_signed(&changes)
            .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
        let mut effect = AdaptiveDelta::<Row, 4>::default();
        for (key, weight) in plan.effect {
            effect.push_weighted(weight, vec![Value::I64(key)]);
        }
        Ok(PlannedDeltaEffect {
            patch: plan.patch,
            effect,
        })
    }

    fn plan_i64_rows_delta<D: DeltaView<Row>>(
        &self,
        buckets: &BTreeMap<i64, Vec<Row>>,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<I64RowsMutationPlan, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let mut affected = BTreeMap::<i64, Vec<(i64, Row)>>::new();
        let mut error = None;
        input_delta.visit(|weight, row| {
            if error.is_some() || weight == 0 {
                return;
            }
            let Some(Value::I64(key)) = row.get(self.column) else {
                error = Some(RelQueryError::TypeMismatch);
                return;
            };
            affected
                .entry(*key)
                .or_default()
                .push((weight, row.clone()));
        });
        if let Some(error) = error {
            return Err(error);
        }
        let equivalences = relation_column_equivalences(&self.input_type);
        let is_set = matches!(
            self.input_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let mut changes = Vec::with_capacity(affected.len());
        for (key, entries) in affected {
            let mut bucket = buckets.get(&key).cloned().unwrap_or_default();
            for (weight, row) in entries.iter().filter(|(weight, _)| *weight < 0) {
                for _ in 0..weight.unsigned_abs() {
                    Self::remove_semantic_row(&mut bucket, row, equivalences, context, registry)?;
                }
            }
            for (weight, row) in entries.iter().filter(|(weight, _)| *weight > 0) {
                let magnitude = u64::try_from(*weight)
                    .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
                for _ in 0..magnitude {
                    if is_set
                        && Self::contains_semantic_row(
                            &bucket,
                            row,
                            equivalences,
                            context,
                            registry,
                        )?
                    {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                    bucket.push(row.clone());
                }
            }
            changes.push((key, (!bucket.is_empty()).then_some(bucket)));
        }
        let before = self.i64_output_rows(buckets);
        let after = self.i64_output_rows_after_changes(buckets, &changes);
        let effect =
            adaptive_delta_between_rows(&before, &after, &self.result_type, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: I64RowsMutationPlan { changes },
            effect,
        })
    }

    /// Computes the post-mutation WITH-TIES prefix by merging the immutable
    /// ordered buckets with the sorted changed-bucket overlay.  No complete
    /// `BTreeMap<i64, Vec<Row>>` candidate is constructed.
    fn i64_output_rows_after_changes(
        &self,
        buckets: &BTreeMap<i64, Vec<Row>>,
        changes: &[(i64, Option<Vec<Row>>)],
    ) -> Vec<Row> {
        if self.k == 0 {
            return Vec::new();
        }
        let mut output = Vec::new();
        match self.direction {
            OrderDirection::Ascending => {
                let mut base = buckets.iter().peekable();
                let mut delta = changes.iter().peekable();
                while output.len() < self.k {
                    let base_key = base.peek().map(|(key, _)| **key);
                    let delta_key = delta.peek().map(|change| change.0);
                    match (base_key, delta_key) {
                        (Some(base_key), Some(delta_key)) => match base_key.cmp(&delta_key) {
                            std::cmp::Ordering::Less => {
                                let (_, rows) = base.next().expect("peeked base bucket");
                                output.extend(rows.iter().cloned());
                            }
                            std::cmp::Ordering::Equal => {
                                let _ = base.next().expect("peeked base bucket");
                                let (_, replacement) = delta.next().expect("peeked delta bucket");
                                if let Some(rows) = replacement {
                                    output.extend(rows.iter().cloned());
                                }
                            }
                            std::cmp::Ordering::Greater => {
                                let (_, replacement) = delta.next().expect("peeked delta bucket");
                                if let Some(rows) = replacement {
                                    output.extend(rows.iter().cloned());
                                }
                            }
                        },
                        (Some(_), None) => {
                            let (_, rows) = base.next().expect("peeked base bucket");
                            output.extend(rows.iter().cloned());
                        }
                        (None, Some(_)) => {
                            let (_, replacement) = delta.next().expect("peeked delta bucket");
                            if let Some(rows) = replacement {
                                output.extend(rows.iter().cloned());
                            }
                        }
                        (None, None) => break,
                    }
                }
            }
            OrderDirection::Descending => {
                let mut base = buckets.iter().rev().peekable();
                let mut delta = changes.iter().rev().peekable();
                while output.len() < self.k {
                    let base_key = base.peek().map(|(key, _)| **key);
                    let delta_key = delta.peek().map(|change| change.0);
                    match (base_key, delta_key) {
                        (Some(base_key), Some(delta_key)) => match base_key.cmp(&delta_key) {
                            std::cmp::Ordering::Greater => {
                                let (_, rows) = base.next().expect("peeked base bucket");
                                output.extend(rows.iter().cloned());
                            }
                            std::cmp::Ordering::Equal => {
                                let _ = base.next().expect("peeked base bucket");
                                let (_, replacement) = delta.next().expect("peeked delta bucket");
                                if let Some(rows) = replacement {
                                    output.extend(rows.iter().cloned());
                                }
                            }
                            std::cmp::Ordering::Less => {
                                let (_, replacement) = delta.next().expect("peeked delta bucket");
                                if let Some(rows) = replacement {
                                    output.extend(rows.iter().cloned());
                                }
                            }
                        },
                        (Some(_), None) => {
                            let (_, rows) = base.next().expect("peeked base bucket");
                            output.extend(rows.iter().cloned());
                        }
                        (None, Some(_)) => {
                            let (_, replacement) = delta.next().expect("peeked delta bucket");
                            if let Some(rows) = replacement {
                                output.extend(rows.iter().cloned());
                            }
                        }
                        (None, None) => break,
                    }
                }
            }
        }
        output
    }

    fn i64_scalar_output_rows(state: &topk_i64::I64TopKState) -> Vec<Row> {
        state
            .selected_counts()
            .into_iter()
            .flat_map(|(key, count)| (0..count).map(move |_| vec![Value::I64(key)]))
            .collect()
    }

    fn remove_semantic_row(
        rows: &mut Vec<Row>,
        row: &Row,
        equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let mut found = None;
        for (index, candidate) in rows.iter().enumerate() {
            if rows_semantically_equal(candidate, row, equivalences, context, registry)? {
                found = Some(index);
                break;
            }
        }
        let Some(index) = found else {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        };
        rows.remove(index);
        Ok(())
    }

    fn contains_semantic_row(
        rows: &[Row],
        row: &Row,
        equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, RelQueryError> {
        for candidate in rows {
            if rows_semantically_equal(candidate, row, equivalences, context, registry)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn output_value(&self) -> RelationValue {
        let rows = match &self.storage {
            MaintainedTopKStorage::I64Scalar(state) => Self::i64_scalar_output_rows(state),
            MaintainedTopKStorage::I64Rows(buckets) => self.i64_output_rows(buckets),
            MaintainedTopKStorage::SemanticOrdered { rows, .. } => {
                rows.output_rows(self.direction, self.k)
            }
        };
        match &self.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        }
    }

    fn i64_output_rows(&self, buckets: &BTreeMap<i64, Vec<Row>>) -> Vec<Row> {
        if self.k == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        match self.direction {
            OrderDirection::Ascending => {
                for bucket in buckets.values() {
                    out.extend(bucket.iter().cloned());
                    if out.len() >= self.k {
                        break;
                    }
                }
            }
            OrderDirection::Descending => {
                for bucket in buckets.values().rev() {
                    out.extend(bucket.iter().cloned());
                    if out.len() >= self.k {
                        break;
                    }
                }
            }
        }
        out
    }
}

fn rel_delta_group_optimized(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    let RelExpr::Group {
        input,
        group_columns,
        group_equivalences,
        aggregate,
    } = query
    else {
        return Ok(None);
    };
    let Some(input_delta) = rel_delta_optimized_inner(input, old, change, context, registry)?
    else {
        return Ok(None);
    };
    rel_delta_group_local_replay(
        input,
        &input_delta,
        query,
        old,
        GroupReplaySpec {
            group_columns,
            group_equivalences,
            aggregate,
            context,
            registry,
        },
    )
    .map(Some)
}

fn rel_delta_join_optimized(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    let RelExpr::JoinEq {
        left,
        right,
        left_column,
        right_column,
        equivalence,
    } = query
    else {
        return Ok(None);
    };
    let Some(left_delta) = rel_delta_optimized_inner(left, old, change, context, registry)? else {
        return Ok(None);
    };
    let Some(right_delta) = rel_delta_optimized_inner(right, old, change, context, registry)?
    else {
        return Ok(None);
    };
    rel_delta_join_local_replay(
        left,
        right,
        &left_delta,
        &right_delta,
        query,
        old,
        JoinReplaySpec {
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
            context,
            registry,
        },
    )
    .map(Some)
}

fn rel_delta_top_k_optimized(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    let RelExpr::TopKWithTies {
        input,
        column,
        ordering,
        direction,
        k,
    } = query
    else {
        return Ok(None);
    };
    let Some(input_delta) = rel_delta_optimized_inner(input, old, change, context, registry)?
    else {
        return Ok(None);
    };
    rel_delta_top_k_local_replay(
        input,
        &input_delta,
        query,
        old,
        TopKReplaySpec {
            column: *column,
            ordering: *ordering,
            direction: *direction,
            k: *k,
            context,
            registry,
        },
    )
    .map(Some)
}

fn rel_delta_scan(
    relation: kernel_types::SemanticId,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let prepared = RelExpr::Scan(relation).prepare(context, registry)?;
    let result_type = prepared.result_type().clone();
    let column_equivalences = relation_column_equivalences(&result_type);
    let old_rows = old
        .relations
        .get(&relation)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let next = match change {
        Change::NoChange => old,
        Change::Replace(next) => next,
        Change::Fine(fine) => fine.endpoint(),
    };
    let next_rows = next
        .relations
        .get(&relation)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            next_rows,
            old_rows,
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old_rows,
            next_rows,
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

fn rel_delta_filter(
    input_delta: RelationDelta,
    column: usize,
    value: &Value,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let column_type = input_delta
        .result_type
        .columns
        .get(column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    validate_query_equivalence(equivalence, column_type, context, registry)?;
    let input_equivalence = relation_column_equivalence(&input_delta.result_type, column)?;
    if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }
    if !value_shape_matches_type(value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }

    let filter_rows = |rows: Vec<Row>| -> Result<Vec<Row>, RelQueryError> {
        rows.into_iter()
            .filter_map(|row| {
                let Some(candidate) = row.get(column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                match registry.equivalent(context, equivalence, candidate, value) {
                    Ok(true) => Some(Ok(row)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error.into())),
                }
            })
            .collect()
    };

    Ok(RelationDelta {
        inserted: filter_rows(input_delta.inserted)?,
        removed: filter_rows(input_delta.removed)?,
        result_type: input_delta.result_type,
    })
}

fn rel_delta_filter_columns(
    input_delta: RelationDelta,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let left_type = input_delta
        .result_type
        .columns
        .get(left_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let right_type = input_delta
        .result_type
        .columns
        .get(right_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    if !query_types_compatible(left_type, right_type, &context.schema) {
        return Err(RelQueryError::TypeMismatch);
    }
    validate_query_equivalence(equivalence, left_type, context, registry)?;
    validate_query_equivalence(equivalence, right_type, context, registry)?;
    let left_input_equivalence =
        relation_column_equivalence(&input_delta.result_type, left_column)?;
    let right_input_equivalence =
        relation_column_equivalence(&input_delta.result_type, right_column)?;
    if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
        || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
    {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }

    let filter_rows = |rows: Vec<Row>| -> Result<Vec<Row>, RelQueryError> {
        rows.into_iter()
            .filter_map(|row| {
                let Some(left) = row.get(left_column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                let Some(right) = row.get(right_column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                match registry.equivalent(context, equivalence, left, right) {
                    Ok(true) => Some(Ok(row)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error.into())),
                }
            })
            .collect()
    };

    Ok(RelationDelta {
        inserted: filter_rows(input_delta.inserted)?,
        removed: filter_rows(input_delta.removed)?,
        result_type: input_delta.result_type,
    })
}

fn rel_delta_project_bag(
    input_delta: RelationDelta,
    columns: &[usize],
    query: &RelExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let inserted = project_rows(input_delta.inserted, columns)?;
    let removed = project_rows(input_delta.removed, columns)?;
    let column_equivalences = relation_column_equivalences(&result_type);
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            &inserted,
            &removed,
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            &removed,
            &inserted,
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

fn project_rows(rows: Vec<Row>, columns: &[usize]) -> Result<Vec<Row>, RelQueryError> {
    rows.into_iter()
        .map(|row| project_row(&row, columns))
        .collect()
}

fn project_row(row: &Row, columns: &[usize]) -> Result<Row, RelQueryError> {
    columns
        .iter()
        .map(|column| {
            row.get(*column)
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect()
}

fn materialize_delta_view<D: DeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    #[cfg(test)]
    RELATION_DELTA_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
    materialize_delta_view_uncounted(delta, result_type)
}

fn materialize_delta_view_uncounted<D: DeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    let mut error = None;
    delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let magnitude = if weight < 0 {
            let Some(value) = weight.checked_neg() else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
            value
        } else {
            weight
        };
        let Ok(magnitude) = usize::try_from(magnitude) else {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        };
        let target = if weight < 0 {
            &mut removed
        } else {
            &mut inserted
        };
        if target.try_reserve(magnitude).is_err() {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        }
        target.extend(std::iter::repeat_n(row.clone(), magnitude));
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(RelationDelta {
        inserted,
        removed,
        result_type,
    })
}

#[cfg(test)]
std::thread_local! {
    static RELATION_DELTA_MATERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_relation_delta_materialization_count() {
    RELATION_DELTA_MATERIALIZATIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn relation_delta_materialization_count() -> usize {
    RELATION_DELTA_MATERIALIZATIONS.with(std::cell::Cell::get)
}

fn maintained_delta_from_relation_delta(delta: RelationDelta) -> MaintainedDelta {
    maintained_delta_from_rows(delta.inserted, delta.removed)
}

fn maintained_delta_from_rows(inserted: Vec<Row>, removed: Vec<Row>) -> MaintainedDelta {
    let mut delta = MaintainedDelta::default();
    for row in removed {
        delta.push_weighted(-1, row);
    }
    for row in inserted {
        delta.push_weighted(1, row);
    }
    delta
}

fn rel_delta_project_set(
    input: &RelExpr,
    input_delta: RelationDelta,
    columns: &[usize],
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let old_input = input.evaluate(old, context, registry)?.into_rows();
    let old_rows = project_rows(old_input, columns)?;
    let inserted = project_rows(input_delta.inserted, columns)?;
    let removed = project_rows(input_delta.removed, columns)?;
    set_output_delta_from_supports(&old_rows, inserted, removed, result_type, context, registry)
}

fn rel_delta_distinct(
    input: &RelExpr,
    input_delta: RelationDelta,
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let old_rows = input.evaluate(old, context, registry)?.into_rows();
    set_output_delta_from_supports(
        &old_rows,
        input_delta.inserted,
        input_delta.removed,
        result_type,
        context,
        registry,
    )
}

fn set_output_delta_from_supports(
    old_rows: &[Row],
    inserted_rows: Vec<Row>,
    removed_rows: Vec<Row>,
    result_type: RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let mut state = MaterializedSetSupportState::build(old_rows, result_type, context, registry)?;
    state.apply_rows_delta(inserted_rows, removed_rows, context, registry)
}

#[derive(Debug)]
struct SupportDeltaPlan {
    key: CanonicalRowKey,
    representative: Row,
    removals: i64,
    insertions: i64,
}

fn canonical_row_key(
    row: &Row,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<CanonicalRowKey, RelQueryError> {
    if row.len() != column_equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    row.iter()
        .zip(column_equivalences)
        .map(|(value, equivalence)| {
            registry
                .canonical_equivalence_key(context, *equivalence, value)
                .map_err(RelQueryError::from)
        })
        .collect()
}

fn canonical_row_position_index(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<CanonicalRowPositionIndex>, RelQueryError> {
    let mut by_key = BTreeMap::<CanonicalRowKey, Vec<usize>>::new();
    let mut by_position = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Some(key) = try_canonical_row_key(row, column_equivalences, context, registry)? else {
            return Ok(None);
        };
        by_key.entry(key.clone()).or_default().push(index);
        by_position.push(key);
    }
    Ok(Some(CanonicalRowPositionIndex {
        by_key,
        by_position,
    }))
}

fn canonical_row_supports(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(Vec<(Row, i64)>, SupportLookup), RelQueryError> {
    let mut supports: Vec<(Row, i64)> = Vec::new();
    let mut lookup = SupportLookup::new();
    for row in rows {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        if let Some(index) = lookup.get(&key).copied() {
            supports[index].1 += 1;
        } else {
            let index = supports.len();
            supports.push((row.clone(), 1));
            lookup.insert(key, index);
        }
    }
    Ok((supports, lookup))
}

fn apply_relation_delta_to_value(
    old: RelationValue,
    delta: &RelationDelta,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let column_equivalences = relation_column_equivalences(&delta.result_type);
    let expected_set = matches!(
        delta.result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(old, RelationValue::Set { .. }) {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    let mut rows = old.into_rows();
    for removed in &delta.removed {
        let mut found = None;
        for (index, candidate) in rows.iter().enumerate() {
            if rows_semantically_equal(candidate, removed, column_equivalences, context, registry)?
            {
                found = Some(index);
                break;
            }
        }
        let Some(index) = found else {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        };
        rows.remove(index);
    }
    for inserted in &delta.inserted {
        if expected_set {
            for candidate in &rows {
                if rows_semantically_equal(
                    candidate,
                    inserted,
                    column_equivalences,
                    context,
                    registry,
                )? {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            }
        }
        rows.push(inserted.clone());
    }
    Ok(match &delta.result_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn relation_value_from_rows(rows: Vec<Row>, relation_type: &RelType) -> RelationValue {
    match &relation_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    }
}

#[derive(Clone, Copy)]
struct JoinReplaySpec<'a> {
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn rel_delta_join_local_replay(
    left: &RelExpr,
    right: &RelExpr,
    left_delta: &RelationDelta,
    right_delta: &RelationDelta,
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    spec: JoinReplaySpec<'_>,
) -> Result<RelationDelta, RelQueryError> {
    let old_left = left.evaluate(old, spec.context, spec.registry)?;
    let old_right = right.evaluate(old, spec.context, spec.registry)?;
    let next_left =
        apply_relation_delta_to_value(old_left.clone(), left_delta, spec.context, spec.registry)?;
    let next_right =
        apply_relation_delta_to_value(old_right.clone(), right_delta, spec.context, spec.registry)?;
    let old_output = join_relation_values(
        old_left,
        old_right,
        spec.left_column,
        spec.right_column,
        spec.equivalence,
        spec.context,
        spec.registry,
    )?;
    let next_output = join_relation_values(
        next_left,
        next_right,
        spec.left_column,
        spec.right_column,
        spec.equivalence,
        spec.context,
        spec.registry,
    )?;
    relation_delta_between_values(
        &old_output,
        &next_output,
        query.typecheck(spec.context, spec.registry)?,
        spec.context,
        spec.registry,
    )
}

fn adaptive_delta_between_rows(
    old: &[Row],
    next: &[Row],
    result_type: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
    let column_equivalences = relation_column_equivalences(result_type);
    let inserted = unmatched_semantic_rows(next, old, column_equivalences, context, registry)?;
    let removed = unmatched_semantic_rows(old, next, column_equivalences, context, registry)?;
    let mut effect = AdaptiveDelta::<Row, 4>::default();
    for row in removed {
        effect.push_weighted(-1, row);
    }
    for row in inserted {
        effect.push_weighted(1, row);
    }
    Ok(effect)
}

fn relation_delta_between_values(
    old: &RelationValue,
    next: &RelationValue,
    result_type: RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let column_equivalences = relation_column_equivalences(&result_type);
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            next.rows(),
            old.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old.rows(),
            next.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

fn join_relation_values(
    left_value: RelationValue,
    right_value: RelationValue,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let output_set_equivalences = match (&left_value, &right_value) {
        (
            RelationValue::Set {
                column_equivalences: left,
                ..
            },
            RelationValue::Set {
                column_equivalences: right,
                ..
            },
        ) => Some(left.iter().chain(right).copied().collect::<Vec<_>>()),
        _ => None,
    };
    let left_rows = left_value.into_rows();
    let right_rows = right_value.into_rows();
    let mut out = Vec::new();
    for left_row in &left_rows {
        let left_key = left_row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        for right_row in &right_rows {
            let right_key = right_row
                .get(right_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if registry.equivalent(context, equivalence, left_key, right_key)? {
                let mut joined = Vec::with_capacity(left_row.len() + right_row.len());
                joined.extend(left_row.iter().cloned());
                joined.extend(right_row.iter().cloned());
                out.push(joined);
            }
        }
    }
    Ok(match output_set_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows: out,
            column_equivalences,
        },
        None => RelationValue::Bag(out),
    })
}

#[derive(Clone, Copy)]
struct TopKReplaySpec<'a> {
    column: usize,
    ordering: kernel_types::SemanticId,
    direction: OrderDirection,
    k: usize,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn rel_delta_top_k_local_replay(
    input: &RelExpr,
    input_delta: &RelationDelta,
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    spec: TopKReplaySpec<'_>,
) -> Result<RelationDelta, RelQueryError> {
    let old_input = input.evaluate(old, spec.context, spec.registry)?;
    let next_input =
        apply_relation_delta_to_value(old_input.clone(), input_delta, spec.context, spec.registry)?;
    let old_output = top_k_relation_value(
        old_input,
        spec.column,
        spec.ordering,
        spec.direction,
        spec.k,
        spec.context,
        spec.registry,
    )?;
    let next_output = top_k_relation_value(
        next_input,
        spec.column,
        spec.ordering,
        spec.direction,
        spec.k,
        spec.context,
        spec.registry,
    )?;
    relation_delta_between_values(
        &old_output,
        &next_output,
        query.typecheck(spec.context, spec.registry)?,
        spec.context,
        spec.registry,
    )
}

#[derive(Clone, Copy)]
struct GroupReplaySpec<'a> {
    group_columns: &'a [usize],
    group_equivalences: &'a [kernel_types::SemanticId],
    aggregate: &'a AggregateSpec,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn rel_delta_group_local_replay(
    input: &RelExpr,
    input_delta: &RelationDelta,
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    spec: GroupReplaySpec<'_>,
) -> Result<RelationDelta, RelQueryError> {
    let old_input = input.evaluate(old, spec.context, spec.registry)?;
    let next_input =
        apply_relation_delta_to_value(old_input.clone(), input_delta, spec.context, spec.registry)?;
    let old_output = group_relation_value(
        old_input,
        spec.group_columns,
        spec.group_equivalences,
        spec.aggregate,
        spec.context,
        spec.registry,
    )?;
    let next_output = group_relation_value(
        next_input,
        spec.group_columns,
        spec.group_equivalences,
        spec.aggregate,
        spec.context,
        spec.registry,
    )?;
    relation_delta_between_values(
        &old_output,
        &next_output,
        query.typecheck(spec.context, spec.registry)?,
        spec.context,
        spec.registry,
    )
}

fn top_k_relation_value(
    input_value: RelationValue,
    column: usize,
    ordering: kernel_types::SemanticId,
    direction: OrderDirection,
    k: usize,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let set_equivalences = match &input_value {
        RelationValue::Set {
            column_equivalences,
            ..
        } => Some(column_equivalences.clone()),
        RelationValue::Bag(_) => None,
    };
    let mut rows = input_value.into_rows();
    if k == 0 || rows.is_empty() {
        rows.clear();
    } else {
        fallible_insertion_sort_rows(&mut rows, column, ordering, direction, context, registry)?;
        if k < rows.len() {
            let threshold = rows[k - 1]
                .get(column)
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let mut keep = k;
            while keep < rows.len() {
                let candidate = rows[keep]
                    .get(column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                if compare_with_direction(
                    context, registry, ordering, candidate, &threshold, direction,
                )? != std::cmp::Ordering::Equal
                {
                    break;
                }
                keep += 1;
            }
            rows.truncate(keep);
        }
    }
    Ok(match set_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows,
            column_equivalences,
        },
        None => RelationValue::Bag(rows),
    })
}

#[must_use]
pub fn rel_derivative_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Change<RelQueryResult> {
    let next = change.apply(old);
    let new_output = query.evaluate(&next, context, registry);
    if rel_impact_by_recompute(query, old, change, context, registry) == Impact::Unaffected {
        Change::NoChange
    } else {
        Change::Replace(new_output)
    }
}

#[must_use]
pub fn rel_impact_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Impact {
    if matches!(change, Change::NoChange) {
        return Impact::Unaffected;
    }
    let Ok(prepared) = query.prepare(context, registry) else {
        return Impact::Unknown;
    };
    let old_output = prepared.evaluate(old, context, registry);
    let next = change.apply(old);
    let new_output = prepared.evaluate(&next, context, registry);
    match (&old_output, &new_output) {
        (Ok(left), Ok(right)) => match relation_values_semantically_equivalent(
            left,
            right,
            prepared.result_type(),
            context,
            registry,
        ) {
            Ok(true) => Impact::Unaffected,
            Ok(false) => Impact::Changed,
            Err(_) => Impact::Unknown,
        },
        (Err(left), Err(right)) if left == right => Impact::Unaffected,
        (Err(_) | Ok(_), Err(_)) | (Err(_), Ok(_)) => Impact::Changed,
    }
}

fn relation_values_semantically_equivalent(
    left: &RelationValue,
    right: &RelationValue,
    relation_type: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    let expected_set = matches!(
        relation_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(left, RelationValue::Set { .. })
        || expected_set != matches!(right, RelationValue::Set { .. })
    {
        return Ok(false);
    }
    let column_equivalences = match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    rows_as_multisets_equivalent(
        left.rows(),
        right.rows(),
        column_equivalences,
        context,
        registry,
    )
}

fn rows_as_multisets_equivalent(
    left: &[Row],
    right: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    if let (Some(left_counts), Some(right_counts)) = (
        canonical_row_multiset_counts(left, column_equivalences, context, registry)?,
        canonical_row_multiset_counts(right, column_equivalences, context, registry)?,
    ) {
        return Ok(left_counts == right_counts);
    }
    rows_as_multisets_equivalent_by_matching(left, right, column_equivalences, context, registry)
}

fn rows_as_multisets_equivalent_by_matching(
    left: &[Row],
    right: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    let mut matched = vec![false; right.len()];
    for left_row in left {
        if left_row.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut found = None;
        for (index, right_row) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if rows_semantically_equal(left_row, right_row, column_equivalences, context, registry)?
            {
                found = Some(index);
                break;
            }
        }
        let Some(index) = found else {
            return Ok(false);
        };
        matched[index] = true;
    }
    Ok(true)
}

fn canonical_row_multiset_counts(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<BTreeMap<CanonicalRowKey, usize>>, RelQueryError> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let Some(key) = try_canonical_row_key(row, column_equivalences, context, registry)? else {
            return Ok(None);
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    Ok(Some(counts))
}

fn try_canonical_row_key(
    row: &Row,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<CanonicalRowKey>, RelQueryError> {
    if row.len() != column_equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    let mut key = Vec::with_capacity(row.len());
    for (value, equivalence) in row.iter().zip(column_equivalences) {
        match registry.canonical_equivalence_key(context, *equivalence, value) {
            Ok(part) => key.push(part),
            Err(kernel_semantics::SemanticError::WrongModuleKind(_)) => return Ok(None),
            Err(error) => return Err(RelQueryError::Semantic(error)),
        }
    }
    Ok(Some(key))
}

fn rows_semantically_equal(
    left: &Row,
    right: &Row,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.len() != column_equivalences.len() || right.len() != column_equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    for ((left_value, right_value), equivalence) in left.iter().zip(right).zip(column_equivalences)
    {
        if !registry.equivalent(context, *equivalence, left_value, right_value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn unmatched_semantic_rows(
    source: &[Row],
    target: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    if let Some(mut target_counts) =
        canonical_row_multiset_counts(target, column_equivalences, context, registry)?
    {
        let mut unmatched = Vec::new();
        for source_row in source {
            let Some(key) =
                try_canonical_row_key(source_row, column_equivalences, context, registry)?
            else {
                return unmatched_semantic_rows_by_matching(
                    source,
                    target,
                    column_equivalences,
                    context,
                    registry,
                );
            };
            let supported = target_counts.get_mut(&key).is_some_and(|count| {
                if *count == 0 {
                    false
                } else {
                    *count -= 1;
                    true
                }
            });
            if !supported {
                unmatched.push(source_row.clone());
            }
        }
        return Ok(unmatched);
    }
    unmatched_semantic_rows_by_matching(source, target, column_equivalences, context, registry)
}

fn unmatched_semantic_rows_by_matching(
    source: &[Row],
    target: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    let mut matched = vec![false; target.len()];
    let mut unmatched = Vec::new();
    for source_row in source {
        if source_row.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut found = None;
        for (index, target_row) in target.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if rows_semantically_equal(
                source_row,
                target_row,
                column_equivalences,
                context,
                registry,
            )? {
                found = Some(index);
                break;
            }
        }
        if let Some(index) = found {
            matched[index] = true;
        } else {
            unmatched.push(source_row.clone());
        }
    }
    Ok(unmatched)
}

#[must_use]
pub fn check_derivative_law(
    query: &ExactQuery,
    old: &Value,
    input_change: &Change<Value>,
    output_change: &Change<QueryResult>,
) -> bool {
    let old_output = query.evaluate(old);
    let via_delta = output_change.apply(&old_output);
    let next_input = input_change.apply(old);
    let from_scratch = query.evaluate(&next_input);
    via_delta == from_scratch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(values: &[i64]) -> Value {
        Value::Seq(values.iter().copied().map(Value::I64).collect())
    }

    #[test]
    fn universal_derivative_satisfies_from_scratch_law() {
        let query = ExactQuery::new(Expr::SeqSumI64(Box::new(Expr::Input)));
        let old = seq(&[1, 2, 3]);
        let change = Change::Replace(seq(&[1, 2, 3, 4]));
        let output_change = derivative_by_recompute(&query, &old, &change);
        assert!(check_derivative_law(&query, &old, &change, &output_change));
    }

    #[test]
    fn impact_can_certify_unchanged_result_without_host_callback() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        let old = seq(&[1, 2]);
        let change = Change::Replace(seq(&[7, 8]));
        assert_eq!(
            impact_by_recompute(&query, &old, &change),
            Impact::Unaffected
        );
    }

    #[test]
    fn logical_errors_are_deterministic_values_of_query_semantics() {
        let query = ExactQuery::new(Expr::SeqSumI64(Box::new(Expr::Input)));
        let bad = Value::Seq(vec![Value::Text("not an integer".into())]);
        assert_eq!(query.evaluate(&bad), Err(QueryError::TypeMismatch));
    }

    #[test]
    fn optimized_sequence_length_delta_matches_from_scratch() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        let old = seq(&[1, 2, 3, 4]);
        let splice = SeqSplice {
            start: 1,
            delete_count: 1,
            insert: vec![Value::I64(8), Value::I64(9), Value::I64(10)],
        };
        let fine = derivative_seq_splice(&query, &old, &splice)
            .unwrap()
            .expect("supported fine rule");
        let Value::Seq(old_values) = &old else {
            unreachable!();
        };
        let next = Value::Seq(splice.apply(old_values).unwrap());
        let universal = derivative_by_recompute(&query, &old, &Change::Replace(next.clone()));
        assert_eq!(fine, universal);
        assert!(check_derivative_law(
            &query,
            &old,
            &Change::Replace(next),
            &fine
        ));
    }

    #[test]
    fn integer_overflow_is_not_plan_dependent_wraparound() {
        let query = ExactQuery::new(Expr::AddI64(
            Box::new(Expr::Const(Value::I64(i64::MAX))),
            Box::new(Expr::Const(Value::I64(1))),
        ));
        assert_eq!(
            query.evaluate(&Value::Unit),
            Err(QueryError::ArithmeticOverflow)
        );
    }
}

pub type Row = Vec<Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationValue {
    Bag(Vec<Row>),
    Set {
        rows: Vec<Row>,
        column_equivalences: Vec<kernel_types::SemanticId>,
    },
}

impl RelationValue {
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }

    fn rows_mut(&mut self) -> &mut Vec<Row> {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }

    #[must_use]
    pub fn into_rows(self) -> Vec<Row> {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelQueryError {
    UnknownRelation(kernel_types::SemanticId),
    ColumnOutOfBounds,
    EquivalenceArityMismatch,
    TypeMismatch,
    EquivalenceNotCongruentWithInputEquality,
    OrderingNotCongruentWithEquality,
    InconsistentIncrementalDelta,
    SemanticRevisionMismatch,
    RevisionBindingMismatch,
    InvalidRevisionTransition,
    RevisionBoundMutationRequiresPreparedTransition,
    StalePreparedTransition,
    TransitionEpochExhausted,
    DerivedIdentityExhausted,
    CanonicalObservationUnavailable,
    RecursiveAtomOutsideCarrier,
    NonFiniteRecursiveMultiplicity,
    PositiveBagFixpoint(kernel_fixpoint::PositiveBagFixpointError),
    Semantic(kernel_semantics::SemanticError),
    Aggregate(kernel_aggregate::AggregateError),
}

impl From<kernel_aggregate::AggregateError> for RelQueryError {
    fn from(value: kernel_aggregate::AggregateError) -> Self {
        Self::Aggregate(value)
    }
}

impl From<kernel_fixpoint::PositiveBagFixpointError> for RelQueryError {
    fn from(value: kernel_fixpoint::PositiveBagFixpointError) -> Self {
        Self::PositiveBagFixpoint(value)
    }
}

impl From<kernel_semantics::SemanticError> for RelQueryError {
    fn from(value: kernel_semantics::SemanticError) -> Self {
        Self::Semantic(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelType {
    pub columns: Vec<kernel_schema::TypeExpr>,
    pub semantics: kernel_schema::RelationSemantics,
}

/// One finite carrier atom of a grounded positive recursive relational query.
/// `seed_multiplicity` is the exact non-recursive Bag contribution at the
/// fixpoint base. Runtime row handles are deliberately absent: this is a
/// semantic/query object, not physical identity authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveRecursiveRowAtom {
    pub row: Row,
    pub seed_multiplicity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveRecursiveRowRule {
    pub body: Vec<usize>,
    pub head: usize,
    pub coefficient: u64,
}

/// Query-level positive recursion leaf after APNF/SAMF grounding.
///
/// The finite carrier is explicit. Rule bodies may repeat an atom because Bag
/// proof-tree multiplicity distinguishes duplicate recursive occurrences.
/// Evaluation returns compact `N∞` weights and never expands a large or
/// infinite multiplicity into repeated rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixpointCall {
    pub result_type: RelType,
    pub atoms: Vec<PositiveRecursiveRowAtom>,
    pub rules: Vec<PositiveRecursiveRowRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRecursiveBag {
    entries: Vec<(Row, kernel_fixpoint::NaturalInfinity)>,
}

impl CompactRecursiveBag {
    #[must_use]
    pub fn entries(&self) -> &[(Row, kernel_fixpoint::NaturalInfinity)] {
        &self.entries
    }

    #[must_use]
    pub fn has_infinite_multiplicity(&self) -> bool {
        self.entries.iter().any(|(_, weight)| weight.is_infinite())
    }

    pub fn require_finite(
        &self,
    ) -> Result<&[(Row, kernel_fixpoint::NaturalInfinity)], RelQueryError> {
        if self.has_infinite_multiplicity() {
            return Err(RelQueryError::NonFiniteRecursiveMultiplicity);
        }
        Ok(&self.entries)
    }
}

impl FixpointCall {
    pub fn typecheck(
        &self,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        if !matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Bag { .. }
        ) {
            return Err(RelQueryError::TypeMismatch);
        }
        for atom in &self.atoms {
            if atom.row.len() != self.result_type.columns.len()
                || atom
                    .row
                    .iter()
                    .zip(&self.result_type.columns)
                    .any(|(value, ty)| !value_shape_matches_type(value, ty))
            {
                return Err(RelQueryError::TypeMismatch);
            }
        }
        for rule in &self.rules {
            if rule.head >= self.atoms.len()
                || rule.body.iter().any(|&atom| atom >= self.atoms.len())
            {
                return Err(RelQueryError::RecursiveAtomOutsideCarrier);
            }
        }
        Ok(self.result_type.clone())
    }

    pub fn evaluate_compact(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<CompactRecursiveBag, RelQueryError> {
        self.typecheck(context, registry)?;
        let seed_multiplicity = self
            .atoms
            .iter()
            .map(|atom| atom.seed_multiplicity)
            .collect::<Vec<_>>();
        let rules = self
            .rules
            .iter()
            .map(|rule| {
                kernel_fixpoint::PositiveBagRule::new(
                    rule.body
                        .iter()
                        .copied()
                        .map(kernel_fixpoint::GroundedAtomId::new),
                    kernel_fixpoint::GroundedAtomId::new(rule.head),
                    rule.coefficient,
                )
            })
            .collect();
        let program =
            kernel_fixpoint::PositiveBagProgram::new(self.atoms.len(), seed_multiplicity, rules)?;
        let certificate = kernel_fixpoint::solve_positive_bag(&program)?;
        kernel_fixpoint::check_positive_bag(&program, &certificate)?;
        let entries = self
            .atoms
            .iter()
            .enumerate()
            .filter_map(|(index, atom)| {
                let weight = certificate
                    .multiplicity(kernel_fixpoint::GroundedAtomId::new(index))?
                    .clone();
                (!matches!(weight, kernel_fixpoint::NaturalInfinity::Finite(ref n) if n.is_zero()))
                    .then(|| (atom.row.clone(), weight))
            })
            .collect();
        Ok(CompactRecursiveBag { entries })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelExpr {
    expr: RelExpr,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
}

impl PreparedRelExpr {
    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    pub fn evaluate(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let eval = RelEvalContext {
            model,
            semantic: context,
            registry,
        };
        self.expr.evaluate_unchecked(&eval)
    }

    pub fn rebind_preserving_semantics(
        &self,
        target: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if !registry.context_conservatively_extends(&self.semantic_context, target)? {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        self.expr.prepare(target, registry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateSpec {
    Count {
        result_equivalence: kernel_types::SemanticId,
    },
    ExactF64Sum {
        value_column: usize,
        result_equivalence: kernel_types::SemanticId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelExpr {
    Scan(kernel_types::SemanticId),
    FilterEqConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterEqColumns {
        input: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Project {
        input: Box<Self>,
        columns: Vec<usize>,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Distinct {
        input: Box<Self>,
        column_equivalences: Vec<kernel_types::SemanticId>,
    },
    Group {
        input: Box<Self>,
        group_columns: Vec<usize>,
        group_equivalences: Vec<kernel_types::SemanticId>,
        aggregate: AggregateSpec,
    },
    TopKWithTies {
        input: Box<Self>,
        column: usize,
        ordering: kernel_types::SemanticId,
        direction: OrderDirection,
        k: usize,
    },
    PromoteToBag(Box<Self>),
}

/// Exact differential class of a relational operator.
///
/// This is a semantic maintenance classification, not a physical-state
/// prescription. Physical implementations may lower the same class to SAMF
/// fibers/annotations, specialized native state, or a recomputation oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelDifferentialClass {
    Source,
    Linear,
    ZeroCrossing,
    BilinearPullback,
    Annotation,
    OrderedBoundary,
    BlockerZeroCrossing,
}

/// Reconstructible state capability required by an exact differential node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelDifferentialStateRequirement {
    SetSupport,
    JoinFibers,
    GroupAnnotations,
    OrderedCut,
    BlockerMass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RelDifferentialNode {
    Scan {
        relation: kernel_types::SemanticId,
    },
    FilterEqConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterEqColumns {
        input: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Project {
        input: Box<Self>,
        input_expr: RelExpr,
        columns: Vec<usize>,
        result_expr: RelExpr,
        set_semantics: bool,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_expr: RelExpr,
        right_expr: RelExpr,
        result_expr: RelExpr,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
        result_expr: RelExpr,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        result_expr: RelExpr,
    },
    Distinct {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
    },
    Group {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
        group_columns: Vec<usize>,
        group_equivalences: Vec<kernel_types::SemanticId>,
        aggregate: AggregateSpec,
    },
    TopKWithTies {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
        column: usize,
        ordering: kernel_types::SemanticId,
        direction: OrderDirection,
        k: usize,
    },
    PromoteToBag {
        input: Box<Self>,
        result_type: RelType,
    },
}

impl RelDifferentialNode {
    #[allow(clippy::too_many_lines)]
    fn compile(
        expr: &RelExpr,
        node: NodeId,
        graph: &PreparedRelGraph,
    ) -> Result<Self, RelQueryError> {
        match expr {
            RelExpr::Scan(relation) => Ok(Self::Scan {
                relation: *relation,
            }),
            RelExpr::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Ok(Self::FilterEqConst {
                input: Box::new(Self::compile(
                    input,
                    Self::unary_child(node, graph)?,
                    graph,
                )?),
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            }),
            RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Ok(Self::FilterEqColumns {
                input: Box::new(Self::compile(
                    input,
                    Self::unary_child(node, graph)?,
                    graph,
                )?),
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            }),
            RelExpr::Project { input, columns } => {
                let input_id = Self::unary_child(node, graph)?;
                let input_type = graph
                    .result_type(input_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok(Self::Project {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    columns: columns.clone(),
                    result_expr: expr.clone(),
                    set_semantics: matches!(
                        input_type.semantics,
                        kernel_schema::RelationSemantics::Set { .. }
                    ),
                })
            }
            RelExpr::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => {
                let (left_id, right_id) = Self::binary_children(node, graph)?;
                Ok(Self::JoinEq {
                    left: Box::new(Self::compile(left, left_id, graph)?),
                    right: Box::new(Self::compile(right, right_id, graph)?),
                    left_expr: left.as_ref().clone(),
                    right_expr: right.as_ref().clone(),
                    result_expr: expr.clone(),
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                })
            }
            RelExpr::Difference { left, right } => {
                Self::compile_blocker(left, right, expr, node, graph, false)
            }
            RelExpr::AntiJoin { left, right, .. } => {
                Self::compile_blocker(left, right, expr, node, graph, true)
            }
            RelExpr::Distinct { input, .. } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::Distinct {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                })
            }
            RelExpr::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::Group {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                    group_columns: group_columns.clone(),
                    group_equivalences: group_equivalences.clone(),
                    aggregate: aggregate.clone(),
                })
            }
            RelExpr::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::TopKWithTies {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                    column: *column,
                    ordering: *ordering,
                    direction: *direction,
                    k: *k,
                })
            }
            RelExpr::PromoteToBag(input) => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::PromoteToBag {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    result_type: graph
                        .result_type(node)
                        .cloned()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                })
            }
        }
    }

    fn unary_child(node: NodeId, graph: &PreparedRelGraph) -> Result<NodeId, RelQueryError> {
        graph
            .unary_input(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn binary_children(
        node: NodeId,
        graph: &PreparedRelGraph,
    ) -> Result<(NodeId, NodeId), RelQueryError> {
        graph
            .binary_inputs(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn compile_blocker(
        left: &RelExpr,
        right: &RelExpr,
        result_expr: &RelExpr,
        node: NodeId,
        graph: &PreparedRelGraph,
        anti_join: bool,
    ) -> Result<Self, RelQueryError> {
        let (left_id, right_id) = Self::binary_children(node, graph)?;
        let left = Box::new(Self::compile(left, left_id, graph)?);
        let right = Box::new(Self::compile(right, right_id, graph)?);
        Ok(if anti_join {
            Self::AntiJoin {
                left,
                right,
                result_expr: result_expr.clone(),
            }
        } else {
            Self::Difference {
                left,
                right,
                result_expr: result_expr.clone(),
            }
        })
    }

    const fn class(&self) -> RelDifferentialClass {
        match self {
            Self::Scan { .. } => RelDifferentialClass::Source,
            Self::FilterEqConst { .. }
            | Self::FilterEqColumns { .. }
            | Self::PromoteToBag { .. }
            | Self::Project {
                set_semantics: false,
                ..
            } => RelDifferentialClass::Linear,
            Self::Project {
                set_semantics: true,
                ..
            }
            | Self::Distinct { .. } => RelDifferentialClass::ZeroCrossing,
            Self::JoinEq { .. } => RelDifferentialClass::BilinearPullback,
            Self::Difference { .. } | Self::AntiJoin { .. } => {
                RelDifferentialClass::BlockerZeroCrossing
            }
            Self::Group { .. } => RelDifferentialClass::Annotation,
            Self::TopKWithTies { .. } => RelDifferentialClass::OrderedBoundary,
        }
    }

    fn collect_state_requirements(&self, out: &mut BTreeSet<RelDifferentialStateRequirement>) {
        match self {
            Self::Project {
                input,
                set_semantics,
                ..
            } => {
                input.collect_state_requirements(out);
                if *set_semantics {
                    out.insert(RelDifferentialStateRequirement::SetSupport);
                }
            }
            Self::Distinct { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::SetSupport);
            }
            Self::JoinEq { left, right, .. } => {
                left.collect_state_requirements(out);
                right.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::JoinFibers);
            }
            Self::Difference { left, right, .. } | Self::AntiJoin { left, right, .. } => {
                left.collect_state_requirements(out);
                right.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::BlockerMass);
            }
            Self::Group { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::GroupAnnotations);
            }
            Self::TopKWithTies { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::OrderedCut);
            }
            Self::FilterEqConst { input, .. }
            | Self::FilterEqColumns { input, .. }
            | Self::PromoteToBag { input, .. } => input.collect_state_requirements(out),
            Self::Scan { .. } => {}
        }
    }

    fn apply(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        match self {
            Self::Scan { relation } => rel_delta_scan(*relation, old, change, context, registry),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => rel_delta_filter(
                input.apply(old, change, context, registry)?,
                *column,
                value,
                *equivalence,
                context,
                registry,
            ),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => rel_delta_filter_columns(
                input.apply(old, change, context, registry)?,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Project { .. } => self.apply_project(old, change, context, registry),
            Self::JoinEq { .. } => self.apply_join(old, change, context, registry),
            Self::Difference { result_expr, .. } | Self::AntiJoin { result_expr, .. } => {
                rel_delta_by_recompute(result_expr, old, change, context, registry)
            }
            Self::Distinct {
                input,
                input_expr,
                result_expr,
            } => rel_delta_distinct(
                input_expr,
                input.apply(old, change, context, registry)?,
                result_expr,
                old,
                context,
                registry,
            ),
            Self::Group { .. } => self.apply_group(old, change, context, registry),
            Self::TopKWithTies { .. } => self.apply_top_k(old, change, context, registry),
            Self::PromoteToBag { input, result_type } => {
                let input_delta = input.apply(old, change, context, registry)?;
                Ok(RelationDelta {
                    inserted: input_delta.inserted,
                    removed: input_delta.removed,
                    result_type: result_type.clone(),
                })
            }
        }
    }

    fn apply_project(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::Project {
            input,
            input_expr,
            columns,
            result_expr,
            set_semantics,
        } = self
        else {
            unreachable!("apply_project is only called for project nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        if *set_semantics {
            rel_delta_project_set(
                input_expr,
                input_delta,
                columns,
                result_expr,
                old,
                context,
                registry,
            )
        } else {
            rel_delta_project_bag(input_delta, columns, result_expr, context, registry)
        }
    }

    fn apply_join(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::JoinEq {
            left,
            right,
            left_expr,
            right_expr,
            result_expr,
            left_column,
            right_column,
            equivalence,
        } = self
        else {
            unreachable!("apply_join is only called for join nodes");
        };
        let left_delta = left.apply(old, change, context, registry)?;
        let right_delta = right.apply(old, change, context, registry)?;
        rel_delta_join_local_replay(
            left_expr,
            right_expr,
            &left_delta,
            &right_delta,
            result_expr,
            old,
            JoinReplaySpec {
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
                context,
                registry,
            },
        )
    }

    fn apply_group(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::Group {
            input,
            input_expr,
            result_expr,
            group_columns,
            group_equivalences,
            aggregate,
        } = self
        else {
            unreachable!("apply_group is only called for group nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        rel_delta_group_local_replay(
            input_expr,
            &input_delta,
            result_expr,
            old,
            GroupReplaySpec {
                group_columns,
                group_equivalences,
                aggregate,
                context,
                registry,
            },
        )
    }

    fn apply_top_k(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::TopKWithTies {
            input,
            input_expr,
            result_expr,
            column,
            ordering,
            direction,
            k,
        } = self
        else {
            unreachable!("apply_top_k is only called for TopK nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        rel_delta_top_k_local_replay(
            input_expr,
            &input_delta,
            result_expr,
            old,
            TopKReplaySpec {
                column: *column,
                ordering: *ordering,
                direction: *direction,
                k: *k,
                context,
                registry,
            },
        )
    }
}

/// Pinned exact differential program compiled from one relational expression.
///
/// The program is reconstructible from `RelExpr + Γ`; it is never semantic
/// authority. It provides the stable production boundary for Γ-DTC while the
/// lower-level physical kernels migrate to shared SAMF overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelDifferentialProgram {
    semantic_context: kernel_schema::SemanticContext,
    root: RelDifferentialNode,
    physical: CompiledDeltaProgram,
}

impl RelDifferentialProgram {
    pub fn compile(
        query: &RelExpr,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let physical = CompiledDeltaProgram::compile(query, context, registry)?;
        let root = RelDifferentialNode::compile(
            query,
            physical.execution_graph().root(),
            physical.execution_graph(),
        )?;
        Ok(Self {
            semantic_context: context.clone(),
            root,
            physical,
        })
    }

    #[must_use]
    pub const fn root_class(&self) -> RelDifferentialClass {
        self.root.class()
    }

    #[must_use]
    pub fn state_requirements(&self) -> BTreeSet<RelDifferentialStateRequirement> {
        let mut requirements = BTreeSet::new();
        self.root.collect_state_requirements(&mut requirements);
        requirements
    }

    #[must_use]
    pub const fn physical_program(&self) -> &CompiledDeltaProgram {
        &self.physical
    }

    pub fn apply(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        self.root.apply(old, change, context, registry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelObservationKey {
    rows: BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, usize>,
}

impl RelObservationKey {
    #[must_use]
    pub fn distinct_row_classes(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn row_multiplicity(&self) -> usize {
        self.rows.values().sum()
    }
}

/// Exact observation-fiber guard for one pinned relational observation.
///
/// The guard is derived/reconstructible state. Its normalized output key names
/// the current observation fiber, while the differential program provides the
/// exact impact test for candidate model deltas. `source_relations` is only a
/// sound routing envelope; it never replaces exact differential validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelObservationGuard {
    semantic_context: kernel_schema::SemanticContext,
    query: RelExpr,
    observed: RelObservationKey,
    source_relations: BTreeSet<kernel_types::SemanticId>,
    differential: RelDifferentialProgram,
}

impl RelObservationGuard {
    pub fn observe(
        query: &RelExpr,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let prepared = query.prepare(context, registry)?;
        let value = prepared.evaluate(model, context, registry)?;
        let column_equivalences = relation_column_equivalences(prepared.result_type());
        let rows =
            canonical_row_multiset_counts(value.rows(), column_equivalences, context, registry)?
                .ok_or(RelQueryError::CanonicalObservationUnavailable)?;
        let mut source_relations = BTreeSet::new();
        collect_rel_source_relations(query, &mut source_relations);
        Ok(Self {
            semantic_context: context.clone(),
            query: query.clone(),
            observed: RelObservationKey { rows },
            source_relations,
            differential: RelDifferentialProgram::compile(query, context, registry)?,
        })
    }

    #[must_use]
    pub const fn observed_key(&self) -> &RelObservationKey {
        &self.observed
    }

    #[must_use]
    pub fn source_relations(&self) -> &BTreeSet<kernel_types::SemanticId> {
        &self.source_relations
    }

    #[must_use]
    pub const fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub const fn differential(&self) -> &RelDifferentialProgram {
        &self.differential
    }

    pub fn impact(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if matches!(change, Change::NoChange) {
            return Ok(Impact::Unaffected);
        }
        let delta = self.differential.apply(old, change, context, registry)?;
        Ok(if delta.is_empty() {
            Impact::Unaffected
        } else {
            Impact::Changed
        })
    }

    /// Exact transition impact between two complete finite models under the
    /// pinned semantic context of this observation.
    ///
    /// This convenience boundary keeps callers outside `kernel-query` from
    /// depending on the current coarse `Change<FiniteModel>` representation.
    /// Γ-DTC remains the production impact engine; `impact_by_recompute_oracle`
    /// remains an independent parity oracle during rollout.
    pub fn impact_between(
        &self,
        old: &kernel_model::FiniteModel,
        new: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, RelQueryError> {
        self.impact(old, &Change::Replace(new.clone()), context, registry)
    }

    /// Independent full-recompute transition oracle corresponding to
    /// `impact_between`.
    #[must_use]
    pub fn impact_between_by_recompute_oracle(
        &self,
        old: &kernel_model::FiniteModel,
        new: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Impact {
        self.impact_by_recompute_oracle(old, &Change::Replace(new.clone()), context, registry)
    }

    /// Exact parity oracle retained during OFC rollout.
    #[must_use]
    pub fn impact_by_recompute_oracle(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Impact {
        rel_impact_by_recompute(&self.query, old, change, context, registry)
    }
}

fn collect_rel_source_relations(query: &RelExpr, out: &mut BTreeSet<kernel_types::SemanticId>) {
    match query {
        RelExpr::Scan(relation) => {
            out.insert(*relation);
        }
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::TopKWithTies { input, .. }
        | RelExpr::PromoteToBag(input) => collect_rel_source_relations(input, out),
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            collect_rel_source_relations(left, out);
            collect_rel_source_relations(right, out);
        }
    }
}

struct RelEvalContext<'a> {
    model: &'a kernel_model::FiniteModel,
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

impl RelExpr {
    #[must_use]
    pub fn scan_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        let mut relations = BTreeSet::new();
        collect_rel_source_relations(self, &mut relations);
        relations
    }

    pub fn prepare(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRelExpr, RelQueryError> {
        Ok(PreparedRelExpr {
            expr: self.clone(),
            result_type: self.typecheck(context, registry)?,
            semantic_context: context.clone(),
        })
    }

    pub fn typecheck(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        match self {
            Self::Scan(relation) => Self::typecheck_scan(*relation, context),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Self::typecheck_filter(input, *column, value, *equivalence, context, registry),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_filter_columns(
                input,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Project { input, columns } => {
                Self::typecheck_project(input, columns, context, registry)
            }
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Difference { left, right } => {
                Self::typecheck_difference(left, right, context, registry)
            }
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_anti_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Distinct {
                input,
                column_equivalences,
            } => Self::typecheck_distinct(input, column_equivalences, context, registry),
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => Self::typecheck_group(
                input,
                group_columns,
                group_equivalences,
                aggregate,
                context,
                registry,
            ),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                ..
            } => Self::typecheck_top_k_with_ties(input, *column, *ordering, context, registry),
            Self::PromoteToBag(input) => {
                let input_type = input.typecheck(context, registry)?;
                let column_equivalences = match input_type.semantics {
                    kernel_schema::RelationSemantics::Set {
                        column_equivalences,
                    }
                    | kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    } => column_equivalences,
                };
                Ok(RelType {
                    columns: input_type.columns,
                    semantics: kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    },
                })
            }
        }
    }

    fn typecheck_difference(
        left: &Self,
        right: &Self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        if left_type != right_type {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(left_type)
    }

    fn typecheck_anti_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let _ = Self::typecheck_join(
            left,
            right,
            left_column,
            right_column,
            equivalence,
            context,
            registry,
        )?;
        Ok(left_type)
    }

    fn typecheck_scan(
        relation: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
    ) -> Result<RelType, RelQueryError> {
        let definition = context
            .schema
            .relation(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        Ok(RelType {
            columns: definition.columns.clone(),
            semantics: definition.semantics.clone(),
        })
    }

    fn typecheck_filter(
        input: &Self,
        column: usize,
        value: &Value,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_type = input_type
            .columns
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        validate_query_equivalence(equivalence, column_type, context, registry)?;
        let input_equivalence = relation_column_equivalence(&input_type, column)?;
        if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        if !value_shape_matches_type(value, column_type) {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(input_type)
    }

    fn typecheck_filter_columns(
        input: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let left_type = input_type
            .columns
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let right_type = input_type
            .columns
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !query_types_compatible(left_type, right_type, &context.schema) {
            return Err(RelQueryError::TypeMismatch);
        }
        validate_query_equivalence(equivalence, left_type, context, registry)?;
        validate_query_equivalence(equivalence, right_type, context, registry)?;
        let left_input_equivalence = relation_column_equivalence(&input_type, left_column)?;
        let right_input_equivalence = relation_column_equivalence(&input_type, right_column)?;
        if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
            || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
        {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        Ok(input_type)
    }

    fn typecheck_project(
        input: &Self,
        columns: &[usize],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let projected_columns = columns
            .iter()
            .map(|column| {
                input_type
                    .columns
                    .get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let semantics = match input_type.semantics {
            kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => kernel_schema::RelationSemantics::Bag {
                column_equivalences: columns
                    .iter()
                    .map(|column| {
                        column_equivalences
                            .get(*column)
                            .copied()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => {
                let projected = columns
                    .iter()
                    .map(|column| {
                        column_equivalences
                            .get(*column)
                            .copied()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: projected,
                }
            }
        };
        Ok(RelType {
            columns: projected_columns,
            semantics,
        })
    }

    fn typecheck_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        let left_key = left_type
            .columns
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let right_key = right_type
            .columns
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !query_types_compatible(left_key, right_key, &context.schema) {
            return Err(RelQueryError::TypeMismatch);
        }
        validate_query_equivalence(equivalence, left_key, context, registry)?;
        validate_query_equivalence(equivalence, right_key, context, registry)?;
        let left_input_equivalence = relation_column_equivalence(&left_type, left_column)?;
        let right_input_equivalence = relation_column_equivalence(&right_type, right_column)?;
        if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
            || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
        {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        let semantics = match (&left_type.semantics, &right_type.semantics) {
            (
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: left,
                },
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: right,
                },
            ) => kernel_schema::RelationSemantics::Set {
                column_equivalences: left.iter().chain(right).copied().collect(),
            },
            (left_semantics, right_semantics) => {
                let left = match left_semantics {
                    kernel_schema::RelationSemantics::Set {
                        column_equivalences,
                    }
                    | kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    } => column_equivalences,
                };
                let right = match right_semantics {
                    kernel_schema::RelationSemantics::Set {
                        column_equivalences,
                    }
                    | kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    } => column_equivalences,
                };
                kernel_schema::RelationSemantics::Bag {
                    column_equivalences: left.iter().chain(right).copied().collect(),
                }
            }
        };
        let columns = left_type
            .columns
            .into_iter()
            .chain(right_type.columns)
            .collect();
        Ok(RelType { columns, semantics })
    }

    fn typecheck_group(
        input: &Self,
        group_columns: &[usize],
        group_equivalences: &[kernel_types::SemanticId],
        aggregate: &AggregateSpec,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        if group_columns.len() != group_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut columns = Vec::with_capacity(group_columns.len() + 1);
        for (column, equivalence) in group_columns.iter().zip(group_equivalences) {
            let ty = input_type
                .columns
                .get(*column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            validate_query_equivalence(*equivalence, ty, context, registry)?;
            let input_equivalence = relation_column_equivalence(&input_type, *column)?;
            if !registry.equivalence_refines(context, input_equivalence, *equivalence)? {
                return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
            }
            columns.push(ty.clone());
        }
        let (result_type, result_equivalence) = match aggregate {
            AggregateSpec::Count { result_equivalence } => (
                kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64),
                *result_equivalence,
            ),
            AggregateSpec::ExactF64Sum {
                value_column,
                result_equivalence,
            } => {
                let value_type = input_type
                    .columns
                    .get(*value_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let f64_type = kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64);
                if value_type != &f64_type {
                    return Err(RelQueryError::TypeMismatch);
                }
                (f64_type, *result_equivalence)
            }
        };
        validate_query_equivalence(result_equivalence, &result_type, context, registry)?;
        columns.push(result_type);
        let mut column_equivalences = group_equivalences.to_vec();
        column_equivalences.push(result_equivalence);
        Ok(RelType {
            columns,
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences,
            },
        })
    }

    fn typecheck_distinct(
        input: &Self,
        column_equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        if input_type.columns.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        for (equivalence, column) in column_equivalences.iter().zip(&input_type.columns) {
            validate_query_equivalence(*equivalence, column, context, registry)?;
        }
        for (index, equivalence) in column_equivalences.iter().enumerate() {
            let input_equivalence = relation_column_equivalence(&input_type, index)?;
            if !registry.equivalence_refines(context, input_equivalence, *equivalence)? {
                return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
            }
        }
        Ok(RelType {
            columns: input_type.columns,
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: column_equivalences.to_vec(),
            },
        })
    }

    fn typecheck_top_k_with_ties(
        input: &Self,
        column: usize,
        ordering: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_type = input_type
            .columns
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let ordering_domain = registry.ordering_domain(context, ordering)?;
        let expected = kernel_semantics::domain_for_type(column_type)
            .map(kernel_semantics::OrderingDomain::from)
            .ok_or(RelQueryError::TypeMismatch)?;
        if ordering_domain != expected {
            return Err(RelQueryError::TypeMismatch);
        }
        let column_equivalences = match &input_type.semantics {
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            }
            | kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        let equivalence = *column_equivalences
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !registry.ordering_congruent_with_equivalence(context, ordering, equivalence)? {
            return Err(RelQueryError::OrderingNotCongruentWithEquality);
        }
        Ok(input_type)
    }

    pub fn evaluate(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        self.prepare(context, registry)?
            .evaluate(model, context, registry)
    }

    fn evaluate_unchecked(
        &self,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        match self {
            Self::Scan(relation) => Self::eval_scan(*relation, eval),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Self::eval_filter(input, *column, value, *equivalence, eval),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_filter_columns(input, *left_column, *right_column, *equivalence, eval),
            Self::Project { input, columns } => Self::eval_project(input, columns, eval),
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_join(left, right, *left_column, *right_column, *equivalence, eval),
            Self::Difference { left, right } => Self::eval_difference(left, right, eval),
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_anti_join(left, right, *left_column, *right_column, *equivalence, eval),
            Self::Distinct {
                input,
                column_equivalences,
            } => Self::eval_distinct(input, column_equivalences, eval),
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => Self::eval_group(input, group_columns, group_equivalences, aggregate, eval),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => Self::eval_top_k_with_ties(input, *column, *ordering, *direction, *k, eval),
            Self::PromoteToBag(input) => Ok(RelationValue::Bag(
                input
                    .evaluate(eval.model, eval.semantic, eval.registry)?
                    .into_rows(),
            )),
        }
    }

    fn eval_scan(
        relation: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let definition = eval
            .semantic
            .schema
            .relation(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        let rows = eval
            .model
            .relations
            .get(&relation)
            .cloned()
            .unwrap_or_default();
        Ok(match &definition.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        })
    }

    fn eval_top_k_with_ties(
        input: &Self,
        column: usize,
        ordering: kernel_types::SemanticId,
        direction: OrderDirection,
        k: usize,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        top_k_relation_value(
            input_value,
            column,
            ordering,
            direction,
            k,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_filter(
        input: &Self,
        column: usize,
        value: &Value,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let set_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let mut out = Vec::new();
        for row in input_value.into_rows() {
            let candidate = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            if eval
                .registry
                .equivalent(eval.semantic, equivalence, candidate, value)?
            {
                out.push(row);
            }
        }
        Ok(match set_equivalences {
            Some(column_equivalences) => RelationValue::Set {
                rows: out,
                column_equivalences,
            },
            None => RelationValue::Bag(out),
        })
    }

    fn eval_filter_columns(
        input: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let set_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let mut out = Vec::new();
        for row in input_value.into_rows() {
            let left = row
                .get(left_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let right = row
                .get(right_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if eval
                .registry
                .equivalent(eval.semantic, equivalence, left, right)?
            {
                out.push(row);
            }
        }
        Ok(match set_equivalences {
            Some(column_equivalences) => RelationValue::Set {
                rows: out,
                column_equivalences,
            },
            None => RelationValue::Bag(out),
        })
    }

    fn eval_project(
        input: &Self,
        columns: &[usize],
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let input_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let rows = input_value
            .into_rows()
            .into_iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|column| {
                        row.get(*column)
                            .cloned()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect()
            })
            .collect::<Result<Vec<Row>, _>>()?;
        let Some(equivalences) = input_equivalences else {
            return Ok(RelationValue::Bag(rows));
        };
        let projected_equivalences = columns
            .iter()
            .map(|column| {
                equivalences
                    .get(*column)
                    .copied()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rows = distinct_rows(rows, &projected_equivalences, eval.semantic, eval.registry)?;
        Ok(RelationValue::Set {
            rows,
            column_equivalences: projected_equivalences,
        })
    }

    fn eval_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        join_relation_values(
            left_value,
            right_value,
            left_column,
            right_column,
            equivalence,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_difference(
        left: &Self,
        right: &Self,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_type = left.typecheck(eval.semantic, eval.registry)?;
        let column_equivalences = relation_column_equivalences(&left_type);
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        difference_relation_values(
            left_value,
            right_value,
            column_equivalences,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_anti_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        anti_join_relation_values(
            left_value,
            &right_value,
            left_column,
            right_column,
            equivalence,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_group(
        input: &Self,
        group_columns: &[usize],
        group_equivalences: &[kernel_types::SemanticId],
        aggregate: &AggregateSpec,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        group_relation_value(
            input_value,
            group_columns,
            group_equivalences,
            aggregate,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_distinct(
        input: &Self,
        column_equivalences: &[kernel_types::SemanticId],
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let rows = input
            .evaluate(eval.model, eval.semantic, eval.registry)?
            .into_rows();
        let rows = distinct_rows(rows, column_equivalences, eval.semantic, eval.registry)?;
        Ok(RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        })
    }
}

/// Exact semantic difference under the supplied row equivalences.
///
/// Set inputs use support subtraction. Bag inputs use truncated natural
/// subtraction (monus) per Γ-canonical row class, preserving a representative
/// from the left input for every surviving occurrence.
pub fn difference_relation_values(
    left: RelationValue,
    right: RelationValue,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    match (left, right) {
        (
            RelationValue::Set {
                rows: left_rows,
                column_equivalences: left_equivalences,
            },
            RelationValue::Set {
                rows: right_rows,
                column_equivalences: right_equivalences,
            },
        ) => {
            if left_equivalences != column_equivalences || right_equivalences != column_equivalences
            {
                return Err(RelQueryError::TypeMismatch);
            }
            let mut blocked = BTreeSet::new();
            for row in right_rows {
                blocked.insert(canonical_row_key(
                    &row,
                    column_equivalences,
                    context,
                    registry,
                )?);
            }
            let mut rows = Vec::new();
            for row in left_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                if !blocked.contains(&key) {
                    rows.push(row);
                }
            }
            Ok(RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.to_vec(),
            })
        }
        (RelationValue::Bag(left_rows), RelationValue::Bag(right_rows)) => {
            let mut blockers = BTreeMap::<CanonicalRowKey, usize>::new();
            for row in right_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                *blockers.entry(key).or_default() += 1;
            }
            let mut rows = Vec::new();
            for row in left_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                if let Some(count) = blockers.get_mut(&key)
                    && *count > 0
                {
                    *count -= 1;
                    continue;
                }
                rows.push(row);
            }
            Ok(RelationValue::Bag(rows))
        }
        _ => Err(RelQueryError::TypeMismatch),
    }
}

/// Exact anti-semi join. Right multiplicity is a blocker predicate only:
/// one or more Γ-equivalent right keys suppress the complete left-key fiber.
/// Left multiplicity is otherwise preserved unchanged.
pub fn anti_join_relation_values(
    left: RelationValue,
    right: &RelationValue,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let left_equivalences = match &left {
        RelationValue::Set {
            column_equivalences,
            ..
        } => Some(column_equivalences.clone()),
        RelationValue::Bag(_) => None,
    };
    let mut blocked = BTreeSet::new();
    for row in right.rows() {
        let value = row
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        blocked.insert(
            registry
                .canonical_equivalence_key(context, equivalence, value)
                .map_err(RelQueryError::from)?,
        );
    }
    let mut rows = Vec::new();
    for row in left.into_rows() {
        let value = row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let key = registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(RelQueryError::from)?;
        if !blocked.contains(&key) {
            rows.push(row);
        }
    }
    Ok(match left_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows,
            column_equivalences,
        },
        None => RelationValue::Bag(rows),
    })
}

fn group_relation_value(
    input_value: RelationValue,
    group_columns: &[usize],
    group_equivalences: &[kernel_types::SemanticId],
    aggregate: &AggregateSpec,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    enum State {
        Count(kernel_aggregate::ExactCount),
        ExactF64Sum(kernel_aggregate::ExactF64Sum),
    }

    let rows = input_value.into_rows();
    let mut groups: Vec<(Vec<Value>, State)> = Vec::new();
    if rows.is_empty() && group_columns.is_empty() {
        let state = match aggregate {
            AggregateSpec::Count { .. } => State::Count(kernel_aggregate::ExactCount::default()),
            AggregateSpec::ExactF64Sum { .. } => {
                State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
            }
        };
        groups.push((Vec::new(), state));
    }
    for row in rows {
        let key = group_columns
            .iter()
            .map(|column| {
                row.get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut found = None;
        'groups: for (index, (existing, _)) in groups.iter().enumerate() {
            for ((left, right), equivalence) in existing.iter().zip(&key).zip(group_equivalences) {
                if !registry.equivalent(context, *equivalence, left, right)? {
                    continue 'groups;
                }
            }
            found = Some(index);
            break;
        }
        let index = if let Some(index) = found {
            index
        } else {
            let state = match aggregate {
                AggregateSpec::Count { .. } => {
                    State::Count(kernel_aggregate::ExactCount::default())
                }
                AggregateSpec::ExactF64Sum { .. } => {
                    State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
                }
            };
            groups.push((key, state));
            groups.len() - 1
        };
        match (&mut groups[index].1, aggregate) {
            (State::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
            (State::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { value_column, .. }) => {
                let value = row
                    .get(*value_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let Value::F64Bits(bits) = value else {
                    return Err(RelQueryError::TypeMismatch);
                };
                sum.add(f64::from_bits(*bits))?;
            }
            _ => unreachable!("aggregate state is created from the same AggregateSpec"),
        }
    }
    let mut rows = Vec::with_capacity(groups.len());
    for (mut key, state) in groups {
        let value = match state {
            State::Count(count) => Value::I64(count.finish_i64()?),
            State::ExactF64Sum(sum) => Value::F64Bits(sum.finish().to_bits()),
        };
        key.push(value);
        rows.push(key);
    }
    let result_equivalence = match aggregate {
        AggregateSpec::Count { result_equivalence }
        | AggregateSpec::ExactF64Sum {
            result_equivalence, ..
        } => *result_equivalence,
    };
    let mut column_equivalences = group_equivalences.to_vec();
    column_equivalences.push(result_equivalence);
    Ok(RelationValue::Set {
        rows,
        column_equivalences,
    })
}

fn compare_with_direction(
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    ordering: kernel_types::SemanticId,
    left: &Value,
    right: &Value,
    direction: OrderDirection,
) -> Result<std::cmp::Ordering, RelQueryError> {
    let ordering = registry.compare(context, ordering, left, right)?;
    Ok(match direction {
        OrderDirection::Ascending => ordering,
        OrderDirection::Descending => ordering.reverse(),
    })
}

fn fallible_insertion_sort_rows(
    rows: &mut [Row],
    column: usize,
    ordering: kernel_types::SemanticId,
    direction: OrderDirection,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelQueryError> {
    for index in 1..rows.len() {
        let mut cursor = index;
        while cursor > 0 {
            let right = rows[cursor]
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let left = rows[cursor - 1]
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if compare_with_direction(context, registry, ordering, right, left, direction)?
                != std::cmp::Ordering::Less
            {
                break;
            }
            rows.swap(cursor - 1, cursor);
            cursor -= 1;
        }
    }
    Ok(())
}

fn value_shape_matches_type(value: &Value, ty: &kernel_schema::TypeExpr) -> bool {
    value_shape_matches_type_inner(value, ty, &std::collections::BTreeMap::new())
}

fn value_shape_matches_type_inner<'a>(
    value: &Value,
    ty: &'a kernel_schema::TypeExpr,
    recursive: &std::collections::BTreeMap<kernel_schema::TypeVar, &'a kernel_schema::TypeExpr>,
) -> bool {
    match (value, ty) {
        (Value::Unit, kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Unit))
        | (Value::Bool(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Bool))
        | (Value::I64(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64))
        | (Value::F64Bits(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64))
        | (Value::Text(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Text)) => {
            true
        }
        (
            Value::LiveEntityRef { entity_type, .. },
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(expected)),
        )
        | (
            Value::HistoricalEntityId { entity_type, .. },
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(
                expected,
            )),
        ) => entity_type == expected,
        (Value::Product(values), kernel_schema::TypeExpr::Product(fields)) => {
            values.len() == fields.len()
                && fields.iter().all(|(field, field_type)| {
                    values.get(field).is_some_and(|value| {
                        value_shape_matches_type_inner(value, field_type, recursive)
                    })
                })
        }
        (Value::Option(value), kernel_schema::TypeExpr::Option(inner)) => value
            .as_deref()
            .is_none_or(|value| value_shape_matches_type_inner(value, inner, recursive)),
        (Value::Variant { tag, value }, kernel_schema::TypeExpr::Sum(variants)) => variants
            .get(tag)
            .is_some_and(|variant| value_shape_matches_type_inner(value, variant, recursive)),
        (Value::Seq(values), kernel_schema::TypeExpr::Seq(element)) => values
            .iter()
            .all(|value| value_shape_matches_type_inner(value, element, recursive)),
        (
            Value::Set {
                equivalence,
                elements,
            },
            kernel_schema::TypeExpr::Set {
                element,
                equivalence: expected,
            },
        ) => {
            equivalence == expected
                && elements
                    .iter()
                    .all(|value| value_shape_matches_type_inner(value, element, recursive))
        }
        (
            Value::Bag {
                equivalence,
                entries,
            },
            kernel_schema::TypeExpr::Bag {
                element,
                equivalence: expected,
            },
        ) => {
            equivalence == expected
                && entries
                    .iter()
                    .all(|(value, _)| value_shape_matches_type_inner(value, element, recursive))
        }
        (
            Value::Map {
                key_equivalence,
                entries,
            },
            kernel_schema::TypeExpr::Map {
                key,
                value,
                key_equivalence: expected,
            },
        ) => {
            key_equivalence == expected
                && entries.iter().all(|(entry_key, entry_value)| {
                    value_shape_matches_type_inner(entry_key, key, recursive)
                        && value_shape_matches_type_inner(entry_value, value, recursive)
                })
        }
        (_, kernel_schema::TypeExpr::Mu { binder, body }) => {
            let mut next = recursive.clone();
            next.insert(*binder, ty);
            value_shape_matches_type_inner(value, body, &next)
        }
        (_, kernel_schema::TypeExpr::Var(var)) => recursive
            .get(var)
            .is_some_and(|bound| value_shape_matches_type_inner(value, bound, recursive)),
        _ => false,
    }
}

fn query_types_compatible(
    left: &kernel_schema::TypeExpr,
    right: &kernel_schema::TypeExpr,
    schema: &kernel_schema::Schema,
) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(left)),
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(right)),
        )
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(left)),
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(right)),
        ) => schema.is_subtype(*left, *right) || schema.is_subtype(*right, *left),
        _ => false,
    }
}

fn validate_query_equivalence(
    equivalence: kernel_types::SemanticId,
    ty: &kernel_schema::TypeExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelQueryError> {
    let expected = kernel_semantics::domain_for_type(ty).ok_or(RelQueryError::TypeMismatch)?;
    let actual = registry.equivalence_domain(context, equivalence)?;
    if expected == actual {
        Ok(())
    } else {
        Err(RelQueryError::Semantic(
            kernel_semantics::SemanticError::EquivalenceDomainMismatch {
                equivalence,
                expected,
                actual,
            },
        ))
    }
}

fn relation_column_equivalence(
    relation_type: &RelType,
    column: usize,
) -> Result<kernel_types::SemanticId, RelQueryError> {
    relation_column_equivalences(relation_type)
        .get(column)
        .copied()
        .ok_or(RelQueryError::ColumnOutOfBounds)
}

fn relation_column_equivalences(relation_type: &RelType) -> &[kernel_types::SemanticId] {
    match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    }
}

fn distinct_rows(
    rows: Vec<Row>,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    if let Some(first) = rows.first()
        && first.len() != column_equivalences.len()
    {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    let mut out: Vec<Row> = Vec::new();
    'candidate: for row in rows {
        if row.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        for existing in &out {
            let mut same = true;
            for ((left, right), equivalence) in existing.iter().zip(&row).zip(column_equivalences) {
                if !registry.equivalent(context, *equivalence, left, right)? {
                    same = false;
                    break;
                }
            }
            if same {
                continue 'candidate;
            }
        }
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
mod relational_tests {
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn setup() -> (
        SemanticContext,
        SemanticRegistry,
        SemanticId,
        SemanticId,
        SemanticId,
    ) {
        let text_eq = SemanticId::new(100);
        let i64_eq = SemanticId::new(101);
        let left = SemanticId::new(200);
        let right = SemanticId::new(201);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::Text),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![text_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        (context, registry, text_eq, left, right)
    }

    #[test]
    fn compiled_linear_island_matches_maintained_chain_and_uses_source_coordinates() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("alpha".into()),
                    equivalence: text_eq,
                }),
                columns: vec![1, 0],
            }),
            column: 1,
            value: Value::Text("ALPHA".into()),
            equivalence: text_eq,
        };
        let differential = RelDifferentialProgram::compile(&query, &context, &registry).unwrap();
        let islands = differential.physical_program().linear_islands();
        assert_eq!(islands.len(), 1);
        assert_eq!(islands[0].input_width(), 2);
        assert_eq!(islands[0].projection(), &[1, 0]);
        assert_eq!(islands[0].predicates().len(), 2);
        assert!(islands[0].predicates().iter().all(|predicate| matches!(
            predicate,
            LinearIslandPredicate::EqConst {
                source_column: 0,
                ..
            }
        )));

        let source_delta = RelationDelta {
            inserted: vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("beta".into()), Value::I64(2)],
                vec![Value::Text("ALPHA".into()), Value::I64(3)],
            ],
            removed: Vec::new(),
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let fused = islands[0]
            .execute(&source_delta.as_delta_view(), &context, &registry)
            .unwrap();
        let mut fused_rows = Vec::new();
        fused.visit(|weight, row| fused_rows.push((weight, row.clone())));

        let model = FiniteModel::default();
        let mut maintained =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let output = maintained
            .apply_relation_deltas(
                &BTreeMap::from([(relation, source_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        let expected = output
            .inserted
            .into_iter()
            .map(|row| (1, row))
            .chain(output.removed.into_iter().map(|row| (-1, row)))
            .collect::<Vec<_>>();
        assert_eq!(fused_rows, expected);
    }

    #[test]
    fn compiled_delta_program_records_every_non_linear_barrier_class() {
        let (mut context, mut registry, text_eq, left, right) = setup();
        let i64_eq = SemanticId::new(101);
        let i64_order = SemanticId::new(102);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        context.environment.pin_module(i64_order, order_digest);

        let unary = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Group {
                input: Box::new(RelExpr::Project {
                    input: Box::new(RelExpr::Distinct {
                        input: Box::new(RelExpr::Scan(left)),
                        column_equivalences: vec![text_eq, i64_eq],
                    }),
                    columns: vec![0],
                }),
                group_columns: vec![0],
                group_equivalences: vec![text_eq],
                aggregate: AggregateSpec::Count {
                    result_equivalence: i64_eq,
                },
            }),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 3,
        };
        let unary_program = RelDifferentialProgram::compile(&unary, &context, &registry).unwrap();
        assert_eq!(
            unary_program.physical_program().barriers(),
            &[
                BarrierKernelClass::ZeroCrossing,
                BarrierKernelClass::ZeroCrossing,
                BarrierKernelClass::Annotation,
                BarrierKernelClass::OrderedBoundary,
            ]
        );

        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        assert_eq!(
            RelDifferentialProgram::compile(&join, &context, &registry)
                .unwrap()
                .physical_program()
                .barriers(),
            &[BarrierKernelClass::BilinearPullback]
        );

        let blocker = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };
        assert_eq!(
            RelDifferentialProgram::compile(&blocker, &context, &registry)
                .unwrap()
                .physical_program()
                .barriers(),
            &[BarrierKernelClass::BlockerZeroCrossing]
        );
    }

    #[test]
    fn blocker_difference_plans_weighted_zero_crossing_without_mutation() {
        let (context, registry, _, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let result_type = left_type.clone();
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let left = RelationValue::Bag(vec![alpha.clone(), alpha.clone(), alpha.clone()]);
        let right = RelationValue::Bag(vec![alpha.clone()]);
        let kind = MaintainedBlockerKind::Difference;
        let mut state = MaterializedBlockerDeltaState::build(
            &left,
            &right,
            BlockerBuildSpec {
                kind: &kind,
                left_type,
                right_type,
                result_type,
                context: &context,
                registry: &registry,
            },
        )
        .unwrap();

        let before = state.clone();
        let left_delta = AdaptiveDelta::<Row, 4>::default();
        let mut right_delta = AdaptiveDelta::<Row, 4>::default();
        right_delta.push_weighted(2, alpha.clone());
        let planned = state
            .plan_delta_views(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        let mut effect = Vec::new();
        planned
            .effect
            .visit(|weight, row| effect.push((weight, row.clone())));
        assert_eq!(effect, vec![(-2, alpha.clone())]);
        assert_eq!(state, before);
        state.commit_patch(planned.patch);
        assert!(state.output_value().rows().is_empty());

        let before_invalid = state.clone();
        let mut invalid = AdaptiveDelta::<Row, 4>::default();
        invalid.push_weighted(-4, alpha);
        assert!(matches!(
            state.plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &invalid,
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn blocker_antijoin_preserves_same_key_left_replacement_and_zero_crossing() {
        let (context, registry, text_eq, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let first = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let second = vec![Value::Text("alpha".into()), Value::I64(2)];
        let replacement = vec![Value::Text("ALPHA".into()), Value::I64(3)];
        let blocker = vec![Value::Text("aLpHa".into()), Value::I64(99)];
        let kind = MaintainedBlockerKind::AntiJoin {
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut state = MaterializedBlockerDeltaState::build(
            &RelationValue::Bag(vec![first.clone(), second.clone()]),
            &RelationValue::Bag(Vec::new()),
            BlockerBuildSpec {
                kind: &kind,
                left_type,
                right_type,
                result_type: RelExpr::Scan(left_id)
                    .typecheck(&context, &registry)
                    .unwrap(),
                context: &context,
                registry: &registry,
            },
        )
        .unwrap();

        let mut left_delta = AdaptiveDelta::<Row, 4>::default();
        left_delta.push_weighted(-1, first.clone());
        left_delta.push_weighted(1, replacement.clone());
        let planned = state
            .plan_delta_views(
                &left_delta,
                &AdaptiveDelta::<Row, 4>::default(),
                &context,
                &registry,
            )
            .unwrap();
        let mut replacement_effect = Vec::new();
        planned
            .effect
            .visit(|weight, row| replacement_effect.push((weight, row.clone())));
        assert_eq!(
            replacement_effect,
            vec![(-1, first), (1, replacement.clone())]
        );
        state.commit_patch(planned.patch);

        let before_crossing = state.clone();
        let mut right_delta = AdaptiveDelta::<Row, 4>::default();
        right_delta.push_weighted(2, blocker.clone());
        let crossing = state
            .plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &right_delta,
                &context,
                &registry,
            )
            .unwrap();
        let mut crossing_effect = Vec::new();
        crossing
            .effect
            .visit(|weight, row| crossing_effect.push((weight, row.clone())));
        assert_eq!(crossing_effect.len(), 2);
        assert!(crossing_effect.iter().all(|(weight, _)| *weight == -1));
        assert!(crossing_effect.iter().any(|(_, row)| row == &second));
        assert!(crossing_effect.iter().any(|(_, row)| row == &replacement));
        assert_eq!(state, before_crossing);
        state.commit_patch(crossing.patch);
        assert!(state.output_value().rows().is_empty());

        let before_invalid = state.clone();
        let mut invalid = AdaptiveDelta::<Row, 4>::default();
        invalid.push_weighted(-3, blocker);
        assert!(matches!(
            state.plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &invalid,
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn recursive_difference_matches_recompute_for_two_sided_transition() {
        let (context, registry, _, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let alpha1 = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let alpha1_alt = vec![Value::Text("ALPHA".into()), Value::I64(1)];
        let beta2 = vec![Value::Text("Beta".into()), Value::I64(2)];
        let gamma3 = vec![Value::Text("Gamma".into()), Value::I64(3)];
        let query = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left_id)),
            right: Box::new(RelExpr::Scan(right_id)),
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left_id, vec![alpha1.clone(), alpha1.clone(), beta2.clone()]);
        old.relations.insert(right_id, vec![alpha1_alt.clone()]);
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![gamma3.clone()],
            removed: vec![beta2],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: vec![alpha1_alt.clone()],
            removed: Vec::new(),
            result_type: right_type,
        };
        let mut next = old.clone();
        next.relations
            .insert(left_id, vec![alpha1.clone(), alpha1, gamma3]);
        next.relations
            .insert(right_id, vec![alpha1_alt.clone(), alpha1_alt]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let actual = state
            .apply_relation_deltas(
                &BTreeMap::from([(left_id, left_delta), (right_id, right_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&actual, &oracle, &context, &registry).unwrap()
        );
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&next, &context, &registry).unwrap()
        );
    }

    #[test]
    fn recursive_antijoin_matches_recompute_and_invalid_blocker_is_atomic() {
        let (context, registry, text_eq, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let alpha1 = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let alpha3 = vec![Value::Text("alpha".into()), Value::I64(3)];
        let beta2 = vec![Value::Text("Beta".into()), Value::I64(2)];
        let blocker = vec![Value::Text("aLpHa".into()), Value::I64(99)];
        let query = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(left_id)),
            right: Box::new(RelExpr::Scan(right_id)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left_id, vec![alpha1.clone(), beta2.clone()]);
        old.relations.insert(right_id, vec![blocker.clone()]);
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![alpha3.clone()],
            removed: vec![beta2],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![blocker.clone()],
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations.insert(left_id, vec![alpha1, alpha3]);
        next.relations.insert(right_id, Vec::new());
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let actual = state
            .apply_relation_deltas(
                &BTreeMap::from([(left_id, left_delta), (right_id, right_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&actual, &oracle, &context, &registry).unwrap()
        );
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&next, &context, &registry).unwrap()
        );
        let before_invalid = state.clone();
        let invalid = RelationDelta {
            inserted: Vec::new(),
            removed: vec![blocker],
            result_type: right_type,
        };
        assert_eq!(
            state.apply_relation_deltas(
                &BTreeMap::from([(right_id, invalid)]),
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn validated_frames_keep_same_relation_self_join_leaves_disjoint() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relation)),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let first = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let second = vec![Value::Text("ALPHA".into()), Value::I64(2)];
        let mut old = FiniteModel::default();
        old.relations.insert(relation, vec![first.clone()]);
        let mut maintained =
            MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let scan_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        maintained
            .apply_relation_deltas(
                &BTreeMap::from([(
                    relation,
                    RelationDelta {
                        inserted: vec![second.clone()],
                        removed: Vec::new(),
                        result_type: scan_type,
                    },
                )]),
                &context,
                &registry,
            )
            .unwrap();

        let mut new = old;
        new.relations.get_mut(&relation).unwrap().push(second);
        assert_eq!(
            maintained.output_value(&context, &registry).unwrap(),
            query.evaluate(&new, &context, &registry).unwrap()
        );
    }

    #[test]
    fn maintained_plan_clone_uses_cow_and_isolates_mutation() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut candidate = state.clone();
        assert!(Arc::ptr_eq(&state.arena, &candidate.arena));

        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Gamma".into()), Value::I64(3)]],
            removed: vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        candidate
            .apply_relation_deltas(&BTreeMap::from([(relation, delta)]), &context, &registry)
            .unwrap();

        assert!(!Arc::ptr_eq(&state.arena, &candidate.arena));
        assert_eq!(
            state.output_value(&context, &registry).unwrap().rows(),
            &[
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ]
        );
        assert_eq!(
            candidate.output_value(&context, &registry).unwrap().rows(),
            &[
                vec![Value::Text("Beta".into()), Value::I64(2)],
                vec![Value::Text("Gamma".into()), Value::I64(3)],
            ]
        );
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_maintained_plan_is_direct_flat_arena() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
        );
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        assert_eq!(
            state.arena.len(),
            state
                .differential
                .physical_program()
                .execution_graph()
                .node_count()
        );
        assert!(
            state
                .differential
                .physical_program()
                .execution_graph()
                .has_typed_metadata()
        );
    }

    #[test]
    fn relation_delta_prepares_intent_bearing_gamma_rewrite_from_exact_endpoint() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let old = RelationValue::Bag(vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Beta".into()), Value::I64(2)]],
            removed: vec![vec![Value::Text("alpha".into()), Value::I64(1)]],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        let spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7000)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7001)),
            footprint: kernel_change::RewriteFootprint::default(),
        };
        let prepared = delta
            .prepare_relation_rewrite(
                &old,
                &context,
                &registry,
                &spec,
                vec![SemanticId::new(7002)],
            )
            .unwrap();
        assert_eq!(prepared.rewrite.spec, spec.id);
        assert_eq!(prepared.rewrite.law_set, spec.law_set);
        assert_eq!(
            prepared.rewrite.explicit_inputs,
            vec![SemanticId::new(7002)]
        );
        assert_eq!(prepared.delta, delta);
        assert_eq!(
            prepared.rewrite.apply(&old),
            RelationValue::Bag(vec![vec![Value::Text("Beta".into()), Value::I64(2)]])
        );
    }

    #[test]
    fn filter_and_distinct_use_semantic_equality_not_rust_equality() {
        let (context, registry, text_eq, relation, _) = setup();
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("alpha".into()), Value::I64(2)],
                vec![Value::Text("Beta".into()), Value::I64(3)],
            ],
        );
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("ALPHA".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry),
            Ok(RelationValue::Set {
                rows: vec![vec![Value::Text("Alpha".into())]],
                column_equivalences: vec![text_eq],
            })
        );
    }

    #[test]
    fn differential_program_classifies_state_and_matches_recompute_oracle() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("ALPHA".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let program = RelDifferentialProgram::compile(&query, &context, &registry).unwrap();
        assert_eq!(program.root_class(), RelDifferentialClass::ZeroCrossing);
        assert_eq!(
            program.state_requirements(),
            BTreeSet::from([RelDifferentialStateRequirement::SetSupport])
        );

        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations.get_mut(&relation).unwrap().extend([
            vec![Value::Text("alpha".into()), Value::I64(3)],
            vec![Value::Text("Gamma".into()), Value::I64(4)],
        ]);
        let change = Change::Replace(next);
        let exact = program.apply(&old, &change, &context, &registry).unwrap();
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        assert_eq!(exact, oracle);
    }

    #[test]
    fn differential_program_rejects_semantic_context_drift() {
        let (context, registry, _, relation, _) = setup();
        let program =
            RelDifferentialProgram::compile(&RelExpr::Scan(relation), &context, &registry).unwrap();
        let mut drifted = context.clone();
        drifted.environment = SemanticEnvironment::new(SemanticEnvId::new(999));
        assert_eq!(
            program.apply(
                &FiniteModel::default(),
                &Change::NoChange,
                &drifted,
                &registry
            ),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn observation_guard_uses_dtc_for_exact_fiber_impact_and_keeps_oracle_parity() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(left)),
                    column: 0,
                    value: Value::Text("alpha".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        old.relations.insert(
            right,
            vec![vec![Value::Text("Other".into()), Value::I64(9)]],
        );
        let guard = RelObservationGuard::observe(&query, &old, &context, &registry).unwrap();
        assert_eq!(guard.source_relations(), &BTreeSet::from([left]));
        assert_eq!(guard.observed_key().distinct_row_classes(), 1);
        assert_eq!(guard.observed_key().row_multiplicity(), 1);

        let mut unrelated = old.clone();
        unrelated
            .relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::Text("Else".into()), Value::I64(10)]);
        let change = Change::Replace(unrelated);
        assert_eq!(
            guard.impact(&old, &change, &context, &registry).unwrap(),
            Impact::Unaffected
        );
        assert_eq!(
            guard.impact_by_recompute_oracle(&old, &change, &context, &registry),
            Impact::Unaffected
        );

        // The source relation changes, but the Distinct observation remains in
        // the same semantic fiber because "Alpha" ==Γ "alpha".
        let mut duplicate = old.clone();
        duplicate
            .relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(2)]);
        let change = Change::Replace(duplicate);
        assert_eq!(
            guard.impact(&old, &change, &context, &registry).unwrap(),
            Impact::Unaffected
        );
        assert_eq!(
            guard.impact_by_recompute_oracle(&old, &change, &context, &registry),
            Impact::Unaffected
        );
    }

    #[test]
    fn canonical_relation_multiset_matches_reference_for_structural_composite_rows() {
        let text_eq = SemanticId::new(98_300);
        let set_eq = SemanticId::new(98_301);
        let i64_eq = SemanticId::new(98_302);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_300));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_300));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let equivalences = [set_eq, i64_eq];
        let source = vec![
            vec![set(&["A", "B"]), Value::I64(1)],
            vec![set(&["b", "a"]), Value::I64(1)],
            vec![set(&["C"]), Value::I64(2)],
            vec![set(&["D"]), Value::I64(3)],
        ];
        let target = vec![
            vec![set(&["B", "A"]), Value::I64(1)],
            vec![set(&["c"]), Value::I64(2)],
            vec![set(&["X"]), Value::I64(9)],
        ];

        let canonical =
            unmatched_semantic_rows(&source, &target, &equivalences, &context, &registry).unwrap();
        let reference = unmatched_semantic_rows_by_matching(
            &source,
            &target,
            &equivalences,
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(canonical, reference);
        assert_eq!(
            canonical,
            vec![
                vec![set(&["b", "a"]), Value::I64(1)],
                vec![set(&["D"]), Value::I64(3)],
            ]
        );

        let equivalent = vec![
            vec![set(&["d"]), Value::I64(3)],
            vec![set(&["C"]), Value::I64(2)],
            vec![set(&["A", "B"]), Value::I64(1)],
            vec![set(&["B", "a"]), Value::I64(1)],
        ];
        assert_eq!(
            rows_as_multisets_equivalent(&source, &equivalent, &equivalences, &context, &registry),
            rows_as_multisets_equivalent_by_matching(
                &source,
                &equivalent,
                &equivalences,
                &context,
                &registry,
            )
        );
        assert!(
            rows_as_multisets_equivalent(&source, &equivalent, &equivalences, &context, &registry,)
                .unwrap()
        );

        let fewer = equivalent[..3].to_vec();
        assert!(
            !rows_as_multisets_equivalent(&source, &fewer, &equivalences, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    #[ignore = "diagnostic benchmark; run explicitly in release mode"]
    fn pass55_canonical_relation_multiset_benchmark() {
        use std::time::Instant;

        let (context, registry, text_eq, _, _) = setup();
        let equivalences = [text_eq];
        let source = (0..4_000)
            .map(|value| vec![Value::Text(format!("Key-{value:05}"))])
            .collect::<Vec<_>>();
        let mut target = source.clone();
        target.reverse();

        let baseline_start = Instant::now();
        let baseline = unmatched_semantic_rows_by_matching(
            &source,
            &target,
            &equivalences,
            &context,
            &registry,
        )
        .unwrap();
        let baseline_elapsed = baseline_start.elapsed();

        let canonical_start = Instant::now();
        let canonical =
            unmatched_semantic_rows(&source, &target, &equivalences, &context, &registry).unwrap();
        let canonical_elapsed = canonical_start.elapsed();

        assert_eq!(canonical, baseline);
        println!(
            "PASS55_RELATION_MULTISET baseline_ns={} canonical_ns={} ratio_milli={}",
            baseline_elapsed.as_nanos(),
            canonical_elapsed.as_nanos(),
            baseline_elapsed
                .as_nanos()
                .saturating_mul(1_000)
                .checked_div(canonical_elapsed.as_nanos())
                .unwrap_or(u128::MAX)
        );
    }

    #[test]
    fn join_preserves_bag_multiplicity_and_uses_pinned_equality() {
        let (context, registry, text_eq, left, right) = setup();
        let mut model = FiniteModel::default();
        model.relations.insert(
            left,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(2)],
            ],
        );
        model
            .relations
            .insert(right, vec![vec![Value::Text("a".into()), Value::I64(10)]]);
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 2);
        assert_eq!(result.rows()[0][1], Value::I64(1));
        assert_eq!(result.rows()[1][1], Value::I64(2));
    }

    #[test]
    fn top_k_with_ties_is_exact_without_observing_physical_row_order() {
        let relation = SemanticId::new(220);
        let i64_eq = SemanticId::new(221);
        let i64_order = SemanticId::new(222);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(220));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(220));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::I64(2)],
                vec![Value::I64(3)],
                vec![Value::I64(1)],
                vec![Value::I64(2)],
            ],
        );

        let ascending = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 2,
        };
        assert_eq!(
            ascending.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(2)],
            ]))
        );

        let descending = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 2,
        };
        assert_eq!(
            descending.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::I64(3)],
                vec![Value::I64(2)],
                vec![Value::I64(2)],
            ]))
        );
    }

    #[test]
    fn top_k_rejects_ordering_with_wrong_semantic_domain_before_execution() {
        let relation = SemanticId::new(230);
        let i64_eq = SemanticId::new(231);
        let text_order = SemanticId::new(232);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::TextBinary);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(230));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(230));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }

    #[test]
    fn top_k_requires_ordering_congruent_with_relation_equality() {
        let relation = SemanticId::new(240);
        let text_eq = SemanticId::new(241);
        let binary_order = SemanticId::new(242);
        let ci_order = SemanticId::new(243);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let binary_digest = registry.install_ordering(OrderingModule::TextBinary);
        let ci_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(240));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(binary_order, binary_digest);
        environment.pin_module(ci_order, ci_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(240));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };

        let incompatible = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: binary_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            incompatible.prepare(&context, &registry),
            Err(RelQueryError::OrderingNotCongruentWithEquality)
        );

        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("b".into())],
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
            ],
        );
        let compatible = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: ci_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            compatible.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
            ]))
        );
    }

    #[test]
    fn filter_rejects_equality_that_can_observe_relation_representatives() {
        let relation = SemanticId::new(245);
        let relation_eq = SemanticId::new(246);
        let exact_eq = SemanticId::new(247);
        let mut registry = SemanticRegistry::default();
        let relation_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(245));
        environment.pin_module(relation_eq, relation_digest);
        environment.pin_module(exact_eq, exact_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(245));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![relation_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };

        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("A".into()),
            equivalence: exact_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );
    }

    #[test]
    fn coarser_query_equality_is_safe_over_finer_relation_equality() {
        let relation = SemanticId::new(248);
        let exact_eq = SemanticId::new(249);
        let ci_eq = SemanticId::new(2500);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(248));
        environment.pin_module(exact_eq, exact_digest);
        environment.pin_module(ci_eq, ci_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(248));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![exact_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("A".into()),
            equivalence: ci_eq,
        };
        assert!(query.prepare(&context, &registry).is_ok());
    }

    #[test]
    fn equality_operators_reject_representative_observing_refinements() {
        let left = SemanticId::new(2510);
        let right = SemanticId::new(2511);
        let ci_eq = SemanticId::new(2512);
        let exact_eq = SemanticId::new(2513);
        let i64_eq = SemanticId::new(2514);
        let mut registry = SemanticRegistry::default();
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2510));
        environment.pin_module(ci_eq, ci_digest);
        environment.pin_module(exact_eq, exact_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2510));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::Text),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![ci_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };

        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: exact_eq,
        };
        assert_eq!(
            join.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );

        let distinct = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(left)),
            column_equivalences: vec![exact_eq, i64_eq],
        };
        assert_eq!(
            distinct.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );

        let group = RelExpr::Group {
            input: Box::new(RelExpr::Scan(left)),
            group_columns: vec![0],
            group_equivalences: vec![exact_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        assert_eq!(
            group.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );
    }

    #[test]
    fn relational_impact_uses_bag_semantics_not_rust_row_representation() {
        let relation = SemanticId::new(250);
        let text_eq = SemanticId::new(251);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(250));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(250));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into())]]);
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into())]]);
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Unaffected
        );
        assert_eq!(
            rel_derivative_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace({
                    let mut changed = FiniteModel::default();
                    changed
                        .relations
                        .insert(relation, vec![vec![Value::Text("a".into())]]);
                    changed
                }),
                &context,
                &registry,
            ),
            Change::NoChange
        );
        let delta = rel_delta_by_recompute(
            &RelExpr::Scan(relation),
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        assert!(delta.is_empty());
        assert_eq!(
            delta.result_type,
            RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap()
        );
    }
    #[test]
    fn relational_bag_impact_ignores_physical_row_order() {
        let (context, registry, _, relation, _) = setup();
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("B".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("B".into()), Value::I64(2)],
                vec![Value::Text("A".into()), Value::I64(1)],
            ],
        );
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Unaffected
        );
    }

    #[test]
    fn relational_bag_impact_detects_multiplicity_change() {
        let (context, registry, _, relation, _) = setup();
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(1)],
            ],
        );
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Changed
        );
        let delta = rel_delta_by_recompute(
            &RelExpr::Scan(relation),
            &old,
            &Change::Replace(next),
            &context,
            &registry,
        )
        .unwrap();
        assert!(delta.removed.is_empty());
        assert_eq!(delta.inserted.len(), 1);
        assert!(matches!(&delta.inserted[0][0], Value::Text(_)));
    }

    #[test]
    fn optimized_scan_filter_project_delta_matches_recompute_oracle() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("ALPHA".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations
            .get_mut(&relation)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(3)]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("scan/filter/bag-project path is supported");
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(optimized.inserted.len(), 1);
        assert!(optimized.removed.is_empty());
    }

    #[test]
    fn optimized_bag_project_cancels_projected_replacement_pairs() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("ALPHA".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
        );
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![vec![Value::Text("alpha".into()), Value::I64(2)]],
        );
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("scan/filter/bag-project path is supported");
        assert!(oracle.is_empty());
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_delta_matches_oracle_over_small_hostile_bag_state_space() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("A".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("scan/filter/bag-project path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn materialized_set_support_state_updates_without_rebuilding_old_rows() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let row_upper = vec![Value::Text("A".into())];
        let row_lower = vec![Value::Text("a".into())];
        let mut state = MaterializedSetSupportState::build(
            &[row_upper.clone(), row_lower.clone()],
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            2
        );

        let first = state
            .apply_rows_delta(Vec::new(), vec![row_upper.clone()], &context, &registry)
            .unwrap();
        assert!(first.is_empty());
        assert_eq!(
            state
                .support_count(&row_lower, &context, &registry)
                .unwrap(),
            1
        );

        let second = state
            .apply_rows_delta(Vec::new(), vec![row_lower.clone()], &context, &registry)
            .unwrap();
        assert!(second.inserted.is_empty());
        assert_eq!(second.removed.len(), 1);
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            0
        );

        let third = state
            .apply_rows_delta(vec![row_lower.clone()], Vec::new(), &context, &registry)
            .unwrap();
        assert_eq!(third.inserted.len(), 1);
        assert!(third.removed.is_empty());
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            1
        );
    }

    #[test]
    fn materialized_set_support_state_is_context_bound_and_atomic_on_error() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let row = vec![Value::Text("A".into())];
        let mut state = MaterializedSetSupportState::build(
            std::slice::from_ref(&row),
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply_rows_delta(vec![vec![Value::I64(7)]], Vec::new(), &context, &registry,),
            Err(RelQueryError::TypeMismatch)
        );
        assert_eq!(state, before);
        assert_eq!(
            state.apply_rows_delta(Vec::new(), vec![row.clone(), row], &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);

        let mut other_context = context.clone();
        other_context.environment = SemanticEnvironment::new(SemanticEnvId::new(9999));
        assert_eq!(
            state.support_count(&vec![Value::Text("A".into())], &other_context, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn zero_crossing_kernel_plans_without_mutating_and_commits_once() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let upper = vec![Value::Text("A".into())];
        let lower = vec![Value::Text("a".into())];
        let mut state = MaterializedSetSupportState::build(
            &[upper.clone(), lower.clone()],
            result_type,
            &context,
            &registry,
        )
        .unwrap();

        let remove_both = CompactDelta::Two(
            Weighted {
                weight: -1,
                row: upper.clone(),
            },
            Weighted {
                weight: -1,
                row: lower.clone(),
            },
        );
        let planned = state
            .plan_delta_view(&remove_both, &context, &registry)
            .unwrap();
        assert_eq!(state.support_count(&upper, &context, &registry).unwrap(), 2);
        assert_eq!(planned.effect.support_len(), 1);
        state.commit_support_patch(planned.patch);
        assert_eq!(state.support_count(&upper, &context, &registry).unwrap(), 0);

        let invalid = CompactDelta::one(-1, upper.clone());
        let before = state.clone();
        assert_eq!(
            state.plan_delta_view(&invalid, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn materialized_group_count_state_matches_recompute_across_sequential_changes() {
        let (context, registry, text_eq, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };
        let sequence = [0_u8, 3, 2, 6, 7, 4, 5, 1, 0];
        let mut old = model_for(sequence[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("group state is supported");
        assert!(state.semantic_lookup.is_some());
        assert!(state.group_encoders.is_some());
        for &mask in &sequence[1..] {
            let next = model_for(mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next mask {mask:03b}: maintained={maintained:?} oracle={oracle:?}"
            );
            old = next;
        }
        assert_eq!(state.group_count(), 0);
    }

    #[test]
    fn materialized_exact_f64_group_state_matches_recompute_with_deletions() {
        let relation = SemanticId::new(2580);
        let text_eq = SemanticId::new(2581);
        let f64_eq = SemanticId::new(2582);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2580));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2580));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::F64Bits(0.1_f64.to_bits())],
            vec![Value::Text("a".into()), Value::F64Bits(0.2_f64.to_bits())],
            vec![
                Value::Text("B".into()),
                Value::F64Bits((-3.5_f64).to_bits()),
            ],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };
        let sequence = [0_u8, 7, 6, 2, 3, 1, 5, 4, 0];
        let mut old = model_for(sequence[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("exact group state is supported");
        for &mask in &sequence[1..] {
            let next = model_for(mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next mask {mask:03b}: maintained={maintained:?} oracle={oracle:?}"
            );
            old = next;
        }
        assert_eq!(state.group_count(), 0);
    }

    #[test]
    fn materialized_global_group_count_preserves_empty_identity_row() {
        let (context, registry, _, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let mut empty = FiniteModel::default();
        empty.relations.insert(relation, Vec::new());
        let mut one = FiniteModel::default();
        one.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut state = MaterializedGroupDeltaState::build(&query, &empty, &context, &registry)
            .unwrap()
            .unwrap();
        for (old, next) in [
            (&empty, &one),
            (&one, &empty),
            (&empty, &one),
            (&one, &empty),
        ] {
            let change = Change::Replace(next.clone());
            let oracle = rel_delta_by_recompute(&query, old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
        }
        assert_eq!(state.group_count(), 1);
    }

    #[test]
    fn materialized_group_input_delta_rejects_malformed_or_underflowing_change_atomically() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: SemanticId::new(101),
            },
        };
        let mut model = FiniteModel::default();
        let row = vec![Value::Text("A".into()), Value::I64(1)];
        model.relations.insert(relation, vec![row.clone()]);
        let mut state = MaterializedGroupDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let before = state.clone();
        let input_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        let malformed = RelationDelta {
            inserted: vec![vec![Value::Text("B".into()), Value::Text("wrong".into())]],
            removed: Vec::new(),
            result_type: input_type.clone(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
        assert_eq!(state, before);

        let underflow = RelationDelta {
            inserted: Vec::new(),
            removed: vec![row.clone(), row],
            result_type: input_type,
        };
        assert_eq!(
            state.apply_input_delta(&underflow, &context, &registry),
            Err(RelQueryError::Aggregate(
                kernel_aggregate::AggregateError::CountUnderflow
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn materialized_i64_count_fast_path_matches_recompute_sequentially() {
        let relation = SemanticId::new(2590);
        let i64_eq = SemanticId::new(2591);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2590));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2590));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let model_for = |values: &[i64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::I64(*value)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![1, 1, 2],
            vec![1, 2, 3],
            vec![3, 3],
            vec![],
            vec![2, 2, 2],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn dense_i64_group_plans_without_mutating_and_matches_recompute() {
        let relation = SemanticId::new(2592);
        let i64_eq = SemanticId::new(2593);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2592));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2592));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::I64(0)],
                vec![Value::I64(1)],
                vec![Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations.insert(
            relation,
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.dense_i64_count.is_some());

        let before = state.clone();
        let carrier = CompactDelta::replace(vec![Value::I64(0)], vec![Value::I64(3)]);
        let planned = state.plan_i64_count_delta(&carrier).unwrap();
        assert!(planned.patch.retain_dense);
        assert_eq!(planned.patch.dense_move, Some((0, 3)));
        assert_eq!(state, before);
        state.commit_i64_count_patch(planned.patch).unwrap();

        let change = Change::Replace(next);
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained =
            materialize_delta_view(&planned.effect, state.result_type.clone()).unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
        assert!(state.dense_i64_count.is_some());
    }

    #[test]
    fn dense_i64_group_outlier_falls_back_before_commit_without_semantic_error() {
        let relation = SemanticId::new(2594);
        let i64_eq = SemanticId::new(2595);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2594));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2594));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::I64(0)], vec![Value::I64(1)]]);
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![vec![Value::I64(1)], vec![Value::I64(10_000)]],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.dense_i64_count.is_some());

        let before = state.clone();
        let carrier = CompactDelta::replace(vec![Value::I64(0)], vec![Value::I64(10_000)]);
        let planned = state.plan_i64_count_delta(&carrier).unwrap();
        assert!(!planned.patch.retain_dense);
        assert_eq!(state, before);
        state.commit_i64_count_patch(planned.patch).unwrap();
        assert!(state.dense_i64_count.is_none());

        let change = Change::Replace(next);
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained =
            materialize_delta_view(&planned.effect, state.result_type.clone()).unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    fn optimized_set_project_uses_support_counts_when_projection_hides_a_removal() {
        let relation = SemanticId::new(2520);
        let text_eq = SemanticId::new(2521);
        let i64_eq = SemanticId::new(2522);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2520));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2520));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("A".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(2)]]);
        let change = Change::Replace(next);

        assert!(
            rel_delta_by_recompute(&query, &old, &change, &context, &registry)
                .unwrap()
                .is_empty()
        );
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("set projection support-count path is supported");
        assert!(optimized.is_empty());
    }

    #[test]
    fn materialized_support_state_matches_set_projection_oracle_over_all_small_transitions() {
        let relation = SemanticId::new(2525);
        let text_eq = SemanticId::new(2526);
        let i64_eq = SemanticId::new(2527);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2525));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2525));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let old_projected = project_rows(
                    old.relations.get(&relation).cloned().unwrap_or_default(),
                    &[0],
                )
                .unwrap();
                let mut state = MaterializedSetSupportState::build(
                    &old_projected,
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let scan_delta = rel_delta_optimized(
                    &RelExpr::Scan(relation),
                    &old,
                    &change,
                    &context,
                    &registry,
                )
                .unwrap()
                .expect("scan delta is supported");
                let maintained = state
                    .apply_rows_delta(
                        project_rows(scan_delta.inserted, &[0]).unwrap(),
                        project_rows(scan_delta.removed, &[0]).unwrap(),
                        &context,
                        &registry,
                    )
                    .unwrap();
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn long_lived_materialized_project_state_survives_sequential_model_changes() {
        let relation = SemanticId::new(2528);
        let text_eq = SemanticId::new(2529);
        let i64_eq = SemanticId::new(2539);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2528));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2528));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        let mut old = model_for(0);
        let mut state = MaterializedRelDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("set projection has materialized state");
        for next_mask in [3_u8, 2, 6, 0, 5, 1, 7, 4] {
            let next = model_for(next_mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
            );
            old = next;
        }
    }

    #[test]
    fn long_lived_materialized_distinct_state_survives_sequential_model_changes() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        let mut old = model_for(0);
        let mut state = MaterializedRelDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("distinct has materialized state");
        for next_mask in [3_u8, 2, 6, 0, 5, 1, 7, 4] {
            let next = model_for(next_mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
            );
            old = next;
        }
    }

    #[test]
    fn optimized_set_project_matches_oracle_over_small_support_state_space() {
        let relation = SemanticId::new(2530);
        let text_eq = SemanticId::new(2531);
        let i64_eq = SemanticId::new(2532);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2530));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2530));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("set projection support-count path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn materialized_support_state_matches_distinct_oracle_over_all_small_transitions() {
        let (context, registry, text_eq, relation, _) = setup();
        let child = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let query = RelExpr::Distinct {
            input: Box::new(child.clone()),
            column_equivalences: vec![text_eq],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let next = model_for(next_mask);
                let change = Change::Replace(next);
                let old_child = child
                    .evaluate(&old, &context, &registry)
                    .unwrap()
                    .into_rows();
                let mut state = MaterializedSetSupportState::build(
                    &old_child,
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let child_delta = rel_delta_optimized(&child, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("bag project child delta is supported");
                let maintained = state
                    .apply_rows_delta(
                        child_delta.inserted,
                        child_delta.removed,
                        &context,
                        &registry,
                    )
                    .unwrap();
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_distinct_support_counts_survive_duplicate_removal() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into()), Value::I64(2)]]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("distinct support-count path is supported");
        assert!(oracle.is_empty());
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_distinct_matches_oracle_over_small_hostile_state_space() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("bag project + distinct support-count path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_promote_to_bag_transports_input_delta() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::Scan(relation)));
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut next = old.clone();
        next.relations
            .get_mut(&relation)
            .unwrap()
            .push(vec![Value::Text("B".into()), Value::I64(2)]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("promote-to-bag path is supported");
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_join_local_replay_matches_oracle_over_two_sided_state_space() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let left_rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("B".into()), Value::I64(2)],
        ];
        let right_rows = [
            vec![Value::Text("a".into()), Value::I64(10)],
            vec![Value::Text("B".into()), Value::I64(20)],
        ];

        let model_for = |state: u8| {
            let left_mask = state & 0b0011;
            let right_mask = (state >> 2) & 0b0011;
            let mut model = FiniteModel::default();
            model.relations.insert(
                left,
                left_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| left_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model.relations.insert(
                right,
                right_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| right_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_state in 0_u8..16 {
            for next_state in 0_u8..16 {
                let old = model_for(old_state);
                let change = Change::Replace(model_for(next_state));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("join local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_state:04b}, next={next_state:04b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_top_k_local_replay_matches_oracle_across_threshold_changes() {
        let relation = SemanticId::new(2540);
        let i64_eq = SemanticId::new(2541);
        let i64_order = SemanticId::new(2542);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2540));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2540));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let rows = [
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(2)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("top-k local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_group_count_local_replay_matches_oracle_across_group_birth_and_death() {
        let (context, registry, text_eq, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_global_group_count_handles_empty_identity_transitions() {
        let (context, registry, _, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("B".into()), Value::I64(2)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..4 {
            for next_mask in 0_u8..4 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("global group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:02b}, next={next_mask:02b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_exact_f64_group_local_replay_matches_oracle() {
        let relation = SemanticId::new(2550);
        let text_eq = SemanticId::new(2551);
        let f64_eq = SemanticId::new(2552);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2550));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2550));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::F64Bits(1.0_f64.to_bits())],
            vec![Value::Text("a".into()), Value::F64Bits(2.0_f64.to_bits())],
            vec![Value::Text("B".into()), Value::F64Bits(4.0_f64.to_bits())],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("exact-sum group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_composed_set_distinct_join_promote_pipeline_matches_oracle() {
        let (context, registry, text_eq, left, right) = setup();
        let distinct_text = |relation| RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::JoinEq {
            left: Box::new(distinct_text(left)),
            right: Box::new(distinct_text(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        }));
        let left_rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
        ];
        let right_rows = [
            vec![Value::Text("A".into()), Value::I64(10)],
            vec![Value::Text("B".into()), Value::I64(20)],
        ];
        let model_for = |state: u8| {
            let left_mask = state & 0b0011;
            let right_mask = (state >> 2) & 0b0011;
            let mut model = FiniteModel::default();
            model.relations.insert(
                left,
                left_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| left_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model.relations.insert(
                right,
                right_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| right_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_state in 0_u8..16 {
            for next_state in 0_u8..16 {
                let old = model_for(old_state);
                let change = Change::Replace(model_for(next_state));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("composed incremental pipeline is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_state:04b}, next={next_state:04b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn scan_preserves_set_semantics_from_schema() {
        let relation = SemanticId::new(300);
        let text_eq = SemanticId::new(301);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![vec![Value::Text("x".into())]]);
        assert_eq!(
            RelExpr::Scan(relation).evaluate(&model, &context, &registry),
            Ok(RelationValue::Set {
                rows: vec![vec![Value::Text("x".into())]],
                column_equivalences: vec![text_eq],
            })
        );
    }
    #[test]
    fn typecheck_rejects_wrong_equality_even_for_empty_relation() {
        let relation = SemanticId::new(400);
        let wrong_eq = SemanticId::new(401);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(wrong_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![wrong_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![wrong_eq],
        };
        assert!(matches!(
            query.prepare(&context, &registry),
            Err(RelQueryError::Semantic(
                kernel_semantics::SemanticError::EquivalenceDomainMismatch { .. }
            ))
        ));
    }
    #[test]
    fn typecheck_rejects_wrong_constant_type_on_empty_relation() {
        let relation = SemanticId::new(500);
        let text_eq = SemanticId::new(501);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(7),
            equivalence: text_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }

    #[test]
    fn join_rejects_unrelated_entity_reference_types() {
        let left_relation = SemanticId::new(510);
        let right_relation = SemanticId::new(511);
        let person = SemanticId::new(512);
        let order = SemanticId::new(513);
        let entity_eq = SemanticId::new(514);
        let order_eq = SemanticId::new(515);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let order_digest =
            registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(order));
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(entity_eq, digest);
        environment.pin_module(order_eq, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: left_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: right_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(order))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![order_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: entity_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }
    #[test]
    fn prepared_query_pins_full_semantic_context_not_only_revision_numbers() {
        let relation = SemanticId::new(600);
        let text_eq = SemanticId::new(601);
        let i64_eq = SemanticId::new(602);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut base_environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        base_environment.pin_module(text_eq, text_digest);
        base_environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: base_environment.clone(),
        };
        let prepared = RelExpr::Scan(relation)
            .prepare(&context, &registry)
            .unwrap();

        let mut changed_schema = Schema::new(SchemaRevisionId::new(1));
        changed_schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let changed_context = SemanticContext {
            schema: changed_schema,
            environment: base_environment,
        };
        assert_eq!(
            prepared.evaluate(&FiniteModel::default(), &changed_context, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn prepared_semantic_query_can_rebind_across_same_contract_implementation_upgrade() {
        let relation = SemanticId::new(650);
        let text_eq = SemanticId::new(651);
        let mut registry = SemanticRegistry::default();
        let old = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let new = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let changed = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let make_context = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(text_eq, digest);
            SemanticContext {
                schema: schema.clone(),
                environment,
            }
        };
        let source = make_context(1, old);
        let target = make_context(2, new);
        let changed_law = make_context(3, changed);
        let prepared = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("x".into()),
            equivalence: text_eq,
        }
        .prepare(&source, &registry)
        .unwrap();

        let rebound = prepared
            .rebind_preserving_semantics(&target, &registry)
            .unwrap();
        assert_eq!(rebound.result_type(), prepared.result_type());
        assert_eq!(
            prepared.rebind_preserving_semantics(&changed_law, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }
    #[test]
    fn exact_f64_group_sum_is_reproducible_and_semantically_typed() {
        let relation = SemanticId::new(800);
        let text_eq = SemanticId::new(801);
        let f64_eq = SemanticId::new(802);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("x".into()), Value::F64Bits(1e16_f64.to_bits())],
                vec![Value::Text("x".into()), Value::F64Bits(1.0_f64.to_bits())],
                vec![
                    Value::Text("x".into()),
                    Value::F64Bits((-1e16_f64).to_bits()),
                ],
                vec![Value::Text("y".into()), Value::F64Bits(2.0_f64.to_bits())],
            ],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let prepared = query.prepare(&context, &registry).unwrap();
        assert_eq!(
            prepared.result_type(),
            &RelType {
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            }
        );
        assert_eq!(
            prepared.evaluate(&model, &context, &registry).unwrap(),
            RelationValue::Set {
                rows: vec![
                    vec![Value::Text("x".into()), Value::F64Bits(1.0_f64.to_bits())],
                    vec![Value::Text("y".into()), Value::F64Bits(2.0_f64.to_bits())],
                ],
                column_equivalences: vec![text_eq, f64_eq],
            }
        );
    }

    #[test]
    fn exact_f64_group_sum_rejects_non_finite_input_explicitly() {
        let relation = SemanticId::new(810);
        let text_eq = SemanticId::new(811);
        let f64_eq = SemanticId::new(812);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![
                Value::Text("x".into()),
                Value::F64Bits(f64::NAN.to_bits()),
            ]],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry),
            Err(RelQueryError::Aggregate(
                kernel_aggregate::AggregateError::NonFiniteInput
            ))
        );
    }
    #[test]
    fn filter_rejects_reference_literal_with_wrong_nominal_type_before_execution() {
        let relation = SemanticId::new(820);
        let person = SemanticId::new(821);
        let order = SemanticId::new(822);
        let entity_eq = SemanticId::new(823);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(entity_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::LiveEntityRef {
                entity_type: order,
                id: kernel_types::EntityId::new(1),
            },
            equivalence: entity_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }
    #[test]
    fn distinct_supports_schema_derived_structural_product_equality() {
        let relation = SemanticId::new(830);
        let product_eq = SemanticId::new(831);
        let text_eq = SemanticId::new(832);
        let i64_eq = SemanticId::new(833);
        let name = SemanticId::new(834);
        let age = SemanticId::new(835);
        let product_type = TypeExpr::Product(std::collections::BTreeMap::from([
            (name, TypeExpr::Scalar(ScalarType::Text)),
            (age, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: std::collections::BTreeMap::from([(name, text_eq), (age, i64_eq)]),
                },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![product_type],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![product_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let row = |label: &str| {
            vec![Value::Product(std::collections::BTreeMap::from([
                (name, Value::Text(label.into())),
                (age, Value::I64(30)),
            ]))]
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![row("ALICE"), row("alice")]);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![product_eq],
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 1);

        let state = MaterializedSetSupportState::build(
            &[row("ALICE"), row("alice")],
            RelType {
                columns: vec![TypeExpr::Product(std::collections::BTreeMap::from([
                    (name, TypeExpr::Scalar(ScalarType::Text)),
                    (age, TypeExpr::Scalar(ScalarType::I64)),
                ]))],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![product_eq],
                },
            },
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(state.support_lookup.len(), 1);
        assert_eq!(
            state.support_count(&row("aLiCe"), &context, &registry),
            Ok(2)
        );
    }
    #[test]
    fn filter_accepts_structural_option_literal_with_derived_equivalence() {
        let relation = SemanticId::new(900);
        let text_eq = SemanticId::new(901);
        let option_eq = SemanticId::new(902);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                option_eq,
                kernel_schema::StructuralEquivalenceDef::Option { inner: text_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Option(Box::new(TypeExpr::Scalar(
                    ScalarType::Text,
                )))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![option_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Option(Some(Box::new(Value::Text("ALPHA".into()))))],
                vec![Value::Option(Some(Box::new(Value::Text("beta".into()))))],
                vec![Value::Option(None)],
            ],
        );
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Option(Some(Box::new(Value::Text("alpha".into())))),
            equivalence: option_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 1);
        assert_eq!(
            result.rows()[0][0],
            Value::Option(Some(Box::new(Value::Text("ALPHA".into()))))
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn filter_accepts_guarded_recursive_literal_with_recursive_equivalence() {
        let relation = SemanticId::new(905);
        let root_eq = SemanticId::new(906);
        let sum_eq = SemanticId::new(907);
        let product_eq = SemanticId::new(908);
        let var_eq = SemanticId::new(909);
        let unit_eq = SemanticId::new(910);
        let text_eq = SemanticId::new(911);
        let nil_tag = SemanticId::new(912);
        let cons_tag = SemanticId::new(913);
        let head_field = SemanticId::new(914);
        let tail_field = SemanticId::new(915);

        let mut registry = SemanticRegistry::default();
        let unit_digest = registry.install_equivalence(EquivalenceModule::UnitExact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(905));
        environment.pin_module(unit_eq, unit_digest);
        environment.pin_module(text_eq, text_digest);
        let x = kernel_schema::TypeVar(0);
        let recursive_type = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(std::collections::BTreeMap::from([
                (nil_tag, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons_tag,
                    TypeExpr::Product(std::collections::BTreeMap::from([
                        (head_field, TypeExpr::Scalar(ScalarType::Text)),
                        (tail_field, TypeExpr::Var(x)),
                    ])),
                ),
            ]))),
        };
        let mut schema = Schema::new(SchemaRevisionId::new(905));
        schema
            .define_structural_equivalence(
                root_eq,
                kernel_schema::StructuralEquivalenceDef::Mu { body: sum_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sum_eq,
                kernel_schema::StructuralEquivalenceDef::Sum {
                    variants: std::collections::BTreeMap::from([
                        (nil_tag, unit_eq),
                        (cons_tag, product_eq),
                    ]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: std::collections::BTreeMap::from([
                        (head_field, text_eq),
                        (tail_field, var_eq),
                    ]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                var_eq,
                kernel_schema::StructuralEquivalenceDef::Var { binder: root_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![recursive_type],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![root_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        let nil = || Value::Variant {
            tag: nil_tag,
            value: Box::new(Value::Unit),
        };
        let cons = |head: &str, tail: Value| Value::Variant {
            tag: cons_tag,
            value: Box::new(Value::Product(std::collections::BTreeMap::from([
                (head_field, Value::Text(head.into())),
                (tail_field, tail),
            ]))),
        };
        let stored = cons("ALPHA", cons("Beta", nil()));
        let literal = cons("alpha", cons("beta", nil()));
        let other = cons("gamma", nil());
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![vec![stored.clone()], vec![other]]);
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: literal,
            equivalence: root_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows(), &[vec![stored]]);
    }

    #[test]
    fn exact_query_typecheck_rejects_ambiguous_untyped_constant_but_accepts_annotated_one() {
        let option_type = TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::Text)));
        let untyped = ExactQuery::new(Expr::Const(Value::Option(None)));
        assert_eq!(
            untyped.typecheck(&TypeExpr::Scalar(ScalarType::Unit)),
            Err(QueryTypeError::AmbiguousConstantType)
        );
        let typed = ExactQuery::new(Expr::TypedConst {
            value: Value::Option(None),
            ty: option_type.clone(),
        });
        assert_eq!(
            typed.typecheck(&TypeExpr::Scalar(ScalarType::Unit)),
            Ok(option_type)
        );
    }
    #[test]
    fn generic_group_count_uses_monoid_identity_for_empty_global_group() {
        let relation = SemanticId::new(910);
        let i64_eq = SemanticId::new(911);
        let text_eq = SemanticId::new(912);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(i64_eq, digest);
        environment.pin_module(text_eq, text_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        assert_eq!(
            query
                .evaluate(&FiniteModel::default(), &context, &registry)
                .unwrap(),
            RelationValue::Set {
                rows: vec![vec![Value::I64(0)]],
                column_equivalences: vec![i64_eq],
            }
        );
    }

    #[test]
    fn generic_group_count_respects_semantic_group_equality() {
        let relation = SemanticId::new(920);
        let text_eq = SemanticId::new(921);
        let count_eq = SemanticId::new(922);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let count_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(count_eq, count_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
                vec![Value::Text("B".into())],
            ],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry).unwrap(),
            RelationValue::Set {
                rows: vec![
                    vec![Value::Text("A".into()), Value::I64(2)],
                    vec![Value::Text("B".into()), Value::I64(1)],
                ],
                column_equivalences: vec![text_eq, count_eq],
            }
        );
    }

    #[test]
    fn materialized_i64_top_k_matches_recompute_across_threshold_and_tie_changes() {
        let relation = SemanticId::new(9600);
        let i64_eq = SemanticId::new(9601);
        let i64_order = SemanticId::new(9602);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9600));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9600));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let model_for = |values: &[i64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::I64(*value)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![1, 2, 2, 3, 4],
            vec![0, 2, 2, 3, 4],
            vec![0, 1, 2, 3, 4],
            vec![3, 3, 3, 4],
            vec![5, 5, 1, 1, 1],
            vec![],
            vec![7, 7, 7, 6],
        ];
        for direction in [OrderDirection::Ascending, OrderDirection::Descending] {
            let query = RelExpr::TopKWithTies {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                ordering: i64_order,
                direction,
                k: 2,
            };
            let mut old = model_for(&states[0]);
            let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
                .unwrap()
                .expect("top-k state is supported");
            for values in &states[1..] {
                let next = model_for(values);
                let change = Change::Replace(next.clone());
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let maintained = state
                    .apply_model_change(&old, &change, &context, &registry)
                    .unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "direction={direction:?} values={values:?}: maintained={maintained:?} oracle={oracle:?}"
                );
                assert_eq!(state.row_count(), values.len());
                old = next;
            }
        }
    }

    #[test]
    fn materialized_top_k_generic_text_ordering_preserves_semantic_ties() {
        let relation = SemanticId::new(9610);
        let text_eq = SemanticId::new(9611);
        let text_order = SemanticId::new(9612);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry
            .install_equivalence(kernel_semantics::EquivalenceModule::TextAsciiCaseInsensitive);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9610));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9610));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let model_for = |values: &[&str]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::Text((*value).into())])
                    .collect(),
            );
            model
        };
        let states = [
            vec!["A", "a", "B"],
            vec!["a", "B", "c"],
            vec!["Z", "z", "a"],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(
            state.storage,
            MaintainedTopKStorage::SemanticOrdered { .. }
        ));
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn materialized_top_k_i64_rows_uses_universal_plan_commit_and_is_atomic() {
        let relation = SemanticId::new(9615);
        let i64_eq = SemanticId::new(9616);
        let text_eq = SemanticId::new(9617);
        let i64_order = SemanticId::new(9618);
        let mut registry = SemanticRegistry::default();
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9615));
        environment.pin_module(i64_eq, i64_digest);
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9615));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq, text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 2,
        };
        let row = |key, text: &str| vec![Value::I64(key), Value::Text(text.into())];
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![row(1, "a"), row(2, "b"), row(2, "c"), row(4, "d")],
        );
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(state.storage, MaintainedTopKStorage::I64Rows(_)));

        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![row(0, "z"), row(2, "c"), row(3, "x"), row(4, "d")],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
        old = next;

        let before = state.clone();
        let malformed = RelationDelta {
            inserted: vec![],
            removed: vec![row(99, "missing")],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
        assert_eq!(state.row_count(), old.relations[&relation].len());
    }

    #[test]
    fn materialized_top_k_semantic_ordered_plan_failure_is_atomic() {
        let relation = SemanticId::new(9619);
        let text_eq = SemanticId::new(9623);
        let text_order = SemanticId::new(9624);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let order_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9619));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9619));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![Value::Text("A".into())], vec![Value::Text("b".into())]],
        );
        let mut state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(
            state.storage,
            MaintainedTopKStorage::SemanticOrdered { .. }
        ));
        let before = state.clone();
        let malformed = RelationDelta {
            inserted: vec![vec![Value::Text("c".into())]],
            removed: vec![vec![Value::Text("missing".into())]],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn materialized_top_k_rejects_missing_removal_atomically() {
        let relation = SemanticId::new(9620);
        let i64_eq = SemanticId::new(9621);
        let i64_order = SemanticId::new(9622);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9620));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9620));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(relation, vec![vec![Value::I64(1)]]);
        let mut state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let before = state.clone();
        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(2)]],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&delta, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn materialized_join_i64_two_sided_delta_matches_recompute_and_is_atomic() {
        let left = SemanticId::new(9700);
        let right = SemanticId::new(9701);
        let i64_eq = SemanticId::new(9702);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9700));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9700));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            left,
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
        );
        old.relations.insert(
            right,
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
            ],
        );
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(11)]],
            removed: vec![vec![Value::I64(2), Value::I64(20)]],
            result_type: left_type.clone(),
        };
        let right_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(101)]],
            removed: Vec::new(),
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations.insert(
            left,
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(1), Value::I64(11)],
            ],
        );
        next.relations.insert(
            right,
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
                vec![Value::I64(1), Value::I64(101)],
            ],
        );
        let oracle =
            rel_delta_by_recompute(&query, &old, &Change::Replace(next), &context, &registry)
                .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );

        let before = state.clone();
        let missing = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(9), Value::I64(9)]],
            result_type: left_type,
        };
        let empty_right = RelationDelta {
            inserted: Vec::new(),
            removed: Vec::new(),
            result_type: right_type,
        };
        assert_eq!(
            state.apply_input_deltas(&missing, &empty_right, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn materialized_join_i64_delta_view_preserves_weighted_bilinear_cross_term() {
        let left = SemanticId::new(97_100);
        let right = SemanticId::new(97_101);
        let i64_eq = SemanticId::new(97_102);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97_100));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(97_100));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::I64(1), Value::I64(10)]]);
        old.relations
            .insert(right, vec![vec![Value::I64(1), Value::I64(100)]]);
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let before_state = state.clone();

        let mut left_delta = AdaptiveDelta::<Row, 2>::default();
        left_delta.push_weighted(2, vec![Value::I64(1), Value::I64(11)]);
        let mut right_delta = AdaptiveDelta::<Row, 2>::default();
        right_delta.push_weighted(1, vec![Value::I64(1), Value::I64(101)]);

        let planned = state
            .plan_delta_views(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert_eq!(state, before_state);
        let maintained =
            materialize_delta_view(&planned.effect, state.result_type.clone()).unwrap();

        let mut next = old.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .extend(std::iter::repeat_n(vec![Value::I64(1), Value::I64(11)], 2));
        next.relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::I64(1), Value::I64(101)]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );

        state.commit_join_patch(planned.patch);
        let maintained_output = state.output_value(&context, &registry).unwrap();
        let oracle_output = query.evaluate(&next, &context, &registry).unwrap();
        assert!(
            relation_values_semantically_equivalent(
                &maintained_output,
                &oracle_output,
                &state.result_type,
                &context,
                &registry,
            )
            .unwrap()
        );

        let before_invalid = state.clone();
        let mut valid_left = AdaptiveDelta::<Row, 2>::default();
        valid_left.push_weighted(1, vec![Value::I64(1), Value::I64(12)]);
        let mut invalid_right = AdaptiveDelta::<Row, 2>::default();
        invalid_right.push_weighted(-1, vec![Value::I64(9), Value::I64(999)]);
        assert!(matches!(
            state.plan_delta_views(&valid_left, &invalid_right, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn materialized_join_generic_preserves_ascii_ci_semantics() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        old.relations
            .insert(right, vec![vec![Value::Text("a".into()), Value::I64(10)]]);
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(
            state.storage,
            MaintainedJoinStorage::SemanticIndexed(_)
        ));
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![Value::Text("ALPHA".into()), Value::I64(2)]],
            removed: Vec::new(),
            result_type: left_type.clone(),
        };
        let right_delta = RelationDelta {
            inserted: vec![vec![Value::Text("alpha".into()), Value::I64(20)]],
            removed: Vec::new(),
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::Text("ALPHA".into()), Value::I64(2)]);
        next.relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(20)]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );

        old = next;
        let left_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::Text("alpha".into()), Value::I64(2)]],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::Text("A".into()), Value::I64(10)]],
            result_type: right_type,
        };
        let mut next = old.clone();
        next.relations.get_mut(&left).unwrap().remove(1);
        next.relations.get_mut(&right).unwrap().remove(0);
        let oracle =
            rel_delta_by_recompute(&query, &old, &Change::Replace(next), &context, &registry)
                .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn owned_join_group_top_k_tree_matches_recompute_across_leaf_deltas() {
        let left = SemanticId::new(9720);
        let right = SemanticId::new(9721);
        let i64_eq = SemanticId::new(9722);
        let i64_order = SemanticId::new(9723);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9720));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9720));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let group = RelExpr::Group {
            input: Box::new(join),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(group),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 1,
        };
        let model_for = |left_rows: Vec<Row>, right_rows: Vec<Row>| {
            let mut model = FiniteModel::default();
            model.relations.insert(left, left_rows);
            model.relations.insert(right, right_rows);
            model
        };
        let mut old = model_for(
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
            ],
        );
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let mut state = MaterializedJoinGroupTopKState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let group_query = match &query {
            RelExpr::TopKWithTies { input, .. } => input.as_ref(),
            _ => unreachable!(),
        };
        let join_query = match group_query {
            RelExpr::Group { input, .. } => input.as_ref(),
            _ => unreachable!(),
        };
        assert_eq!(
            state.join,
            MaterializedJoinDeltaState::build(join_query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            state.group,
            MaterializedGroupDeltaState::build(group_query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            state.top_k,
            MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        let steps = [
            (
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(11)]],
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: Vec::new(),
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
            (
                RelationDelta {
                    inserted: Vec::new(),
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(101)]],
                    removed: vec![vec![Value::I64(2), Value::I64(200)]],
                    result_type: right_type.clone(),
                },
            ),
            (
                RelationDelta {
                    inserted: Vec::new(),
                    removed: vec![vec![Value::I64(1), Value::I64(10)]],
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: vec![vec![Value::I64(2), Value::I64(201)]],
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
        ];
        for (left_delta, right_delta) in steps {
            let old_left =
                RelationValue::Bag(old.relations.get(&left).cloned().unwrap_or_default());
            let old_right =
                RelationValue::Bag(old.relations.get(&right).cloned().unwrap_or_default());
            let next_left =
                apply_relation_delta_to_value(old_left, &left_delta, &context, &registry).unwrap();
            let next_right =
                apply_relation_delta_to_value(old_right, &right_delta, &context, &registry)
                    .unwrap();
            let next = model_for(next_left.into_rows(), next_right.into_rows());
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            let maintained = state
                .apply_join_input_deltas(&left_delta, &right_delta, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn recursive_maintained_plan_owns_nontrivial_join_group_top_k_tree() {
        let left = SemanticId::new(9730);
        let right = SemanticId::new(9731);
        let i64_eq = SemanticId::new(9732);
        let i64_order = SemanticId::new(9733);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9730));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9730));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let left_filtered = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(left)),
            column: 1,
            value: Value::I64(1),
            equivalence: i64_eq,
        };
        let right_projected = RelExpr::Project {
            input: Box::new(RelExpr::Scan(right)),
            columns: vec![0],
        };
        let joined = RelExpr::JoinEq {
            left: Box::new(left_filtered),
            right: Box::new(right_projected),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let projected = RelExpr::Project {
            input: Box::new(joined),
            columns: vec![0],
        };
        let grouped = RelExpr::Group {
            input: Box::new(projected),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(grouped),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 1,
        };
        let model_for = |left_rows: Vec<Row>, right_rows: Vec<Row>| {
            let mut model = FiniteModel::default();
            model.relations.insert(left, left_rows);
            model.relations.insert(right, right_rows);
            model
        };
        let mut old = model_for(
            vec![
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(2), Value::I64(1)],
                vec![Value::I64(3), Value::I64(0)],
            ],
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
        );
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&old, &context, &registry).unwrap()
        );
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();

        let mut resolved_state = state.clone();
        let left_bindings = old.relations[&left]
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, row)| {
                (
                    kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    },
                    row,
                )
            })
            .collect::<Vec<_>>();
        resolved_state
            .attach_storage_rows(left, &left_bindings)
            .unwrap();
        let right_bindings = old.relations[&right]
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, row)| {
                (
                    kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    },
                    row,
                )
            })
            .collect::<Vec<_>>();
        resolved_state
            .attach_storage_rows(right, &right_bindings)
            .unwrap();
        let resolved_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(1)]],
            removed: vec![vec![Value::I64(2), Value::I64(1)]],
            result_type: left_type.clone(),
        };
        let resolved = StorageResolvedRelationDelta::from_parts(
            left,
            resolved_delta.clone(),
            vec![kernel_types::StableRowHandle {
                slot: 1,
                generation: 0,
            }],
            vec![kernel_types::StableRowHandle {
                slot: 1,
                generation: 1,
            }],
        );
        let mut resolved_next = old.clone();
        resolved_next.relations.get_mut(&left).unwrap().remove(1);
        resolved_next
            .relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::I64(1), Value::I64(1)]);
        let resolved_oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(resolved_next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let mut resolved_map = BTreeMap::new();
        resolved_map.insert(left, resolved);
        reset_relation_delta_materialization_count();
        let resolved_output = resolved_state
            .apply_storage_resolved_deltas(&resolved_map, &context, &registry)
            .unwrap();
        assert_eq!(relation_delta_materialization_count(), 1);
        assert!(
            relation_deltas_semantically_equivalent(
                &resolved_output,
                &resolved_oracle,
                &context,
                &registry,
            )
            .unwrap()
        );
        assert_eq!(
            resolved_state.output_value(&context, &registry).unwrap(),
            query.evaluate(&resolved_next, &context, &registry).unwrap()
        );

        let steps = [
            (
                left,
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(1)]],
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
            ),
            (
                right,
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(11)]],
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
            (
                left,
                RelationDelta {
                    inserted: Vec::new(),
                    removed: vec![vec![Value::I64(2), Value::I64(1)]],
                    result_type: left_type.clone(),
                },
            ),
        ];
        for (relation, delta) in steps {
            let mut deltas = BTreeMap::new();
            deltas.insert(relation, delta.clone());
            let mut next = old.clone();
            let old_value =
                RelationValue::Bag(next.relations.get(&relation).cloned().unwrap_or_default());
            next.relations.insert(
                relation,
                apply_relation_delta_to_value(old_value, &delta, &context, &registry)
                    .unwrap()
                    .into_rows(),
            );
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            reset_relation_delta_materialization_count();
            let maintained = state
                .apply_relation_deltas(&deltas, &context, &registry)
                .unwrap();
            assert_eq!(relation_delta_materialization_count(), 1);
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            assert_eq!(
                state.output_value(&context, &registry).unwrap(),
                query.evaluate(&next, &context, &registry).unwrap()
            );
            old = next;
        }

        let before = state.clone();
        let mut invalid = BTreeMap::new();
        invalid.insert(
            left,
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(999), Value::I64(1)]],
                result_type: left_type,
            },
        );
        reset_relation_delta_materialization_count();
        assert_eq!(
            state.apply_relation_deltas(&invalid, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(relation_delta_materialization_count(), 0);
        assert_eq!(state, before);
    }

    #[test]
    fn recursive_maintained_plan_supports_distinct_and_promote_to_bag() {
        let relation = SemanticId::new(9740);
        let i64_eq = SemanticId::new(9741);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9740));
        environment.pin_module(i64_eq, eq_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9740));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 1,
                    value: Value::I64(1),
                    equivalence: i64_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![i64_eq],
        }));
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(2), Value::I64(0)],
            ],
        );
        let relation_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let deltas = [
            RelationDelta {
                inserted: vec![vec![Value::I64(1), Value::I64(1)]],
                removed: Vec::new(),
                result_type: relation_type.clone(),
            },
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(1), Value::I64(1)]],
                result_type: relation_type.clone(),
            },
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(1), Value::I64(1)]],
                result_type: relation_type,
            },
        ];
        for delta in deltas {
            let mut leaf = BTreeMap::new();
            leaf.insert(relation, delta.clone());
            let old_value =
                RelationValue::Bag(old.relations.get(&relation).cloned().unwrap_or_default());
            let next_value =
                apply_relation_delta_to_value(old_value, &delta, &context, &registry).unwrap();
            let mut next = old.clone();
            next.relations.insert(relation, next_value.into_rows());
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            let maintained = state
                .apply_relation_deltas(&leaf, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            assert_eq!(
                state.output_value(&context, &registry).unwrap(),
                query.evaluate(&next, &context, &registry).unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn storage_resolved_removal_rejects_handle_payload_mismatch() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let mut model = FiniteModel::default();
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let beta = vec![Value::Text("Beta".into()), Value::I64(2)];
        model
            .relations
            .insert(relation, vec![alpha.clone(), beta.clone()]);

        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let alpha_handle = kernel_types::StableRowHandle {
            slot: 0,
            generation: 0,
        };
        let beta_handle = kernel_types::StableRowHandle {
            slot: 1,
            generation: 0,
        };
        state
            .attach_storage_rows(
                relation,
                &[(alpha_handle, alpha.clone()), (beta_handle, beta.clone())],
            )
            .unwrap();

        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![beta],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let forged = StorageResolvedRelationDelta::from_parts(
            relation,
            delta,
            vec![alpha_handle],
            Vec::new(),
        );
        let map = BTreeMap::from([(relation, forged)]);

        assert_eq!(
            state.apply_storage_resolved_deltas(&map, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
    }

    #[test]
    fn attach_storage_rows_validates_before_cow_commit_and_is_failure_atomic() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let beta = vec![Value::Text("Beta".into()), Value::I64(2)];
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![alpha.clone(), beta.clone()]);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let before = state.clone();
        let epoch = state.transition_epoch();
        let shared_arena = Arc::clone(&state.arena);
        let handles = [
            kernel_types::StableRowHandle {
                slot: 0,
                generation: 0,
            },
            kernel_types::StableRowHandle {
                slot: 1,
                generation: 0,
            },
        ];

        assert_eq!(
            state.attach_storage_rows(
                relation,
                &[(handles[0], beta.clone()), (handles[1], alpha.clone())],
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
        assert_eq!(state.transition_epoch(), epoch);
        assert!(Arc::ptr_eq(&state.arena, &shared_arena));

        state
            .attach_storage_rows(relation, &[(handles[0], alpha), (handles[1], beta)])
            .unwrap();
        assert_eq!(state.transition_epoch(), epoch + 1);
        assert!(!Arc::ptr_eq(&state.arena, &shared_arena));
        assert_eq!(
            before.output_value(&context, &registry).unwrap(),
            state.output_value(&context, &registry).unwrap()
        );
    }

    #[test]
    fn composite_primitive_group_uses_semantic_index_and_matches_recompute() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0, 1],
            group_equivalences: vec![text_eq, i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(1)],
                vec![Value::Text("B".into()), Value::I64(2)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.semantic_lookup.is_some());
        assert_eq!(state.group_encoders.as_ref().map(Vec::len), Some(2));
        assert_eq!(state.group_count(), 2);

        let mut next = old.clone();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("b".into()), Value::I64(2)],
                vec![Value::Text("C".into()), Value::I64(3)],
            ],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(state.group_count(), 3);
    }

    #[test]
    fn structural_and_primitive_group_uses_canonical_lookup_and_matches_recompute() {
        let relation = SemanticId::new(98_150);
        let text_eq = SemanticId::new(98_151);
        let set_eq = SemanticId::new(98_152);
        let i64_eq = SemanticId::new(98_153);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_150));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_150));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Set {
                        element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                        equivalence: text_eq,
                    },
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![set_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0, 1],
            group_equivalences: vec![set_eq, i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![set(&["A", "B"]), Value::I64(1)],
                vec![set(&["b", "a"]), Value::I64(1)],
                vec![set(&["C"]), Value::I64(3)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.semantic_lookup.is_some());
        assert!(state.group_encoders.is_none());
        assert!(state.canonical_group_lookup);
        assert_eq!(state.group_count(), 2);

        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![set(&["B", "A"]), Value::I64(1)],
                vec![set(&["c"]), Value::I64(3)],
                vec![set(&["D", "E"]), Value::I64(4)],
            ],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(state.group_count(), 3);
    }

    #[test]
    fn materialized_f64_total_top_k_uses_order_index_across_hostile_values() {
        let relation = SemanticId::new(98_100);
        let f64_eq = SemanticId::new(98_101);
        let f64_order = SemanticId::new(98_102);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let order_digest = registry.install_ordering(OrderingModule::F64Total);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_100));
        environment.pin_module(f64_eq, eq_digest);
        environment.pin_module(f64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_100));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: f64_order,
            direction: OrderDirection::Ascending,
            k: 3,
        };
        let model_for = |bits: &[u64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                bits.iter()
                    .copied()
                    .map(|bits| vec![Value::F64Bits(bits)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![
                f64::NEG_INFINITY.to_bits(),
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
                1.0_f64.to_bits(),
                0x7ff8_0000_0000_0001,
                0xfff8_0000_0000_0001,
            ],
            vec![
                0xfff8_0000_0000_0002,
                (-1.0_f64).to_bits(),
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
                f64::INFINITY.to_bits(),
                0x7ff8_0000_0000_0002,
            ],
            vec![
                f64::NEG_INFINITY.to_bits(),
                f64::INFINITY.to_bits(),
                5e-324_f64.to_bits(),
                (-5e-324_f64).to_bits(),
            ],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(
            state.storage,
            MaintainedTopKStorage::SemanticOrdered { .. }
        ));
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn semantic_indexed_join_accepts_same_call_remove_insert_replacement() {
        let (context, registry, text_eq, left_relation, right_relation) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            left_relation,
            vec![vec![Value::Text("key-0".into()), Value::I64(0)]],
        );
        model.relations.insert(
            right_relation,
            (0..1000)
                .map(|i| vec![Value::Text(format!("KEY-{i}")), Value::I64(i)])
                .collect(),
        );
        let mut state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let left_type = RelExpr::Scan(left_relation)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_relation)
            .typecheck(&context, &registry)
            .unwrap();
        let left_delta = RelationDelta {
            removed: vec![vec![Value::Text("key-0".into()), Value::I64(0)]],
            inserted: vec![vec![Value::Text("KEY-0".into()), Value::I64(1)]],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            removed: Vec::new(),
            inserted: Vec::new(),
            result_type: right_type,
        };
        assert!(
            state
                .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
                .is_ok()
        );
    }

    #[test]
    fn structural_join_equivalence_uses_canonical_index_and_delta_matches_oracle() {
        let left = SemanticId::new(98_200);
        let right = SemanticId::new(98_201);
        let product_eq = SemanticId::new(98_202);
        let text_eq = SemanticId::new(98_203);
        let i64_eq = SemanticId::new(98_204);
        let field = SemanticId::new(98_205);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_200));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let product_type = TypeExpr::Product(BTreeMap::from([(
            field,
            TypeExpr::Scalar(ScalarType::Text),
        )]));
        let mut schema = Schema::new(SchemaRevisionId::new(98_200));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, text_eq)]),
                },
            )
            .unwrap();
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![product_type.clone(), TypeExpr::Scalar(ScalarType::I64)],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![product_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let product =
            |label: &str| Value::Product(BTreeMap::from([(field, Value::Text(label.into()))]));
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(left, vec![vec![product("Alpha"), Value::I64(1)]]);
        model
            .relations
            .insert(right, vec![vec![product("alpha"), Value::I64(2)]]);
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: product_eq,
        };
        let state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(matches!(
            state.storage,
            MaintainedJoinStorage::StructuralIndexed(_)
        ));
        let mut state = state;
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![product("BETA"), Value::I64(3)]],
            removed: Vec::new(),
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: vec![
                vec![product("beta"), Value::I64(4)],
                vec![product("ALPHA"), Value::I64(5)],
            ],
            removed: Vec::new(),
            result_type: right_type,
        };
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();

        let mut next = model.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .push(vec![product("BETA"), Value::I64(3)]);
        next.relations.get_mut(&right).unwrap().extend([
            vec![product("beta"), Value::I64(4)],
            vec![product("ALPHA"), Value::I64(5)],
        ]);
        let oracle =
            rel_delta_by_recompute(&query, &model, &Change::Replace(next), &context, &registry)
                .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
    }
}

#[cfg(test)]
mod positive_recursive_query_tests {
    use super::*;

    fn bag_type() -> RelType {
        RelType {
            columns: vec![kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::I64,
            )],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![],
            },
        }
    }

    fn empty_context() -> (
        kernel_schema::SemanticContext,
        kernel_semantics::SemanticRegistry,
    ) {
        (
            kernel_schema::SemanticContext {
                schema: kernel_schema::Schema::new(kernel_types::SchemaRevisionId::new(1)),
                environment: kernel_schema::SemanticEnvironment::new(
                    kernel_types::SemanticEnvId::new(1),
                ),
            },
            kernel_semantics::SemanticRegistry::default(),
        )
    }

    #[test]
    fn fixpoint_call_returns_compact_exact_finite_bag_weights() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(1)],
                    seed_multiplicity: 2,
                },
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(2)],
                    seed_multiplicity: 0,
                },
            ],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![0, 0],
                head: 1,
                coefficient: 3,
            }],
        };
        let (context, registry) = empty_context();
        let result = call.evaluate_compact(&context, &registry).unwrap();
        assert_eq!(result.entries().len(), 2);
        assert_eq!(
            result.entries()[1].1,
            kernel_fixpoint::NaturalInfinity::finite_u64(12)
        );
        assert!(result.require_finite().is_ok());
    }

    #[test]
    fn fixpoint_call_reports_nonfinite_without_expanding_rows() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![PositiveRecursiveRowAtom {
                row: vec![Value::I64(1)],
                seed_multiplicity: 1,
            }],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![0],
                head: 0,
                coefficient: 1,
            }],
        };
        let (context, registry) = empty_context();
        let result = call.evaluate_compact(&context, &registry).unwrap();
        assert_eq!(result.entries().len(), 1);
        assert!(result.entries()[0].1.is_infinite());
        assert_eq!(
            result.require_finite(),
            Err(RelQueryError::NonFiniteRecursiveMultiplicity)
        );
    }

    #[test]
    fn hostile_fixpoint_call_rejects_atom_outside_finite_carrier() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![PositiveRecursiveRowAtom {
                row: vec![Value::I64(1)],
                seed_multiplicity: 1,
            }],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![7],
                head: 0,
                coefficient: 1,
            }],
        };
        let (context, registry) = empty_context();
        assert_eq!(
            call.evaluate_compact(&context, &registry),
            Err(RelQueryError::RecursiveAtomOutsideCarrier)
        );
    }
}
