impl DurableRuntimeSupervisor {
    pub fn create(
        root: RuntimeRevisionBundle,
        directory: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_recovery_policy(
            root,
            directory,
            PhysicalRecoveryPolicy::default(),
            registry,
        )
    }

    pub fn create_with_recovery_policy(
        root: RuntimeRevisionBundle,
        directory: impl AsRef<std::path::Path>,
        physical_recovery_policy: PhysicalRecoveryPolicy,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        let directory = directory.as_ref().to_path_buf();
        let runtime = DurableRuntime::create(root, &directory, registry)?;
        Ok(Self {
            directory,
            runtime: Mutex::new(Some(runtime)),
            physical_recovery_policy,
        })
    }

    pub fn open(directory: impl AsRef<std::path::Path>) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_recovery_policy(directory, PhysicalRecoveryPolicy::default())
            .map(|(supervisor, _)| supervisor)
    }

    pub fn open_with_recovery_policy(
        directory: impl AsRef<std::path::Path>,
        physical_recovery_policy: PhysicalRecoveryPolicy,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        let directory = directory.as_ref().to_path_buf();
        let (runtime, report) =
            DurableRuntime::open_with_recovery_policy(&directory, physical_recovery_policy)?;
        Ok((
            Self {
                directory,
                runtime: Mutex::new(Some(runtime)),
                physical_recovery_policy,
            },
            report,
        ))
    }

    fn reopen_locked(
        &self,
        slot: &mut Option<DurableRuntime>,
    ) -> Result<PhysicalRecoveryReport, RuntimeRecoveryError> {
        drop(slot.take());
        let (recovered, report) = DurableRuntime::open_with_recovery_policy(
            &self.directory,
            self.physical_recovery_policy,
        )?;
        *slot = Some(recovered);
        Ok(report)
    }

    /// The supervisor mutex protects reconstructible process state, not
    /// durable authority. If a panic poisons it, discard that runtime and
    /// rebuild from the published durable generation before serving again.
    fn lock_runtime_recovering(
        &self,
    ) -> Result<MutexGuard<'_, Option<DurableRuntime>>, RuntimeRecoveryError> {
        match self.runtime.lock() {
            Ok(guard) => Ok(guard),
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                self.reopen_locked(&mut guard)?;
                self.runtime.clear_poison();
                Ok(guard)
            }
        }
    }

    pub fn recover(&self) -> Result<(), RuntimeRecoveryError> {
        self.recover_with_report().map(|_| ())
    }

    pub fn recover_with_report(&self) -> Result<PhysicalRecoveryReport, RuntimeRecoveryError> {
        let mut slot = self.lock_runtime_recovering()?;
        self.reopen_locked(&mut slot)
    }

    pub fn resume_deferred_physical_recovery(
        &self,
        prior_report: &PhysicalRecoveryReport,
        policy: PhysicalRecoveryPolicy,
    ) -> Result<PhysicalRecoveryReport, RuntimeRecoveryError> {
        let slot = self.lock_runtime_recovering()?;
        slot.as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .resume_deferred_physical_recovery(prior_report, policy)
            .map_err(RuntimeRecoveryError::from)
    }

    pub fn snapshot(&self) -> Result<RuntimeRevisionSnapshot, RuntimeRecoveryError> {
        let mut slot = self.lock_runtime_recovering()?;
        let result = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .snapshot();
        match result {
            Ok(snapshot) => Ok(snapshot),
            Err(PhysicalExecutionError::RuntimeRecoveryRequired) => {
                let _ = self.reopen_locked(&mut slot)?;
                slot.as_ref()
                    .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
                    .snapshot()
                    .map_err(RuntimeRecoveryError::from)
            }
            Err(error) => Err(RuntimeRecoveryError::Runtime(error)),
        }
    }

    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let slot = self
            .lock_runtime_recovering()
            .map_err(|_| DurabilityError::Poisoned)?;
        slot.as_ref()
            .ok_or(DurabilityError::Poisoned)?
            .transaction_outcome(transaction_id)
    }

    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let slot = self
            .lock_runtime_recovering()
            .map_err(|_| DurabilityError::Poisoned)?;
        slot.as_ref()
            .ok_or(DurabilityError::Poisoned)?
            .transaction_outcome_at(epoch, transaction_id)
    }

    pub fn retry_horizon(&self) -> Result<(IdempotencyEpoch, IdempotencyEpoch), DurabilityError> {
        let slot = self
            .lock_runtime_recovering()
            .map_err(|_| DurabilityError::Poisoned)?;
        slot.as_ref()
            .ok_or(DurabilityError::Poisoned)?
            .retry_horizon()
    }

    pub fn commit_revision(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let mut slot = self
            .lock_runtime_recovering()
            .map_err(DurableRuntimeCommitError::Recovery)?;
        let first = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .commit_revision(transaction_id, request);
        let must_recover = matches!(
            first,
            Err(DurableRuntimeCommitError::CommitDurabilityUncertain(_)
                | DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::RuntimeRecoveryRequired
                ))
        );
        if !must_recover {
            return first;
        }

        let _ = self
            .reopen_locked(&mut slot)
            .map_err(DurableRuntimeCommitError::Recovery)?;
        let runtime = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?;
        runtime.commit_revision(transaction_id, request)
    }

    pub fn replace_revision(
        &self,
        transaction_id: ClientTransactionId,
        request: &FullRevisionTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let mut slot = self
            .lock_runtime_recovering()
            .map_err(DurableRuntimeCommitError::Recovery)?;
        let first = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .replace_revision(transaction_id, request);
        let must_recover = matches!(
            first,
            Err(DurableRuntimeCommitError::CommitDurabilityUncertain(_)
                | DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::RuntimeRecoveryRequired
                ))
        );
        if !must_recover {
            return first;
        }

        let _ = self
            .reopen_locked(&mut slot)
            .map_err(DurableRuntimeCommitError::Recovery)?;
        let runtime = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?;
        runtime.replace_revision(transaction_id, request)
    }

    pub fn replace_revision_and_materializations(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionAndMaterializationsTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let mut slot = self
            .lock_runtime_recovering()
            .map_err(DurableRuntimeCommitError::Recovery)?;
        let first = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .replace_revision_and_materializations(transaction_id, request);
        let must_recover = matches!(
            first,
            Err(DurableRuntimeCommitError::CommitDurabilityUncertain(_)
                | DurableRuntimeCommitError::Runtime(
                    PhysicalExecutionError::RuntimeRecoveryRequired
                ))
        );
        if !must_recover {
            return first;
        }

        let _ = self
            .reopen_locked(&mut slot)
            .map_err(DurableRuntimeCommitError::Recovery)?;
        slot.as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .replace_revision_and_materializations(transaction_id, request)
    }

    pub fn checkpoint(&self) -> Result<DurableGenerationReceipt, DurableRuntimeCheckpointError> {
        let mut slot = self
            .lock_runtime_recovering()
            .map_err(DurableRuntimeCheckpointError::Recovery)?;
        let first = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .checkpoint();
        if !matches!(
            first,
            Err(DurableRuntimeCheckpointError::Runtime(
                PhysicalExecutionError::RuntimeRecoveryRequired
            ))
        ) {
            return first;
        }
        self.reopen_locked(&mut slot)
            .map_err(DurableRuntimeCheckpointError::Recovery)?;
        slot.as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .checkpoint()
    }

    pub fn reconfigure_materializations(
        &self,
        specs: &[RuntimeMaterializationSpec],
    ) -> Result<DurableMaterializationConfigOutcome, DurableRuntimeCheckpointError> {
        let mut slot = self
            .lock_runtime_recovering()
            .map_err(DurableRuntimeCheckpointError::Recovery)?;
        let first = slot
            .as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .reconfigure_materializations(specs);
        if !matches!(
            first,
            Err(DurableRuntimeCheckpointError::Runtime(
                PhysicalExecutionError::RuntimeRecoveryRequired
            ) | DurableRuntimeCheckpointError::DurabilityUncertain(_))
        ) {
            return first;
        }
        self.reopen_locked(&mut slot)
            .map_err(DurableRuntimeCheckpointError::Recovery)?;
        slot.as_ref()
            .ok_or(PhysicalExecutionError::RuntimeRecoveryRequired)?
            .reconfigure_materializations(specs)
    }
}

