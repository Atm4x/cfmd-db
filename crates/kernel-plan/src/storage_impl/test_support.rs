// HOSTILE[P170][TEST-ONLY][CLEAN:P169.T]: legacy root tests cross the storage owner only through this cfg(test) facade; production internals stay private.
use super::{
    Arc, I64IndexBinding, InstalledRelation, LayoutId, MaterializedI64IndexState,
    MaterializedObservableAtomState, MaterializedSemanticIndexState,
    MaterializedSemanticQuotientFactorState, MaterializedSemanticQuotientSupportState,
    PersistentOrdMap, PersistentOrdSet, PhysicalExecutionError, PhysicalRowId, PhysicalStore,
    RevisionId, SemanticId, SemanticIndexBinding, SemanticQuotientSupportBinding,
    UnifiedArtifactId,
};

/// Test-only access to storage-owner internals that were historically reachable because
/// `kernel-plan` lived in one monolithic module. Production code must not depend on this trait.
pub(crate) trait PhysicalStoreTestExt {
    fn relations_for_test(
        &self,
    ) -> &PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>>;
    fn relations_mut_for_test(
        &mut self,
    ) -> &mut PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>>;
    fn i64_indexes_for_test(
        &self,
    ) -> &PersistentOrdMap<I64IndexBinding, Arc<MaterializedI64IndexState>>;
    fn semantic_indexes_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticIndexState>>;
    fn semantic_quotient_factors_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticQuotientFactorState>>;
    fn semantic_quotient_supports_for_test(
        &self,
    ) -> &PersistentOrdMap<
        SemanticQuotientSupportBinding,
        Arc<MaterializedSemanticQuotientSupportState>,
    >;
    fn has_semantic_statistics_for_test(&self, binding: &SemanticIndexBinding) -> bool;
    fn semantic_statistics_empty_for_test(&self) -> bool;
    fn shares_semantic_statistics_root_for_test(&self, other: &PhysicalStore) -> bool;
    fn observable_atom_states_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedObservableAtomState>>;
    fn row_occurrence_atoms_for_test(
        &self,
    ) -> &PersistentOrdMap<(SemanticId, LayoutId), Arc<MaterializedObservableAtomState>>;
    fn advisor_managed_artifacts_for_test(&self) -> &PersistentOrdSet<UnifiedArtifactId>;
    fn derived_artifact_ids_for_test(
        &self,
        relation: SemanticId,
        layout: LayoutId,
    ) -> Vec<UnifiedArtifactId>;
    fn derived_artifact_cache_initialized_for_test(&self) -> bool;
    fn shares_derived_artifact_cache_for_test(&self, other: &PhysicalStore) -> bool;
    fn set_revision_for_test(&mut self, revision: Option<RevisionId>);

    fn relation_entry_mut(
        &mut self,
        key: (SemanticId, LayoutId),
    ) -> Option<&mut Arc<InstalledRelation>>;

    fn i64_index(&self, binding: I64IndexBinding) -> Option<&MaterializedI64IndexState>;
    fn remove_i64_index(&mut self, binding: I64IndexBinding);
    fn remove_semantic_index(&mut self, binding: &SemanticIndexBinding);
    fn remove_semantic_statistics(&mut self, binding: &SemanticIndexBinding);
    fn remove_observable_atom_state(&mut self, binding: &SemanticIndexBinding);
    fn set_semantic_statistics_row_count(
        &mut self,
        binding: &SemanticIndexBinding,
        row_count: usize,
    );

    fn semantic_index(
        &self,
        index: &SemanticIndexBinding,
    ) -> Option<&MaterializedSemanticIndexState>;

    fn semantic_quotient_factor(
        &self,
        index: &SemanticIndexBinding,
    ) -> Option<&MaterializedSemanticQuotientFactorState>;
}

impl PhysicalStoreTestExt for PhysicalStore {
    fn relations_for_test(
        &self,
    ) -> &PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>> {
        &self.relations
    }

    fn relations_mut_for_test(
        &mut self,
    ) -> &mut PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>> {
        &mut self.relations
    }

    fn i64_indexes_for_test(
        &self,
    ) -> &PersistentOrdMap<I64IndexBinding, Arc<MaterializedI64IndexState>> {
        &self.i64_indexes
    }

    fn semantic_indexes_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticIndexState>> {
        &self.semantic_indexes
    }

    fn semantic_quotient_factors_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticQuotientFactorState>> {
        &self.semantic_quotient_factors
    }

    fn semantic_quotient_supports_for_test(
        &self,
    ) -> &PersistentOrdMap<
        SemanticQuotientSupportBinding,
        Arc<MaterializedSemanticQuotientSupportState>,
    > {
        &self.semantic_quotient_supports
    }

    fn has_semantic_statistics_for_test(&self, binding: &SemanticIndexBinding) -> bool {
        self.semantic_statistics.contains_key(binding)
    }

    fn semantic_statistics_empty_for_test(&self) -> bool {
        self.semantic_statistics.is_empty()
    }

    fn shares_semantic_statistics_root_for_test(&self, other: &PhysicalStore) -> bool {
        self.semantic_statistics
            .shares_root_with(&other.semantic_statistics)
    }

