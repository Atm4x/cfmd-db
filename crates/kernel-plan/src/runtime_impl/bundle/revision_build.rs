impl RuntimeRevisionBundle {
    pub(crate) fn relation_base_witnesses(
        revision: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PersistentOrdMap<SemanticId, RelationBaseWitness>, PhysicalExecutionError> {
        let mut witnesses = PersistentOrdMap::default();
        for definition in revision.semantic_context().schema.relations() {
            let relation = definition.id;
            let result_type = RelExpr::Scan(relation)
                .typecheck(revision.semantic_context(), registry)?;
            let rows = revision
                .state()
                .model
                .relations
                .materialize_owned(&relation)
                .unwrap_or_default();
            witnesses.insert(
                relation,
                RelationBaseWitness::build(
                    revision.id(),
                    relation,
                    &rows,
                    result_type,
                    revision.semantic_context(),
                    registry,
                )?,
            );
        }
        Ok(witnesses)
    }

    fn materialization_scan_seeds(
        query: &RelExpr,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
    ) -> Result<BTreeMap<SemanticId, kernel_query::RelationScanOccurrenceSeed>, PhysicalExecutionError> {
        query
            .scan_relations()
            .into_iter()
            .map(|relation| {
                let witness = relation_bases.get(&relation).ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
                )?;
                Ok((relation, witness.logical_scan_occurrence_seed()?))
            })
            .collect()
    }

    fn materialization_dependency_indexes(
        specs: &PersistentOrdMap<kernel_types::MaterializationId, RelExpr>,
    ) -> (
        PersistentOrdMap<kernel_types::MaterializationId, PersistentOrdSet<SemanticId>>,
        PersistentOrdMap<SemanticId, PersistentOrdSet<kernel_types::MaterializationId>>,
    ) {
        let mut dependencies = PersistentOrdMap::default();
        let mut by_relation: PersistentOrdMap<
            SemanticId,
            PersistentOrdSet<kernel_types::MaterializationId>,
        > = PersistentOrdMap::default();
        for (&id, query) in specs {
            let relation_dependencies = query
                .scan_relations()
                .into_iter()
                .collect::<PersistentOrdSet<_>>();
            for &relation in &relation_dependencies {
                let mut consumers = by_relation.get(&relation).cloned().unwrap_or_default();
                consumers.insert(id);
                by_relation.insert(relation, consumers);
            }
            dependencies.insert(id, relation_dependencies);
        }
        (dependencies, by_relation)
    }

    fn affected_materializations(
        &self,
        changed_relations: impl IntoIterator<Item = SemanticId>,
    ) -> PersistentOrdSet<kernel_types::MaterializationId> {
        let mut affected = PersistentOrdSet::default();
        for relation in changed_relations {
            if let Some(consumers) = self.materializations_by_relation.get(&relation) {
                affected.extend(consumers.iter().copied());
            }
        }
        affected
    }



    fn candidate_violation_state_for_relation_transition(
        &self,
        request: &RevisionTransitionRequest<'_>,
        validate_target_endpoint: bool,
    ) -> Result<RuntimeViolationState, PhysicalExecutionError> {
        if !validate_target_endpoint {
            // `ExactDerived` targets are built from this validated source by
            // the revision-level exact-delta certificate path. Their structural
            // and model-rule validity has already been paid at construction, so
            // rebuilding a row-level VMF here would duplicate O(N) work.
            self.violation_state.require_zero()?;
            return Ok(RuntimeViolationState::from_measure(
                request.target_revision,
                kernel_violation::ViolationMeasure::new(),
                request.target_revision.model_rule_witnesses().clone(),
            ));
        }
        RuntimeViolationState::candidate_for_relation_transition(
            &self.violation_state,
            request.target_revision,
            request.mutations,
            request.registry,
        )
    }

    /// Returns a revision-bound Γ-VMF invariant-closure certificate only when
    /// the runtime's exact maintained violation measure is zero and bound to
    /// the same authoritative revision.
    pub fn invariant_closure_certificate(
        &self,
    ) -> Result<RuntimeInvariantClosureCertificate, PhysicalExecutionError> {
        self.violation_state.require_bound_to(&self.revision)?;
        self.violation_state.closure_certificate()
    }

    pub fn build(
        revision: kernel_revision::Revision,
        physical: PhysicalStore,
        relation_layouts: BTreeMap<SemanticId, LayoutBinding>,
        materialization_specs: &[RuntimeMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let root_id = allocate_runtime_root_id()?;
        Self::build_with_identity(
            revision,
            physical,
            relation_layouts,
            materialization_specs,
            registry,
            RuntimeRootIdentity {
                root_id,
                version: RuntimeRootVersion(0),
            },
        )
    }

    fn build_with_identity(
        revision: kernel_revision::Revision,
        mut physical: PhysicalStore,
        relation_layouts: BTreeMap<SemanticId, LayoutBinding>,
        materialization_specs: &[RuntimeMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
        root_identity: RuntimeRootIdentity,
    ) -> Result<Self, PhysicalExecutionError> {
        Self::validate_physical_snapshot(&revision, &physical, &relation_layouts, registry)?;
        let violation_state = RuntimeViolationState::build(&revision, registry)?;
        violation_state.require_zero()?;
        let relation_bases = Self::relation_base_witnesses(&revision, registry)?;
        // Removal-heavy commits must not pay an O(N) first-write index build.
        // The full-row occurrence directory is reconstructible write-acceleration
        // authority, so build/restore it once with the runtime root and maintain
        // it incrementally thereafter.
        for (&relation, &layout) in &relation_layouts {
            physical.ensure_relation_write_occurrence_atom(
                relation,
                layout,
                revision.semantic_context(),
                registry,
            )?;
        }
        physical.bind_revision(revision.id())?;

        let mut materializations = PersistentOrdMap::default();
        let mut materialization_spec_map = PersistentOrdMap::default();
        for spec in materialization_specs {
            if materializations.contains_key(&spec.id) {
                return Err(PhysicalExecutionError::DuplicateMaterialization(spec.id));
            }
            let scan_seeds = Self::materialization_scan_seeds(&spec.query, &relation_bases)?;
            let mut maintained = MaterializedRelPlanState::build_with_scan_seeds(
                &spec.query,
                &revision.state().model,
                revision.semantic_context(),
                registry,
                &scan_seeds,
            )?;
            for relation in maintained.scan_relations() {
                let layout = relation_layouts.get(&relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
                )?;
                let rows = physical.logical_rows_with_handles(relation, layout)?;
                maintained.attach_storage_rows(relation, &rows)?;
            }
            maintained.bind_revision(revision.id())?;
            materialization_spec_map.insert(spec.id, spec.query.clone());
            materializations.insert(spec.id, maintained);
        }

        let (materialization_dependencies, materializations_by_relation) =
            Self::materialization_dependency_indexes(&materialization_spec_map);
        let historical = RuntimeHistoricalDerivedIndex::default();
        Ok(Self {
            root_identity,
            revision,
            violation_state,
            physical,
            relation_layouts: relation_layouts.into_iter().collect(),
            relation_bases,
            historical,
            materialization_specs: materialization_spec_map,
            materializations,
            materialization_dependencies,
            materializations_by_relation,
        })
    }

    fn validate_physical_snapshot(
        revision: &kernel_revision::Revision,
        physical: &PhysicalStore,
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        if physical.revision().is_some() {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        let schema_relations = revision
            .semantic_context()
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect::<BTreeSet<_>>();
        if relation_layouts.len() != physical.installed_relation_count()
            || relation_layouts.keys().copied().collect::<BTreeSet<_>>() != schema_relations
        {
            return Err(PhysicalExecutionError::PhysicalRelationRegistryMismatch);
        }

        for relation in schema_relations {
            let logical_rows = revision
                .state()
                .model
                .relations
                .materialize_owned(&relation)
                .unwrap_or_default();
            let layout = relation_layouts.get(&relation).copied().ok_or(
                PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
            )?;
            let installed = physical.installed(relation, layout)?;
            let rows = installed
                .scan_positions()
                .map(|position| materialize_native_row(&installed.data, position))
                .collect::<Result<Vec<_>, _>>()?;
            let result_type =
                RelExpr::Scan(relation).typecheck(revision.semantic_context(), registry)?;
            let to_exact_value = |rows| match &result_type.semantics {
                kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                } => RelationValue::Set {
                    rows,
                    column_equivalences: column_equivalences.clone(),
                },
            };
            let physical_value = to_exact_value(rows);
            let logical_value = to_exact_value(logical_rows);
            if physical_value != logical_value {
                return Err(PhysicalExecutionError::LogicalPhysicalStateMismatch(
                    relation,
                ));
            }
        }

        for (&relation, &layout) in relation_layouts {
            if !physical.contains_installed_relation(relation, layout.id) {
                return Err(PhysicalExecutionError::PhysicalRelationRegistryMismatch);
            }
        }
        Ok(())
    }

}
