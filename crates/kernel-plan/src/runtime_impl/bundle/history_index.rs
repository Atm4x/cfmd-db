impl RuntimeHistoricalDerivedIndex {
    fn from_current(
        revision: RevisionId,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
    ) -> Self {
        let mut out = Self {
            lineage_floor: Some(revision),
            ..Self::default()
        };
        for (&relation, base) in relation_bases {
            out.insert_support(revision, relation, base.support_witness());
        }
        out
    }

    fn insert_support(
        &mut self,
        revision: RevisionId,
        relation: SemanticId,
        support: RelationSupportWitness,
    ) {
        let mut timeline = self
            .relation_supports
            .get(&relation)
            .cloned()
            .unwrap_or_default();
        timeline.insert(revision, support);
        self.relation_supports.insert(relation, timeline);
    }

    fn index_relation_delta(
        &mut self,
        target_revision: RevisionId,
        relation: SemanticId,
        delta: RelationDelta,
    ) {
        let mut timeline = self
            .relation_deltas
            .get(&relation)
            .cloned()
            .unwrap_or_default();
        timeline.insert(target_revision, delta);
        self.relation_deltas.insert(relation, timeline);
    }

    fn support_at(
        &self,
        revision: RevisionId,
        relation: SemanticId,
    ) -> Option<&RelationSupportWitness> {
        let timeline = self.relation_supports.get(&relation)?;
        if let Some(exact) = timeline.get(&revision) {
            return Some(exact);
        }
        let rank = timeline.rank_before(&revision);
        if rank == 0 {
            return None;
        }
        let key = timeline.key_at_rank(rank - 1)?;
        timeline.get(key)
    }

    fn reset_at(
        &mut self,
        revision: RevisionId,
        opaque_effect: Option<u128>,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
    ) {
        let retained_schema_epochs = self.retained_schema_epochs.clone();
        *self = Self::from_current(revision, relation_bases);
        self.retained_schema_epochs = retained_schema_epochs;
        self.floor_opaque_effect = opaque_effect;
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    fn seal_schema_epoch(
        &mut self,
        source_revision: RevisionId,
        target_revision: RevisionId,
        effect_id: u128,
        self_context: kernel_schema::SemanticContext,
        source_fields: kernel_model::CowMap<(SemanticId, kernel_types::EntityId), Value>,
        program: kernel_transport::SchemaMigrationProgram,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
    ) {
        let index = RuntimeRetainedEpochIndex {
            lineage_floor: self.lineage_floor,
            relation_supports: self.relation_supports.clone(),
            relation_deltas: self.relation_deltas.clone(),
            writes: self.writes.clone(),
            observations: self.observations.clone(),
            joint_observation_groups: self.joint_observation_groups.clone(),
            relational_observations: self.relational_observations.clone(),
            relational_capsule_keys: self.relational_capsule_keys.clone(),
            relational_capsules: self.relational_capsules.clone(),
            relational_routes: self.relational_routes.clone(),
            exact_effects: self.exact_effects.clone(),
        };
        self.retained_schema_epochs.insert(
            source_revision,
            RuntimeRetainedSchemaEpoch {
                effect_id,
                source_revision,
                target_revision,
                source_context: self_context,
                source_fields,
                program,
                index,
            },
        );
        self.reset_at(target_revision, Some(effect_id), relation_bases);
    }

    fn index_exact_effect(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        footprint: RuntimeHistoryFootprint,
    ) {
        self.exact_effects.insert(target_revision, effect_id);
        for (coordinate, action) in footprint.writes {
            let mut timeline = self.writes.get(&coordinate).cloned().unwrap_or_default();
            timeline.insert(
                target_revision,
                RuntimeIndexedHistoryAction { effect_id, action },
            );
            self.writes.insert(coordinate, timeline);
        }
    }

    fn index_causal_observations(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        observations: impl IntoIterator<
            Item = (
                RuntimeHistoryCoordinate,
                Option<Value>,
                Option<kernel_schema::SemanticRuleExpr>,
            ),
        >,
    ) {
        for (coordinate, exact_value, preservation_rule) in observations {
            let mut timeline = self
                .observations
                .get(&coordinate)
                .cloned()
                .unwrap_or_default();
            timeline.insert(
                target_revision,
                RuntimeIndexedCausalObservation {
                    effect_id,
                    exact_value,
                    preservation_rule,
                    joint_group_ids: Vec::new(),
                },
            );
            self.observations.insert(coordinate, timeline);
        }
    }

    fn index_causal_observation_groups(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        groups: impl IntoIterator<Item = RuntimeJointCausalObservationGroup>,
    ) {
        for group in groups {
            let key = (effect_id, group.group_id);
            if let Some(existing) = self.joint_observation_groups.get(&key) {
                debug_assert_eq!(existing, &group);
            } else {
                self.joint_observation_groups.insert(key, group.clone());
            }
            for field in group.observed_fields.keys() {
                let coordinates = [
                    RuntimeHistoryCoordinate::Field {
                        field: *field,
                        owner: group.owner,
                    },
                    RuntimeHistoryCoordinate::ObjectField {
                        relation: group.relation,
                        owner: group.owner,
                        field: *field,
                    },
                ];
                for coordinate in coordinates {
                    let mut timeline = self
                        .observations
                        .get(&coordinate)
                        .cloned()
                        .unwrap_or_default();
                    let mut indexed = timeline.get(&target_revision).cloned().unwrap_or(
                        RuntimeIndexedCausalObservation {
                            effect_id,
                            exact_value: None,
                            preservation_rule: None,
                            joint_group_ids: Vec::new(),
                        },
                    );
                    debug_assert_eq!(indexed.effect_id, effect_id);
                    if !indexed.joint_group_ids.contains(&group.group_id) {
                        indexed.joint_group_ids.push(group.group_id);
                    }
                    timeline.insert(target_revision, indexed);
                    self.observations.insert(coordinate, timeline);
                }
            }
        }
    }

    fn index_relational_causal_capsule(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        observation_id: u32,
        capsule: &RelCausalCapsule,
    ) -> Result<(), PhysicalExecutionError> {
        let query_identity = kernel_durability::canonical_rel_expr_identity(capsule.query())
            .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
        let capsule_key = (capsule.revision(), query_identity);
        let capsule_ref = if let Some(existing) = self.relational_capsule_keys.get(&capsule_key) {
            *existing
        } else {
            let next = u32::try_from(self.relational_capsules.len())
                .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
            self.relational_capsule_keys.insert(capsule_key, next);
            self.relational_capsules.insert(next, capsule.clone());
            next
        };
        let observation_ref = RuntimeRelationalObservationRef {
            effect_id,
            observation_id,
        };
        let indexed = RuntimeIndexedRelationalCausalObservation { capsule_ref };
        if let Some(existing) = self.relational_observations.get(&observation_ref) {
            if existing != &indexed {
                return Err(PhysicalExecutionError::InvalidRevisionTransition);
            }
        } else {
            self.relational_observations
                .insert(observation_ref, indexed);
        }
        for relation in capsule.source_relations() {
            let mut timeline = self
                .relational_routes
                .get(&relation)
                .cloned()
                .unwrap_or_default();
            let mut refs = timeline.get(&target_revision).cloned().unwrap_or_default();
            refs.insert(observation_ref);
            timeline.insert(target_revision, refs);
            self.relational_routes.insert(relation, timeline);
        }
        Ok(())
    }

    fn relational_observations_after(
        &self,
        source_revision: RevisionId,
        relation: SemanticId,
    ) -> Vec<(RuntimeRelationalObservationRef, &RelCausalCapsule)> {
        let Some(timeline) = self.relational_routes.get(&relation) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(revision) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(refs) = timeline.get(revision) {
                for observation_ref in refs {
                    if !seen.insert(*observation_ref) {
                        continue;
                    }
                    let Some(indexed) = self.relational_observations.get(observation_ref) else {
                        continue;
                    };
                    if let Some(capsule) = self.relational_capsules.get(&indexed.capsule_ref) {
                        out.push((*observation_ref, capsule));
                    }
                }
            }
            rank += 1;
        }
        out
    }
    fn observations_after<'a>(
        &'a self,
        source_revision: RevisionId,
        coordinate: &RuntimeHistoryCoordinate,
    ) -> Vec<&'a RuntimeIndexedCausalObservation> {
        let Some(timeline) = self.observations.get(coordinate) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(key) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(observation) = timeline.get(key) {
                out.push(observation);
            }
            rank += 1;
        }
        out
    }

    fn joint_observation_group(
        &self,
        effect_id: u128,
        group_id: u32,
    ) -> Option<&RuntimeJointCausalObservationGroup> {
        self.joint_observation_groups.get(&(effect_id, group_id))
    }

    fn actions_after<'a>(
        &'a self,
        source_revision: RevisionId,
        coordinate: &RuntimeHistoryCoordinate,
    ) -> Vec<&'a RuntimeIndexedHistoryAction> {
        let Some(timeline) = self.writes.get(coordinate) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(key) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(action) = timeline.get(key) {
                out.push(action);
            }
            rank += 1;
        }
        out
    }

    fn contains_revision(&self, revision: RevisionId) -> bool {
        self.lineage_floor == Some(revision) || self.exact_effects.contains_key(&revision)
    }

    fn exact_effect_count_after(&self, source_revision: RevisionId) -> usize {
        let mut rank = self.exact_effects.rank_before(&source_revision);
        if self.exact_effects.get(&source_revision).is_some() {
            rank += 1;
        }
        self.exact_effects.len().saturating_sub(rank)
    }
}

