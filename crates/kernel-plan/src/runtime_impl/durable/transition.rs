impl PreparedRuntimeRevisionTransition {
    #[must_use]
    pub const fn descriptor(&self) -> &RevisionCommitDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.descriptor.source_revision
    }

    #[must_use]
    pub fn target_revision(&self) -> RevisionId {
        self.descriptor.target_revision()
    }

    /// Sparse maintained-output deltas. Relation-data transitions include only
    /// materializations whose base-relation dependency frontier intersects the
    /// changed relations; unaffected materializations are carried forward by the
    /// runtime bundle without rebinding or re-execution.
    #[must_use]
    pub const fn output_deltas(&self) -> &BTreeMap<kernel_types::MaterializationId, RelationDelta> {
        &self.output_deltas
    }

    /// Advances one revision-bound ordered materialization artifact from the exact sparse
    /// maintained-output delta already certified by this prepared runtime transition.
    /// No query execution, full output canonicalization, or full sort occurs here.
    pub fn advance_ordered_view(
        &self,
        snapshot: &OrderedViewSnapshot,
        materialization: kernel_types::MaterializationId,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<OrderedViewSnapshot, OrderedViewError> {
        if snapshot.revision != self.source_revision()
            || snapshot.semantic_context != *self.candidate.revision().semantic_context()
        {
            return Err(OrderedViewError::SnapshotBindingMismatch);
        }
        let maintained = self
            .candidate
            .materialization(materialization)
            .ok_or(OrderedViewError::UnknownMaterialization(materialization))?;
        if maintained.query() != &snapshot.logical {
            return Err(OrderedViewError::MaterializationQueryMismatch);
        }
        snapshot.advance_with_delta(
            self.target_revision(),
            self.output_deltas.get(&materialization),
            registry,
        )
    }

    #[must_use]
    pub const fn rewrite_intents(&self) -> &BTreeMap<SemanticId, RuntimeRewriteIntent> {
        self.descriptor.rewrite_intents()
    }

    /// Binds residual/cube coherence and exact VMF closure to this concrete
    /// single-relation prepared Rewrite candidate.
    pub fn bind_coherent_resolution<I: Clone + PartialEq + Eq>(
        self,
        relation: SemanticId,
        cube: RewriteResidualCubeCertificate<RelationValue, I>,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedCoherentResolutionTransition<I>, PhysicalExecutionError> {
        let invariant_closure =
            self.require_coherent_resolution_endpoint(relation, cube.common_endpoint())?;
        Ok(PreparedCoherentResolutionTransition {
            prepared: self,
            relation,
            cube,
            invariant_closure,
        })
    }

    fn require_coherent_resolution_endpoint(
        &self,
        relation: SemanticId,
        coherence_endpoint: &RelationValue,
    ) -> Result<RuntimeInvariantClosureCertificate, PhysicalExecutionError> {
        let relation_deltas = self
            .descriptor
            .relation_deltas()
            .ok_or(PhysicalExecutionError::ResolutionRequiresSingleRelationRewrite)?;
        if relation_deltas.len() != 1
            || !relation_deltas.contains_key(&relation)
            || self.descriptor.rewrite_intents().len() != 1
            || !self.descriptor.rewrite_intents().contains_key(&relation)
        {
            return Err(PhysicalExecutionError::ResolutionRequiresSingleRelationRewrite);
        }

        if !relation_value_matches_revision_relation(
            coherence_endpoint,
            &self.candidate.revision,
            relation,
        ) {
            return Err(PhysicalExecutionError::ResolutionCoherenceEndpointMismatch);
        }
        self.candidate.invariant_closure_certificate()
    }

    /// Performs the last fallible freshness check while acquiring the sole
    /// writer-publication guard. After this succeeds the live root cannot
    /// advance until `publish` or guard drop.
    pub fn seal(
        self,
        live: &RuntimeRevisionCell,
    ) -> Result<SealedRuntimeRevisionTransition<'_>, PhysicalExecutionError> {
        self.candidate
            .violation_state
            .require_bound_to(&self.candidate.revision)?;
        self.candidate.violation_state.require_zero()?;
        let guard = live
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(current) = &*guard else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        if current.root_identity != self.source_identity
            || current.revision.id() != self.descriptor.source_revision
        {
            return Err(PhysicalExecutionError::StalePreparedTransition);
        }
        Ok(SealedRuntimeRevisionTransition {
            live: guard,
            candidate: *self.candidate,
            descriptor: self.descriptor,
            output_deltas: self.output_deltas,
        })
    }
}

impl<I> PreparedCoherentResolutionTransition<I> {
    #[must_use]
    pub const fn relation(&self) -> SemanticId {
        self.relation
    }

    #[must_use]
    pub const fn cube(&self) -> &RewriteResidualCubeCertificate<RelationValue, I> {
        &self.cube
    }

    #[must_use]
    pub const fn invariant_closure(&self) -> RuntimeInvariantClosureCertificate {
        self.invariant_closure
    }

    #[must_use]
    pub fn target_revision(&self) -> RevisionId {
        self.prepared.target_revision()
    }

    pub fn seal(
        self,
        live: &RuntimeRevisionCell,
    ) -> Result<SealedRuntimeRevisionTransition<'_>, PhysicalExecutionError> {
        self.prepared.seal(live)
    }
}

impl SealedRuntimeRevisionTransition<'_> {
    #[must_use]
    pub const fn descriptor(&self) -> &RevisionCommitDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.descriptor.source_revision
    }

    #[must_use]
    pub fn target_revision(&self) -> RevisionId {
        self.descriptor.target_revision()
    }

    #[must_use]
    pub const fn output_deltas(&self) -> &BTreeMap<kernel_types::MaterializationId, RelationDelta> {
        &self.output_deltas
    }

    #[must_use]
    pub const fn rewrite_intents(&self) -> &BTreeMap<SemanticId, RuntimeRewriteIntent> {
        self.descriptor.rewrite_intents()
    }

    /// Infallible reader-visible publication. No semantic validation, freshness
    /// check, row lookup or index planning occurs after the seal boundary.
    #[must_use]
    pub fn publish(mut self) -> BTreeMap<kernel_types::MaterializationId, RelationDelta> {
        *self.live = RuntimeRevisionCellState::Serving(Arc::new(self.candidate));
        self.output_deltas
    }

    /// Fail-stops the runtime after an uncertain durable COMMIT outcome. The
    /// old root must not continue serving because recovery may discover that
    /// the target revision is already the durable head.
    fn require_recovery(mut self) {
        *self.live = RuntimeRevisionCellState::RecoveryRequired;
    }
}

