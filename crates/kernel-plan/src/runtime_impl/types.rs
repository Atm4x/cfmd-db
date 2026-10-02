#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuntimeDurabilityBackend {
    #[default]
    SingleFile,
    Directory,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeStorageOptions {
    pub backend: RuntimeDurabilityBackend,
    pub encryption: kernel_durability::StorageEncryption,
}

impl RuntimeStorageOptions {
    #[must_use]
    pub const fn new(backend: RuntimeDurabilityBackend) -> Self {
        Self {
            backend,
            encryption: kernel_durability::StorageEncryption::None,
        }
    }

    #[must_use]
    pub fn with_encryption(mut self, encryption: kernel_durability::StorageEncryption) -> Self {
        self.encryption = encryption;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeRootVersion(u64);

impl RuntimeRootVersion {
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeRootIdentity {
    root_id: u64,
    version: RuntimeRootVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHistoryEffectKind {
    RelationData,
    RelationRewrite,
    RelationResolution,
    MixedRevision,
    FullRevision,
    SchemaMigration,
    LegacyTargetOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHistoryReversibility {
    ExactPlanInverse,
    ComplementRequired,
    NonPlanTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryRelationMutation {
    pub relation: SemanticId,
    pub inserted: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
    pub object_field_writes: Vec<kernel_durability::DurableObjectFieldWrite>,
    pub authorization: kernel_durability::DurableRelationAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryEffect {
    pub effect_id: u128,
    pub prerequisites: Vec<u128>,
    pub transaction_id: ClientTransactionId,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub kind: RuntimeHistoryEffectKind,
    pub reversibility: RuntimeHistoryReversibility,
    pub relation_mutations: Vec<RuntimeHistoryRelationMutation>,
    pub model_delta: Option<DurableModelDelta>,
    pub model_complement: Option<DurableModelDelta>,
    pub semantic_change: Option<SemanticChangeEvent>,
}

/// Exact write coordinate used to prove that a historical inverse can be
/// transported across later committed effects without changing either
/// transition's meaning. Relation coordinates are Γ-canonical equivalence
/// classes rather than host hashes; model coordinates name the persisted
/// carrier/field/lifecycle authorities directly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeHistoryCoordinate {
    RelationClass {
        relation: SemanticId,
        canonical_key: Box<[u8]>,
    },
    CarrierPresence {
        carrier: SemanticId,
    },
    CarrierMember {
        carrier: SemanticId,
        entity: kernel_types::EntityId,
    },
    Field {
        field: SemanticId,
        owner: kernel_types::EntityId,
    },
    ObjectField {
        relation: SemanticId,
        owner: kernel_types::EntityId,
        field: SemanticId,
    },
    LifecycleEntity {
        entity: kernel_types::EntityId,
    },
    LifecycleRoot {
        entity: kernel_types::EntityId,
    },
    KeepsAlivePresence {
        parent: kernel_types::EntityId,
    },
    KeepsAliveEdge {
        parent: kernel_types::EntityId,
        child: kernel_types::EntityId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeHistoryFootprint {
    pub writes: BTreeMap<RuntimeHistoryCoordinate, kernel_change::RewriteActionLaw>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryRebaseCertificate {
    pub effect_id: u128,
    pub original_target_revision: RevisionId,
    pub current_revision: RevisionId,
    pub intervening_effects: Vec<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryRebaseConflict {
    pub effect_id: u128,
    pub current_revision: RevisionId,
    pub conflicting_effects: Vec<u128>,
    pub coordinates: Vec<RuntimeHistoryCoordinate>,
    pub opaque_effects: Vec<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeHistoryRebaseOutcome {
    Certified(RuntimeHistoryRebaseCertificate),
    Conflict(RuntimeHistoryRebaseConflict),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTransitionRebaseCertificate {
    pub source_revision: RevisionId,
    pub current_revision: RevisionId,
    pub intervening_effects: Vec<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTransitionRebaseConflict {
    pub source_revision: RevisionId,
    pub current_revision: RevisionId,
    pub conflicting_effects: Vec<u128>,
    pub coordination_effects: Vec<u128>,
    pub coordinates: Vec<RuntimeHistoryCoordinate>,
    pub opaque_effects: Vec<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTransitionRebaseOutcome {
    Certified(RuntimeTransitionRebaseCertificate),
    Conflict(RuntimeTransitionRebaseConflict),
}

fn durable_model_delta_is_empty(delta: &DurableModelDelta) -> bool {
    delta.carriers.is_empty()
        && delta.fields.is_empty()
        && delta.lifecycle_entities_inserted.is_empty()
        && delta.lifecycle_entities_removed.is_empty()
        && delta.lifecycle_roots_inserted.is_empty()
        && delta.lifecycle_roots_removed.is_empty()
        && delta.lifecycle_keeps_alive.is_empty()
}

impl RuntimeHistoryEffect {
    fn from_durable(record: &DurableRevisionEffectRecord) -> Self {
        let semantic_change = record.semantic_change_event();
        let (kind, reversibility, relation_mutations, model_delta, model_complement) =
            match &record.intent {
                DurableTransactionIntent::RelationDataExact {
                    relation_mutations, ..
                } => (
                    RuntimeHistoryEffectKind::RelationData,
                    RuntimeHistoryReversibility::ExactPlanInverse,
                    relation_mutations.as_slice(),
                    None,
                    None,
                ),
                DurableTransactionIntent::RelationDataResidualExact {
                    realized_mutations, ..
                } => (
                    RuntimeHistoryEffectKind::RelationData,
                    RuntimeHistoryReversibility::ExactPlanInverse,
                    realized_mutations.as_slice(),
                    None,
                    None,
                ),
                DurableTransactionIntent::RelationRewriteExact {
                    relation_mutations, ..
                } => (
                    RuntimeHistoryEffectKind::RelationRewrite,
                    RuntimeHistoryReversibility::ExactPlanInverse,
                    relation_mutations.as_slice(),
                    None,
                    None,
                ),
                DurableTransactionIntent::RelationResolutionExact {
                    relation_mutations, ..
                } => (
                    RuntimeHistoryEffectKind::RelationResolution,
                    RuntimeHistoryReversibility::ExactPlanInverse,
                    relation_mutations.as_slice(),
                    None,
                    None,
                ),
                DurableTransactionIntent::MixedRevisionExact {
                    relation_mutations,
                    model_delta,
                    model_complement,
                    ..
                } => (
                    RuntimeHistoryEffectKind::MixedRevision,
                    if durable_model_delta_is_empty(model_delta) || model_complement.is_some() {
                        RuntimeHistoryReversibility::ExactPlanInverse
                    } else {
                        RuntimeHistoryReversibility::ComplementRequired
                    },
                    relation_mutations.as_slice(),
                    Some(model_delta.clone()),
                    model_complement.as_deref().cloned(),
                ),
                DurableTransactionIntent::MixedRevisionResidualExact {
                    realized_relation_mutations,
                    realized_model_delta,
                    realized_model_complement,
                    ..
                } => (
                    RuntimeHistoryEffectKind::MixedRevision,
                    RuntimeHistoryReversibility::ExactPlanInverse,
                    realized_relation_mutations.as_slice(),
                    Some(realized_model_delta.clone()),
                    Some(realized_model_complement.as_ref().clone()),
                ),
                DurableTransactionIntent::Exact { .. } => (
                    RuntimeHistoryEffectKind::FullRevision,
                    RuntimeHistoryReversibility::NonPlanTransition,
                    &[] as &[DurableRelationMutation],
                    None,
                    None,
                ),
                DurableTransactionIntent::SchemaMigrationExact { .. } => (
                    RuntimeHistoryEffectKind::SchemaMigration,
                    RuntimeHistoryReversibility::NonPlanTransition,
                    &[] as &[DurableRelationMutation],
                    None,
                    None,
                ),
                DurableTransactionIntent::LegacyTargetOnly { .. } => (
                    RuntimeHistoryEffectKind::LegacyTargetOnly,
                    RuntimeHistoryReversibility::NonPlanTransition,
                    &[] as &[DurableRelationMutation],
                    None,
                    None,
                ),
            };
        debug_assert_eq!(
            record.kind(),
            match kind {
                RuntimeHistoryEffectKind::RelationData => DurableEffectKind::RelationData,
                RuntimeHistoryEffectKind::RelationRewrite => DurableEffectKind::RelationRewrite,
                RuntimeHistoryEffectKind::RelationResolution =>
                    DurableEffectKind::RelationResolution,
                RuntimeHistoryEffectKind::MixedRevision => DurableEffectKind::MixedRevision,
                RuntimeHistoryEffectKind::FullRevision => DurableEffectKind::FullRevision,
                RuntimeHistoryEffectKind::SchemaMigration => DurableEffectKind::SchemaMigration,
                RuntimeHistoryEffectKind::LegacyTargetOnly => DurableEffectKind::LegacyTargetOnly,
            }
        );
        Self {
            effect_id: record.id.0,
            prerequisites: record.prerequisites.iter().map(|id| id.0).collect(),
            transaction_id: record.transaction_id,
            source_revision: record.source_revision,
            target_revision: record.target_revision,
            kind,
            reversibility,
            relation_mutations: relation_mutations
                .iter()
                .map(|mutation| RuntimeHistoryRelationMutation {
                    relation: mutation.relation,
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    object_field_writes: mutation.object_field_writes.clone(),
                    authorization: mutation.authorization,
                })
                .collect(),
            model_delta,
            model_complement,
            semantic_change,
        }
    }
}

#[derive(Debug)]
pub enum RuntimeHistoricalSnapshotError {
    Durability(DurabilityError),
    Recovery(RuntimeRecoveryError),
    Runtime(PhysicalExecutionError),
    Revision(kernel_revision::RevisionError),
    Unavailable { revision: RevisionId },
    EffectUnavailable { effect_id: u128 },
    EffectNotReversible { effect_id: u128 },
}

impl From<DurabilityError> for RuntimeHistoricalSnapshotError {
    fn from(value: DurabilityError) -> Self {
        Self::Durability(value)
    }
}

impl From<RuntimeRecoveryError> for RuntimeHistoricalSnapshotError {
    fn from(value: RuntimeRecoveryError) -> Self {
        Self::Recovery(value)
    }
}

impl From<PhysicalExecutionError> for RuntimeHistoricalSnapshotError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Runtime(value)
    }
}

impl From<kernel_revision::RevisionError> for RuntimeHistoricalSnapshotError {
    fn from(value: kernel_revision::RevisionError) -> Self {
        Self::Revision(value)
    }
}

static NEXT_RUNTIME_ROOT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn allocate_runtime_root_id() -> Result<u64, PhysicalExecutionError> {
    NEXT_RUNTIME_ROOT_ID
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| current.checked_add(1),
        )
        .map_err(|_| PhysicalExecutionError::RuntimeRootIdentityExhausted)
}

/// One normalized base-relation mutation participating in a logical revision.
/// A revision batch may contain several relations, but at most one mutation per
/// relation; callers normalize multiple edits to one relation before prepare.
pub struct RevisionRelationMutation<'a> {
    pub relation: SemanticId,
    pub delta: &'a RelationDelta,
    pub object_field_writes: &'a [kernel_durability::DurableObjectFieldWrite],
    pub authorization: kernel_durability::DurableRelationAuthorization,
}

/// Failure while deriving an exact logical target Revision from one immutable
/// runtime snapshot plus source-relative relation deltas.
#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeRevisionDerivationError {
    Runtime(PhysicalExecutionError),
    Revision(kernel_revision::RevisionError),
}

impl From<PhysicalExecutionError> for RuntimeRevisionDerivationError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Runtime(value)
    }
}

impl From<kernel_revision::RevisionError> for RuntimeRevisionDerivationError {
    fn from(value: kernel_revision::RevisionError) -> Self {
        Self::Revision(value)
    }
}

/// One intent-bearing relation rewrite participating in a logical revision.
/// The embedded Γ-validated delta remains the DTC/physical maintenance effect;
/// Rewrite identity/law identity are preserved independently.
pub struct RevisionRelationRewrite<'a, I = Value> {
    pub relation: SemanticId,
    pub rewrite: &'a PreparedRelationRewrite<I>,
}

pub struct RevisionRewriteTransitionRequest<'a, I = Value> {
    pub target_revision: &'a kernel_revision::Revision,
    pub rewrites: &'a [RevisionRelationRewrite<'a, I>],
    pub registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeRewriteIntent {
    pub spec: RewriteSpecId,
    pub law_set: RewriteLawSetId,
}

/// One maintained query registered under the authoritative runtime revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMaterializationSpec {
    pub id: kernel_types::MaterializationId,
    pub query: RelExpr,
}

/// Logical descriptor carried across the pre-durable seal boundary.
///
/// It intentionally contains semantic relation deltas rather than physical row
/// handles/layout mutations. A later WAL layer can serialize this descriptor
/// without making reconstructible physical state durable authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionCommitDescriptor {
    source_revision: RevisionId,
    target: Box<kernel_revision::Revision>,
    change: RevisionCommitChange,
    rewrite_intents: BTreeMap<SemanticId, RuntimeRewriteIntent>,
    object_field_writes: BTreeMap<SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>>,
    relation_authorizations: BTreeMap<SemanticId, kernel_durability::DurableRelationAuthorization>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionCommitChange {
    RelationData {
        semantic_revision: kernel_types::SemanticRevision,
        relation_deltas: BTreeMap<SemanticId, RelationDelta>,
    },
    /// Full logical revision authority with incremental physical relation
    /// publication. Durable recovery retains the exact target revision because
    /// lifecycle/carrier/field changes are not derivable from relation deltas.
    MixedRevision {
        semantic_revision: kernel_types::SemanticRevision,
        relation_deltas: BTreeMap<SemanticId, RelationDelta>,
        model_delta: DurableModelDelta,
        model_complement: Box<DurableModelDelta>,
    },
    FullRevision,
    FullRevisionAndMaterializations {
        materializations: Vec<DurableMaterializationSpec>,
    },
}

impl RevisionCommitDescriptor {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub fn target_revision(&self) -> RevisionId {
        self.target.id()
    }

    #[must_use]
    pub fn target(&self) -> &kernel_revision::Revision {
        self.target.as_ref()
    }

    #[must_use]
    pub const fn change(&self) -> &RevisionCommitChange {
        &self.change
    }

    #[must_use]
    pub const fn rewrite_intents(&self) -> &BTreeMap<SemanticId, RuntimeRewriteIntent> {
        &self.rewrite_intents
    }

    #[must_use]
    pub const fn semantic_revision(&self) -> Option<kernel_types::SemanticRevision> {
        match &self.change {
            RevisionCommitChange::RelationData {
                semantic_revision, ..
            }
            | RevisionCommitChange::MixedRevision {
                semantic_revision, ..
            } => Some(*semantic_revision),
            RevisionCommitChange::FullRevision
            | RevisionCommitChange::FullRevisionAndMaterializations { .. } => None,
        }
    }

    #[must_use]
    pub fn relation_deltas(&self) -> Option<&BTreeMap<SemanticId, RelationDelta>> {
        match &self.change {
            RevisionCommitChange::RelationData {
                relation_deltas, ..
            }
            | RevisionCommitChange::MixedRevision {
                relation_deltas, ..
            } => Some(relation_deltas),
            RevisionCommitChange::FullRevision
            | RevisionCommitChange::FullRevisionAndMaterializations { .. } => None,
        }
    }

    pub fn durable_descriptor(
        &self,
        transaction_id: ClientTransactionId,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<DurableRevisionDescriptor, DurabilityError> {
        match &self.change {
            RevisionCommitChange::RelationData {
                semantic_revision,
                relation_deltas,
            } => {
                let relation_mutations = relation_deltas
                    .iter()
                    .map(|(&relation, delta)| DurableRelationMutation {
                        relation,
                        inserted: delta.inserted.clone(),
                        removed: delta.removed.clone(),
                        object_field_writes: self.object_field_writes.get(&relation).cloned().unwrap_or_default(),
                        authorization: self.relation_authorizations.get(&relation).copied().unwrap_or_default(),
                    })
                    .collect();
                if self.rewrite_intents.is_empty() {
                    Ok(DurableRevisionDescriptor::relation_data(
                        transaction_id,
                        self.source_revision,
                        self.target.as_ref(),
                        *semantic_revision,
                        relation_mutations,
                        registry,
                    )?)
                } else {
                    let rewrite_intents = self
                        .rewrite_intents
                        .iter()
                        .map(|(&relation, intent)| DurableRelationRewriteIntent {
                            relation,
                            rewrite_spec: intent.spec.0,
                            law_set: intent.law_set.0,
                        })
                        .collect();
                    Ok(DurableRevisionDescriptor::relation_rewrites(
                        transaction_id,
                        self.source_revision,
                        self.target.as_ref(),
                        *semantic_revision,
                        relation_mutations,
                        rewrite_intents,
                        registry,
                    )?)
                }
            }
            RevisionCommitChange::MixedRevision {
                semantic_revision,
                relation_deltas,
                model_delta,
                model_complement,
            } => {
                let relation_mutations = relation_deltas
                    .iter()
                    .map(|(&relation, delta)| DurableRelationMutation {
                        relation,
                        inserted: delta.inserted.clone(),
                        removed: delta.removed.clone(),
                        object_field_writes: self.object_field_writes.get(&relation).cloned().unwrap_or_default(),
                        authorization: self.relation_authorizations.get(&relation).copied().unwrap_or_default(),
                    })
                    .collect();
                DurableRevisionDescriptor::mixed_revision(
                    transaction_id,
                    self.source_revision,
                    self.target.as_ref(),
                    *semantic_revision,
                    relation_mutations,
                    model_delta.clone(),
                    model_complement.as_ref().clone(),
                    registry,
                )
                .map_err(DurabilityError::Encode)
            }
            RevisionCommitChange::FullRevision => DurableRevisionDescriptor::full_revision(
                transaction_id,
                self.source_revision,
                self.target.as_ref(),
                registry,
            )
            .map_err(DurabilityError::Encode),
            RevisionCommitChange::FullRevisionAndMaterializations { materializations } => {
                DurableRevisionDescriptor::full_revision_and_materializations(
                    transaction_id,
                    self.source_revision,
                    self.target.as_ref(),
                    materializations,
                    registry,
                )
                .map_err(DurabilityError::Encode)
            }
        }
    }

    pub fn durable_resolution_descriptor(
        &self,
        transaction_id: ClientTransactionId,
        causal_parents: Vec<RevisionId>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<DurableRevisionDescriptor, DurabilityError> {
        let RevisionCommitChange::RelationData {
            semantic_revision,
            relation_deltas,
        } = &self.change
        else {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable resolution requires relation-data change",
            });
        };
        if self.rewrite_intents.is_empty() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable resolution requires exact Rewrite intent",
            });
        }
        let relation_mutations = relation_deltas
            .iter()
            .map(|(&relation, delta)| DurableRelationMutation {
                relation,
                inserted: delta.inserted.clone(),
                removed: delta.removed.clone(),
                object_field_writes: Vec::new(),
                authorization: self.relation_authorizations.get(&relation).copied().unwrap_or_default(),
            })
            .collect();
        let rewrite_intents = self
            .rewrite_intents
            .iter()
            .map(|(&relation, intent)| DurableRelationRewriteIntent {
                relation,
                rewrite_spec: intent.spec.0,
                law_set: intent.law_set.0,
            })
            .collect();
        DurableRevisionDescriptor::relation_resolution(
            transaction_id,
            self.source_revision,
            self.target.as_ref(),
            *semantic_revision,
            DurableRelationResolution {
                relation_mutations,
                rewrite_intents,
                causal_parents,
            },
            registry,
        )
        .map_err(DurabilityError::Encode)
    }
}

#[derive(Debug)]
pub enum DurableRuntimeCommitError {
    Runtime(PhysicalExecutionError),
    Recovery(RuntimeRecoveryError),
    MigrationTransport(kernel_transport::TransportError),
    PrepareDurability(DurabilityError),
    CommitDurabilityUncertain(DurabilityError),
    TransactionIdConflict {
        transaction_id: ClientTransactionId,
        committed_target: RevisionId,
        requested_target: RevisionId,
    },
}

impl From<PhysicalExecutionError> for DurableRuntimeCommitError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Runtime(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRuntimeCommitReceipt {
    pub durable: DurableCommitReceipt,
    pub publication: RuntimePublicationEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimePublicationEffect {
    Incremental(BTreeMap<kernel_types::MaterializationId, RelationDelta>),
    Rebuilt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableRuntimeCommitOutcome {
    Committed(DurableRuntimeCommitReceipt),
    AlreadyCommitted { target_revision: RevisionId },
}

#[derive(Debug)]
pub enum DurableRuntimeCheckpointError {
    Runtime(PhysicalExecutionError),
    Recovery(RuntimeRecoveryError),
    Durability(DurabilityError),
    DurabilityUncertain(DurabilityError),
}

#[derive(Debug)]
pub enum DurableRuntimeHistoricalError {
    Durability(DurabilityError),
    Complement(kernel_durability::HistoricalComplementError),
    Restore(kernel_durability::HistoricalRestoreError),
}

impl From<DurabilityError> for DurableRuntimeHistoricalError {
    fn from(value: DurabilityError) -> Self {
        Self::Durability(value)
    }
}

impl From<kernel_durability::HistoricalComplementError> for DurableRuntimeHistoricalError {
    fn from(value: kernel_durability::HistoricalComplementError) -> Self {
        Self::Complement(value)
    }
}

impl From<kernel_durability::HistoricalRestoreError> for DurableRuntimeHistoricalError {
    fn from(value: kernel_durability::HistoricalRestoreError) -> Self {
        Self::Restore(value)
    }
}

impl From<PhysicalExecutionError> for DurableRuntimeCheckpointError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Runtime(value)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeRecoveryError {
    Durability(DurabilityError),
    Runtime(PhysicalExecutionError),
    Revision(kernel_revision::RevisionError),
    MigrationTransport(kernel_transport::TransportError),
    BaseRevisionMismatch,
    SemanticRevisionMismatch,
    DurableHeadMismatch,
}

impl From<DurabilityError> for RuntimeRecoveryError {
    fn from(value: DurabilityError) -> Self {
        Self::Durability(value)
    }
}

impl From<PhysicalExecutionError> for RuntimeRecoveryError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Runtime(value)
    }
}

impl From<kernel_revision::RevisionError> for RuntimeRecoveryError {
    fn from(value: kernel_revision::RevisionError) -> Self {
        Self::Revision(value)
    }
}

/// Complete logical source->target revision transition request.
///
/// The target is an already validated `kernel_revision::Revision`. The source
/// revision and pinned semantic context come from the live runtime bundle.
pub struct RevisionTransitionRequest<'a> {
    pub target_revision: &'a kernel_revision::Revision,
    pub mutations: &'a [RevisionRelationMutation<'a>],
    pub registry: &'a kernel_semantics::SemanticRegistry,
}