impl RuntimeRetainedEpochIndex {
    fn index_relation_delta(
        &mut self,
        target_revision: RevisionId,
        relation: SemanticId,
        delta: RelationDelta,
    ) {
        let mut timeline = self
            .relation_deltas
            .get(&relation)
            .cloned()
            .unwrap_or_default();
        timeline.insert(target_revision, delta);
        self.relation_deltas.insert(relation, timeline);
    }

    fn relation_deltas_between(
        &self,
        source_revision: RevisionId,
        boundary_revision: RevisionId,
        relations: &BTreeSet<SemanticId>,
    ) -> Option<BTreeMap<RevisionId, BTreeMap<SemanticId, RelationDelta>>> {
        let lineage_floor = self.lineage_floor?;
        if source_revision < lineage_floor || boundary_revision < source_revision {
            return None;
        }
        let mut out = BTreeMap::<RevisionId, BTreeMap<SemanticId, RelationDelta>>::new();
        for relation in relations {
            let Some(timeline) = self.relation_deltas.get(relation) else {
                continue;
            };
            let mut rank = timeline.rank_before(&source_revision);
            if timeline.get(&source_revision).is_some() {
                rank += 1;
            }
            while rank < timeline.len() {
                let revision = *timeline.key_at_rank(rank)?;
                if revision > boundary_revision {
                    break;
                }
                let delta = timeline.get(&revision)?.clone();
                out.entry(revision).or_default().insert(*relation, delta);
                rank += 1;
            }
        }
        Some(out)
    }

    fn exact_effect_at(&self, target_revision: RevisionId) -> Option<u128> {
        self.exact_effects.get(&target_revision).copied()
    }

