impl DurableRuntime {
    /// Test-only one-shot adapter over the prepared field publication path.
    #[cfg(test)]
    pub(crate) fn commit_schema_aware_field_intent(
        &self,
        transaction_id: ClientTransactionId,
        request: &SchemaAwareFieldTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        if let Some(target_revision) = self.check_mixed_client_retry(
            transaction_id,
            request.formation_semantic_revision,
            &[],
            request.client_model_delta,
            request.client_guard_digest,
            request.formation_revision,
        )? {
            return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision });
        }
        let prepared = self.prepare_schema_aware_field_publication(request, &BTreeSet::new())?;
        self.commit_prepared_schema_aware_publication(transaction_id, &prepared)
    }

    /// Prepares one exact field-only client intent for the exact current head. Field-value
    /// transport, causal certification and publication-authority transport share the same
    /// retained-epoch traversal. Carrier/lifecycle effects remain fail-closed.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn prepare_schema_aware_field_publication(
        &self,
        request: &SchemaAwareFieldTransitionRequest<'_>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        if !model_delta_is_field_only(request.client_model_delta) {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        if request.guard_observation.is_none() {
            return self.prepare_schema_aware_publication(
                request.formation_revision,
                request.formation_semantic_revision,
                &[],
                request.client_model_delta,
                &BTreeSet::new(),
                source_field_writes,
                request.client_guard_digest,
            );
        }
        if request
            .guard_observation
            .is_some_and(|guard| guard.source_revision() != request.formation_revision)
        {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }

        let head_snapshot = self.snapshot()?;
        let head_revision = head_snapshot.revision().id();
        if request.formation_revision > head_revision {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: request.formation_revision,
                },
            );
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
            .first().map_or_else(|| head_snapshot.revision().semantic_context().clone(), |epoch| epoch.source_context.clone());
        if context.revision() != request.formation_semantic_revision {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        drop(head_snapshot);

        let mut realized = request.client_model_delta.clone();
        let mut field_writes = source_field_writes.clone();
        let guard_coordinates = request
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
        let mut formation_seal: Option<SchemaEpochFormationSeal> = None;
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
            let (target_updates, _boundary_inputs) = transport
                .transport_field_updates_from_root_exact(&epoch.source_fields, &updates)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;

            let source_dependencies = if formation_seal.is_none() {
                guard_coordinates.clone()
            } else {
                BTreeSet::new()
            };
            certify_retained_epoch_field_segment(
                segment_source,
                &epoch.index,
                &realized,
                &source_dependencies,
                formation_seal.is_some(),
                epoch.source_revision,
            )?;
            if formation_seal.is_none() {
                formation_seal = Some(SchemaEpochFormationSeal {
                    formation_revision: request.formation_revision,
                    boundary_revision: epoch.source_revision,
                    semantic_revision: request.formation_semantic_revision,
                });
            }

            let mut fields_by_relation = BTreeMap::<SemanticId, BTreeSet<SemanticId>>::new();
            for (relation, field) in field_writes {
                fields_by_relation
                    .entry(relation)
                    .or_default()
                    .insert(field);
            }
            let mut next_field_writes = BTreeSet::new();
            for (relation, fields) in fields_by_relation {
                for target in transport
                    .transport_relation_write_footprint_exact(relation, &fields)
                    .map_err(DurableRuntimeCommitError::MigrationTransport)?
                {
                    next_field_writes.extend(
                        target
                            .target_columns
                            .into_iter()
                            .map(|field| (target.target_relation, field)),
                    );
                }
            }
            field_writes = next_field_writes;
            realized = DurableModelDelta {
                fields: target_updates
                    .into_iter()
                    .map(
                        |(field, owner, value)| kernel_durability::DurableFieldPatch {
                            field,
                            owner,
                            value,
                        },
                    )
                    .collect(),
                ..DurableModelDelta::default()
            };
            context = program.target().clone();
            segment_source = epoch.target_revision;
        }

        if formation_seal.is_some() {
            let snapshot = self.snapshot()?;
            let writes = realized.fields.iter().map(|patch| {
                (
                    RuntimeHistoryCoordinate::Field {
                        field: patch.field,
                        owner: patch.owner,
                    },
                    patch.value.clone(),
                )
            });
            match snapshot
                .certify_retroactive_exact_writes_against_causal_observations(
                    segment_source,
                    writes,
                )
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
                })? {
                RuntimeTransitionRebaseOutcome::Certified(_) => {}
                RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                    return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                        conflict,
                    ));
                }
            }
        }

        let rebase_certificate = match self
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
            })? {
            RuntimeTransitionRebaseOutcome::Certified(certificate) => certificate,
            RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                    conflict,
                ));
            }
        };

        let final_snapshot = self.snapshot()?;
        if final_snapshot.revision().id() != head_revision
            || final_snapshot.revision().semantic_context() != &context
        {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: head_revision,
                },
            );
        }
        let publication_guard = formation_seal
            .is_none()
            .then(|| {
                RuntimeGuardObservationFootprint::new(segment_source, guard_coordinates.clone())
            })
            .flatten();

        Ok(PreparedSchemaAwarePublication {
            formation_revision: request.formation_revision,
            formation_semantic_revision: request.formation_semantic_revision,
            authorized_head_revision: head_revision,
            intervening_effect_count: rebase_certificate.intervening_effect_count,
            relation_writes: BTreeSet::new(),
            field_writes,
            relation_authorizations: BTreeMap::new(),
            model_authority: model_authority_footprint(
                final_snapshot.revision().state(),
                &realized,
            ),
            client_guard_digest: request.client_guard_digest,
            source_model_delta: request.client_model_delta.clone(),
            current_model_delta: realized,
            publication_guard,
            source_mutations: Vec::new(),
            current_mutations: Vec::new(),
        })
    }
}

