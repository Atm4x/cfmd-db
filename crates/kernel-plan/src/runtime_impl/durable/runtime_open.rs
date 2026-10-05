impl DurableRuntime {
    fn canonical_durable_relation_mutations(
        mutations: &[RevisionRelationMutation<'_>],
    ) -> Result<Vec<DurableRelationMutation>, PhysicalExecutionError> {
        let mut seen = BTreeSet::new();
        let mut durable = Vec::with_capacity(mutations.len());
        for mutation in mutations {
            if !seen.insert(mutation.relation) {
                return Err(PhysicalExecutionError::DuplicateRelationMutation(
                    mutation.relation,
                ));
            }
            durable.push(DurableRelationMutation {
                relation: mutation.relation,
                inserted: mutation.delta.inserted.clone(),
                removed: mutation.delta.removed.clone(),
                object_field_writes: mutation.object_field_writes.to_vec(),
                authorization: mutation.authorization,
            });
        }
        durable.sort_by_key(|mutation| mutation.relation);
        Ok(durable)
    }

    fn canonical_durable_relation_rewrites<I>(
        rewrites: &[RevisionRelationRewrite<'_, I>],
    ) -> Result<
        (
            Vec<DurableRelationMutation>,
            Vec<DurableRelationRewriteIntent>,
        ),
        PhysicalExecutionError,
    > {
        let mut seen = BTreeSet::new();
        let mut mutations = Vec::with_capacity(rewrites.len());
        let mut intents = Vec::with_capacity(rewrites.len());
        for rewrite in rewrites {
            if !seen.insert(rewrite.relation) {
                return Err(PhysicalExecutionError::DuplicateRelationMutation(
                    rewrite.relation,
                ));
            }
            mutations.push(DurableRelationMutation {
                relation: rewrite.relation,
                inserted: rewrite.rewrite.delta().inserted.clone(),
                removed: rewrite.rewrite.delta().removed.clone(),
                object_field_writes: Vec::new(),
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            });
            intents.push(DurableRelationRewriteIntent {
                relation: rewrite.relation,
                rewrite_spec: rewrite.rewrite.rewrite().spec().0,
                law_set: rewrite.rewrite.rewrite().law_set().0,
            });
        }
        mutations.sort_by_key(|mutation| mutation.relation);
        intents.sort_by_key(|intent| intent.relation);
        Ok((mutations, intents))
    }

    fn derive_relation_target(
        &self,
        source: &kernel_revision::Revision,
        target_revision: RevisionId,
        mutations: &[RevisionRelationMutation<'_>],
    ) -> Result<DerivedRelationEndpoint, DurableRuntimeCommitError> {
        let snapshot = self.cell.snapshot()?;
        if snapshot.revision() != source {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let revision = snapshot
            .derive_relation_target_revision(target_revision, mutations, &self.registry)
            .map_err(|error| match error {
                RuntimeRevisionDerivationError::Runtime(error) => {
                    DurableRuntimeCommitError::from(error)
                }
                RuntimeRevisionDerivationError::Revision(error) => {
                    DurableRuntimeCommitError::Recovery(RuntimeRecoveryError::Revision(error))
                }
            })?;
        let mut exact_deltas = BTreeMap::new();
        for mutation in mutations {
            exact_deltas.insert(mutation.relation, mutation.delta.clone());
        }
        Ok(DerivedRelationEndpoint {
            revision,
            exact_deltas,
        })
    }

    pub fn create(
        root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_backend(root, path, registry, RuntimeDurabilityBackend::Directory)
    }

    pub fn create_with_backend(
        root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
        backend: RuntimeDurabilityBackend,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_backend_and_revision_publication_notifier(
            root,
            path,
            registry,
            backend,
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
    }

    pub fn create_with_storage_options(
        root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
        storage: &RuntimeStorageOptions,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_storage_options_and_revision_publication_notifier(
            root,
            path,
            registry,
            storage,
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
    }

    pub fn create_with_revision_publication_notifier(
        root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_backend_and_revision_publication_notifier(
            root,
            path,
            registry,
            RuntimeDurabilityBackend::Directory,
            revision_publication,
        )
    }

    pub fn create_with_backend_and_revision_publication_notifier(
        root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
        backend: RuntimeDurabilityBackend,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_storage_options_and_revision_publication_notifier(
            root,
            path,
            registry,
            &RuntimeStorageOptions::new(backend),
            revision_publication,
        )
    }

    pub fn create_with_storage_options_and_revision_publication_notifier(
        mut root: RuntimeRevisionBundle,
        path: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
        storage: &RuntimeStorageOptions,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<Self, DurabilityError> {
        root.historical =
            RuntimeHistoricalDerivedIndex::from_current(root.revision.id(), &root.relation_bases);
        let materialization_specs = root.durable_materialization_specs();
        let physical_artifact_specs = root.durable_physical_artifact_specs();
        let artifact_cores =
            root.durable_artifact_cores()
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "failed to derive durable artifact core",
                })?;
        let durability = match storage.backend {
            RuntimeDurabilityBackend::SingleFile =>
                DurableRevisionStore::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
                    path,
                    &storage.encryption,
                    root.revision(),
                    &materialization_specs,
                    &physical_artifact_specs,
                    &artifact_cores,
                    registry,
                )?,
            RuntimeDurabilityBackend::Directory => {
                if storage.encryption.algorithm().is_some() {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "directory storage encryption is not implemented",
                    });
                }
                DurableRevisionStore::create_with_materializations_physical_artifacts_and_cores(
                    path,
                    root.revision(),
                    &materialization_specs,
                    &physical_artifact_specs,
                    &artifact_cores,
                    registry,
                )?
            }
        };
        Ok(Self {
            cell: RuntimeRevisionCell::new(root),
            durability: Mutex::new(durability),
            registry: registry.clone(),
            revision_publication,
        })
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_backend(path, RuntimeDurabilityBackend::Directory)
    }

    pub fn open_with_backend(
        path: impl AsRef<std::path::Path>,
        backend: RuntimeDurabilityBackend,
    ) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_storage_options_recovery_policy_and_revision_publication_notifier(
            path,
            &RuntimeStorageOptions::new(backend),
            PhysicalRecoveryPolicy::default(),
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
        .map(|(runtime, _)| runtime)
    }

    pub fn open_with_storage_options(
        path: impl AsRef<std::path::Path>,
        storage: &RuntimeStorageOptions,
    ) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_storage_options_recovery_policy_and_revision_publication_notifier(
            path,
            storage,
            PhysicalRecoveryPolicy::default(),
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
        .map(|(runtime, _)| runtime)
    }

    pub fn open_with_revision_publication_notifier(
        directory: impl AsRef<std::path::Path>,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_recovery_policy_and_revision_publication_notifier(
            directory,
            PhysicalRecoveryPolicy::default(),
            revision_publication,
        )
        .map(|(runtime, _)| runtime)
    }

    pub fn open_with_recovery_policy(
        directory: impl AsRef<std::path::Path>,
        physical_recovery_policy: PhysicalRecoveryPolicy,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        Self::open_with_recovery_policy_and_revision_publication_notifier(
            directory,
            physical_recovery_policy,
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
    }

    pub fn open_with_recovery_policy_and_revision_publication_notifier(
        path: impl AsRef<std::path::Path>,
        physical_recovery_policy: PhysicalRecoveryPolicy,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        Self::open_with_backend_recovery_policy_and_revision_publication_notifier(
            path,
            RuntimeDurabilityBackend::Directory,
            physical_recovery_policy,
            revision_publication,
        )
    }

    pub fn open_with_backend_recovery_policy_and_revision_publication_notifier(
        path: impl AsRef<std::path::Path>,
        backend: RuntimeDurabilityBackend,
        physical_recovery_policy: PhysicalRecoveryPolicy,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        Self::open_with_storage_options_recovery_policy_and_revision_publication_notifier(
            path,
            &RuntimeStorageOptions::new(backend),
            physical_recovery_policy,
            revision_publication,
        )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub fn open_with_storage_options_recovery_policy_and_revision_publication_notifier(
        path: impl AsRef<std::path::Path>,
        storage: &RuntimeStorageOptions,
        physical_recovery_policy: PhysicalRecoveryPolicy,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        let (mut durability, scan) = match storage.backend {
            RuntimeDurabilityBackend::SingleFile => {
                DurableRevisionStore::open_single_file_with_encryption(path, &storage.encryption)?
            }
            RuntimeDurabilityBackend::Directory => {
                if storage.encryption.algorithm().is_some() {
                    return Err(RuntimeRecoveryError::Durability(
                        DurabilityError::Protocol {
                            offset: 0,
                            reason: "directory storage encryption is not implemented",
                        },
                    ));
                }
                DurableRevisionStore::open(path)?
            }
        };
        let registry = durability.semantic_registry().clone();
        let materialization_specs = durability
            .materialization_specs()
            .iter()
            .map(|spec| RuntimeMaterializationSpec {
                id: spec.id,
                query: spec.query.clone(),
            })
            .collect::<Vec<_>>();
        let physical_artifact_specs = durability.physical_artifact_specs().to_vec();
        let artifact_cores = durability.artifact_cores().to_vec();
        let (mut root, recovery_report) = recover_runtime_bundle_with_policy_and_cores(
            durability.checkpoint_revision(),
            &scan,
            &materialization_specs,
            &physical_artifact_specs,
            &artifact_cores,
            physical_recovery_policy,
            &registry,
        )?;
        let durable_head = durability.durable_head();
        let causal_floor = durability.causal_coverage_root();
        if let Some(records) =
            durability.revision_transition_records_back_to(causal_floor, durable_head)?
        {
            let effects = records
                .iter()
                .map(RuntimeHistoryEffect::from_durable)
                .collect::<Vec<_>>();
            root.rebuild_historical_derived_index(&effects, &registry)?;

            let mut epoch_effects = Vec::<RuntimeHistoryEffect>::new();
            for record in records.iter().rev() {
                let DurableTransactionIntent::SchemaMigration { program, .. } = &record.intent
                else {
                    epoch_effects.push(RuntimeHistoryEffect::from_durable(record));
                    continue;
                };
                let source = if let Some(source) =
                    durability.historical_revision_from_realization(record.id)?
                {
                    source
                } else {
                    let material = durability
                        .historical_epoch_material(record.id)?
                        .ok_or(RuntimeRecoveryError::BaseRevisionMismatch)?;
                    crate::replay_durable_revisions_until(
                        material.checkpoint(),
                        material.recovery_scan(),
                        material.semantic_registry(),
                        record.source_revision,
                    )?
                };
                if source.id() != record.source_revision {
                    return Err(RuntimeRecoveryError::BaseRevisionMismatch);
                }
                let relation_bases =
                    RuntimeRevisionBundle::relation_base_witnesses(&source, &registry)?;
                let effects_backwards = epoch_effects.iter().rev().cloned().collect::<Vec<_>>();
                let index = RuntimeRetainedEpochIndex::rebuild_from_boundary(
                    record.source_revision,
                    &source.state().model,
                    &relation_bases,
                    &effects_backwards,
                    source.semantic_context(),
                    &registry,
                )?;
                root.historical.retained_schema_epochs.insert(
                    record.source_revision,
                    RuntimeRetainedSchemaEpoch {
                        source_revision: record.source_revision,
                        target_revision: record.target_revision,
                        source_context: source.semantic_context().clone(),
                        source_fields: source.state().model.fields.clone(),
                        program: program.clone(),
                        index,
                    },
                );
                epoch_effects.clear();
            }
        }
        if root.revision().id() != durable_head {
            return Err(RuntimeRecoveryError::DurableHeadMismatch);
        }
        Ok((
            Self {
                cell: RuntimeRevisionCell::new(root),
                durability: Mutex::new(durability),
                registry,
                revision_publication,
            },
            recovery_report,
        ))
    }

    #[must_use]
    pub const fn semantic_registry(&self) -> &kernel_semantics::SemanticRegistry {
        &self.registry
    }

    pub fn snapshot(&self) -> Result<RuntimeRevisionSnapshot, PhysicalExecutionError> {
        self.cell.snapshot()
    }

    /// Retries advisor-owned physical recipes that a prior bounded reopen left
    /// deferred. Logical Revision and durable authority remain unchanged; any
    /// accepted artifacts are published as a new immutable runtime root.
    pub fn resume_deferred_physical_recovery(
        &self,
        prior_report: &PhysicalRecoveryReport,
        policy: PhysicalRecoveryPolicy,
    ) -> Result<PhysicalRecoveryReport, PhysicalExecutionError> {
        let deferred = prior_report.deferred_advisor_artifacts();
        self.cell
            .resume_deferred_physical_recovery(&deferred, policy, None, &self.registry)
    }

    pub(super) fn resume_deferred_physical_recovery_with_telemetry(
        &self,
        prior_report: &PhysicalRecoveryReport,
        policy: PhysicalRecoveryPolicy,
        telemetry: &UnifiedAdvisorTelemetry,
    ) -> Result<PhysicalRecoveryReport, PhysicalExecutionError> {
        let deferred = prior_report.deferred_advisor_artifacts();
        self.cell.resume_deferred_physical_recovery(
            &deferred,
            policy,
            Some(telemetry),
            &self.registry,
        )
    }
}