    fn index_causal_observation_groups(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        groups: impl IntoIterator<Item = RuntimeJointCausalObservationGroup>,
    ) {
        for group in groups {
            let key = (effect_id, group.group_id);
            if let Some(existing) = self.joint_observation_groups.get(&key) {
                debug_assert_eq!(existing, &group);
            } else {
                self.joint_observation_groups.insert(key, group.clone());
            }
            for field in group.observed_fields.keys() {
                let coordinates = [
                    RuntimeHistoryCoordinate::Field {
                        field: *field,
                        owner: group.owner,
                    },
                    RuntimeHistoryCoordinate::ObjectField {
                        relation: group.relation,
                        owner: group.owner,
                        field: *field,
                    },
                ];
                for coordinate in coordinates {
                    let mut timeline = self
                        .observations
                        .get(&coordinate)
                        .cloned()
                        .unwrap_or_default();
                    let mut indexed = timeline.get(&target_revision).cloned().unwrap_or(
                        RuntimeIndexedCausalObservation {
                            effect_id,
                            exact_value: None,
                            preservation_rule: None,
                            joint_group_ids: Vec::new(),
                        },
                    );
                    debug_assert_eq!(indexed.effect_id, effect_id);
                    if !indexed.joint_group_ids.contains(&group.group_id) {
                        indexed.joint_group_ids.push(group.group_id);
                    }
                    timeline.insert(target_revision, indexed);
                    self.observations.insert(coordinate, timeline);
                }
            }
        }
    }

    fn index_relational_causal_capsule(
        &mut self,
        target_revision: RevisionId,
        effect_id: u128,
        observation_id: u32,
        capsule: &RelCausalCapsule,
    ) -> Result<(), PhysicalExecutionError> {
        let query_identity = kernel_durability::canonical_rel_expr_identity(capsule.query())
            .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
        let capsule_key = (capsule.revision(), query_identity);
        let capsule_ref = if let Some(existing) = self.relational_capsule_keys.get(&capsule_key) {
            *existing
        } else {
            let next = u32::try_from(self.relational_capsules.len())
                .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
            self.relational_capsule_keys.insert(capsule_key, next);
            self.relational_capsules.insert(next, capsule.clone());
            next
        };
        let observation_ref = RuntimeRelationalObservationRef {
            effect_id,
            observation_id,
        };
        let indexed = RuntimeIndexedRelationalCausalObservation { capsule_ref };
        if let Some(existing) = self.relational_observations.get(&observation_ref) {
            if existing != &indexed {
                return Err(PhysicalExecutionError::InvalidRevisionTransition);
            }
        } else {
            self.relational_observations
                .insert(observation_ref, indexed);
        }
        for relation in capsule.source_relations() {
            let mut timeline = self
                .relational_routes
                .get(&relation)
                .cloned()
                .unwrap_or_default();
            let mut refs = timeline.get(&target_revision).cloned().unwrap_or_default();
            refs.insert(observation_ref);
            timeline.insert(target_revision, refs);
            self.relational_routes.insert(relation, timeline);
        }
        Ok(())
    }

