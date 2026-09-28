use super::{
    MaterializedGroupDeltaState, MaterializedJoinDeltaState, MaterializedTopKDeltaState, RelExpr,
    RelQueryError, RelationDelta,
};

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
        if !group.supports_join_group_topk_composition() {
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

    #[cfg(test)]
    pub(super) fn test_states(
        &self,
    ) -> (
        &MaterializedJoinDeltaState,
        &MaterializedGroupDeltaState,
        &MaterializedTopKDeltaState,
    ) {
        (&self.join, &self.group, &self.top_k)
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
