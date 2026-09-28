use std::collections::{BTreeMap, BTreeSet};

use crate::rewrite::{
    CompiledRewriteCoordinationGraph, PairCoordinationDecision, PreparedRewrite,
    PreparedRewriteCoordination, RewriteCoordinationRegistry, RewriteCoordinationRegistryError,
    RewriteResidualCubeWitness, RewriteResidualFamilyRegistry, RewriteResidualPairResolver,
    RewriteResidualResolutionAuthority, RewriteSequentialPairResolver, SharedPreparedRewrite,
};

use super::super::{RevisionEffect, RevisionEffectId, RevisionEffectIdeal};
use super::{
    RevisionEffectResidualCubeLayerCertificate, RevisionEffectResidualLayerCertificate,
    RevisionEffectResidualLayerError, RevisionEffectResidualResolvedLayerCertificate,
};

struct ResidualFrontier<'a, T, I> {
    common: BTreeSet<RevisionEffectId>,
    left: Vec<&'a RevisionEffect<SharedPreparedRewrite<T, I>>>,
    right: Vec<&'a RevisionEffect<SharedPreparedRewrite<T, I>>>,
}

impl<T: PartialEq + Eq, I: PartialEq + Eq> RevisionEffectIdeal<SharedPreparedRewrite<T, I>> {
    fn residual_frontier<'a>(
        &'a self,
        other: &'a Self,
    ) -> Result<ResidualFrontier<'a, T, I>, RevisionEffectResidualLayerError> {
        let common = self.common_ideal(other)?;
        let common_ids = common.events.keys().copied().collect::<BTreeSet<_>>();
        let left = self.exclusive_from(&common)?;
        let right = other.exclusive_from(&common)?;
        for event in left.iter().chain(right.iter()) {
            if !event.prerequisites.is_subset(&common_ids) {
                return Err(RevisionEffectResidualLayerError::NonFrontierExclusiveEffect(event.id));
            }
        }
        Ok(ResidualFrontier {
            common: common_ids,
            left,
            right,
        })
    }
}

impl<T: PartialEq + Eq, I: PartialEq + Eq> RevisionEffectIdeal<SharedPreparedRewrite<T, I>> {
    pub fn certify_registered_residual_cube_frontier(
        &self,
        other: &Self,
        registry: &RewriteResidualFamilyRegistry,
        witness: RewriteResidualCubeWitness<T, I>,
    ) -> Result<RevisionEffectResidualCubeLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 1 || schedule.right_layers.len() != 1 {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        let left_frontier = schedule.left_layers[0].clone();
        let right_frontier = schedule.right_layers[0].clone();
        if !matches!((left_frontier.len(), right_frontier.len()), (2, 1) | (1, 2)) {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }

        let mut ordered = left_frontier
            .iter()
            .map(|id| (0_u8, *id, &self.events[id].payload))
            .chain(
                right_frontier
                    .iter()
                    .map(|id| (1_u8, *id, &other.events[id].payload)),
            )
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(branch, id, _)| (*branch, *id));
        let [(_, _, a), (_, _, b), (_, _, c)] = ordered.as_slice() else {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        };
        let cube = registry.certify_cube(a, b, c, witness)?;

