use std::collections::BTreeMap;

use crate::rewrite::{PreparedRewrite, SharedPreparedRewrite};

use super::{
    RewriteResidualBraidCubeCertificate, RewriteResidualCubeWitness, RewriteResidualDiamond,
    RewriteResidualFamilyRegistry, RewriteResidualRegistryError, RewriteSequentialComposition,
    RewriteSequentialFamilyRegistry, RewriteSequentialRegistryError,
};

/// Total residual operator used by the finite concurrent normalizer.
///
/// The provider must return residual candidates for every pair in every
/// reachable context in which it is invoked. Semantic authority still comes
/// from `RewriteResidualFamilyRegistry`: every candidate is certified before
/// it can enter a normalization certificate.
pub trait RewriteResidualPairResolver<T, I> {
    fn resolve_pair(
        &self,
        base: &T,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
    ) -> (PreparedRewrite<T, I>, PreparedRewrite<T, I>);
}

impl<T, I, F> RewriteResidualPairResolver<T, I> for F
where
    F: Fn(
        &T,
        &PreparedRewrite<T, I>,
        &PreparedRewrite<T, I>,
    ) -> (PreparedRewrite<T, I>, PreparedRewrite<T, I>),
{
    fn resolve_pair(
        &self,
        base: &T,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
    ) -> (PreparedRewrite<T, I>, PreparedRewrite<T, I>) {
        self(base, left, right)
    }
}

/// Total sequential-composition operator paired with the residual resolver.
/// The returned candidate is accepted only after the sequential registry has
/// certified both family identity and extensional endpoint.
pub trait RewriteSequentialPairResolver<T, I> {
    fn resolve_composite(
        &self,
        base: &T,
        first: &PreparedRewrite<T, I>,
        second: &PreparedRewrite<T, I>,
    ) -> PreparedRewrite<T, I>;
}

impl<T, I, F> RewriteSequentialPairResolver<T, I> for F
where
    F: Fn(&T, &PreparedRewrite<T, I>, &PreparedRewrite<T, I>) -> PreparedRewrite<T, I>,
{
    fn resolve_composite(
        &self,
        base: &T,
        first: &PreparedRewrite<T, I>,
        second: &PreparedRewrite<T, I>,
    ) -> PreparedRewrite<T, I> {
        self(base, first, second)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteResidualResolutionError {
    EmptyFrontier,
    DuplicateKey,
    Residual(RewriteResidualRegistryError),
    Sequential(RewriteSequentialRegistryError),
}

impl From<RewriteResidualRegistryError> for RewriteResidualResolutionError {
    fn from(value: RewriteResidualRegistryError) -> Self {
        Self::Residual(value)
    }
}

impl From<RewriteSequentialRegistryError> for RewriteResidualResolutionError {
    fn from(value: RewriteSequentialRegistryError) -> Self {
        Self::Sequential(value)
    }
}

/// One recursive prefix of a finite braid normalization proof.
///
/// `pair_diamonds` certifies all pair residuals available at this context.
/// `pivot_braids` contains the Yang-Baxter critical-pair certificates between
/// the canonical pivot and every pair in the tail. The remaining tail is then
/// certified recursively at the state reached through the pivot.
#[derive(Debug, PartialEq, Eq)]
pub struct RewriteConcurrentNormalizationLevelCertificate<K, T, I> {
    pub pivot: K,
    pub pair_diamonds: BTreeMap<(K, K), RewriteResidualDiamond<T, I>>,
    pub pivot_braids: BTreeMap<(K, K), RewriteResidualBraidCubeCertificate<T, I>>,
    pub composition: Option<RewriteSequentialComposition<T, I>>,
}

impl<K: Clone, T, I> Clone for RewriteConcurrentNormalizationLevelCertificate<K, T, I> {
    fn clone(&self) -> Self {
        Self {
            pivot: self.pivot.clone(),
            pair_diamonds: self.pair_diamonds.clone(),
            pivot_braids: self.pivot_braids.clone(),
            composition: self.composition.clone(),
        }
    }
}

/// Finite arbitrary-width concurrent normalization certificate.
///
/// The canonical order is determined only by `K`. Pair residuals are resolved
/// and certified in each reachable canonical-prefix context; full braid cubes
/// establish local confluence for moving the pivot through any ordering of the
/// tail. Induction over the recursively residualized tail yields one canonical
/// composite without `Single/Pair/Triple` execution routing or permutation
/// enumeration.
#[derive(Debug, PartialEq, Eq)]
pub struct RewriteConcurrentNormalizationCertificate<K, T, I> {
    pub canonical_order: Vec<K>,
    pub canonical_path: Vec<SharedPreparedRewrite<T, I>>,
    pub levels: Vec<RewriteConcurrentNormalizationLevelCertificate<K, T, I>>,
    composite: SharedPreparedRewrite<T, I>,
}

impl<K: Clone, T, I> Clone for RewriteConcurrentNormalizationCertificate<K, T, I> {
    fn clone(&self) -> Self {
        Self {
            canonical_order: self.canonical_order.clone(),
            canonical_path: self.canonical_path.clone(),
            levels: self.levels.clone(),
            composite: self.composite.clone(),
        }
    }
}

impl<K, T, I> RewriteConcurrentNormalizationCertificate<K, T, I> {
    #[must_use]
    pub fn composite(&self) -> &PreparedRewrite<T, I> {
        self.composite.as_rewrite()
    }

    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.composite.endpoint()
    }
}

struct BraidLowerFaces<'a, T, I> {
    ab: &'a RewriteResidualDiamond<T, I>,
    ac: &'a RewriteResidualDiamond<T, I>,
    bc: &'a RewriteResidualDiamond<T, I>,
}

