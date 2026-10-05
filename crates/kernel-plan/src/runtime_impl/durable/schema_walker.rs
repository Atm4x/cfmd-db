impl DurableRuntime {
    /// Compatibility wrapper over the prepared field publication path.
    pub fn commit_schema_aware_field_intent(
        &self,
        transaction_id: ClientTransactionId,
        request: &SchemaAwareFieldTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        if let Some(target_revision) = self.check_mixed_client_retry(
            transaction_id,
            request.formation_semantic_revision,
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
            client_guard_digest: request.client_guard_digest,
            source_model_delta: request.client_model_delta.clone(),
            current_model_delta: realized,
            publication_guard,
            source_mutations: Vec::new(),
            current_mutations: Vec::new(),
        })
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

impl DurableRuntime {
    /// Prepares one exact relation-data intent for the exact current head. Data transport,
    /// conflict certification and required publication authority are derived in one retained-epoch
    /// traversal. Grants are never transported; only the effect's required authority footprint is.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn prepare_schema_aware_publication(
        &self,
        formation_revision: RevisionId,
        formation_semantic_revision: kernel_types::SemanticRevision,
        mutations: &[RevisionRelationMutation<'_>],
        source_relation_writes: &BTreeSet<SemanticId>,
        source_field_writes: &BTreeSet<(SemanticId, SemanticId)>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<PreparedSchemaAwarePublication, DurableRuntimeCommitError> {
        if mutations.is_empty()
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
        if formation_revision > head_revision {
            return Err(
                DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                    revision: formation_revision,
                },
            );
        }
        let historical = head_snapshot.root().historical.clone();
        let retained_epochs = historical
            .retained_schema_epochs
            .iter()
            .filter_map(|(_, epoch)| {
                (epoch.source_revision >= formation_revision
                    && epoch.target_revision <= head_revision)
                    .then_some(epoch.clone())
            })
            .collect::<Vec<_>>();
        let mut context = retained_epochs
            .first().map_or_else(|| head_snapshot.revision().semantic_context().clone(), |epoch| epoch.source_context.clone());
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
        let mut relation_writes = source_relation_writes.clone();
        let mut field_writes = source_field_writes.clone();
        let mut relation_authorizations = mutations
            .iter()
            .filter(|mutation| mutation.authorization != kernel_durability::DurableRelationAuthorization::default())
            .map(|mutation| (mutation.relation, mutation.authorization))
            .collect::<BTreeMap<_, _>>();
        let mut segment_source = formation_revision;

        for epoch in retained_epochs {
            if epoch.source_context != context {
                return Err(DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                ));
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

            let transport = epoch
                .program
                .verify(&context, &self.registry)
                .map_err(DurableRuntimeCommitError::MigrationTransport)?;

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
            .certify_transition_rebase(segment_source, &current_refs, None, None)
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
            client_guard_digest,
            source_model_delta: DurableModelDelta::default(),
            current_model_delta: DurableModelDelta::default(),
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
        if prepared.current_model_delta != DurableModelDelta::default() {
            if !prepared.source_mutations.is_empty() || !prepared.current_mutations.is_empty() {
                return Err(DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::InvalidRevisionTransition,
                ));
            }
            if let Some(target_revision) = self.check_mixed_client_retry(
                transaction_id,
                prepared.formation_semantic_revision,
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
            drop(snapshot);
            let mut target_state = current.state().clone();
            prepared.current_model_delta.apply_to(&mut target_state);
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
            return self
                .commit_mixed_revision_residual_guarded_with_client_semantics_and_dependencies(
                    transaction_id,
                    &MixedRevisionTransitionRequest {
                        source_revision: current.id(),
                        target_revision: &target,
                        mutations: &[],
                        model_delta: &prepared.current_model_delta,
                        model_complement: &complement,
                        registry: &self.registry,
                    },
                    &[],
                    &prepared.source_model_delta,
                    prepared.formation_semantic_revision,
                    prepared.client_guard_digest,
                    prepared.publication_guard.as_ref(),
                    &[],
                );
        }

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
