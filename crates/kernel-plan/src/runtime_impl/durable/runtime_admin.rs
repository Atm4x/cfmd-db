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
        self.revision_publication
            .notify_relations_published(relations);
    }

    /// Reconstructs an exact committed logical revision reachable from the
    /// current durable head through reversible causal effects.
    ///
    /// No historical snapshot is persisted separately: the current immutable
    /// revision plus the Γ-REIC effect ideal and exact forward/reverse deltas
    /// are the sole authority. Non-reversible legacy/full/schema boundaries
    /// fail closed instead of synthesizing an approximate state.
    pub fn factorized_read_snapshot_at(
        &self,
        revision: RevisionId,
    ) -> Result<Option<kernel_durability::DurableFactorizedReadSnapshot>, DurabilityError> {
        {
            let durability = self
                .durability
                .lock()
                .map_err(|_| DurabilityError::Poisoned)?;
            if let Some(snapshot) = durability.historical_factorized_read_snapshot(revision) {
                return Ok(Some(snapshot));
            }
        }

        let head = self
            .snapshot()
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "runtime snapshot unavailable for factorized history derivation",
            })?
            .revision()
            .id();
        let Some(effects) = self.revision_history(head)? else {
            return Ok(None);
        };

        let mut queue = VecDeque::new();
        let mut seeded = BTreeSet::new();
        for effect in &effects {
            if !seeded.insert(effect.source_revision) {
                continue;
            }
            let snapshot = {
                let durability = self
                    .durability
                    .lock()
                    .map_err(|_| DurabilityError::Poisoned)?;
                durability.historical_factorized_read_snapshot(effect.source_revision)
            };
            if let Some(snapshot) = snapshot {
                queue.push_back(snapshot);
            }
        }

        let mut visited = BTreeSet::new();
        while let Some(snapshot) = queue.pop_front() {
            if snapshot.revision() == revision {
                return Ok(Some(snapshot));
            }
            if !visited.insert(snapshot.revision()) {
                continue;
            }
            for effect in effects.iter().filter(|effect| {
                effect.source_revision == snapshot.revision()
                    && effect.reversibility == RuntimeHistoryReversibility::ExactPlanInverse
                    && effect.semantic_change.is_none()
                    && matches!(
                        effect.kind,
                        RuntimeHistoryEffectKind::RelationData
                            | RuntimeHistoryEffectKind::RelationRewrite
                            | RuntimeHistoryEffectKind::RelationResolution
                            | RuntimeHistoryEffectKind::MixedRevision
                    )
            }) {
                let mut next = snapshot.clone();
                let mut valid =
                    !effect.relation_mutations.is_empty() || effect.model_delta.is_some();
                for mutation in &effect.relation_mutations {
                    let Ok(result_type) = RelExpr::Scan(mutation.relation)
                        .typecheck(next.semantic_context(), &self.registry)
                    else {
                        valid = false;
                        break;
                    };
                    let delta = RelationDelta {
                        inserted: mutation.inserted.clone(),
                        removed: mutation.removed.clone(),
                        result_type,
                    };
                    if let Ok(advanced) = next.advance_relation_delta(
                        effect.target_revision,
                        mutation.relation,
                        &delta,
                        &self.registry,
                    ) { next = advanced } else {
                        valid = false;
                        break;
                    }
                }
                if valid && let Some(model_delta) = &effect.model_delta {
                    match next.advance_model_delta(effect.target_revision, model_delta) {
                        Ok(advanced) => next = advanced,
                        Err(_) => valid = false,
                    }
                }
                if valid && !visited.contains(&next.revision()) {
                    queue.push_back(next);
                }
            }
        }
        Ok(None)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
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
        let mut adjacency = BTreeMap::<RevisionId, Vec<(RevisionId, usize, bool, bool)>>::new();
        for (index, effect) in effects.iter().enumerate() {
            if effect.reversibility == RuntimeHistoryReversibility::ExactPlanInverse {
                adjacency.entry(effect.source_revision).or_default().push((
                    effect.target_revision,
                    index,
                    true,
                    false,
                ));
                adjacency.entry(effect.target_revision).or_default().push((
                    effect.source_revision,
                    index,
                    false,
                    false,
                ));
            } else if effect.kind == RuntimeHistoryEffectKind::SchemaMigration
                && effect.semantic_change.is_some()
            {
                // Crossing a schema boundary backwards is not an inverse
                // migration.  It switches authority to the pinned source
                // epoch; the source world is then reconstructed from that
                // epoch's own checkpoint + WAL.
                adjacency.entry(effect.target_revision).or_default().push((
                    effect.source_revision,
                    index,
                    false,
                    true,
                ));
            }
        }

        let mut queue = VecDeque::from([head.id()]);
        let mut predecessor = BTreeMap::<RevisionId, (RevisionId, usize, bool, bool)>::new();
        let mut seen = BTreeSet::from([head.id()]);
        while let Some(current) = queue.pop_front() {
            if current == revision {
                break;
            }
            for &(next, effect, forward, migration) in
                adjacency.get(&current).map_or(&[][..], Vec::as_slice)
            {
                if seen.insert(next) {
                    predecessor.insert(next, (current, effect, forward, migration));
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
            let &(previous, effect, forward, migration) = predecessor
                .get(&cursor)
                .ok_or(RuntimeHistoricalSnapshotError::Unavailable { revision })?;
            reversed_path.push((effect, forward, migration));
            cursor = previous;
        }
        reversed_path.reverse();

        let mut current = head;
        let mut registry = self.registry.clone();
        for (effect_index, forward, migration) in reversed_path {
            let effect = &effects[effect_index];
            if migration {
                if forward || current.id() != effect.target_revision {
                    return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
                }
                let semantic_change = effect
                    .semantic_change
                    .as_ref()
                    .ok_or(RuntimeHistoricalSnapshotError::Unavailable { revision })?;
                let root_backed = {
                    let durability = self
                        .durability
                        .lock()
                        .map_err(|_| DurabilityError::Poisoned)?;
                    durability.historical_revision_from_realization(
                        kernel_change::RevisionEffectId(effect.effect_id),
                    )?
                };
                let restored = if let Some(restored) = root_backed {
                    restored
                } else {
                    let material = {
                        let mut durability = self
                            .durability
                            .lock()
                            .map_err(|_| DurabilityError::Poisoned)?;
                        durability.historical_epoch_material(kernel_change::RevisionEffectId(
                            effect.effect_id,
                        ))?
                    }
                    .ok_or(RuntimeHistoricalSnapshotError::Unavailable {
                        revision: effect.source_revision,
                    })?;
                    registry = material.semantic_registry().clone();
                    crate::replay_durable_revisions_until(
                        material.checkpoint(),
                        material.recovery_scan(),
                        material.semantic_registry(),
                        effect.source_revision,
                    )?
                };
                if restored.semantic_revision().schema != semantic_change.source_schema
                    || restored.id() != effect.source_revision
                {
                    return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
                }
                current = restored;
            } else {
                current = apply_runtime_history_effect(&current, effect, forward, &registry)?;
            }
        }
        if current.id() != revision {
            return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
        }
        Ok(current)
    }

    /// Validates one exact relation delta in its historical formation world
    /// using only retained Γ-support plus exact durable deltas.
    ///
    /// The current `RelationBaseWitness` is projected to a support-only
    /// authority in O(1), then rewound along the exact reversible causal path.
    /// No historical relation rows are materialized or Γ-canonicalized again.
    /// Schema/non-plan boundaries fail closed because support transport across
    /// a semantic-context change requires an explicit bridge law.
    pub fn validate_relation_delta_at(
        &self,
        revision: RevisionId,
        relation: SemanticId,
        delta: &RelationDelta,
    ) -> Result<(), RuntimeHistoricalSnapshotError> {
        let snapshot = self.snapshot()?;
        let support = snapshot
            .root()
            .historical_support_at(revision, relation)
            .ok_or(RuntimeHistoricalSnapshotError::Unavailable { revision })?;
        if support.result_type() != &delta.result_type {
            return Err(RuntimeHistoricalSnapshotError::Unavailable { revision });
        }
        support
            .validate_exact_delta(delta, &self.registry)
            .map_err(PhysicalExecutionError::from)
            .map_err(Into::into)
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
        let target_footprint = runtime_history_effect_footprint(
            target,
            target_source.semantic_context(),
            &self.registry,
        )?;
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
            let footprint = runtime_history_effect_footprint(
                effect,
                source.semantic_context(),
                &self.registry,
            )?;
            let overlap = target_footprint
                .writes
                .keys()
                .filter(|coordinate| footprint.writes.contains_key(*coordinate))
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

    /// Certifies transport of one proposed exact transition from its source revision to the
    /// current durable head. Only exact semantic write footprints are accepted: every
    /// intervening exact effect must be disjoint, while opaque/full/schema boundaries fail
    /// closed. This is the product-facing bridge to Γ-aware stale-intent transport; it does not
    /// synthesize a merge endpoint for overlapping writes.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn certify_transition_rebase(
        &self,
        source_revision: RevisionId,
        mutations: &[RevisionRelationMutation<'_>],
        model_delta: Option<&DurableModelDelta>,
        model_complement: Option<&DurableModelDelta>,
    ) -> Result<RuntimeTransitionRebaseOutcome, RuntimeHistoricalSnapshotError> {
        let snapshot = self.snapshot()?;
        let head = snapshot.revision();
        let historical = &snapshot.root().historical;
        if source_revision == head.id() {
            return Ok(RuntimeTransitionRebaseOutcome::Certified(
                RuntimeTransitionRebaseCertificate {
                    source_revision,
                    current_revision: head.id(),
                    intervening_effect_count: 0,
                },
            ));
        }
        if source_revision > head.id() {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }
        if historical
            .lineage_floor
            .is_some_and(|floor| source_revision < floor)
        {
            if historical.floor_opaque_effect.is_none() {
                return Err(RuntimeHistoricalSnapshotError::Unavailable {
                    revision: source_revision,
                });
            }
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: Vec::new(),
                    coordination_effects: Vec::new(),
                    coordinates: Vec::new(),
                    opaque_effects: historical.floor_opaque_effect.into_iter().collect(),
                },
            ));
        }
        if !historical.contains_revision(source_revision) {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }

        let proposed = proposed_transition_footprint(
            source_revision,
            head.semantic_context(),
            mutations,
            model_delta,
            model_complement,
            &self.registry,
        )?;
        let mut conflicting_effects = BTreeSet::new();
        let mut coordination_effects = BTreeSet::new();
        let mut conflict_coordinates = BTreeSet::new();

        for (coordinate, proposed_action) in &proposed.writes {
            for indexed in historical.actions_after(source_revision, coordinate) {
                let law = if matches!(coordinate, RuntimeHistoryCoordinate::ObjectField { .. }) {
                    kernel_change::PairRewriteLaw::DefiniteIntentConflict
                } else {
                    let left = BTreeMap::from([((), proposed_action.clone())]);
                    let right = BTreeMap::from([((), indexed.action.clone())]);
                    kernel_change::infer_write_action_law(&left, &right)
                };
                match law {
                    kernel_change::PairRewriteLaw::StrongCommute
                    | kernel_change::PairRewriteLaw::SameIdempotentIntent => {}
                    kernel_change::PairRewriteLaw::DefiniteIntentConflict => {
                        conflicting_effects.insert(indexed.effect_id);
                        conflict_coordinates.insert(coordinate.clone());
                    }
                    kernel_change::PairRewriteLaw::Unknown => {
                        coordination_effects.insert(indexed.effect_id);
                        conflict_coordinates.insert(coordinate.clone());
                    }
                }
            }
        }

        if !conflicting_effects.is_empty() || !coordination_effects.is_empty() {
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: conflicting_effects.into_iter().collect(),
                    coordination_effects: coordination_effects.into_iter().collect(),
                    coordinates: conflict_coordinates.into_iter().collect(),
                    opaque_effects: Vec::new(),
                },
            ));
        }

        let mut relation_deltas = BTreeMap::<SemanticId, RelationDelta>::new();
        for mutation in mutations {
            match relation_deltas.entry(mutation.relation) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(mutation.delta.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get().result_type != mutation.delta.result_type {
                        return Err(RuntimeHistoricalSnapshotError::Unavailable {
                            revision: source_revision,
                        });
                    }
                    entry.get_mut().inserted.extend(mutation.delta.inserted.clone());
                    entry.get_mut().removed.extend(mutation.delta.removed.clone());
                }
            }
        }
        if !relation_deltas.is_empty()
            && let RuntimeTransitionRebaseOutcome::Conflict(conflict) = snapshot
                .certify_retroactive_relation_deltas_against_relational_causal_observations(
                    source_revision,
                    &relation_deltas,
                    &self.registry,
                )?
            {
                return Ok(RuntimeTransitionRebaseOutcome::Conflict(conflict));
            }

        Ok(RuntimeTransitionRebaseOutcome::Certified(
            RuntimeTransitionRebaseCertificate {
                source_revision,
                current_revision: head.id(),
                intervening_effect_count: historical.exact_effect_count_after(source_revision),
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

    pub fn schema_migration_physical_states(
        &self,
    ) -> Result<Vec<kernel_durability::SchemaMigrationPhysicalState>, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.schema_migration_physical_states()
    }

    /// Publishes one native checkpoint only when at least one semantic schema
    /// cutover is still represented above the checkpoint frontier.  This is an
    /// operational physical transition; it does not create a database revision
    /// or semantic history event.
    pub fn materialize_pending_schema_migrations(
        &self,
    ) -> Result<Option<DurableGenerationReceipt>, DurableRuntimeCheckpointError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        let pending = durability
            .schema_migration_physical_states()
            .map_err(DurableRuntimeCheckpointError::Durability)?
            .into_iter()
            .any(|state| state.authority.is_pending());
        if !pending {
            return Ok(None);
        }
        self.cell.checkpoint_durable(&mut durability).map(Some)
    }

    pub fn checkpoint(&self) -> Result<DurableGenerationReceipt, DurableRuntimeCheckpointError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        self.cell.checkpoint_durable(&mut durability)
    }

    /// Irreversibly expires local causal replay before the current head and
    /// publishes the new causal coverage root. The P462 derived Γ-support and
    /// coordinate timelines are reset in the same runtime publication so they
    /// cannot retain authority that durability has released.
    pub fn release_causal_history_before_head(
        &self,
    ) -> Result<Option<DurableGenerationReceipt>, DurableRuntimeCheckpointError> {
        let snapshot = self
            .snapshot()
            .map_err(DurableRuntimeCheckpointError::Runtime)?;
        let revision = snapshot.revision().clone();
        drop(snapshot);
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        let receipt = durability
            .release_causal_history_before_head(&revision)
            .map_err(|error| {
                if durability.requires_recovery() {
                    let _ = self.cell.force_recovery_required();
                    DurableRuntimeCheckpointError::DurabilityUncertain(error)
                } else {
                    DurableRuntimeCheckpointError::Durability(error)
                }
            })?;
        if receipt.is_some() {
            self.cell
                .reset_historical_derived_to_current()
                .map_err(DurableRuntimeCheckpointError::Runtime)?;
        }
        Ok(receipt)
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

    pub fn install_observable_atom_state(
        &self,
        binding: SemanticIndexBinding,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell
            .install_observable_atom_state(binding, &self.registry)
    }

    pub fn advise_i64_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
    ) -> Result<I64IndexAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_i64_indexes(workload, policy, &self.registry)
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

