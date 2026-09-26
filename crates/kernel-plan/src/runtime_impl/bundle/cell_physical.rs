impl RuntimeRevisionCell {
    /// Reconstructible index publication is also a whole-root swap. Existing
    /// readers keep their prior immutable snapshot while new readers observe
    /// the incremented root version under the same semantic revision.
    pub fn install_i64_index(
        &self,
        index: I64IndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = live.physical.clone();
        physical.install_i64_index(index, live.revision.semantic_context(), registry)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(())
    }

    pub fn install_semantic_statistics(
        &self,
        binding: SemanticIndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticKeyStatistics, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = live.physical.clone();
        let snapshot = physical.install_semantic_statistics(
            binding,
            live.revision.semantic_context(),
            registry,
        )?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(snapshot)
    }

    pub fn install_semantic_index(
        &self,
        index: SemanticIndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = live.physical.clone();
        physical.install_semantic_index(index, live.revision.semantic_context(), registry)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(())
    }

    pub fn install_observable_atom_state(
        &self,
        binding: SemanticIndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let mut physical = live.physical.clone();
        physical.install_observable_atom_state(
            binding,
            live.revision.semantic_context(),
            registry,
        )?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(())
    }

    pub fn advise_semantic_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: SemanticIndexAdvisorPolicy,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticIndexAdvisorReport, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let mut physical = live.physical.clone();
        let source_epoch = physical.transition_epoch();
        let report = physical.advise_semantic_indexes(
            workload,
            policy,
            live.revision.semantic_context(),
            registry,
        )?;
        if physical.transition_epoch() == source_epoch {
            return Ok(report);
        }
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(report)
    }

    pub(crate) fn advise_unified_observable_atoms(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        telemetry: &UnifiedAdvisorTelemetry,
        policy: UnifiedAdvisorPolicy,
        pressure_policy: PhysicalPressurePolicy,
        pressure_sample: PhysicalPressureSample,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let mut physical = live.physical.clone();
        let source_epoch = physical.transition_epoch();
        let report = physical.advise_unified_observable_atoms(
            workload,
            UnifiedObservableAdvisorInputs {
                telemetry,
                policy,
                pressure_policy,
                pressure_sample,
            },
            live.revision.semantic_context(),
            registry,
        )?;
        if physical.transition_epoch() == source_epoch {
            return Ok(report);
        }
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(report)
    }

    pub fn advise_i64_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<I64IndexAdvisorReport, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let mut physical = live.physical.clone();
        let source_epoch = physical.transition_epoch();
        let report = physical.advise_i64_indexes(
            workload,
            policy,
            live.revision.semantic_context(),
            registry,
        )?;
        if physical.transition_epoch() == source_epoch {
            return Ok(report);
        }
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(report)
    }

    pub fn advise_semantic_statistics(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticStatisticsAdvisorReport, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let mut physical = live.physical.clone();
        let source_epoch = physical.transition_epoch();
        let report = physical.advise_semantic_statistics(
            workload,
            policy,
            live.revision.semantic_context(),
            registry,
        )?;
        if physical.transition_epoch() == source_epoch {
            return Ok(report);
        }
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(report)
    }

    fn resume_deferred_physical_recovery(
        &self,
        specs: &[DurablePhysicalArtifactSpec],
        policy: PhysicalRecoveryPolicy,
        telemetry: Option<&UnifiedAdvisorTelemetry>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PhysicalRecoveryReport, PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let deferred = specs
            .iter()
            .filter(|spec| durable_physical_artifact_is_advisor_managed(spec))
            .cloned()
            .collect::<Vec<_>>();
        if deferred.is_empty() {
            return Ok(PhysicalRecoveryReport {
                total_estimated_bytes_after: live
                    .physical
                    .artifact_memory_report()
                    .total_estimated_retained_bytes,
                ..PhysicalRecoveryReport::default()
            });
        }

        let mut physical = live.physical.clone();
        let recovery_relation_layouts = live
            .relation_layouts
            .iter()
            .map(|(&relation, &layout)| (relation, layout))
            .collect::<BTreeMap<_, _>>();
        let report = physical.restore_durable_physical_artifacts(&PhysicalRecoveryInputs {
            specs: &deferred,
            artifact_cores: &[],
            target_revision: live.revision.id(),
            relation_layouts: &recovery_relation_layouts,
            policy,
            telemetry,
            context: live.revision.semantic_context(),
            registry,
        });
        if report.rebuilt.is_empty() {
            return Ok(report);
        }

        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical,
            relation_layouts: live.relation_layouts.clone(),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(report)
    }
}