/// Proof-carrying resolver authority for finite concurrent Rewrite families.
///
/// The two resolver providers are total candidate generators. They never grant
/// authority by themselves: residual and sequential registries certify every
/// generated edge, and the n-ary normalizer additionally checks the full braid
/// residual tuple at each critical triple in every canonical-prefix context.
pub struct RewriteResidualResolutionAuthority<'a, R, S> {
    residual_registry: &'a RewriteResidualFamilyRegistry,
    sequential_registry: &'a RewriteSequentialFamilyRegistry,
    residual_resolver: R,
    sequential_resolver: S,
}

impl<'a, R, S> RewriteResidualResolutionAuthority<'a, R, S> {
    #[must_use]
    pub const fn new(
        residual_registry: &'a RewriteResidualFamilyRegistry,
        sequential_registry: &'a RewriteSequentialFamilyRegistry,
        residual_resolver: R,
        sequential_resolver: S,
    ) -> Self {
        Self {
            residual_registry,
            sequential_registry,
            residual_resolver,
            sequential_resolver,
        }
    }

    pub fn resolve<T, I>(
        &self,
        base: &T,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
    ) -> Result<RewriteResidualDiamond<T, I>, RewriteResidualResolutionError>
    where
        T: PartialEq + Eq,
        R: RewriteResidualPairResolver<T, I>,
    {
        let (right_after_left, left_after_right) =
            self.residual_resolver.resolve_pair(base, left, right);
        self.residual_registry
            .certify(left, right, right_after_left, left_after_right)
            .map_err(Into::into)
    }

    pub fn compose<T, I>(
        &self,
        base: &T,
        first: SharedPreparedRewrite<T, I>,
        second: SharedPreparedRewrite<T, I>,
    ) -> Result<RewriteSequentialComposition<T, I>, RewriteResidualResolutionError>
    where
        T: PartialEq + Eq,
        S: RewriteSequentialPairResolver<T, I>,
    {
        let composite = self
            .sequential_resolver
            .resolve_composite(base, &first, &second)
            .into();
        self.sequential_registry
            .certify_shared(first, second, composite)
            .map_err(Into::into)
    }