    fn observable_atom_states_for_test(
        &self,
    ) -> &PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedObservableAtomState>> {
        &self.observable_atom_states
    }

    fn row_occurrence_atoms_for_test(
        &self,
    ) -> &PersistentOrdMap<(SemanticId, LayoutId), Arc<MaterializedObservableAtomState>> {
        &self.row_occurrence_atoms
    }

    fn advisor_managed_artifacts_for_test(&self) -> &PersistentOrdSet<UnifiedArtifactId> {
        &self.advisor_managed_artifacts
    }

    fn derived_artifact_ids_for_test(
        &self,
        relation: SemanticId,
        layout: LayoutId,
    ) -> Vec<UnifiedArtifactId> {
        self.derived_artifacts_by_relation_layout()
            .get(&(relation, layout))
            .into_iter()
            .flatten()
            .filter_map(|target| match target {
                super::DerivedArtifactTarget::Artifact(id) => Some(id.clone()),
                super::DerivedArtifactTarget::RowOccurrenceAtom(_, _) => None,
            })
            .collect()
    }

    fn derived_artifact_cache_initialized_for_test(&self) -> bool {
        self.derived_artifacts_by_relation_layout.0.get().is_some()
    }

    fn shares_derived_artifact_cache_for_test(&self, other: &PhysicalStore) -> bool {
        match (
            self.derived_artifacts_by_relation_layout.0.get(),
            other.derived_artifacts_by_relation_layout.0.get(),
        ) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }

    fn set_revision_for_test(&mut self, revision: Option<RevisionId>) {
        self.revision = revision;
    }

    fn relation_entry_mut(
        &mut self,
        key: (SemanticId, LayoutId),
    ) -> Option<&mut Arc<InstalledRelation>> {
        self.relations_mut_internal().get_mut(&key)
    }

    fn i64_index(&self, binding: I64IndexBinding) -> Option<&MaterializedI64IndexState> {
        self.i64_indexes.get(&binding).map(Arc::as_ref)
    }

    fn remove_i64_index(&mut self, binding: I64IndexBinding) {
        self.i64_indexes_mut_internal().remove(&binding);
    }

    fn remove_semantic_index(&mut self, binding: &SemanticIndexBinding) {
        self.semantic_indexes_mut_internal().remove(binding);
    }

    fn remove_semantic_statistics(&mut self, binding: &SemanticIndexBinding) {
        self.semantic_statistics_mut_internal().remove(binding);
    }

    fn remove_observable_atom_state(&mut self, binding: &SemanticIndexBinding) {
        self.observable_atom_states_mut_internal().remove(binding);
    }

    fn set_semantic_statistics_row_count(
        &mut self,
        binding: &SemanticIndexBinding,
        row_count: usize,
    ) {
        let state = self
            .semantic_statistics_mut_internal()
            .get_mut(binding)
            .expect("test semantic statistics binding");
        Arc::make_mut(state).row_count = row_count;
    }

    fn semantic_index(
        &self,
        index: &SemanticIndexBinding,
    ) -> Option<&MaterializedSemanticIndexState> {
        self.semantic_indexes.get(index).map(Arc::as_ref)
    }

    fn semantic_quotient_factor(
        &self,
        index: &SemanticIndexBinding,
    ) -> Option<&MaterializedSemanticQuotientFactorState> {
        self.semantic_quotient_factors.get(index).map(Arc::as_ref)
    }
}

/// Test-only semantic-index probing without widening the production index API.
pub(crate) trait SemanticIndexStateTestExt {
    fn probe_value(
        &self,
        value: &kernel_model::Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<&kernel_semantic_index::SemanticBucket<PhysicalRowId>>, PhysicalExecutionError>;
}

impl SemanticIndexStateTestExt for MaterializedSemanticIndexState {
    fn probe_value(
        &self,
        value: &kernel_model::Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<&kernel_semantic_index::SemanticBucket<PhysicalRowId>>, PhysicalExecutionError>
    {
        self.probe_values(&[value], context, registry)
    }
}

pub(crate) trait I64IndexStateTestExt {
    fn bucket_keys(&self) -> Vec<i64>;
    fn probe_len_for_test(&self, key: i64) -> Option<usize>;
}

impl I64IndexStateTestExt for MaterializedI64IndexState {
    fn bucket_keys(&self) -> Vec<i64> {
        self.buckets.keys().copied().collect()
    }

    fn probe_len_for_test(&self, key: i64) -> Option<usize> {
        self.probe(key).map(super::OrderedPhysicalRowBucket::len)
    }
}

pub(crate) fn build_i64_index_state_for_test(
    binding: I64IndexBinding,
    relation: &InstalledRelation,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaterializedI64IndexState, PhysicalExecutionError> {
    MaterializedI64IndexState::build(binding, relation, context, registry)
}

pub(crate) fn semantic_index_estimated_retained_bytes(
    state: &MaterializedSemanticIndexState,
) -> usize {
    super::semantic_index_estimated_retained_bytes(state)
}

pub(crate) fn native_semantic_column_work_units(
    data: &super::NativeRelation,
    column: usize,
) -> Result<usize, PhysicalExecutionError> {
    super::native_semantic_column_work_units(data, column)
}