impl RuntimeRevisionSnapshot {
    pub fn certify_retroactive_writes_against_causal_observations(
        &self,
        source_revision: RevisionId,
        coordinates: impl IntoIterator<Item = RuntimeHistoryCoordinate>,
    ) -> Result<RuntimeTransitionRebaseOutcome, RuntimeHistoricalSnapshotError> {
        self.certify_retroactive_exact_writes_against_causal_observations(
            source_revision,
            coordinates.into_iter().map(|coordinate| (coordinate, None)),
        )
    }

    pub fn certify_retroactive_relation_deltas_against_relational_causal_observations(
        &self,
        source_revision: RevisionId,
        deltas: &BTreeMap<SemanticId, RelationDelta>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RuntimeTransitionRebaseOutcome, RuntimeHistoricalSnapshotError> {
        let head = self.revision();
        let historical = &self.root().historical;
        if source_revision > head.id() || !historical.contains_revision(source_revision) {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }
        let mut observations = BTreeMap::new();
        for relation in deltas.keys() {
            for (observation_ref, capsule) in
                historical.relational_observations_after(source_revision, *relation)
            {
                observations.entry(observation_ref).or_insert(capsule);
            }
        }
        let observation_entries = observations.into_iter().collect::<Vec<_>>();
        let capsules = observation_entries
            .iter()
            .map(|(_, capsule)| *capsule)
            .collect::<Vec<_>>();
        let impacts = RelCausalCapsule::impact_many_relation_deltas(
            &capsules,
            deltas,
            head.semantic_context(),
            registry,
        )
        .map_err(PhysicalExecutionError::from)?;
        let mut coordination_effects = BTreeSet::new();
        for ((observation_ref, _), impact) in observation_entries.into_iter().zip(impacts) {
            if impact == Impact::Changed {
                coordination_effects.insert(observation_ref.effect_id);
            }
        }
        if !coordination_effects.is_empty() {
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: Vec::new(),
                    coordination_effects: coordination_effects.into_iter().collect(),
                    coordinates: Vec::new(),
                    opaque_effects: Vec::new(),
                },
            ));
        }
        Ok(RuntimeTransitionRebaseOutcome::Certified(
            RuntimeTransitionRebaseCertificate {
                source_revision,
                current_revision: head.id(),
                intervening_effect_count: historical.exact_effect_count_after(source_revision),
            },
        ))
    }

    /// Exact B/C-native anti-dependency proof for retroactive sealed writes.
    ///
    /// An overlapping later observation is allowed only when the durable
    /// observation carries an exact value and the retroactively inserted write
    /// leaves that observed input byte-for-byte/semantically equal. This is a
    /// universal preservation certificate: it never inspects an old-schema
    /// guard and never guesses predicate semantics.
    pub fn certify_retroactive_exact_writes_against_causal_observations(
        &self,
        source_revision: RevisionId,
        writes: impl IntoIterator<Item = (RuntimeHistoryCoordinate, Option<Value>)>,
    ) -> Result<RuntimeTransitionRebaseOutcome, RuntimeHistoricalSnapshotError> {
        let head = self.revision();
        let historical = &self.root().historical;
        if source_revision > head.id() || !historical.contains_revision(source_revision) {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }
        let writes = writes.into_iter().collect::<Vec<_>>();
        let proposed_fields = proposed_field_values(&writes);
        let mut effects = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        let mut joint_groups = BTreeMap::<(u128, u32), RuntimeJointCausalObservationGroup>::new();
        for (coordinate, proposed_value) in &writes {
            for observation in historical.observations_after(source_revision, coordinate) {
                let exact_preserved = proposed_value
                    .as_ref()
                    .zip(observation.exact_value.as_ref())
                    .is_some_and(|(proposed, observed)| proposed == observed);
                let predicate_preserved = proposed_value
                    .as_ref()
                    .zip(observation.preservation_rule.as_ref())
                    .is_some_and(|(proposed, rule)| {
                        kernel_validation::semantic_rule_matches(rule, proposed).unwrap_or(false)
                    });
                let has_scalar_certificate =
                    observation.exact_value.is_some() || observation.preservation_rule.is_some();
                if (has_scalar_certificate && !exact_preserved && !predicate_preserved)
                    || (!has_scalar_certificate && observation.joint_group_ids.is_empty())
                {
                    effects.insert(observation.effect_id);
                    blocked.insert(coordinate.clone());
                }
                for &group_id in &observation.joint_group_ids {
                    if let Some(group) =
                        historical.joint_observation_group(observation.effect_id, group_id)
                    {
                        joint_groups
                            .entry((observation.effect_id, group_id))
                            .or_insert_with(|| group.clone());
                    }
                }
            }
        }
        for ((effect_id, _), group) in joint_groups {
            if !joint_causal_observation_preserved(&group, &proposed_fields) {
                effects.insert(effect_id);
                for field in group.observed_fields.keys() {
                    if proposed_fields.contains_key(&(group.owner, *field)) {
                        blocked.insert(RuntimeHistoryCoordinate::Field {
                            field: *field,
                            owner: group.owner,
                        });
                    }
                }
            }
        }
        if !effects.is_empty() {
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: Vec::new(),
                    coordination_effects: effects.into_iter().collect(),
                    coordinates: blocked.into_iter().collect(),
                    opaque_effects: Vec::new(),
                },
            ));
        }
        Ok(RuntimeTransitionRebaseOutcome::Certified(
            RuntimeTransitionRebaseCertificate {
                source_revision,
                current_revision: head.id(),
                intervening_effect_count: historical.exact_effect_count_after(source_revision),
            },
        ))
    }

    /// Certifies that every passive guard/read dependency remained untouched
    /// between `source_revision` and this exact immutable snapshot. Unlike
    /// write/write rebase, any later action on a dependency invalidates the
    /// observation; there is deliberately no idempotent-write escape hatch.
    ///
    /// Callers that prepare a transition from this same snapshot inherit an
    /// atomic freshness seal: `PreparedRuntimeRevisionTransition::seal` rejects
    /// publication if the live root advanced after this proof.
    pub fn certify_read_dependencies_stable(
        &self,
        observation: &RuntimeGuardObservationFootprint,
    ) -> Result<RuntimeTransitionRebaseOutcome, RuntimeHistoricalSnapshotError> {
        let source_revision = observation.source_revision();
        let dependencies = observation.coordinates();
        let head = self.revision();
        let historical = &self.root().historical;
        if source_revision == head.id() {
            return Ok(RuntimeTransitionRebaseOutcome::Certified(
                RuntimeTransitionRebaseCertificate {
                    source_revision,
                    current_revision: head.id(),
                    intervening_effect_count: 0,
                },
            ));
        }
        if source_revision > head.id() {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }
        if historical
            .lineage_floor
            .is_some_and(|floor| source_revision < floor)
        {
            if historical.floor_opaque_effect.is_none() {
                return Err(RuntimeHistoricalSnapshotError::Unavailable {
                    revision: source_revision,
                });
            }
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: Vec::new(),
                    coordination_effects: Vec::new(),
                    coordinates: dependencies.to_vec(),
                    opaque_effects: historical.floor_opaque_effect.into_iter().collect(),
                },
            ));
        }
        if !historical.contains_revision(source_revision) {
            return Err(RuntimeHistoricalSnapshotError::Unavailable {
                revision: source_revision,
            });
        }

        let mut conflicting_effects = BTreeSet::new();
        let mut conflict_coordinates = BTreeSet::new();
        for coordinate in dependencies {
            for indexed in historical.actions_after(source_revision, coordinate) {
                conflicting_effects.insert(indexed.effect_id);
                conflict_coordinates.insert(coordinate.clone());
            }
        }
        if !conflicting_effects.is_empty() {
            return Ok(RuntimeTransitionRebaseOutcome::Conflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision: head.id(),
                    conflicting_effects: conflicting_effects.into_iter().collect(),
                    coordination_effects: Vec::new(),
                    coordinates: conflict_coordinates.into_iter().collect(),
                    opaque_effects: Vec::new(),
                },
            ));
        }
        Ok(RuntimeTransitionRebaseOutcome::Certified(
            RuntimeTransitionRebaseCertificate {
                source_revision,
                current_revision: head.id(),
                intervening_effect_count: historical.exact_effect_count_after(source_revision),
            },
        ))
    }
}

