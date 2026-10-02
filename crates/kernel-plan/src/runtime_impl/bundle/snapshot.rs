impl RuntimeRevisionSnapshot {
    #[must_use]
    pub fn root(&self) -> &RuntimeRevisionBundle {
        self.root.as_ref()
    }

    #[must_use]
    pub fn root_version(&self) -> RuntimeRootVersion {
        self.root.root_identity.version
    }
}

impl std::ops::Deref for RuntimeRevisionSnapshot {
    type Target = RuntimeRevisionBundle;

    fn deref(&self) -> &Self::Target {
        self.root.as_ref()
    }
}



impl RuntimeRevisionSnapshot {
    /// Derives the exact logical target produced by applying normalized relation deltas to this
    /// immutable source snapshot. This is read-only authority shared by product preview and the
    /// durable derived-relation commit path, so preview and publication cannot disagree about the
    /// logical endpoint.
    #[allow(clippy::too_many_lines, reason = "Keep the complete operator or protocol case analysis together.")]
    pub fn derive_relation_target_revision(
        &self,
        target_revision: RevisionId,
        mutations: &[RevisionRelationMutation<'_>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_revision::Revision, RuntimeRevisionDerivationError> {
        let source = self.revision();
        let mut seen = BTreeSet::new();
        for mutation in mutations {
            if !seen.insert(mutation.relation) {
                return Err(PhysicalExecutionError::DuplicateRelationMutation(mutation.relation).into());
            }
        }
        let append_only_bag_fast_path = mutations.iter().all(|mutation| {
            mutation.delta.removed.is_empty()
                && source
                    .semantic_context()
                    .schema
                    .relation(mutation.relation)
                    .is_some_and(|definition| {
                        matches!(definition.semantics, kernel_schema::RelationSemantics::Bag { .. })
                    })
                && !source
                    .live_ref_sensitivity()
                    .relation_has_live_refs(mutation.relation)
                && !mutation
                    .delta
                    .inserted
                    .iter()
                    .flatten()
                    .any(Value::contains_live_ref)
        });
        if append_only_bag_fast_path {
            let appends = mutations
                .iter()
                .map(|mutation| (mutation.relation, mutation.delta.inserted.clone()))
                .collect::<Vec<_>>();
            return kernel_revision::Revision::build_append_only_bag_relations(
                target_revision,
                source,
                registry,
                &appends,
            )
            .map_err(Into::into);
        }

        let mut candidate = source.relation_update_candidate();
        let live = &source.state().lifecycle.entities;
        for mutation in mutations {
            if mutation.delta.inserted.iter().any(|row| {
                row.iter()
                    .any(|value| value.first_dangling_live_ref(live).is_some())
            }) {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch.into());
            }
            let definition = source
                .semantic_context()
                .schema
                .relation(mutation.relation)
                .ok_or(PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation))?;
            let source_rows = source
                .state()
                .model
                .relations
                .materialize_owned(&mutation.relation)
                .unwrap_or_default();
            let old = relation_value_from_rows(
                source_rows.clone(),
                &RelType {
                    columns: definition.columns.clone(),
                    semantics: definition.semantics.clone(),
                },
                source.semantic_context(),
                registry,
            )?;
            let next = mutation
                .delta
                .apply_to_value(old, source.semantic_context(), registry)
                .map_err(PhysicalExecutionError::from)?;
            let next_rows = next.into_rows();
            let survivor_count = next_rows
                .len()
                .checked_sub(mutation.delta.inserted.len())
                .ok_or(PhysicalExecutionError::LogicalRevisionMutationMismatch)?;
            let survivors = &next_rows[..survivor_count];
            let mut survivor = 0_usize;
            let mut removed_positions = Vec::with_capacity(mutation.delta.removed.len());
            for (position, row) in source_rows.iter().enumerate() {
                if survivor < survivors.len() && row == &survivors[survivor] {
                    survivor += 1;
                } else {
                    removed_positions.push(position);
                }
            }
            if survivor != survivors.len()
                || removed_positions.len() != mutation.delta.removed.len()
            {
                return Err(PhysicalExecutionError::LogicalRevisionMutationMismatch.into());
            }
            candidate
                .patch_relation_rows(
                    mutation.relation,
                    &removed_positions,
                    mutation.delta.inserted.clone(),
                )
                .map_err(RuntimeRevisionDerivationError::from)?;
        }
        candidate.build(target_revision, registry).map_err(Into::into)
    }
}