fn certify_formation_guard_at_boundary(
    source_revision: RevisionId,
    epoch: &RuntimeRetainedEpochIndex,
    boundary_fields: &kernel_model::CowMap<(SemanticId, kernel_types::EntityId), Value>,
    proposed: &DurableModelDelta,
    observation: &RuntimeGuardObservationFootprint,
    boundary_revision: RevisionId,
) -> Result<(), DurableRuntimeCommitError> {
    let proposed_fields = proposed
        .fields
        .iter()
        .map(|patch| ((patch.owner, patch.field), patch.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let candidate_value = |owner: kernel_types::EntityId, field: SemanticId| {
        proposed_fields
            .get(&(owner, field))
            .cloned()
            .unwrap_or_else(|| boundary_fields.get(&(field, owner)).cloned())
    };
    let mut blocked = BTreeSet::new();
    let mut effects = BTreeSet::new();

    for coordinate in observation.coordinates() {
        let field_coordinate = match coordinate {
            RuntimeHistoryCoordinate::Field { field, owner }
            | RuntimeHistoryCoordinate::ObjectField { field, owner, .. } => {
                Some((*owner, *field))
            }
            _ => None,
        };
        let exact = observation.exact_value(coordinate);
        let rule = observation.preservation_rule(coordinate);
        if let Some((owner, field)) = field_coordinate
            && (exact.is_some() || rule.is_some())
        {
            let candidate = candidate_value(owner, field);
            let exact_preserved = exact.is_some_and(|expected| candidate.as_ref() == Some(expected));
            let predicate_preserved = rule.is_some_and(|predicate| {
                candidate.as_ref().is_some_and(|value| {
                    kernel_validation::semantic_rule_matches(predicate, value).unwrap_or(false)
                })
            });
            if exact_preserved || predicate_preserved {
                continue;
            }
            blocked.insert(coordinate.clone());
            effects.extend(
                epoch
                    .actions_after(source_revision, coordinate)
                    .into_iter()
                    .map(|action| action.effect_id),
            );
            continue;
        }
        if let Some(indexed) = epoch.first_action_after(source_revision, coordinate) {
            blocked.insert(coordinate.clone());
            effects.insert(indexed.effect_id);
        }
    }

    for group in observation.joint_groups() {
        let mut candidate = BTreeMap::new();
        for field in group.observed_fields.keys() {
            if let Some(value) = candidate_value(group.owner, *field) {
                candidate.insert(*field, value);
            }
        }
        if !kernel_validation::semantic_rule_matches_fields(&group.predicate, &candidate)
            .unwrap_or(false)
        {
            for field in group.observed_fields.keys() {
                let coordinate = RuntimeHistoryCoordinate::Field {
                    field: *field,
                    owner: group.owner,
                };
                blocked.insert(coordinate.clone());
                effects.extend(
                    epoch
                        .actions_after(source_revision, &coordinate)
                        .into_iter()
                        .map(|action| action.effect_id),
                );
            }
        }
    }

    if blocked.is_empty() {
        return Ok(());
    }
    Err(DurableRuntimeCommitError::GuardDependencyConflict(
        RuntimeTransitionRebaseConflict {
            source_revision,
            current_revision: boundary_revision,
            conflicting_effects: effects.into_iter().collect(),
            coordination_effects: Vec::new(),
            coordinates: blocked.into_iter().collect(),
            opaque_effects: Vec::new(),
        },
    ))
}

fn model_authority_footprint(
    source: &kernel_model::DatabaseState,
    delta: &DurableModelDelta,
) -> RuntimeModelAuthorityFootprint {
    let mut footprint = RuntimeModelAuthorityFootprint::default();
    for patch in &delta.carriers {
        if source.model.carriers.contains_key(&patch.carrier) != patch.target_present {
            footprint.carrier_presence.insert(patch.carrier);
        }
        footprint
            .carrier_members
            .extend(patch.inserted.iter().chain(&patch.removed).map(|entity| (patch.carrier, *entity)));
    }
    footprint
        .lifecycle_entities
        .extend(delta.lifecycle_entities_inserted.iter().chain(&delta.lifecycle_entities_removed).copied());
    footprint
        .lifecycle_roots
        .extend(delta.lifecycle_roots_inserted.iter().chain(&delta.lifecycle_roots_removed).copied());
    for patch in &delta.lifecycle_keeps_alive {
        if source.lifecycle.keeps_alive.contains_key(&patch.parent) != patch.target_present {
            footprint.keeps_alive_presence.insert(patch.parent);
        }
        footprint.keeps_alive_edges.extend(
            patch.inserted.iter().chain(&patch.removed).map(|child| (patch.parent, *child)),
        );
    }
    footprint
}

fn model_delta_is_field_only(delta: &DurableModelDelta) -> bool {
    delta.carriers.is_empty()
        && delta.lifecycle_entities_inserted.is_empty()
        && delta.lifecycle_entities_removed.is_empty()
        && delta.lifecycle_roots_inserted.is_empty()
        && delta.lifecycle_roots_removed.is_empty()
        && delta.lifecycle_keeps_alive.is_empty()
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn certify_retained_epoch_field_segment(
    source_revision: RevisionId,
    epoch: &RuntimeRetainedEpochIndex,
    proposed: &DurableModelDelta,
    dependencies: &BTreeSet<RuntimeHistoryCoordinate>,
    enforce_causal_observations: bool,
    current_revision: RevisionId,
) -> Result<(), DurableRuntimeCommitError> {
    if epoch
        .lineage_floor
        .is_some_and(|floor| source_revision < floor)
    {
        return Err(
            DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: source_revision,
            },
        );
    }
    let proposed_coordinates = proposed
        .fields
        .iter()
        .map(|patch| RuntimeHistoryCoordinate::Field {
            field: patch.field,
            owner: patch.owner,
        })
        .collect::<BTreeSet<_>>();

    if enforce_causal_observations {
        let mut effects = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        let proposed_fields = proposed
            .fields
            .iter()
            .map(|patch| ((patch.owner, patch.field), patch.value.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut joint_groups = BTreeMap::<(u128, u32), RuntimeJointCausalObservationGroup>::new();
        for patch in &proposed.fields {
            let coordinate = RuntimeHistoryCoordinate::Field {
                field: patch.field,
                owner: patch.owner,
            };
            for observation in epoch.observations_after(source_revision, &coordinate) {
                let exact_preserved = patch.value.as_ref().is_some_and(|proposed| {
                    observation
                        .exact_value
                        .as_ref()
                        .is_some_and(|observed| observed == proposed)
                });
                let predicate_preserved = patch.value.as_ref().is_some_and(|proposed| {
                    observation.preservation_rule.as_ref().is_some_and(|rule| {
                        kernel_validation::semantic_rule_matches(rule, proposed).unwrap_or(false)
                    })
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
                        epoch.joint_observation_group(observation.effect_id, group_id)
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
            return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                RuntimeTransitionRebaseConflict {
                    source_revision,
                    current_revision,
                    conflicting_effects: Vec::new(),
                    coordination_effects: effects.into_iter().collect(),
                    coordinates: blocked.into_iter().collect(),
                    opaque_effects: Vec::new(),
                },
            ));
        }
    }

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

fn non_field_model_delta(delta: &DurableModelDelta) -> DurableModelDelta {
    DurableModelDelta {
        carriers: delta.carriers.clone(),
        lifecycle_entities_inserted: delta.lifecycle_entities_inserted.clone(),
        lifecycle_entities_removed: delta.lifecycle_entities_removed.clone(),
        lifecycle_roots_inserted: delta.lifecycle_roots_inserted.clone(),
        lifecycle_roots_removed: delta.lifecycle_roots_removed.clone(),
        lifecycle_keeps_alive: delta.lifecycle_keeps_alive.clone(),
        ..DurableModelDelta::default()
    }
}

fn certify_retained_epoch_model_coordinate_segment(
    source_revision: RevisionId,
    epoch: &RuntimeRetainedEpochIndex,
    semantic_context: &kernel_schema::SemanticContext,
    proposed: &DurableModelDelta,
    current_revision: RevisionId,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), DurableRuntimeCommitError> {
    let proposed = non_field_model_delta(proposed);
    if proposed == DurableModelDelta::default() {
        return Ok(());
    }
    let footprint = proposed_transition_footprint(
        source_revision,
        semantic_context,
        &[],
        Some(&proposed),
        None,
        registry,
    )
    .map_err(|error| match error {
        RuntimeHistoricalSnapshotError::Durability(error) => {
            DurableRuntimeCommitError::PrepareDurability(error)
        }
        RuntimeHistoricalSnapshotError::Runtime(error) => DurableRuntimeCommitError::Runtime(error),
        RuntimeHistoricalSnapshotError::Unavailable { revision } => {
            DurableRuntimeCommitError::SchemaAwareTransitionUnavailable { revision }
        }
        _ => DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::InvalidRevisionTransition,
        ),
    })?;
    let mut conflicting_effects = BTreeSet::new();
    let mut coordination_effects = BTreeSet::new();
    let mut coordinates = BTreeSet::new();
    for (coordinate, proposed_action) in &footprint.writes {
        for observation in epoch.observations_after(source_revision, coordinate) {
            coordination_effects.insert(observation.effect_id);
            coordinates.insert(coordinate.clone());
        }
        for indexed in epoch.actions_after(source_revision, coordinate) {
            let left = BTreeMap::from([((), proposed_action.clone())]);
            let right = BTreeMap::from([((), indexed.action.clone())]);
            match kernel_change::infer_write_action_law(&left, &right) {
                kernel_change::PairRewriteLaw::StrongCommute
                | kernel_change::PairRewriteLaw::SameIdempotentIntent => {}
                kernel_change::PairRewriteLaw::Unknown => {
                    coordination_effects.insert(indexed.effect_id);
                    coordinates.insert(coordinate.clone());
                }
                kernel_change::PairRewriteLaw::DefiniteIntentConflict => {
                    conflicting_effects.insert(indexed.effect_id);
                    coordinates.insert(coordinate.clone());
                }
            }
        }
    }
    if !conflicting_effects.is_empty() || !coordination_effects.is_empty() {
        return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
            RuntimeTransitionRebaseConflict {
                source_revision,
                current_revision,
                conflicting_effects: conflicting_effects.into_iter().collect(),
                coordination_effects: coordination_effects.into_iter().collect(),
                coordinates: coordinates.into_iter().collect(),
                opaque_effects: Vec::new(),
            },
        ));
    }
    Ok(())
}

impl DurableRuntime {
    /// Compiles retained migration lineage into one current-world schema bridge.
    ///
    /// The bridge is certificate-only: it never reconstructs the historical source
    /// database state. `None` means retained lineage cannot prove a path from the
    /// requested source schema revision to the authoritative head.
    pub fn current_schema_bridge(
        &self,
        source_schema_revision: kernel_types::SchemaRevisionId,
    ) -> Result<Option<CurrentSchemaBridge>, DurableRuntimeCommitError> {
        let head = self.snapshot()?;
        let target_context = head.revision().semantic_context().clone();
        if target_context.schema.revision == source_schema_revision {
            return Ok(Some(CurrentSchemaBridge::new(
                target_context.clone(),
                target_context,
                Vec::new(),
            )));
        }

        let epochs = head
            .root()
            .historical
            .retained_schema_epochs
            .iter()
            .map(|(_, epoch)| epoch.clone())
            .collect::<Vec<_>>();
        drop(head);

        let Some(start) = epochs
            .iter()
            .position(|epoch| epoch.source_context.schema.revision == source_schema_revision)
        else {
            return Ok(None);
        };
        let source_context = epochs[start].source_context.clone();
        let mut context = source_context.clone();
        let mut steps = Vec::new();
        for epoch in epochs.into_iter().skip(start) {
            if epoch.source_context != context {
                return Ok(None);
            }
            let bridge = kernel_transport::SchemaBridge::verify(
                &epoch.program,
                &context,
                &self.registry,
            )
            .map_err(DurableRuntimeCommitError::MigrationTransport)?;
            context = bridge.target().clone();
            steps.push(bridge);
            if context == target_context {
                return Ok(Some(CurrentSchemaBridge::new(
                    source_context,
                    target_context,
                    steps,
                )));
            }
        }
        Ok(None)
    }

    /// Resolves the semantic formation world for a stale intent from retained epoch authority
    /// without reconstructing/materializing the historical database revision.
    pub fn schema_aware_formation_context_witness(
        &self,
        formation_revision: RevisionId,
        formation_semantic_revision: kernel_types::SemanticRevision,
    ) -> Result<SchemaAwareFormationContextWitness, DurableRuntimeCommitError> {
        let head_snapshot = self.snapshot()?;
        let head_revision = head_snapshot.revision().id();
        if formation_revision > head_revision {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: formation_revision,
                },
            );
        }
        let retained_epochs = head_snapshot
            .root()
            .historical
            .retained_schema_epochs
            .iter()
            .filter_map(|(_, epoch)| {
                (epoch.source_revision >= formation_revision
                    && epoch.target_revision <= head_revision)
                    .then_some(epoch.clone())
            })
            .collect::<Vec<_>>();
        let context = retained_epochs.first().map_or_else(
            || head_snapshot.revision().semantic_context().clone(),
            |epoch| epoch.source_context.clone(),
        );
        if context.revision() != formation_semantic_revision {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        Ok(SchemaAwareFormationContextWitness {
            formation_revision,
            authorized_head_revision: head_revision,
            semantic_context: context,
            retained_epochs,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep exact Γ-DTC formation-capsule lineage certification together."
    )]
    fn certify_formation_relational_observations_at_boundary(
        &self,
        observations: &[RuntimeRelationalCausalObservation],
        boundary_revision: RevisionId,
        context: &kernel_schema::SemanticContext,
        index: &RuntimeRetainedEpochIndex,
    ) -> Result<(), DurableRuntimeCommitError> {
        if observations.is_empty() {
            return Ok(());
        }
        let observed_revision = observations[0].observed_revision;
        if observations
            .iter()
            .any(|observation| observation.observed_revision != observed_revision)
        {
            return Err(DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: observed_revision,
            });
        }
        let mut capsules = observations
            .iter()
            .map(|observation| observation.capsule.clone())
            .collect::<Vec<_>>();
        let source_sets = capsules
            .iter()
            .map(RelCausalCapsule::source_relations)
            .collect::<Vec<_>>();
        let all_sources = source_sets
            .iter()
            .flat_map(|relations| relations.iter().copied())
            .collect::<BTreeSet<_>>();
        let deltas_by_revision = index
            .relation_deltas_between(observed_revision, boundary_revision, &all_sources)
            .ok_or(DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: observed_revision,
            })?;

        for (target_revision, deltas) in deltas_by_revision {
            let effect_id = index
                .exact_effect_at(target_revision)
                .ok_or(DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: target_revision,
                })?;
            for (observation_index, capsule) in capsules.iter_mut().enumerate() {
                if !source_sets[observation_index]
                    .iter()
                    .any(|relation| deltas.contains_key(relation))
                {
                    continue;
                }
                match capsule
                    .impact_relation_deltas(&deltas, context, &self.registry)
                    .map_err(PhysicalExecutionError::from)?
                {
                    Impact::Unaffected => {}
                    Impact::Changed | Impact::Unknown => {
                        return Err(DurableRuntimeCommitError::GuardDependencyConflict(
                            RuntimeTransitionRebaseConflict {
                                source_revision: observations[observation_index].observed_revision,
                                current_revision: boundary_revision,
                                conflicting_effects: Vec::new(),
                                coordination_effects: vec![effect_id],
                                coordinates: Vec::new(),
                                opaque_effects: Vec::new(),
                            },
                        ));
                    }
                }
                *capsule = capsule
                    .advance(target_revision, &deltas, context, &self.registry)
                    .map_err(PhysicalExecutionError::from)?;
            }
        }
        Ok(())
    }

    /// Prepares one exact relation-data intent for the exact current head. Data transport,
    /// conflict certification and required publication authority are derived in one retained-epoch
    /// traversal. Grants are never transported; only the effect's required authority footprint is.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep formation identity, mixed effect and authority footprint explicit at the kernel boundary."
    )]
    pub fn prepare_schema_aware_publication(
        &self,
        formation_revision: RevisionId,
        formation_semantic_revision: kernel_types::SemanticRevision,
        mutations: &[RevisionRelationMutation<'_>],
        source_model_delta: &DurableModelDelta,
        source_relation_writes: &BTreeSet<SemanticId>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        self.prepare_schema_aware_publication_with_formation_proof(
            formation_revision,
            formation_semantic_revision,
            mutations,
            source_model_delta,
            source_relation_writes,
            source_field_writes,
            client_guard_digest,
            None,
            &[],
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep formation identity, exact effect and formation proof inputs explicit at the kernel boundary."
    )]
    pub fn prepare_schema_aware_publication_with_formation_proof(
        &self,
        formation_revision: RevisionId,
        formation_semantic_revision: kernel_types::SemanticRevision,
        mutations: &[RevisionRelationMutation<'_>],
        source_model_delta: &DurableModelDelta,
        source_relation_writes: &BTreeSet<SemanticId>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        let witness = self.schema_aware_formation_context_witness(
            formation_revision,
            formation_semantic_revision,
        )?;
        self.prepare_schema_aware_publication_with_witness_and_formation_proof(
            &witness,
            mutations,
            source_model_delta,
            source_relation_writes,
            source_field_writes,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep formation witness, mixed effect and authority footprint explicit at the kernel boundary."
    )]
    pub fn prepare_schema_aware_publication_with_witness(
        &self,
        witness: &SchemaAwareFormationContextWitness,
        mutations: &[RevisionRelationMutation<'_>],
        source_model_delta: &DurableModelDelta,
        source_relation_writes: &BTreeSet<SemanticId>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        self.prepare_schema_aware_publication_with_witness_and_formation_proof(
            witness,
            mutations,
            source_model_delta,
            source_relation_writes,
            source_field_writes,
            client_guard_digest,
            None,
            &[],
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep formation witness, mixed effect, authority footprint and proof material explicit at the kernel boundary."
    )]
    pub fn prepare_schema_aware_publication_with_witness_and_formation_proof(
        &self,
        witness: &SchemaAwareFormationContextWitness,
        mutations: &[RevisionRelationMutation<'_>],
        source_model_delta: &DurableModelDelta,
        source_relation_writes: &BTreeSet<SemanticId>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        let formation_revision = witness.formation_revision;
        let formation_semantic_revision = witness.semantic_context.revision();
        if guard_observation
            .is_some_and(|observation| observation.source_revision() != formation_revision)
            || relational_observations.iter().any(|observation| {
                observation.observed_revision != formation_revision
                    || observation.capsule.revision() != formation_revision
            })
        {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        if (mutations.is_empty() && source_model_delta == &DurableModelDelta::default())
            || mutations
                .iter()
                .any(|mutation| !mutation.object_field_writes.is_empty())
        {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }

        let head_snapshot = self.snapshot()?;
        let head_revision = head_snapshot.revision().id();
        if formation_revision > head_revision || witness.authorized_head_revision != head_revision {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: formation_revision,
                },
            );
        }
        let retained_epochs = witness.retained_epochs.clone();
        let mut context = witness.semantic_context.clone();
        if context.revision() != formation_semantic_revision {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        drop(head_snapshot);

        let source_mutations = mutations
            .iter()
            .map(|mutation| {
                (
                    mutation.relation,
                    mutation.delta.clone(),
                    mutation.authorization,
                )
            })
            .collect::<Vec<_>>();
        let mut realized = source_mutations.clone();
        let mut realized_model_delta = source_model_delta.clone();
        let mut relation_writes = source_relation_writes.clone();
        let mut field_writes = source_field_writes.clone();
        let mut relation_authorizations = mutations
            .iter()
            .filter(|mutation| mutation.authorization != kernel_durability::DurableRelationAuthorization::default())
            .map(|mutation| (mutation.relation, mutation.authorization))
            .collect::<BTreeMap<_, _>>();
        let mut segment_source = formation_revision;
        let mut crossed_schema_boundary = false;

        for epoch in retained_epochs {
            if epoch.source_context != context {
                return Err(DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                ));
            }
            if !crossed_schema_boundary {
                if let Some(observation) = guard_observation {
                    certify_formation_guard_at_boundary(
                        formation_revision,
                        &epoch.index,
                        &epoch.source_fields,
                        &realized_model_delta,
                        observation,
                        epoch.source_revision,
                    )?;
                }
                self.certify_formation_relational_observations_at_boundary(
                    relational_observations,
                    epoch.source_revision,
                    &context,
                    &epoch.index,
                )?;
            }
            let refs = realized
                .iter()
                .map(
                    |(relation, delta, authorization)| RevisionRelationMutation {
                        relation: *relation,
                        delta,
                        object_field_writes: &[],
                        authorization: *authorization,
                    },
                )
                .collect::<Vec<_>>();
            certify_retained_epoch_relation_segment(
                segment_source,
                &epoch.index,
                &context,
                &refs,
                epoch.source_revision,
                &self.registry,
            )?;
            if realized_model_delta != DurableModelDelta::default() {
                certify_retained_epoch_field_segment(
                    segment_source,
                    &epoch.index,
                    &realized_model_delta,
                    &BTreeSet::new(),
                    crossed_schema_boundary,
                    epoch.source_revision,
                )?;
                certify_retained_epoch_model_coordinate_segment(
                    segment_source,
                    &epoch.index,
                    &context,
                    &realized_model_delta,
                    epoch.source_revision,
                    &self.registry,
                )?;
            }

            let transport = epoch
                .program
                .verify(&context, &self.registry)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;

            if realized_model_delta != DurableModelDelta::default() {
                let non_fields = non_field_model_delta(&realized_model_delta);
                let updates = realized_model_delta
                    .fields
                    .iter()
                    .map(|patch| (patch.field, patch.owner, patch.value.clone()))
                    .collect::<Vec<_>>();
                let (target_updates, _boundary_inputs) = transport
                    .transport_field_updates_from_root_exact(&epoch.source_fields, &updates)
                    .map_err(DurableRuntimeCommitError::MigrationTransport)?;
                realized_model_delta = DurableModelDelta {
                    fields: target_updates
                        .into_iter()
                        .map(
                            |(field, owner, value)| kernel_durability::DurableFieldPatch {
                                field,
                                owner,
                                value,
                            },
                        )
                        .collect(),
                    ..non_fields
                };
            }

            let mut next_realized = Vec::<(
                SemanticId,
                RelationDelta,
                kernel_durability::DurableRelationAuthorization,
            )>::new();
            let mut targets = BTreeSet::new();
            for (relation, delta, authorization) in &realized {
                let action_preserved = !(authorization.object_create
                    || authorization.object_delete
                    || authorization.relationship_attach
                    || authorization.relationship_detach
                    || authorization.relationship_move)
                    || matches!(
                        transport.relation_slice(*relation),
                        Some(kernel_transport::MigrationRelationSlice::Passthrough { relation: target })
                            if target == *relation
                    );
                if !action_preserved {
                    return Err(DurableRuntimeCommitError::MigrationTransport(
                        kernel_transport::TransportError::UnrepresentableSourceRelationEffect(
                            *relation,
                        ),
                    ));
                }
                for (target_relation, target_delta) in transport
                    .transport_relation_delta_exact(*relation, delta, &self.registry)
                    .map_err(DurableRuntimeCommitError::MigrationTransport)?
                {
                    if !targets.insert(target_relation) {
                        return Err(DurableRuntimeCommitError::Runtime(
                            PhysicalExecutionError::InvalidRevisionTransition,
                        ));
                    }
                    let preserves_actions = target_relation == *relation && action_preserved;
                    let target_authorization = kernel_durability::DurableRelationAuthorization {
                        relation_write: authorization.relation_write,
                        object_create: preserves_actions && authorization.object_create,
                        object_delete: preserves_actions && authorization.object_delete,
                        relationship_attach: preserves_actions && authorization.relationship_attach,
                        relationship_detach: preserves_actions && authorization.relationship_detach,
                        relationship_move: preserves_actions && authorization.relationship_move,
                    };
                    next_realized.push((target_relation, target_delta, target_authorization));
                }
            }

            let mut next_relation_writes = BTreeSet::new();
            for relation in relation_writes {
                next_relation_writes.extend(
                    transport
                        .transport_relation_write_targets_exact(relation)
                        .map_err(DurableRuntimeCommitError::MigrationTransport)?,
                );
            }
            let mut fields_by_relation = BTreeMap::<SemanticId, BTreeSet<SemanticId>>::new();
            for (relation, field) in field_writes {
                fields_by_relation
                    .entry(relation)
                    .or_default()
                    .insert(field);
            }
            let mut next_field_writes = BTreeSet::new();
            for (relation, fields) in fields_by_relation {
                for target in transport
                    .transport_relation_write_footprint_exact(relation, &fields)
                    .map_err(DurableRuntimeCommitError::MigrationTransport)?
                {
                    next_field_writes.extend(
                        target
                            .target_columns
                            .into_iter()
                            .map(|field| (target.target_relation, field)),
                    );
                }
            }
            let mut next_authorizations =
                BTreeMap::<SemanticId, kernel_durability::DurableRelationAuthorization>::new();
            for (relation, authorization) in relation_authorizations {
                for target_relation in transport
                    .transport_relation_write_targets_exact(relation)
                    .map_err(DurableRuntimeCommitError::MigrationTransport)?
                {
                    let target = next_authorizations.entry(target_relation).or_default();
                    target.relation_write |= authorization.relation_write;
                }
                let has_action = authorization.object_create
                    || authorization.object_delete
                    || authorization.relationship_attach
                    || authorization.relationship_detach
                    || authorization.relationship_move;
                if has_action {
                    match transport.relation_slice(relation) {
                        Some(kernel_transport::MigrationRelationSlice::Passthrough {
                            relation: target,
                        }) if target == relation => {
                            let target = next_authorizations.entry(target).or_default();
                            target.object_create |= authorization.object_create;
                            target.object_delete |= authorization.object_delete;
                            target.relationship_attach |= authorization.relationship_attach;
                            target.relationship_detach |= authorization.relationship_detach;
                            target.relationship_move |= authorization.relationship_move;
                        }
                        _ => {
                            return Err(DurableRuntimeCommitError::MigrationTransport(
                                kernel_transport::TransportError::UnrepresentableSourceRelationEffect(
                                    relation,
                                ),
                            ));
                        }
                    }
                }
            }

            realized = next_realized;
            relation_writes = next_relation_writes;
            field_writes = next_field_writes;
            relation_authorizations = next_authorizations;
            context = epoch.program.target().clone();
            segment_source = epoch.target_revision;
            crossed_schema_boundary = true;
        }

        if !crossed_schema_boundary
            && (guard_observation.is_some() || !relational_observations.is_empty())
        {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: formation_revision,
                },
            );
        }

        if crossed_schema_boundary && realized_model_delta != DurableModelDelta::default() {
            let snapshot = self.snapshot()?;
            let footprint = proposed_transition_footprint(
                segment_source,
                &context,
                &[],
                Some(&realized_model_delta),
                None,
                &self.registry,
            )
            .map_err(|_| {
                DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                )
            })?;
            let field_values = realized_model_delta
                .fields
                .iter()
                .map(|patch| ((patch.field, patch.owner), patch.value.clone()))
                .collect::<BTreeMap<_, _>>();
            let writes = footprint.writes.keys().cloned().map(|coordinate| {
                let value = match &coordinate {
                    RuntimeHistoryCoordinate::Field { field, owner } => {
                        field_values.get(&(*field, *owner)).cloned().flatten()
                    }
                    _ => None,
                };
                (coordinate, value)
            });
            match snapshot
                .certify_retroactive_exact_writes_against_causal_observations(
                    segment_source,
                    writes,
                )
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
                    return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                        conflict,
                    ));
                }
            }
        }

        for (relation, delta, _) in &realized {
            self.validate_relation_delta_at(segment_source, *relation, delta)
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
                })?;
        }
        let current_refs = realized
            .iter()
            .map(
                |(relation, delta, authorization)| RevisionRelationMutation {
                    relation: *relation,
                    delta,
                    object_field_writes: &[],
                    authorization: *authorization,
                },
            )
            .collect::<Vec<_>>();
        let rebase_certificate = match self
            .certify_transition_rebase(
                segment_source,
                &current_refs,
                (realized_model_delta != DurableModelDelta::default())
                    .then_some(&realized_model_delta),
                None,
            )
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
            })? {
            RuntimeTransitionRebaseOutcome::Certified(certificate) => certificate,
            RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
                    conflict,
                ));
            }
        };

        let final_snapshot = self.snapshot()?;
        if final_snapshot.revision().id() != head_revision
            || final_snapshot.revision().semantic_context() != &context
        {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: head_revision,
                },
            );
        }

        Ok(PreparedSchemaAwarePublication {
            formation_revision,
            formation_semantic_revision,
            authorized_head_revision: head_revision,
            intervening_effect_count: rebase_certificate.intervening_effect_count,
            relation_writes,
            field_writes,
            relation_authorizations,
            model_authority: model_authority_footprint(
                final_snapshot.revision().state(),
                &realized_model_delta,
            ),
            client_guard_digest,
            source_model_delta: source_model_delta.clone(),
            current_model_delta: realized_model_delta,
            publication_guard: None,
            source_mutations,
            current_mutations: realized,
        })
    }

    /// Publishes a previously prepared schema-aware effect. No migration is re-walked here: the
    /// prepared artifact is valid only while its exact authorized HEAD remains current.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn commit_prepared_schema_aware_publication(
        &self,
        transaction_id: ClientTransactionId,
        prepared: &PreparedSchemaAwarePublication,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let source_refs = prepared
            .source_mutations
            .iter()
            .map(
                |(relation, delta, authorization)| RevisionRelationMutation {
                    relation: *relation,
                    delta,
                    object_field_writes: &[],
                    authorization: *authorization,
                },
            )
            .collect::<Vec<_>>();
        if prepared.current_model_delta != DurableModelDelta::default() {
            if let Some(target_revision) = self.check_mixed_client_retry(
                transaction_id,
                prepared.formation_semantic_revision,
                &source_refs,
                &prepared.source_model_delta,
                prepared.client_guard_digest,
                prepared.formation_revision,
            )? {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision });
            }
            let snapshot = self.snapshot()?;
            if snapshot.revision().id() != prepared.authorized_head_revision {
                return Err(
                    DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                        revision: prepared.authorized_head_revision,
                    },
                );
            }
            let current = snapshot.revision().clone();
            let mut residuals = Vec::new();
            for (relation, delta, authorization) in &prepared.current_mutations {
                let rows = current.state().model.relations.get(relation);
                let witness = snapshot.relation_base_witness(*relation).ok_or(
                    DurableRuntimeCommitError::Runtime(
                        PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                    ),
                )?;
                let residual = witness
                    .residualize_delta_against_support(delta, |position| {
                        rows.and_then(|rows| rows.get(position)).cloned()
                    })
                    .map_err(PhysicalExecutionError::from)?;
                if !residual.is_empty() {
                    residuals.push((*relation, residual, *authorization));
                }
            }
            drop(snapshot);
            let mut target_state = current.state().clone();
            for (relation, delta, _) in &residuals {
                let expr = kernel_query::RelExpr::Scan(*relation);
                let old = expr
                    .evaluate(&target_state.model, current.semantic_context(), &self.registry)
                    .map_err(PhysicalExecutionError::from)?;
                let next = delta
                    .apply_to_value(old, current.semantic_context(), &self.registry)
                    .map_err(PhysicalExecutionError::from)?;
                target_state.model.relations.insert(*relation, next.into_rows());
            }
            prepared.current_model_delta.apply_to(&mut target_state);
            let realized_model_delta = DurableModelDelta::between(current.state(), &target_state);
            if residuals.is_empty() && realized_model_delta == DurableModelDelta::default() {
                let durable_client_mutations =
                    Self::canonical_durable_relation_mutations(&source_refs)?;
                let mut durability = self.durability.lock().map_err(|_| {
                    let _ = self.cell.force_recovery_required();
                    DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
                })?;
                let seal = durability
                    .durably_seal_satisfied_client_intent(
                        transaction_id,
                        DurableClientIntent::MixedRevision {
                            semantic_revision: prepared.formation_semantic_revision,
                            relation_mutations: durable_client_mutations,
                            model_delta: prepared.source_model_delta.clone(),
                            guard_digest: prepared.client_guard_digest,
                        },
                    )
                    .map_err(DurableRuntimeCommitError::CommitDurabilityUncertain)?;
                return Ok(match seal {
                    kernel_durability::DurableSatisfiedIntentSealOutcome::Sealed { revision } => {
                        DurableRuntimeCommitOutcome::AlreadySatisfied {
                            target_revision: revision,
                        }
                    }
                    kernel_durability::DurableSatisfiedIntentSealOutcome::AlreadySealed {
                        revision,
                    } => DurableRuntimeCommitOutcome::AlreadyCommitted {
                        target_revision: revision,
                    },
                });
            }
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
                current.semantic_context(),
                &self.registry,
                target_state,
            )
            .map_err(|_| {
                DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                )
            })?;
            let complement = DurableModelDelta::between(target.state(), current.state());
            let realized_refs = residuals
                .iter()
                .map(
                    |(relation, delta, authorization)| RevisionRelationMutation {
                        relation: *relation,
                        delta,
                        object_field_writes: &[],
                        authorization: *authorization,
                    },
                )
                .collect::<Vec<_>>();
            return self
                .commit_mixed_revision_residual_guarded_with_client_semantics_and_dependencies(
                    transaction_id,
                    &MixedRevisionTransitionRequest {
                        source_revision: current.id(),
                        target_revision: &target,
                        mutations: &realized_refs,
                        model_delta: &realized_model_delta,
                        model_complement: &complement,
                        registry: &self.registry,
                    },
                    &source_refs,
                    &prepared.source_model_delta,
                    prepared.formation_semantic_revision,
                    prepared.client_guard_digest,
                    prepared.publication_guard.as_ref(),
                    &[],
                );
        }
        let durable_client_mutations = Self::canonical_durable_relation_mutations(&source_refs)?;
        let head_snapshot = self.snapshot()?;
        let head_revision = head_snapshot.revision().id();
        let requested_target = head_revision
            .raw()
            .checked_add(1)
            .map(RevisionId::new)
            .ok_or(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ))?;
        {
            let durability = self.durability.lock().map_err(|_| {
                let _ = self.cell.force_recovery_required();
                DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
            })?;
            if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
                let matches = matches!(
                    &committed_intent.intent,
                    DurableClientIntent::RelationData {
                        semantic_revision,
                        relation_mutations,
                        guard_digest,
                        ..
                    } if *semantic_revision == prepared.formation_semantic_revision
                        && relation_mutations == &durable_client_mutations
                        && *guard_digest == prepared.client_guard_digest
                );
                if matches {
                    return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                        target_revision: committed_intent.target_revision(),
                    });
                }
                return Err(DurableRuntimeCommitError::TransactionIdConflict {
                    transaction_id,
                    committed_target: committed_intent.target_revision(),
                    requested_target,
                });
            }
        }
        if head_revision != prepared.authorized_head_revision {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: prepared.authorized_head_revision,
                },
            );
        }
        let current = head_snapshot.revision().clone();
        let mut residuals = Vec::new();
        for (relation, delta, authorization) in &prepared.current_mutations {
            let rows = current.state().model.relations.get(relation);
            let witness = head_snapshot.relation_base_witness(*relation).ok_or(
                DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(*relation),
                ),
            )?;
            let residual = witness
                .residualize_delta_against_support(delta, |position| {
                    rows.and_then(|rows| rows.get(position)).cloned()
                })
                .map_err(PhysicalExecutionError::from)?;
            if !residual.is_empty() {
                residuals.push((*relation, residual, *authorization));
            }
        }
        drop(head_snapshot);
        let realized_refs = residuals
            .iter()
            .map(
                |(relation, delta, authorization)| RevisionRelationMutation {
                    relation: *relation,
                    delta,
                    object_field_writes: &[],
                    authorization: *authorization,
                },
            )
            .collect::<Vec<_>>();
        if realized_refs.is_empty() {
            let mut durability = self.durability.lock().map_err(|_| {
                let _ = self.cell.force_recovery_required();
                DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
            })?;
            let seal = durability
                .durably_seal_satisfied_client_intent(
                    transaction_id,
                    DurableClientIntent::RelationData {
                        semantic_revision: prepared.formation_semantic_revision,
                        relation_mutations: durable_client_mutations,
                        guard_digest: prepared.client_guard_digest,
                    },
                )
                .map_err(DurableRuntimeCommitError::CommitDurabilityUncertain)?;
            return Ok(match seal {
                kernel_durability::DurableSatisfiedIntentSealOutcome::Sealed { revision } => {
                    DurableRuntimeCommitOutcome::AlreadySatisfied {
                        target_revision: revision,
                    }
                }
                kernel_durability::DurableSatisfiedIntentSealOutcome::AlreadySealed {
                    revision,
                } => DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: revision,
                },
            });
        }
        let target_revision = current
            .id()
            .raw()
            .checked_add(1)
            .map(RevisionId::new)
            .ok_or(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ))?;
        self.commit_derived_relation_data_residual_guarded_with_client_semantics(
            transaction_id,
            &DerivedRelationTransitionRequest {
                source_revision: current.id(),
                target_revision,
                mutations: &realized_refs,
            },
            &source_refs,
            prepared.formation_semantic_revision,
            prepared.client_guard_digest,
        )
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn certify_retained_epoch_relation_segment(
    source_revision: RevisionId,
    epoch: &RuntimeRetainedEpochIndex,
    semantic_context: &kernel_schema::SemanticContext,
    proposed: &[RevisionRelationMutation<'_>],
    current_revision: RevisionId,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), DurableRuntimeCommitError> {
    if epoch
        .lineage_floor
        .is_some_and(|floor| source_revision < floor)
    {
        return Err(
            DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: source_revision,
            },
        );
    }
    for mutation in proposed {
        let support = epoch.support_at(source_revision, mutation.relation).ok_or(
            DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                revision: source_revision,
            },
        )?;
        if support.result_type() != &mutation.delta.result_type {
            return Err(DurableRuntimeCommitError::Runtime(
                PhysicalExecutionError::InvalidRevisionTransition,
            ));
        }
        support
            .validate_exact_delta(mutation.delta, registry)
            .map_err(PhysicalExecutionError::from)?;
    }

    let footprint = proposed_transition_footprint(
        source_revision,
        semantic_context,
        proposed,
        None,
        None,
        registry,
    )
    .map_err(|error| match error {
        RuntimeHistoricalSnapshotError::Runtime(error) => DurableRuntimeCommitError::Runtime(error),
        RuntimeHistoricalSnapshotError::Durability(error) => {
            DurableRuntimeCommitError::PrepareDurability(error)
        }
        RuntimeHistoricalSnapshotError::Unavailable { revision } => {
            DurableRuntimeCommitError::SchemaAwareTransitionUnavailable { revision }
        }
        _ => DurableRuntimeCommitError::Runtime(PhysicalExecutionError::InvalidRevisionTransition),
    })?;
    let mut conflicting_effects = BTreeSet::new();
    let mut coordination_effects = BTreeSet::new();
    let mut coordinates = BTreeSet::new();
    for (coordinate, proposed_action) in &footprint.writes {
        for indexed in epoch.actions_after(source_revision, coordinate) {
            let left = BTreeMap::from([((), proposed_action.clone())]);
            let right = BTreeMap::from([((), indexed.action.clone())]);
            match kernel_change::infer_write_action_law(&left, &right) {
                kernel_change::PairRewriteLaw::StrongCommute
                | kernel_change::PairRewriteLaw::SameIdempotentIntent => {}
                kernel_change::PairRewriteLaw::DefiniteIntentConflict => {
                    conflicting_effects.insert(indexed.effect_id);
                    coordinates.insert(coordinate.clone());
                }
                kernel_change::PairRewriteLaw::Unknown => {
                    coordination_effects.insert(indexed.effect_id);
                    coordinates.insert(coordinate.clone());
                }
            }
        }
    }
    if !conflicting_effects.is_empty() || !coordination_effects.is_empty() {
        return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
            RuntimeTransitionRebaseConflict {
                source_revision,
                current_revision,
                conflicting_effects: conflicting_effects.into_iter().collect(),
                coordination_effects: coordination_effects.into_iter().collect(),
                coordinates: coordinates.into_iter().collect(),
                opaque_effects: Vec::new(),
            },
        ));
    }

    let mut relation_deltas = BTreeMap::<SemanticId, RelationDelta>::new();
    for mutation in proposed {
        match relation_deltas.entry(mutation.relation) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(mutation.delta.clone());
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().result_type != mutation.delta.result_type {
                    return Err(DurableRuntimeCommitError::Runtime(
                        PhysicalExecutionError::InvalidRevisionTransition,
                    ));
                }
                entry
                    .get_mut()
                    .inserted
                    .extend(mutation.delta.inserted.clone());
                entry
                    .get_mut()
                    .removed
                    .extend(mutation.delta.removed.clone());
            }
        }
    }
    let mut relational_observations = BTreeMap::new();
    for relation in relation_deltas.keys() {
        for (observation_ref, capsule) in
            epoch.relational_observations_after(source_revision, *relation)
        {
            relational_observations
                .entry(observation_ref)
                .or_insert(capsule);
        }
    }
    let observation_entries = relational_observations.into_iter().collect::<Vec<_>>();
    let capsules = observation_entries
        .iter()
        .map(|(_, capsule)| *capsule)
        .collect::<Vec<_>>();
    let impacts = RelCausalCapsule::impact_many_relation_deltas(
        &capsules,
        &relation_deltas,
        semantic_context,
        registry,
    )
    .map_err(PhysicalExecutionError::from)?;
    let mut relational_coordination = BTreeSet::new();
    for ((observation_ref, _), impact) in observation_entries.into_iter().zip(impacts) {
        if impact == Impact::Changed {
            relational_coordination.insert(observation_ref.effect_id);
        }
    }
    if !relational_coordination.is_empty() {
        return Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(
            RuntimeTransitionRebaseConflict {
                source_revision,
                current_revision,
                conflicting_effects: Vec::new(),
                coordination_effects: relational_coordination.into_iter().collect(),
                coordinates: Vec::new(),
                opaque_effects: Vec::new(),
            },
        ));
    }
    Ok(())
}
