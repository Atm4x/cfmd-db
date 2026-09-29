impl DurableRuntime {
    pub fn rewrap_storage_encryption(
        &self,
        next: &kernel_durability::StorageEncryption,
    ) -> Result<u64, DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.rewrap_single_file_database_master_key(next)
    }

    pub fn retire_previous_storage_encryption_key(
        &self,
        acknowledged_key_epoch: u64,
    ) -> Result<(), DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.retire_previous_single_file_wrapped_key(acknowledged_key_epoch)
    }

    /// Monotone in-process wake generation for reader-visible Revision publication.
    ///
    /// The generation is deliberately not a state authority: durable history and
    /// the current immutable Revision remain authoritative. It exists only so a
    /// subscriber can block without polling and then recover the exact committed
    /// effect from durable history.
    #[must_use]
    pub fn revision_publication_generation(&self) -> u64 {
        self.revision_publication.generation()
    }

    /// Creates a per-subscription wake/cancellation capability.
    ///
    /// The handle owns the wake backend but not this runtime, so a blocked
    /// subscriber cannot keep the database runtime alive during shutdown.
    #[must_use]
    pub fn revision_publication_wait_handle(&self) -> RuntimeRevisionPublicationWaitHandle {
        RuntimeRevisionPublicationWaitHandle::new(
            Arc::clone(&self.revision_publication),
            Box::default(),
        )
    }

    #[must_use]
    pub fn revision_publication_wait_handle_for_relations(
        &self,
        dependencies: impl IntoIterator<Item = SemanticId>,
    ) -> RuntimeRevisionPublicationWaitHandle {
        let mut dependencies = dependencies.into_iter().collect::<Vec<_>>();
        dependencies.sort_unstable();
        dependencies.dedup();
        RuntimeRevisionPublicationWaitHandle::new(
            Arc::clone(&self.revision_publication),
            dependencies.into_boxed_slice(),
        )
    }

    pub(crate) fn signal_revision_publication(&self) {
        self.revision_publication.notify_revision_published();
    }

    pub(crate) fn signal_relation_publication(&self, relations: &[SemanticId]) {
        self.revision_publication.notify_relations_published(relations);
    }

    /// Reconstructs an exact committed logical revision reachable from the
    /// current durable head through reversible causal effects.
    ///
    /// No historical snapshot is persisted separately: the current immutable
    /// revision plus the Γ-REIC effect ideal and exact forward/reverse deltas
    /// are the sole authority. Non-reversible legacy/full/schema boundaries
    /// fail closed instead of synthesizing an approximate state.
    pub fn revision_at(
        &self,
        revision: RevisionId,
    ) -> Result<kernel_revision::Revision, RuntimeHistoricalSnapshotError> {
        let head = self.snapshot()?.revision().clone();
        if head.id() == revision {
            return Ok(head);
        }
        let effects = self
            .revision_history(head.id())?
            .ok_or(RuntimeHistoricalSnapshotError::Unavailable { revision })?;
        let mut adjacency = BTreeMap::<RevisionId, Vec<(RevisionId, usize, bool)>>::new();
        for (index, effect) in effects.iter().enumerate() {
            if effect.reversibility != RuntimeHistoryReversibility::ExactPlanInverse {
                continue;
            }
            adjacency.entry(effect.source_revision).or_default().push((
                effect.target_revision,
                index,
                true,
            ));
            adjacency.entry(effect.target_revision).or_default().push((
                effect.source_revision,
                index,
                false,
            ));
        }

        let mut queue = VecDeque::from([head.id()]);
        let mut predecessor = BTreeMap::<RevisionId, (RevisionId, usize, bool)>::new();
        let mut seen = BTreeSet::from([head.id()]);
        while let Some(current) = queue.pop_front() {
            if current == revision {
                break;
            }
            for &(next, effect, forward) in adjacency.get(&current).map_or(&[][..], Vec::as_slice) {
                if seen.insert(next) {
                    predecessor.insert(next, (current, effect, forward));
                    queue.push_back(next);
                }
            }
        }
        if !seen.contains(&revision) {
            return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
        }

        let mut reversed_path = Vec::new();
        let mut cursor = revision;
        while cursor != head.id() {
            let &(previous, effect, forward) = predecessor
                .get(&cursor)
                .ok_or(RuntimeHistoricalSnapshotError::Unavailable { revision })?;
            reversed_path.push((effect, forward));
            cursor = previous;
        }
        reversed_path.reverse();

        let mut current = head;
        for (effect_index, forward) in reversed_path {
            current = apply_runtime_history_effect(
                &current,
                &effects[effect_index],
                forward,
                &self.registry,
            )?;
        }
        if current.id() != revision {
            return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
        }
        Ok(current)
    }

    /// Certifies transport of one historical inverse from its original target
    /// revision to the current durable head.
    ///
    /// The certificate is issued only when every intervening exact effect has
    /// a disjoint semantic write footprint. Unknown/full/schema effects are
    /// deliberately opaque and therefore conflict rather than being replayed
    /// optimistically.
    pub fn certify_history_inverse_rebase(
        &self,
        effect_id: u128,
    ) -> Result<RuntimeHistoryRebaseOutcome, RuntimeHistoricalSnapshotError> {
        let head = self.snapshot()?.revision().clone();
        let head_effects = self.revision_history(head.id())?.ok_or(
            RuntimeHistoricalSnapshotError::Unavailable {
                revision: head.id(),
            },
        )?;
        let target = head_effects
            .iter()
            .find(|effect| effect.effect_id == effect_id)
            .ok_or(RuntimeHistoricalSnapshotError::EffectUnavailable { effect_id })?;
        if target.reversibility != RuntimeHistoryReversibility::ExactPlanInverse {
            return Err(RuntimeHistoricalSnapshotError::EffectNotReversible { effect_id });
        }

        if target.target_revision == head.id() {
            return Ok(RuntimeHistoryRebaseOutcome::Certified(
                RuntimeHistoryRebaseCertificate {
                    effect_id,
                    original_target_revision: target.target_revision,
                    current_revision: head.id(),
                    intervening_effects: Vec::new(),
                },
            ));
        }

        let anchored = self.revision_history(target.target_revision)?.ok_or(
            RuntimeHistoricalSnapshotError::Unavailable {
                revision: target.target_revision,
            },
        )?;
        if !anchored.iter().any(|effect| effect.effect_id == effect_id) {
            return Err(RuntimeHistoricalSnapshotError::EffectUnavailable { effect_id });
        }
        let anchored_ids = anchored
            .iter()
            .map(|effect| effect.effect_id)
            .collect::<BTreeSet<_>>();

        let target_source = self.revision_at(target.source_revision)?;
        let target_footprint =
            runtime_history_effect_footprint(target, &target_source, &self.registry)?;
        let mut intervening_effects = Vec::new();
        let mut conflicting_effects = BTreeSet::new();
        let mut conflict_coordinates = BTreeSet::new();
        let mut opaque_effects = BTreeSet::new();

        for effect in &head_effects {
            if anchored_ids.contains(&effect.effect_id) {
                continue;
            }
            intervening_effects.push(effect.effect_id);
            if effect.reversibility != RuntimeHistoryReversibility::ExactPlanInverse {
                opaque_effects.insert(effect.effect_id);
                continue;
            }
            let source = self.revision_at(effect.source_revision)?;
            let footprint = runtime_history_effect_footprint(effect, &source, &self.registry)?;
            let overlap = target_footprint
                .writes
                .intersection(&footprint.writes)
                .cloned()
                .collect::<Vec<_>>();
            if !overlap.is_empty() {
                conflicting_effects.insert(effect.effect_id);
                conflict_coordinates.extend(overlap);
            }
        }

        if !conflicting_effects.is_empty() || !opaque_effects.is_empty() {
            return Ok(RuntimeHistoryRebaseOutcome::Conflict(
                RuntimeHistoryRebaseConflict {
                    effect_id,
                    current_revision: head.id(),
                    conflicting_effects: conflicting_effects.into_iter().collect(),
                    coordinates: conflict_coordinates.into_iter().collect(),
                    opaque_effects: opaque_effects.into_iter().collect(),
                },
            ));
        }

        Ok(RuntimeHistoryRebaseOutcome::Certified(
            RuntimeHistoryRebaseCertificate {
                effect_id,
                original_target_revision: target.target_revision,
                current_revision: head.id(),
                intervening_effects,
            },
        ))
    }

    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.transaction_outcome(transaction_id))
    }

    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.transaction_outcome_at(epoch, transaction_id))
    }

    pub fn retry_horizon(&self) -> Result<(IdempotencyEpoch, IdempotencyEpoch), DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok((
            durability.current_idempotency_epoch(),
            durability.minimum_retry_epoch(),
        ))
    }

    pub fn advance_idempotency_epoch(&self, next: IdempotencyEpoch) -> Result<(), DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.advance_idempotency_epoch(next)
    }

    /// Advances the durable retry watermark in memory and drops retry-ledger
    /// payloads below it. Call `checkpoint` to publish the watermark and GC
    /// boundary durably. Γ-REIC causal records remain self-contained.
    pub fn expire_retry_history_before(
        &self,
        minimum: IdempotencyEpoch,
    ) -> Result<usize, DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.expire_retry_history_before(minimum)
    }

    pub fn revision_effect_ideal(
        &self,
        revision: RevisionId,
    ) -> Result<Option<kernel_change::RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError>
    {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.revision_effect_ideal(revision)
    }

    pub fn revision_history(
        &self,
        revision: RevisionId,
    ) -> Result<Option<Vec<RuntimeHistoryEffect>>, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability
            .revision_effect_records_for_revision(revision)?
            .map(|records| {
                records
                    .iter()
                    .map(RuntimeHistoryEffect::from_durable)
                    .collect()
            }))
    }

    pub fn causal_coverage_root(&self) -> Result<RevisionId, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.causal_coverage_root())
    }

    pub fn local_historical_complement_chain(
        &self,
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    ) -> Result<kernel_durability::LocalHistoricalComplementChain, DurableRuntimeHistoricalError>
    {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.local_historical_complement_chain(source, target)?)
    }

    pub fn restore_historical_value(
        &self,
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
        target_value: &kernel_model::Value,
        registry: &kernel_durability::HistoricalLensRegistry,
    ) -> Result<kernel_model::Value, DurableRuntimeHistoricalError> {
        let chain = self.local_historical_complement_chain(source, target)?;
        Ok(chain.restore_value(target_value, registry)?)
    }

    pub fn checkpoint(&self) -> Result<DurableGenerationReceipt, DurableRuntimeCheckpointError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        self.cell.checkpoint_durable(&mut durability)
    }

    pub fn compact_obsolete_generations(&self) -> Result<(), DurableRuntimeCheckpointError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        durability
            .compact_obsolete_generations()
            .map_err(DurableRuntimeCheckpointError::DurabilityUncertain)
    }

    pub fn install_i64_index(
        &self,
        index: I64IndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell.install_i64_index(index, registry)
    }

    pub fn install_semantic_index(
        &self,
        index: SemanticIndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell.install_semantic_index(index, registry)
    }

    pub fn install_observable_atom_state(
        &self,
        binding: SemanticIndexBinding,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell
            .install_observable_atom_state(binding, &self.registry)
    }

    pub fn advise_semantic_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: SemanticIndexAdvisorPolicy,
    ) -> Result<SemanticIndexAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_semantic_indexes(workload, policy, &self.registry)
    }

    pub fn advise_i64_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
    ) -> Result<I64IndexAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_i64_indexes(workload, policy, &self.registry)
    }

    pub fn advise_semantic_statistics(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
    ) -> Result<SemanticStatisticsAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_semantic_statistics(workload, policy, &self.registry)
    }

    pub(crate) fn advise_unified_observable_atoms(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        telemetry: &UnifiedAdvisorTelemetry,
        policy: UnifiedAdvisorPolicy,
        pressure_policy: PhysicalPressurePolicy,
        pressure_sample: PhysicalPressureSample,
    ) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
        self.cell.advise_unified_observable_atoms(
            workload,
            telemetry,
            policy,
            pressure_policy,
            pressure_sample,
            &self.registry,
        )
    }
}

