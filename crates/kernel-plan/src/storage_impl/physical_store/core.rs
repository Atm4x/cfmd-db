impl PhysicalStore {
    #[must_use]
    pub(super) fn installed_relation_count(&self) -> usize {
        self.relations.len()
    }

    #[must_use]
    pub(super) fn contains_installed_relation(
        &self,
        relation: SemanticId,
        layout: LayoutId,
    ) -> bool {
        self.relations.contains_key(&(relation, layout))
    }

    fn invalidate_derived_artifact_dependencies(&mut self) {
        self.derived_artifacts_by_relation_layout.0.take();
    }

    fn derived_artifacts_by_relation_layout(&self) -> &Arc<DerivedArtifactDependencyMap> {
        self.derived_artifacts_by_relation_layout.0.get_or_init(|| {
            let mut by_relation_layout =
                BTreeMap::<(SemanticId, LayoutId), BTreeSet<DerivedArtifactTarget>>::new();
            let mut register = |key, target| {
                by_relation_layout.entry(key).or_default().insert(target);
            };

            for binding in self.i64_indexes.keys() {
                register(
                    (binding.relation, binding.layout.id),
                    DerivedArtifactTarget::Artifact(UnifiedArtifactId::I64Index(*binding)),
                );
            }
            for binding in self.semantic_indexes.keys() {
                register(
                    (binding.relation, binding.layout.id),
                    DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticIndex(
                        binding.clone(),
                    )),
                );
            }
            for binding in self.semantic_quotient_factors.keys() {
                register(
                    (binding.relation, binding.layout.id),
                    DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientFactor(
                        binding.clone(),
                    )),
                );
            }
            for binding in self.semantic_statistics.keys() {
                register(
                    (binding.relation, binding.layout.id),
                    DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticStatistics(
                        binding.clone(),
                    )),
                );
            }
            for binding in self.observable_atom_states.keys() {
                register(
                    (binding.relation, binding.layout.id),
                    DerivedArtifactTarget::Artifact(UnifiedArtifactId::ObservableAtom(
                        binding.clone(),
                    )),
                );
            }
            for binding in self.semantic_quotient_supports.keys() {
                for leaf in &binding.leaves {
                    register(
                        (leaf.relation, leaf.layout.id),
                        DerivedArtifactTarget::Artifact(
                            UnifiedArtifactId::SemanticQuotientSupport(binding.clone()),
                        ),
                    );
                }
            }
            for &(relation, layout) in self.row_occurrence_atoms.keys() {
                register(
                    (relation, layout),
                    DerivedArtifactTarget::RowOccurrenceAtom(relation, layout),
                );
            }

            Arc::new(
                by_relation_layout
                    .into_iter()
                    .map(|(key, targets)| (key, targets.into_iter().collect()))
                    .collect(),
            )
        })
    }

    fn derived_artifact_targets(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> &[DerivedArtifactTarget] {
        self.derived_artifacts_by_relation_layout()
            .get(&(relation, layout.id))
            .map_or(&[], Vec::as_slice)
    }

    fn relations_mut_internal(
        &mut self,
    ) -> &mut PersistentOrdMap<(SemanticId, LayoutId), Arc<InstalledRelation>> {
        &mut self.relations
    }

    fn i64_indexes_mut_internal(
        &mut self,
    ) -> &mut PersistentOrdMap<I64IndexBinding, Arc<MaterializedI64IndexState>> {
        self.invalidate_derived_artifact_dependencies();
        &mut self.i64_indexes
    }

    fn semantic_indexes_mut_internal(
        &mut self,
    ) -> &mut PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticIndexState>> {
        self.invalidate_derived_artifact_dependencies();
        &mut self.semantic_indexes
    }

    pub(super) fn semantic_quotient_factors_mut(
        &mut self,
    ) -> &mut PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticQuotientFactorState>>
    {
        self.invalidate_derived_artifact_dependencies();
        &mut self.semantic_quotient_factors
    }

     fn semantic_quotient_supports_mut(
        &mut self,
    ) -> &mut PersistentOrdMap<
        SemanticQuotientSupportBinding,
        Arc<MaterializedSemanticQuotientSupportState>,
    > {
        self.invalidate_derived_artifact_dependencies();
        &mut self.semantic_quotient_supports
    }

    fn semantic_statistics_mut_internal(
        &mut self,
    ) -> &mut PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedSemanticStatisticsState>> {
        self.invalidate_derived_artifact_dependencies();
        &mut self.semantic_statistics
    }

    fn observable_atom_states_mut_internal(
        &mut self,
    ) -> &mut PersistentOrdMap<SemanticIndexBinding, Arc<MaterializedObservableAtomState>> {
        self.invalidate_derived_artifact_dependencies();
        &mut self.observable_atom_states
    }

    fn row_occurrence_atoms_mut(
        &mut self,
    ) -> &mut PersistentOrdMap<(SemanticId, LayoutId), Arc<MaterializedObservableAtomState>> {
        self.invalidate_derived_artifact_dependencies();
        &mut self.row_occurrence_atoms
    }

    pub(super) fn advisor_managed_artifacts_mut(&mut self) -> &mut PersistentOrdSet<UnifiedArtifactId> {
        &mut self.advisor_managed_artifacts
    }

    // HOSTILE[P171][ACTIVE][CLEAN]: sibling execution/preparation modules consume semantic
    // capabilities through this owner facade rather than reading PhysicalStore directories.
    pub(super) fn semantic_fiber_bindings_for(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> BTreeSet<SemanticIndexBinding> {
        self.semantic_indexes
            .keys()
            .chain(self.observable_atom_states.keys())
            .filter(|binding| binding.relation == relation && binding.layout == layout)
            .cloned()
            .collect()
    }

}
