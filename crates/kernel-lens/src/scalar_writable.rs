use kernel_change::PreparedRewrite;
#[cfg(test)]
use kernel_change::{FineChange, FineChangeKind, RewriteEffect};
use kernel_model::Value;
#[cfg(test)]
use kernel_types::SemanticId;

#[cfg(test)]
use std::collections::BTreeMap;

use crate::{LensError, LensExpr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritabilityFailure {
    ConstantHasNoSourceOwner,
    DerivedOperatorHasNoCertifiedLift,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableCompilation {
    Writable(WritableViewPlan),
    ReadOnly(WritabilityFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableViewPlan {
    pub query: kernel_query::ExactQuery,
    pub rewrite_spec: kernel_change::RewriteSpecId,
    pub lens: LensExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableExecutionError {
    RewriteFamilyMismatch {
        expected: kernel_change::RewriteSpecId,
        actual: kernel_change::RewriteSpecId,
    },
    Lens(LensError),
}

impl From<LensError> for WritableExecutionError {
    fn from(value: LensError) -> Self {
        Self::Lens(value)
    }
}

impl WritableViewPlan {
    pub fn lift_rewrite<I: Clone>(
        &self,
        source: &Value,
        rewrite: &PreparedRewrite<Value, I>,
    ) -> Result<PreparedRewrite<Value, I>, WritableExecutionError> {
        if rewrite.spec != self.rewrite_spec {
            return Err(WritableExecutionError::RewriteFamilyMismatch {
                expected: self.rewrite_spec,
                actual: rewrite.spec,
            });
        }
        self.lens.lift_rewrite(source, rewrite).map_err(Into::into)
    }
}

/// Structural writability compiler for the first certified fragment.
///
/// It intentionally accepts only identity and product-field projection chains.
/// Derived arithmetic/branching/aggregate operators remain read-only until an
/// explicit certified lift is provided; the compiler never selects an
/// arbitrary source preimage.
#[must_use]
pub fn compile_writable_query(
    query: &kernel_query::ExactQuery,
    rewrite_spec: kernel_change::RewriteSpecId,
) -> WritableCompilation {
    match compile_writable_expr(query.root()) {
        Ok(lens) => WritableCompilation::Writable(WritableViewPlan {
            query: query.clone(),
            rewrite_spec,
            lens,
        }),
        Err(reason) => WritableCompilation::ReadOnly(reason),
    }
}

fn compile_writable_expr(expr: &kernel_query::Expr) -> Result<LensExpr, WritabilityFailure> {
    match expr {
        kernel_query::Expr::Input => Ok(LensExpr::Identity),
        kernel_query::Expr::ProductField { input, field } => {
            let outer = compile_writable_expr(input)?;
            Ok(LensExpr::Compose(
                Box::new(outer),
                Box::new(LensExpr::ProductField(*field)),
            ))
        }
        kernel_query::Expr::Const(_) | kernel_query::Expr::TypedConst { .. } => {
            Err(WritabilityFailure::ConstantHasNoSourceOwner)
        }
        kernel_query::Expr::SeqLength(_)
        | kernel_query::Expr::SeqSumI64(_)
        | kernel_query::Expr::AddI64(_, _)
        | kernel_query::Expr::If { .. } => {
            Err(WritabilityFailure::DerivedOperatorHasNoCertifiedLift)
        }
    }
}

#[cfg(test)]
mod writable_tests {
    use super::*;
    use kernel_change::{RewriteLawSetId, RewriteSpecId};
    use kernel_query::{ExactQuery, Expr};

    #[test]
    fn identity_and_product_field_chain_compile_to_writable_plan() {
        let query = ExactQuery::new(Expr::ProductField {
            input: Box::new(Expr::Input),
            field: SemanticId(2),
        });
        let spec = RewriteSpecId(SemanticId(200));
        let WritableCompilation::Writable(plan) = compile_writable_query(&query, spec) else {
            panic!("product field must be structurally writable");
        };
        assert_eq!(plan.rewrite_spec, spec);
        let source = source_for_writable_test();
        let rewrite: PreparedRewrite<Value, Value> = PreparedRewrite {
            spec,
            explicit_inputs: vec![],
            effect: RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, Value::I64(99))),
            law_set: RewriteLawSetId(SemanticId(201)),
        };
        let lifted = plan.lift_rewrite(&source, &rewrite).unwrap();
        let updated = lifted.apply(&source);
        assert_eq!(query.evaluate(&updated), Ok(Value::I64(99)));
    }

    #[test]
    fn derived_query_is_read_only_instead_of_guessing_a_preimage() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        assert_eq!(
            compile_writable_query(&query, RewriteSpecId(SemanticId(210))),
            WritableCompilation::ReadOnly(WritabilityFailure::DerivedOperatorHasNoCertifiedLift)
        );
    }

    #[test]
    fn writable_plan_rejects_wrong_rewrite_family() {
        let query = ExactQuery::new(Expr::Input);
        let WritableCompilation::Writable(plan) =
            compile_writable_query(&query, RewriteSpecId(SemanticId(220)))
        else {
            unreachable!();
        };
        let source = Value::I64(1);
        let rewrite = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(221)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(Value::I64(2)),
            law_set: RewriteLawSetId(SemanticId(222)),
        };
        assert!(matches!(
            plan.lift_rewrite(&source, &rewrite),
            Err(WritableExecutionError::RewriteFamilyMismatch { .. })
        ));
    }

    fn source_for_writable_test() -> Value {
        Value::Product(BTreeMap::from([
            (SemanticId(1), Value::I64(10)),
            (SemanticId(2), Value::I64(20)),
            (SemanticId(3), Value::I64(30)),
        ]))
    }
}