fn apply_runtime_history_effect(
    source: &kernel_revision::Revision,
    effect: &RuntimeHistoryEffect,
    forward: bool,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<kernel_revision::Revision, RuntimeHistoricalSnapshotError> {
    let (expected_source, target_revision, model_delta) = if forward {
        (
            effect.source_revision,
            effect.target_revision,
            effect.model_delta.as_ref(),
        )
    } else {
        (
            effect.target_revision,
            effect.source_revision,
            effect.model_complement.as_ref(),
        )
    };
    if source.id() != expected_source
        || effect.reversibility != RuntimeHistoryReversibility::ExactPlanInverse
    {
        return Err(RuntimeHistoricalSnapshotError::Unavailable {
            revision: target_revision,
        });
    }

    let mut state = source.state().clone();
    for mutation in &effect.relation_mutations {
        let relation = RelExpr::Scan(mutation.relation);
        let result_type = relation
            .typecheck(source.semantic_context(), registry)
            .map_err(PhysicalExecutionError::from)?;
        let old = relation
            .evaluate(&state.model, source.semantic_context(), registry)
            .map_err(PhysicalExecutionError::from)?;
        let delta = RelationDelta {
            inserted: if forward {
                mutation.inserted.clone()
            } else {
                mutation.removed.clone()
            },
            removed: if forward {
                mutation.removed.clone()
            } else {
                mutation.inserted.clone()
            },
            result_type,
        };
        let next = delta
            .apply_to_value(old, source.semantic_context(), registry)
            .map_err(PhysicalExecutionError::from)?;
        state
            .model
            .relations
            .insert(mutation.relation, next.into_rows());
    }
    if let Some(model_delta) = model_delta {
        model_delta.apply_to(&mut state);
    }
    Ok(kernel_revision::Revision::build(
        target_revision,
        source.semantic_context(),
        registry,
        state,
    )?)
}

fn runtime_history_effect_footprint(
    effect: &RuntimeHistoryEffect,
    source: &kernel_revision::Revision,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RuntimeHistoryFootprint, RuntimeHistoricalSnapshotError> {
    let mut writes = BTreeSet::new();
    for mutation in &effect.relation_mutations {
        let relation = RelExpr::Scan(mutation.relation);
        let result_type = relation
            .typecheck(source.semantic_context(), registry)
            .map_err(PhysicalExecutionError::from)?;
        let delta = RelationDelta {
            inserted: mutation.inserted.clone(),
            removed: mutation.removed.clone(),
            result_type,
        };
        let footprint = delta
            .rewrite_footprint(mutation.relation, source.semantic_context(), registry)
            .map_err(PhysicalExecutionError::from)?;
        for coordinate in footprint.writes.keys() {
            match coordinate {
                kernel_change::SemanticWriteCoordinate::RelationClass {
                    relation,
                    canonical_key,
                } => {
                    writes.insert(RuntimeHistoryCoordinate::RelationClass {
                        relation: *relation,
                        canonical_key: canonical_key.clone(),
                    });
                }
                // RelationDelta::rewrite_footprint currently emits only class
                // coordinates. Treat any future widening as unavailable rather
                // than accidentally weakening the certificate.
                _ => {
                    return Err(RuntimeHistoricalSnapshotError::EffectUnavailable {
                        effect_id: effect.effect_id,
                    });
                }
            }
        }
    }

    if let Some(delta) = &effect.model_delta {
        add_model_delta_footprint(&mut writes, delta, effect.model_complement.as_ref());
    }
    Ok(RuntimeHistoryFootprint { writes })
}

fn add_model_delta_footprint(
    writes: &mut BTreeSet<RuntimeHistoryCoordinate>,
    delta: &DurableModelDelta,
    complement: Option<&DurableModelDelta>,
) {
    let complement_carriers = complement
        .into_iter()
        .flat_map(|delta| delta.carriers.iter())
        .map(|patch| (patch.carrier, patch.target_present))
        .collect::<BTreeMap<_, _>>();
    for patch in &delta.carriers {
        if complement_carriers
            .get(&patch.carrier)
            .is_some_and(|source_present| *source_present != patch.target_present)
        {
            writes.insert(RuntimeHistoryCoordinate::CarrierPresence {
                carrier: patch.carrier,
            });
        }
        for entity in patch.inserted.iter().chain(&patch.removed) {
            writes.insert(RuntimeHistoryCoordinate::CarrierMember {
                carrier: patch.carrier,
                entity: *entity,
            });
        }
    }
    for patch in &delta.fields {
        writes.insert(RuntimeHistoryCoordinate::Field {
            field: patch.field,
            owner: patch.owner,
        });
    }
    for entity in delta
        .lifecycle_entities_inserted
        .iter()
        .chain(&delta.lifecycle_entities_removed)
    {
        writes.insert(RuntimeHistoryCoordinate::LifecycleEntity { entity: *entity });
    }
    for entity in delta
        .lifecycle_roots_inserted
        .iter()
        .chain(&delta.lifecycle_roots_removed)
    {
        writes.insert(RuntimeHistoryCoordinate::LifecycleRoot { entity: *entity });
    }
    let complement_keeps_alive = complement
        .into_iter()
        .flat_map(|delta| delta.lifecycle_keeps_alive.iter())
        .map(|patch| (patch.parent, patch.target_present))
        .collect::<BTreeMap<_, _>>();
    for patch in &delta.lifecycle_keeps_alive {
        if complement_keeps_alive
            .get(&patch.parent)
            .is_some_and(|source_present| *source_present != patch.target_present)
        {
            writes.insert(RuntimeHistoryCoordinate::KeepsAlivePresence {
                parent: patch.parent,
            });
        }
        for child in patch.inserted.iter().chain(&patch.removed) {
            writes.insert(RuntimeHistoryCoordinate::KeepsAliveEdge {
                parent: patch.parent,
                child: *child,
            });
        }
    }
}
