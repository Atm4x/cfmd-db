mod resolution;
pub use resolution::*;

use std::collections::BTreeMap;

use kernel_types::SemanticId;

use super::{PreparedRewrite, RewriteLawSetId, RewriteSpecId, SharedPreparedRewrite};

#[derive(Debug, PartialEq, Eq)]
pub struct RewriteResidualDiamond<T, I> {
    pub right_after_left: SharedPreparedRewrite<T, I>,
    pub left_after_right: SharedPreparedRewrite<T, I>,
}

impl<T, I> Clone for RewriteResidualDiamond<T, I> {
    fn clone(&self) -> Self {
        Self {
            right_after_left: self.right_after_left.clone(),
            left_after_right: self.left_after_right.clone(),
        }
    }
}

impl<T, I> RewriteResidualDiamond<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.right_after_left.endpoint()
    }
}

/// Proof that the two upper cube paths carry the same exact residual intent.
///
/// The concrete residual rewrite is already owned by an upper face of the
/// enclosing cube certificate, so this marker retains only its family identity
/// instead of duplicating an endpoint-bearing `PreparedRewrite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteCubeCoherence {
    coherent_family: RewriteFamilyIdentity,
}

impl RewriteCubeCoherence {
    #[must_use]
    pub const fn coherent_family(&self) -> RewriteFamilyIdentity {
        self.coherent_family
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteCoherenceError {
    DiamondEndpointMismatch,
    CubeResidualIntentMismatch,
    CubeEndpointMismatch,
    BraidResidualTupleMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteFamilyIdentity {
    pub spec: RewriteSpecId,
    pub law_set: RewriteLawSetId,
}

impl<T, I> From<&PreparedRewrite<T, I>> for RewriteFamilyIdentity {
    fn from(rewrite: &PreparedRewrite<T, I>) -> Self {
        Self {
            spec: rewrite.spec,
            law_set: rewrite.law_set,
        }
    }
}

impl<T, I> From<&SharedPreparedRewrite<T, I>> for RewriteFamilyIdentity {
    fn from(rewrite: &SharedPreparedRewrite<T, I>) -> Self {
        rewrite.as_rewrite().into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteResidualFamilyId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteResidualFamilyKey {
    pub left: RewriteFamilyIdentity,
    pub right: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteResidualFamilySpec {
    pub id: RewriteResidualFamilyId,
    pub key: RewriteResidualFamilyKey,
    pub right_after_left: RewriteFamilyIdentity,
    pub left_after_right: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteResidualFamilyRegistry {
    by_pair: BTreeMap<RewriteResidualFamilyKey, RewriteResidualFamilySpec>,
    by_id: BTreeMap<RewriteResidualFamilyId, RewriteResidualFamilyKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteSequentialFamilyId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteSequentialFamilyKey {
    pub first: RewriteFamilyIdentity,
    pub second: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteSequentialFamilySpec {
    pub id: RewriteSequentialFamilyId,
    pub key: RewriteSequentialFamilyKey,
    pub composite: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteSequentialFamilyRegistry {
    by_pair: BTreeMap<RewriteSequentialFamilyKey, RewriteSequentialFamilySpec>,
    by_id: BTreeMap<RewriteSequentialFamilyId, RewriteSequentialFamilyKey>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RewriteSequentialComposition<T, I> {
    pub first: SharedPreparedRewrite<T, I>,
    pub second: SharedPreparedRewrite<T, I>,
    pub composite: SharedPreparedRewrite<T, I>,
}

impl<T, I> Clone for RewriteSequentialComposition<T, I> {
    fn clone(&self) -> Self {
        Self {
            first: self.first.clone(),
            second: self.second.clone(),
            composite: self.composite.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteSequentialRegistryError {
    FamilyIdConflict,
    PairAlreadyRegistered,
    FamilyNotRegistered,
    InputIdentityMismatch,
    CompositeIdentityMismatch,
    CompositeEndpointMismatch,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RewriteResidualCubeWitness<T, I> {
    pub b_after_a: SharedPreparedRewrite<T, I>,
    pub a_after_b: SharedPreparedRewrite<T, I>,
    pub c_after_a: SharedPreparedRewrite<T, I>,
    pub a_after_c: SharedPreparedRewrite<T, I>,
    pub c_after_b: SharedPreparedRewrite<T, I>,
    pub b_after_c: SharedPreparedRewrite<T, I>,
    pub c_after_ab: SharedPreparedRewrite<T, I>,
    pub b_after_ac: SharedPreparedRewrite<T, I>,
    pub c_after_ba: SharedPreparedRewrite<T, I>,
    pub a_after_bc: SharedPreparedRewrite<T, I>,
    pub b_after_ca: SharedPreparedRewrite<T, I>,
    pub a_after_cb: SharedPreparedRewrite<T, I>,
}

impl<T, I> Clone for RewriteResidualCubeWitness<T, I> {
    fn clone(&self) -> Self {
        Self {
            b_after_a: self.b_after_a.clone(),
            a_after_b: self.a_after_b.clone(),
            c_after_a: self.c_after_a.clone(),
            a_after_c: self.a_after_c.clone(),
            c_after_b: self.c_after_b.clone(),
            b_after_c: self.b_after_c.clone(),
            c_after_ab: self.c_after_ab.clone(),
            b_after_ac: self.b_after_ac.clone(),
            c_after_ba: self.c_after_ba.clone(),
            a_after_bc: self.a_after_bc.clone(),
            b_after_ca: self.b_after_ca.clone(),
            a_after_cb: self.a_after_cb.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RewriteResidualCubeCertificate<T, I> {
    pub ab: RewriteResidualDiamond<T, I>,
    pub ac: RewriteResidualDiamond<T, I>,
    pub bc: RewriteResidualDiamond<T, I>,
    pub after_a: RewriteResidualDiamond<T, I>,
    pub after_b: RewriteResidualDiamond<T, I>,
    pub after_c: RewriteResidualDiamond<T, I>,
    pub cube: RewriteCubeCoherence,
}

impl<T, I> Clone for RewriteResidualCubeCertificate<T, I> {
    fn clone(&self) -> Self {
        Self {
            ab: self.ab.clone(),
            ac: self.ac.clone(),
            bc: self.bc.clone(),
            after_a: self.after_a.clone(),
            after_b: self.after_b.clone(),
            after_c: self.after_c.clone(),
            cube: self.cube,
        }
    }
}

impl<T, I> RewriteResidualCubeCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.after_a.common_endpoint()
    }

    #[must_use]
    pub fn coherent_residual(&self) -> &PreparedRewrite<T, I> {
        &self.after_a.right_after_left
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RewriteResidualBraidCubeCertificate<T, I> {
    pub cube: RewriteResidualCubeCertificate<T, I>,
}

impl<T, I> Clone for RewriteResidualBraidCubeCertificate<T, I> {
    fn clone(&self) -> Self {
        Self {
            cube: self.cube.clone(),
        }
    }
}

impl<T, I> RewriteResidualBraidCubeCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.cube.common_endpoint()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteResidualRegistryError {
    FamilyIdConflict,
    PairAlreadyRegistered,
    FamilyNotRegistered,
    InputIdentityMismatch,
    ResidualIdentityMismatch,
    Coherence(RewriteCoherenceError),
}

impl RewriteResidualFamilyRegistry {
    pub fn register(
        &mut self,
        spec: RewriteResidualFamilySpec,
    ) -> Result<(), RewriteResidualRegistryError> {
        if let Some(existing_key) = self.by_id.get(&spec.id) {
            return if *existing_key == spec.key && self.by_pair.get(&spec.key) == Some(&spec) {
                Ok(())
            } else {
                Err(RewriteResidualRegistryError::FamilyIdConflict)
            };
        }
        if self.by_pair.contains_key(&spec.key) {
            return Err(RewriteResidualRegistryError::PairAlreadyRegistered);
        }
        self.by_id.insert(spec.id, spec.key);
        self.by_pair.insert(spec.key, spec);
        Ok(())
    }

    #[must_use]
    pub fn family(&self, key: RewriteResidualFamilyKey) -> Option<&RewriteResidualFamilySpec> {
        self.by_pair.get(&key)
    }

    pub fn certify<T: PartialEq + Eq, I>(
        &self,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
        right_after_left: PreparedRewrite<T, I>,
        left_after_right: PreparedRewrite<T, I>,
    ) -> Result<RewriteResidualDiamond<T, I>, RewriteResidualRegistryError> {
        self.certify_shared(
            left,
            right,
            right_after_left.into(),
            left_after_right.into(),
        )
    }

    pub fn certify_shared<T: PartialEq + Eq, I>(
        &self,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
        right_after_left: SharedPreparedRewrite<T, I>,
        left_after_right: SharedPreparedRewrite<T, I>,
    ) -> Result<RewriteResidualDiamond<T, I>, RewriteResidualRegistryError> {
        let key = RewriteResidualFamilyKey {
            left: left.into(),
            right: right.into(),
        };
        let family = self
            .by_pair
            .get(&key)
            .ok_or(RewriteResidualRegistryError::FamilyNotRegistered)?;
        if RewriteFamilyIdentity::from(left) != family.key.left
            || RewriteFamilyIdentity::from(right) != family.key.right
        {
            return Err(RewriteResidualRegistryError::InputIdentityMismatch);
        }
        if RewriteFamilyIdentity::from(&right_after_left) != family.right_after_left
            || RewriteFamilyIdentity::from(&left_after_right) != family.left_after_right
        {
            return Err(RewriteResidualRegistryError::ResidualIdentityMismatch);
        }
        certify_shared_residual_diamond(right_after_left, left_after_right)
            .map_err(RewriteResidualRegistryError::Coherence)
    }

    pub fn certify_cube<T: PartialEq + Eq, I: PartialEq + Eq>(
        &self,
        a: &PreparedRewrite<T, I>,
        b: &PreparedRewrite<T, I>,
        c: &PreparedRewrite<T, I>,
        witness: RewriteResidualCubeWitness<T, I>,
    ) -> Result<RewriteResidualCubeCertificate<T, I>, RewriteResidualRegistryError> {
        let RewriteResidualCubeWitness {
            b_after_a,
            a_after_b,
            c_after_a,
            a_after_c,
            c_after_b,
            b_after_c,
            c_after_ab,
            b_after_ac,
            c_after_ba,
            a_after_bc,
            b_after_ca,
            a_after_cb,
        } = witness;

        let cube = certify_cube_coherence(&c_after_ab, &c_after_ba)
            .map_err(RewriteResidualRegistryError::Coherence)?;

        // Upper faces are certified first while the lower residuals are still
        // available by reference. Every prepared rewrite is then moved into
        // exactly one owning certificate; no endpoint-bearing rewrite needs
        // to be cloned merely to retain proof structure.
        let after_a = self.certify_shared(&b_after_a, &c_after_a, c_after_ab, b_after_ac)?;
        let after_b = self.certify_shared(&a_after_b, &c_after_b, c_after_ba, a_after_bc)?;
        let after_c = self.certify_shared(&a_after_c, &b_after_c, b_after_ca, a_after_cb)?;

        if after_a.common_endpoint() != after_b.common_endpoint()
            || after_a.common_endpoint() != after_c.common_endpoint()
        {
            return Err(RewriteResidualRegistryError::Coherence(
                RewriteCoherenceError::CubeEndpointMismatch,
            ));
        }

        let ab = self.certify_shared(a, b, b_after_a, a_after_b)?;
        let ac = self.certify_shared(a, c, c_after_a, a_after_c)?;
        let bc = self.certify_shared(b, c, c_after_b, b_after_c)?;

        Ok(RewriteResidualCubeCertificate {
            ab,
            ac,
            bc,
            after_a,
            after_b,
            after_c,
            cube,
        })
    }

    /// Certifies the full Yang-Baxter residual tuple, not only the historical
    /// cube endpoint/coherent-top-residual condition. This stronger authority
    /// is the local braid critical-pair law required by finite n-ary
    /// normalization.
    pub fn certify_braid_cube<T: PartialEq + Eq, I: PartialEq + Eq>(
        &self,
        a: &PreparedRewrite<T, I>,
        b: &PreparedRewrite<T, I>,
        c: &PreparedRewrite<T, I>,
        witness: RewriteResidualCubeWitness<T, I>,
    ) -> Result<RewriteResidualBraidCubeCertificate<T, I>, RewriteResidualRegistryError> {
        let cube = self.certify_cube(a, b, c, witness)?;
        if cube.after_a.left_after_right != cube.after_c.right_after_left
            || cube.after_b.left_after_right != cube.after_c.left_after_right
        {
            return Err(RewriteResidualRegistryError::Coherence(
                RewriteCoherenceError::BraidResidualTupleMismatch,
            ));
        }
        Ok(RewriteResidualBraidCubeCertificate { cube })
    }
}

impl RewriteSequentialFamilyRegistry {
    pub fn register(
        &mut self,
        spec: RewriteSequentialFamilySpec,
    ) -> Result<(), RewriteSequentialRegistryError> {
        if let Some(existing_key) = self.by_id.get(&spec.id) {
            return if *existing_key == spec.key && self.by_pair.get(&spec.key) == Some(&spec) {
                Ok(())
            } else {
                Err(RewriteSequentialRegistryError::FamilyIdConflict)
            };
        }
        if self.by_pair.contains_key(&spec.key) {
            return Err(RewriteSequentialRegistryError::PairAlreadyRegistered);
        }
        self.by_id.insert(spec.id, spec.key);
        self.by_pair.insert(spec.key, spec);
        Ok(())
    }

    pub fn certify<T: PartialEq + Eq, I>(
        &self,
        first: PreparedRewrite<T, I>,
        second: PreparedRewrite<T, I>,
        composite: PreparedRewrite<T, I>,
    ) -> Result<RewriteSequentialComposition<T, I>, RewriteSequentialRegistryError> {
        self.certify_shared(first.into(), second.into(), composite.into())
    }

    pub fn certify_shared<T: PartialEq + Eq, I>(
        &self,
        first: SharedPreparedRewrite<T, I>,
        second: SharedPreparedRewrite<T, I>,
        composite: SharedPreparedRewrite<T, I>,
    ) -> Result<RewriteSequentialComposition<T, I>, RewriteSequentialRegistryError> {
        let key = RewriteSequentialFamilyKey {
            first: (&first).into(),
            second: (&second).into(),
        };
        let family = self
            .by_pair
            .get(&key)
            .ok_or(RewriteSequentialRegistryError::FamilyNotRegistered)?;
        if RewriteFamilyIdentity::from(&first) != family.key.first
            || RewriteFamilyIdentity::from(&second) != family.key.second
        {
            return Err(RewriteSequentialRegistryError::InputIdentityMismatch);
        }
        if RewriteFamilyIdentity::from(&composite) != family.composite {
            return Err(RewriteSequentialRegistryError::CompositeIdentityMismatch);
        }
        if composite.endpoint() != second.endpoint() {
            return Err(RewriteSequentialRegistryError::CompositeEndpointMismatch);
        }
        Ok(RewriteSequentialComposition {
            first,
            second,
            composite,
        })
    }
}

pub fn certify_residual_diamond<T: PartialEq + Eq, I>(
    right_after_left: PreparedRewrite<T, I>,
    left_after_right: PreparedRewrite<T, I>,
) -> Result<RewriteResidualDiamond<T, I>, RewriteCoherenceError> {
    certify_shared_residual_diamond(right_after_left.into(), left_after_right.into())
}

pub fn certify_shared_residual_diamond<T: PartialEq + Eq, I>(
    right_after_left: SharedPreparedRewrite<T, I>,
    left_after_right: SharedPreparedRewrite<T, I>,
) -> Result<RewriteResidualDiamond<T, I>, RewriteCoherenceError> {
    if right_after_left.endpoint() != left_after_right.endpoint() {
        return Err(RewriteCoherenceError::DiamondEndpointMismatch);
    }
    Ok(RewriteResidualDiamond {
        right_after_left,
        left_after_right,
    })
}

pub fn certify_cube_coherence<T: PartialEq + Eq, I: PartialEq + Eq>(
    through_left_then_right: &PreparedRewrite<T, I>,
    through_right_then_left: &PreparedRewrite<T, I>,
) -> Result<RewriteCubeCoherence, RewriteCoherenceError> {
    if through_left_then_right != through_right_then_left {
        return Err(RewriteCoherenceError::CubeResidualIntentMismatch);
    }
    Ok(RewriteCubeCoherence {
        coherent_family: through_left_then_right.into(),
    })
}