/// One validated semantic revision endpoint whose relation component can be
/// published through the incremental physical delta path while lifecycle,
/// carrier and field state changes atomically with the same revision.
///
/// This is the general mixed-data counterpart of `RevisionTransitionRequest`:
/// the semantic context must remain pinned, the supplied relation mutations
/// must exactly explain every changed relation, but non-relation model state
/// is allowed to differ because it is already certified by `Revision::build`.
pub struct MixedRevisionTransitionRequest<'a> {
    pub source_revision: RevisionId,
    pub target_revision: &'a kernel_revision::Revision,
    pub mutations: &'a [RevisionRelationMutation<'a>],
    pub model_delta: &'a DurableModelDelta,
    pub model_complement: &'a DurableModelDelta,
    pub registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationEndpointValidation {
    /// Arbitrary target: replay the claimed delta. Certified relation-only
    /// provenance may eliminate only the global untouched/static comparison.
    ReplayClaimedDelta,
    /// Private durable-derived target built by this runtime from the exact
    /// supplied delta. The caller must first verify relation-only provenance
    /// against the retained source snapshot.
    ExactDerived,
}

/// Compact durable relation-data request whose target content is derived by
/// the runtime from one authoritative source Revision plus the supplied typed
/// deltas. Unlike `RevisionTransitionRequest`, this surface does not accept an
/// independently constructed target snapshot, so `(source, target id, delta)`
/// is the complete client-controlled logical request identity.
pub struct DerivedRelationTransitionRequest<'a> {
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub mutations: &'a [RevisionRelationMutation<'a>],
}