    fn relational_observations_after(
        &self,
        source_revision: RevisionId,
        relation: SemanticId,
    ) -> Vec<(RuntimeRelationalObservationRef, &RelCausalCapsule)> {
        let Some(timeline) = self.relational_routes.get(&relation) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(revision) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(refs) = timeline.get(revision) {
                for observation_ref in refs {
                    if !seen.insert(*observation_ref) {
                        continue;
                    }
                    let Some(indexed) = self.relational_observations.get(observation_ref) else {
                        continue;
                    };
                    if let Some(capsule) = self.relational_capsules.get(&indexed.capsule_ref) {
                        out.push((*observation_ref, capsule));
                    }
                }
            }
            rank += 1;
        }
        out
    }

    fn support_at(
        &self,
        revision: RevisionId,
        relation: SemanticId,
    ) -> Option<&RelationSupportWitness> {
        let timeline = self.relation_supports.get(&relation)?;
        if let Some(exact) = timeline.get(&revision) {
            return Some(exact);
        }
        let rank = timeline.rank_before(&revision);
        if rank == 0 {
            return None;
        }
        let key = timeline.key_at_rank(rank - 1)?;
        timeline.get(key)
    }

    fn actions_after<'a>(
        &'a self,
        source_revision: RevisionId,
        coordinate: &RuntimeHistoryCoordinate,
    ) -> Vec<&'a RuntimeIndexedHistoryAction> {
        let Some(timeline) = self.writes.get(coordinate) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(key) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(action) = timeline.get(key) {
                out.push(action);
            }
            rank += 1;
        }
        out
    }
    fn observations_after<'a>(
        &'a self,
        source_revision: RevisionId,
        coordinate: &RuntimeHistoryCoordinate,
    ) -> Vec<&'a RuntimeIndexedCausalObservation> {
        let Some(timeline) = self.observations.get(coordinate) else {
            return Vec::new();
        };
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let mut out = Vec::new();
        while rank < timeline.len() {
            let Some(key) = timeline.key_at_rank(rank) else {
                break;
            };
            if let Some(observation) = timeline.get(key) {
                out.push(observation);
            }
            rank += 1;
        }
        out
    }

    fn joint_observation_group(
        &self,
        effect_id: u128,
        group_id: u32,
    ) -> Option<&RuntimeJointCausalObservationGroup> {
        self.joint_observation_groups.get(&(effect_id, group_id))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn rebuild_from_boundary(
        boundary_revision: RevisionId,
        boundary_model: &kernel_model::FiniteModel,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
        effects_backwards: &[RuntimeHistoryEffect],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let mut out = Self {
            lineage_floor: Some(boundary_revision),
            ..Self::default()
        };
        let mut supports = relation_bases
            .iter()
            .map(|(&relation, base)| (relation, base.support_witness()))
            .collect::<BTreeMap<_, _>>();
        for (&relation, support) in &supports {
            let mut timeline = PersistentOrdMap::default();
            timeline.insert(boundary_revision, support.clone());
            out.relation_supports.insert(relation, timeline);
        }
        let mut floor = boundary_revision;
        for effect in effects_backwards {
            let exact = effect.reversibility == RuntimeHistoryReversibility::ExactPlanInverse
                && effect.semantic_change.is_none()
                && matches!(
                    effect.kind,
                    RuntimeHistoryEffectKind::RelationData
                        | RuntimeHistoryEffectKind::RelationRewrite
                        | RuntimeHistoryEffectKind::RelationResolution
                        | RuntimeHistoryEffectKind::MixedRevision
                );
            if !exact {
                floor = effect.target_revision;
                break;
            }
            let footprint = runtime_history_effect_footprint(effect, context, registry)
                .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
            out.exact_effects
                .insert(effect.target_revision, effect.effect_id);
            for (coordinate, action) in footprint.writes {
                let mut timeline = out.writes.get(&coordinate).cloned().unwrap_or_default();
                timeline.insert(
                    effect.target_revision,
                    RuntimeIndexedHistoryAction {
                        effect_id: effect.effect_id,
                        action,
                    },
                );
                out.writes.insert(coordinate, timeline);
            }
            for coordinate in &effect.causal_observations {
                let mut timeline = out
                    .observations
                    .get(coordinate)
                    .cloned()
                    .unwrap_or_default();
                timeline.insert(
                    effect.target_revision,
                    RuntimeIndexedCausalObservation {
                        effect_id: effect.effect_id,
                        exact_value: effect.causal_observation_values.get(coordinate).cloned(),
                        preservation_rule: effect
                            .causal_observation_predicates
                            .get(coordinate)
                            .cloned(),
                        joint_group_ids: Vec::new(),
                    },
                );
                out.observations.insert(coordinate.clone(), timeline);
            }
            out.index_causal_observation_groups(
                effect.target_revision,
                effect.effect_id,
                effect.causal_observation_groups.iter().cloned(),
            );
            for mutation in &effect.relation_mutations {
                let support = supports.get(&mutation.relation).ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation),
                )?;
                let transition = RelationDelta {
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    result_type: support.result_type().clone(),
                };
                out.index_relation_delta(
                    effect.target_revision,
                    mutation.relation,
                    transition.clone(),
                );
                let rewound = support
                    .rewind_exact(effect.source_revision, &transition, registry)
                    .map_err(PhysicalExecutionError::from)?;
                supports.insert(mutation.relation, rewound.clone());
                let mut timeline = out
                    .relation_supports
                    .get(&mutation.relation)
                    .cloned()
                    .unwrap_or_default();
                timeline.insert(effect.source_revision, rewound);
                out.relation_supports.insert(mutation.relation, timeline);
            }
            floor = effect.source_revision;
        }
        for (relation, support) in supports {
            let mut timeline = out
                .relation_supports
                .get(&relation)
                .cloned()
                .unwrap_or_default();
            timeline.insert(floor, support);
            out.relation_supports.insert(relation, timeline);
        }
        for (route_revision, effect_id, observation_id, capsule) in
            reconstruct_relational_causal_capsules(
                boundary_revision,
                boundary_model,
                effects_backwards,
                context,
                registry,
            )?
        {
            out.index_relational_causal_capsule(
                route_revision,
                effect_id,
                observation_id,
                &capsule,
            )?;
        }
        out.lineage_floor = Some(floor);
        Ok(out)
    }

    fn first_action_after(
        &self,
        source_revision: RevisionId,
        coordinate: &RuntimeHistoryCoordinate,
    ) -> Option<&RuntimeIndexedHistoryAction> {
        let timeline = self.writes.get(coordinate)?;
        let mut rank = timeline.rank_before(&source_revision);
        if timeline.get(&source_revision).is_some() {
            rank += 1;
        }
        let key = timeline.key_at_rank(rank)?;
        timeline.get(key)
    }
}

impl RuntimeRevisionBundle {
    fn index_support_successors(
        historical: &mut RuntimeHistoricalDerivedIndex,
        target_revision: RevisionId,
        relation_bases: &PersistentOrdMap<SemanticId, RelationBaseWitness>,
        changed_relations: impl IntoIterator<Item = SemanticId>,
    ) {
        for relation in changed_relations {
            if let Some(base) = relation_bases.get(&relation) {
                historical.insert_support(target_revision, relation, base.support_witness());
            }
        }
    }

    fn bind_committed_history_effect(
        &mut self,
        effect_id: u128,
        descriptor: &RevisionCommitDescriptor,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let target_revision = descriptor.target_revision();
        let Some(relation_deltas) = descriptor.relation_deltas() else {
            self.historical
                .reset_at(target_revision, Some(effect_id), &self.relation_bases);
            return Ok(());
        };
        for (&relation, delta) in relation_deltas {
            self.historical
                .index_relation_delta(target_revision, relation, delta.clone());
        }
        let mut footprint = RuntimeHistoryFootprint::default();
        for (&relation, delta) in relation_deltas {
            if let Some(field_writes) = descriptor.object_field_writes.get(&relation)
                && !field_writes.is_empty()
            {
                for write in field_writes {
                    insert_runtime_history_action(
                        &mut footprint.writes,
                        RuntimeHistoryCoordinate::ObjectField {
                            relation,
                            owner: write.owner,
                            field: write.field,
                        },
                        RewriteActionLaw::Opaque,
                    );
                }
                continue;
            }
            let delta_footprint =
                delta.rewrite_footprint(relation, self.revision.semantic_context(), registry)?;
            for (coordinate, action) in delta_footprint.writes {
                let kernel_change::SemanticWriteCoordinate::RelationClass {
                    relation,
                    canonical_key,
                } = coordinate
                else {
                    return Err(PhysicalExecutionError::InvalidRevisionTransition);
                };
                insert_runtime_history_action(
                    &mut footprint.writes,
                    RuntimeHistoryCoordinate::RelationClass {
                        relation,
                        canonical_key,
                    },
                    action,
                );
            }
        }
        if let RevisionCommitChange::MixedRevision {
            model_delta,
            model_complement,
            ..
        } = descriptor.change()
        {
            add_model_delta_footprint(&mut footprint.writes, model_delta, Some(model_complement));
        }
        self.historical
            .index_exact_effect(target_revision, effect_id, footprint);
        self.historical.index_causal_observations(
            target_revision,
            effect_id,
            descriptor
                .causal_observations()
                .iter()
                .cloned()
                .map(|coordinate| {
                    let exact_value = descriptor
                        .causal_observation_exact_value(&coordinate)
                        .cloned();
                    let preservation_rule = descriptor
                        .causal_observation_preservation_rule(&coordinate)
                        .cloned();
                    (coordinate, exact_value, preservation_rule)
                }),
        );
        self.historical.index_causal_observation_groups(
            descriptor.target_revision(),
            effect_id,
            descriptor.causal_observation_groups().iter().cloned(),
        );
        for observation in descriptor.relational_causal_observations() {
            self.historical.index_relational_causal_capsule(
                descriptor.target_revision(),
                effect_id,
                observation.observation_id,
                &observation.capsule,
            )?;
        }
        Ok(())
    }

