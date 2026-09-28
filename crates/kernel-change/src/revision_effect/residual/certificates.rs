use std::collections::{BTreeMap, BTreeSet};

use crate::rewrite::{
    PreparedRewrite, RewriteConcurrentNormalizationCertificate, RewriteCoordinationRegistryError,
    RewriteResidualCubeCertificate, RewriteResidualCubeWitness, RewriteResidualDiamond,
    RewriteResidualRegistryError, RewriteResidualResolutionError, RewriteSequentialComposition,
    RewriteSequentialRegistryError, SharedPreparedRewrite,
};

use super::super::{RevisionEffectCausalLayerSchedule, RevisionEffectId, RevisionEffectIdealError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub diamonds: BTreeMap<(RevisionEffectId, RevisionEffectId), RewriteResidualDiamond<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualResolvedLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentNormalizationCertificate<RevisionEffectId, T, I>,
    pub right_normalization: RewriteConcurrentNormalizationCertificate<RevisionEffectId, T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
}

impl<T, I> RevisionEffectResidualResolvedLayerCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.cross.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualCubeLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub cube: RewriteResidualCubeCertificate<T, I>,
}

impl<T, I> RevisionEffectResidualCubeLayerCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.cube.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentPairWitness<T, I> {
    pub right_after_left: SharedPreparedRewrite<T, I>,
    pub left_after_right: SharedPreparedRewrite<T, I>,
    pub left_then_right_composite: SharedPreparedRewrite<T, I>,
    pub right_then_left_composite: SharedPreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentPairCertificate<T, I> {
    pub diamond: RewriteResidualDiamond<T, I>,
    pub left_then_right: RewriteSequentialComposition<T, I>,
    pub right_then_left: RewriteSequentialComposition<T, I>,
    pub(super) composite: SharedPreparedRewrite<T, I>,
}

impl<T, I> RewriteConcurrentPairCertificate<T, I> {
    #[must_use]
    pub fn composite(&self) -> &PreparedRewrite<T, I> {
        self.composite.as_rewrite()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentTripleWitness<T, I> {
    pub cube: RewriteResidualCubeWitness<T, I>,
    pub ab_composite: SharedPreparedRewrite<T, I>,
    pub ac_composite: SharedPreparedRewrite<T, I>,
    pub bc_composite: SharedPreparedRewrite<T, I>,
    pub final_composite: SharedPreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentTripleCertificate<T, I> {
    pub cube: RewriteResidualCubeCertificate<T, I>,
    pub ab: RewriteConcurrentPairCertificate<T, I>,
    pub ac: RewriteConcurrentPairCertificate<T, I>,
    pub bc: RewriteConcurrentPairCertificate<T, I>,
    pub final_paths: Vec<RewriteSequentialComposition<T, I>>,
    pub(super) composite: SharedPreparedRewrite<T, I>,
}

impl<T, I> RewriteConcurrentTripleCertificate<T, I> {
    #[must_use]
    pub fn composite(&self) -> &PreparedRewrite<T, I> {
        self.composite.as_rewrite()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteConcurrentBranchWitness<T, I> {
    Single,
    Pair(Box<RewriteConcurrentPairWitness<T, I>>),
    Triple(Box<RewriteConcurrentTripleWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteConcurrentBranchCertificate<T, I> {
    Single(SharedPreparedRewrite<T, I>),
    Pair(Box<RewriteConcurrentPairCertificate<T, I>>),
    Triple(Box<RewriteConcurrentTripleCertificate<T, I>>),
}

impl<T, I> RewriteConcurrentBranchCertificate<T, I> {
    #[must_use]
    pub fn composite(&self) -> &PreparedRewrite<T, I> {
        match self {
            Self::Single(rewrite) => rewrite.as_rewrite(),
            Self::Pair(certificate) => certificate.composite(),
            Self::Triple(certificate) => certificate.composite(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualNormalizedLayerWitness<T, I> {
    pub left: RewriteConcurrentBranchWitness<T, I>,
    pub right: RewriteConcurrentBranchWitness<T, I>,
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualNormalizedLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentBranchCertificate<T, I>,
    pub right_normalization: RewriteConcurrentBranchCertificate<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
}

impl<T, I> RevisionEffectResidualNormalizedLayerCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.cross.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareLayerWitness<T, I> {
    pub left: RewriteConcurrentPairWitness<T, I>,
    pub right: RewriteConcurrentPairWitness<T, I>,
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub right_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
}

impl<T, I> RevisionEffectResidualSquareLayerCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.cross.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareChainWitness<T, I> {
    pub first: RevisionEffectResidualSquareLayerWitness<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RevisionEffectResidualSquareLayerCertificate<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepCertificate<T, I>>,
}

impl<T, I> RevisionEffectResidualSquareChainCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        match self.subsequent.last() {
            Some(step) => step.cross.common_endpoint(),
            None => self.first.common_endpoint(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedFirstWitness<T, I> {
    Singleton {
        right_after_left: PreparedRewrite<T, I>,
        left_after_right: PreparedRewrite<T, I>,
    },
    Square(Box<RevisionEffectResidualSquareLayerWitness<T, I>>),
    Normalized(Box<RevisionEffectResidualNormalizedLayerWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedFirstCertificate<T, I> {
    Singleton(RewriteResidualDiamond<T, I>),
    Square(Box<RevisionEffectResidualSquareLayerCertificate<T, I>>),
    Normalized(Box<RevisionEffectResidualNormalizedLayerCertificate<T, I>>),
}

impl<T, I> RevisionEffectResidualMixedFirstCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        match self {
            Self::Singleton(certificate) => certificate.common_endpoint(),
            Self::Square(certificate) => certificate.common_endpoint(),
            Self::Normalized(certificate) => certificate.common_endpoint(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedSquareStepWitness<T, I> {
    pub left: RewriteConcurrentPairWitness<T, I>,
    pub right: RewriteConcurrentPairWitness<T, I>,
    pub transport: RevisionEffectResidualChainStepWitness<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedStepWitness<T, I> {
    Singleton(Box<RevisionEffectResidualChainStepWitness<T, I>>),
    Square(Box<RevisionEffectResidualMixedSquareStepWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedSquareStepCertificate<T, I> {
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub right_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub transport: RevisionEffectResidualChainStepCertificate<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedStepCertificate<T, I> {
    Singleton(Box<RevisionEffectResidualChainStepCertificate<T, I>>),
    Square(Box<RevisionEffectResidualMixedSquareStepCertificate<T, I>>),
}

impl<T, I> RevisionEffectResidualMixedStepCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        match self {
            Self::Singleton(certificate) => certificate.cross.common_endpoint(),
            Self::Square(certificate) => certificate.transport.cross.common_endpoint(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedChainWitness<T, I> {
    pub first: RevisionEffectResidualMixedFirstWitness<T, I>,
    pub subsequent: Vec<RevisionEffectResidualMixedStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RevisionEffectResidualMixedFirstCertificate<T, I>,
    pub subsequent: Vec<RevisionEffectResidualMixedStepCertificate<T, I>>,
}

impl<T, I> RevisionEffectResidualMixedChainCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        match self.subsequent.last() {
            Some(step) => step.common_endpoint(),
            None => self.first.common_endpoint(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectTwoLayerResidualWitness<T, I> {
    pub first_right_after_left: PreparedRewrite<T, I>,
    pub first_left_after_right: PreparedRewrite<T, I>,
    pub right_prefix_after_left_second: PreparedRewrite<T, I>,
    pub left_second_after_right_prefix: PreparedRewrite<T, I>,
    pub left_prefix_after_right_second: PreparedRewrite<T, I>,
    pub right_second_after_left_prefix: PreparedRewrite<T, I>,
    pub second_right_after_left: PreparedRewrite<T, I>,
    pub second_left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectTwoLayerResidualCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RewriteResidualDiamond<T, I>,
    pub left_transport: RewriteResidualDiamond<T, I>,
    pub right_transport: RewriteResidualDiamond<T, I>,
    pub second: RewriteResidualDiamond<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainStepWitness<T, I> {
    pub right_prefix_after_left: PreparedRewrite<T, I>,
    pub left_after_right_prefix: PreparedRewrite<T, I>,
    pub left_prefix_after_right: PreparedRewrite<T, I>,
    pub right_after_left_prefix: PreparedRewrite<T, I>,
    pub cross_right_after_left: PreparedRewrite<T, I>,
    pub cross_left_after_right: PreparedRewrite<T, I>,
    pub cumulative_right_prefix: Option<PreparedRewrite<T, I>>,
    pub cumulative_left_prefix: Option<PreparedRewrite<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainWitness<T, I> {
    pub first_right_after_left: PreparedRewrite<T, I>,
    pub first_left_after_right: PreparedRewrite<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainStepCertificate<T, I> {
    pub left_transport: RewriteResidualDiamond<T, I>,
    pub right_transport: RewriteResidualDiamond<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
    pub cumulative_right_prefix: Option<RewriteSequentialComposition<T, I>>,
    pub cumulative_left_prefix: Option<RewriteSequentialComposition<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RewriteResidualDiamond<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepCertificate<T, I>>,
}

impl<T, I> RevisionEffectResidualChainCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        match self.subsequent.last() {
            Some(step) => step.cross.common_endpoint(),
            None => self.first.common_endpoint(),
        }
    }
}

impl<T, I> RevisionEffectTwoLayerResidualCertificate<T, I> {
    #[must_use]
    pub fn common_endpoint(&self) -> &T {
        self.second.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualLayerError {
    Ideal(RevisionEffectIdealError),
    IntentConflicts(BTreeSet<(RevisionEffectId, RevisionEffectId)>),
    NonFrontierExclusiveEffect(RevisionEffectId),
    MissingResidualPair(RevisionEffectId, RevisionEffectId),
    UnsupportedLayerShape,
    WitnessLayerCountMismatch,
    ConcurrentCompositeIntentMismatch,
    Residual(RewriteResidualRegistryError),
    Sequential(RewriteSequentialRegistryError),
    Coordination(RewriteCoordinationRegistryError),
    Resolution(RewriteResidualResolutionError),
}

impl From<RevisionEffectIdealError> for RevisionEffectResidualLayerError {
    fn from(value: RevisionEffectIdealError) -> Self {
        Self::Ideal(value)
    }
}

impl From<RewriteResidualRegistryError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteResidualRegistryError) -> Self {
        Self::Residual(value)
    }
}

impl From<RewriteSequentialRegistryError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteSequentialRegistryError) -> Self {
        Self::Sequential(value)
    }
}

impl From<RewriteCoordinationRegistryError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteCoordinationRegistryError) -> Self {
        Self::Coordination(value)
    }
}

impl From<RewriteResidualResolutionError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteResidualResolutionError) -> Self {
        Self::Resolution(value)
    }
}
