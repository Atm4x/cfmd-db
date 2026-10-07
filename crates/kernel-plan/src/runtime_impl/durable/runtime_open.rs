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

    pub fn create_volatile(
        root: RuntimeRevisionBundle,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_volatile_with_revision_publication_notifier(
            root,
            registry,
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
    }

    pub fn create_volatile_with_revision_publication_notifier(
        mut root: RuntimeRevisionBundle,
        registry: &kernel_semantics::SemanticRegistry,
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
                    reason: "failed to derive volatile artifact core",
                })?;
        let authority = DurableRevisionStore::create_volatile_with_materializations_physical_artifacts_and_cores(
            root.revision(),
            &materialization_specs,
            &physical_artifact_specs,
            &artifact_cores,
            registry,
        )?;
        Ok(Self {
            cell: RuntimeRevisionCell::new(root),
            durability: Mutex::new(authority),
            registry: registry.clone(),
            revision_publication,
        })
    }

    /// Promotes the process-local authority into a verified single-file store.
    /// All I/O and reopen verification complete before the owner swap; once
    /// `staged` exists, replacing the single persistence owner is infallible
    /// and performs no I/O or semantic publication.
    pub fn promote_volatile_to_single_file(
        &self,
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
    ) -> Result<(), DurabilityError> {
        let mut authority = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        if !authority.is_volatile() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "runtime persistence authority is already durable",
            });
        }
        let snapshot = self
            .cell
            .snapshot()
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "runtime snapshot unavailable during persistence promotion",
            })?;
        if authority.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile persistence authority is not aligned with runtime head",
            });
        }
        let staged = authority.repersist_volatile_to_single_file(
            snapshot.revision(),
            path,
            encryption,
        )?;
        if staged.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "promoted persistence authority changed runtime revision",
            });
        }
        *authority = staged;
        Ok(())
    }

    /// Retires the current recoverable physical owner while retaining the same
    /// live semantic/runtime lineage in volatile form.
    pub fn demote_durable_to_volatile(&self) -> Result<(), DurabilityError> {
        let snapshot = self.cell.snapshot().map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "runtime snapshot unavailable during persistence demotion",
        })?;
        let mut authority = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        if authority.is_volatile() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "runtime persistence authority is already volatile",
            });
        }
        if authority.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable persistence authority is not aligned with runtime head",
            });
        }
        authority.demote_to_volatile()
    }

    /// Writes a quiescent, self-contained single-file backup from the exact
    /// runtime authority cut. This is representation-only: no semantic
    /// revision is published and the live owner is not replaced.
    ///
    /// External freshness is intentionally fail-closed here: duplicating a
    /// live freshness authority would create two stores claiming the same
    /// monotonic trust cut. Disaster-recovery rebind requires a separate
    /// authority-transfer operation.
    pub fn backup_to_single_file(
        &self,
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
    ) -> Result<RevisionId, DurabilityError> {
        let snapshot = self
            .cell
            .snapshot()
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "runtime snapshot unavailable during backup",
            })?;
        let mut authority = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        if authority.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence authority is not aligned with runtime head",
            });
        }
        let image = authority.canonical_persistence_image(snapshot.revision())?;
        let staged = DurableRevisionStore::stage_single_file_from_persistence_image(
            path, encryption, &image,
        )?;
        Ok(staged.durable_head())
    }

    /// Creates a second independently live single-file database from the current semantic/history
    /// cut. Retry/prepared/replication/freshness authority is not copied; the target store bootstraps
    /// fresh operational roots before the file is reopened through the ordinary recovery path.
    pub fn fork_to_single_file(
        &self,
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
    ) -> Result<Self, RuntimeRecoveryError> {
        let snapshot = self.cell.snapshot().map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "runtime snapshot unavailable during live fork",
        })?;
        let mut authority = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        if authority.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence authority is not aligned with runtime head",
            }
            .into());
        }
        let image = authority.fork_persistence_image(snapshot.revision())?;
        let staged = DurableRevisionStore::stage_single_file_from_fork_image(
            path.as_ref(),
            encryption,
            &image,
        )?;
        if staged.durable_head() != snapshot.revision().id() {
            return Err(RuntimeRecoveryError::DurableHeadMismatch);
        }
        drop(staged);
        drop(authority);

        let storage = RuntimeStorageOptions::new(RuntimeDurabilityBackend::SingleFile)
            .with_encryption(encryption.clone());
        let fork = Self::open_with_storage_options(path, &storage)?;
        if fork.snapshot()?.revision().id() != snapshot.revision().id() {
            return Err(RuntimeRecoveryError::DurableHeadMismatch);
        }
        Ok(fork)
    }

    /// Creates a second independently live single-file database whose first recoverable target
    /// generation is already bound to a newly bootstrapped external-freshness root. The source
    /// freshness authority, when present, is neither copied nor rebound.
    pub fn fork_to_single_file_with_external_freshness(
        &self,
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
        target_config: kernel_durability::ExternalFreshnessConfig,
        authority: Box<dyn kernel_durability::ExternalFreshnessAuthority>,
    ) -> Result<Self, RuntimeRecoveryError> {
        let snapshot = self.cell.snapshot().map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "runtime snapshot unavailable during live fork",
        })?;
        let mut source = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        if source.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence authority is not aligned with runtime head",
            }
            .into());
        }
        let image = source.fork_persistence_image(snapshot.revision())?;
        let (staged, scan) =
            DurableRevisionStore::stage_single_file_from_fork_image_with_external_freshness_bootstrap(
                path.as_ref(),
                encryption,
                &image,
                target_config,
                authority,
            )?;
        if staged.durable_head() != snapshot.revision().id() {
            return Err(RuntimeRecoveryError::DurableHeadMismatch);
        }
        drop(source);

        Self::recover_opened_durability(
            staged,
            &scan,
            PhysicalRecoveryPolicy::default(),
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
        .map(|(runtime, _)| runtime)
    }

    pub fn verify_single_file_backup(
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
    ) -> Result<RevisionId, DurabilityError> {
        DurableRevisionStore::verify_single_file_backup(path, encryption)
    }

    pub fn restore_single_file_backup(
        backup_path: impl AsRef<std::path::Path>,
        backup_encryption: &kernel_durability::StorageEncryption,
        target_path: impl AsRef<std::path::Path>,
        target_encryption: &kernel_durability::StorageEncryption,
    ) -> Result<RevisionId, DurabilityError> {
        let restored = DurableRevisionStore::restore_single_file_backup(
            backup_path,
            backup_encryption,
            target_path,
            target_encryption,
        )?;
        Ok(restored.durable_head())
    }

    #[must_use]
    pub fn is_volatile_persistence(&self) -> bool {
        self.durability
            .lock()
            .is_ok_and(|authority| authority.is_volatile())
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
        let (durability, scan) = match storage.backend {
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
        Self::recover_opened_durability(
            durability,
            &scan,
            physical_recovery_policy,
            revision_publication,
        )
    }

    pub fn open_single_file_with_external_freshness_and_encryption(
        path: impl AsRef<std::path::Path>,
        config: kernel_durability::ExternalFreshnessConfig,
        authority: Box<dyn kernel_durability::ExternalFreshnessAuthority>,
        encryption: &kernel_durability::StorageEncryption,
    ) -> Result<Self, RuntimeRecoveryError> {
        Self::open_single_file_with_external_freshness_and_encryption_and_revision_publication_notifier(
            path,
            config,
            authority,
            encryption,
            Arc::new(InProcessRevisionPublicationNotifier::default()),
        )
    }

    pub fn open_single_file_with_external_freshness_and_encryption_and_revision_publication_notifier(
        path: impl AsRef<std::path::Path>,
        config: kernel_durability::ExternalFreshnessConfig,
        authority: Box<dyn kernel_durability::ExternalFreshnessAuthority>,
        encryption: &kernel_durability::StorageEncryption,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<Self, RuntimeRecoveryError> {
        let (durability, scan) =
            DurableRevisionStore::open_single_file_with_external_freshness_and_encryption(
                path, config, authority, encryption,
            )?;
        Self::recover_opened_durability(
            durability,
            &scan,
            PhysicalRecoveryPolicy::default(),
            revision_publication,
        )
        .map(|(runtime, _)| runtime)
    }

    fn recover_opened_durability(
        mut durability: DurableRevisionStore,
        scan: &kernel_durability::RecoveryScan,
        physical_recovery_policy: PhysicalRecoveryPolicy,
        revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
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
            scan,
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
                        effect_id: record.id.0,
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

    pub fn transfer_external_freshness_to_single_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
        encryption: &kernel_durability::StorageEncryption,
        target_config: kernel_durability::ExternalFreshnessConfig,
        authority: Box<dyn kernel_durability::ExternalFreshnessAuthority>,
    ) -> Result<(), DurabilityError> {
        let snapshot = self.cell.snapshot().map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "runtime snapshot unavailable during authority transfer",
        })?;
        let persistence = self.durability.get_mut().map_err(|_| DurabilityError::Poisoned)?;
        if persistence.is_volatile() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "authority transfer requires durable source persistence",
            });
        }
        if persistence.durable_head() != snapshot.revision().id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence authority is not aligned with runtime head",
            });
        }
        let target = persistence.transfer_external_freshness_to_single_file(
            snapshot.revision(),
            path,
            encryption,
            target_config,
            authority,
        )?;
        *persistence = target;
        Ok(())
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