    fn historical_support_at(
        &self,
        revision: RevisionId,
        relation: SemanticId,
    ) -> Option<&RelationSupportWitness> {
        if !self.historical.contains_revision(revision) {
            return None;
        }
        self.historical.support_at(revision, relation)
    }
}

impl PreparedRuntimeRevisionTransition {
    fn bind_committed_schema_migration_history_effect(
        mut self,
        effect_id: u128,
        program: &kernel_transport::SchemaMigrationProgram,
    ) -> Self {
        let source_revision = self.descriptor.source_revision();
        let target_revision = self.descriptor.target_revision();
        // A full-revision migration candidate is rebuilt from the target world and
        // therefore starts with an empty reconstructible historical index. The
        // schema boundary must seal the *source* persistent index, then reset that
        // authority to the target epoch. Otherwise pre-migration Γ/action roots are
        // silently lost exactly where formation-world transactions need them.
        let mut historical = self.source_historical.clone();
        historical.seal_schema_epoch(
            source_revision,
            target_revision,
            effect_id,
            self.source_context.clone(),
            self.source_fields.clone(),
            program.clone(),
            &self.candidate.relation_bases,
        );
        self.candidate.historical = historical;
        self
    }

    fn bind_committed_history_effect(
        mut self,
        effect_id: u128,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        self.candidate
            .bind_committed_history_effect(effect_id, &self.descriptor, registry)?;
        Ok(self)
    }
}

impl RuntimeRevisionBundle {
    fn rebuild_historical_derived_index(
        &mut self,
        effects_backwards: &[RuntimeHistoryEffect],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let head_revision = self.revision.id();
        let mut index =
            RuntimeHistoricalDerivedIndex::from_current(head_revision, &self.relation_bases);
        let mut supports = self
            .relation_bases
            .iter()
            .map(|(&relation, base)| (relation, base.support_witness()))
            .collect::<BTreeMap<_, _>>();
        let mut floor = head_revision;

        for effect in effects_backwards {
            let exact = effect.reversibility == RuntimeHistoryReversibility::ExactPlanInverse
                && effect.semantic_change.is_none()
                && matches!(
                    effect.kind,
                    RuntimeHistoryEffectKind::RelationData
                        | RuntimeHistoryEffectKind::RelationRewrite
                        | RuntimeHistoryEffectKind::RelationResolution
                        | RuntimeHistoryEffectKind::MixedRevision
                );
            if !exact {
                floor = effect.target_revision;
                index.floor_opaque_effect = Some(effect.effect_id);
                break;
            }

            let footprint = runtime_history_effect_footprint(
                effect,
                self.revision.semantic_context(),
                registry,
            )
            .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
            index.index_exact_effect(effect.target_revision, effect.effect_id, footprint);
            index.index_causal_observations(
                effect.target_revision,
                effect.effect_id,
                effect
                    .causal_observations
                    .iter()
                    .cloned()
                    .map(|coordinate| {
                        let exact_value =
                            effect.causal_observation_values.get(&coordinate).cloned();
                        let preservation_rule = effect
                            .causal_observation_predicates
                            .get(&coordinate)
                            .cloned();
                        (coordinate, exact_value, preservation_rule)
                    }),
            );
            index.index_causal_observation_groups(
                effect.target_revision,
                effect.effect_id,
                effect.causal_observation_groups.iter().cloned(),
            );

            for mutation in &effect.relation_mutations {
                let support = supports.get(&mutation.relation).ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation),
                )?;
                let transition = RelationDelta {
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    result_type: support.result_type().clone(),
                };
                index.index_relation_delta(
                    effect.target_revision,
                    mutation.relation,
                    transition.clone(),
                );
                let rewound = support
                    .rewind_exact(effect.source_revision, &transition, registry)
                    .map_err(PhysicalExecutionError::from)?;
                supports.insert(mutation.relation, rewound.clone());
                index.insert_support(effect.source_revision, mutation.relation, rewound);
            }
            floor = effect.source_revision;
        }

        for (relation, support) in supports {
            index.insert_support(floor, relation, support);
        }
        for (route_revision, effect_id, observation_id, capsule) in
            reconstruct_relational_causal_capsules(
                head_revision,
                &self.revision.state().model,
                effects_backwards,
                self.revision.semantic_context(),
                registry,
            )?
        {
            index.index_relational_causal_capsule(
                route_revision,
                effect_id,
                observation_id,
                &capsule,
            )?;
        }
        index.lineage_floor = Some(floor);
        self.historical = index;
        Ok(())
    }
}

