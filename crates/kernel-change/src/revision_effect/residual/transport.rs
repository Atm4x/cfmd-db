use std::collections::BTreeSet;

use crate::rewrite::{
    PreparedRewrite, RewriteResidualFamilyRegistry, RewriteSequentialFamilyRegistry,
    SharedPreparedRewrite,
};

use super::super::{RevisionEffectId, RevisionEffectIdeal};
use super::certificates::{
    RevisionEffectResidualChainCertificate, RevisionEffectResidualChainStepCertificate,
    RevisionEffectResidualChainStepWitness, RevisionEffectResidualChainWitness,
    RevisionEffectResidualLayerError, RevisionEffectResidualMixedChainCertificate,
    RevisionEffectResidualMixedChainWitness, RevisionEffectResidualMixedFirstCertificate,
    RevisionEffectResidualMixedFirstWitness, RevisionEffectResidualMixedSquareStepCertificate,
    RevisionEffectResidualMixedSquareStepWitness, RevisionEffectResidualMixedStepCertificate,
    RevisionEffectResidualMixedStepWitness, RevisionEffectResidualNormalizedLayerCertificate,
    RevisionEffectResidualNormalizedLayerWitness, RevisionEffectResidualSquareChainCertificate,
    RevisionEffectResidualSquareChainWitness, RevisionEffectResidualSquareLayerCertificate,
    RevisionEffectResidualSquareLayerWitness, RevisionEffectTwoLayerResidualCertificate,
    RevisionEffectTwoLayerResidualWitness,
};
use super::concurrent::{
    certify_concurrent_branch, certify_concurrent_pair, certify_square_normalization,
};

struct ResidualChainProgress<T, I> {
    cumulative_right_prefix: SharedPreparedRewrite<T, I>,
    cumulative_left_prefix: SharedPreparedRewrite<T, I>,
}

struct ResidualChainStep<'a, T, I> {
    left_rewrite: &'a PreparedRewrite<T, I>,
    right_rewrite: &'a PreparedRewrite<T, I>,
    witness: RevisionEffectResidualChainStepWitness<T, I>,
    needs_next_prefix: bool,
}

struct ResidualChainRegistries<'a> {
    residual: &'a RewriteResidualFamilyRegistry,
    sequential: &'a RewriteSequentialFamilyRegistry,
}

struct ResidualSquareLayer<T, I> {
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualSquareLayerWitness<T, I>,
}

struct ResidualMixedFirstLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualMixedFirstWitness<T, I>,
}

struct ResidualNormalizedMixedFirstLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualNormalizedLayerWitness<T, I>,
}

struct ResidualMixedLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<SharedPreparedRewrite<T, I>>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualMixedStepWitness<T, I>,
    needs_next_prefix: bool,
}

struct ResidualMixedFirstResult<T, I> {
    certificate: RevisionEffectResidualMixedFirstCertificate<T, I>,
    progress: ResidualChainProgress<T, I>,
}

fn certify_residual_chain_step<T: PartialEq + Eq, I: PartialEq + Eq>(
    progress: &mut ResidualChainProgress<T, I>,
    step: ResidualChainStep<'_, T, I>,
    registries: &ResidualChainRegistries<'_>,
) -> Result<RevisionEffectResidualChainStepCertificate<T, I>, RevisionEffectResidualLayerError> {
    let left_transport = registries.residual.certify(
        step.left_rewrite,
        &progress.cumulative_right_prefix,
        step.witness.right_prefix_after_left,
        step.witness.left_after_right_prefix,
    )?;
    let right_transport = registries.residual.certify(
        step.right_rewrite,
        &progress.cumulative_left_prefix,
        step.witness.left_prefix_after_right,
        step.witness.right_after_left_prefix,
    )?;
    let cross = registries.residual.certify(
        &left_transport.left_after_right,
        &right_transport.left_after_right,
        step.witness.cross_right_after_left,
        step.witness.cross_left_after_right,
    )?;
    let cumulative_right_prefix = if step.needs_next_prefix {
        let composite = step
            .witness
            .cumulative_right_prefix
            .ok_or(RevisionEffectResidualLayerError::WitnessLayerCountMismatch)?;
        let certificate = registries.sequential.certify_shared(
            left_transport.right_after_left.clone(),
            cross.right_after_left.clone(),
            composite.into(),
        )?;
        progress.cumulative_right_prefix = certificate.composite.clone();
        Some(certificate)
    } else {
        None
    };
    let cumulative_left_prefix = if step.needs_next_prefix {
        let composite = step
            .witness
            .cumulative_left_prefix
            .ok_or(RevisionEffectResidualLayerError::WitnessLayerCountMismatch)?;
        let certificate = registries.sequential.certify_shared(
            right_transport.right_after_left.clone(),
            cross.left_after_right.clone(),
            composite.into(),
        )?;
        progress.cumulative_left_prefix = certificate.composite.clone();
        Some(certificate)
    } else {
        None
    };
    Ok(RevisionEffectResidualChainStepCertificate {
        left_transport,
        right_transport,
        cross,
        cumulative_right_prefix,
        cumulative_left_prefix,
    })
}

