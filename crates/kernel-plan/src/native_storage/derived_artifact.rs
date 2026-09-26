#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UnifiedArtifactId {
    I64Index(I64IndexBinding),
    SemanticIndex(SemanticIndexBinding),
    ObservableAtom(SemanticIndexBinding),
    SemanticQuotientFactor(SemanticIndexBinding),
    SemanticQuotientSupport(crate::semantic_quotient_physical::SemanticQuotientSupportBinding),
    SemanticStatistics(SemanticIndexBinding),
}
