#[derive(Debug, Clone, Default, PartialEq, Eq)]
// HOSTILE[P165][ACTIVE][PRIMARY]: PhysicalStore is the authoritative physical snapshot for one
// published revision. Reconstructible indexes/artifacts live here; logical authority remains in
// RuntimeRevisionBundle::revision and publication swaps the bundle atomically.
pub struct PhysicalStore {
    relations: PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>>,
    i64_indexes: PersistentOrdMap<I64IndexBinding, Arc<MaterializedI64IndexState>>,
    semantic_indexes: PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticIndexState>>,
    semantic_quotient_factors:
        PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticQuotientFactorState>>,
    semantic_quotient_supports: PersistentOrdMap<
        SemanticQuotientSupportBinding,
        Arc<MaterializedSemanticQuotientSupportState>,
    >,
    semantic_statistics:
        PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticStatisticsState>>,
    observable_atom_states:
        PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedObservableAtomState>>,
    // Internal write-acceleration substrate. These full-row Γ occurrence directories are
    // reconstructible and intentionally excluded from advisor/durable artifact identity.
    row_occurrence_atoms:
        PersistentOrdMap<(SemanticId, LayoutId), Arc<MaterializedObservableAtomState>>,
    derived_artifacts_by_relation_layout: DerivedArtifactDependencyCache,
    advisor_managed_artifacts: PersistentOrdSet<UnifiedArtifactId>,
    semantic_quotient_support_local_delta_updates: u64,
    transition_epoch: u64,
    revision: Option<RevisionId>,
    state_identity: Arc<()>,
}

/// Correctness-first storage prepare result. The retained source snapshot makes
/// freshness exact even for two different stores with equal epoch/revision counters.
#[derive(Debug, PartialEq, Eq)]
struct PreparedPhysicalStoreTransition {
    source_epoch: u64,
    source_revision: Option<RevisionId>,
    source_identity: Arc<()>,
    candidate: Box<PhysicalStore>,
    resolved_delta: StorageResolvedRelationDelta,
}

impl PreparedPhysicalStoreTransition {
    fn into_candidate_if_current(
        self,
        current: &PhysicalStore,
    ) -> Result<(PhysicalStore, StorageResolvedRelationDelta), PhysicalExecutionError> {
        if !current.can_commit_prepared(&self) {
            return Err(PhysicalExecutionError::StalePreparedTransition);
        }
        Ok((*self.candidate, self.resolved_delta))
    }
}