/// Private proof object for one endpoint that this runtime derived from one
/// retained authoritative source by applying exactly the recorded relation
/// deltas.  The revision's relation-only provenance proves inherited static
/// state; this wrapper additionally binds the concrete delta payloads that
/// produced the touched endpoints, so later derived publication cannot
/// accidentally reuse `ExactDerived` with a different mutation set.
#[derive(Debug)]
// HOSTILE[P161][ACTIVE][CLEAN]: exact private source/delta-derived endpoint authority.
struct DerivedRelationEndpoint {
    revision: kernel_revision::Revision,
    exact_deltas: BTreeMap<SemanticId, RelationDelta>,
}

impl DerivedRelationEndpoint {
    #[must_use]
    const fn revision(&self) -> &kernel_revision::Revision {
        &self.revision
    }

    fn certifies_mutations(&self, mutations: &[RevisionRelationMutation<'_>]) -> bool {
        if self.exact_deltas.len() != mutations.len() {
            return false;
        }
        let mut seen = BTreeSet::new();
        mutations.iter().all(|mutation| {
            seen.insert(mutation.relation)
                && self
                    .exact_deltas
                    .get(&mutation.relation)
                    .is_some_and(|delta| delta == mutation.delta)
        })
    }
}

/// Compact durable relation Rewrite request. The runtime derives target
/// relation content from the authoritative source and embedded typed deltas;
/// durable identity additionally retains RewriteSpec/law-set identity.
pub struct DerivedRelationRewriteTransitionRequest<'a, I = Value> {
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub rewrites: &'a [RevisionRelationRewrite<'a, I>],
}

