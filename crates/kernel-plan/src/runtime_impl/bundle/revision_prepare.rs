impl RuntimeRevisionBundle {
    fn durable_artifact_cores(&self) -> Result<Vec<DurableArtifactCore>, PhysicalExecutionError> {
        self.physical.durable_artifact_cores(self.revision.id())
    }

    fn candidate_physical_store_for_revision(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<
        (
            PhysicalStore,
            BTreeMap<SemanticId, StorageResolvedRelationDelta>,
        ),
        PhysicalExecutionError,
    > {
        let mut candidate_store = self.physical.clone();
        let mut resolved = BTreeMap::new();
        let mut physical_changes = Vec::with_capacity(request.mutations.len());
        for mutation in request.mutations {
            let layout = self.relation_layouts[&mutation.relation];
            let (delta, physical_delta) = candidate_store
                .apply_relation_delta_resolved_in_place_deferred_support(
                    mutation.relation,
                    layout,
                    mutation.delta,
                    self.revision.semantic_context(),
                    request.registry,
                )?;
            physical_changes.push((mutation.relation, layout, physical_delta));
            resolved.insert(mutation.relation, delta);
        }
        let support_changes = physical_changes
            .iter()
            .map(|(relation, layout, delta)| (*relation, *layout, delta))
            .collect::<Vec<_>>();
        candidate_store.maintain_semantic_quotient_supports_for_changes(
            &support_changes,
            self.revision.semantic_context(),
            request.registry,
        )?;
        Ok((candidate_store, resolved))
    }

    fn prepare_revision(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.prepare_revision_inner(request, RelationEndpointValidation::ReplayClaimedDelta)
    }

    /// Prepares a target that was derived by this runtime from this exact
    /// immutable source bundle plus the same normalized relation deltas.
    ///
    /// The ordinary arbitrary-target path must re-evaluate the logical endpoint.
    /// The derived path already established that endpoint by construction, so
    /// repeating it would clone/materialize every touched relation a second time.
    /// This method is intentionally private and must only be invoked on the same
    /// retained source snapshot that was used to derive `target_revision`.
    fn prepare_revision_derived(
        &self,
        endpoint: &DerivedRelationEndpoint,
        mutations: &[RevisionRelationMutation<'_>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        if !endpoint.certifies_mutations(mutations) {
            return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
        }
        let touched_relations = mutations
            .iter()
            .map(|mutation| mutation.relation)
            .collect::<BTreeSet<_>>();
        if !endpoint
            .revision()
            .certifies_relation_only_from(&self.revision, &touched_relations)
        {
            return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
        }
        let request = RevisionTransitionRequest {
            target_revision: endpoint.revision(),
            mutations,
            registry,
        };
        self.prepare_revision_inner(&request, RelationEndpointValidation::ExactDerived)
    }

    fn prepare_revision_inner(
        &self,
        request: &RevisionTransitionRequest<'_>,
        endpoint_validation: RelationEndpointValidation,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let replay_claimed_delta = matches!(
            endpoint_validation,
            RelationEndpointValidation::ReplayClaimedDelta
        );
        let source_revision = self.revision.id();
        let target_revision = request.target_revision.id();
        if target_revision == source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition);
        }
        if request.target_revision.semantic_context() != self.revision.semantic_context() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        let candidate_version = self
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;

        let mut seen_relations = BTreeSet::new();
        let mut relation_deltas = BTreeMap::new();
        for mutation in request.mutations {
            if !seen_relations.insert(mutation.relation) {
                return Err(PhysicalExecutionError::DuplicateRelationMutation(
                    mutation.relation,
                ));
            }
            if !self.relation_layouts.contains_key(&mutation.relation) {
                return Err(PhysicalExecutionError::MissingRuntimeRelationBinding(
                    mutation.relation,
                ));
            }
            relation_deltas.insert(mutation.relation, mutation.delta.clone());
        }

        if self.physical.revision() != Some(source_revision) {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }

        if replay_claimed_delta {
            self.validate_target_logical_state(request)?;
        }

        let (mut candidate_store, resolved) =
            self.candidate_physical_store_for_revision(request)?;
        candidate_store.rebind_unpublished_candidate_revision(source_revision, target_revision)?;

        let mut candidate_materializations = self.materializations.clone();
        let mut output_deltas = BTreeMap::new();
        let affected_materializations = self.affected_materializations(resolved.keys().copied());
        for &id in &affected_materializations {
            let dependencies = self
                .materialization_dependencies
                .get(&id)
                .ok_or(PhysicalExecutionError::MaterializationDependencyIndexMismatch)?;
            let relevant = resolved
                .iter()
                .filter(|(relation, _)| dependencies.contains(relation))
                .map(|(relation, delta)| (*relation, delta.clone()))
                .collect::<BTreeMap<_, _>>();
            let candidate_plan = candidate_materializations
                .get_mut(&id)
                .ok_or(PhysicalExecutionError::MaterializationDependencyIndexMismatch)?;
            let output_delta = candidate_plan.apply_storage_resolved_deltas(
                &relevant,
                self.revision.semantic_context(),
                request.registry,
            )?;
            output_deltas.insert(id, output_delta);
        }

        let violation_state = self.candidate_violation_state_for_relation_transition(
            request,
            resolved.keys().copied(),
            replay_claimed_delta,
        )?;
        violation_state.require_zero()?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: self.root_identity.root_id,
                version: RuntimeRootVersion(candidate_version),
            },
            revision: request.target_revision.clone(),
            violation_state,
            physical: candidate_store,
            relation_layouts: self.relation_layouts.clone(),
            materialization_specs: self.materialization_specs.clone(),
            materializations: candidate_materializations,
            materialization_dependencies: self.materialization_dependencies.clone(),
            materializations_by_relation: self.materializations_by_relation.clone(),
        };
        let descriptor = RevisionCommitDescriptor {
            source_revision,
            target: Box::new(request.target_revision.clone()),
            change: RevisionCommitChange::RelationData {
                semantic_revision: self.revision.semantic_revision(),
                relation_deltas,
            },
            rewrite_intents: BTreeMap::new(),
        };
        Ok(PreparedRuntimeRevisionTransition {
            descriptor,
            source_identity: self.root_identity,
            candidate: Box::new(candidate),
            output_deltas,
        })
    }

    fn prepare_rewrites<I>(
        &self,
        request: &RevisionRewriteTransitionRequest<'_, I>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.prepare_rewrites_inner(request, None)
    }

    fn prepare_rewrites_derived<I>(
        &self,
        endpoint: &DerivedRelationEndpoint,
        rewrites: &[RevisionRelationRewrite<'_, I>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let request = RevisionRewriteTransitionRequest {
            target_revision: endpoint.revision(),
            rewrites,
            registry,
        };
        self.prepare_rewrites_inner(&request, Some(endpoint))
    }

    fn prepare_rewrites_inner<I>(
        &self,
        request: &RevisionRewriteTransitionRequest<'_, I>,
        derived_endpoint: Option<&DerivedRelationEndpoint>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let mut seen = BTreeSet::new();
        let mut mutations = Vec::with_capacity(request.rewrites.len());
        let mut rewrite_intents = BTreeMap::new();

        for relation_rewrite in request.rewrites {
            if !seen.insert(relation_rewrite.relation) {
                return Err(PhysicalExecutionError::DuplicateRelationMutation(
                    relation_rewrite.relation,
                ));
            }
            mutations.push(RevisionRelationMutation {
                relation: relation_rewrite.relation,
                delta: &relation_rewrite.rewrite.delta,
            });
            rewrite_intents.insert(
                relation_rewrite.relation,
                RuntimeRewriteIntent {
                    spec: relation_rewrite.rewrite.rewrite.spec,
                    law_set: relation_rewrite.rewrite.rewrite.law_set,
                },
            );
        }

        if derived_endpoint.is_some_and(|endpoint| !endpoint.certifies_mutations(&mutations)) {
            return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
        }

        for relation_rewrite in request.rewrites {
            let rewrite_endpoint = relation_rewrite.rewrite.rewrite.effect.endpoint();
            let effect_matches = if let Some(endpoint) = derived_endpoint {
                relation_value_matches_revision_relation(
                    rewrite_endpoint,
                    endpoint.revision(),
                    relation_rewrite.relation,
                )
            } else {
                let old = RelExpr::Scan(relation_rewrite.relation).evaluate(
                    &self.revision.state().model,
                    self.revision.semantic_context(),
                    request.registry,
                )?;
                let delta_endpoint = relation_rewrite.rewrite.delta.apply_to_value(
                    old,
                    self.revision.semantic_context(),
                    request.registry,
                )?;
                rewrite_endpoint == &delta_endpoint
            };
            if !effect_matches {
                return Err(PhysicalExecutionError::RewriteEffectMismatch(
                    relation_rewrite.relation,
                ));
            }
        }

        let mut prepared = match derived_endpoint {
            Some(endpoint) => {
                self.prepare_revision_derived(endpoint, &mutations, request.registry)?
            }
            None => self.prepare_revision(&RevisionTransitionRequest {
                target_revision: request.target_revision,
                mutations: &mutations,
                registry: request.registry,
            })?,
        };
        prepared.descriptor.rewrite_intents = rewrite_intents;
        Ok(prepared)
    }

    fn prepare_full_revision(
        &self,
        request: &FullRevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let source_revision = self.revision.id();
        let target_revision = request.target_revision.id();
        if target_revision == source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition);
        }
        if self.physical.revision() != Some(source_revision) {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }

        let candidate_version = self
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = PhysicalStore::default();
        let mut relation_layouts = BTreeMap::new();
        for relation in request
            .target_revision
            .semantic_context()
            .schema
            .relations()
        {
            let rows = request
                .target_revision
                .state()
                .model
                .relations
                .get(&relation.id)
                .cloned()
                .unwrap_or_default();
            physical.install(
                relation.id,
                LayoutBinding::RECOVERY_ROW_STORE,
                NativeRelation::row_store(rows),
            )?;
            relation_layouts.insert(relation.id, LayoutBinding::RECOVERY_ROW_STORE);
        }
        let specs = self
            .materialization_specs
            .iter()
            .map(|(&id, query)| RuntimeMaterializationSpec {
                id,
                query: query.clone(),
            })
            .collect::<Vec<_>>();
        let candidate = Self::build_with_identity(
            request.target_revision.clone(),
            physical,
            relation_layouts,
            &specs,
            request.registry,
            RuntimeRootIdentity {
                root_id: self.root_identity.root_id,
                version: RuntimeRootVersion(candidate_version),
            },
        )?;

        Ok(PreparedRuntimeRevisionTransition {
            descriptor: RevisionCommitDescriptor {
                source_revision,
                target: Box::new(request.target_revision.clone()),
                change: RevisionCommitChange::FullRevision,
                rewrite_intents: BTreeMap::new(),
            },
            source_identity: self.root_identity,
            candidate: Box::new(candidate),
            output_deltas: BTreeMap::new(),
        })
    }

    fn prepare_revision_and_materializations(
        &self,
        request: &RevisionAndMaterializationsTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        let source_revision = self.revision.id();
        let target_revision = request.target_revision.id();
        if target_revision == source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition);
        }
        if self.physical.revision() != Some(source_revision) {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }

        let candidate_version = self
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = PhysicalStore::default();
        let mut relation_layouts = BTreeMap::new();
        for relation in request
            .target_revision
            .semantic_context()
            .schema
            .relations()
        {
            let rows = request
                .target_revision
                .state()
                .model
                .relations
                .get(&relation.id)
                .cloned()
                .unwrap_or_default();
            physical.install(
                relation.id,
                LayoutBinding::RECOVERY_ROW_STORE,
                NativeRelation::row_store(rows),
            )?;
            relation_layouts.insert(relation.id, LayoutBinding::RECOVERY_ROW_STORE);
        }
        let candidate = Self::build_with_identity(
            request.target_revision.clone(),
            physical,
            relation_layouts,
            request.materializations,
            request.registry,
            RuntimeRootIdentity {
                root_id: self.root_identity.root_id,
                version: RuntimeRootVersion(candidate_version),
            },
        )?;
        let materializations = candidate.durable_materialization_specs();

        Ok(PreparedRuntimeRevisionTransition {
            descriptor: RevisionCommitDescriptor {
                source_revision,
                target: Box::new(request.target_revision.clone()),
                change: RevisionCommitChange::FullRevisionAndMaterializations { materializations },
                rewrite_intents: BTreeMap::new(),
            },
            source_identity: self.root_identity,
            candidate: Box::new(candidate),
            output_deltas: BTreeMap::new(),
        })
    }

    fn prepare_materialization_configuration(
        &self,
        specs: &[RuntimeMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedMaterializationConfiguration, PhysicalExecutionError> {
        let next_version = self
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        if self.physical.revision() != Some(self.revision.id()) {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }

        let mut materializations = PersistentOrdMap::default();
        let mut materialization_specs = PersistentOrdMap::default();
        for spec in specs {
            if materializations.contains_key(&spec.id) {
                return Err(PhysicalExecutionError::DuplicateMaterialization(spec.id));
            }
            let mut maintained = MaterializedRelPlanState::build(
                &spec.query,
                &self.revision.state().model,
                self.revision.semantic_context(),
                registry,
            )?;
            for relation in maintained.scan_relations() {
                let layout = self.relation_layouts.get(&relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
                )?;
                let rows = self.physical.logical_rows_with_handles(relation, layout)?;
                maintained.attach_storage_rows(relation, &rows)?;
            }
            materialization_specs.insert(spec.id, spec.query.clone());
            materializations.insert(spec.id, maintained);
        }

        let (materialization_dependencies, materializations_by_relation) =
            Self::materialization_dependency_indexes(&materialization_specs);
        Ok(PreparedMaterializationConfiguration {
            source_identity: self.root_identity,
            candidate: Box::new(RuntimeRevisionBundle {
                root_identity: RuntimeRootIdentity {
                    root_id: self.root_identity.root_id,
                    version: RuntimeRootVersion(next_version),
                },
                revision: self.revision.clone(),
                violation_state: self.violation_state.clone(),
                physical: self.physical.clone(),
                relation_layouts: self.relation_layouts.clone(),
                materialization_specs,
                materializations,
                materialization_dependencies,
                materializations_by_relation,
            }),
        })
    }

    fn validate_target_logical_state(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<(), PhysicalExecutionError> {
        let source = self.revision.state();
        let target = request.target_revision.state();

        let mutated_relations: BTreeSet<_> = request
            .mutations
            .iter()
            .map(|mutation| mutation.relation)
            .collect();

        if !request
            .target_revision
            .certifies_relation_only_from(&self.revision, &mutated_relations)
        {
            if source.lifecycle != target.lifecycle
                || source.model.carriers != target.model.carriers
                || source.model.fields != target.model.fields
            {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
            }

            let source_untouched = source
                .model
                .relations
                .iter()
                .filter(|(relation, _)| !mutated_relations.contains(relation));
            let target_untouched = target
                .model
                .relations
                .iter()
                .filter(|(relation, _)| !mutated_relations.contains(relation));
            if !source_untouched.eq(target_untouched) {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
            }
        }

        let mut affected = BTreeMap::<SemanticId, RelationValue>::new();
        for mutation in request.mutations {
            let old = match affected.remove(&mutation.relation) {
                Some(value) => value,
                None => RelExpr::Scan(mutation.relation).evaluate(
                    &source.model,
                    self.revision.semantic_context(),
                    request.registry,
                )?,
            };
            let next = mutation.delta.apply_to_value(
                old,
                self.revision.semantic_context(),
                request.registry,
            )?;
            affected.insert(mutation.relation, next);
        }

        for relation in mutated_relations {
            let Some(actual) = affected.remove(&relation) else {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
            };
            let Some(expected_rows) = target.model.relations.get(&relation) else {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
            };
            if actual.into_rows() != *expected_rows {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch);
            }
        }

        Ok(())
    }
}