fn proposed_field_values(
    writes: &[(RuntimeHistoryCoordinate, Option<Value>)],
) -> BTreeMap<(kernel_types::EntityId, SemanticId), Option<Value>> {
    let mut out = BTreeMap::new();
    for (coordinate, value) in writes {
        match coordinate {
            RuntimeHistoryCoordinate::Field { field, owner }
            | RuntimeHistoryCoordinate::ObjectField { field, owner, .. } => {
                out.insert((*owner, *field), value.clone());
            }
            _ => {}
        }
    }
    out
}

fn joint_causal_observation_preserved(
    group: &RuntimeJointCausalObservationGroup,
    proposed_fields: &BTreeMap<(kernel_types::EntityId, SemanticId), Option<Value>>,
) -> bool {
    let mut candidate = group.observed_fields.clone();
    for field in group.observed_fields.keys() {
        if let Some(proposed) = proposed_fields.get(&(group.owner, *field)) {
            match proposed {
                Some(value) => {
                    candidate.insert(*field, value.clone());
                }
                None => {
                    candidate.remove(field);
                }
            }
        }
    }
    kernel_validation::semantic_rule_matches_fields(&group.predicate, &candidate).unwrap_or(false)
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

fn proposed_transition_footprint(
    source_revision: RevisionId,
    semantic_context: &kernel_schema::SemanticContext,
    mutations: &[RevisionRelationMutation<'_>],
    model_delta: Option<&DurableModelDelta>,
    model_complement: Option<&DurableModelDelta>,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RuntimeHistoryFootprint, RuntimeHistoricalSnapshotError> {
    let mut proposed = RuntimeHistoryFootprint::default();
    for mutation in mutations {
        if !mutation.object_field_writes.is_empty() {
            for write in mutation.object_field_writes {
                insert_runtime_history_action(
                    &mut proposed.writes,
                    RuntimeHistoryCoordinate::ObjectField {
                        relation: mutation.relation,
                        owner: write.owner,
                        field: write.field,
                    },
                    kernel_change::RewriteActionLaw::Opaque,
                );
            }
            continue;
        }
        let footprint = mutation
            .delta
            .rewrite_footprint(mutation.relation, semantic_context, registry)
            .map_err(PhysicalExecutionError::from)?;
        for (coordinate, action) in &footprint.writes {
            let kernel_change::SemanticWriteCoordinate::RelationClass {
                relation,
                canonical_key,
            } = coordinate
            else {
                return Err(RuntimeHistoricalSnapshotError::Unavailable {
                    revision: source_revision,
                });
            };
            insert_runtime_history_action(
                &mut proposed.writes,
                RuntimeHistoryCoordinate::RelationClass {
                    relation: *relation,
                    canonical_key: canonical_key.clone(),
                },
                action.clone(),
            );
        }
    }
    if let Some(delta) = model_delta {
        add_model_delta_footprint(&mut proposed.writes, delta, model_complement);
    }
    Ok(proposed)
}

fn runtime_history_effect_footprint(
    effect: &RuntimeHistoryEffect,
    semantic_context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RuntimeHistoryFootprint, RuntimeHistoricalSnapshotError> {
    let mut writes = BTreeMap::new();
    for mutation in &effect.relation_mutations {
        if !mutation.object_field_writes.is_empty() {
            for write in &mutation.object_field_writes {
                insert_runtime_history_action(
                    &mut writes,
                    RuntimeHistoryCoordinate::ObjectField {
                        relation: mutation.relation,
                        owner: write.owner,
                        field: write.field,
                    },
                    kernel_change::RewriteActionLaw::Opaque,
                );
            }
            continue;
        }
        let relation = RelExpr::Scan(mutation.relation);
        let result_type = relation
            .typecheck(semantic_context, registry)
            .map_err(PhysicalExecutionError::from)?;
        let delta = RelationDelta {
            inserted: mutation.inserted.clone(),
            removed: mutation.removed.clone(),
            result_type,
        };
        let footprint = delta
            .rewrite_footprint(mutation.relation, semantic_context, registry)
            .map_err(PhysicalExecutionError::from)?;
        for (coordinate, action) in &footprint.writes {
            match coordinate {
                kernel_change::SemanticWriteCoordinate::RelationClass {
                    relation,
                    canonical_key,
                } => {
                    insert_runtime_history_action(
                        &mut writes,
                        RuntimeHistoryCoordinate::RelationClass {
                            relation: *relation,
                            canonical_key: canonical_key.clone(),
                        },
                        action.clone(),
                    );
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

fn insert_runtime_history_action(
    writes: &mut BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
    coordinate: RuntimeHistoryCoordinate,
    action: kernel_change::RewriteActionLaw,
) {
    use std::collections::btree_map::Entry;

    match writes.entry(coordinate) {
        Entry::Vacant(entry) => {
            entry.insert(action);
        }
        Entry::Occupied(mut entry) if entry.get() != &action => {
            entry.insert(kernel_change::RewriteActionLaw::Opaque);
        }
        Entry::Occupied(_) => {}
    }
}

fn add_model_delta_footprint(
    writes: &mut BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
    delta: &DurableModelDelta,
    complement: Option<&DurableModelDelta>,
) {
    add_carrier_delta_footprint(writes, delta, complement);
    add_field_and_lifecycle_delta_footprint(writes, delta);
    add_keeps_alive_delta_footprint(writes, delta, complement);
}

fn add_carrier_delta_footprint(
    writes: &mut BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
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
            insert_runtime_history_action(
                writes,
                RuntimeHistoryCoordinate::CarrierPresence {
                    carrier: patch.carrier,
                },
                if patch.target_present {
                    kernel_change::RewriteActionLaw::EnsurePresent
                } else {
                    kernel_change::RewriteActionLaw::EnsureAbsent
                },
            );
        }
        for entity in patch.inserted.iter().chain(&patch.removed) {
            insert_runtime_history_action(
                writes,
                RuntimeHistoryCoordinate::CarrierMember {
                    carrier: patch.carrier,
                    entity: *entity,
                },
                kernel_change::RewriteActionLaw::Opaque,
            );
        }
    }
}

fn add_field_and_lifecycle_delta_footprint(
    writes: &mut BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
    delta: &DurableModelDelta,
) {
    for patch in &delta.fields {
        insert_runtime_history_action(
            writes,
            RuntimeHistoryCoordinate::Field {
                field: patch.field,
                owner: patch.owner,
            },
            kernel_change::RewriteActionLaw::Opaque,
        );
    }
    for (entities, action) in [
        (
            &delta.lifecycle_entities_inserted,
            kernel_change::RewriteActionLaw::EnsurePresent,
        ),
        (
            &delta.lifecycle_entities_removed,
            kernel_change::RewriteActionLaw::EnsureAbsent,
        ),
    ] {
        for entity in entities {
            insert_runtime_history_action(
                writes,
                RuntimeHistoryCoordinate::LifecycleEntity { entity: *entity },
                action.clone(),
            );
        }
    }
    for (roots, action) in [
        (
            &delta.lifecycle_roots_inserted,
            kernel_change::RewriteActionLaw::EnsurePresent,
        ),
        (
            &delta.lifecycle_roots_removed,
            kernel_change::RewriteActionLaw::EnsureAbsent,
        ),
    ] {
        for entity in roots {
            insert_runtime_history_action(
                writes,
                RuntimeHistoryCoordinate::LifecycleRoot { entity: *entity },
                action.clone(),
            );
        }
    }
}

fn add_keeps_alive_delta_footprint(
    writes: &mut BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
    delta: &DurableModelDelta,
    complement: Option<&DurableModelDelta>,
) {
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
            insert_runtime_history_action(
                writes,
                RuntimeHistoryCoordinate::KeepsAlivePresence {
                    parent: patch.parent,
                },
                if patch.target_present {
                    kernel_change::RewriteActionLaw::EnsurePresent
                } else {
                    kernel_change::RewriteActionLaw::EnsureAbsent
                },
            );
        }
        for (children, action) in [
            (
                &patch.inserted,
                kernel_change::RewriteActionLaw::EnsurePresent,
            ),
            (
                &patch.removed,
                kernel_change::RewriteActionLaw::EnsureAbsent,
            ),
        ] {
            for child in children {
                insert_runtime_history_action(
                    writes,
                    RuntimeHistoryCoordinate::KeepsAliveEdge {
                        parent: patch.parent,
                        child: *child,
                    },
                    action.clone(),
                );
            }
        }
    }
}