/// Full semantic revision replacement. This is the correctness-first durable
/// path for schema/Γ/lifecycle/field changes that cannot be represented as an
/// incremental relation-data delta under one pinned semantic context.
pub struct FullRevisionTransitionRequest<'a> {
    pub target_revision: &'a kernel_revision::Revision,
    pub registry: &'a kernel_semantics::SemanticRegistry,
}

/// One atomic control-plane transition that replaces the complete semantic
/// revision and the maintained materialization registry under the same durable
/// client transaction identity.
pub struct RevisionAndMaterializationsTransitionRequest<'a> {
    pub target_revision: &'a kernel_revision::Revision,
    pub materializations: &'a [RuntimeMaterializationSpec],
    pub registry: &'a kernel_semantics::SemanticRegistry,
}

/// Owned relation mutation used by bounded repair providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRepairRelationMutation {
    pub relation: SemanticId,
    pub delta: RelationDelta,
}

/// One finite repair candidate. The provider supplies intent/policy; the
/// runtime remains responsible for exact VMF validation and OFC preservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeRepairCandidate {
    RelationData {
        target_revision: kernel_revision::Revision,
        mutations: Vec<RuntimeRepairRelationMutation>,
    },
    FullRevision {
        target_revision: kernel_revision::Revision,
    },
    /// Full semantic transition accompanied by a verified Γ/TSC witness that
    /// transports the observed source fiber into the target context.
    TransportedFullRevision {
        target_revision: kernel_revision::Revision,
        observation_transport: Box<RuntimeRepairObservationTransport>,
    },
}