        Ok(RevisionEffectResidualCubeLayerCertificate {
            common: schedule.common,
            left_frontier,
            right_frontier,
            cube,
        })
    }

    /// Certifies one concurrent branch-exclusive frontier layer against an
    /// opaque pair classifier. Every admitted exclusive event must depend only
    /// on the common ideal; deeper effects are rejected before classification.
    pub fn certify_registered_residual_frontier(
        &self,
        other: &Self,
        registry: &RewriteResidualFamilyRegistry,
        classify: impl Fn(&PreparedRewrite<T, I>, &PreparedRewrite<T, I>) -> PairCoordinationDecision,
        residuals: impl Fn(
            RevisionEffectId,
            RevisionEffectId,
        ) -> Option<(PreparedRewrite<T, I>, PreparedRewrite<T, I>)>,
    ) -> Result<RevisionEffectResidualLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let frontier = self.residual_frontier(other)?;
        let common_ids = frontier.common;
        let left = frontier.left;
        let right = frontier.right;

        let mut requires_residual = BTreeSet::new();
        let mut intent_conflicts = BTreeSet::new();
        for left_event in &left {
            for right_event in &right {
                let pair = (left_event.id, right_event.id);
                match classify(&left_event.payload, &right_event.payload) {
                    PairCoordinationDecision::CoordinationFree => {}
                    PairCoordinationDecision::RequiresCoordination => {
                        requires_residual.insert(pair);
                    }
                    PairCoordinationDecision::IntentConflict => {
                        intent_conflicts.insert(pair);
                    }
                }
            }
        }
        if !intent_conflicts.is_empty() {
            return Err(RevisionEffectResidualLayerError::IntentConflicts(
                intent_conflicts,
            ));
        }

        let left_by_id = left
            .iter()
            .map(|event| (event.id, *event))
            .collect::<BTreeMap<_, _>>();
        let right_by_id = right
            .iter()
            .map(|event| (event.id, *event))
            .collect::<BTreeMap<_, _>>();
        let mut diamonds = BTreeMap::new();
        for &(left_id, right_id) in &requires_residual {
            let left_event = left_by_id[&left_id];
            let right_event = right_by_id[&right_id];
            let (right_after_left, left_after_right) = residuals(left_id, right_id).ok_or(
                RevisionEffectResidualLayerError::MissingResidualPair(left_id, right_id),
            )?;
            let diamond = registry.certify(
                &left_event.payload,
                &right_event.payload,
                right_after_left,
                left_after_right,
            )?;
            diamonds.insert((left_id, right_id), diamond);
        }

        Ok(RevisionEffectResidualLayerCertificate {
            common: common_ids,
            left_frontier: left.into_iter().map(|event| event.id).collect(),
            right_frontier: right.into_iter().map(|event| event.id).collect(),
            diamonds,
        })
    }

    /// Certifies arbitrary-width branch frontiers through one total residual
    /// resolution authority. Both branches are normalized in deterministic
    /// `RevisionEffectId` order; no Single/Pair/Triple execution routing is used.
    pub fn certify_resolved_residual_frontier<R, S>(
        &self,
        other: &Self,
        base: &T,
        authority: &RewriteResidualResolutionAuthority<'_, R, S>,
    ) -> Result<
        RevisionEffectResidualResolvedLayerCertificate<T, I>,
        RevisionEffectResidualLayerError,
    >
    where
        R: RewriteResidualPairResolver<T, I>,
        S: RewriteSequentialPairResolver<T, I>,
    {
        let frontier = self.residual_frontier(other)?;
        let common = frontier.common;
        let left_frontier = frontier
            .left
            .iter()
            .map(|event| event.id)
            .collect::<BTreeSet<_>>();
        let right_frontier = frontier
            .right
            .iter()
            .map(|event| event.id)
            .collect::<BTreeSet<_>>();
        let left_normalization = authority.certify_finite_concurrent(
            base,
            frontier
                .left
                .iter()
                .map(|event| (event.id, event.payload.clone())),
        )?;
        let right_normalization = authority.certify_finite_concurrent(
            base,
            frontier
                .right
                .iter()
                .map(|event| (event.id, event.payload.clone())),
        )?;
        let cross = authority.resolve(
            base,
            left_normalization.composite(),
            right_normalization.composite(),
        )?;

        Ok(RevisionEffectResidualResolvedLayerCertificate {
            common,
            left_frontier,
            right_frontier,
            left_normalization,
            right_normalization,
            cross,
        })
    }

    /// Compatibility convenience that compiles the exact semantic coordination
    /// graph for this call. Hot/prepared callers should use
    /// `certify_prepared_residual_frontier` and retain the prepared graph.
    pub fn certify_compiled_residual_frontier(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        coordination_registry: &RewriteCoordinationRegistry,
        residuals: impl Fn(
            RevisionEffectId,
            RevisionEffectId,
        ) -> Option<(PreparedRewrite<T, I>, PreparedRewrite<T, I>)>,
    ) -> Result<RevisionEffectResidualLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let frontier = self.residual_frontier(other)?;
        let prepared = coordination_registry.prepare(
            frontier
                .left
                .iter()
                .chain(frontier.right.iter())
                .map(|event| (event.id, &event.payload)),
        )?;
        certify_prepared_coordination_frontier(frontier, residual_registry, &prepared, residuals)
    }

    /// Certifies a frontier using a previously prepared sparse coordination
    /// graph. Membership is checked exactly before the graph can grant any
    /// coordination-free claim.
    pub fn certify_prepared_residual_frontier(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        prepared: &PreparedRewriteCoordination<RevisionEffectId>,
        residuals: impl Fn(
            RevisionEffectId,
            RevisionEffectId,
        ) -> Option<(PreparedRewrite<T, I>, PreparedRewrite<T, I>)>,
    ) -> Result<RevisionEffectResidualLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let frontier = self.residual_frontier(other)?;
        certify_prepared_coordination_frontier(frontier, residual_registry, prepared, residuals)
    }
}

