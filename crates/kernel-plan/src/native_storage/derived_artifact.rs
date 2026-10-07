#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UnifiedArtifactId {
    I64Index(I64IndexBinding),
    SemanticFiber {
        binding: SemanticIndexBinding,
        profile: SemanticFiberProfile,
    },
    SemanticQuotientSupport(crate::semantic_quotient_physical::SemanticQuotientSupportBinding),
}

impl UnifiedArtifactId {
    #[must_use]
    pub(crate) fn semantic_fiber(
        binding: SemanticIndexBinding,
        profile: SemanticFiberProfile,
    ) -> Self {
        Self::SemanticFiber { binding, profile }
    }

    #[must_use]
    pub(crate) fn semantic_cardinality(binding: SemanticIndexBinding) -> Self {
        Self::semantic_fiber(binding, SemanticFiberProfile::Cardinality)
    }

    #[must_use]
    pub(crate) fn semantic_quotient(binding: SemanticIndexBinding) -> Self {
        Self::semantic_fiber(binding, SemanticFiberProfile::Quotient)
    }

    #[must_use]
    pub(crate) fn semantic_observable(binding: SemanticIndexBinding) -> Self {
        Self::semantic_fiber(binding, SemanticFiberProfile::Observable)
    }
}