fn reconstruct_relational_causal_capsules(
    boundary_revision: RevisionId,
    boundary_model: &kernel_model::FiniteModel,
    effects_backwards: &[RuntimeHistoryEffect],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<(RevisionId, u128, u32, RelCausalCapsule)>, PhysicalExecutionError> {
    reconstruct_relational_causal_capsules_with_stats(
        boundary_revision,
        boundary_model,
        effects_backwards,
        context,
        registry,
    )
    .map(|(capsules, _)| capsules)
}

type RebuiltRelationalCapsules = Vec<(RevisionId, u128, u32, RelCausalCapsule)>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RelationalCapsuleRebuildStats {
    unique_lineages: usize,
    root_occurrences: usize,
    unique_nodes: usize,
    fanout_edges: usize,
    exact_effects_scanned: usize,
    effect_delta_maps_built: usize,
    forest_rewinds: usize,
    node_transitions: usize,
    observation_routes: usize,
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn reconstruct_relational_causal_capsules_with_stats(
    boundary_revision: RevisionId,
    boundary_model: &kernel_model::FiniteModel,
    effects_backwards: &[RuntimeHistoryEffect],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<
    (
        RebuiltRelationalCapsules,
        RelationalCapsuleRebuildStats,
    ),
    PhysicalExecutionError,
> {
    let mut lineages = BTreeMap::<
        Vec<u8>,
        (
            RelExpr,
            BTreeMap<
                RevisionId,
                Vec<(
                    RevisionId,
                    u128,
                    u32,
                    kernel_durability::DurableIntentPrefix,
                )>,
            >,
        ),
    >::new();
    let mut exact_effects = Vec::new();
    for effect in effects_backwards {
        let exact = effect.reversibility == RuntimeHistoryReversibility::ExactPlanInverse
            && effect.semantic_change.is_none()
            && matches!(
                effect.kind,
                RuntimeHistoryEffectKind::RelationData
                    | RuntimeHistoryEffectKind::RelationRewrite
                    | RuntimeHistoryEffectKind::RelationResolution
                    | RuntimeHistoryEffectKind::MixedRevision
            );
        if !exact {
            break;
        }
        exact_effects.push(effect);
        for observation in &effect.relational_causal_observations {
            let identity = kernel_durability::canonical_rel_expr_identity(&observation.query)
                .map_err(|_| PhysicalExecutionError::InvalidRevisionTransition)?;
            let (_, revisions) = lineages
                .entry(identity)
                .or_insert_with(|| (observation.query.clone(), BTreeMap::new()));
            revisions
                .entry(observation.observed_revision)
                .or_default()
                .push((
                    effect.target_revision,
                    effect.effect_id,
                    observation.observation_id,
                    observation.intent_prefix.clone(),
                ));
        }
    }

    if lineages.is_empty() {
        return Ok((
            Vec::new(),
            RelationalCapsuleRebuildStats {
                exact_effects_scanned: exact_effects.len(),
                ..RelationalCapsuleRebuildStats::default()
            },
        ));
    }

    let mut queries = Vec::with_capacity(lineages.len());
    let mut requests_by_revision =
        BTreeMap::<RevisionId, Vec<(usize, RevisionId, u128, u32, kernel_durability::DurableIntentPrefix)>>::new();
    let mut observation_routes = 0usize;
    for (_, (query, requested)) in lineages {
        let root = queries.len();
        queries.push(query);
        for (observed_revision, refs) in requested {
            let routes = requests_by_revision.entry(observed_revision).or_default();
            for (route_revision, effect_id, observation_id, intent_prefix) in refs {
                routes.push((root, route_revision, effect_id, observation_id, intent_prefix));
                observation_routes += 1;
            }
        }
    }

    let mut forest = kernel_query::RelObservationForest::build(
        &queries,
        boundary_model,
        context,
        registry,
    )?;
    forest.bind_revision(boundary_revision)?;
    let source_relations = queries
        .iter()
        .flat_map(RelExpr::scan_relations)
        .collect::<BTreeSet<_>>();
    let mut stats = RelationalCapsuleRebuildStats {
        unique_lineages: queries.len(),
        root_occurrences: forest.root_count(),
        unique_nodes: forest.unique_node_count(),
        fanout_edges: forest.fanout_edge_count(),
        exact_effects_scanned: exact_effects.len(),
        observation_routes,
        ..RelationalCapsuleRebuildStats::default()
    };

    let mut out = Vec::new();
    let mut prefix_result_types = BTreeMap::<SemanticId, kernel_query::RelType>::new();
    let mut emit_requests = |revision: RevisionId,
                             forest: &kernel_query::RelObservationForest,
                             out: &mut Vec<(RevisionId, u128, u32, RelCausalCapsule)>|
     -> Result<(), PhysicalExecutionError> {
        let Some(refs) = requests_by_revision.remove(&revision) else {
            return Ok(());
        };
        let shared = Arc::new(forest.clone());
        let mut prefixed = BTreeMap::<usize, (kernel_durability::DurableIntentPrefix, Vec<(usize, RevisionId, u128, u32)>)>::new();
        for (root, route_revision, effect_id, observation_id, intent_prefix) in refs {
            if intent_prefix.is_empty() {
                out.push((
                    route_revision,
                    effect_id,
                    observation_id,
                    RelCausalCapsule::capture_forest_root(Arc::clone(&shared), root)?,
                ));
                continue;
            }
            let identity = Arc::as_ptr(
                intent_prefix
                    .tail()
                    .ok_or(PhysicalExecutionError::InvalidRevisionTransition)?,
            ) as usize;
            let entry = prefixed
                .entry(identity)
                .or_insert_with(|| (intent_prefix, Vec::new()));
            entry.1.push((root, route_revision, effect_id, observation_id));
        }
        for (_, (intent_prefix, routes)) in prefixed {
            let mut candidate = forest.clone();
            let mut candidate_revision = revision;
            for segment in intent_prefix.segments_oldest_first() {
                let mut deltas = BTreeMap::new();
                for mutation in segment {
                    if !source_relations.contains(&mutation.relation) {
                        continue;
                    }
                    let result_type = if let Some(cached) = prefix_result_types.get(&mutation.relation) {
                        cached.clone()
                    } else {
                        let ty = RelExpr::Scan(mutation.relation).typecheck(context, registry)?;
                        prefix_result_types.insert(mutation.relation, ty.clone());
                        ty
                    };
                    deltas.insert(
                        mutation.relation,
                        RelationDelta {
                            inserted: mutation.inserted.clone(),
                            removed: mutation.removed.clone(),
                            result_type,
                        },
                    );
                }
                if deltas.is_empty() {
                    continue;
                }
                candidate_revision = RevisionId::new(
                    candidate_revision
                        .raw()
                        .checked_add(1)
                        .ok_or(PhysicalExecutionError::InvalidRevisionTransition)?,
                );
                candidate = candidate
                    .candidate_from_relation_deltas_for_revision(
                        candidate_revision,
                        &deltas,
                        context,
                        registry,
                    )?
                    .0;
            }
            let candidate = Arc::new(candidate);
            for (root, route_revision, effect_id, observation_id) in routes {
                out.push((
                    route_revision,
                    effect_id,
                    observation_id,
                    RelCausalCapsule::capture_forest_root(Arc::clone(&candidate), root)?,
                ));
            }
        }
        Ok(())
    };

    let mut result_types = BTreeMap::<SemanticId, kernel_query::RelType>::new();
    let mut world_revision = boundary_revision;
    emit_requests(world_revision, &forest, &mut out)?;
    for effect in &exact_effects {
        if effect.target_revision != world_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition);
        }
        let relevant_mutations = effect
            .relation_mutations
            .iter()
            .filter(|mutation| source_relations.contains(&mutation.relation))
            .collect::<Vec<_>>();
        if !relevant_mutations.is_empty() {
            stats.effect_delta_maps_built += 1;
            let mut deltas = BTreeMap::new();
            for mutation in relevant_mutations {
                let result_type = if let Some(cached) = result_types.get(&mutation.relation) {
                    cached.clone()
                } else {
                    let ty = RelExpr::Scan(mutation.relation).typecheck(context, registry)?;
                    result_types.insert(mutation.relation, ty.clone());
                    ty
                };
                match deltas.entry(mutation.relation) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(RelationDelta {
                            inserted: mutation.inserted.clone(),
                            removed: mutation.removed.clone(),
                            result_type,
                        });
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if entry.get().result_type != result_type {
                            return Err(PhysicalExecutionError::InvalidRevisionTransition);
                        }
                        entry.get_mut().inserted.extend(mutation.inserted.clone());
                        entry.get_mut().removed.extend(mutation.removed.clone());
                    }
                }
            }
            let (rewound, _, visited_nodes) = forest.rewind_forward_deltas_for_revision_with_stats(
                effect.source_revision,
                &deltas,
                context,
                registry,
            )?;
            stats.forest_rewinds += 1;
            stats.node_transitions = stats
                .node_transitions
                .checked_add(visited_nodes)
                .ok_or(PhysicalExecutionError::InvalidRevisionTransition)?;
            forest = rewound;
        }
        world_revision = effect.source_revision;
        emit_requests(world_revision, &forest, &mut out)?;
    }
    if !requests_by_revision.is_empty() {
        return Err(PhysicalExecutionError::InvalidRevisionTransition);
    }
    Ok((out, stats))
}

