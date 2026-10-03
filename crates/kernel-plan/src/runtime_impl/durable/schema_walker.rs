impl DurableRuntime {
    /// Transports one exact field-only client intent across durable schema epochs and
    /// publishes the current-schema residual under one final freshness seal.
    ///
    /// This is deliberately fail-closed outside the field-coordinate class: carrier,
    /// lifecycle and relation effects require their own certified migration provenance.
    /// Ordinary history inside old epochs is checked from the authoritative durable
    /// transition lineage; the current epoch uses the P462 coordinate index.
    pub fn commit_schema_aware_field_intent(
        &self,
        transaction_id: ClientTransactionId,
        request: &SchemaAwareFieldTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        if !model_delta_is_field_only(request.client_model_delta) {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        if request
            .guard_observation
            .is_some_and(|guard| guard.source_revision() != request.formation_revision)
        {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }

        if let Some(target_revision) = self.check_mixed_client_retry(
            transaction_id,
            request.formation_semantic_revision,
            request.client_model_delta,
            request.client_guard_digest,
            request.formation_revision,
        )? {
            return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision });
        }

        let head_snapshot = self.snapshot()?;
        let head_revision = head_snapshot.revision().id();
        if request.formation_revision > head_revision {
            return Err(DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: request.formation_revision,
            });
        }
        let historical = head_snapshot.root().historical.clone();
        let retained_epochs = historical
            .retained_schema_epochs
            .iter()
            .filter_map(|(_, epoch)| {
                (epoch.source_revision >= request.formation_revision
                    && epoch.target_revision <= head_revision)
                    .then_some(epoch.clone())
            })
            .collect::<Vec<_>>();
        let mut context = retained_epochs
            .first()
            .map(|epoch| epoch.source_context.clone())
            .unwrap_or_else(|| head_snapshot.revision().semantic_context().clone());
        if context.revision() != request.formation_semantic_revision {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        drop(head_snapshot);

        let mut realized = request.client_model_delta.clone();
        let mut guard_coordinates = request
            .guard_observation
            .map(|guard| guard.coordinates().iter().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        if guard_coordinates
            .iter()
            .any(|coordinate| !matches!(coordinate, RuntimeHistoryCoordinate::Field { .. }))
        {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }

        let mut segment_source = request.formation_revision;
        for epoch in retained_epochs {
            let program = &epoch.program;
            if epoch.source_context != context {
                return Err(DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                ));
            }

            let transport = program
                .verify(&context, &self.registry)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;
            let updates = realized
                .fields
                .iter()
                .map(|patch| (patch.field, patch.owner, patch.value.clone()))
                .collect::<Vec<_>>();
            let (target_updates, transport_dependencies) = transport
                .transport_field_updates_from_root_exact(&epoch.source_fields, &updates)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;

            let mut source_dependencies = guard_coordinates.clone();
            source_dependencies.extend(transport_dependencies.into_iter().map(|(field, owner)| {
                RuntimeHistoryCoordinate::Field { field, owner }
            }));
            certify_retained_epoch_field_segment(
                segment_source,
                &epoch.index,
                &realized,
                &source_dependencies,
                epoch.source_revision,
            )?;

            let source_guard_fields = guard_coordinates
                .iter()
                .map(|coordinate| match coordinate {
                    RuntimeHistoryCoordinate::Field { field, owner } => Ok((*field, *owner)),
                    _ => Err(DurableRuntimeCommitError::Runtime(
                        PhysicalExecutionError::InvalidRevisionTransition,
                    )),
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            let target_guard_fields = transport
                .transport_field_coordinate_dependencies_exact(&source_guard_fields)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;
            guard_coordinates = target_guard_fields
                .into_iter()
                .map(|(field, owner)| RuntimeHistoryCoordinate::Field { field, owner })
                .collect();
            realized = DurableModelDelta {
                fields: target_updates
                    .into_iter()
                    .map(|(field, owner, value)| kernel_durability::DurableFieldPatch {
                        field,
                        owner,
                        value,
                    })
                    .collect(),
                ..DurableModelDelta::default()
            };
            context = program.target().clone();
            segment_source = epoch.target_revision;
        }

        // Current-epoch proof uses the structural P462 index rather than rescanning
        // the durable suffix.
        match self
            .certify_transition_rebase(segment_source, &[], Some(&realized), None)
            .map_err(|error| match error {
                RuntimeHistoricalSnapshotError::Durability(error) => {
                    DurableRuntimeCommitError::PrepareDurability(error)
                }
                RuntimeHistoricalSnapshotError::Runtime(error) => {
                    DurableRuntimeCommitError::Runtime(error)
                }
                RuntimeHistoricalSnapshotError::Unavailable { revision } => {
                    DurableRuntimeCommitError::SchemaAwareTransitionUnavailable { revision }
                }
                _ => DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                ),
            })?
        {
            RuntimeTransitionRebaseOutcome::Certified(_) => {}
            RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict));
            }
        }

        let snapshot = self.snapshot()?;
        if snapshot.revision().semantic_context() != &context {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        let current = snapshot.revision().clone();
        drop(snapshot);
        let mut target_state = current.state().clone();
        realized.apply_to(&mut target_state);
        let target_revision = current
            .id()
            .raw()
            .checked_add(1)
            .map(RevisionId::new)
            .ok_or(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ))?;
        let target = kernel_revision::Revision::build(
            target_revision,
            &context,
            &self.registry,
            target_state,
        )
        .map_err(|_| DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::InvalidRevisionTransition,
        ))?;
        let complement = DurableModelDelta::between(target.state(), current.state());
        let guard = RuntimeGuardObservationFootprint::new(segment_source, guard_coordinates);
        self.commit_mixed_revision_residual_guarded_with_client_semantics_and_dependencies(
            transaction_id,
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &target,
                mutations: &[],
                model_delta: &realized,
                model_complement: &complement,
                registry: &self.registry,
            },
            &[],
            request.client_model_delta,
            request.formation_semantic_revision,
            request.client_guard_digest,
            guard.as_ref(),
        )
    }
}

