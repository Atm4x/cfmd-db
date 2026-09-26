use super::{
    I64IndexBinding, LayoutBinding, PhysicalExecutionError, PhysicalStore, Plan,
    SemanticAccessCostModel, SemanticId, SemanticIndexBinding, Value,
};
use crate::native_relation::native_row_count;
use crate::plan::transparent_direct_scan_relation;

// HOSTILE[P179][ACTIVE][CLEAN]: join-access admission/cost policy is a neutral planning
// capability shared by direct and multiway execution; it is not executor representation.
#[derive(Debug, Clone, Copy)]
pub(super) struct JoinKeySpec {
    pub(super) left_column: usize,
    pub(super) right_column: usize,
    pub(super) equivalence: SemanticId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JoinAccessFamily {
    FullScan,
    PersistedI64,
    PersistedSemantic,
    EphemeralI64,
}

impl JoinAccessFamily {
    const fn tie_priority(self) -> u8 {
        match self {
            Self::FullScan => 0,
            Self::PersistedI64 => 1,
            Self::PersistedSemantic => 2,
            Self::EphemeralI64 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// HOSTILE[P161][ACTIVE][CLEAN:P161.E]: access selection is pre-build admission; an admitted
// ephemeral artifact is consumed once built instead of being re-costed and discarded.
pub(super) struct JoinAccessDecision {
    pub(super) family: JoinAccessFamily,
    pub(super) work: usize,
    pub(super) estimated_output_rows: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RightJoinAccessRequest {
    pub(super) left_rows: usize,
    pub(super) right_relation: SemanticId,
    pub(super) right_layout: LayoutBinding,
    pub(super) right_column: usize,
    pub(super) equivalence: SemanticId,
    pub(super) allow_ephemeral: bool,
}

fn consider_join_access_candidate(best: &mut JoinAccessDecision, candidate: JoinAccessDecision) {
    if candidate.work < best.work
        || (candidate.work == best.work
            && candidate.family.tie_priority() < best.family.tie_priority())
    {
        *best = candidate;
    }
}

#[derive(Debug, Clone, Copy)]
struct TransientJoinEstimate {
    left_rows: usize,
    right_rows: usize,
    distinct_keys: usize,
    is_i64: bool,
}

fn consider_ephemeral_join_candidates(
    best: &mut JoinAccessDecision,
    estimate: TransientJoinEstimate,
) {
    let TransientJoinEstimate {
        left_rows,
        right_rows,
        distinct_keys,
        is_i64,
    } = estimate;
    let estimated_output_rows =
        SemanticAccessCostModel::estimated_join_output_rows(left_rows, right_rows, distinct_keys);
    if is_i64 {
        consider_join_access_candidate(
            best,
            JoinAccessDecision {
                family: JoinAccessFamily::EphemeralI64,
                work: SemanticAccessCostModel::ephemeral_i64_join_access_work(
                    left_rows, right_rows,
                ),
                estimated_output_rows,
            },
        );
    }
}

pub(super) fn right_scan_join_access_decision(
    request: RightJoinAccessRequest,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<JoinAccessDecision, PhysicalExecutionError> {
    let RightJoinAccessRequest {
        left_rows,
        right_relation,
        right_layout,
        right_column,
        equivalence,
        allow_ephemeral,
    } = request;
    let right = store.installed(right_relation, right_layout)?;
    let right_rows = native_row_count(&right.data);
    let mut best = JoinAccessDecision {
        family: JoinAccessFamily::FullScan,
        work: SemanticAccessCostModel::canonical_bucket_join_access_work(left_rows, right_rows, 1),
        estimated_output_rows: left_rows.saturating_mul(right_rows),
    };
    if left_rows == 0 || right_rows == 0 {
        return Ok(best);
    }

    let resolved = registry.resolve_primitive_equivalence(context, equivalence)?;
    let is_i64 = resolved.as_ref().is_some_and(|resolved| {
        matches!(
            resolved.bind_right(&Value::I64(0)),
            Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
        )
    });

    if is_i64
        && let Some(index) = store.i64_index_capability(I64IndexBinding {
            relation: right_relation,
            layout: right_layout,
            key_column: right_column,
            equivalence,
        })
    {
        let distinct = index.distinct_key_count();
        consider_join_access_candidate(
            &mut best,
            JoinAccessDecision {
                family: JoinAccessFamily::PersistedI64,
                work: SemanticAccessCostModel::persisted_join_access_work(left_rows, 1),
                estimated_output_rows: SemanticAccessCostModel::estimated_join_output_rows(
                    left_rows, right_rows, distinct,
                ),
            },
        );
    }

    let binding =
        SemanticIndexBinding::single(right_relation, right_layout, right_column, equivalence);
    let retained_statistics = store.semantic_statistics(&binding, context, registry)?;
    let transient_distinct = retained_statistics.map_or(right_rows, |statistics| {
        statistics.distinct_key_count.max(1)
    });
    if let Some(capability) = store.semantic_fiber_capability(&binding, context, registry)? {
        let distinct = capability.distinct_key_count();
        consider_join_access_candidate(
            &mut best,
            JoinAccessDecision {
                family: JoinAccessFamily::PersistedSemantic,
                work: SemanticAccessCostModel::persisted_join_access_work(
                    left_rows,
                    binding.key_parts.len(),
                ),
                estimated_output_rows: SemanticAccessCostModel::estimated_join_output_rows(
                    left_rows, right_rows, distinct,
                ),
            },
        );
    }

    if allow_ephemeral {
        consider_ephemeral_join_candidates(
            &mut best,
            TransientJoinEstimate {
                left_rows,
                right_rows,
                distinct_keys: transient_distinct,
                is_i64,
            },
        );
    }
    Ok(best)
}

pub(super) fn direct_join_access_decision(
    left: &Plan,
    right: &Plan,
    key: JoinKeySpec,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<JoinAccessDecision>, PhysicalExecutionError> {
    let Some((left_relation, left_layout)) = transparent_direct_scan_relation(left, store) else {
        return Ok(None);
    };
    let Some((right_relation, right_layout)) = transparent_direct_scan_relation(right, store)
    else {
        return Ok(None);
    };
    let left_rows = native_row_count(&store.installed(left_relation, left_layout)?.data);
    right_scan_join_access_decision(
        RightJoinAccessRequest {
            left_rows,
            right_relation,
            right_layout,
            right_column: key.right_column,
            equivalence: key.equivalence,
            allow_ephemeral: true,
        },
        store,
        context,
        registry,
    )
    .map(Some)
}

// HOSTILE[P179][TEST-ONLY][CLEAN]: root integration tests observe planner behavior without
// constructing or matching production join-access representation.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum JoinAccessKind {
        FullScan,
        PersistedI64,
        PersistedSemantic,
        EphemeralI64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct JoinAccessObservation {
        pub(crate) family: JoinAccessKind,
        pub(crate) work: usize,
        pub(crate) estimated_output_rows: usize,
    }

    #[derive(Debug, Clone, Copy)]
    pub(crate) struct JoinAccessProbe {
        pub(crate) left_rows: usize,
        pub(crate) right_relation: SemanticId,
        pub(crate) right_layout: LayoutBinding,
        pub(crate) right_column: usize,
        pub(crate) equivalence: SemanticId,
        pub(crate) allow_ephemeral: bool,
    }

    pub(crate) fn observe_join_access(decision: JoinAccessDecision) -> JoinAccessObservation {
        JoinAccessObservation {
            family: match decision.family {
                JoinAccessFamily::FullScan => JoinAccessKind::FullScan,
                JoinAccessFamily::PersistedI64 => JoinAccessKind::PersistedI64,
                JoinAccessFamily::PersistedSemantic => JoinAccessKind::PersistedSemantic,
                JoinAccessFamily::EphemeralI64 => JoinAccessKind::EphemeralI64,
            },
            work: decision.work,
            estimated_output_rows: decision.estimated_output_rows,
        }
    }

    pub(crate) fn observe_right_join_access_for_test(
        probe: JoinAccessProbe,
        store: &PhysicalStore,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<JoinAccessObservation, PhysicalExecutionError> {
        right_scan_join_access_decision(
            RightJoinAccessRequest {
                left_rows: probe.left_rows,
                right_relation: probe.right_relation,
                right_layout: probe.right_layout,
                right_column: probe.right_column,
                equivalence: probe.equivalence,
                allow_ephemeral: probe.allow_ephemeral,
            },
            store,
            context,
            registry,
        )
        .map(observe_join_access)
    }
}