/// Verified observation transport for repair candidates that cross a semantic
/// context boundary. This is an adapter to the existing checked transport
/// calculus, not an authority or permission to alter the observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeRepairObservationTransport {
    Definitional(kernel_transport::DefinitionalTransport),
    EquivalentSemanticEnvironment(kernel_transport::EquivalentSemanticEnvironmentTransport),
    ConservativeSemanticEnvironmentExtension(
        kernel_transport::ConservativeSemanticEnvironmentExtension,
    ),
    BijectiveIdentity(kernel_transport::BijectiveIdentityRevisionTransport),
}

impl RuntimeRepairObservationTransport {
    fn source_context(&self) -> &kernel_schema::SemanticContext {
        match self {
            Self::Definitional(transport) => transport.source(),
            Self::EquivalentSemanticEnvironment(transport) => transport.source(),
            Self::ConservativeSemanticEnvironmentExtension(transport) => transport.source(),
            Self::BijectiveIdentity(transport) => transport.source(),
        }
    }

    fn target_context(&self) -> &kernel_schema::SemanticContext {
        match self {
            Self::Definitional(transport) => transport.target(),
            Self::EquivalentSemanticEnvironment(transport) => transport.target(),
            Self::ConservativeSemanticEnvironmentExtension(transport) => transport.target(),
            Self::BijectiveIdentity(transport) => transport.target(),
        }
    }

