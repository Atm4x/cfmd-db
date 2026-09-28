use std::collections::BTreeMap;

use super::{RelExpr, RelQueryError, RelType, RelationValue, rel_eval::evaluate_prepared_expr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelExpr {
    expr: RelExpr,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    compiled_orderings: BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
}

impl PreparedRelExpr {
    pub(super) fn new(
        expr: RelExpr,
        result_type: RelType,
        semantic_context: kernel_schema::SemanticContext,
        compiled_orderings: BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
    ) -> Self {
        Self {
            expr,
            result_type,
            semantic_context,
            compiled_orderings,
        }
    }

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
        evaluate_prepared_expr(
            &self.expr,
            model,
            context,
            registry,
            &self.compiled_orderings,
        )
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