#[cfg(test)]
mod causal_group_index_compression_tests {
    use super::*;

    fn indexed_shape(field_count: usize) -> (usize, usize, usize) {
        let mut index = RuntimeHistoricalDerivedIndex::default();
        let relation = SemanticId::new(0x4840);
        let owner = kernel_types::EntityId::new(0x4841);
        let observed_fields = (0..field_count)
            .map(|offset| {
                (
                    SemanticId::new(0x5000 + offset as u128),
                    Value::I64(i64::try_from(offset).expect("fixture value fits i64")),
                )
            })
            .collect::<BTreeMap<_, _>>();
        index.index_causal_observation_groups(
            RevisionId::new(2),
            0x4842,
            [RuntimeJointCausalObservationGroup {
                group_id: 7,
                relation,
                owner,
                observed_fields,
                predicate: kernel_schema::SemanticRuleExpr::True,
            }],
        );

        let payload_count = index.joint_observation_groups.len();
        let payload_fields = index
            .joint_observation_groups
            .values()
            .map(|group| group.observed_fields.len())
            .sum::<usize>();
        let route_refs = index
            .observations
            .values()
            .flat_map(kernel_persistent::PersistentOrdMap::values)
            .map(|observation| observation.joint_group_ids.len())
            .sum::<usize>();
        (payload_count, payload_fields, route_refs)
    }

    #[test]
    fn p484_joint_group_index_interns_payload_once_and_scales_linearly() {
        for field_count in [8usize, 64, 512] {
            let (payload_count, payload_fields, route_refs) = indexed_shape(field_count);
            assert_eq!(payload_count, 1, "field_count={field_count}");
            assert_eq!(payload_fields, field_count, "field_count={field_count}");
            assert_eq!(route_refs, field_count * 2, "field_count={field_count}");
        }
    }
}

#[cfg(test)]
mod p486_relational_capsule_index_tests {
    use super::*;
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    fn capsule() -> (RelCausalCapsule, SemanticId) {
        let equivalence = SemanticId::new(0x4860);
        let relation = SemanticId::new(0x4861);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(equivalence, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(relation, Vec::new());
        let mut maintained =
            MaterializedRelPlanState::build(&RelExpr::Scan(relation), &model, &context, &registry)
                .unwrap();
        maintained.bind_revision(RevisionId::new(10)).unwrap();
        (RelCausalCapsule::capture(&maintained).unwrap(), relation)
    }

    #[test]
    fn p486_hot_relation_routes_compact_refs_to_one_interned_capsule() {
        let (capsule, relation) = capsule();
        let mut index = RuntimeHistoricalDerivedIndex::default();
        for observation_id in 0..1024u32 {
            index
                .index_relational_causal_capsule(
                    RevisionId::new(11),
                    0x4862 + u128::from(observation_id),
                    observation_id,
                    &capsule,
                )
                .unwrap();
        }
        assert_eq!(index.relational_capsules.len(), 1);
        assert_eq!(index.relational_capsule_keys.len(), 1);
        assert_eq!(index.relational_observations.len(), 1024);
        let timeline = index.relational_routes.get(&relation).unwrap();
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline.get(&RevisionId::new(11)).unwrap().len(), 1024);
    }
}