fn model_delta_is_field_only(delta: &DurableModelDelta) -> bool {
    delta.carriers.is_empty()
        && delta.lifecycle_entities_inserted.is_empty()
        && delta.lifecycle_entities_removed.is_empty()
        && delta.lifecycle_roots_inserted.is_empty()
        && delta.lifecycle_roots_removed.is_empty()
        && delta.lifecycle_keeps_alive.is_empty()
}

fn certify_retained_epoch_field_segment(
    source_revision: RevisionId,
    epoch: &RuntimeRetainedEpochIndex,
    proposed: &DurableModelDelta,
    dependencies: &BTreeSet<RuntimeHistoryCoordinate>,
    current_revision: RevisionId,
) -> Result<(), DurableRuntimeCommitError> {
    if epoch.lineage_floor.is_some_and(|floor| source_revision < floor) {
        return Err(DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
            revision: source_revision,
        });
    }
    let proposed_coordinates = proposed
        .fields
        .iter()
        .map(|patch| RuntimeHistoryCoordinate::Field {
            field: patch.field,
            owner: patch.owner,
        })
        .collect::<BTreeSet<_>>();

    for coordinate in &proposed_coordinates {
        if let Some(indexed) = epoch.first_action_after(source_revision, coordinate) {
            return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision,
                    conflicting_effects: vec![indexed.effect_id],
                    coordination_effects: Vec::new(),
                    coordinates: vec![coordinate.clone()],
                    opaque_effects: Vec::new(),
                },
            ));
        }
    }
    for coordinate in dependencies {
        if let Some(indexed) = epoch.first_action_after(source_revision, coordinate) {
            return Err(DurableRuntimeCommitError::GuardDependencyConflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision,
                    conflicting_effects: vec![indexed.effect_id],
                    coordination_effects: Vec::new(),
                    coordinates: vec![coordinate.clone()],
                    opaque_effects: Vec::new(),
                },
            ));
        }
    }
    Ok(())
}
