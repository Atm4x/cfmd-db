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
                inserted: rewrite.rewrite.delta.inserted.clone(),
                removed: rewrite.rewrite.delta.removed.clone(),
            });
            intents.push(DurableRelationRewriteIntent {
                relation: rewrite.relation,
                rewrite_spec: rewrite.rewrite.rewrite.spec.0,
                law_set: rewrite.rewrite.rewrite.law_set.0,
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
        let mut exact_deltas = BTreeMap::new();
        for mutation in mutations {
            if exact_deltas
                .insert(mutation.relation, mutation.delta.clone())
                .is_some()
            {
                return Err(
                    PhysicalExecutionError::DuplicateRelationMutation(mutation.relation).into(),
                );
            }
        }
        let append_only_bag_fast_path = mutations.iter().all(|mutation| {
            mutation.delta.removed.is_empty()
                && source
                    .semantic_context()
                    .schema
                    .relation(mutation.relation)
                    .is_some_and(|definition| {
                        matches!(
                            definition.semantics,
                            kernel_schema::RelationSemantics::Bag { .. }
                        )
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
            let revision = kernel_revision::Revision::build_append_only_bag_relations(
                target_revision,
                source,
                &self.registry,
                &appends,
            )
            .map_err(|error| {
                DurableRuntimeCommitError::Recovery(RuntimeRecoveryError::Revision(error))
            })?;
            return Ok(DerivedRelationEndpoint {
                revision,
                exact_deltas,
            });
        }

        let mut candidate = source.relation_update_candidate();
        let live = &source.state().lifecycle.entities;
        for mutation in mutations {
            // RelationUpdateCandidate::build performs one relation-only lifecycle
            // normalization step: rows containing dangling live references are
            // removed.  A derived target is allowed to bypass the later full
            // endpoint replay only when that normalization is provably the
            // identity.  Because the authoritative source is already normalized,
            // checking the inserted support is sufficient: removals cannot create
            // a dangling reference and untouched source rows were valid already.
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
                .ok_or(PhysicalExecutionError::MissingRuntimeRelationBinding(
                    mutation.relation,
                ))?;
            let rows = source
                .state()
                .model
                .relations
                .get(&mutation.relation)
                .cloned()
                .unwrap_or_default();
            let old = relation_value_from_rows(
                rows,
                &RelType {
                    columns: definition.columns.clone(),
                    semantics: definition.semantics.clone(),
                },
                source.semantic_context(),
                &self.registry,
            )?;
            let next = mutation
                .delta
                .apply_to_value(old, source.semantic_context(), &self.registry)
                .map_err(PhysicalExecutionError::from)?;
            candidate.replace_relation_rows(mutation.relation, next.into_rows());
        }
        let revision = candidate
            .build(target_revision, &self.registry)
            .map_err(|error| {
                DurableRuntimeCommitError::Recovery(RuntimeRecoveryError::Revision(error))
            })?;
        Ok(DerivedRelationEndpoint {
            revision,
            exact_deltas,
        })
    }

    pub fn create(
        root: RuntimeRevisionBundle,
        directory: impl AsRef<std::path::Path>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        let materialization_specs = root.durable_materialization_specs();
        let physical_artifact_specs = root.durable_physical_artifact_specs();
        let artifact_cores =
            root.durable_artifact_cores()
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "failed to derive durable artifact core",
                })?;
        let durability =
            DurableRevisionStore::create_with_materializations_physical_artifacts_and_cores(
                directory,
                root.revision(),
                &materialization_specs,
                &physical_artifact_specs,
                &artifact_cores,
                registry,
            )?;
        Ok(Self {
            cell: RuntimeRevisionCell::new(root),
            durability: Mutex::new(durability),
            registry: registry.clone(),
        })
    }

    pub fn open(directory: impl AsRef<std::path::Path>) -> Result<Self, RuntimeRecoveryError> {
        Self::open_with_recovery_policy(directory, PhysicalRecoveryPolicy::default())
            .map(|(runtime, _)| runtime)
    }

    pub fn open_with_recovery_policy(
        directory: impl AsRef<std::path::Path>,
        physical_recovery_policy: PhysicalRecoveryPolicy,
    ) -> Result<(Self, PhysicalRecoveryReport), RuntimeRecoveryError> {
        let (durability, scan) = DurableRevisionStore::open(directory)?;
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
