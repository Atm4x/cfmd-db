impl RuntimeRevisionBundle {
    #[must_use]
    pub const fn revision(&self) -> &kernel_revision::Revision {
        &self.revision
    }

    #[must_use]
    pub const fn violation_state(&self) -> &RuntimeViolationState {
        &self.violation_state
    }

    #[must_use]
    pub const fn revision_id(&self) -> RevisionId {
        self.revision.id()
    }

    #[must_use]
    pub const fn root_version(&self) -> RuntimeRootVersion {
        self.root_identity.version
    }

    #[must_use]
    pub const fn physical_store(&self) -> &PhysicalStore {
        &self.physical
    }

    /// Captures one exact OFC observation fiber against this immutable runtime
    /// root. The returned guard is root/revision bound and can later classify
    /// prepared transitions without trusting caller-provided dependency sets.
    pub fn observe_query(
        &self,
        query: &RelExpr,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RuntimeObservationGuard, PhysicalExecutionError> {
        Ok(RuntimeObservationGuard {
            source_identity: self.root_identity,
            source_revision: self.revision.id(),
            guard: RelObservationGuard::observe(
                query,
                &self.revision.state().model,
                self.revision.semantic_context(),
                registry,
            )?,
        })
    }

    fn prepare_repair_candidate(
        &self,
        candidate: RuntimeRepairCandidate,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> (
        Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError>,
        Option<Box<RuntimeRepairObservationTransport>>,
    ) {
        match candidate {
            RuntimeRepairCandidate::RelationData {
                target_revision,
                mutations,
            } => {
                let mutation_refs = mutations
                    .iter()
                    .map(|mutation| RevisionRelationMutation {
                        relation: mutation.relation,
                        delta: &mutation.delta,
                    })
                    .collect::<Vec<_>>();
                (
                    self.prepare_revision(&RevisionTransitionRequest {
                        target_revision: &target_revision,
                        mutations: &mutation_refs,
                        registry,
                    }),
                    None,
                )
            }
            RuntimeRepairCandidate::FullRevision { target_revision } => (
                self.prepare_full_revision(&FullRevisionTransitionRequest {
                    target_revision: &target_revision,
                    registry,
                }),
                None,
            ),
            RuntimeRepairCandidate::TransportedFullRevision {
                target_revision,
                observation_transport,
            } => (
                self.prepare_full_revision(&FullRevisionTransitionRequest {
                    target_revision: &target_revision,
                    registry,
                }),
                Some(observation_transport),
            ),
        }
    }

    /// Searches a finite provider frontier for an exact observation-preserving
    /// repair. Provider generation is bounded policy; every candidate is
    /// checked by the ordinary transition path, VMF and OFC before selection.
    pub fn prepare_bounded_repair<P: RepairCandidateProvider>(
        &self,
        guard: &RuntimeObservationGuard,
        provider: &P,
        policy: RepairSearchPolicy,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RepairSearchOutcome, PhysicalExecutionError> {
        let candidates = provider.candidates(self, policy.max_candidates.saturating_add(1))?;
        if candidates.len() > policy.max_candidates {
            return Ok(RepairSearchOutcome::BudgetExceeded {
                supplied: candidates.len(),
                maximum: policy.max_candidates,
            });
        }

        let mut report = RepairSearchReport {
            supplied: candidates.len(),
            ..RepairSearchReport::default()
        };
        let mut accepted = Vec::new();
        for candidate in candidates {
            report.examined = report.examined.saturating_add(1);
            let (prepared, observation_transport) =
                self.prepare_repair_candidate(candidate, registry);
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(
                    PhysicalExecutionError::CandidateViolationStateNonZero
                    | PhysicalExecutionError::LogicalRevisionMutationMismatch
                    | PhysicalExecutionError::Validation(_),
                ) => {
                    report.rejected_invalid = report.rejected_invalid.saturating_add(1);
                    continue;
                }
                Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild) => {
                    report.rejected_transport = report.rejected_transport.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            };

            let impact = match observation_transport.as_ref() {
                Some(transport) => {
                    guard.impact_prepared_transported(self, &prepared, transport, registry)
                }
                None => guard.impact_prepared(self, &prepared, registry),
            };
            let impact = match impact {
                Ok(impact) => impact,
                Err(
                    PhysicalExecutionError::SemanticContextTransitionRequiresRebuild
                    | PhysicalExecutionError::RepairObservationTransportMismatch
                    | PhysicalExecutionError::RepairTransport(_),
                ) => {
                    report.rejected_transport = report.rejected_transport.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            match impact {
                Impact::Unaffected => {
                    report.accepted = report.accepted.saturating_add(1);
                    accepted.push(prepared);
                }
                Impact::Changed | Impact::Unknown => {
                    report.rejected_observation_change =
                        report.rejected_observation_change.saturating_add(1);
                }
            }
        }

        Ok(match accepted.len() {
            0 => RepairSearchOutcome::NoRepair(report),
            1 => RepairSearchOutcome::Prepared {
                transition: accepted.pop().expect("length checked"),
                report,
            },
            valid_candidates => RepairSearchOutcome::Ambiguous {
                valid_candidates,
                report,
            },
        })
    }

    #[must_use]
    pub fn relation_layout(&self, relation: SemanticId) -> Option<LayoutBinding> {
        self.relation_layouts.get(&relation).copied()
    }

    #[must_use]
    pub fn materialization(
        &self,
        id: kernel_types::MaterializationId,
    ) -> Option<&MaterializedRelPlanState> {
        self.materializations.get(&id)
    }

    /// Reader-visible revision for a runtime-owned maintained materialization.
    ///
    /// Runtime-owned maintained plans deliberately remain internally unbound:
    /// `RuntimeRevisionBundle` is the sole publication/revision authority. This
    /// avoids rewriting every materialization merely because an unrelated base
    /// relation advanced the global revision.
    #[must_use]
    pub fn materialization_revision(
        &self,
        id: kernel_types::MaterializationId,
    ) -> Option<RevisionId> {
        self.materializations
            .contains_key(&id)
            .then_some(self.revision.id())
    }

    #[must_use]
    pub const fn materializations(
        &self,
    ) -> &PersistentOrdMap<kernel_types::MaterializationId, MaterializedRelPlanState> {
        &self.materializations
    }

    #[must_use]
    pub fn durable_materialization_specs(&self) -> Vec<DurableMaterializationSpec> {
        self.materialization_specs
            .iter()
            .map(|(&id, query)| DurableMaterializationSpec {
                id,
                query: query.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn durable_physical_artifact_specs(&self) -> Vec<DurablePhysicalArtifactSpec> {
        let mut specs = self.physical.durable_physical_artifact_specs();
        for (&relation, &layout) in &self.relation_layouts {
            let Ok(installed) = self.physical.installed(relation, layout) else {
                continue;
            };
            let kind = match installed.data {
                NativeRelation::RowStore(_) => DurableRelationLayoutKind::RowStore,
                NativeRelation::Columnar { .. } => DurableRelationLayoutKind::ValueColumnar,
                NativeRelation::I64Columnar { .. } => DurableRelationLayoutKind::I64Columnar,
                NativeRelation::TypedColumnar { .. } => DurableRelationLayoutKind::TypedColumnar,
            };
            specs.push(DurablePhysicalArtifactSpec::RelationLayout {
                relation,
                layout_id: layout.id.0,
                kind,
            });
        }
        specs.sort();
        specs.dedup();
        specs
    }

}