    fn transport_source_revision(
        &self,
        source: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_revision::Revision, kernel_transport::TransportError> {
        let id = source.id();
        match self {
            Self::Definitional(transport) => transport.transport_revision(source, id, registry),
            Self::EquivalentSemanticEnvironment(transport) => {
                transport.transport_revision(source, id, registry)
            }
            Self::ConservativeSemanticEnvironmentExtension(transport) => {
                transport.transport_revision(source, id, registry)
            }
            Self::BijectiveIdentity(transport) => {
                transport.transport_revision(source, id, registry)
            }
        }
    }

    fn transport_query(
        &self,
        query: &RelExpr,
    ) -> Result<RelExpr, kernel_transport::TransportError> {
        match self {
            Self::BijectiveIdentity(transport) => {
                kernel_transport::transport_rel_expr(transport.identity(), query)
            }
            Self::Definitional(_)
            | Self::EquivalentSemanticEnvironment(_)
            | Self::ConservativeSemanticEnvironmentExtension(_) => Ok(query.clone()),
        }
    }
}

/// Stable adapter seam for Rewrite/WritableLens synthesis. Candidate
/// generation is untrusted bounded policy; verification stays in the runtime.
pub trait RepairCandidateProvider {
    fn candidates(
        &self,
        source: &RuntimeRevisionBundle,
        maximum: usize,
    ) -> Result<Vec<RuntimeRepairCandidate>, PhysicalExecutionError>;
}

impl RepairCandidateProvider for Vec<RuntimeRepairCandidate> {
    fn candidates(
        &self,
        _source: &RuntimeRevisionBundle,
        _maximum: usize,
    ) -> Result<Vec<RuntimeRepairCandidate>, PhysicalExecutionError> {
        Ok(self.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairSearchPolicy {
    pub max_candidates: usize,
}

impl Default for RepairSearchPolicy {
    fn default() -> Self {
        Self { max_candidates: 64 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairSearchReport {
    pub supplied: usize,
    pub examined: usize,
    pub rejected_invalid: usize,
    pub rejected_transport: usize,
    pub rejected_observation_change: usize,
    pub accepted: usize,
}

#[derive(Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Rare control-plane result intentionally owns one prepared transition without extra allocation.
pub enum RepairSearchOutcome {
    NoRepair(RepairSearchReport),
    Prepared {
        transition: PreparedRuntimeRevisionTransition,
        report: RepairSearchReport,
    },
    Ambiguous {
        valid_candidates: usize,
        report: RepairSearchReport,
    },
    BudgetExceeded {
        supplied: usize,
        maximum: usize,
    },
}

/// Reconstructible exact VMF state bound to one validated logical revision.
///
/// The authoritative state remains `Revision`. This measure exists so the
/// runtime publication boundary can require `V = 0` and later swap full
/// recomputation for Γ-DTC maintenance without changing commit semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeViolationState {
    revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    measure: kernel_violation::ViolationMeasure<kernel_validation::DynamicViolationWitness>,
}

impl RuntimeViolationState {
    #[cfg(test)]
    fn measure_mut_for_test(
        &mut self,
    ) -> &mut kernel_violation::ViolationMeasure<kernel_validation::DynamicViolationWitness> {
        &mut self.measure
    }
}

/// Opaque proof that the exact Γ-VMF measure for one logical revision is zero.
///
/// This is revision-bound rather than family-bound: residual/confluence code
/// may reason about Rewrite families, but publication still needs an exact
/// candidate-state validity proof under the pinned semantic revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInvariantClosureCertificate {
    revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
}

impl RuntimeInvariantClosureCertificate {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn semantic_revision(&self) -> kernel_types::SemanticRevision {
        self.semantic_revision
    }
}

/// Revision/root-bound exact observation-fiber guard.
///
/// The inner OFC guard owns the normalized output fiber and pinned Γ-DTC
/// program. Root/revision binding prevents a guard prepared from one runtime
/// lineage from being silently reused against another same-number revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeObservationGuard {
    source_identity: RuntimeRootIdentity,
    source_revision: RevisionId,
    guard: RelObservationGuard,
}

impl RuntimeObservationGuard {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub fn source_relations(&self) -> &BTreeSet<SemanticId> {
        self.guard.source_relations()
    }

    /// Exact OFC impact for one prepared revision transition.
    ///
    /// The relation envelope is only a sound fast path. Any potentially
    /// relevant transition is decided by the pinned Γ-DTC program over the
    /// source and candidate logical models.
    pub fn impact_prepared(
        &self,
        source: &RuntimeRevisionBundle,
        prepared: &PreparedRuntimeRevisionTransition,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, PhysicalExecutionError> {
        if source.root_identity != self.source_identity
            || source.revision.id() != self.source_revision
            || prepared.source_identity != self.source_identity
            || prepared.source_revision() != self.source_revision
        {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        if source.revision.semantic_context() != prepared.candidate.revision.semantic_context() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }

        if let Some(deltas) = prepared.descriptor.relation_deltas()
            && deltas
                .keys()
                .all(|relation| !self.guard.source_relations().contains(relation))
        {
            return Ok(Impact::Unaffected);
        }

        Ok(self.guard.impact_between(
            &source.revision.state().model,
            &prepared.candidate.revision.state().model,
            source.revision.semantic_context(),
            registry,
        )?)
    }

    /// Exact OFC check across a verified semantic transport. The source
    /// observation is transported into the target Γ first; comparison then
    /// happens wholly inside that target context.
    pub fn impact_prepared_transported(
        &self,
        source: &RuntimeRevisionBundle,
        prepared: &PreparedRuntimeRevisionTransition,
        transport: &RuntimeRepairObservationTransport,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, PhysicalExecutionError> {
        if source.root_identity != self.source_identity
            || source.revision.id() != self.source_revision
            || prepared.source_identity != self.source_identity
            || prepared.source_revision() != self.source_revision
        {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        if transport.source_context() != source.revision.semantic_context()
            || transport.target_context() != prepared.candidate.revision.semantic_context()
        {
            return Err(PhysicalExecutionError::RepairObservationTransportMismatch);
        }

        let baseline = transport
            .transport_source_revision(&source.revision, registry)
            .map_err(PhysicalExecutionError::RepairTransport)?;
        let query = transport
            .transport_query(self.guard.query())
            .map_err(PhysicalExecutionError::RepairTransport)?;
        let target_guard = RelObservationGuard::observe(
            &query,
            &baseline.state().model,
            baseline.semantic_context(),
            registry,
        )?;
        Ok(target_guard.impact_between(
            &baseline.state().model,
            &prepared.candidate.revision.state().model,
            baseline.semantic_context(),
            registry,
        )?)
    }

    /// Independent full-recompute parity oracle for rollout/hostile tests.
    pub fn impact_prepared_by_recompute_oracle(
        &self,
        source: &RuntimeRevisionBundle,
        prepared: &PreparedRuntimeRevisionTransition,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, PhysicalExecutionError> {
        if source.root_identity != self.source_identity
            || source.revision.id() != self.source_revision
            || prepared.source_identity != self.source_identity
            || prepared.source_revision() != self.source_revision
        {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        if source.revision.semantic_context() != prepared.candidate.revision.semantic_context() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        Ok(self.guard.impact_between_by_recompute_oracle(
            &source.revision.state().model,
            &prepared.candidate.revision.state().model,
            source.revision.semantic_context(),
            registry,
        ))
    }
}

impl RuntimeViolationState {
    fn build(
        revision: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let measure = kernel_validation::dynamic_violation_measure(
            revision.semantic_context(),
            registry,
            revision.state(),
            revision.dense_type_extents(),
        )?;
        Ok(Self {
            revision: revision.id(),
            semantic_revision: revision.semantic_revision(),
            measure,
        })
    }

    fn candidate_for_relation_transition(
        source: &Self,
        target: &kernel_revision::Revision,
        changed_relations: impl IntoIterator<Item = SemanticId>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        source.require_zero()?;
        if source.semantic_revision != target.semantic_revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }

        let mut measure = kernel_violation::ViolationMeasure::new();
        for relation in changed_relations.into_iter().collect::<BTreeSet<_>>() {
            let relation_measure = kernel_validation::relation_dynamic_violation_measure(
                target.semantic_context(),
                registry,
                target.state(),
                target.dense_type_extents(),
                relation,
            )?;
            for (witness, mass) in relation_measure.iter() {
                measure.add(witness.clone(), mass).map_err(|error| {
                    PhysicalExecutionError::Validation(
                        kernel_validation::ValidationError::ViolationMeasure(error),
                    )
                })?;
            }
        }
        Ok(Self {
            revision: target.id(),
            semantic_revision: target.semantic_revision(),
            measure,
        })
    }

    fn transport_zero_to_relation_target(
        source: &Self,
        target: &kernel_revision::Revision,
    ) -> Result<Self, PhysicalExecutionError> {
        source.require_zero()?;
        if source.semantic_revision != target.semantic_revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        Ok(Self {
            revision: target.id(),
            semantic_revision: target.semantic_revision(),
            measure: kernel_violation::ViolationMeasure::new(),
        })
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.measure.is_zero()
    }

    #[must_use]
    pub fn witness_count(&self) -> usize {
        self.measure.witness_count()
    }

    fn require_zero(&self) -> Result<(), PhysicalExecutionError> {
        if self.is_zero() {
            Ok(())
        } else {
            Err(PhysicalExecutionError::CandidateViolationStateNonZero)
        }
    }

    fn closure_certificate(
        &self,
    ) -> Result<RuntimeInvariantClosureCertificate, PhysicalExecutionError> {
        self.require_zero()?;
        Ok(RuntimeInvariantClosureCertificate {
            revision: self.revision,
            semantic_revision: self.semantic_revision,
        })
    }

    fn require_bound_to(
        &self,
        revision: &kernel_revision::Revision,
    ) -> Result<(), PhysicalExecutionError> {
        if self.revision == revision.id() && self.semantic_revision == revision.semantic_revision()
        {
            Ok(())
        } else {
            Err(PhysicalExecutionError::RevisionBindingMismatch)
        }
    }
}