fn certify_mixed_first_layer<T: PartialEq + Eq, I: PartialEq + Eq>(
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualMixedFirstLayer<'_, T, I>,
) -> Result<ResidualMixedFirstResult<T, I>, RevisionEffectResidualLayerError> {
    match layer.witness {
        RevisionEffectResidualMixedFirstWitness::Singleton {
            right_after_left,
            left_after_right,
        } if layer.left_frontier.len() == 1 && layer.right_frontier.len() == 1 => {
            let left_id = *layer.left_frontier.first().expect("shape checked");
            let right_id = *layer.right_frontier.first().expect("shape checked");
            let left_rewrite = &layer.left.events[&left_id].payload;
            let right_rewrite = &layer.right.events[&right_id].payload;
            let first = registries.residual.certify(
                left_rewrite,
                right_rewrite,
                right_after_left,
                left_after_right,
            )?;
            let progress = ResidualChainProgress {
                cumulative_right_prefix: first.right_after_left.clone(),
                cumulative_left_prefix: first.left_after_right.clone(),
            };
            Ok(ResidualMixedFirstResult {
                certificate: RevisionEffectResidualMixedFirstCertificate::Singleton(first),
                progress,
            })
        }
        RevisionEffectResidualMixedFirstWitness::Square(witness)
            if layer.left_frontier.len() == 2 && layer.right_frontier.len() == 2 =>
        {
            let first = layer.left.certify_registered_residual_square_layer(
                layer.right,
                registries,
                ResidualSquareLayer {
                    common: layer.common,
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    witness: *witness,
                },
            )?;
            let progress = ResidualChainProgress {
                cumulative_right_prefix: first.cross.right_after_left.clone(),
                cumulative_left_prefix: first.cross.left_after_right.clone(),
            };
            Ok(ResidualMixedFirstResult {
                certificate: RevisionEffectResidualMixedFirstCertificate::Square(Box::new(first)),
                progress,
            })
        }
        RevisionEffectResidualMixedFirstWitness::Normalized(witness)
            if (1..=3).contains(&layer.left_frontier.len())
                && (1..=3).contains(&layer.right_frontier.len()) =>
        {
            certify_normalized_mixed_first_layer(
                registries,
                ResidualNormalizedMixedFirstLayer {
                    left: layer.left,
                    right: layer.right,
                    common: layer.common,
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    witness: *witness,
                },
            )
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

fn certify_normalized_mixed_first_layer<T: PartialEq + Eq, I: PartialEq + Eq>(
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualNormalizedMixedFirstLayer<'_, T, I>,
) -> Result<ResidualMixedFirstResult<T, I>, RevisionEffectResidualLayerError> {
    let RevisionEffectResidualNormalizedLayerWitness {
        left,
        right,
        right_after_left,
        left_after_right,
    } = layer.witness;
    let left_normalization = certify_concurrent_branch(
        layer.left,
        &layer.left_frontier,
        registries.residual,
        registries.sequential,
        left,
    )?;
    let right_normalization = certify_concurrent_branch(
        layer.right,
        &layer.right_frontier,
        registries.residual,
        registries.sequential,
        right,
    )?;
    let cross = registries.residual.certify(
        left_normalization.composite(),
        right_normalization.composite(),
        right_after_left,
        left_after_right,
    )?;
    let progress = ResidualChainProgress {
        cumulative_right_prefix: cross.right_after_left.clone(),
        cumulative_left_prefix: cross.left_after_right.clone(),
    };
    Ok(ResidualMixedFirstResult {
        certificate: RevisionEffectResidualMixedFirstCertificate::Normalized(Box::new(
            RevisionEffectResidualNormalizedLayerCertificate {
                common: layer.common,
                left_frontier: layer.left_frontier,
                right_frontier: layer.right_frontier,
                left_normalization,
                right_normalization,
                cross,
            },
        )),
        progress,
    })
}

fn certify_mixed_chain_step<T: PartialEq + Eq, I: PartialEq + Eq>(
    progress: &mut ResidualChainProgress<T, I>,
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualMixedLayer<'_, T, I>,
) -> Result<RevisionEffectResidualMixedStepCertificate<T, I>, RevisionEffectResidualLayerError> {
    match layer.witness {
        RevisionEffectResidualMixedStepWitness::Singleton(witness)
            if layer.left_frontier.len() == 1 && layer.right_frontier.len() == 1 =>
        {
            let left_id = *layer.left_frontier.first().expect("shape checked");
            let right_id = *layer.right_frontier.first().expect("shape checked");
            let certificate = certify_residual_chain_step(
                progress,
                ResidualChainStep {
                    left_rewrite: &layer.left.events[&left_id].payload,
                    right_rewrite: &layer.right.events[&right_id].payload,
                    witness: *witness,
                    needs_next_prefix: layer.needs_next_prefix,
                },
                registries,
            )?;
            Ok(RevisionEffectResidualMixedStepCertificate::Singleton(
                Box::new(certificate),
            ))
        }
        RevisionEffectResidualMixedStepWitness::Square(witness)
            if layer.left_frontier.len() == 2 && layer.right_frontier.len() == 2 =>
        {
            let RevisionEffectResidualMixedSquareStepWitness {
                left,
                right,
                transport,
            } = *witness;
            let left_normalization = certify_square_normalization(
                layer.left,
                &layer.left_frontier,
                registries.residual,
                registries.sequential,
                left,
            )?;
            let right_normalization = certify_square_normalization(
                layer.right,
                &layer.right_frontier,
                registries.residual,
                registries.sequential,
                right,
            )?;
            let certificate = certify_residual_chain_step(
                progress,
                ResidualChainStep {
                    left_rewrite: left_normalization.composite(),
                    right_rewrite: right_normalization.composite(),
                    witness: transport,
                    needs_next_prefix: layer.needs_next_prefix,
                },
                registries,
            )?;
            Ok(RevisionEffectResidualMixedStepCertificate::Square(
                Box::new(RevisionEffectResidualMixedSquareStepCertificate {
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    left_normalization,
                    right_normalization,
                    transport: certificate,
                }),
            ))
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

impl<T: PartialEq + Eq, I: PartialEq + Eq> RevisionEffectIdeal<SharedPreparedRewrite<T, I>> {
    fn certify_registered_residual_square_layer(
        &self,
        other: &Self,
        registries: &ResidualChainRegistries<'_>,
        layer: ResidualSquareLayer<T, I>,
    ) -> Result<RevisionEffectResidualSquareLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let ResidualSquareLayer {
            common,
            left_frontier,
            right_frontier,
            witness,
        } = layer;
        if left_frontier.len() != 2 || right_frontier.len() != 2 {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        let mut left_ids = left_frontier.iter().copied();
        let left_a = left_ids.next().expect("shape checked");
        let left_b = left_ids.next().expect("shape checked");
        let mut right_ids = right_frontier.iter().copied();
        let right_a = right_ids.next().expect("shape checked");
        let right_b = right_ids.next().expect("shape checked");

        let left_normalization = certify_concurrent_pair(
            &self.events[&left_a].payload,
            &self.events[&left_b].payload,
            registries.residual,
            registries.sequential,
            witness.left,
        )?;
        let right_normalization = certify_concurrent_pair(
            &other.events[&right_a].payload,
            &other.events[&right_b].payload,
            registries.residual,
            registries.sequential,
            witness.right,
        )?;
        let cross = registries.residual.certify(
            left_normalization.composite(),
            right_normalization.composite(),
            witness.right_after_left,
            witness.left_after_right,
        )?;
        Ok(RevisionEffectResidualSquareLayerCertificate {
            common,
            left_frontier,
            right_frontier,
            left_normalization,
            right_normalization,
            cross,
        })
    }

    pub fn certify_registered_residual_square_frontier(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualSquareLayerWitness<T, I>,
    ) -> Result<RevisionEffectResidualSquareLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let mixed = self.certify_registered_residual_mixed_chain(
            other,
            residual_registry,
            sequential_registry,
            RevisionEffectResidualMixedChainWitness {
                first: RevisionEffectResidualMixedFirstWitness::Square(Box::new(witness)),
                subsequent: Vec::new(),
            },
        )?;
        if !mixed.subsequent.is_empty() {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        match mixed.first {
            RevisionEffectResidualMixedFirstCertificate::Square(certificate) => Ok(*certificate),
            _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
        }
    }

    pub fn certify_registered_residual_square_chain(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualSquareChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualSquareChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let mixed = self.certify_registered_residual_mixed_chain(
            other,
            residual_registry,
            sequential_registry,
            RevisionEffectResidualMixedChainWitness {
                first: RevisionEffectResidualMixedFirstWitness::Square(Box::new(witness.first)),
                subsequent: witness
                    .subsequent
                    .into_iter()
                    .map(|step| RevisionEffectResidualMixedStepWitness::Singleton(Box::new(step)))
                    .collect(),
            },
        )?;
        let RevisionEffectResidualMixedFirstCertificate::Square(first) = mixed.first else {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        };
        let first = *first;
        let mut subsequent = Vec::with_capacity(mixed.subsequent.len());
        for step in mixed.subsequent {
            match step {
                RevisionEffectResidualMixedStepCertificate::Singleton(certificate) => {
                    subsequent.push(*certificate);
                }
                RevisionEffectResidualMixedStepCertificate::Square(_) => {
                    return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
                }
            }
        }
        Ok(RevisionEffectResidualSquareChainCertificate {
            schedule: mixed.schedule,
            first,
            subsequent,
        })
    }

    pub fn certify_registered_residual_mixed_chain(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualMixedChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualMixedChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        let depth = schedule.left_layers.len();
        if depth == 0 || schedule.right_layers.len() != depth {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        if witness.subsequent.len() + 1 != depth {
            return Err(RevisionEffectResidualLayerError::WitnessLayerCountMismatch);
        }
        let registries = ResidualChainRegistries {
            residual: residual_registry,
            sequential: sequential_registry,
        };
        let first_result = certify_mixed_first_layer(
            &registries,
            ResidualMixedFirstLayer {
                left: self,
                right: other,
                common: schedule.common.clone(),
                left_frontier: schedule.left_layers[0].clone(),
                right_frontier: schedule.right_layers[0].clone(),
                witness: witness.first,
            },
        )?;
        let first = first_result.certificate;
        let mut progress = first_result.progress;
        let mut certificates = Vec::with_capacity(depth.saturating_sub(1));
        for (offset, layer_witness) in witness.subsequent.into_iter().enumerate() {
            let layer_index = offset + 1;
            certificates.push(certify_mixed_chain_step(
                &mut progress,
                &registries,
                ResidualMixedLayer {
                    left: self,
                    right: other,
                    left_frontier: schedule.left_layers[layer_index].clone(),
                    right_frontier: schedule.right_layers[layer_index].clone(),
                    witness: layer_witness,
                    needs_next_prefix: layer_index + 1 < depth,
                },
            )?);
        }
        Ok(RevisionEffectResidualMixedChainCertificate {
            schedule,
            first,
            subsequent: certificates,
        })
    }

    pub fn certify_registered_residual_normalized_frontier(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualNormalizedLayerWitness<T, I>,
    ) -> Result<
        RevisionEffectResidualNormalizedLayerCertificate<T, I>,
        RevisionEffectResidualLayerError,
    > {
        let mixed = self.certify_registered_residual_mixed_chain(
            other,
            residual_registry,
            sequential_registry,
            RevisionEffectResidualMixedChainWitness {
                first: RevisionEffectResidualMixedFirstWitness::Normalized(Box::new(witness)),
                subsequent: Vec::new(),
            },
        )?;
        if !mixed.subsequent.is_empty() {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        match mixed.first {
            RevisionEffectResidualMixedFirstCertificate::Normalized(certificate) => {
                Ok(*certificate)
            }
            _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
        }
    }

    pub fn certify_registered_residual_chain(
        &self,
        other: &Self,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let mixed = self.certify_registered_residual_mixed_chain(
            other,
            residual_registry,
            sequential_registry,
            RevisionEffectResidualMixedChainWitness {
                first: RevisionEffectResidualMixedFirstWitness::Singleton {
                    right_after_left: witness.first_right_after_left,
                    left_after_right: witness.first_left_after_right,
                },
                subsequent: witness
                    .subsequent
                    .into_iter()
                    .map(|step| RevisionEffectResidualMixedStepWitness::Singleton(Box::new(step)))
                    .collect(),
            },
        )?;
        let RevisionEffectResidualMixedFirstCertificate::Singleton(first) = mixed.first else {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        };
        let mut subsequent = Vec::with_capacity(mixed.subsequent.len());
        for step in mixed.subsequent {
            match step {
                RevisionEffectResidualMixedStepCertificate::Singleton(certificate) => {
                    subsequent.push(*certificate);
                }
                RevisionEffectResidualMixedStepCertificate::Square(_) => {
                    return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
                }
            }
        }
        Ok(RevisionEffectResidualChainCertificate {
            schedule: mixed.schedule,
            first,
            subsequent,
        })
    }

    pub fn certify_registered_two_layer_chain(
        &self,
        other: &Self,
        registry: &RewriteResidualFamilyRegistry,
        witness: RevisionEffectTwoLayerResidualWitness<T, I>,
    ) -> Result<RevisionEffectTwoLayerResidualCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 2
            || schedule.right_layers.len() != 2
            || schedule.left_layers.iter().any(|layer| layer.len() != 1)
            || schedule.right_layers.iter().any(|layer| layer.len() != 1)
        {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }

        let left_first_id = *schedule.left_layers[0].first().expect("shape checked");
        let left_second_id = *schedule.left_layers[1].first().expect("shape checked");
        let right_first_id = *schedule.right_layers[0].first().expect("shape checked");
        let right_second_id = *schedule.right_layers[1].first().expect("shape checked");
        let left_first = &self.events[&left_first_id].payload;
        let left_second = &self.events[&left_second_id].payload;
        let right_first = &other.events[&right_first_id].payload;
        let right_second = &other.events[&right_second_id].payload;

        let first = registry.certify(
            left_first,
            right_first,
            witness.first_right_after_left,
            witness.first_left_after_right,
        )?;
        let left_transport = registry.certify(
            left_second,
            &first.right_after_left,
            witness.right_prefix_after_left_second,
            witness.left_second_after_right_prefix,
        )?;
        let right_transport = registry.certify(
            right_second,
            &first.left_after_right,
            witness.left_prefix_after_right_second,
            witness.right_second_after_left_prefix,
        )?;
        let second = registry.certify(
            &left_transport.left_after_right,
            &right_transport.left_after_right,
            witness.second_right_after_left,
            witness.second_left_after_right,
        )?;
        Ok(RevisionEffectTwoLayerResidualCertificate {
            schedule,
            first,
            left_transport,
            right_transport,
            second,
        })
    }
}
