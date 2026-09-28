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
        let materialization_specs = root.durable_materialization_specs();
        let physical_artifact_specs = root.durable_physical_artifact_specs();
        let artifact_cores =
            root.durable_artifact_cores()
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "failed to derive durable artifact core",
                })?;
        let durability = match backend {
            RuntimeDurabilityBackend::SingleFile =>
                DurableRevisionStore::create_single_file_with_materializations_physical_artifacts_and_cores(
                    path,
                    root.revision(),
                    &materialization_specs,
                    &physical_artifact_specs,
                    &artifact_cores,
                    registry,
                )?,
            RuntimeDurabilityBackend::Directory =>
                DurableRevisionStore::create_with_materializations_physical_artifacts_and_cores(
                    path,
                    root.revision(),
                    &materialization_specs,
                    &physical_artifact_specs,
                    &artifact_cores,
                    registry,
                )?,
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
        Self::open_with_backend_recovery_policy_and_revision_publication_notifier(
            path,
            backend,
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
        let (durability, scan) = match backend {
            RuntimeDurabilityBackend::SingleFile => DurableRevisionStore::open_single_file(path)?,
            RuntimeDurabilityBackend::Directory => DurableRevisionStore::open(path)?,
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
        let (root, recovery_report) = recover_runtime_bundle_with_policy_and_cores(
            durability.checkpoint_revision(),
            &scan,
            &materialization_specs,
            &physical_artifact_specs,
            &artifact_cores,
            physical_recovery_policy,
            &registry,
        )?;
        if root.revision().id() != durability.durable_head() {
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
