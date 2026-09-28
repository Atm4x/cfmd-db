use std::collections::BTreeSet;

use crate::rewrite::{
    RewriteResidualFamilyRegistry, RewriteSequentialFamilyRegistry, SharedPreparedRewrite,
};

use super::super::{RevisionEffectId, RevisionEffectIdeal};
use super::certificates::{
    RevisionEffectResidualLayerError, RewriteConcurrentBranchCertificate,
    RewriteConcurrentBranchWitness, RewriteConcurrentPairCertificate, RewriteConcurrentPairWitness,
    RewriteConcurrentTripleCertificate, RewriteConcurrentTripleWitness,
};

pub(super) fn certify_concurrent_pair<T: PartialEq + Eq, I: PartialEq + Eq>(
    left: &SharedPreparedRewrite<T, I>,
    right: &SharedPreparedRewrite<T, I>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentPairWitness<T, I>,
) -> Result<RewriteConcurrentPairCertificate<T, I>, RevisionEffectResidualLayerError> {
    let diamond = residual_registry.certify_shared(
        left,
        right,
        witness.right_after_left,
        witness.left_after_right,
    )?;
    let left_then_right = sequential_registry.certify_shared(
        left.clone(),
        diamond.right_after_left.clone(),
        witness.left_then_right_composite,
    )?;
    let right_then_left = sequential_registry.certify_shared(
        right.clone(),
        diamond.left_after_right.clone(),
        witness.right_then_left_composite,
    )?;
    if left_then_right.composite != right_then_left.composite {
        return Err(RevisionEffectResidualLayerError::ConcurrentCompositeIntentMismatch);
    }
    Ok(RewriteConcurrentPairCertificate {
        diamond,
        composite: left_then_right.composite.clone(),
        left_then_right,
        right_then_left,
    })
}

pub fn certify_registered_concurrent_triple<T: PartialEq + Eq, I: PartialEq + Eq>(
    a: &SharedPreparedRewrite<T, I>,
    b: &SharedPreparedRewrite<T, I>,
    c: &SharedPreparedRewrite<T, I>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentTripleWitness<T, I>,
) -> Result<RewriteConcurrentTripleCertificate<T, I>, RevisionEffectResidualLayerError> {
    let ab = certify_concurrent_pair(
        a,
        b,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.b_after_a.clone(),
            left_after_right: witness.cube.a_after_b.clone(),
            left_then_right_composite: witness.ab_composite.clone(),
            right_then_left_composite: witness.ab_composite.clone(),
        },
    )?;
    let ac = certify_concurrent_pair(
        a,
        c,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.c_after_a.clone(),
            left_after_right: witness.cube.a_after_c.clone(),
            left_then_right_composite: witness.ac_composite.clone(),
            right_then_left_composite: witness.ac_composite.clone(),
        },
    )?;
    let bc = certify_concurrent_pair(
        b,
        c,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.c_after_b.clone(),
            left_after_right: witness.cube.b_after_c.clone(),
            left_then_right_composite: witness.bc_composite.clone(),
            right_then_left_composite: witness.bc_composite.clone(),
        },
    )?;
    let cube = residual_registry.certify_cube(a, b, c, witness.cube)?;
    let final_paths = vec![
        sequential_registry.certify_shared(
            ab.composite.clone(),
            cube.after_a.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify_shared(
            ab.composite.clone(),
            cube.after_b.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify_shared(
            ac.composite.clone(),
            cube.after_a.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify_shared(
            ac.composite.clone(),
            cube.after_c.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify_shared(
            bc.composite.clone(),
            cube.after_b.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify_shared(
            bc.composite.clone(),
            cube.after_c.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
    ];
    Ok(RewriteConcurrentTripleCertificate {
        cube,
        ab,
        ac,
        bc,
        final_paths,
        composite: witness.final_composite,
    })
}

pub(super) fn certify_concurrent_branch<T: PartialEq + Eq, I: PartialEq + Eq>(
    ideal: &RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    frontier: &BTreeSet<RevisionEffectId>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentBranchWitness<T, I>,
) -> Result<RewriteConcurrentBranchCertificate<T, I>, RevisionEffectResidualLayerError> {
    match (frontier.len(), witness) {
        (1, RewriteConcurrentBranchWitness::Single) => {
            let id = *frontier.first().expect("shape checked");
            Ok(RewriteConcurrentBranchCertificate::Single(
                ideal.events[&id].payload.clone(),
            ))
        }
        (2, RewriteConcurrentBranchWitness::Pair(witness)) => {
            let mut ids = frontier.iter().copied();
            let first = ids.next().expect("shape checked");
            let second = ids.next().expect("shape checked");
            let certificate = certify_concurrent_pair(
                &ideal.events[&first].payload,
                &ideal.events[&second].payload,
                residual_registry,
                sequential_registry,
                *witness,
            )?;
            Ok(RewriteConcurrentBranchCertificate::Pair(Box::new(
                certificate,
            )))
        }
        (3, RewriteConcurrentBranchWitness::Triple(witness)) => {
            let mut ids = frontier.iter().copied();
            let a = ids.next().expect("shape checked");
            let b = ids.next().expect("shape checked");
            let c = ids.next().expect("shape checked");
            let certificate = certify_registered_concurrent_triple(
                &ideal.events[&a].payload,
                &ideal.events[&b].payload,
                &ideal.events[&c].payload,
                residual_registry,
                sequential_registry,
                *witness,
            )?;
            Ok(RewriteConcurrentBranchCertificate::Triple(Box::new(
                certificate,
            )))
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

pub(super) fn certify_square_normalization<T: PartialEq + Eq, I: PartialEq + Eq>(
    ideal: &RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    frontier: &BTreeSet<RevisionEffectId>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentPairWitness<T, I>,
) -> Result<RewriteConcurrentPairCertificate<T, I>, RevisionEffectResidualLayerError> {
    if frontier.len() != 2 {
        return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
    }
    let mut ids = frontier.iter().copied();
    let first = ids.next().expect("shape checked");
    let second = ids.next().expect("shape checked");
    certify_concurrent_pair(
        &ideal.events[&first].payload,
        &ideal.events[&second].payload,
        residual_registry,
        sequential_registry,
        witness,
    )
}