    pub fn certify_braid<T, I>(
        &self,
        base: &T,
        a: &PreparedRewrite<T, I>,
        b: &PreparedRewrite<T, I>,
        c: &PreparedRewrite<T, I>,
    ) -> Result<RewriteResidualBraidCubeCertificate<T, I>, RewriteResidualResolutionError>
    where
        T: PartialEq + Eq,
        I: PartialEq + Eq,
        R: RewriteResidualPairResolver<T, I>,
    {
        let ab = self.resolve(base, a, b)?;
        let ac = self.resolve(base, a, c)?;
        let bc = self.resolve(base, b, c)?;
        self.certify_braid_from_lower(
            a,
            b,
            c,
            &BraidLowerFaces {
                ab: &ab,
                ac: &ac,
                bc: &bc,
            },
        )
    }

    /// Certifies one arbitrary-width finite concurrent family in canonical key
    /// order. The algorithm is prefix-recursive rather than permutation-based:
    /// one pivot is fixed, all tail rewrites are residualized through it, braid
    /// critical pairs involving that pivot are certified, then the tail is
    /// normalized in the reached context.
    pub fn certify_finite_concurrent<K, T, I, W>(
        &self,
        base: &T,
        rewrites: impl IntoIterator<Item = (K, W)>,
    ) -> Result<RewriteConcurrentNormalizationCertificate<K, T, I>, RewriteResidualResolutionError>
    where
        K: Copy + Ord,
        T: PartialEq + Eq,
        I: PartialEq + Eq,
        W: Into<SharedPreparedRewrite<T, I>>,
        R: RewriteResidualPairResolver<T, I>,
        S: RewriteSequentialPairResolver<T, I>,
    {
        let mut ordered = BTreeMap::new();
        for (key, rewrite) in rewrites {
            if ordered.insert(key, rewrite.into()).is_some() {
                return Err(RewriteResidualResolutionError::DuplicateKey);
            }
        }
        if ordered.is_empty() {
            return Err(RewriteResidualResolutionError::EmptyFrontier);
        }
        let ordered = ordered.into_iter().collect::<Vec<_>>();
        self.certify_ordered_prefix(base, &ordered)
    }

    fn certify_ordered_prefix<K, T, I>(
        &self,
        base: &T,
        current: &[(K, SharedPreparedRewrite<T, I>)],
    ) -> Result<RewriteConcurrentNormalizationCertificate<K, T, I>, RewriteResidualResolutionError>
    where
        K: Copy + Ord,
        T: PartialEq + Eq,
        I: PartialEq + Eq,
        R: RewriteResidualPairResolver<T, I>,
        S: RewriteSequentialPairResolver<T, I>,
    {
        let (pivot_key, pivot) = current
            .first()
            .expect("finite concurrent normalization is entered non-empty");
        if current.len() == 1 {
            return Ok(RewriteConcurrentNormalizationCertificate {
                canonical_order: vec![*pivot_key],
                canonical_path: vec![pivot.clone()],
                levels: vec![RewriteConcurrentNormalizationLevelCertificate {
                    pivot: *pivot_key,
                    pair_diamonds: BTreeMap::new(),
                    pivot_braids: BTreeMap::new(),
                    composition: None,
                }],
                composite: pivot.clone(),
            });
        }

        let mut pair_diamonds = BTreeMap::new();
        for left_index in 0..current.len() {
            for right_index in (left_index + 1)..current.len() {
                let (left_key, left) = &current[left_index];
                let (right_key, right) = &current[right_index];
                pair_diamonds.insert(
                    (*left_key, *right_key),
                    self.resolve(base, left.as_rewrite(), right.as_rewrite())?,
                );
            }
        }

        let mut pivot_braids = BTreeMap::new();
        for left_index in 1..current.len() {
            for right_index in (left_index + 1)..current.len() {
                let (left_key, left) = &current[left_index];
                let (right_key, right) = &current[right_index];
                let ab = &pair_diamonds[&(*pivot_key, *left_key)];
                let ac = &pair_diamonds[&(*pivot_key, *right_key)];
                let bc = &pair_diamonds[&(*left_key, *right_key)];
                let braid = self.certify_braid_from_lower(
                    pivot.as_rewrite(),
                    left.as_rewrite(),
                    right.as_rewrite(),
                    &BraidLowerFaces { ab, ac, bc },
                )?;
                pivot_braids.insert((*left_key, *right_key), braid);
            }
        }

        let next_base = pivot.endpoint();
        let mut residual_tail = Vec::with_capacity(current.len() - 1);
        for (key, _) in &current[1..] {
            let diamond = &pair_diamonds[&(*pivot_key, *key)];
            residual_tail.push((*key, diamond.right_after_left.clone()));
        }
        let tail = self.certify_ordered_prefix(next_base, &residual_tail)?;
        let composition = self.compose(base, pivot.clone(), tail.composite.clone())?;

        let mut canonical_order = Vec::with_capacity(current.len());
        canonical_order.push(*pivot_key);
        canonical_order.extend(tail.canonical_order.iter().copied());
        let mut canonical_path = Vec::with_capacity(current.len());
        canonical_path.push(pivot.clone());
        canonical_path.extend(tail.canonical_path.iter().cloned());
        let composite = composition.composite.clone();
        let mut levels = Vec::with_capacity(current.len());
        levels.push(RewriteConcurrentNormalizationLevelCertificate {
            pivot: *pivot_key,
            pair_diamonds,
            pivot_braids,
            composition: Some(composition),
        });
        levels.extend(tail.levels);

        Ok(RewriteConcurrentNormalizationCertificate {
            canonical_order,
            canonical_path,
            levels,
            composite,
        })
    }

