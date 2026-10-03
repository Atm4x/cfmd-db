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
        let mut timeline = self.relation_supports.get(&relation).cloned().unwrap_or_default();
        timeline.insert(revision, support);
        self.relation_supports.insert(relation, timeline);
    }

    fn support_at(&self, revision: RevisionId, relation: SemanticId) -> Option<&RelationSupportWitness> {
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

    #[allow(clippy::too_many_arguments, reason = "Keep explicit semantic and durability inputs at this boundary.")]
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
            writes: self.writes.clone(),
            exact_effects: self.exact_effects.clone(),
        };
        self.retained_schema_epochs.insert(
            source_revision,
            RuntimeRetainedSchemaEpoch {
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
            timeline.insert(target_revision, RuntimeIndexedHistoryAction { effect_id, action });
            self.writes.insert(coordinate, timeline);
        }
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
            let Some(key) = timeline.key_at_rank(rank) else { break };
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
    fn rebuild_from_boundary(
        boundary_revision: RevisionId,
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
            out.exact_effects.insert(effect.target_revision, effect.effect_id);
            for (coordinate, action) in footprint.writes {
                let mut timeline = out.writes.get(&coordinate).cloned().unwrap_or_default();
                timeline.insert(
                    effect.target_revision,
                    RuntimeIndexedHistoryAction { effect_id: effect.effect_id, action },
                );
                out.writes.insert(coordinate, timeline);
            }
            for mutation in &effect.relation_mutations {
                let support = supports.get(&mutation.relation).ok_or(
                    PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation),
                )?;
                let transition = RelationDelta {
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    result_type: support.result_type().clone(),
                };
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
            let mut timeline = out.relation_supports.get(&relation).cloned().unwrap_or_default();
            timeline.insert(floor, support);
            out.relation_supports.insert(relation, timeline);
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
            self.historical.reset_at(target_revision, Some(effect_id), &self.relation_bases);
            return Ok(());
        };
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
            let delta_footprint = delta.rewrite_footprint(
                relation,
                self.revision.semantic_context(),
                registry,
            )?;
            for (coordinate, action) in delta_footprint.writes {
                let kernel_change::SemanticWriteCoordinate::RelationClass { relation, canonical_key } = coordinate else {
                    return Err(PhysicalExecutionError::InvalidRevisionTransition);
                };
                insert_runtime_history_action(
                    &mut footprint.writes,
                    RuntimeHistoryCoordinate::RelationClass { relation, canonical_key },
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
        self.historical.index_exact_effect(target_revision, effect_id, footprint);
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
        self.candidate.historical.seal_schema_epoch(
            source_revision,
            target_revision,
            effect_id,
            self.source_context.clone(),
            self.source_fields.clone(),
            program.clone(),
            &self.candidate.relation_bases,
        );
        self
    }

    fn bind_committed_history_effect(
        mut self,
        effect_id: u128,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        self.candidate.bind_committed_history_effect(effect_id, &self.descriptor, registry)?;
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
        let mut index = RuntimeHistoricalDerivedIndex::from_current(head_revision, &self.relation_bases);
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

            for mutation in &effect.relation_mutations {
                let support = supports
                    .get(&mutation.relation)
                    .ok_or(PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation))?;
                let transition = RelationDelta {
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    result_type: support.result_type().clone(),
                };
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
        index.lineage_floor = Some(floor);
        self.historical = index;
        Ok(())
    }
}
