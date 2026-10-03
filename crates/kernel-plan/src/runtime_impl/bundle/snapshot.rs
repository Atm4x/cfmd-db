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
            let base = self.relation_bases.get(&mutation.relation).ok_or(
                PhysicalExecutionError::MissingRuntimeRelationBinding(mutation.relation),
            )?;
            candidate
                .patch_relation_delta_with_witness(
                    mutation.relation,
                    target_revision,
                    mutation.delta,
                    base,
                    mutation.validation_footprint(),
                    registry,
                )
                .map_err(RuntimeRevisionDerivationError::from)?;
        }
        candidate.build(target_revision, registry).map_err(Into::into)
    }
}
