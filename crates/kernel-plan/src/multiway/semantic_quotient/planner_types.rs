#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticQuotientSearchCertificate {
    GyoAcyclic,
    BoundedCyclic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedSemanticQuotientProgram {
    pub(super) specs: Vec<(SemanticId, Vec<MultiwayJoinColumnRef>)>,
    pub(super) hypergraph_order: Option<Vec<usize>>,
    search_certificate: Option<SemanticQuotientSearchCertificate>,
}
