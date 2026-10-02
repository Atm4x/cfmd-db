use std::collections::{BTreeMap, BTreeSet};

use super::{
    RelExpr, RelQueryError, RelType, RelationOccurrenceCertificate, RelationScanOccurrenceSeed, RelationValue,
    rel_eval::{evaluate_prepared_expr, evaluate_prepared_expr_with_occurrence_certificate,
        evaluate_prepared_expr_with_occurrence_certificate_seeded, evaluate_prepared_expr_seeded},
};

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

    #[must_use]
    pub const fn expression(&self) -> &RelExpr {
        &self.expr
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

    pub fn evaluate_with_occurrence_certificate(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(RelationValue, RelationOccurrenceCertificate), RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        evaluate_prepared_expr_with_occurrence_certificate(
            &self.expr,
            model,
            context,
            registry,
            &self.compiled_orderings,
        )
    }

    pub fn evaluate_with_occurrence_certificate_seeded(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scan_seeds: &BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>,
    ) -> Result<(RelationValue, RelationOccurrenceCertificate), RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        evaluate_prepared_expr_with_occurrence_certificate_seeded(
            &self.expr,
            model,
            context,
            registry,
            &self.compiled_orderings,
            scan_seeds,
        )
    }

    pub fn evaluate_seeded(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scan_seeds: &BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>,
    ) -> Result<RelationValue, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        evaluate_prepared_expr_seeded(
            &self.expr,
            model,
            context,
            registry,
            &self.compiled_orderings,
            scan_seeds,
        )
    }

    #[must_use]
    pub fn scan_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        self.expr.scan_relations()
    }

    pub fn read_footprint(&self) -> Result<crate::RelReadFootprint, RelQueryError> {
        crate::access::read_footprint(&self.expr, &self.semantic_context, self.result_type.columns.len())
    }

    #[must_use]
    pub fn emits_occurrence_certificate_with_scan_seeds(
        &self,
        seeded_relations: &std::collections::BTreeSet<kernel_types::SemanticId>,
    ) -> bool {
        matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) && expr_emits_occurrence_certificate_with_scan_seeds(&self.expr, seeded_relations)
    }

    #[must_use]
    pub fn emits_occurrence_certificate(&self) -> bool {
        matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) && expr_emits_occurrence_certificate(&self.expr)
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

fn expr_emits_occurrence_certificate(expr: &RelExpr) -> bool {
    match expr {
        RelExpr::Union { .. }
        | RelExpr::Difference { .. }
        | RelExpr::Distinct { .. }
        | RelExpr::Project { .. } => true,
        RelExpr::AntiJoin { left, .. } => expr_emits_occurrence_certificate(left),
        RelExpr::JoinEq { left, right, .. } => {
            expr_emits_occurrence_certificate(left) && expr_emits_occurrence_certificate(right)
        }
        RelExpr::Scan(_)
        | RelExpr::FilterEqConst { .. }
        | RelExpr::FilterOrderConst { .. }
        | RelExpr::FilterEqColumns { .. }
        | RelExpr::Group { .. }
        | RelExpr::TopKWithTies { .. }
        | RelExpr::PromoteToBag(_) => false,
    }
}

fn expr_emits_occurrence_certificate_with_scan_seeds(
    expr: &RelExpr,
    seeded_relations: &std::collections::BTreeSet<kernel_types::SemanticId>,
) -> bool {
    match expr {
        RelExpr::Scan(relation) => seeded_relations.contains(relation),
        RelExpr::Union { .. }
        | RelExpr::Difference { .. }
        | RelExpr::Distinct { .. }
        | RelExpr::Project { .. } => true,
        RelExpr::AntiJoin { left, .. } => {
            expr_emits_occurrence_certificate_with_scan_seeds(left, seeded_relations)
        }
        RelExpr::JoinEq { left, right, .. } => {
            expr_emits_occurrence_certificate_with_scan_seeds(left, seeded_relations)
                && expr_emits_occurrence_certificate_with_scan_seeds(right, seeded_relations)
        }
        RelExpr::FilterEqConst { .. }
        | RelExpr::FilterOrderConst { .. }
        | RelExpr::FilterEqColumns { .. }
        | RelExpr::Group { .. }
        | RelExpr::TopKWithTies { .. }
        | RelExpr::PromoteToBag(_) => false,
    }
}
