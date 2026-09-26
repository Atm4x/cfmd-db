impl PreparedMaterializationConfiguration {
    fn seal(
        self,
        live: &RuntimeRevisionCell,
    ) -> Result<SealedMaterializationConfiguration<'_>, PhysicalExecutionError> {
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
        if current.root_identity != self.source_identity {
            return Err(PhysicalExecutionError::StalePreparedTransition);
        }
        Ok(SealedMaterializationConfiguration {
            live: guard,
            candidate: *self.candidate,
        })
    }
}

impl SealedMaterializationConfiguration<'_> {
    #[must_use]
    fn durable_materialization_specs(&self) -> Vec<DurableMaterializationSpec> {
        self.candidate.durable_materialization_specs()
    }

    #[must_use]
    fn revision(&self) -> &kernel_revision::Revision {
        self.candidate.revision()
    }

    fn publish(mut self) {
        *self.live = RuntimeRevisionCellState::Serving(Arc::new(self.candidate));
    }

    fn require_recovery(mut self) {
        *self.live = RuntimeRevisionCellState::RecoveryRequired;
    }
}