#[cfg(test)]
mod p488_relational_capsule_rebuild_scaling_tests {
    use super::*;
    use kernel_durability::DurableRelationalCausalObservation;
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    fn context() -> (
        SemanticContext,
        SemanticRegistry,
        SemanticId,
        SemanticId,
        SemanticId,
    ) {
        let equivalence = SemanticId::new(0x4880);
        let hot = SemanticId::new(0x4881);
        let cold = SemanticId::new(0x4882);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(0x4880));
        environment.pin_module(equivalence, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(0x4880));
        for relation in [hot, cold] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![equivalence],
                    },
                })
                .unwrap();
        }
        (
            SemanticContext {
                schema,
                environment,
            },
            registry,
            equivalence,
            hot,
            cold,
        )
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn p488_reopen_rebuild_sweeps_history_once_and_rewinds_only_semantically_affected_lineages() {
        let (context, registry, equivalence, hot, cold) = context();
        let query_count = 64usize;
        let history_depth = 128usize;
        let hot_stride = 32usize;
        let boundary = RevisionId::new(20_000);
        let oldest = RevisionId::new(boundary.raw() - history_depth as u64);

        let mut model = FiniteModel::default();
        let mut hot_rows = Vec::new();
        let mut cold_rows = Vec::new();
        let mut effects = Vec::with_capacity(history_depth);
        for offset in 0..history_depth {
            let target = RevisionId::new(boundary.raw() - offset as u64);
            let source = RevisionId::new(target.raw() - 1);
            let is_hot = offset % hot_stride == 0;
            let relation = if is_hot { hot } else { cold };
            let row = vec![Value::I64(100_000 + i64::try_from(offset).expect("fixture value fits i64"))];
            if is_hot {
                hot_rows.push(row.clone());
            } else {
                cold_rows.push(row.clone());
            }
            let relational_causal_observations = if offset == 0 {
                (0..query_count)
                    .map(|query| DurableRelationalCausalObservation {
                        observation_id: u32::try_from(query).expect("fixture observation id fits u32"),
                        observed_revision: oldest,
                        intent_prefix: kernel_durability::DurableIntentPrefix::empty(),
                        query: RelExpr::FilterEqConst {
                            input: Box::new(RelExpr::Scan(hot)),
                            column: 0,
                            value: Value::I64(i64::try_from(query).expect("fixture value fits i64")),
                            equivalence,
                        },
                    })
                    .collect()
            } else {
                Vec::new()
            };
            effects.push(RuntimeHistoryEffect {
                effect_id: 0x4880_0000 + offset as u128,
                prerequisites: Vec::new(),
                transaction_id: ClientTransactionId::new(0x4880_0000 + offset as u128),
                source_revision: source,
                target_revision: target,
                kind: RuntimeHistoryEffectKind::RelationData,
                reversibility: RuntimeHistoryReversibility::ExactPlanInverse,
                relation_mutations: vec![RuntimeHistoryRelationMutation {
                    relation,
                    inserted: vec![row],
                    removed: Vec::new(),
                    object_field_writes: Vec::new(),
                    authorization: kernel_durability::DurableRelationAuthorization::default(),
                }],
                model_delta: None,
                model_complement: None,
                semantic_change: None,
                schema_migration_program: None,
                causal_observations: Vec::new(),
                causal_observation_values: BTreeMap::new(),
                causal_observation_predicates: BTreeMap::new(),
                causal_observation_groups: Vec::new(),
                relational_causal_observations,
            });
        }
        model.relations.insert(hot, hot_rows);
        model.relations.insert(cold, cold_rows);

        let (capsules, stats) = reconstruct_relational_causal_capsules_with_stats(
            boundary,
            &model,
            &effects,
            &context,
            &registry,
        )
        .unwrap();
        let hot_effects = history_depth.div_ceil(hot_stride);
        assert_eq!(capsules.len(), query_count);
        assert_eq!(stats.unique_lineages, query_count);
        assert_eq!(stats.exact_effects_scanned, history_depth);
        assert_eq!(stats.effect_delta_maps_built, hot_effects);
        assert_eq!(stats.observation_routes, query_count);
        assert_eq!(stats.root_occurrences, query_count);
        assert_eq!(stats.unique_nodes, query_count + 1);
        assert_eq!(stats.fanout_edges, query_count);
        assert_eq!(stats.forest_rewinds, hot_effects);
        assert_eq!(
            stats.node_transitions, hot_effects,
            "P493 equality-family dispatch proves these hot rows miss every root without transitioning 64 stateless filter cells"
        );
        assert!(stats.node_transitions < (query_count * 2) * history_depth);

        let capsule_refs = capsules
            .iter()
            .map(|(_, _, _, capsule)| capsule)
            .collect::<Vec<_>>();
        let probe_delta = RelationDelta {
            inserted: vec![vec![Value::I64(7)]],
            removed: Vec::new(),
            result_type: RelExpr::Scan(hot).typecheck(&context, &registry).unwrap(),
        };
        let (_, plan_count) = RelCausalCapsule::impact_many_relation_deltas_with_stats(
            &capsule_refs,
            &BTreeMap::from([(hot, probe_delta)]),
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(
            plan_count, 1,
            "all reconstructed observation roots at one frontier must share one forest plan"
        );
    }
}