fn certify_prepared_coordination_frontier<T, I>(
    frontier: ResidualFrontier<'_, T, I>,
    residual_registry: &RewriteResidualFamilyRegistry,
    prepared: &PreparedRewriteCoordination<RevisionEffectId>,
    residuals: impl Fn(
        RevisionEffectId,
        RevisionEffectId,
    ) -> Option<(PreparedRewrite<T, I>, PreparedRewrite<T, I>)>,
) -> Result<RevisionEffectResidualLayerCertificate<T, I>, RevisionEffectResidualLayerError>
where
    T: PartialEq + Eq,
    I: PartialEq + Eq,
{
    let common_ids = frontier.common;
    let left = frontier.left;
    let right = frontier.right;
    let left_ids = left.iter().map(|event| event.id).collect::<BTreeSet<_>>();
    let right_ids = right.iter().map(|event| event.id).collect::<BTreeSet<_>>();
    let expected_members = left_ids
        .iter()
        .chain(right_ids.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    if prepared.members() != &expected_members {
        return Err(RevisionEffectResidualLayerError::Coordination(
            RewriteCoordinationRegistryError::PreparedSetMismatch,
        ));
    }
    let graph: &CompiledRewriteCoordinationGraph<RevisionEffectId> = prepared.graph();
    let orient_cross_pair = |a: RevisionEffectId, b: RevisionEffectId| {
        if left_ids.contains(&a) && right_ids.contains(&b) {
            Some((a, b))
        } else if left_ids.contains(&b) && right_ids.contains(&a) {
            Some((b, a))
        } else {
            None
        }
    };

    let intent_conflicts = graph
        .intent_conflicts()
        .iter()
        .filter_map(|&(a, b)| orient_cross_pair(a, b))
        .collect::<BTreeSet<_>>();
    if !intent_conflicts.is_empty() {
        return Err(RevisionEffectResidualLayerError::IntentConflicts(
            intent_conflicts,
        ));
    }
    let requires_residual = graph
        .requires_coordination()
        .iter()
        .filter_map(|&(a, b)| orient_cross_pair(a, b))
        .collect::<BTreeSet<_>>();

    let left_by_id = left
        .iter()
        .map(|event| (event.id, *event))
        .collect::<BTreeMap<_, _>>();
    let right_by_id = right
        .iter()
        .map(|event| (event.id, *event))
        .collect::<BTreeMap<_, _>>();
    let mut diamonds = BTreeMap::new();
    for &(left_id, right_id) in &requires_residual {
        let left_event = left_by_id[&left_id];
        let right_event = right_by_id[&right_id];
        let (right_after_left, left_after_right) = residuals(left_id, right_id).ok_or(
            RevisionEffectResidualLayerError::MissingResidualPair(left_id, right_id),
        )?;
        let diamond = residual_registry.certify(
            &left_event.payload,
            &right_event.payload,
            right_after_left,
            left_after_right,
        )?;
        diamonds.insert((left_id, right_id), diamond);
    }

    Ok(RevisionEffectResidualLayerCertificate {
        common: common_ids,
        left_frontier: left_ids,
        right_frontier: right_ids,
        diamonds,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use kernel_types::SemanticId;

    use crate::revision_effect::{RevisionEffect, RevisionEffectId, RevisionEffectIdeal};
    use crate::rewrite::{
        PreparedRewrite, RewriteEffect, RewriteFamilyIdentity, RewriteLawSetId,
        RewriteResidualFamilyId, RewriteResidualFamilyKey, RewriteResidualFamilyRegistry,
        RewriteResidualFamilySpec, RewriteResidualResolutionAuthority, RewriteSequentialFamilyId,
        RewriteSequentialFamilyKey, RewriteSequentialFamilyRegistry, RewriteSequentialFamilySpec,
        RewriteSpecId, SharedPreparedRewrite,
    };

    fn family() -> RewriteFamilyIdentity {
        RewriteFamilyIdentity {
            spec: RewriteSpecId(SemanticId(50)),
            law_set: RewriteLawSetId(SemanticId(51)),
        }
    }

    fn rewrite(delta: i32, endpoint: i32) -> PreparedRewrite<i32, i32> {
        PreparedRewrite {
            spec: family().spec,
            law_set: family().law_set,
            explicit_inputs: vec![delta],
            effect: RewriteEffect::Replace(endpoint),
        }
    }

    fn event(
        id: u128,
        root: Option<u128>,
        delta: i32,
    ) -> RevisionEffect<SharedPreparedRewrite<i32, i32>> {
        RevisionEffect {
            id: RevisionEffectId(id),
            prerequisites: root
                .into_iter()
                .map(RevisionEffectId)
                .collect::<BTreeSet<_>>(),
            payload: rewrite(delta, delta).into(),
        }
    }

    #[test]
    fn resolved_frontier_normalizes_width_four_by_two_without_shape_witnesses() {
        let identity = family();
        let mut residual = RewriteResidualFamilyRegistry::default();
        residual
            .register(RewriteResidualFamilySpec {
                id: RewriteResidualFamilyId(SemanticId(52)),
                key: RewriteResidualFamilyKey {
                    left: identity,
                    right: identity,
                },
                right_after_left: identity,
                left_after_right: identity,
            })
            .unwrap();
        let mut sequential = RewriteSequentialFamilyRegistry::default();
        sequential
            .register(RewriteSequentialFamilySpec {
                id: RewriteSequentialFamilyId(SemanticId(53)),
                key: RewriteSequentialFamilyKey {
                    first: identity,
                    second: identity,
                },
                composite: identity,
            })
            .unwrap();
        let authority = RewriteResidualResolutionAuthority::new(
            &residual,
            &sequential,
            |base: &i32, left: &PreparedRewrite<i32, i32>, right: &PreparedRewrite<i32, i32>| {
                let left_delta = left.explicit_inputs[0];
                let right_delta = right.explicit_inputs[0];
                let endpoint = *base + left_delta + right_delta;
                (
                    rewrite(right_delta, endpoint),
                    rewrite(left_delta, endpoint),
                )
            },
            |base: &i32, first: &PreparedRewrite<i32, i32>, second: &PreparedRewrite<i32, i32>| {
                let delta = first.explicit_inputs[0] + second.explicit_inputs[0];
                rewrite(delta, *base + delta)
            },
        );

        let root = event(1, None, 0);
        let left = RevisionEffectIdeal::new([
            root.clone(),
            event(2, Some(1), 1),
            event(3, Some(1), 2),
            event(4, Some(1), 3),
            event(5, Some(1), 4),
        ])
        .unwrap();
        let right =
            RevisionEffectIdeal::new([root, event(6, Some(1), 5), event(7, Some(1), 6)]).unwrap();

        let certificate = left
            .certify_resolved_residual_frontier(&right, &0, &authority)
            .unwrap();
        assert_eq!(certificate.left_frontier.len(), 4);
        assert_eq!(certificate.right_frontier.len(), 2);
        assert_eq!(certificate.left_normalization.common_endpoint(), &10);
        assert_eq!(certificate.right_normalization.common_endpoint(), &11);
        assert_eq!(certificate.common_endpoint(), &21);
    }
}
