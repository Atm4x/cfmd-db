impl PhysicalStore {
    #[must_use]
    pub fn durable_physical_artifact_specs(&self) -> Vec<DurablePhysicalArtifactSpec> {
        let mut specs = Vec::new();
        for binding in self.i64_indexes.keys() {
            specs.push(DurablePhysicalArtifactSpec::I64Index {
                relation: binding.relation,
                key_column: binding.key_column,
                equivalence: binding.equivalence,
                advisor_managed: self
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::I64Index(*binding)),
            });
        }
        for binding in self.semantic_indexes.keys() {
            specs.push(DurablePhysicalArtifactSpec::SemanticIndex {
                relation: binding.relation,
                key_parts: durable_semantic_key_parts(binding),
                advisor_managed: self
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticIndex(binding.clone())),
            });
        }
        for binding in self.semantic_quotient_factors.keys() {
            specs.push(DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation: binding.relation,
                key_parts: durable_semantic_key_parts(binding),
                advisor_managed: self
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone())),
            });
        }
        for binding in self.semantic_statistics.keys() {
            specs.push(DurablePhysicalArtifactSpec::SemanticStatistics {
                relation: binding.relation,
                key_parts: durable_semantic_key_parts(binding),
                advisor_managed: self
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticStatistics(binding.clone())),
            });
        }
        for binding in self.observable_atom_states.keys() {
            specs.push(DurablePhysicalArtifactSpec::ObservableAtom {
                relation: binding.relation,
                key_parts: durable_semantic_key_parts(binding),
                advisor_managed: self
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::ObservableAtom(binding.clone())),
            });
        }
        specs.sort();
        specs.dedup();
        specs
    }

    pub(super) fn durable_artifact_cores(
        &self,
        source_revision: RevisionId,
    ) -> Result<Vec<DurableArtifactCore>, PhysicalExecutionError> {
        let mut cores = Vec::new();
        for (binding, state) in &self.observable_atom_states {
            let relation = self.installed(binding.relation, binding.layout)?;
            cores.push(state.durable_core(source_revision, relation)?);
        }
        cores.sort();
        cores.dedup();
        Ok(cores)
    }

    pub(super) fn restore_durable_physical_artifacts(
        &mut self,
        inputs: &PhysicalRecoveryInputs<'_>,
    ) -> PhysicalRecoveryReport {
        let mut report = PhysicalRecoveryReport::default();
        let scheduled_specs = self.schedule_durable_physical_artifact_rebuilds(
            inputs.specs,
            inputs.relation_layouts,
            inputs.telemetry,
            &mut report,
        );
        for (spec, work) in scheduled_specs {
            if self.try_admit_rehydrated_artifact(&spec, inputs, &mut report) {
                continue;
            }
            self.try_rebuild_scheduled_artifact(spec, work, inputs, &mut report);
        }
        self.state_identity = Arc::new(());
        report.total_estimated_bytes_after =
            self.artifact_memory_report().total_estimated_retained_bytes;
        report
    }

    fn try_admit_rehydrated_artifact(
        &mut self,
        spec: &DurablePhysicalArtifactSpec,
        inputs: &PhysicalRecoveryInputs<'_>,
        report: &mut PhysicalRecoveryReport,
    ) -> bool {
        let Some(core) =
            matching_durable_artifact_core(spec, inputs.artifact_cores, inputs.target_revision)
        else {
            return false;
        };
        let advisor_managed = durable_physical_artifact_is_advisor_managed(spec);
        let current_estimated_bytes = self.artifact_memory_report().total_estimated_retained_bytes;
        let mut candidate = self.clone();
        if candidate
            .try_rehydrate_durable_physical_artifact(
                spec,
                core,
                inputs.relation_layouts,
                inputs.context,
                inputs.registry,
            )
            .is_err()
        {
            return false;
        }
        let candidate_estimated_bytes = candidate
            .artifact_memory_report()
            .total_estimated_retained_bytes;
        let incremental_estimated_bytes =
            candidate_estimated_bytes.saturating_sub(current_estimated_bytes);
        let next_advisor_estimated_bytes =
            report
                .advisor_estimated_bytes
                .saturating_add(if advisor_managed {
                    incremental_estimated_bytes
                } else {
                    0
                });
        if advisor_managed
            && (candidate_estimated_bytes > inputs.policy.max_total_estimated_bytes
                || next_advisor_estimated_bytes > inputs.policy.max_advisor_estimated_bytes)
        {
            report.skipped_estimated_byte_budget.push(spec.clone());
            return true;
        }
        *self = candidate;
        report.advisor_estimated_bytes = next_advisor_estimated_bytes;
        report.rehydrated.push(spec.clone());
        true
    }

    fn try_rebuild_scheduled_artifact(
        &mut self,
        spec: DurablePhysicalArtifactSpec,
        work: PhysicalRebuildWorkEstimate,
        inputs: &PhysicalRecoveryInputs<'_>,
        report: &mut PhysicalRecoveryReport,
    ) {
        let advisor_managed = durable_physical_artifact_is_advisor_managed(&spec);
        let next_total_key_evaluations = report
            .attempted_rebuild_key_evaluations
            .saturating_add(work.key_evaluations);
        let next_advisor_key_evaluations =
            report
                .advisor_rebuild_key_evaluations
                .saturating_add(if advisor_managed {
                    work.key_evaluations
                } else {
                    0
                });
        if advisor_managed
            && next_advisor_key_evaluations > inputs.policy.max_advisor_rebuild_key_evaluations
        {
            report.skipped_key_evaluation_budget.push(spec);
            return;
        }
        let next_total_semantic_work = report
            .attempted_rebuild_semantic_work_units
            .saturating_add(work.semantic_work_units);
        let next_advisor_semantic_work =
            report
                .advisor_rebuild_semantic_work_units
                .saturating_add(if advisor_managed {
                    work.semantic_work_units
                } else {
                    0
                });
        if advisor_managed
            && next_advisor_semantic_work > inputs.policy.max_advisor_rebuild_semantic_work_units
        {
            report.skipped_semantic_work_budget.push(spec);
            return;
        }
        report.attempted_rebuild_key_evaluations = next_total_key_evaluations;
        report.advisor_rebuild_key_evaluations = next_advisor_key_evaluations;
        report.attempted_rebuild_semantic_work_units = next_total_semantic_work;
        report.advisor_rebuild_semantic_work_units = next_advisor_semantic_work;
        let current_estimated_bytes = self.artifact_memory_report().total_estimated_retained_bytes;
        let mut candidate = self.clone();
        if candidate
            .try_restore_durable_physical_artifact(
                &spec,
                inputs.relation_layouts,
                inputs.context,
                inputs.registry,
            )
            .is_err()
        {
            report.dropped_incompatible.push(spec);
            return;
        }
        let candidate_estimated_bytes = candidate
            .artifact_memory_report()
            .total_estimated_retained_bytes;
        let incremental_estimated_bytes =
            candidate_estimated_bytes.saturating_sub(current_estimated_bytes);
        let next_advisor_estimated_bytes =
            report
                .advisor_estimated_bytes
                .saturating_add(if advisor_managed {
                    incremental_estimated_bytes
                } else {
                    0
                });
        if advisor_managed
            && (candidate_estimated_bytes > inputs.policy.max_total_estimated_bytes
                || next_advisor_estimated_bytes > inputs.policy.max_advisor_estimated_bytes)
        {
            report.skipped_estimated_byte_budget.push(spec);
            return;
        }
        *self = candidate;
        report.advisor_estimated_bytes = next_advisor_estimated_bytes;
        report.rebuilt.push(spec);
    }

    fn try_rehydrate_durable_physical_artifact(
        &mut self,
        spec: &DurablePhysicalArtifactSpec,
        core: &DurableArtifactCore,
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let (relation, key_parts, advisor_managed, encoded_keys_by_ordinal) = match (spec, core) {
            (
                DurablePhysicalArtifactSpec::ObservableAtom {
                    relation,
                    key_parts,
                    advisor_managed,
                },
                DurableArtifactCore::ObservableAtom {
                    relation: core_relation,
                    key_parts: core_key_parts,
                    encoded_keys_by_ordinal,
                    ..
                },
            ) if relation == core_relation && key_parts == core_key_parts => (
                *relation,
                key_parts,
                *advisor_managed,
                encoded_keys_by_ordinal,
            ),
            _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
        };
        let layout = relation_layouts.get(&relation).copied().ok_or(
            PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
        )?;
        let binding = recovered_semantic_index_binding(relation, layout, key_parts)?;
        let relation_state = self.installed(binding.relation, binding.layout)?;
        let state = MaterializedObservableAtomState::build_from_durable_core(
            binding.clone(),
            relation_state,
            encoded_keys_by_ordinal,
            context,
            registry,
        )?;
        self.observable_atom_states_mut_internal()
            .insert(binding.clone(), Arc::new(state));
        if advisor_managed {
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::ObservableAtom(binding));
        }
        Ok(())
    }

    fn durable_artifact_capabilities(
        spec: &DurablePhysicalArtifactSpec,
    ) -> BTreeSet<PhysicalCapability> {
        match spec {
            DurablePhysicalArtifactSpec::RelationLayout { .. } => BTreeSet::new(),
            DurablePhysicalArtifactSpec::I64Index { .. }
            | DurablePhysicalArtifactSpec::SemanticIndex { .. } => {
                BTreeSet::from([PhysicalCapability::PointLookup])
            }
            DurablePhysicalArtifactSpec::SemanticQuotientFactor { .. } => {
                BTreeSet::from([PhysicalCapability::QuotientFiber])
            }
            DurablePhysicalArtifactSpec::SemanticStatistics { .. } => {
                BTreeSet::from([PhysicalCapability::ExactCardinality])
            }
            DurablePhysicalArtifactSpec::ObservableAtom { .. } => {
                BTreeSet::from([PhysicalCapability::ObservableFiber])
            }
        }
    }

    fn durable_spec_unified_artifact_id(
        spec: &DurablePhysicalArtifactSpec,
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
    ) -> Result<Option<UnifiedArtifactId>, PhysicalExecutionError> {
        let recovered_layout = |relation: SemanticId| {
            relation_layouts.get(&relation).copied().ok_or(
                PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
            )
        };
        Ok(match spec {
            DurablePhysicalArtifactSpec::RelationLayout { .. } => None,
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column,
                equivalence,
                ..
            } => Some(UnifiedArtifactId::I64Index(I64IndexBinding {
                relation: *relation,
                layout: recovered_layout(*relation)?,
                key_column: *key_column,
                equivalence: *equivalence,
            })),
            DurablePhysicalArtifactSpec::SemanticIndex {
                relation,
                key_parts,
                ..
            } => Some(UnifiedArtifactId::SemanticIndex(
                recovered_semantic_index_binding(
                    *relation,
                    recovered_layout(*relation)?,
                    key_parts,
                )?,
            )),
            DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation,
                key_parts,
                ..
            } => Some(UnifiedArtifactId::SemanticQuotientFactor(
                recovered_semantic_index_binding(
                    *relation,
                    recovered_layout(*relation)?,
                    key_parts,
                )?,
            )),
            DurablePhysicalArtifactSpec::SemanticStatistics {
                relation,
                key_parts,
                ..
            } => Some(UnifiedArtifactId::SemanticStatistics(
                recovered_semantic_index_binding(
                    *relation,
                    recovered_layout(*relation)?,
                    key_parts,
                )?,
            )),
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation,
                key_parts,
                ..
            } => Some(UnifiedArtifactId::ObservableAtom(
                recovered_semantic_index_binding(
                    *relation,
                    recovered_layout(*relation)?,
                    key_parts,
                )?,
            )),
        })
    }

    fn schedule_durable_physical_artifact_rebuilds(
        &self,
        specs: &[DurablePhysicalArtifactSpec],
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
        telemetry: Option<&UnifiedAdvisorTelemetry>,
        report: &mut PhysicalRecoveryReport,
    ) -> Vec<(DurablePhysicalArtifactSpec, PhysicalRebuildWorkEstimate)> {
        let mut semantic_column_work_cache = BTreeMap::<(SemanticId, usize), usize>::new();
        let mut optional_specs = specs
            .iter()
            .filter(|spec| !matches!(spec, DurablePhysicalArtifactSpec::RelationLayout { .. }))
            .cloned()
            .collect::<Vec<_>>();
        optional_specs.sort();
        let mut candidates = Vec::with_capacity(optional_specs.len());
        for spec in optional_specs {
            let Ok(work) = self.durable_physical_artifact_rebuild_work(
                &spec,
                relation_layouts,
                &mut semantic_column_work_cache,
            ) else {
                report.dropped_incompatible.push(spec);
                continue;
            };
            let observed = telemetry
                .and_then(|telemetry| {
                    Self::durable_spec_unified_artifact_id(&spec, relation_layouts)
                        .ok()
                        .flatten()
                        .map(|id| telemetry.get(&id))
                })
                .unwrap_or_default();
            candidates.push(advisor::RecoveryCandidate {
                key: spec.clone(),
                capabilities: Self::durable_artifact_capabilities(&spec),
                work: PhysicalWorkEstimate {
                    read_work_saved: observed.read_work_saved,
                    maintenance_work: observed.maintenance_work,
                    build_work: work.semantic_work_units as u128,
                },
                footprint: ResourceFootprint::from_atom(spec.clone(), 0),
                manual_pin: !durable_physical_artifact_is_advisor_managed(&spec),
                secondary_build_work: work.key_evaluations,
                detail: work,
            });
        }
        advisor::order_recovery_candidates(candidates)
            .into_iter()
            .map(|candidate| (candidate.key, candidate.detail))
            .collect()
    }

    fn durable_physical_artifact_rebuild_work(
        &self,
        spec: &DurablePhysicalArtifactSpec,
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
        semantic_column_work_cache: &mut BTreeMap<(SemanticId, usize), usize>,
    ) -> Result<PhysicalRebuildWorkEstimate, PhysicalExecutionError> {
        let (relation, key_parts) = match spec {
            DurablePhysicalArtifactSpec::RelationLayout { .. } => {
                return Ok(PhysicalRebuildWorkEstimate::default());
            }
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column,
                ..
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let installed = self.installed(*relation, layout)?;
                let row_count = native_row_count(&installed.data);
                let semantic_work_units = *semantic_column_work_cache
                    .entry((*relation, *key_column))
                    .or_insert(native_semantic_column_work_units(
                        &installed.data,
                        *key_column,
                    )?);
                return Ok(PhysicalRebuildWorkEstimate {
                    key_evaluations: row_count,
                    semantic_work_units,
                });
            }
            DurablePhysicalArtifactSpec::SemanticIndex {
                relation,
                key_parts,
                ..
            }
            | DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation,
                key_parts,
                ..
            }
            | DurablePhysicalArtifactSpec::SemanticStatistics {
                relation,
                key_parts,
                ..
            }
            | DurablePhysicalArtifactSpec::ObservableAtom {
                relation,
                key_parts,
                ..
            } => (*relation, key_parts),
        };
        let layout = relation_layouts.get(&relation).copied().ok_or(
            PhysicalExecutionError::MissingRuntimeRelationBinding(relation),
        )?;
        let installed = self.installed(relation, layout)?;
        let row_count = native_row_count(&installed.data);
        let semantic_work_units = key_parts.iter().try_fold(0_usize, |work, part| {
            let column_work =
                if let Some(work) = semantic_column_work_cache.get(&(relation, part.column)) {
                    *work
                } else {
                    let work = native_semantic_column_work_units(&installed.data, part.column)?;
                    semantic_column_work_cache.insert((relation, part.column), work);
                    work
                };
            Ok::<_, PhysicalExecutionError>(work.saturating_add(column_work))
        })?;
        Ok(PhysicalRebuildWorkEstimate {
            key_evaluations: row_count.saturating_mul(key_parts.len().max(1)),
            semantic_work_units,
        })
    }

    fn try_restore_durable_physical_artifact(
        &mut self,
        spec: &DurablePhysicalArtifactSpec,
        relation_layouts: &BTreeMap<SemanticId, LayoutBinding>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        match spec {
            DurablePhysicalArtifactSpec::RelationLayout { .. } => {}
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column,
                equivalence,
                advisor_managed,
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let binding = I64IndexBinding {
                    relation: *relation,
                    layout,
                    key_column: *key_column,
                    equivalence: *equivalence,
                };
                self.install_i64_index(binding, context, registry)?;
                if *advisor_managed {
                    self.advisor_managed_artifacts_mut()
                        .insert(UnifiedArtifactId::I64Index(binding));
                }
            }
            DurablePhysicalArtifactSpec::SemanticIndex {
                relation,
                key_parts,
                advisor_managed,
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let binding = recovered_semantic_index_binding(*relation, layout, key_parts)?;
                // Legacy durable semantic-index recipes are accepted for backward
                // compatibility, but recovery converges them immediately onto the
                // current SAMF/observable-atom representation.  The logical
                // capability is the same Γ-keyed lookup; recreating the legacy
                // runtime state would only keep an obsolete execution family alive
                // after every restart.
                self.install_observable_atom_state(binding.clone(), context, registry)?;
                if *advisor_managed {
                    self.advisor_managed_artifacts_mut()
                        .insert(UnifiedArtifactId::ObservableAtom(binding));
                }
            }
            DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation,
                key_parts,
                advisor_managed,
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let binding = recovered_semantic_index_binding(*relation, layout, key_parts)?;
                let relation_state = self.installed(binding.relation, binding.layout)?;
                let state = MaterializedSemanticQuotientFactorState::build(
                    binding.clone(),
                    relation_state,
                    context,
                    registry,
                )?;
                self.semantic_quotient_factors_mut()
                    .insert(binding.clone(), Arc::new(state));
                if *advisor_managed {
                    self.advisor_managed_artifacts_mut()
                        .insert(UnifiedArtifactId::SemanticQuotientFactor(binding));
                }
            }
            DurablePhysicalArtifactSpec::SemanticStatistics {
                relation,
                key_parts,
                advisor_managed,
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let binding = recovered_semantic_index_binding(*relation, layout, key_parts)?;
                self.install_semantic_statistics(binding.clone(), context, registry)?;
                if *advisor_managed {
                    self.advisor_managed_artifacts_mut()
                        .insert(UnifiedArtifactId::SemanticStatistics(binding));
                }
            }
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation,
                key_parts,
                advisor_managed,
            } => {
                let layout = relation_layouts.get(relation).copied().ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                )?;
                let binding = recovered_semantic_index_binding(*relation, layout, key_parts)?;
                self.install_observable_atom_state(binding.clone(), context, registry)?;
                if *advisor_managed {
                    self.advisor_managed_artifacts_mut()
                        .insert(UnifiedArtifactId::ObservableAtom(binding));
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn artifact_memory_report(&self) -> PhysicalArtifactMemoryReport {
        let mut report = PhysicalArtifactMemoryReport::default();
        let mut add = |family: PhysicalArtifactFamily, bytes: usize, advisor_managed: bool| {
            let entry = report.families.entry(family).or_default();
            entry.artifacts = entry.artifacts.saturating_add(1);
            entry.advisor_managed_artifacts = entry
                .advisor_managed_artifacts
                .saturating_add(usize::from(advisor_managed));
            entry.estimated_retained_bytes = entry.estimated_retained_bytes.saturating_add(bytes);
            report.total_estimated_retained_bytes =
                report.total_estimated_retained_bytes.saturating_add(bytes);
        };
        let mut dense_identity_maps = BTreeSet::new();
        for state in self.relations.values() {
            add(
                PhysicalArtifactFamily::RelationLayout,
                installed_relation_estimated_retained_bytes(state),
                false,
            );
            if let NativeRelation::TypedColumnar { columns, .. } = &state.data {
                for column in columns {
                    if let NativeColumn::DenseLiveEntityIds { ids, .. } = column {
                        let identity = Arc::as_ptr(ids) as usize;
                        if dense_identity_maps.insert(identity) {
                            add(
                                PhysicalArtifactFamily::SharedDenseIdentityMap,
                                ids.estimated_retained_bytes(),
                                false,
                            );
                        }
                    }
                }
            }
        }
        for (binding, state) in &self.i64_indexes {
            add(
                PhysicalArtifactFamily::I64Index,
                i64_index_estimated_retained_bytes(state),
                self.advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::I64Index(*binding)),
            );
        }
        for (binding, state) in &self.semantic_indexes {
            add(
                PhysicalArtifactFamily::SemanticIndex,
                semantic_index_estimated_retained_bytes(state),
                self.advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticIndex(binding.clone())),
            );
        }
        for state in self.observable_atom_states.values() {
            add(
                PhysicalArtifactFamily::ObservableAtom,
                observable_atom_estimated_retained_bytes(state),
                false,
            );
        }
        for state in self.row_occurrence_atoms.values() {
            add(
                PhysicalArtifactFamily::ObservableAtom,
                observable_atom_estimated_retained_bytes(state),
                false,
            );
        }
        for (binding, state) in &self.semantic_quotient_factors {
            add(
                PhysicalArtifactFamily::SemanticQuotientFactor,
                quotient_factor_estimated_retained_bytes(state),
                self.advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone())),
            );
        }
        for (binding, state) in &self.semantic_quotient_supports {
            add(
                PhysicalArtifactFamily::SemanticQuotientSupport,
                quotient_support_estimated_retained_bytes(state),
                self.advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticQuotientSupport(binding.clone())),
            );
        }
        for (binding, state) in &self.semantic_statistics {
            add(
                PhysicalArtifactFamily::SemanticStatistics,
                semantic_statistics_estimated_retained_bytes(state),
                self.advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticStatistics(binding.clone())),
            );
        }
        report
    }

}
