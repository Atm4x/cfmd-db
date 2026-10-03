// HOSTILE[P173][TEST-ONLY][CLEAN]: root hostile tests use owner actions/probes, never production fields.

impl RuntimeRevisionBundle {
    pub(crate) fn clone_for_test(&self) -> Self {
        Self {
            root_identity: self.root_identity,
            revision: self.revision.clone(),
            violation_state: self.violation_state.clone(),
            physical: self.physical.clone(),
            relation_layouts: self.relation_layouts.clone(),
            relation_bases: self.relation_bases.clone(),
            historical: self.historical.clone(),
            materialization_specs: self.materialization_specs.clone(),
            materializations: self.materializations.clone(),
            materialization_dependencies: self.materialization_dependencies.clone(),
            materializations_by_relation: self.materializations_by_relation.clone(),
        }
    }

    pub(crate) fn prepare_revision_for_test(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.prepare_revision(request)
    }

    pub(crate) fn prepare_revision_derived_for_test(
        &self,
        revision: kernel_revision::Revision,
        exact_deltas: BTreeMap<SemanticId, RelationDelta>,
        mutations: &[RevisionRelationMutation<'_>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let endpoint = DerivedRelationEndpoint {
            revision,
            exact_deltas,
        };
        self.prepare_revision_derived(&endpoint, mutations, registry)
    }

    pub(crate) fn prepare_rewrites_for_test<I>(
        &self,
        request: &RevisionRewriteTransitionRequest<'_, I>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.prepare_rewrites(request)
    }

    pub(crate) fn prepare_rewrites_derived_for_test<I>(
        &self,
        revision: kernel_revision::Revision,
        exact_deltas: BTreeMap<SemanticId, RelationDelta>,
        rewrites: &[RevisionRelationRewrite<'_, I>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let endpoint = DerivedRelationEndpoint {
            revision,
            exact_deltas,
        };
        self.prepare_rewrites_derived(&endpoint, rewrites, registry)
    }
}

impl RuntimeRevisionSnapshot {
    pub(crate) fn historical_exact_effect_count_for_test(&self) -> usize {
        self.root.historical.exact_effects.len()
    }

    pub(crate) fn historical_lineage_floor_for_test(&self) -> Option<RevisionId> {
        self.root.historical.lineage_floor
    }

    pub(crate) fn historical_exact_effect_storage_probe_for_test(
        &self,
    ) -> kernel_persistent::PersistentOrdMapStorageProbe<RevisionId, u128> {
        self.root
            .historical
            .exact_effects
            .unique_storage_probe_against(&PersistentOrdMap::default())
    }
}
impl RuntimeRevisionBundle {
    pub(crate) fn physical_store_mut_for_test(&mut self) -> &mut PhysicalStore {
        &mut self.physical
    }

    pub(crate) fn insert_relation_layout_for_test(
        &mut self,
        relation: SemanticId,
        binding: LayoutBinding,
    ) {
        self.relation_layouts.insert(relation, binding);
    }

    pub(crate) fn relation_layouts_share_root_with_for_test(&self, other: &Self) -> bool {
        self.relation_layouts
            .shares_root_with(&other.relation_layouts)
    }

    pub(crate) fn materialization_specs_share_root_with_for_test(&self, other: &Self) -> bool {
        self.materialization_specs
            .shares_root_with(&other.materialization_specs)
    }

    pub(crate) fn materializations_share_root_with_for_test(&self, other: &Self) -> bool {
        self.materializations
            .shares_root_with(&other.materializations)
    }

    pub(crate) fn materialization_dependency_contains_for_test(
        &self,
        relation: SemanticId,
        id: kernel_types::MaterializationId,
    ) -> bool {
        self.materializations_by_relation
            .get(&relation)
            .is_some_and(|consumers| consumers.contains(&id))
    }
}

impl PreparedRuntimeRevisionTransition {
    pub(crate) fn candidate_materialization_for_test(
        &self,
        id: kernel_types::MaterializationId,
    ) -> Option<&MaterializedRelPlanState> {
        self.candidate.materialization(id)
    }

    pub(crate) fn candidate_materialization_revision_for_test(
        &self,
        id: kernel_types::MaterializationId,
    ) -> Option<RevisionId> {
        self.candidate.materialization_revision(id)
    }

    pub(crate) fn candidate_directories_share_with_for_test(
        &self,
        source: &RuntimeRevisionBundle,
    ) -> (bool, bool, bool) {
        (
            source
                .relation_layouts
                .shares_root_with(&self.candidate.relation_layouts),
            source
                .materialization_specs
                .shares_root_with(&self.candidate.materialization_specs),
            source
                .materializations
                .shares_root_with(&self.candidate.materializations),
        )
    }

    pub(crate) fn candidate_invariant_closure_certificate_for_test(
        &self,
    ) -> Result<RuntimeInvariantClosureCertificate, PhysicalExecutionError> {
        self.candidate.invariant_closure_certificate()
    }

    pub(crate) fn inject_candidate_violation_for_test(
        &mut self,
        witness: kernel_validation::DynamicViolationWitness,
        mass: u64,
    ) {
        self.candidate
            .violation_state
            .measure_mut_for_test()
            .add(witness, mass)
            .expect("test violation injection must remain representable");
    }
}

impl RuntimeRevisionCell {
    pub(crate) fn prepare_revision_for_test(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.prepare_revision(request)
    }
}

impl DurableRuntime {
    pub(crate) fn with_durability_for_test<R>(
        &self,
        inspect: impl FnOnce(&DurableRevisionStore) -> R,
    ) -> R {
        let durability = self
            .durability
            .lock()
            .expect("test durability lock must not be poisoned");
        inspect(&durability)
    }
}

impl DurableRuntimeSupervisor {
    pub(crate) fn force_live_runtime_recovery_required_for_test(&self) {
        let slot = self
            .runtime
            .lock()
            .expect("test supervisor runtime lock must not be poisoned");
        slot.as_ref()
            .expect("test supervisor must own a live runtime")
            .cell
            .force_recovery_required()
            .expect("test recovery-required transition must succeed");
    }

    pub(crate) fn poison_runtime_slot_for_test(supervisor: Arc<Self>) {
        let _ = std::thread::spawn(move || {
            let _guard = supervisor.runtime.lock().expect("test lock starts healthy");
            panic!("intentional reconstructible runtime poison");
        })
        .join();
    }

    pub(crate) fn runtime_slot_is_poisoned_for_test(&self) -> bool {
        self.runtime.is_poisoned()
    }
}


pub(crate) fn benchmark_retained_epoch_first_conflict_for_test(
    depth: u64,
    iterations: u64,
) -> std::time::Duration {
    let coordinate = RuntimeHistoryCoordinate::Field {
        field: SemanticId::new(99_001),
        owner: kernel_types::EntityId::new(99_002),
    };
    let mut index = RuntimeRetainedEpochIndex::default();
    index.lineage_floor = Some(RevisionId::new(1));
    let mut timeline = PersistentOrdMap::default();
    for revision in 2..=depth + 1 {
        timeline.insert(
            RevisionId::new(revision),
            RuntimeIndexedHistoryAction {
                effect_id: u128::from(revision),
                action: RewriteActionLaw::Opaque,
            },
        );
    }
    index.writes.insert(coordinate.clone(), timeline);

    let started = std::time::Instant::now();
    let mut checksum = 0_u128;
    for _ in 0..iterations {
        checksum ^= index
            .first_action_after(RevisionId::new(1), &coordinate)
            .expect("indexed conflict")
            .effect_id;
    }
    std::hint::black_box(checksum);
    started.elapsed()
}