    fn certify_braid_from_lower<T, I>(
        &self,
        a: &PreparedRewrite<T, I>,
        b: &PreparedRewrite<T, I>,
        c: &PreparedRewrite<T, I>,
        lower: &BraidLowerFaces<'_, T, I>,
    ) -> Result<RewriteResidualBraidCubeCertificate<T, I>, RewriteResidualResolutionError>
    where
        T: PartialEq + Eq,
        I: PartialEq + Eq,
        R: RewriteResidualPairResolver<T, I>,
    {
        let after_a = self.resolve(
            a.endpoint(),
            &lower.ab.right_after_left,
            &lower.ac.right_after_left,
        )?;
        let after_b = self.resolve(
            b.endpoint(),
            &lower.ab.left_after_right,
            &lower.bc.right_after_left,
        )?;
        let after_c = self.resolve(
            c.endpoint(),
            &lower.ac.left_after_right,
            &lower.bc.left_after_right,
        )?;
        let witness = RewriteResidualCubeWitness {
            b_after_a: lower.ab.right_after_left.clone(),
            a_after_b: lower.ab.left_after_right.clone(),
            c_after_a: lower.ac.right_after_left.clone(),
            a_after_c: lower.ac.left_after_right.clone(),
            c_after_b: lower.bc.right_after_left.clone(),
            b_after_c: lower.bc.left_after_right.clone(),
            c_after_ab: after_a.right_after_left,
            b_after_ac: after_a.left_after_right,
            c_after_ba: after_b.right_after_left,
            a_after_bc: after_b.left_after_right,
            b_after_ca: after_c.right_after_left,
            a_after_cb: after_c.left_after_right,
        };
        self.residual_registry
            .certify_braid_cube(a, b, c, witness)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use kernel_types::SemanticId;

    use crate::rewrite::{
        PreparedRewrite, RewriteEffect, RewriteFamilyIdentity, RewriteLawSetId,
        RewriteResidualFamilyId, RewriteResidualFamilyKey, RewriteResidualFamilyRegistry,
        RewriteResidualFamilySpec, RewriteSequentialFamilyId, RewriteSequentialFamilyKey,
        RewriteSequentialFamilyRegistry, RewriteSequentialFamilySpec, RewriteSpecId,
    };

    use super::RewriteResidualResolutionAuthority;

    fn identity() -> RewriteFamilyIdentity {
        RewriteFamilyIdentity {
            spec: RewriteSpecId(SemanticId(1)),
            law_set: RewriteLawSetId(SemanticId(2)),
        }
    }

    fn rewrite(delta: i32, endpoint: i32) -> PreparedRewrite<i32, i32> {
        PreparedRewrite {
            spec: identity().spec,
            law_set: identity().law_set,
            explicit_inputs: vec![delta],
            effect: RewriteEffect::Replace(endpoint),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneEndpoint(i32);

    fn non_clone_rewrite(delta: i32, endpoint: i32) -> PreparedRewrite<NonCloneEndpoint, i32> {
        PreparedRewrite {
            spec: identity().spec,
            law_set: identity().law_set,
            explicit_inputs: vec![delta],
            effect: RewriteEffect::Replace(NonCloneEndpoint(endpoint)),
        }
    }

    fn registries() -> (
        RewriteResidualFamilyRegistry,
        RewriteSequentialFamilyRegistry,
    ) {
        let family = identity();
        let mut residual = RewriteResidualFamilyRegistry::default();
        residual
            .register(RewriteResidualFamilySpec {
                id: RewriteResidualFamilyId(SemanticId(3)),
                key: RewriteResidualFamilyKey {
                    left: family,
                    right: family,
                },
                right_after_left: family,
                left_after_right: family,
            })
            .unwrap();
        let mut sequential = RewriteSequentialFamilyRegistry::default();
        sequential
            .register(RewriteSequentialFamilySpec {
                id: RewriteSequentialFamilyId(SemanticId(4)),
                key: RewriteSequentialFamilyKey {
                    first: family,
                    second: family,
                },
                composite: family,
            })
            .unwrap();
        (residual, sequential)
    }

    #[test]
    fn finite_braid_normalizer_handles_width_four_without_shape_routing() {
        let (residual, sequential) = registries();
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

        let first = authority
            .certify_finite_concurrent(
                &0,
                [
                    (40_u8, rewrite(4, 4)),
                    (10, rewrite(1, 1)),
                    (30, rewrite(3, 3)),
                    (20, rewrite(2, 2)),
                ],
            )
            .unwrap();
        let second = authority
            .certify_finite_concurrent(
                &0,
                [
                    (20_u8, rewrite(2, 2)),
                    (30, rewrite(3, 3)),
                    (10, rewrite(1, 1)),
                    (40, rewrite(4, 4)),
                ],
            )
            .unwrap();

        assert_eq!(first.canonical_order, vec![10, 20, 30, 40]);
        assert_eq!(first.canonical_order, second.canonical_order);
        assert_eq!(first.common_endpoint(), &10);
        assert_eq!(first.composite(), second.composite());
        assert_eq!(first.levels.len(), 4);
        assert_eq!(first.levels[0].pivot_braids.len(), 3);
        assert_eq!(first.levels[1].pivot_braids.len(), 1);
    }

    #[test]
    fn finite_braid_normalizer_does_not_require_cloneable_endpoints() {
        let (residual, sequential) = registries();
        let authority = RewriteResidualResolutionAuthority::new(
            &residual,
            &sequential,
            |base: &NonCloneEndpoint,
             left: &PreparedRewrite<NonCloneEndpoint, i32>,
             right: &PreparedRewrite<NonCloneEndpoint, i32>| {
                let left_delta = left.explicit_inputs[0];
                let right_delta = right.explicit_inputs[0];
                let endpoint = base.0 + left_delta + right_delta;
                (
                    non_clone_rewrite(right_delta, endpoint),
                    non_clone_rewrite(left_delta, endpoint),
                )
            },
            |base: &NonCloneEndpoint,
             first: &PreparedRewrite<NonCloneEndpoint, i32>,
             second: &PreparedRewrite<NonCloneEndpoint, i32>| {
                let delta = first.explicit_inputs[0] + second.explicit_inputs[0];
                non_clone_rewrite(delta, base.0 + delta)
            },
        );

        let certificate = authority
            .certify_finite_concurrent(
                &NonCloneEndpoint(0),
                [
                    (40_u8, non_clone_rewrite(4, 4)),
                    (10, non_clone_rewrite(1, 1)),
                    (30, non_clone_rewrite(3, 3)),
                    (20, non_clone_rewrite(2, 2)),
                ],
            )
            .unwrap();

        let cloned = certificate.clone();

        assert_eq!(certificate.canonical_order, vec![10, 20, 30, 40]);
        assert_eq!(certificate.common_endpoint(), &NonCloneEndpoint(10));
        assert_eq!(cloned.common_endpoint(), &NonCloneEndpoint(10));
        assert_eq!(certificate.levels.len(), 4);
    }
}
