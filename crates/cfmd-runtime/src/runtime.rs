use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    CandidatePreview, CommitOutcome, Error, ErrorKind, IntentJournal, IntentReadiness, Plan,
    PreparedQuery, Query, RelationId, RelationResult, Result, RevisionId, Row, Schema, SchemaView,
    SemanticRuleExpr, TransactionId,
    control::{ControlAuthority, DatabaseControlPermission},
    query::{query_error, query_error_at},
    schema::{
        PrimitiveEquivalence, PrimitiveOrdering, RelationSemantics, StructuralEquivalence,
        exact_aggregate_measure_to_kernel, ordered_statistic_bound_to_kernel,
        semantic_rule_to_kernel, text_pattern_to_kernel, type_to_kernel,
    },
    security::{Permission, RuntimeAuthority, Session, SessionDatabase},
};

#[derive(Debug, Clone)]
pub struct Database {
    runtime: Arc<kernel_plan::DurableRuntime>,
    identity: u64,
}

/// Transport-neutral exact base-relation effect formed at a caller-known revision.
///
/// This is an effect carrier, not another transaction abstraction. Bindings may use it to preserve
/// the exact relation mutation across transport while the runtime remains the sole owner of stale
/// effect certification, durability, authorization and idempotency.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactRelationMutation {
    pub relation: RelationId,
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupVerification {
    revision: RevisionId,
}

impl BackupVerification {
    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Storage {
    #[default]
    Auto,
    SingleFile,
    Directory,
}

#[derive(Debug, Clone)]
pub struct ExternalFreshness {
    store_id: [u8; 32],
    trust_root_epoch: u64,
    verifying_keys: Vec<[u8; 32]>,
    deployment_policy_epoch: u64,
    endpoint: SocketAddr,
    connect_timeout: Duration,
    io_timeout: Duration,
}

impl ExternalFreshness {
    #[must_use]
    pub fn tcp(
        store_id: [u8; 32],
        trust_root_epoch: u64,
        verifying_keys: Vec<[u8; 32]>,
        deployment_policy_epoch: u64,
        endpoint: SocketAddr,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> Self {
        Self {
            store_id,
            trust_root_epoch,
            verifying_keys,
            deployment_policy_epoch,
            endpoint,
            connect_timeout,
            io_timeout,
        }
    }

    fn resolve_kernel(
        &self,
    ) -> Result<(
        kernel_durability::ExternalFreshnessConfig,
        Box<dyn kernel_durability::ExternalFreshnessAuthority>,
    )> {
        let config = kernel_durability::ExternalFreshnessConfig::bootstrap(
            self.store_id,
            self.trust_root_epoch,
            &self.verifying_keys,
            self.deployment_policy_epoch,
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::InvalidPlan,
                format!("invalid external freshness configuration: {error:?}"),
            )
        })?;
        let authority = kernel_durability::TcpExternalFreshnessAuthority::new(
            self.endpoint,
            self.connect_timeout,
            self.io_timeout,
        );
        Ok((config, Box::new(authority)))
    }
}

#[derive(Clone)]
pub struct EncryptionKey(kernel_plan::StorageEncryptionKey);

impl EncryptionKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self> {
        kernel_plan::StorageEncryptionKey::try_new(bytes)
            .map(Self)
            .map_err(|error| {
                Error::new(
                    ErrorKind::ResourceLimit,
                    format!("secure encryption-key memory unavailable: {error:?}"),
                )
            })
    }
}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EncryptionKey(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncryptionKeyId([u8; 16]);

impl EncryptionKeyId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionKeyOperation {
    Create,
    Open,
    Rewrap,
}

pub struct EncryptionKeyDestination<'a> {
    bytes: &'a mut [u8; 32],
    initialized: bool,
}

impl EncryptionKeyDestination<'_> {
    pub fn write(&mut self, bytes: &[u8; 32]) {
        self.bytes.copy_from_slice(bytes);
        self.initialized = true;
    }

    pub fn fill_with<E>(
        &mut self,
        fill: impl FnOnce(&mut [u8; 32]) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        fill(self.bytes)?;
        self.initialized = true;
        Ok(())
    }
}

impl std::fmt::Debug for EncryptionKeyDestination<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncryptionKeyDestination")
            .field("initialized", &self.initialized)
            .field("contents", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EncryptionProviderKeyMetadata {
    key_id: EncryptionKeyId,
    key_epoch: u64,
    minimum_database_key_epoch: u64,
}

impl EncryptionProviderKeyMetadata {
    #[must_use]
    pub const fn new(key_id: EncryptionKeyId, key_epoch: u64) -> Self {
        Self {
            key_id,
            key_epoch,
            minimum_database_key_epoch: 1,
        }
    }

    /// Sets the minimum database-key epoch this external key authority accepts.
    /// Opening a complete but older wrapped-key header below this floor fails closed.
    #[must_use]
    pub const fn with_minimum_database_key_epoch(
        mut self,
        minimum_database_key_epoch: u64,
    ) -> Self {
        self.minimum_database_key_epoch = minimum_database_key_epoch;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptionKeyAcknowledgement {
    provider_key_id: EncryptionKeyId,
    provider_key_epoch: u64,
    database_key_epoch: u64,
}

impl EncryptionKeyAcknowledgement {
    #[must_use]
    pub const fn provider_key_id(self) -> EncryptionKeyId {
        self.provider_key_id
    }

    #[must_use]
    pub const fn provider_key_epoch(self) -> u64 {
        self.provider_key_epoch
    }

    #[must_use]
    pub const fn database_key_epoch(self) -> u64 {
        self.database_key_epoch
    }
}

pub trait EncryptionKeyProvider: std::fmt::Debug + Send + Sync {
    fn provide_key(
        &self,
        path: &Path,
        operation: EncryptionKeyOperation,
        destination: &mut EncryptionKeyDestination<'_>,
    ) -> Result<EncryptionProviderKeyMetadata>;

    fn acknowledge_database_key_epoch(
        &self,
        _path: &Path,
        _acknowledgement: EncryptionKeyAcknowledgement,
    ) -> Result<()> {
        Err(Error::new(
            ErrorKind::Recovery,
            "encryption key provider does not support durable database-key acknowledgement",
        ))
    }
}

fn resolve_provider_key(
    provider: &dyn EncryptionKeyProvider,
    path: &Path,
    operation: EncryptionKeyOperation,
) -> Result<(
    kernel_plan::StorageEncryptionKey,
    EncryptionProviderKeyMetadata,
)> {
    let initialized = kernel_plan::StorageEncryptionKey::try_initialize(|bytes| {
        let mut destination = EncryptionKeyDestination {
            bytes,
            initialized: false,
        };
        let metadata = provider.provide_key(path, operation, &mut destination)?;
        if !destination.initialized {
            return Err(Error::new(
                ErrorKind::Recovery,
                "encryption key provider returned success without initializing the secure key destination",
            ));
        }
        validate_provider_metadata(metadata)?;
        Ok(metadata)
    });

    match initialized {
        Ok(result) => Ok(result),
        Err(kernel_plan::StorageEncryptionKeyInitError::Memory(error)) => Err(Error::new(
            ErrorKind::ResourceLimit,
            format!("secure encryption-key memory unavailable: {error:?}"),
        )),
        Err(kernel_plan::StorageEncryptionKeyInitError::Initializer(error)) => Err(error),
    }
}

fn validate_provider_metadata(metadata: EncryptionProviderKeyMetadata) -> Result<()> {
    if metadata.key_id.0.iter().all(|byte| *byte == 0)
        || metadata.key_epoch == 0
        || metadata.minimum_database_key_epoch == 0
    {
        return Err(Error::new(
            ErrorKind::Recovery,
            "encryption provider key id, provider epoch, and database-key floor must be non-zero",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub enum Encryption {
    #[default]
    None,
    Aes256GcmSiv {
        key: EncryptionKey,
    },
    Aes256GcmSivProvider {
        provider: Arc<dyn EncryptionKeyProvider>,
    },
}

impl Encryption {
    #[must_use]
    pub fn aes256_gcm_siv(key: EncryptionKey) -> Self {
        Self::Aes256GcmSiv { key }
    }

    #[must_use]
    pub fn aes256_gcm_siv_with_provider(provider: Arc<dyn EncryptionKeyProvider>) -> Self {
        Self::Aes256GcmSivProvider { provider }
    }

    fn resolve_kernel(
        &self,
        path: &Path,
        operation: EncryptionKeyOperation,
    ) -> Result<kernel_plan::StorageEncryption> {
        match self {
            Self::None => Ok(kernel_plan::StorageEncryption::None),
            Self::Aes256GcmSiv { key } => Ok(kernel_plan::StorageEncryption::aes256_gcm_siv(
                key.0.clone(),
            )),
            Self::Aes256GcmSivProvider { provider } => {
                let (key, metadata) = resolve_provider_key(provider.as_ref(), path, operation)?;
                Ok(
                    kernel_plan::StorageEncryption::aes256_gcm_siv_wrapped_with_minimum_database_key_epoch(
                        key,
                        metadata.key_id.0,
                        metadata.key_epoch,
                        metadata.minimum_database_key_epoch,
                    ),
                )
            }
        }
    }
}

#[derive(Clone)]
pub struct DatabaseBuilder {
    path: PathBuf,
    storage: Storage,
    schema: Option<Schema>,
    encryption: Encryption,
    external_freshness: Option<ExternalFreshness>,
    publication_notifier: Option<Arc<dyn crate::PublicationNotifier>>,
}

impl std::fmt::Debug for DatabaseBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DatabaseBuilder")
            .field("path", &self.path)
            .field("storage", &self.storage)
            .field("schema", &self.schema.as_ref().map(|_| "<schema>"))
            .field("encryption", &self.encryption)
            .field("external_freshness", &self.external_freshness)
            .field(
                "publication_notifier",
                &self.publication_notifier.as_ref().map(|_| "<notifier>"),
            )
            .finish()
    }
}

impl DatabaseBuilder {
    #[must_use]
    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = storage;
        self
    }

    #[must_use]
    pub fn schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn encryption(mut self, encryption: Encryption) -> Self {
        self.encryption = encryption;
        self
    }

    #[must_use]
    pub fn external_freshness(mut self, external_freshness: ExternalFreshness) -> Self {
        self.external_freshness = Some(external_freshness);
        self
    }

    #[must_use]
    pub fn publication_notifier(mut self, notifier: Arc<dyn crate::PublicationNotifier>) -> Self {
        self.publication_notifier = Some(notifier);
        self
    }

    fn resolved_storage_for_create(&self) -> Storage {
        match self.storage {
            Storage::Auto if self.path.is_dir() => Storage::Directory,
            Storage::Auto => Storage::SingleFile,
            storage => storage,
        }
    }

    fn resolved_storage_for_open(&self) -> Result<Storage> {
        match self.storage {
            Storage::Auto if self.path.is_file() => Ok(Storage::SingleFile),
            Storage::Auto if self.path.is_dir() => Ok(Storage::Directory),
            Storage::Auto => Err(Error::new(
                ErrorKind::Recovery,
                format!("database path does not exist: {}", self.path.display()),
            )
            .with_recovery_diagnostic(crate::RecoveryDiagnostic::new(
                crate::RecoveryOperation::Open,
                crate::RecoveryAuthority::Storage,
                crate::RecoveryReason::PathUnavailable,
                None,
                None,
            ))),
            storage => Ok(storage),
        }
    }

    fn kernel_backend(storage: Storage) -> kernel_plan::RuntimeDurabilityBackend {
        match storage {
            Storage::SingleFile => kernel_plan::RuntimeDurabilityBackend::SingleFile,
            Storage::Directory => kernel_plan::RuntimeDurabilityBackend::Directory,
            Storage::Auto => unreachable!("storage must be resolved before kernel dispatch"),
        }
    }

    /// Creates a database from one complete authoritative typed schema definition.
    ///
    /// The schema type is consumed only as creation authority; the returned runtime authority is
    /// the ordinary schema-neutral [`Database`]. Consumer contexts bind separately afterwards.
    pub fn create_authoritative<S>(mut self) -> Result<Database>
    where
        S: crate::DatabaseDefinition,
    {
        if self.schema.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidSchema,
                "authoritative typed creation cannot be combined with an explicit dynamic schema",
            ));
        }
        self.schema = Some(S::definition()?);
        self.create()
    }

    pub fn create(self) -> Result<Database> {
        if self.external_freshness.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "external freshness bootstrap is not a database-creation operation",
            ));
        }
        let storage = self.resolved_storage_for_create();
        let encryption = self
            .encryption
            .resolve_kernel(&self.path, EncryptionKeyOperation::Create)?;
        let definition = self.schema.ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                "database creation requires a schema",
            )
        })?;
        let (context, registry, relations) = compile_schema(definition)?;
        let root = build_empty_root(&context, &registry, &relations)?;
        let backend = Self::kernel_backend(storage);
        let storage_options =
            kernel_plan::RuntimeStorageOptions::new(backend).with_encryption(encryption);
        let runtime = if let Some(notifier) = self.publication_notifier {
            let bridge = Arc::new(crate::notification::KernelPublicationNotifierBridge::new(
                notifier,
            ));
            kernel_plan::DurableRuntime::create_with_storage_options_and_revision_publication_notifier(
                root, &self.path, &registry, &storage_options, bridge,
            )
        } else {
            kernel_plan::DurableRuntime::create_with_storage_options(
                root,
                &self.path,
                &registry,
                &storage_options,
            )
        }
        .map_err(|error| Error::new(ErrorKind::Recovery, format!("create failed: {error:?}")))?;
        Ok(Database::from_runtime(runtime))
    }

    pub fn open(self) -> Result<Database> {
        if self.schema.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidSchema,
                "schema configuration is only valid when creating a database",
            ));
        }
        let storage = self.resolved_storage_for_open()?;
        let encryption = self
            .encryption
            .resolve_kernel(&self.path, EncryptionKeyOperation::Open)?;
        let backend = Self::kernel_backend(storage);
        let storage_options =
            kernel_plan::RuntimeStorageOptions::new(backend).with_encryption(encryption);
        let runtime = if let Some(external_freshness) = self.external_freshness {
            if storage != Storage::SingleFile {
                return Err(Error::new(
                    ErrorKind::InvalidPlan,
                    "external freshness currently requires single-file storage",
                ));
            }
            let (config, authority) = external_freshness.resolve_kernel()?;
            let notifier: Arc<dyn kernel_plan::RuntimeRevisionPublicationNotifier> =
                if let Some(notifier) = self.publication_notifier {
                    Arc::new(crate::notification::KernelPublicationNotifierBridge::new(notifier))
                } else {
                    Arc::new(kernel_plan::InProcessRevisionPublicationNotifier::default())
                };
            kernel_plan::DurableRuntime::open_single_file_with_external_freshness_and_encryption_and_revision_publication_notifier(
                &self.path,
                config,
                authority,
                &storage_options.encryption,
                notifier,
            )
        } else if let Some(notifier) = self.publication_notifier {
            let bridge = Arc::new(crate::notification::KernelPublicationNotifierBridge::new(notifier));
            kernel_plan::DurableRuntime::open_with_storage_options_recovery_policy_and_revision_publication_notifier(
                &self.path,
                &storage_options,
                kernel_plan::PhysicalRecoveryPolicy::default(),
                bridge,
            )
            .map(|(runtime, _)| runtime)
        } else {
            kernel_plan::DurableRuntime::open_with_storage_options(&self.path, &storage_options)
        }
        .map_err(|error| {
            Error::from_runtime_recovery(
                crate::RecoveryOperation::Open,
                format!("open failed: {error:?}"),
                &error,
            )
        })?;
        Ok(Database::from_runtime(runtime))
    }
}

static NEXT_DATABASE_IDENTITY: AtomicU64 = AtomicU64::new(1);

fn next_database_identity() -> u64 {
    NEXT_DATABASE_IDENTITY.fetch_add(1, Ordering::Relaxed)
}

fn equivalence_module(module: PrimitiveEquivalence) -> kernel_semantics::EquivalenceModule {
    match module {
        PrimitiveEquivalence::UnitExact => kernel_semantics::EquivalenceModule::UnitExact,
        PrimitiveEquivalence::BoolExact => kernel_semantics::EquivalenceModule::BoolExact,
        PrimitiveEquivalence::I64Exact => kernel_semantics::EquivalenceModule::I64Exact,
        PrimitiveEquivalence::F64Bitwise => kernel_semantics::EquivalenceModule::F64Bitwise,
        PrimitiveEquivalence::TextExact => kernel_semantics::EquivalenceModule::TextExact,
        PrimitiveEquivalence::TextAsciiCaseInsensitive => {
            kernel_semantics::EquivalenceModule::TextAsciiCaseInsensitive
        }
        PrimitiveEquivalence::LiveEntityIdExact(entity_type) => {
            kernel_semantics::EquivalenceModule::LiveEntityIdExact(entity_type.into())
        }
        PrimitiveEquivalence::HistoricalEntityIdExact(entity_type) => {
            kernel_semantics::EquivalenceModule::HistoricalEntityIdExact(entity_type.into())
        }
    }
}

fn ordering_module(module: PrimitiveOrdering) -> kernel_semantics::OrderingModule {
    match module {
        PrimitiveOrdering::UnitExact => kernel_semantics::OrderingModule::UnitExact,
        PrimitiveOrdering::BoolAscending => kernel_semantics::OrderingModule::BoolAscending,
        PrimitiveOrdering::I64Ascending => kernel_semantics::OrderingModule::I64Ascending,
        PrimitiveOrdering::F64Total => kernel_semantics::OrderingModule::F64Total,
        PrimitiveOrdering::TextBinary => kernel_semantics::OrderingModule::TextBinary,
        PrimitiveOrdering::TextAsciiCaseInsensitive => {
            kernel_semantics::OrderingModule::TextAsciiCaseInsensitive
        }
        PrimitiveOrdering::TextAsciiCaseInsensitiveThenBinary => {
            kernel_semantics::OrderingModule::TextAsciiCaseInsensitiveThenBinary
        }
        PrimitiveOrdering::LiveEntityIdAscending(entity_type) => {
            kernel_semantics::OrderingModule::LiveEntityIdAscending(entity_type.into())
        }
        PrimitiveOrdering::HistoricalEntityIdAscending(entity_type) => {
            kernel_semantics::OrderingModule::HistoricalEntityIdAscending(entity_type.into())
        }
    }
}

fn relation_semantics(value: &RelationSemantics) -> kernel_schema::RelationSemantics {
    match value {
        RelationSemantics::Bag {
            column_equivalences,
        } => kernel_schema::RelationSemantics::Bag {
            column_equivalences: column_equivalences
                .iter()
                .copied()
                .map(Into::into)
                .collect(),
        },
        RelationSemantics::Set {
            column_equivalences,
        } => kernel_schema::RelationSemantics::Set {
            column_equivalences: column_equivalences
                .iter()
                .copied()
                .map(Into::into)
                .collect(),
        },
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep complete schema compiler case analysis together."
)]
pub(crate) fn compile_schema(
    definition: Schema,
) -> Result<(
    kernel_schema::SemanticContext,
    kernel_semantics::SemanticRegistry,
    BTreeMap<crate::RelationId, crate::RelationSchema>,
)> {
    let Schema {
        revision,
        environment_revision,
        equivalences,
        structural_equivalences,
        orderings,
        relations,
        owned_relationships,
        entity_fields,
        field_rules,
        relation_column_rules,
        entity_rules,
        model_rules,
        access_capabilities,
        access_roles,
    } = definition;
    let mut registry = kernel_semantics::SemanticRegistry::default();
    let mut environment = kernel_schema::SemanticEnvironment::new(
        kernel_types::SemanticEnvId::new(environment_revision),
    );
    for (id, module) in equivalences {
        environment.pin_module(
            id.into(),
            registry.install_equivalence(equivalence_module(module)),
        );
    }
    for (id, module) in orderings {
        environment.pin_module(
            id.into(),
            registry.install_ordering(ordering_module(module)),
        );
    }
    let mut schema = kernel_schema::Schema::new(kernel_types::SchemaRevisionId::new(revision));
    for (id, definition) in structural_equivalences {
        let definition = match definition {
            StructuralEquivalence::Option { inner } => {
                kernel_schema::StructuralEquivalenceDef::Option {
                    inner: inner.into(),
                }
            }
        };
        schema
            .define_structural_equivalence(id.into(), definition)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid structural equivalence: {error:?}"),
                )
            })?;
    }
    for (id, (owner, ty)) in entity_fields {
        schema
            .define_field(kernel_schema::FieldDef {
                id: id.into(),
                owner: owner.into(),
                value: type_to_kernel(&ty),
            })
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid object field schema: {error:?}"),
                )
            })?;
    }
    for (field, rules) in field_rules {
        for rule in rules {
            let rule = match rule {
                crate::FieldRule::I64Range { min, max } => {
                    kernel_schema::FieldRule::I64Range { min, max }
                }
                crate::FieldRule::TextLength { min, max } => {
                    kernel_schema::FieldRule::TextLength { min, max }
                }
                crate::FieldRule::TextOneOf(values) => kernel_schema::FieldRule::TextOneOf(values),
                crate::FieldRule::TextMatches(pattern) => {
                    kernel_schema::FieldRule::TextMatches(text_pattern_to_kernel(pattern))
                }
                crate::FieldRule::Expr(expression) => {
                    kernel_schema::FieldRule::Expr(semantic_rule_to_kernel(expression))
                }
            };
            schema.add_field_rule(field.into(), rule).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid field rule: {error:?}"),
                )
            })?;
        }
    }
    for (owner, rules) in entity_rules {
        for rule in rules {
            schema
                .add_entity_rule(owner.into(), semantic_rule_to_kernel(rule))
                .map_err(|error| {
                    Error::new(
                        ErrorKind::InvalidSchema,
                        format!("invalid entity rule: {error:?}"),
                    )
                })?;
        }
    }
    for relation in relations.values() {
        schema
            .define_relation_with_column_ids(
                kernel_schema::RelationDef {
                    id: relation.id().into(),
                    columns: relation.columns().iter().map(type_to_kernel).collect(),
                    semantics: relation_semantics(relation.semantics()),
                },
                relation
                    .column_ids()
                    .iter()
                    .copied()
                    .map(Into::into)
                    .collect(),
            )
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid relation schema: {error:?}"),
                )
            })?;
    }
    for (relation, (target_relation, orphan_policy)) in owned_relationships {
        schema
            .define_owned_relationship(kernel_schema::OwnedRelationshipDef {
                relation: relation.into(),
                target_relation: target_relation.into(),
                orphan_policy: match orphan_policy {
                    crate::OrphanPolicy::Keep => kernel_schema::OrphanPolicyDef::Keep,
                    crate::OrphanPolicy::DeleteIfUnowned => {
                        kernel_schema::OrphanPolicyDef::DeleteIfUnowned
                    }
                },
            })
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid owned relationship schema: {error:?}"),
                )
            })?;
    }

    let mut schema_access = kernel_schema::SchemaAccess::default();
    for (id, capability) in access_capabilities {
        schema_access.capabilities.insert(
            kernel_types::SemanticId::new(id.raw()),
            kernel_schema::AccessCapabilityDef {
                id: kernel_types::SemanticId::new(id.raw()),
                permissions: capability
                    .permissions()
                    .map(crate::security::permission_coordinate_to_kernel)
                    .collect(),
            },
        );
    }
    for (id, role) in access_roles {
        schema_access.roles.insert(
            kernel_types::SemanticId::new(id.raw()),
            kernel_schema::AccessRoleDef {
                id: kernel_types::SemanticId::new(id.raw()),
                capabilities: role
                    .capabilities()
                    .map(|id| kernel_types::SemanticId::new(id.raw()))
                    .collect(),
                includes: role
                    .includes()
                    .map(|id| kernel_types::SemanticId::new(id.raw()))
                    .collect(),
            },
        );
    }
    schema.set_schema_access(schema_access).map_err(|error| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("invalid schema access: {error:?}"),
        )
    })?;

    for ((relation, column), rules) in relation_column_rules {
        for rule in rules {
            let rule = match rule {
                crate::FieldRule::I64Range { min, max } => {
                    kernel_schema::FieldRule::I64Range { min, max }
                }
                crate::FieldRule::TextLength { min, max } => {
                    kernel_schema::FieldRule::TextLength { min, max }
                }
                crate::FieldRule::TextOneOf(values) => kernel_schema::FieldRule::TextOneOf(values),
                crate::FieldRule::TextMatches(pattern) => {
                    kernel_schema::FieldRule::TextMatches(text_pattern_to_kernel(pattern))
                }
                crate::FieldRule::Expr(expression) => {
                    kernel_schema::FieldRule::Expr(semantic_rule_to_kernel(expression))
                }
            };
            let relation_id: kernel_types::SemanticId = relation.into();
            let column_id = schema
                .relation_column_id(relation_id, column)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidSchema,
                        format!("relation column rule references unknown column {column}"),
                    )
                })?;
            schema
                .add_relation_column_rule(relation_id, column_id, rule)
                .map_err(|error| {
                    Error::new(
                        ErrorKind::InvalidSchema,
                        format!("invalid object field rule: {error:?}"),
                    )
                })?;
        }
    }
    for rule in model_rules {
        let rule = match rule {
            crate::ModelRuleExpr::RelationExactCountRange {
                relation,
                predicate,
                min,
                max,
            } => kernel_schema::ModelRuleExpr::RelationExactMeasure {
                constraint: kernel_schema::ExactMeasureConstraint::Range {
                    measure: kernel_schema::ExactAggregateMeasureExpr::Count {
                        relation: relation.into(),
                        predicate: semantic_rule_to_kernel(predicate),
                    },
                    range: kernel_schema::ExactAggregateRange::Count { min, max },
                },
            },
            crate::ModelRuleExpr::RelationExactF64SumRange {
                relation,
                column,
                predicate,
                min,
                max,
            } => kernel_schema::ModelRuleExpr::RelationExactMeasure {
                constraint: kernel_schema::ExactMeasureConstraint::Range {
                    measure: kernel_schema::ExactAggregateMeasureExpr::F64Sum {
                        relation: relation.into(),
                        column: column.into(),
                        predicate: semantic_rule_to_kernel(predicate),
                    },
                    range: kernel_schema::ExactAggregateRange::F64Sum {
                        min: min.map(|value| {
                            kernel_schema::FiniteF64::from_bits(value.bits())
                                .expect("runtime FiniteF64 is already normalized and finite")
                        }),
                        max: max.map(|value| {
                            kernel_schema::FiniteF64::from_bits(value.bits())
                                .expect("runtime FiniteF64 is already normalized and finite")
                        }),
                    },
                },
            },
            crate::ModelRuleExpr::ExactAggregateCompare {
                left,
                right,
                comparison,
            } => kernel_schema::ModelRuleExpr::RelationExactMeasure {
                constraint: kernel_schema::ExactMeasureConstraint::Compare {
                    left: exact_aggregate_measure_to_kernel(left),
                    right: exact_aggregate_measure_to_kernel(right),
                    comparison: match comparison {
                        crate::RuleOrderComparison::Less => {
                            kernel_schema::RuleOrderComparison::Less
                        }
                        crate::RuleOrderComparison::LessOrEqual => {
                            kernel_schema::RuleOrderComparison::LessOrEqual
                        }
                        crate::RuleOrderComparison::Greater => {
                            kernel_schema::RuleOrderComparison::Greater
                        }
                        crate::RuleOrderComparison::GreaterOrEqual => {
                            kernel_schema::RuleOrderComparison::GreaterOrEqual
                        }
                    },
                },
            },
            crate::ModelRuleExpr::ExactOrderedStatisticRange { measure, min, max } => {
                kernel_schema::ModelRuleExpr::RelationExactMeasure {
                    constraint: kernel_schema::ExactMeasureConstraint::Range {
                        measure: exact_aggregate_measure_to_kernel(measure),
                        range: kernel_schema::ExactAggregateRange::OrderedStatistic {
                            min: min.map(ordered_statistic_bound_to_kernel),
                            max: max.map(ordered_statistic_bound_to_kernel),
                        },
                    },
                }
            }
            crate::ModelRuleExpr::RelationGroupedExactCountRange {
                relation,
                group_columns,
                group_equivalences,
                predicate,
                min,
                max,
            } => kernel_schema::ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns: group_columns.into_iter().map(Into::into).collect(),
                group_equivalences: group_equivalences.into_iter().map(Into::into).collect(),
                constraint: kernel_schema::ExactMeasureConstraint::Range {
                    measure: kernel_schema::ExactAggregateMeasureExpr::Count {
                        relation: relation.into(),
                        predicate: semantic_rule_to_kernel(predicate),
                    },
                    range: kernel_schema::ExactAggregateRange::Count { min, max },
                },
            },
            crate::ModelRuleExpr::RelationGroupedExactF64SumRange {
                relation,
                group_columns,
                group_equivalences,
                column,
                predicate,
                min,
                max,
            } => kernel_schema::ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns: group_columns.into_iter().map(Into::into).collect(),
                group_equivalences: group_equivalences.into_iter().map(Into::into).collect(),
                constraint: kernel_schema::ExactMeasureConstraint::Range {
                    measure: kernel_schema::ExactAggregateMeasureExpr::F64Sum {
                        relation: relation.into(),
                        column: column.into(),
                        predicate: semantic_rule_to_kernel(predicate),
                    },
                    range: kernel_schema::ExactAggregateRange::F64Sum {
                        min: min.map(|value| {
                            kernel_schema::FiniteF64::from_bits(value.bits())
                                .expect("runtime FiniteF64 is already normalized and finite")
                        }),
                        max: max.map(|value| {
                            kernel_schema::FiniteF64::from_bits(value.bits())
                                .expect("runtime FiniteF64 is already normalized and finite")
                        }),
                    },
                },
            },
            crate::ModelRuleExpr::RelationGroupedExactOrderedStatisticRange {
                relation: _,
                group_columns,
                group_equivalences,
                measure,
                min,
                max,
            } => kernel_schema::ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns: group_columns.into_iter().map(Into::into).collect(),
                group_equivalences: group_equivalences.into_iter().map(Into::into).collect(),
                constraint: kernel_schema::ExactMeasureConstraint::Range {
                    measure: exact_aggregate_measure_to_kernel(measure),
                    range: kernel_schema::ExactAggregateRange::OrderedStatistic {
                        min: min.map(ordered_statistic_bound_to_kernel),
                        max: max.map(ordered_statistic_bound_to_kernel),
                    },
                },
            },
            crate::ModelRuleExpr::RelationGroupedExactAggregateCompare {
                relation: _,
                group_columns,
                group_equivalences,
                left,
                right,
                comparison,
            } => kernel_schema::ModelRuleExpr::RelationGroupedExactMeasure {
                group_columns: group_columns.into_iter().map(Into::into).collect(),
                group_equivalences: group_equivalences.into_iter().map(Into::into).collect(),
                constraint: kernel_schema::ExactMeasureConstraint::Compare {
                    left: exact_aggregate_measure_to_kernel(left),
                    right: exact_aggregate_measure_to_kernel(right),
                    comparison: match comparison {
                        crate::RuleOrderComparison::Less => {
                            kernel_schema::RuleOrderComparison::Less
                        }
                        crate::RuleOrderComparison::LessOrEqual => {
                            kernel_schema::RuleOrderComparison::LessOrEqual
                        }
                        crate::RuleOrderComparison::Greater => {
                            kernel_schema::RuleOrderComparison::Greater
                        }
                        crate::RuleOrderComparison::GreaterOrEqual => {
                            kernel_schema::RuleOrderComparison::GreaterOrEqual
                        }
                    },
                },
            },
        };
        schema.add_model_rule(rule).map_err(|error| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("invalid model rule: {error:?}"),
            )
        })?;
    }

    let context = kernel_schema::SemanticContext {
        schema,
        environment,
    };
    context.validate().map_err(|error| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("invalid semantic context: {error:?}"),
        )
    })?;
    Ok((context, registry, relations))
}

fn compile_migration_program(
    source: &kernel_revision::Revision,
    model: &crate::MigrationModel,
) -> Result<kernel_transport::SchemaMigrationProgram> {
    let (target_context, _, _) = compile_schema(model.target())?;
    let field_rewrites = model
        .field_rules()
        .iter()
        .map(|rule| {
            let mut sources = BTreeSet::new();
            rule.value.source_fields(&mut sources);
            kernel_transport::MigrationFieldRewrite {
                source_fields: sources.into_iter().map(Into::into).collect(),
                target_field: rule.target.into(),
                transform: kernel_query::ExactQuery::new(rule.value.to_kernel()),
            }
        })
        .collect::<Vec<_>>();
    let relation_rewrites = model
        .relation_rules()
        .iter()
        .map(|rule| compile_migration_relation_rewrite(source, &target_context, rule))
        .collect::<crate::Result<Vec<_>>>()?;
    let program = kernel_transport::SchemaMigrationProgram::new(
        target_context,
        field_rewrites,
        relation_rewrites,
    );
    Ok(program)
}

fn verify_migration_security_approval(
    database_identity: u64,
    authority: &ControlAuthority,
    impact: &crate::MigrationSecurityImpact,
    approval: Option<&crate::MigrationSecurityApproval>,
) -> Result<()> {
    if matches!(authority, ControlAuthority::Unrestricted) {
        return Ok(());
    }
    match (impact.requires_security_approval(), approval) {
        (true, None) => Err(Error::new(
            ErrorKind::PermissionDenied,
            "migration security approval is required for this sealed impact",
        )),
        (_, Some(approval)) => approval.verify_for(database_identity, impact),
        (false, None) => Ok(()),
    }
}

fn migration_model_rule_relations(
    rule: &kernel_schema::ModelRuleExpr,
) -> BTreeSet<kernel_types::SemanticId> {
    fn measure(
        out: &mut BTreeSet<kernel_types::SemanticId>,
        value: &kernel_schema::ExactAggregateMeasureExpr,
    ) {
        let relation = match value {
            kernel_schema::ExactAggregateMeasureExpr::Count { relation, .. }
            | kernel_schema::ExactAggregateMeasureExpr::F64Sum { relation, .. }
            | kernel_schema::ExactAggregateMeasureExpr::OrderedStatistic { relation, .. } => {
                *relation
            }
        };
        out.insert(relation);
    }

    fn constraint(
        out: &mut BTreeSet<kernel_types::SemanticId>,
        value: &kernel_schema::ExactMeasureConstraint,
    ) {
        match value {
            kernel_schema::ExactMeasureConstraint::Range { measure: value, .. } => {
                measure(out, value);
            }
            kernel_schema::ExactMeasureConstraint::Compare { left, right, .. } => {
                measure(out, left);
                measure(out, right);
            }
        }
    }

    let mut out = BTreeSet::new();
    match rule {
        kernel_schema::ModelRuleExpr::RelationExactMeasure { constraint: value }
        | kernel_schema::ModelRuleExpr::RelationGroupedExactMeasure {
            constraint: value, ..
        } => constraint(&mut out, value),
    }
    out
}

fn migration_semantic_rule_fields(
    rule: &kernel_schema::SemanticRuleExpr,
) -> (BTreeSet<kernel_types::SemanticId>, bool) {
    fn value(
        value: &kernel_schema::RuleValueExpr,
        fields: &mut BTreeSet<kernel_types::SemanticId>,
        uses_input: &mut bool,
    ) {
        match value {
            kernel_schema::RuleValueExpr::Input => *uses_input = true,
            kernel_schema::RuleValueExpr::Field(field) => {
                fields.insert(*field);
            }
        }
    }
    fn visit(
        rule: &kernel_schema::SemanticRuleExpr,
        fields: &mut BTreeSet<kernel_types::SemanticId>,
        uses_input: &mut bool,
    ) {
        use kernel_schema::SemanticRuleExpr as R;
        match rule {
            R::True | R::False => {}
            R::And(rules) | R::Or(rules) => {
                for rule in rules {
                    visit(rule, fields, uses_input);
                }
            }
            R::Not(rule) => visit(rule, fields, uses_input),
            R::I64Range { value: item, .. }
            | R::TextLength { value: item, .. }
            | R::TextOneOf { value: item, .. }
            | R::TextMatches { value: item, .. } => value(item, fields, uses_input),
            R::Equivalent { left, right, .. } | R::Ordered { left, right, .. } => {
                value(left, fields, uses_input);
                value(right, fields, uses_input);
            }
        }
    }
    let mut fields = BTreeSet::new();
    let mut uses_input = false;
    visit(rule, &mut fields, &mut uses_input);
    (fields, uses_input)
}

fn migration_integrity_policy_changes(
    source: &kernel_schema::Schema,
    target: &kernel_schema::Schema,
) -> Vec<crate::MigrationIntegrityPolicyCoordinate> {
    let mut out = BTreeSet::new();
    let fields: BTreeSet<_> = source
        .fields()
        .map(|f| f.id)
        .chain(target.fields().map(|f| f.id))
        .collect();
    for field in fields {
        if source.field_rules(field) != target.field_rules(field) {
            out.insert(crate::MigrationIntegrityPolicyCoordinate::Field(
                crate::FieldId::new(field.raw()),
            ));
        }
    }
    let entities: BTreeSet<_> = source
        .all_entity_rules()
        .map(|(id, _)| id)
        .chain(target.all_entity_rules().map(|(id, _)| id))
        .collect();
    for entity in entities {
        if source.entity_rules(entity) != target.entity_rules(entity) {
            out.insert(crate::MigrationIntegrityPolicyCoordinate::Entity(
                crate::TypeId::new(entity.raw()),
            ));
        }
    }
    let columns: BTreeSet<_> = source
        .all_relation_column_rules()
        .map(|(coord, _)| coord)
        .chain(target.all_relation_column_rules().map(|(coord, _)| coord))
        .collect();
    for (relation, column) in columns {
        if source.relation_column_rules_by_id(relation, column)
            != target.relation_column_rules_by_id(relation, column)
        {
            out.insert(crate::MigrationIntegrityPolicyCoordinate::RelationColumn {
                relation: crate::RelationId::new(relation.raw()),
                column: crate::RelationColumnId::new(column.raw()),
            });
        }
    }
    if source.model_rules() != target.model_rules() {
        out.insert(crate::MigrationIntegrityPolicyCoordinate::Model);
    }
    out.into_iter().collect()
}

fn build_migration_security_impact(
    source: &kernel_revision::Revision,
    model: &crate::MigrationModel,
    program: &kernel_transport::SchemaMigrationProgram,
    transport: &kernel_transport::SchemaMigrationTransport,
) -> Result<crate::MigrationSecurityImpact> {
    let mut identity = b"cfmd.migration.security-impact.v1".to_vec();
    identity.extend_from_slice(&source.id().raw().to_le_bytes());
    identity.extend_from_slice(&model.id().to_le_bytes());
    identity.extend_from_slice(
        &kernel_durability::canonical_schema_migration_program_identity(program).map_err(
            |error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("migration identity encoding failed: {error:?}"),
                )
            },
        )?,
    );
    let digest = crate::MigrationSecurityImpactDigest::from_bytes(kernel_auth::sha256(&identity).0);

    let certification = transport.certify_access_noninterference();
    let mut source_dependencies = BTreeSet::new();
    source_dependencies.extend(
        transport
            .source_field_dependencies()
            .into_iter()
            .map(|id| crate::MigrationDataDependency::Field(crate::FieldId::new(id.raw()))),
    );
    source_dependencies.extend(
        transport
            .source_relation_dependencies()
            .into_iter()
            .map(|id| crate::MigrationDataDependency::Relation(crate::RelationId::new(id.raw()))),
    );

    let validation_dependencies = migration_validation_dependencies(transport);
    source_dependencies.extend(validation_dependencies.iter().copied());

    Ok(crate::MigrationSecurityImpact::from_kernel(
        RevisionId::new(source.id().raw()),
        model.id(),
        digest,
        transport.access_policy_changes(),
        &certification,
        crate::migration::MigrationSecurityImpactParts {
            source_data_dependencies: source_dependencies.into_iter().collect(),
            data_dependent_validation_dependencies: validation_dependencies.into_iter().collect(),
            integrity_policy_changes: migration_integrity_policy_changes(
                &source.semantic_context().schema,
                &transport.target().schema,
            ),
        },
    ))
}

fn migration_validation_dependencies(
    transport: &kernel_transport::SchemaMigrationTransport,
) -> BTreeSet<crate::MigrationDataDependency> {
    let mut validation_dependencies = BTreeSet::new();
    let target_schema = &transport.target().schema;
    for target_field in target_schema.fields() {
        if !target_schema.field_rules(target_field.id).is_empty()
            && let Some(source_fields) = transport.target_field_source_dependencies(target_field.id)
        {
            validation_dependencies.extend(
                source_fields
                    .into_iter()
                    .map(|id| crate::MigrationDataDependency::Field(crate::FieldId::new(id.raw()))),
            );
        }
    }
    let entity_owners: BTreeSet<_> = target_schema
        .all_entity_rules()
        .map(|(owner, _)| owner)
        .collect();
    for owner in entity_owners {
        for rule in target_schema.entity_rules(owner) {
            let (mut target_fields, uses_input) = migration_semantic_rule_fields(rule);
            if uses_input {
                target_fields.extend(
                    target_schema
                        .fields()
                        .filter(|field| field.owner == owner)
                        .map(|field| field.id),
                );
            }
            for target_field in target_fields {
                if let Some(source_fields) =
                    transport.target_field_source_dependencies(target_field)
                {
                    validation_dependencies.extend(source_fields.into_iter().map(|id| {
                        crate::MigrationDataDependency::Field(crate::FieldId::new(id.raw()))
                    }));
                }
            }
        }
    }
    let target_rule_relations: BTreeSet<_> = target_schema
        .all_relation_column_rules()
        .map(|((relation, _), _)| relation)
        .collect();
    for target_relation in target_rule_relations {
        if let Some(slice) = transport.relation_slice(target_relation) {
            validation_dependencies.extend(slice.source_relations().into_iter().map(|id| {
                crate::MigrationDataDependency::Relation(crate::RelationId::new(id.raw()))
            }));
        }
    }
    for rule in target_schema.model_rules() {
        for target_relation in migration_model_rule_relations(rule) {
            if let Some(slice) = transport.relation_slice(target_relation) {
                for source_relation in slice.source_relations() {
                    validation_dependencies.insert(crate::MigrationDataDependency::Relation(
                        crate::RelationId::new(source_relation.raw()),
                    ));
                }
            }
        }
    }
    validation_dependencies
}

fn compile_migration_relation_rewrite(
    source: &kernel_revision::Revision,
    target: &kernel_schema::SemanticContext,
    rule: &crate::MigrationRelationRule,
) -> Result<kernel_transport::MigrationRelationRewrite> {
    match rule {
        crate::MigrationRelationRule::Query { target, query } => Ok(
            kernel_transport::MigrationRelationRewrite::Query(kernel_transport::RelationRewrite {
                target_relation: (*target).into(),
                transform: query.inner.clone(),
            }),
        ),
        crate::MigrationRelationRule::Rows {
            source: source_relation,
            target: target_relation,
            columns,
        } => compile_migration_row_rewrite(
            source,
            target,
            *source_relation,
            *target_relation,
            columns,
        )
        .map(kernel_transport::MigrationRelationRewrite::Rows),
    }
}

fn compile_migration_row_rewrite(
    source: &kernel_revision::Revision,
    target: &kernel_schema::SemanticContext,
    source_relation: RelationId,
    target_relation: RelationId,
    columns: &[crate::MigrationColumnRule],
) -> Result<kernel_transport::MigrationRowRewrite> {
    let source_relation_id: kernel_types::SemanticId = source_relation.into();
    let target_relation_id: kernel_types::SemanticId = target_relation.into();
    let source_column_ids = source
        .semantic_context()
        .schema
        .relation_column_ids(source_relation_id)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                "migration references unknown source relation",
            )
        })?;
    let target_column_ids = target
        .schema
        .relation_column_ids(target_relation_id)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                "migration references unknown target relation",
            )
        })?;
    let columns = columns
        .iter()
        .map(|column| compile_migration_column(column, source_column_ids, target_column_ids))
        .collect::<Result<Vec<_>>>()?;
    Ok(kernel_transport::MigrationRowRewrite {
        source_relation: source_relation_id,
        target_relation: target_relation_id,
        columns,
    })
}

fn compile_migration_column(
    column: &crate::MigrationColumnRule,
    source_column_ids: &[kernel_types::SemanticId],
    target_column_ids: &[kernel_types::SemanticId],
) -> Result<kernel_transport::MigrationColumnRewrite> {
    let source_columns = column
        .source_columns
        .iter()
        .map(|ordinal| {
            source_column_ids.get(*ordinal).copied().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("migration references unknown source column {ordinal}"),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let target_column = target_column_ids
        .get(column.target_column)
        .copied()
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!(
                    "migration references unknown target column {}",
                    column.target_column
                ),
            )
        })?;
    Ok(kernel_transport::MigrationColumnRewrite {
        source_columns,
        target_column,
        transform: kernel_query::ExactQuery::new(
            column.value.to_kernel_for_relation(source_column_ids),
        ),
    })
}

fn build_empty_root(
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    relations: &BTreeMap<crate::RelationId, crate::RelationSchema>,
) -> Result<kernel_plan::RuntimeRevisionBundle> {
    let mut model = kernel_model::FiniteModel::default();
    let mut physical = kernel_plan::PhysicalStore::default();
    let mut layouts = BTreeMap::new();
    for (index, relation) in relations.values().enumerate() {
        let relation_id: kernel_types::SemanticId = relation.id().into();
        model.relations.insert(relation_id, Vec::new());
        let layout = kernel_plan::LayoutBinding {
            id: kernel_plan::LayoutId(u128::try_from(index + 1).expect("usize fits u128")),
            family: kernel_plan::LayoutFamily::Columnar,
        };
        let column_types = relation
            .columns()
            .iter()
            .map(type_to_kernel)
            .collect::<Vec<_>>();
        let empty =
            kernel_plan::NativeRelation::typed_from_rows(&[], &column_types).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("failed to create typed empty relation: {error:?}"),
                )
            })?;
        physical
            .install(relation_id, layout, empty)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("failed to install relation layout: {error:?}"),
                )
            })?;
        layouts.insert(relation_id, layout);
    }
    let revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        context,
        registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .map_err(|error| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("failed to build root revision: {error:?}"),
        )
    })?;
    kernel_plan::RuntimeRevisionBundle::build(revision, physical, layouts, &[], registry).map_err(
        |error| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("failed to build runtime root: {error:?}"),
            )
        },
    )
}

const ENTITY_FNV128_OFFSET: u128 = 144_066_263_297_769_815_596_495_629_667_062_367_629;
const ENTITY_FNV128_PRIME: u128 = 309_485_009_821_345_068_724_781_371;

fn entity_hash_bytes(mut hash: u128, bytes: &[u8]) -> u128 {
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(ENTITY_FNV128_PRIME);
    }
    hash
}

pub(crate) fn lifecycle_entity_id(entity_type: crate::TypeId, raw: u128) -> kernel_types::EntityId {
    let hash = entity_hash_bytes(ENTITY_FNV128_OFFSET, b"cfmd.object.lifecycle-id.v1\0");
    let hash = entity_hash_bytes(hash, &entity_type.raw().to_le_bytes());
    let hash = entity_hash_bytes(hash, &raw.to_le_bytes());
    kernel_types::EntityId::new(hash)
}

fn row_identity(
    row: &crate::Row,
    column: usize,
    expected: crate::TypeId,
) -> Result<kernel_types::EntityId> {
    match row.get(column) {
        Some(crate::Value::HistoricalEntityRef(value)) if value.entity_type == expected => {
            Ok(lifecycle_entity_id(expected, value.id))
        }
        _ => Err(Error::new(
            ErrorKind::InvariantViolation,
            "object identity column does not contain its declared typed identity",
        )),
    }
}

fn mirrored_reference_value(
    value: &crate::Value,
    reference: &crate::plan::ReferenceContract,
) -> Result<kernel_model::Value> {
    mirrored_reference_value_for_role(value, reference.target_type, reference.optional)
}

pub(crate) fn mirrored_reference_value_for_role(
    value: &crate::Value,
    target_type: crate::TypeId,
    optional: bool,
) -> Result<kernel_model::Value> {
    let live = |value: crate::EntityRef| kernel_model::Value::LiveEntityRef {
        entity_type: target_type.into(),
        id: lifecycle_entity_id(target_type, value.id),
    };
    match (value, optional) {
        (crate::Value::HistoricalEntityRef(value), false) if value.entity_type == target_type => {
            Ok(live(*value))
        }
        (crate::Value::Option(None), true) => Ok(kernel_model::Value::Option(None)),
        (crate::Value::Option(Some(value)), true) => match value.as_ref() {
            crate::Value::HistoricalEntityRef(value) if value.entity_type == target_type => {
                Ok(kernel_model::Value::Option(Some(Box::new(live(*value)))))
            }
            _ => Err(Error::new(
                ErrorKind::InvariantViolation,
                "optional object reference has incompatible target identity",
            )),
        },
        _ => Err(Error::new(
            ErrorKind::InvariantViolation,
            "object reference column does not match its declared cardinality",
        )),
    }
}

fn apply_relation_mutations_to_state(
    plan: &Plan,
    state: &mut kernel_model::DatabaseState,
) -> Result<()> {
    for (relation, mutation) in &plan.mutations {
        let rows = state
            .model
            .relations
            .get_mut(&(*relation).into())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    format!("plan targets unknown relation {relation:?}"),
                )
            })?;
        for removed in &mutation.removed {
            let removed = removed
                .iter()
                .cloned()
                .map(Into::into)
                .collect::<Vec<kernel_model::Value>>();
            let index = rows.iter().position(|row| row == &removed).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "object plan removes a row that is not present in its source revision",
                )
            })?;
            rows.remove(index);
        }
        rows.extend(
            mutation
                .inserted
                .iter()
                .cloned()
                .map(|row| row.into_iter().map(Into::into).collect()),
        );
    }
    Ok(())
}

fn apply_object_field_patches_to_state(
    plan: &Plan,
    state: &mut kernel_model::DatabaseState,
) -> Result<()> {
    for ((relation, _identity_raw), patch) in &plan.object_field_patches {
        let rows = state
            .model
            .relations
            .get_mut(&(*relation).into())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    format!("field patch targets unknown relation {relation:?}"),
                )
            })?;
        let identity: kernel_model::Value = patch.identity_value.clone().into();
        let mut matches = rows
            .iter_mut()
            .filter(|row| row.get(patch.identity_column) == Some(&identity));
        let row = matches.next().ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                "object field patch identity is not present in its source revision",
            )
        })?;
        if matches.next().is_some() {
            return Err(Error::new(
                ErrorKind::Cardinality,
                "object field patch identity matched more than one row",
            ));
        }
        for (column, (value, _field)) in &patch.fields {
            let slot = row.get_mut(*column).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    "object field patch column is outside the persisted relation row",
                )
            })?;
            *slot = value.clone().into();
        }
    }
    Ok(())
}

fn refresh_entity_projection(plan: &Plan, state: &mut kernel_model::DatabaseState) -> Result<()> {
    // Clear the old projection for every object relation touched by this Plan.
    // The relation rows are canonical; carrier/lifecycle/reference-field state
    // is rebuilt from their exact final endpoint.
    for contract in plan.object_contracts.values() {
        let entity_type: kernel_types::SemanticId = contract.entity_type.into();
        let previous = state
            .model
            .carriers
            .get(&entity_type)
            .cloned()
            .unwrap_or_default();
        for entity in &previous {
            // Product entities are roots while present in their canonical relation.
            state.lifecycle.roots.remove(entity);
            state.lifecycle.entities.remove(entity);
            for reference in &contract.references {
                state
                    .model
                    .fields
                    .remove(&(reference.field.into(), *entity));
            }
        }
        state.model.carriers.remove(&entity_type);
    }

    let mut claimed_entities = state
        .model
        .carriers
        .values()
        .flat_map(|carrier| carrier.iter().copied())
        .collect::<BTreeSet<_>>();

    for contract in plan.object_contracts.values() {
        let relation_id: kernel_types::SemanticId = contract.relation.into();
        let entity_type: kernel_types::SemanticId = contract.entity_type.into();
        let rows = state
            .model
            .relations
            .get(&relation_id)
            .ok_or_else(|| Error::new(ErrorKind::InvalidPlan, "object relation is missing"))?;
        let mut carrier = BTreeSet::new();
        for kernel_row in rows {
            let row = kernel_row
                .iter()
                .cloned()
                .map(Into::into)
                .collect::<crate::Row>();
            let entity = row_identity(&row, contract.identity_column, contract.identity_type)?;
            if !carrier.insert(entity) {
                return Err(Error::new(
                    ErrorKind::InvariantViolation,
                    "object relation contains duplicate identity values",
                ));
            }
            if !claimed_entities.insert(entity) {
                return Err(Error::new(
                    ErrorKind::InvariantViolation,
                    "object identity collides with another live entity carrier",
                ));
            }
            state.lifecycle.entities.insert(entity);
            state.lifecycle.roots.insert(entity);

            for reference in &contract.references {
                let value = row.get(reference.column).ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvariantViolation,
                        "object reference column is outside the relation row",
                    )
                })?;
                let value = mirrored_reference_value(value, reference)?;
                state
                    .model
                    .fields
                    .insert((reference.field.into(), entity), value);
            }
        }
        state.model.carriers.insert(entity_type, carrier);
    }
    Ok(())
}

fn owned_target_id(value: &kernel_model::Value) -> Result<kernel_types::EntityId> {
    match value {
        kernel_model::Value::HistoricalEntityId { id, .. } => Ok(*id),
        _ => Err(Error::new(
            ErrorKind::InvariantViolation,
            "owned relationship edge target is not a historical entity identity",
        )),
    }
}

fn apply_owned_relationship_policies(
    plan: &Plan,
    state: &mut kernel_model::DatabaseState,
) -> Result<bool> {
    let mut removed_object_rows = false;
    for contract in plan.owned_relations.values() {
        let final_edges = state
            .model
            .relations
            .get(&contract.relation.into())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned relationship relation is missing",
                )
            })?;
        let mut owners_by_target = BTreeMap::new();
        let mut final_targets = BTreeSet::new();
        for row in final_edges {
            let target = owned_target_id(row.get(1).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned relationship edge is missing target column",
                )
            })?)?;
            let owner = row.first().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned relationship edge is missing owner column",
                )
            })?;
            if let Some(previous) = owners_by_target.insert(target, owner.clone())
                && previous != *owner
            {
                return Err(Error::new(
                    ErrorKind::Cardinality,
                    format!(
                        "exclusive ownership violation: target {} already has another owner",
                        target.raw()
                    ),
                ));
            }
            final_targets.insert(target);
        }

        if contract.orphan_policy != crate::plan::OrphanPolicy::DeleteIfUnowned {
            continue;
        }
        let previous_edges = plan
            .source
            .revision()
            .state()
            .model
            .relations
            .get(&contract.relation.into())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned relationship source relation is missing",
                )
            })?;
        let mut previous_targets = BTreeSet::new();
        for row in previous_edges {
            previous_targets.insert(owned_target_id(row.get(1).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned relationship source edge is missing target column",
                )
            })?)?);
        }
        let orphaned = previous_targets
            .difference(&final_targets)
            .copied()
            .collect::<BTreeSet<_>>();
        if orphaned.is_empty() {
            continue;
        }
        let target_rows = state
            .model
            .relations
            .get_mut(&contract.target_relation.into())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    "owned target object relation is missing",
                )
            })?;
        let before = target_rows.len();
        target_rows.retain(|row| {
            row.get(contract.target_identity_column)
                .and_then(|value| match value {
                    kernel_model::Value::HistoricalEntityId { id, .. } => Some(*id),
                    _ => None,
                })
                .is_none_or(|id| !orphaned.contains(&id))
        });
        removed_object_rows |= target_rows.len() != before;
    }
    Ok(removed_object_rows)
}

pub(crate) fn build_object_target(
    plan: &Plan,
    target_revision: kernel_types::RevisionId,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<kernel_revision::Revision> {
    let context = plan.source.revision().semantic_context();
    let mut state = plan.source.revision().state().clone();
    apply_relation_mutations_to_state(plan, &mut state)?;
    refresh_entity_projection(plan, &mut state)?;
    let normalized = kernel_revision::Revision::build(target_revision, context, registry, state)
        .map_err(|error| {
            Error::new(
                ErrorKind::InvariantViolation,
                format!("object lifecycle transition rejected by kernel: {error:?}"),
            )
        })?;
    let mut state = normalized.state().clone();
    let removed_orphans = apply_owned_relationship_policies(plan, &mut state)?;
    if plan.owned_relations.is_empty() && !removed_orphans {
        return Ok(normalized);
    }
    refresh_entity_projection(plan, &mut state)?;
    kernel_revision::Revision::build(target_revision, context, registry, state).map_err(|error| {
        Error::new(
            ErrorKind::InvariantViolation,
            format!("owned relationship transition rejected by kernel: {error:?}"),
        )
    })
}

pub(crate) fn plan_target_revision_id(plan: &Plan) -> Result<kernel_types::RevisionId> {
    plan.source
        .revision()
        .id()
        .raw()
        .checked_add(1)
        .map(kernel_types::RevisionId::new)
        .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))
}

pub(crate) fn plan_relation_deltas(
    plan: &Plan,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<(kernel_types::SemanticId, kernel_query::RelationDelta)>> {
    if !plan.object_field_patches.is_empty() {
        let target = build_plan_target(plan)?;
        return revision_relation_deltas(plan.source.revision(), &target, registry);
    }
    let mut deltas = Vec::with_capacity(plan.mutations.len());
    for (relation, mutation) in &plan.mutations {
        let expression = kernel_query::RelExpr::Scan((*relation).into());
        let result_type = expression
            .typecheck(plan.source.revision().semantic_context(), registry)
            .map_err(|error| query_error(&error))?;
        deltas.push((
            (*relation).into(),
            kernel_query::RelationDelta {
                inserted: mutation
                    .inserted
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                removed: mutation
                    .removed
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                result_type,
            },
        ));
    }
    Ok(deltas)
}

fn revision_relation_deltas(
    source: &kernel_revision::Revision,
    target: &kernel_revision::Revision,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<(kernel_types::SemanticId, kernel_query::RelationDelta)>> {
    let context = source.semantic_context();
    let relation_ids = source
        .state()
        .model
        .relations
        .keys()
        .chain(target.state().model.relations.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut deltas = Vec::new();
    for relation in relation_ids {
        let expression = kernel_query::RelExpr::Scan(relation);
        let result_type = expression
            .typecheck(context, registry)
            .map_err(|error| query_error(&error))?;
        let relation_value = |rows: Vec<Vec<kernel_model::Value>>| match &result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => kernel_query::RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => kernel_query::RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        };
        let old = relation_value(
            source
                .state()
                .model
                .relations
                .materialize_owned(&relation)
                .unwrap_or_default(),
        );
        let next = relation_value(
            target
                .state()
                .model
                .relations
                .materialize_owned(&relation)
                .unwrap_or_default(),
        );
        let delta = kernel_query::RelationDelta::between_values(
            &old,
            &next,
            result_type,
            context,
            registry,
        )
        .map_err(|error| query_error(&error))?;
        if !delta.is_empty() {
            deltas.push((relation, delta));
        }
    }
    Ok(deltas)
}

fn build_explicit_model_target(
    plan: &Plan,
    target_revision: kernel_types::RevisionId,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<kernel_revision::Revision> {
    let mut state = plan.source.revision().state().clone();
    apply_relation_mutations_to_state(plan, &mut state)?;
    apply_object_field_patches_to_state(plan, &mut state)?;
    plan.model_delta
        .as_ref()
        .expect("explicit model target requires a model delta")
        .apply_to(&mut state);
    kernel_revision::Revision::build(
        target_revision,
        plan.source.revision().semantic_context(),
        registry,
        state,
    )
    .map_err(|error| {
        Error::new(
            ErrorKind::InvariantViolation,
            format!("explicit mixed transition rejected by kernel: {error:?}"),
        )
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep exact plan target construction case analysis together."
)]
pub(crate) fn build_plan_target(plan: &Plan) -> Result<kernel_revision::Revision> {
    let target_revision = plan_target_revision_id(plan)?;
    let registry = &plan.registry;
    if plan.model_delta.is_none()
        && plan.mutations.is_empty()
        && !plan.object_field_patches.is_empty()
    {
        let mut candidate = plan.source.revision().relation_update_candidate();
        for ((relation, _identity_raw), patch) in &plan.object_field_patches {
            let relation_id: kernel_types::SemanticId = (*relation).into();
            let identity: kernel_model::Value = patch.identity_value.clone().into();
            let rows = candidate
                .state()
                .model
                .relations
                .get_shared(&relation_id)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidPlan,
                        format!("field patch targets unknown relation {relation:?}"),
                    )
                })?;
            let mut matching_position = None;
            for (position, row) in rows.iter().enumerate() {
                if row.get(patch.identity_column) == Some(&identity)
                    && matching_position.replace(position).is_some()
                {
                    return Err(Error::new(
                        ErrorKind::Cardinality,
                        "object field patch identity matched more than one row",
                    ));
                }
            }
            let position = matching_position.ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    "object field patch identity is not present in its source revision",
                )
            })?;
            let mut row = rows[position].clone();
            let mut fields = BTreeSet::new();
            for (column, (value, field)) in &patch.fields {
                let slot = row.get_mut(*column).ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidSchema,
                        "object field patch column is outside the persisted relation row",
                    )
                })?;
                *slot = value.clone().into();
                fields.insert(*field);
            }
            candidate
                .patch_relation_rows_with_footprint(
                    relation_id,
                    &[position],
                    vec![row],
                    kernel_validation::RelationMutationFootprint::fields(fields),
                )
                .map_err(|error| {
                    Error::new(
                        ErrorKind::InvariantViolation,
                        format!("field patch target rejected by kernel: {error:?}"),
                    )
                })?;
        }
        return candidate.build(target_revision, registry).map_err(|error| {
            Error::new(
                ErrorKind::InvariantViolation,
                format!("field patch target rejected by kernel: {error:?}"),
            )
        });
    }
    if plan.model_delta.is_some() || !plan.object_field_patches.is_empty() {
        let mut state = plan.source.revision().state().clone();
        apply_relation_mutations_to_state(plan, &mut state)?;
        apply_object_field_patches_to_state(plan, &mut state)?;
        if let Some(model_delta) = &plan.model_delta {
            model_delta.apply_to(&mut state);
        }
        kernel_revision::Revision::build(
            target_revision,
            plan.source.revision().semantic_context(),
            registry,
            state,
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::InvariantViolation,
                format!("field patch target rejected by kernel: {error:?}"),
            )
        })
    } else if plan.object_contracts.is_empty() {
        let deltas = plan_relation_deltas(plan, registry)?;
        let mutations = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: &[],
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        plan.source
            .derive_relation_target_revision(target_revision, &mutations, registry)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    format!("candidate relation endpoint rejected by kernel: {error:?}"),
                )
            })
    } else {
        build_object_target(plan, target_revision, registry)
    }
}

fn collect_requirement_fields(
    expression: &SemanticRuleExpr,
    fields: &mut BTreeSet<crate::FieldId>,
) {
    match expression {
        SemanticRuleExpr::True | SemanticRuleExpr::False => {}
        SemanticRuleExpr::And(parts) | SemanticRuleExpr::Or(parts) => {
            for part in parts {
                collect_requirement_fields(part, fields);
            }
        }
        SemanticRuleExpr::Not(part) => collect_requirement_fields(part, fields),
        SemanticRuleExpr::I64Range { value, .. }
        | SemanticRuleExpr::TextLength { value, .. }
        | SemanticRuleExpr::TextOneOf { value, .. }
        | SemanticRuleExpr::TextMatches { value, .. } => {
            if let crate::RuleValueExpr::Field(field) = value {
                fields.insert(*field);
            }
        }
        SemanticRuleExpr::Equivalent { left, right, .. }
        | SemanticRuleExpr::Ordered { left, right, .. } => {
            for value in [left, right] {
                if let crate::RuleValueExpr::Field(field) = value {
                    fields.insert(*field);
                }
            }
        }
    }
}

fn bind_rule_value_to_input(
    value: &crate::RuleValueExpr,
    field: crate::FieldId,
) -> Option<crate::RuleValueExpr> {
    match value {
        crate::RuleValueExpr::Input => Some(crate::RuleValueExpr::Input),
        crate::RuleValueExpr::Field(candidate) if *candidate == field => {
            Some(crate::RuleValueExpr::Input)
        }
        crate::RuleValueExpr::Field(_) => None,
    }
}

fn bind_single_requirement_field_to_input(
    expression: &SemanticRuleExpr,
    field: crate::FieldId,
) -> Option<SemanticRuleExpr> {
    Some(match expression {
        SemanticRuleExpr::True => SemanticRuleExpr::True,
        SemanticRuleExpr::False => SemanticRuleExpr::False,
        SemanticRuleExpr::And(parts) => SemanticRuleExpr::And(
            parts
                .iter()
                .map(|part| bind_single_requirement_field_to_input(part, field))
                .collect::<Option<Vec<_>>>()?,
        ),
        SemanticRuleExpr::Or(parts) => SemanticRuleExpr::Or(
            parts
                .iter()
                .map(|part| bind_single_requirement_field_to_input(part, field))
                .collect::<Option<Vec<_>>>()?,
        ),
        SemanticRuleExpr::Not(part) => SemanticRuleExpr::Not(Box::new(
            bind_single_requirement_field_to_input(part, field)?,
        )),
        SemanticRuleExpr::I64Range { value, min, max } => SemanticRuleExpr::I64Range {
            value: match value {
                crate::RuleValueExpr::Input => crate::RuleValueExpr::Input,
                crate::RuleValueExpr::Field(candidate) if *candidate == field => {
                    crate::RuleValueExpr::Input
                }
                crate::RuleValueExpr::Field(_) => return None,
            },
            min: *min,
            max: *max,
        },
        SemanticRuleExpr::TextLength { value, min, max } => SemanticRuleExpr::TextLength {
            value: match value {
                crate::RuleValueExpr::Input => crate::RuleValueExpr::Input,
                crate::RuleValueExpr::Field(candidate) if *candidate == field => {
                    crate::RuleValueExpr::Input
                }
                crate::RuleValueExpr::Field(_) => return None,
            },
            min: *min,
            max: *max,
        },
        SemanticRuleExpr::TextOneOf { value, allowed } => SemanticRuleExpr::TextOneOf {
            value: match value {
                crate::RuleValueExpr::Input => crate::RuleValueExpr::Input,
                crate::RuleValueExpr::Field(candidate) if *candidate == field => {
                    crate::RuleValueExpr::Input
                }
                crate::RuleValueExpr::Field(_) => return None,
            },
            allowed: allowed.clone(),
        },
        SemanticRuleExpr::TextMatches { value, pattern } => SemanticRuleExpr::TextMatches {
            value: match value {
                crate::RuleValueExpr::Input => crate::RuleValueExpr::Input,
                crate::RuleValueExpr::Field(candidate) if *candidate == field => {
                    crate::RuleValueExpr::Input
                }
                crate::RuleValueExpr::Field(_) => return None,
            },
            pattern: pattern.clone(),
        },
        SemanticRuleExpr::Equivalent {
            left,
            right,
            equivalence,
        } => SemanticRuleExpr::Equivalent {
            left: bind_rule_value_to_input(left, field)?,
            right: bind_rule_value_to_input(right, field)?,
            equivalence: *equivalence,
        },
        SemanticRuleExpr::Ordered {
            left,
            right,
            ordering,
            comparison,
        } => SemanticRuleExpr::Ordered {
            left: bind_rule_value_to_input(left, field)?,
            right: bind_rule_value_to_input(right, field)?,
            ordering: *ordering,
            comparison: *comparison,
        },
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep semantic requirement validation case analysis together."
)]
fn validate_transaction_requirements(
    plan: &Plan,
    requirements: &[crate::intent_journal::IntentRequirement],
) -> Result<Option<kernel_plan::RuntimeGuardObservationFootprint>> {
    if requirements.is_empty() {
        return Ok(None);
    }
    let target = build_plan_target(plan)?;
    let context = target.semantic_context();
    let source_revision = plan.source.revision().id();
    let mut observations = Vec::new();
    let mut joint_groups = Vec::new();
    for (requirement_index, requirement) in requirements.iter().enumerate() {
        let relation = kernel_types::SemanticId::new(requirement.relation.raw());
        let expression = semantic_rule_to_kernel(requirement.expression.clone());
        context
            .schema
            .validate_relation_row_rule(relation, &expression)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    format!(
                        "transaction requirement is not valid for relation {}: {error:?}",
                        requirement.relation.raw()
                    ),
                )
            })?;
        let mut fields = BTreeSet::new();
        collect_requirement_fields(&requirement.expression, &mut fields);
        if fields.is_empty() {
            plan.authority.require_read_relation(requirement.relation)?;
        } else {
            for field in &fields {
                plan.authority.require_read_field(
                    requirement.relation,
                    crate::RelationColumnId::new(field.raw()),
                )?;
            }
        }
        let rows = target
            .state()
            .model
            .relations
            .get(&relation)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    format!(
                        "transaction requirement relation {} is absent",
                        requirement.relation.raw()
                    ),
                )
            })?;
        let identity: kernel_model::Value = requirement.identity_value.clone().into();
        let mut matches = rows
            .iter()
            .filter(|row| row.get(requirement.identity_column) == Some(&identity));
        let row = matches.next().ok_or_else(|| {
            Error::new(
                ErrorKind::TransactionConflict,
                format!(
                    "transaction requirement entity {} is absent from the proposed future world",
                    requirement.entity
                ),
            )
        })?;
        if matches.next().is_some() {
            return Err(Error::new(
                ErrorKind::Cardinality,
                "transaction requirement identity matched more than one future row",
            ));
        }
        let owner = kernel_types::EntityId::new(requirement.entity);
        observations.push((
            kernel_plan::RuntimeHistoryCoordinate::LifecycleEntity { entity: owner },
            None,
            None,
        ));
        let identity_field = context
            .schema
            .relation_column_id(relation, requirement.identity_column)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    "transaction requirement identity column has no semantic id",
                )
            })?;
        observations.push((
            kernel_plan::RuntimeHistoryCoordinate::ObjectField {
                relation,
                owner,
                field: identity_field,
            },
            row.get(requirement.identity_column).cloned(),
            None,
        ));
        observations.push((
            kernel_plan::RuntimeHistoryCoordinate::Field {
                field: identity_field,
                owner,
            },
            row.get(requirement.identity_column).cloned(),
            None,
        ));
        let single_field_rule = if fields.len() == 1 {
            let field = *fields.iter().next().expect("single field");
            bind_single_requirement_field_to_input(&requirement.expression, field)
                .map(semantic_rule_to_kernel)
        } else {
            None
        };
        let mut joint_values = BTreeMap::new();
        for field in &fields {
            let field = kernel_types::SemanticId::new(field.raw());
            let ordinal = context
                .schema
                .relation_column_ordinal(relation, field)
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidSchema,
                        "transaction requirement field has no relation-column ordinal",
                    )
                })?;
            let observed = row.get(ordinal).cloned();
            if fields.len() == 1 {
                observations.push((
                    kernel_plan::RuntimeHistoryCoordinate::ObjectField {
                        relation,
                        owner,
                        field,
                    },
                    observed.clone(),
                    single_field_rule.clone(),
                ));
                observations.push((
                    kernel_plan::RuntimeHistoryCoordinate::Field { field, owner },
                    observed,
                    single_field_rule.clone(),
                ));
            } else if let Some(observed) = observed {
                joint_values.insert(field, observed);
            }
        }
        if fields.len() > 1 {
            let group_id = u32::try_from(requirement_index).map_err(|_| {
                Error::new(ErrorKind::InvalidPlan, "too many transaction requirements")
            })?;
            joint_groups.push(kernel_plan::RuntimeJointCausalObservationGroup {
                group_id,
                relation,
                owner,
                observed_fields: joint_values,
                predicate: expression.clone(),
            });
        }
        let matches = kernel_validation::relation_row_rule_matches(
            &expression,
            relation,
            row,
            context,
            &plan.registry,
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::InvalidPlan,
                format!("transaction requirement evaluation failed: {error:?}"),
            )
        })?;
        if !matches {
            return Err(Error::new(
                ErrorKind::TransactionConflict,
                format!(
                    "transaction requirement failed for entity {}",
                    requirement.entity
                ),
            ));
        }
    }
    Ok(
        kernel_plan::RuntimeGuardObservationFootprint::with_exact_values_rules_and_groups(
            source_revision,
            observations,
            joint_groups,
        ),
    )
}

fn plan_object_field_writes(
    plan: &Plan,
) -> BTreeMap<kernel_types::SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>> {
    let mut field_writes =
        BTreeMap::<kernel_types::SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>>::new(
        );
    for ((relation, _), patch) in &plan.object_field_patches {
        let writes = field_writes.entry((*relation).into()).or_default();
        for (value, field) in patch.fields.values() {
            writes.push(kernel_durability::DurableObjectFieldWrite {
                owner: patch.owner,
                field: *field,
                value: value.clone().into(),
            });
        }
    }
    for writes in field_writes.values_mut() {
        writes.sort_by_key(|write| (write.owner, write.field));
    }
    field_writes
}

fn plan_relation_authorizations(
    plan: &Plan,
) -> BTreeMap<kernel_types::SemanticId, kernel_durability::DurableRelationAuthorization> {
    let mut authorizations = BTreeMap::new();
    for (relation, mutation) in &plan.mutations {
        let authorization = authorizations.entry((*relation).into()).or_default();
        for index in 0..mutation.inserted.len() {
            if let Some(coverage) = plan.history_authorization.get(relation)
                && index < coverage.inserted_prefix
            {
                merge_durable_relation_authorization(authorization, coverage.authorization);
                continue;
            }
            match plan.mutation_actions.get(&(
                *relation,
                crate::plan::MutationDirection::Insert,
                index,
            )) {
                Some(action) => add_durable_mutation_action(authorization, *action),
                None => authorization.relation_write = true,
            }
        }
        for index in 0..mutation.removed.len() {
            if let Some(coverage) = plan.history_authorization.get(relation)
                && index < coverage.removed_prefix
            {
                merge_durable_relation_authorization(authorization, coverage.authorization);
                continue;
            }
            match plan.mutation_actions.get(&(
                *relation,
                crate::plan::MutationDirection::Remove,
                index,
            )) {
                Some(action) => add_durable_mutation_action(authorization, *action),
                None => authorization.relation_write = true,
            }
        }
    }
    for contract in plan.owned_relations.values() {
        if contract.orphan_policy != crate::plan::OrphanPolicy::DeleteIfUnowned {
            continue;
        }
        let can_orphan = plan
            .mutation_actions
            .iter()
            .any(|((relation, _, _), action)| {
                *relation == contract.relation
                    && *action == crate::plan::MutationAction::RelationshipDetach
            });
        if can_orphan {
            authorizations
                .entry(contract.target_relation.into())
                .or_default()
                .object_delete = true;
        }
    }
    authorizations
}

fn merge_durable_relation_authorization(
    target: &mut kernel_durability::DurableRelationAuthorization,
    source: kernel_durability::DurableRelationAuthorization,
) {
    target.relation_write |= source.relation_write;
    target.object_create |= source.object_create;
    target.object_delete |= source.object_delete;
    target.relationship_attach |= source.relationship_attach;
    target.relationship_detach |= source.relationship_detach;
    target.relationship_move |= source.relationship_move;
}

fn add_durable_mutation_action(
    authorization: &mut kernel_durability::DurableRelationAuthorization,
    action: crate::plan::MutationAction,
) {
    match action {
        crate::plan::MutationAction::ObjectCreate => authorization.object_create = true,
        crate::plan::MutationAction::ObjectDelete => authorization.object_delete = true,
        crate::plan::MutationAction::RelationshipAttach => authorization.relationship_attach = true,
        crate::plan::MutationAction::RelationshipDetach => authorization.relationship_detach = true,
        crate::plan::MutationAction::RelationshipMove => authorization.relationship_move = true,
    }
}

struct RebasablePlanEffect {
    deltas: Vec<(kernel_types::SemanticId, kernel_query::RelationDelta)>,
    field_writes:
        BTreeMap<kernel_types::SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>>,
    authorizations:
        BTreeMap<kernel_types::SemanticId, kernel_durability::DurableRelationAuthorization>,
    model_delta: kernel_plan::DurableModelDelta,
    model_complement: kernel_plan::DurableModelDelta,
}

fn plan_rebasable_effect(plan: &Plan) -> Result<RebasablePlanEffect> {
    let target = build_plan_target(plan)?;
    let deltas = revision_relation_deltas(plan.source.revision(), &target, &plan.registry)?;
    let field_writes = plan_object_field_writes(plan);
    let authorizations = plan_relation_authorizations(plan);
    Ok(RebasablePlanEffect {
        field_writes,
        authorizations,
        model_delta: kernel_plan::DurableModelDelta::between(
            plan.source.revision().state(),
            target.state(),
        ),
        model_complement: kernel_plan::DurableModelDelta::between(
            target.state(),
            plan.source.revision().state(),
        ),
        deltas,
    })
}

pub(crate) fn durable_relational_intent_segment(
    source: &ReadContext,
    target: &ReadContext,
) -> Result<Vec<kernel_durability::DurableRelationMutation>> {
    let mut mutations = revision_relation_deltas(
        source.kernel_revision(),
        target.kernel_revision(),
        source.runtime.semantic_registry(),
    )?
    .into_iter()
    .map(
        |(relation, delta)| kernel_durability::DurableRelationMutation {
            relation,
            inserted: delta.inserted,
            removed: delta.removed,
            object_field_writes: Vec::new(),
            authorization: kernel_durability::DurableRelationAuthorization::default(),
        },
    )
    .collect::<Vec<_>>();
    mutations.sort_by_key(|mutation| mutation.relation);
    Ok(mutations)
}

fn certify_plan_rebase(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    effect: &RebasablePlanEffect,
) -> Result<kernel_plan::RuntimeTransitionRebaseOutcome> {
    let mutations = effect
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    runtime
        .certify_transition_rebase(
            plan.source.revision().id(),
            &mutations,
            Some(&effect.model_delta),
            Some(&effect.model_complement),
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::TransactionConflict,
                format!("transaction rebase certification unavailable: {error:?}"),
            )
        })
}

fn exact_rebased_plan(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: RebasablePlanEffect,
) -> Result<Plan> {
    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("transaction rebase snapshot failed: {error:?}"),
        )
    })?;
    let mut rebased = Plan::new(
        runtime,
        snapshot,
        plan.database_identity,
        plan.authority.clone(),
    );
    if !effect.field_writes.is_empty() {
        rebased.object_field_patches = plan.object_field_patches.clone();
        rebased.object_contracts = plan.object_contracts.clone();
        rebased.owned_relations = plan.owned_relations.clone();
        rebased.model_delta.clone_from(&plan.model_delta);
        for (relation, mutation) in &plan.mutations {
            let target = rebased.mutations.entry(*relation).or_default();
            target.inserted.extend(mutation.inserted.clone());
            target.removed.extend(mutation.removed.clone());
        }
        return Ok(rebased);
    }
    for (relation, delta) in effect.deltas {
        let relation = RelationId::new(relation.raw());
        for row in delta.removed {
            rebased.remove(relation, row.into_iter().map(Into::into).collect());
        }
        for row in delta.inserted {
            rebased.insert(relation, row.into_iter().map(Into::into).collect());
        }
    }
    if effect.model_delta != kernel_plan::DurableModelDelta::default() {
        rebased.model_delta = Some(effect.model_delta);
    }
    Ok(rebased)
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep certified relation residual publication protocol together."
)]
fn commit_certified_relation_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<Option<CommitOutcome>> {
    if effect.model_delta != kernel_plan::DurableModelDelta::default()
        || !plan.object_contracts.is_empty()
        || !effect.field_writes.is_empty()
    {
        return Ok(None);
    }

    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("transaction residual snapshot failed: {error:?}"),
        )
    })?;
    let revision = snapshot.revision();
    let context = revision.semantic_context();
    let registry = runtime.semantic_registry();
    let mut residuals = Vec::with_capacity(effect.deltas.len());
    for (relation, delta) in &effect.deltas {
        let rows = revision
            .state()
            .model
            .relations
            .materialize_owned(relation)
            .unwrap_or_default();
        let current = match &delta.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => kernel_query::RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => kernel_query::RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        };
        let residual = delta
            .residualize_against(&current, context, registry)
            .map_err(|error| query_error(&error))?;
        if !residual.is_empty() {
            residuals.push((*relation, residual));
        }
    }

    let realized_refs = residuals
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let client_refs = effect
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let target_revision = revision
        .id()
        .raw()
        .checked_add(1)
        .map(kernel_types::RevisionId::new)
        .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
    let request = kernel_plan::DerivedRelationTransitionRequest {
        source_revision: revision.id(),
        target_revision,
        mutations: &realized_refs,
    };
    let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
    let outcome = runtime.commit_derived_relation_data_residual_guarded_with_dependencies_and_relational_observations(
        transaction_id,
        &request,
        &client_refs,
        client_guard_digest,
        guard_observation,
        relational_observations,
    );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadySatisfied {
                revision: target_revision.into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadyCommitted {
                revision: target_revision.into(),
            }))
        }
        Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
            Err(Error::new(
                ErrorKind::TransactionConflict,
                "transaction id conflicts with a different committed intent",
            ))
        }
        Err(kernel_plan::DurableRuntimeCommitError::Runtime(
            kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
        )) => Err(Error::new(
            ErrorKind::StaleRevision,
            "certified residual source revision is no longer current",
        )),
        Err(error) => Err(Error::new(
            ErrorKind::InvariantViolation,
            format!("residual commit rejected: {error:?}"),
        )),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep certified field residual publication protocol together."
)]
fn commit_certified_field_reapply_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<Option<CommitOutcome>> {
    if effect.field_writes.is_empty() {
        return Ok(None);
    }

    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("transaction field reapply snapshot failed: {error:?}"),
        )
    })?;
    let mut rebased = Plan::new(
        runtime,
        snapshot,
        plan.database_identity,
        plan.authority.clone(),
    );
    rebased.object_field_patches = plan.object_field_patches.clone();
    rebased.object_contracts = plan.object_contracts.clone();
    rebased.owned_relations = plan.owned_relations.clone();
    rebased.model_delta.clone_from(&plan.model_delta);
    for (relation, mutation) in &plan.mutations {
        let target = rebased.mutations.entry(*relation).or_default();
        target.inserted.extend(mutation.inserted.clone());
        target.removed.extend(mutation.removed.clone());
    }

    let realized = plan_rebasable_effect(&rebased)?;
    let target = build_plan_target(&rebased)?;
    let realized_refs = realized
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let client_refs = effect
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let request = kernel_plan::MixedRevisionTransitionRequest {
        source_revision: rebased.source.revision().id(),
        target_revision: &target,
        mutations: &realized_refs,
        model_delta: &realized.model_delta,
        model_complement: &realized.model_complement,
        registry: runtime.semantic_registry(),
    };
    let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
    let outcome = runtime
        .commit_mixed_revision_residual_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            &request,
            &client_refs,
            &effect.model_delta,
            client_guard_digest,
            guard_observation,
            relational_observations,
        );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadySatisfied {
                revision: target_revision.into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadyCommitted {
                revision: target_revision.into(),
            }))
        }
        Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
            Err(Error::new(
                ErrorKind::TransactionConflict,
                "transaction id conflicts with a different committed intent",
            ))
        }
        Err(kernel_plan::DurableRuntimeCommitError::Runtime(
            kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
        )) => Err(Error::new(
            ErrorKind::StaleRevision,
            "certified field reapply source revision is no longer current",
        )),
        Err(error) => Err(Error::new(
            ErrorKind::InvariantViolation,
            format!("field reapply residual commit rejected: {error:?}"),
        )),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep certified mixed residual publication protocol together."
)]
fn commit_certified_mixed_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<Option<CommitOutcome>> {
    if effect.model_delta == kernel_plan::DurableModelDelta::default()
        || !effect.field_writes.is_empty()
    {
        return Ok(None);
    }

    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("transaction mixed residual snapshot failed: {error:?}"),
        )
    })?;
    let revision = snapshot.revision();
    let context = revision.semantic_context();
    let registry = runtime.semantic_registry();
    let mut state = revision.state().clone();
    let mut residuals = Vec::with_capacity(effect.deltas.len());

    for (relation, delta) in &effect.deltas {
        let rows = state
            .model
            .relations
            .materialize_owned(relation)
            .unwrap_or_default();
        let current = match &delta.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => kernel_query::RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => kernel_query::RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        };
        let residual = delta
            .residualize_against(&current, context, registry)
            .map_err(|error| query_error(&error))?;
        if residual.is_empty() {
            continue;
        }
        let next = residual
            .apply_to_value(current, context, registry)
            .map_err(|error| query_error(&error))?;
        state.model.relations.insert(*relation, next.into_rows());
        residuals.push((*relation, residual));
    }

    effect.model_delta.apply_to(&mut state);
    let target_revision = revision
        .id()
        .raw()
        .checked_add(1)
        .map(kernel_types::RevisionId::new)
        .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
    let target = kernel_revision::Revision::build(
        target_revision,
        revision.semantic_context(),
        registry,
        state,
    )
    .map_err(|error| {
        Error::new(
            ErrorKind::InvariantViolation,
            format!("certified mixed residual rejected by kernel: {error:?}"),
        )
    })?;
    let realized_model_delta =
        kernel_plan::DurableModelDelta::between(revision.state(), target.state());
    let realized_model_complement =
        kernel_plan::DurableModelDelta::between(target.state(), revision.state());
    let realized_refs = residuals
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let client_refs = effect
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: effect
                .field_writes
                .get(relation)
                .map_or(&[][..], Vec::as_slice),
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let request = kernel_plan::MixedRevisionTransitionRequest {
        source_revision: revision.id(),
        target_revision: &target,
        mutations: &realized_refs,
        model_delta: &realized_model_delta,
        model_complement: &realized_model_complement,
        registry,
    };
    let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
    let outcome = runtime
        .commit_mixed_revision_residual_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            &request,
            &client_refs,
            &effect.model_delta,
            client_guard_digest,
            guard_observation,
            relational_observations,
        );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadySatisfied {
                revision: target_revision.into(),
            }))
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
            Ok(Some(CommitOutcome::AlreadyCommitted {
                revision: target_revision.into(),
            }))
        }
        Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
            Err(Error::new(
                ErrorKind::TransactionConflict,
                "transaction id conflicts with a different committed intent",
            ))
        }
        Err(kernel_plan::DurableRuntimeCommitError::Runtime(
            kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
        )) => Err(Error::new(
            ErrorKind::StaleRevision,
            "certified mixed residual source revision is no longer current",
        )),
        Err(error) => Err(Error::new(
            ErrorKind::InvariantViolation,
            format!("mixed residual commit rejected: {error:?}"),
        )),
    }
}

impl Database {
    pub(crate) const fn runtime_identity(&self) -> u64 {
        self.identity
    }

    #[must_use]
    pub fn builder(path: impl Into<PathBuf>) -> DatabaseBuilder {
        DatabaseBuilder {
            path: path.into(),
            storage: Storage::Auto,
            schema: None,
            encryption: Encryption::None,
            external_freshness: None,
            publication_notifier: None,
        }
    }

    fn from_runtime(runtime: kernel_plan::DurableRuntime) -> Self {
        Self {
            runtime: Arc::new(runtime),
            identity: next_database_identity(),
        }
    }

    pub fn create(path: impl Into<PathBuf>, definition: Schema) -> Result<Self> {
        Self::builder(path).schema(definition).create()
    }

    /// Creates a process-local database with the same semantic/runtime engine as durable CFMD.
    /// No filesystem persistence is created until [`Self::persist`] or
    /// [`Self::persist_with_encryption`] succeeds.
    pub fn memory<S>() -> Result<Self>
    where
        S: crate::DatabaseDefinition,
    {
        Self::memory_from_schema(S::definition()?)
    }

    /// Dynamic-schema counterpart of [`Self::memory`].
    pub fn memory_from_schema(definition: Schema) -> Result<Self> {
        let (context, registry, relations) = compile_schema(definition)?;
        let root = build_empty_root(&context, &registry, &relations)?;
        let runtime =
            kernel_plan::DurableRuntime::create_volatile(root, &registry).map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::PersistenceTransition,
                    format!("memory database creation failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(Self::from_runtime(runtime))
    }

    #[must_use]
    pub fn is_memory(&self) -> bool {
        self.runtime.is_volatile_persistence()
    }

    /// Publishes this live memory database as a FORMAT V1 single-file database.
    /// The semantic revision and runtime identity are unchanged; existing contexts,
    /// snapshots and clones continue to observe the same runtime lineage.
    pub fn persist(&self, path: impl Into<PathBuf>) -> Result<()> {
        self.persist_with_encryption(path, &Encryption::None)
    }

    /// Encrypted form of [`Self::persist`]. Target protection is applied from the first
    /// staged durable write; no plaintext durable intermediate is created.
    pub fn persist_with_encryption(
        &self,
        path: impl Into<PathBuf>,
        encryption: &Encryption,
    ) -> Result<()> {
        if !self.runtime.is_volatile_persistence() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "persistence transition requires a live memory database",
            ));
        }
        let path = path.into();
        let kernel_encryption = encryption.resolve_kernel(&path, EncryptionKeyOperation::Create)?;
        self.runtime
            .promote_volatile_to_single_file(&path, &kernel_encryption)
            .map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::PersistenceTransition,
                    format!("persistence transition failed: {error:?}"),
                    &error,
                )
            })
    }

    /// Retires recoverable persistence for this live database while keeping
    /// the same runtime identity and semantic head in memory.
    ///
    /// If external freshness is active, the durable predecessor is fenced
    /// before its physical owner is discarded. The retained protection floor
    /// still constrains any later [`Self::persist_with_encryption`] target.
    pub fn make_volatile(&self) -> Result<()> {
        if self.runtime.is_volatile_persistence() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "persistence transition requires a live durable database",
            ));
        }
        self.runtime.demote_durable_to_volatile().map_err(|error| {
            Error::from_durability(
                crate::RecoveryOperation::PersistenceTransition,
                format!("volatile persistence transition failed: {error:?}"),
                &error,
            )
        })
    }

    /// Creates a strict quiescent single-file backup of the current authority cut.
    /// The target encryption must satisfy the source persistence protection floor.
    pub fn backup_to(
        &self,
        path: impl Into<PathBuf>,
        encryption: &Encryption,
    ) -> Result<BackupVerification> {
        let path = path.into();
        let kernel_encryption = encryption.resolve_kernel(&path, EncryptionKeyOperation::Create)?;
        let revision = self
            .runtime
            .backup_to_single_file(&path, &kernel_encryption)
            .map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::Backup,
                    format!("backup failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(BackupVerification {
            revision: revision.into(),
        })
    }

    /// Creates a second independently live database from this database's current semantic/history
    /// cut. The fork receives a fresh runtime identity and fresh retry/prepared/replication roots.
    /// Source and fork may diverge independently after this call.
    pub fn fork_to(&self, path: impl Into<PathBuf>, encryption: &Encryption) -> Result<Self> {
        let path = path.into();
        let kernel_encryption = encryption.resolve_kernel(&path, EncryptionKeyOperation::Create)?;
        let runtime = self
            .runtime
            .fork_to_single_file(&path, &kernel_encryption)
            .map_err(|error| {
                Error::from_runtime_recovery(
                    crate::RecoveryOperation::Fork,
                    format!("live fork failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(Self::from_runtime(runtime))
    }

    /// Creates a second independently live database whose target authority is externally
    /// freshness-anchored from its first recoverable generation. The source freshness root is
    /// never copied or rebound; `target_freshness` bootstraps a distinct target lineage.
    pub fn fork_to_with_external_freshness(
        &self,
        path: impl Into<PathBuf>,
        encryption: &Encryption,
        target_freshness: &ExternalFreshness,
    ) -> Result<Self> {
        let path = path.into();
        let kernel_encryption = encryption.resolve_kernel(&path, EncryptionKeyOperation::Create)?;
        let (target_config, authority) = target_freshness.resolve_kernel()?;
        let runtime = self
            .runtime
            .fork_to_single_file_with_external_freshness(
                &path,
                &kernel_encryption,
                target_config,
                authority,
            )
            .map_err(|error| {
                Error::from_runtime_recovery(
                    crate::RecoveryOperation::Fork,
                    format!("freshness-anchored live fork failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(Self::from_runtime(runtime))
    }

    /// Moves an externally anchored single-file authority to a new store identity.
    ///
    /// The current handle becomes the target on success. The operation requires exclusive
    /// ownership of the runtime so no source `Database`, Context, Snapshot or Plan can remain
    /// usable after the trust cut moves.
    pub fn transfer_authority_to(
        &mut self,
        target_path: impl Into<PathBuf>,
        target_encryption: &Encryption,
        target_freshness: &ExternalFreshness,
    ) -> Result<()> {
        if Arc::strong_count(&self.runtime) != 1 {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "authority transfer requires exclusive Database ownership; drop source clones, contexts, snapshots and plans before transfer",
            ));
        }
        let target_path = target_path.into();
        let target_kernel =
            target_encryption.resolve_kernel(&target_path, EncryptionKeyOperation::Create)?;
        let (target_config, authority) = target_freshness.resolve_kernel()?;
        let runtime = Arc::get_mut(&mut self.runtime).ok_or_else(|| {
            Error::new(
                ErrorKind::InvariantViolation,
                "exclusive authority-transfer ownership changed during admission",
            )
        })?;
        runtime
            .transfer_external_freshness_to_single_file(
                &target_path,
                &target_kernel,
                target_config,
                authority,
            )
            .map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::AuthorityTransfer,
                    format!("authority transfer failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(())
    }

    /// Verifies a backup strictly. Recoverable live-WAL truncation is not accepted
    /// as a valid backup artifact.
    pub fn verify_backup(
        path: impl Into<PathBuf>,
        encryption: &Encryption,
    ) -> Result<BackupVerification> {
        let path = path.into();
        let kernel_encryption = encryption.resolve_kernel(&path, EncryptionKeyOperation::Open)?;
        let revision =
            kernel_plan::DurableRuntime::verify_single_file_backup(&path, &kernel_encryption)
                .map_err(|error| {
                    Error::from_durability(
                        crate::RecoveryOperation::VerifyBackup,
                        format!("backup verification failed: {error:?}"),
                        &error,
                    )
                })?;
        Ok(BackupVerification {
            revision: revision.into(),
        })
    }

    /// Restores a verified backup into a new path and opens the restored database.
    /// Existing targets are never overwritten.
    pub fn restore_backup(
        backup_path: impl Into<PathBuf>,
        backup_encryption: &Encryption,
        target_path: impl Into<PathBuf>,
        target_encryption: Encryption,
    ) -> Result<Self> {
        let backup_path = backup_path.into();
        let target_path = target_path.into();
        let source_kernel =
            backup_encryption.resolve_kernel(&backup_path, EncryptionKeyOperation::Open)?;
        let target_kernel =
            target_encryption.resolve_kernel(&target_path, EncryptionKeyOperation::Create)?;
        let revision = kernel_plan::DurableRuntime::restore_single_file_backup(
            &backup_path,
            &source_kernel,
            &target_path,
            &target_kernel,
        )
        .map_err(|error| {
            Error::from_durability(
                crate::RecoveryOperation::RestoreBackup,
                format!("restore failed: {error:?}"),
                &error,
            )
        })?;
        let database = Self::builder(target_path)
            .encryption(target_encryption)
            .open()?;
        let snapshot = database.snapshot()?;
        if snapshot.revision() != RevisionId::new(revision.raw()) {
            return Err(Error::new(
                ErrorKind::Recovery,
                "restored database revision does not match verified backup",
            )
            .with_recovery_diagnostic(crate::RecoveryDiagnostic::new(
                crate::RecoveryOperation::RestoreBackup,
                crate::RecoveryAuthority::DurableHead,
                crate::RecoveryReason::RestoredRevisionMismatch,
                None,
                None,
            )));
        }
        Ok(database)
    }

    /// Control-plane restore entry for hosted administration surfaces.
    /// Authority is checked before encryption resolution or target staging begins.
    pub fn restore_backup_controlled(
        control: &crate::DatabaseControlSession,
        backup_path: impl Into<PathBuf>,
        backup_encryption: &Encryption,
        target_path: impl Into<PathBuf>,
        target_encryption: Encryption,
    ) -> Result<Self> {
        control.require(DatabaseControlPermission::Restore)?;
        Self::restore_backup(
            backup_path,
            backup_encryption,
            target_path,
            target_encryption,
        )
    }

    /// Creates a schema-neutral long-lived exact watch. The observation may
    /// cross definitionally equivalent schema revisions without fixing a
    /// public materialization type. Structural migrations remain fail-closed
    /// until an explicit descriptor-transport theorem represents the query.
    pub fn migratable_watch(&self, query: &Query) -> Result<crate::MigratableQueryWatch> {
        crate::MigratableQueryWatch::new(&self.snapshot()?, query)
    }

    /// Creates an authoritative database from a complete typed schema while returning a
    /// schema-neutral runtime authority.
    pub fn create_authoritative<S>(path: impl Into<PathBuf>) -> Result<Self>
    where
        S: crate::DatabaseDefinition,
    {
        Self::builder(path).create_authoritative::<S>()
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::builder(path).open()
    }

    /// Prepares one exact migration workflow artifact against the current immutable HEAD.
    ///
    /// The direct `Database` surface is the embedded trusted-process control plane. Hosted or
    /// restricted callers use `AdminDatabase`, whose authority never derives from `Schema.Access`.
    pub fn prepare_migration(
        &self,
        model: &crate::MigrationModel,
    ) -> Result<crate::PreparedMigration> {
        self.prepare_migration_with_control(model, &ControlAuthority::Unrestricted, None)
    }

    /// Describes migration shape/cost without inspecting relation values.
    pub fn plan_migration(&self, model: &crate::MigrationModel) -> Result<crate::MigrationPlan> {
        self.plan_migration_with_control(model, &ControlAuthority::Unrestricted)
    }

    pub(crate) fn plan_migration_with_control(
        &self,
        model: &crate::MigrationModel,
        authority: &ControlAuthority,
    ) -> Result<crate::MigrationPlan> {
        authority.require(DatabaseControlPermission::SchemaPublish)?;
        let source_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("migration planning snapshot failed: {error:?}"),
            )
        })?;
        let source = source_snapshot.revision();
        Ok(crate::MigrationPlan::from_model(
            RevisionId::new(source.id().raw()),
            source.semantic_revision().schema.raw(),
            model,
        ))
    }

    pub(crate) fn prepare_migration_with_control(
        &self,
        model: &crate::MigrationModel,
        authority: &ControlAuthority,
        approval: Option<&crate::MigrationSecurityApproval>,
    ) -> Result<crate::PreparedMigration> {
        authority.require(DatabaseControlPermission::SchemaPublish)?;
        let source_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("migration snapshot failed: {error:?}"),
            )
        })?;
        let source = source_snapshot.revision();
        let plan = crate::MigrationPlan::from_model(
            RevisionId::new(source.id().raw()),
            source.semantic_revision().schema.raw(),
            model,
        );
        let migration_program = compile_migration_program(source, model)?;
        let registry = self.runtime.semantic_registry();
        let transport = migration_program
            .verify(source.semantic_context(), registry)
            .map_err(|error| {
                let diagnostic = crate::MigrationDiagnostic::transport_failure(
                    crate::MigrationWorkflowStage::Validate,
                    crate::MigrationDiagnosticDomain::DataTransport,
                    &error,
                );
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("migration model is not valid for the current schema: {error:?}"),
                )
                .with_migration_diagnostic(diagnostic)
            })?;

        let security_impact =
            build_migration_security_impact(source, model, &migration_program, &transport)?;
        verify_migration_security_approval(self.identity, authority, &security_impact, approval)?;
        if security_impact.requires_data_inspection() {
            authority.require(DatabaseControlPermission::MigrationDataInspect)?;
        }

        let target_id = source
            .id()
            .raw()
            .checked_add(1)
            .map(kernel_types::RevisionId::new)
            .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
        let target = transport
            .transport_revision(source, target_id, registry)
            .map_err(|error| {
                let diagnostic = crate::MigrationDiagnostic::transport_failure(
                    crate::MigrationWorkflowStage::Preview,
                    crate::MigrationDiagnosticDomain::DataTransport,
                    &error,
                );
                Error::new(
                    ErrorKind::InvariantViolation,
                    format!("migration preview rejected target state: {error:?}"),
                )
                .with_migration_diagnostic(diagnostic)
            })?;
        let validation = crate::MigrationValidation::verified(plan, security_impact);
        Ok(crate::PreparedMigration::new(
            self.identity,
            model.id(),
            migration_program,
            target,
            validation,
            approval.cloned(),
        ))
    }

    /// Verifies static schema/transport meaning without materializing target data.
    ///
    /// This operation intentionally does not require `MigrationDataInspect`: its result is a
    /// static security-impact description, not a success/failure oracle over protected values.
    pub fn validate_migration(
        &self,
        model: &crate::MigrationModel,
    ) -> Result<crate::MigrationValidation> {
        self.validate_migration_with_control(model, &ControlAuthority::Unrestricted)
    }

    pub(crate) fn validate_migration_with_control(
        &self,
        model: &crate::MigrationModel,
        authority: &ControlAuthority,
    ) -> Result<crate::MigrationValidation> {
        authority.require(DatabaseControlPermission::SchemaPublish)?;
        let source_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("migration validation snapshot failed: {error:?}"),
            )
        })?;
        let source = source_snapshot.revision();
        let plan = crate::MigrationPlan::from_model(
            RevisionId::new(source.id().raw()),
            source.semantic_revision().schema.raw(),
            model,
        );
        let program = compile_migration_program(source, model)?;
        let transport = program
            .verify(source.semantic_context(), self.runtime.semantic_registry())
            .map_err(|error| {
                let diagnostic = crate::MigrationDiagnostic::transport_failure(
                    crate::MigrationWorkflowStage::Validate,
                    crate::MigrationDiagnosticDomain::DataTransport,
                    &error,
                );
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("migration model is not valid for the current schema: {error:?}"),
                )
                .with_migration_diagnostic(diagnostic)
            })?;
        let security_impact = build_migration_security_impact(source, model, &program, &transport)?;
        Ok(crate::MigrationValidation::verified(plan, security_impact))
    }

    pub fn preview_migration(
        &self,
        model: &crate::MigrationModel,
    ) -> Result<crate::MigrationPreview> {
        Ok(self.prepare_migration(model)?.preview())
    }

    pub fn execute_migration(
        &self,
        prepared: &crate::PreparedMigration,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        self.execute_migration_with_control(
            prepared,
            transaction,
            history,
            &ControlAuthority::Unrestricted,
        )
    }

    pub(crate) fn execute_migration_with_control(
        &self,
        prepared: &crate::PreparedMigration,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
        authority: &ControlAuthority,
    ) -> Result<CommitOutcome> {
        authority.require(DatabaseControlPermission::SchemaPublish)?;
        verify_migration_security_approval(
            self.identity,
            authority,
            prepared.validation().security_impact(),
            prepared.security_approval(),
        )?;
        if prepared
            .validation()
            .security_impact()
            .requires_data_inspection()
        {
            authority.require(DatabaseControlPermission::MigrationDataInspect)?;
        }
        if prepared.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "prepared migration belongs to a different database",
            ));
        }
        let current = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("migration execution snapshot failed: {error:?}"),
            )
        })?;
        if current.revision().id().raw() != prepared.plan().source_revision().raw() {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                format!(
                    "prepared migration was formed at revision {}, current revision is {}",
                    prepared.plan().source_revision().raw(),
                    current.revision().id().raw()
                ),
            ));
        }
        let registry = self.runtime.semantic_registry();
        let complement = match history {
            crate::MigrationHistoryPolicy::Forget => {
                kernel_durability::DurableMigrationComplement::from_capsule(
                    kernel_lens::ComplementCapsule {
                        source_schema: kernel_types::SchemaRevisionId::new(
                            prepared.plan().source_schema_revision(),
                        ),
                        target_schema: kernel_types::SchemaRevisionId::new(
                            prepared.plan().target_schema_revision(),
                        ),
                        lens_spec: kernel_lens::LensSpecId(kernel_types::SemanticId::new(
                            prepared.migration_id,
                        )),
                        semantic_pins: kernel_lens::SemanticManifestId(
                            kernel_types::SemanticId::new(
                                prepared.migration_id ^ 0x4346_4d44_4d49_4752_4154_494f_4e00_0001,
                            ),
                        ),
                        encoding_version: 1,
                        complement: kernel_model::Value::Unit,
                    },
                    kernel_lens::ComplementRetention::Forget,
                )
            }
        };
        let request = kernel_plan::FullRevisionTransitionRequest {
            target_revision: &prepared.target,
            registry,
        };
        authority.with_permission(DatabaseControlPermission::SchemaPublish, || {
            match self.runtime.migrate_schema(
                kernel_types::ClientTransactionId::new(transaction.raw()),
                &request,
                &prepared.program,
                &complement,
            ) {
                Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
                    Ok(CommitOutcome::Committed {
                        revision: receipt.durable.target_revision().into(),
                    })
                }
                Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied {
                    target_revision,
                }) => Ok(CommitOutcome::AlreadySatisfied {
                    revision: target_revision.into(),
                }),
                Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision,
                }) => Ok(CommitOutcome::AlreadyCommitted {
                    revision: target_revision.into(),
                }),
                Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
                    Err(Error::new(
                        ErrorKind::TransactionConflict,
                        "migration transaction id conflict",
                    ))
                }
                Err(error) => Err(Error::new(
                    ErrorKind::InvariantViolation,
                    format!("migration publication rejected: {error:?}"),
                )),
            }
        })
    }

    /// Observes one prepared migration from authoritative current HEAD + durable causal history.
    pub fn observe_migration(
        &self,
        prepared: &crate::PreparedMigration,
    ) -> Result<crate::MigrationObservation> {
        if prepared.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "prepared migration belongs to a different database",
            ));
        }

        let snapshot = self.snapshot()?;
        let current_revision = snapshot.revision();
        let current_schema_revision = snapshot.schema_revision();
        let published_revision = crate::RevisionId::new(prepared.target.id().raw());
        let semantic_change = self
            .runtime
            .semantic_schema_migration_at(
                kernel_types::RevisionId::new(prepared.plan().source_revision().raw()),
                kernel_types::RevisionId::new(published_revision.raw()),
                kernel_types::SemanticId::new(prepared.migration_id),
            )
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("migration observation failed: {error:?}"),
                )
            })?;

        if let Some(change) = semantic_change
            && change.source_schema.raw() == prepared.plan().source_schema_revision()
            && change.target_schema.raw() == prepared.plan().target_schema_revision()
        {
            return Ok(crate::MigrationObservation::cut_over(
                crate::MigrationCutover::new(
                    change.effect_id.0,
                    published_revision,
                    current_revision,
                    current_schema_revision,
                    prepared.plan().target_schema_revision(),
                ),
            ));
        }

        if current_revision == prepared.plan().source_revision()
            && current_schema_revision == prepared.plan().source_schema_revision()
        {
            Ok(crate::MigrationObservation::prepared(
                current_revision,
                current_schema_revision,
            ))
        } else {
            Ok(crate::MigrationObservation::stale(
                current_revision,
                current_schema_revision,
            ))
        }
    }

    pub fn migrate(
        &self,
        model: &crate::MigrationModel,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        let prepared = self.prepare_migration(model)?;
        self.execute_migration(&prepared, transaction, history)
    }

    pub fn create_with_publication_notifier(
        path: impl Into<PathBuf>,
        definition: Schema,
        notifier: Arc<dyn crate::PublicationNotifier>,
    ) -> Result<Self> {
        Self::builder(path)
            .schema(definition)
            .publication_notifier(notifier)
            .create()
    }

    pub fn open_with_publication_notifier(
        path: impl Into<PathBuf>,
        notifier: Arc<dyn crate::PublicationNotifier>,
    ) -> Result<Self> {
        Self::builder(path).publication_notifier(notifier).open()
    }

    pub fn reconfigure_protection(&self, next: &Encryption) -> Result<u64> {
        let Encryption::Aes256GcmSivProvider { provider } = next else {
            return Err(Error::new(
                ErrorKind::Recovery,
                "database master key rewrap requires a provider-backed encryption policy",
            ));
        };
        let path = self
            .runtime
            .persistence_path()
            .map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::ProtectionReconfigure,
                    format!("persistence location unavailable during protection reconfiguration: {error:?}"),
                    &error,
                )
            })?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    "memory database has no durable encryption authority to rewrap",
                )
            })?;
        let (key, metadata) =
            resolve_provider_key(provider.as_ref(), &path, EncryptionKeyOperation::Rewrap)?;
        let kernel =
            kernel_plan::StorageEncryption::aes256_gcm_siv_wrapped_with_minimum_database_key_epoch(
                key,
                metadata.key_id.0,
                metadata.key_epoch,
                metadata.minimum_database_key_epoch,
            );
        let database_key_epoch =
            self.runtime
                .rewrap_storage_encryption(&kernel)
                .map_err(|error| {
                    Error::from_durability(
                        crate::RecoveryOperation::ProtectionReconfigure,
                        format!("database protection reconfiguration failed: {error:?}"),
                        &error,
                    )
                })?;
        provider
            .acknowledge_database_key_epoch(
                &path,
                EncryptionKeyAcknowledgement {
                    provider_key_id: metadata.key_id,
                    provider_key_epoch: metadata.key_epoch,
                    database_key_epoch,
                },
            )
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!(
                        "external database-key acknowledgement failed after local protection handoff; retry is idempotent: {error}"
                    ),
                )
            })?;
        self.runtime
            .retire_previous_storage_encryption_key(database_key_epoch)
            .map_err(|error| {
                Error::from_durability(
                    crate::RecoveryOperation::ProtectionReconfigure,
                    format!("database protection predecessor retirement failed: {error:?}"),
                    &error,
                )
            })?;
        Ok(database_key_epoch)
    }

    pub(crate) fn __require_schema_definition(&self, definition: Schema) -> Result<()> {
        let (expected, _, _) = compile_schema(definition)?;
        let actual = self.snapshot()?;
        let actual = actual.kernel_revision().semantic_context();
        if actual.schema.definitionally_equivalent(&expected.schema)
            && actual
                .environment
                .definitionally_equivalent(&expected.environment)
        {
            return Ok(());
        }
        Err(Error::new(
            ErrorKind::InvalidSchema,
            "persisted database schema does not match the requested typed schema root",
        ))
    }

    pub(crate) fn current_schema_bridge(
        &self,
        source_schema_revision: u64,
    ) -> Result<Option<kernel_plan::CurrentSchemaBridge>> {
        self.runtime
            .current_schema_bridge(kernel_types::SchemaRevisionId::new(source_schema_revision))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("current schema bridge resolution failed: {error:?}"),
                )
            })
    }

    #[doc(hidden)]
    pub fn snapshot(&self) -> Result<ReadContext> {
        let snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(ErrorKind::Internal, format!("snapshot failed: {error:?}"))
        })?;
        let revision = snapshot.revision().clone();
        Ok(ReadContext {
            runtime: Arc::clone(&self.runtime),
            database_identity: self.identity,
            revision: Some(revision),
            factorized: None,
            live_snapshot: Some(snapshot),
            contract_context: None,
            schema_bridge: None,
            relational_causal_capture: None,
            authority: RuntimeAuthority::Unrestricted,
        })
    }

    /// Returns one exact immutable committed world at `revision`.
    ///
    /// Historical worlds share the ordinary read/query vocabulary but do not
    /// carry a write capability. Reconstruction uses the durable causal effect
    /// authority; unavailable/non-reversible history fails closed.
    #[doc(hidden)]
    pub fn at(&self, revision: RevisionId) -> Result<ReadContext> {
        if let Some(factorized) = self
            .runtime
            .factorized_read_snapshot_at(kernel_types::RevisionId::new(revision.raw()))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("historical factorized snapshot failed: {error:?}"),
                )
            })?
        {
            return Ok(ReadContext {
                runtime: Arc::clone(&self.runtime),
                database_identity: self.identity,
                revision: None,
                factorized: Some(factorized),
                live_snapshot: None,
                contract_context: None,
                schema_bridge: None,
                relational_causal_capture: None,
                authority: RuntimeAuthority::Unrestricted,
            });
        }
        let revision = self
            .runtime
            .revision_at(kernel_types::RevisionId::new(revision.raw()))
            .map_err(|error| match error {
                kernel_plan::RuntimeHistoricalSnapshotError::Unavailable { .. } => Error::new(
                    ErrorKind::NotFound,
                    format!(
                        "revision {} is outside exact historical snapshot coverage",
                        revision.raw()
                    ),
                ),
                other => Error::new(
                    ErrorKind::Recovery,
                    format!("historical snapshot reconstruction failed: {other:?}"),
                ),
            })?;
        Ok(ReadContext {
            runtime: Arc::clone(&self.runtime),
            database_identity: self.identity,
            revision: Some(revision),
            factorized: None,
            live_snapshot: None,
            contract_context: None,
            schema_bridge: None,
            relational_causal_capture: None,
            authority: RuntimeAuthority::Unrestricted,
        })
    }

    pub fn current_revision(&self) -> Result<RevisionId> {
        self.snapshot().map(|snapshot| snapshot.revision())
    }

    pub fn history(&self) -> Result<crate::History> {
        self.snapshot()?.history()
    }

    /// Returns the exact durable historical-epoch pins retained by this
    /// database. FORMAT V1 currently creates pins for schema-migration source
    /// epochs. Ordinary readers, watches and replication do not create a
    /// second retention authority.
    pub fn history_retention_pins(&self) -> Result<Vec<crate::HistoryRetentionPin>> {
        let pins = self.runtime.retained_historical_epochs().map_err(|error| {
            Error::new(
                ErrorKind::Recovery,
                format!("historical retention inspection failed: {error:?}"),
            )
        })?;
        Ok(pins
            .into_iter()
            .map(|pin| crate::HistoryRetentionPin {
                database_identity: self.identity,
                effect_id: pin.effect_id(),
                source_revision: RevisionId::new(pin.source_revision().raw()),
                source_schema_revision: pin.source_schema().raw(),
                reason: crate::HistoryRetentionReason::SchemaMigration,
            })
            .collect())
    }

    /// Irreversibly releases one durable historical-retention pin.
    ///
    /// Release is idempotent and does not delete the causal migration event,
    /// retry identity, replication authority or current semantic world. It only
    /// retires the exact source-epoch materialization authority represented by
    /// this pin. A released pin cannot be used to repin history.
    pub fn release_history_retention(&self, pin: &crate::HistoryRetentionPin) -> Result<()> {
        if pin.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "history retention pin belongs to a different open database instance",
            ));
        }
        self.runtime
            .release_historical_epoch_authority(pin.effect_id)
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("historical retention release failed: {error:?}"),
                )
            })?;
        Ok(())
    }

    /// Adds the exact compensating inverse of one durable history entry to an ordinary
    /// transaction. The database remains the visible authority; the history entry only describes
    /// which committed effect is being inverted.
    pub fn undo(&self, transaction: &mut IntentJournal, entry: &crate::HistoryEntry) -> Result<()> {
        let plan = entry.undo_plan()?;
        if plan.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "history entry belongs to a different open database instance",
            ));
        }
        transaction.add_plan(plan)
    }

    /// Adds the current history head's exact inverse to `transaction`.
    pub fn undo_latest(&self, transaction: &mut IntentJournal) -> Result<()> {
        let history = self.history()?;
        let entry = history.latest().ok_or_else(|| {
            Error::new(ErrorKind::NotFound, "history has no transition at its head")
        })?;
        self.undo(transaction, entry)
    }

    /// Returns the current object collection. Reads are snapshot-consistent; transaction-aware
    /// mutation methods keep the database collection visible at the call site.
    pub fn objects<E: crate::Object>(&self) -> Result<crate::ObjectSet<E>> {
        self.snapshot()?.objects::<E>()
    }

    /// Reports whether this transaction can be applied to the current head without changing its
    /// exact semantic effect. A newer global revision is not itself a conflict: the kernel proves
    /// transport across intervening exact effects by Γ-canonical write coordinates. Overlap or
    /// opaque history fails closed.
    pub fn intent_readiness(&self, transaction: &IntentJournal) -> Result<IntentReadiness> {
        self.require_intent_owner(transaction)?;
        let Some(base_revision) = transaction.origin_revision() else {
            return Ok(IntentReadiness::Unbound);
        };
        let plan = transaction.plan()?;
        let current_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("readiness head snapshot failed: {error:?}"),
            )
        })?;
        let current_revision: RevisionId = current_snapshot.revision().id().into();
        let crossed_schema_boundary = current_revision != base_revision
            && current_snapshot.revision().semantic_context().revision()
                != plan.source.revision().semantic_context().revision();
        drop(current_snapshot);
        if current_revision == base_revision {
            authorize_bound_plan(plan)?;
            return Ok(IntentReadiness::Ready {
                revision: current_revision,
            });
        }
        if transaction.is_snapshot_bound() {
            authorize_bound_plan(plan)?;
            return Ok(IntentReadiness::SnapshotChanged {
                snapshot_revision: base_revision,
                current_revision,
            });
        }
        let effect = plan_rebasable_effect(plan)?;
        if crossed_schema_boundary {
            return schema_aware_intent_readiness(
                &self.runtime,
                plan,
                &effect,
                transaction,
                base_revision,
            );
        }
        authorize_bound_plan(plan)?;
        match certify_plan_rebase(&self.runtime, plan, &effect)? {
            kernel_plan::RuntimeTransitionRebaseOutcome::Certified(certificate) => {
                Ok(IntentReadiness::Rebasable {
                    base_revision,
                    current_revision: certificate.current_revision.into(),
                    intervening_effect_count: certificate.intervening_effect_count,
                })
            }
            kernel_plan::RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                Ok(IntentReadiness::Conflict {
                    base_revision,
                    current_revision: conflict.current_revision.into(),
                    conflicting_effects: conflict.conflicting_effects,
                    coordination_effects: conflict.coordination_effects,
                    conflicting_coordinates: conflict.coordinates.len(),
                    opaque_effects: conflict.opaque_effects,
                })
            }
        }
    }

    /// Computes the exact proposed result of `transaction` against the current database world
    /// without publishing it. If the transaction's source revision is older, preview uses the same
    /// Γ-coordinate certificate as history rebase and transports only a proven-disjoint exact
    /// effect. Conflicting or opaque intervening effects fail closed.
    pub fn preview(&self, transaction: &IntentJournal) -> Result<CandidatePreview> {
        self.require_intent_owner(transaction)?;
        let plan = transaction.plan()?;
        let base_revision = plan.base_revision();
        let current_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("preview head snapshot failed: {error:?}"),
            )
        })?;
        let current_revision: RevisionId = current_snapshot.revision().id().into();
        let crossed_schema_boundary = current_revision != base_revision
            && current_snapshot.revision().semantic_context().revision()
                != plan.source.revision().semantic_context().revision();
        drop(current_snapshot);
        if current_revision == base_revision {
            authorize_bound_plan(plan)?;
            validate_transaction_requirements(plan, transaction.requirements())?;
            return Ok(plan.candidate()?.preview());
        }
        if transaction.is_snapshot_bound() {
            authorize_bound_plan(plan)?;
            return Err(Error::new(
                ErrorKind::StaleRevision,
                format!(
                    "snapshot-bound transaction was formed at revision {} but current revision is {}",
                    base_revision.raw(),
                    current_revision.raw(),
                ),
            ));
        }
        let effect = plan_rebasable_effect(plan)?;
        if crossed_schema_boundary {
            let formation_guard =
                validate_transaction_requirements(plan, transaction.requirements())?;
            let relational_observations = transaction.relational_causal_observations()?;
            if let Some(prepared) = prepare_schema_aware_plan(
                &self.runtime,
                plan,
                &effect,
                transaction.client_guard_digest(),
                formation_guard.as_ref(),
                &relational_observations,
            )? {
                let footprint = publication_authority_footprint_from_schema_aware(&prepared);
                plan.authority.require_publication_footprint(&footprint)?;
                return Ok(current_plan_from_prepared_schema_aware(
                    &self.runtime,
                    plan,
                    &prepared,
                )?
                .candidate()?
                .preview());
            }
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "schema-aware preview is unavailable for this effect class",
            ));
        }
        authorize_bound_plan(plan)?;
        match certify_plan_rebase(&self.runtime, plan, &effect)? {
            kernel_plan::RuntimeTransitionRebaseOutcome::Certified(_) => {
                let rebased = exact_rebased_plan(&self.runtime, plan, effect)?;
                validate_transaction_requirements(&rebased, transaction.requirements())?;
                Ok(rebased.candidate()?.preview())
            }
            kernel_plan::RuntimeTransitionRebaseOutcome::Conflict(conflict) => Err(Error::new(
                ErrorKind::TransactionConflict,
                format!(
                    "transaction cannot be previewed on revision {}: definite conflicts {:?}, coordination-required effects {:?}, {} overlapping semantic coordinates; opaque effects {:?}",
                    conflict.current_revision.raw(),
                    conflict.conflicting_effects,
                    conflict.coordination_effects,
                    conflict.coordinates.len(),
                    conflict.opaque_effects,
                ),
            )),
        }
    }

    pub fn plan(&self) -> Result<Plan> {
        self.runtime
            .snapshot()
            .map(|snapshot| {
                Plan::new(
                    &self.runtime,
                    snapshot,
                    self.identity,
                    RuntimeAuthority::Unrestricted,
                )
            })
            .map_err(|error| Error::new(ErrorKind::Internal, format!("snapshot failed: {error:?}")))
    }

    #[must_use]
    pub fn session(&self, session: Session) -> SessionDatabase {
        SessionDatabase::new(self.clone(), session)
    }

    /// Creates an epoch-bound client/data-plane session from external role assignments.
    /// The role IDs are re-resolved against authoritative `Schema.Access` after schema changes.
    pub fn session_for_roles(
        &self,
        principal: crate::PrincipalId,
        roles: impl IntoIterator<Item = crate::RoleId>,
    ) -> Result<SessionDatabase> {
        let roles = roles.into_iter().collect();
        let session = Session::for_schema_roles(principal, Arc::clone(&self.runtime), roles)?;
        Ok(SessionDatabase::new(self.clone(), session))
    }

    #[must_use]
    pub fn into_session(self, session: Session) -> SessionDatabase {
        SessionDatabase::new(self, session)
    }

    /// Binds an already-authenticated database-control session. Control claims are never
    /// resolved from `Schema.Access` or from a client `Session`.
    #[must_use]
    pub fn admin_session(&self, session: crate::DatabaseControlSession) -> crate::AdminDatabase {
        crate::AdminDatabase::new(self.clone(), session)
    }

    #[must_use]
    pub fn into_admin_session(
        self,
        session: crate::DatabaseControlSession,
    ) -> crate::AdminDatabase {
        crate::AdminDatabase::new(self, session)
    }

    pub(crate) fn snapshot_with_authority(
        &self,
        authority: RuntimeAuthority,
    ) -> Result<ReadContext> {
        let snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(ErrorKind::Internal, format!("snapshot failed: {error:?}"))
        })?;
        let revision = snapshot.revision().clone();
        Ok(ReadContext {
            runtime: Arc::clone(&self.runtime),
            database_identity: self.identity,
            revision: Some(revision),
            factorized: None,
            live_snapshot: Some(snapshot),
            contract_context: None,
            schema_bridge: None,
            relational_causal_capture: None,
            authority,
        })
    }

    pub(crate) fn at_with_authority(
        &self,
        revision: RevisionId,
        authority: RuntimeAuthority,
    ) -> Result<ReadContext> {
        authority.require(Permission::HistoricalRead)?;
        if let Some(factorized) = self
            .runtime
            .factorized_read_snapshot_at(kernel_types::RevisionId::new(revision.raw()))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("historical factorized snapshot failed: {error:?}"),
                )
            })?
        {
            return Ok(ReadContext {
                runtime: Arc::clone(&self.runtime),
                database_identity: self.identity,
                revision: None,
                factorized: Some(factorized),
                live_snapshot: None,
                contract_context: None,
                schema_bridge: None,
                relational_causal_capture: None,
                authority,
            });
        }
        let revision = self
            .runtime
            .revision_at(kernel_types::RevisionId::new(revision.raw()))
            .map_err(|error| match error {
                kernel_plan::RuntimeHistoricalSnapshotError::Unavailable { .. } => Error::new(
                    ErrorKind::NotFound,
                    format!(
                        "revision {} is outside exact historical snapshot coverage",
                        revision.raw()
                    ),
                ),
                other => Error::new(
                    ErrorKind::Recovery,
                    format!("historical snapshot reconstruction failed: {other:?}"),
                ),
            })?;
        Ok(ReadContext {
            runtime: Arc::clone(&self.runtime),
            database_identity: self.identity,
            revision: Some(revision),
            factorized: None,
            live_snapshot: None,
            contract_context: None,
            schema_bridge: None,
            relational_causal_capture: None,
            authority,
        })
    }

    pub(crate) fn plan_with_authority(&self, authority: RuntimeAuthority) -> Result<Plan> {
        self.runtime
            .snapshot()
            .map(|snapshot| Plan::new(&self.runtime, snapshot, self.identity, authority))
            .map_err(|error| Error::new(ErrorKind::Internal, format!("snapshot failed: {error:?}")))
    }

    /// Publishes one transaction into this database.
    ///
    /// The transaction remains reusable after publication so the same semantic intent may be
    /// retried idempotently after an uncertain caller-side failure.  If the live head advanced,
    /// publication first asks the kernel to certify transport of the already-formed exact effect;
    /// user code is never re-executed against the newer world.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep Context commit publication decision tree together."
    )]
    pub fn commit(&self, transaction: &IntentJournal) -> Result<CommitOutcome> {
        self.require_intent_owner(transaction)?;
        let plan = transaction.plan()?;
        let transaction_id = transaction.transaction_id()?;
        let client_guard_digest = transaction.client_guard_digest();
        let relational_observations = transaction.relational_causal_observations()?;
        // A stale adaptive transaction cannot publish its original Candidate. Avoid building that
        // obsolete future world solely to evaluate requirements: requirements are checked on the
        // exact rebased Candidate below. Same-schema stale intents still probe the original durable
        // identity first. Once a schema boundary was crossed, however, source-world authorization
        // must not run before authority-footprint transport; the schema-aware walker performs its
        // own durable retry probe under the current-world publication authority.
        let current_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("commit head snapshot failed: {error:?}"),
            )
        })?;
        let current_revision: RevisionId = current_snapshot.revision().id().into();
        let crossed_schema_boundary = current_revision != plan.base_revision()
            && current_snapshot.revision().semantic_context().revision()
                != plan.source.revision().semantic_context().revision();
        drop(current_snapshot);
        let direct_observation = if current_revision == plan.base_revision() {
            validate_transaction_requirements(plan, transaction.requirements())?
        } else {
            None
        };
        if !crossed_schema_boundary {
            match commit_bound_plan_with_relational_observations(
                &self.runtime,
                plan,
                transaction_id,
                client_guard_digest,
                direct_observation.as_ref(),
                &relational_observations,
            ) {
                Ok(outcome) => return Ok(outcome),
                Err(error) if error.kind() == ErrorKind::StaleRevision => {}
                Err(error) => return Err(error),
            }
        }
        if transaction.is_snapshot_bound() {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "snapshot-bound transaction cannot be transported to a newer database revision",
            ));
        }

        let effect = plan_rebasable_effect(plan)?;
        let formation_guard = if crossed_schema_boundary {
            validate_transaction_requirements(plan, transaction.requirements())?
        } else {
            None
        };
        if let Some(outcome) = commit_schema_aware_plan(
            &self.runtime,
            plan,
            &effect,
            transaction_id,
            client_guard_digest,
            formation_guard.as_ref(),
            &relational_observations,
        )? {
            return Ok(outcome);
        }
        match certify_plan_rebase(&self.runtime, plan, &effect)? {
            kernel_plan::RuntimeTransitionRebaseOutcome::Certified(_) => {
                let rebased_requirement_observation = if transaction.requirements().is_empty() {
                    None
                } else {
                    let rebased_for_requirements =
                        exact_rebased_plan(&self.runtime, plan, plan_rebasable_effect(plan)?)?;
                    validate_transaction_requirements(
                        &rebased_for_requirements,
                        transaction.requirements(),
                    )?
                };
                let footprint = publication_authority_footprint(plan);
                plan.authority.with_publication_authority(&footprint, || {
                    if let Some(outcome) = commit_certified_field_reapply_residual(
                        &self.runtime,
                        plan,
                        &effect,
                        transaction_id,
                        client_guard_digest,
                        rebased_requirement_observation.as_ref(),
                        &relational_observations,
                    )? {
                        return Ok(outcome);
                    }
                    if let Some(outcome) = commit_certified_relation_residual(
                        &self.runtime,
                        plan,
                        &effect,
                        transaction_id,
                        client_guard_digest,
                        rebased_requirement_observation.as_ref(),
                        &relational_observations,
                    )? {
                        return Ok(outcome);
                    }
                    if let Some(outcome) = commit_certified_mixed_residual(
                        &self.runtime,
                        &effect,
                        transaction_id,
                        client_guard_digest,
                        rebased_requirement_observation.as_ref(),
                        &relational_observations,
                    )? {
                        return Ok(outcome);
                    }
                    let rebased = exact_rebased_plan(&self.runtime, plan, effect)?;
                    commit_bound_plan_authorized(
                        &self.runtime,
                        &rebased,
                        transaction_id,
                        client_guard_digest,
                        rebased_requirement_observation.as_ref(),
                        &relational_observations,
                    )
                })
            }
            kernel_plan::RuntimeTransitionRebaseOutcome::Conflict(conflict) => Err(Error::new(
                ErrorKind::TransactionConflict,
                format!(
                    "transaction cannot be published on revision {}: definite conflicts {:?}, coordination-required effects {:?}, {} overlapping semantic coordinates; opaque effects {:?}",
                    conflict.current_revision.raw(),
                    conflict.conflicting_effects,
                    conflict.coordination_effects,
                    conflict.coordinates.len(),
                    conflict.opaque_effects,
                ),
            )),
        }
    }

    /// Publishes one transport-neutral exact relation effect formed at `formation_revision`.
    ///
    /// Unknown stale intents are never rebuilt as current-head Plans. The runtime first checks the
    /// existing durable idempotency authority, then validates the exact delta against retained
    /// historical Γ-support. A stale effect is published only after the same Γ-coordinate
    /// transition certificate used by local adaptive transactions.
    #[doc(hidden)]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep exact relation intent publication protocol together."
    )]
    pub(crate) fn commit_exact_relation_intent(
        &self,
        formation_revision: RevisionId,
        formation_schema_revision: u64,
        formation_environment_revision: u64,
        transaction: TransactionId,
        mutations: &[ExactRelationMutation],
        authority: &RuntimeAuthority,
    ) -> Result<CommitOutcome> {
        authority.require_write_entry()?;
        if mutations.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "exact relation intent requires at least one mutation",
            ));
        }

        let formation_semantic_revision = kernel_types::SemanticRevision::new(
            kernel_types::SchemaRevisionId::new(formation_schema_revision),
            kernel_types::SemanticEnvId::new(formation_environment_revision),
        );
        let mut normalized = BTreeMap::<RelationId, (Vec<Row>, Vec<Row>)>::new();
        for mutation in mutations {
            let entry = normalized.entry(mutation.relation).or_default();
            entry.0.extend(mutation.inserted.iter().cloned());
            entry.1.extend(mutation.removed.iter().cloned());
        }
        let normalized = normalized
            .into_iter()
            .map(|(relation, (inserted, removed))| ExactRelationMutation {
                relation,
                inserted,
                removed,
            })
            .collect::<Vec<_>>();

        let current_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("remote intent semantic snapshot failed: {error:?}"),
            )
        })?;
        let current_revision: RevisionId = current_snapshot.revision().id().into();
        let current_semantic_revision = current_snapshot.revision().semantic_context().revision();
        drop(current_snapshot);

        if current_semantic_revision != formation_semantic_revision {
            let formation = self
                .runtime
                .schema_aware_formation_context_witness(
                    kernel_types::RevisionId::new(formation_revision.raw()),
                    formation_semantic_revision,
                )
                .map_err(|error| match error {
                    kernel_plan::DurableRuntimeCommitError::Runtime(
                        kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
                    ) => Error::new(
                        ErrorKind::InvalidPlan,
                        "hosted formation semantic identity does not match the retained formation world",
                    ),
                    kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                        revision,
                    } => Error::new(
                        ErrorKind::FormationProofUnavailable,
                        format!(
                            "hosted formation proof material for revision {} is no longer available",
                            revision.raw()
                        ),
                    ),
                    other => Error::new(
                        ErrorKind::StaleRevision,
                        format!(
                            "explicit formation semantic world {} cannot be verified: {other:?}",
                            formation_revision.raw(),
                        ),
                    ),
                })?;
            let registry = self.runtime.semantic_registry();
            let mut deltas = Vec::with_capacity(normalized.len());
            for mutation in &normalized {
                let relation: kernel_types::SemanticId = mutation.relation.into();
                let result_type = kernel_query::RelExpr::Scan(relation)
                    .typecheck(formation.semantic_context(), registry)
                    .map_err(|error| query_error(&error))?;
                let delta = kernel_query::RelationDelta {
                    inserted: mutation
                        .inserted
                        .iter()
                        .cloned()
                        .map(|row| row.into_iter().map(Into::into).collect())
                        .collect(),
                    removed: mutation
                        .removed
                        .iter()
                        .cloned()
                        .map(|row| row.into_iter().map(Into::into).collect())
                        .collect(),
                    result_type,
                };
                deltas.push((relation, delta));
            }
            let client_refs = deltas
                .iter()
                .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                    relation: *relation,
                    delta,
                    object_field_writes: &[],
                    authorization: kernel_durability::DurableRelationAuthorization::default(),
                })
                .collect::<Vec<_>>();
            let relation_writes = deltas
                .iter()
                .map(|(relation, _)| *relation)
                .collect::<BTreeSet<_>>();
            let prepared = self
                .runtime
                .prepare_schema_aware_publication_with_witness(
                    &formation,
                    &client_refs,
                    &kernel_plan::DurableModelDelta::default(),
                    &relation_writes,
                    &BTreeSet::new(),
                    None,
                )
                .map_err(|error| match error {
                    kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }
                    | kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionConflict(_) => {
                        Error::new(
                            ErrorKind::TransactionConflict,
                            format!("hosted schema-aware preparation rejected: {error:?}"),
                        )
                    }
                    kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                        revision,
                    } => Error::new(
                        ErrorKind::FormationProofUnavailable,
                        format!(
                            "hosted formation proof material for revision {} is no longer available",
                            revision.raw()
                        ),
                    ),
                    other => Error::new(
                        ErrorKind::InvalidPlan,
                        format!("hosted schema-aware preparation rejected: {other:?}"),
                    ),
                })?;
            let footprint = publication_authority_footprint_from_schema_aware(&prepared);
            return authority.with_publication_authority(&footprint, || {
                match self.runtime.commit_prepared_schema_aware_publication(
                    kernel_types::ClientTransactionId::new(transaction.raw()),
                    &prepared,
                ) {
                    Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
                        Ok(CommitOutcome::Committed {
                            revision: receipt.durable.target_revision().into(),
                        })
                    }
                    Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied {
                        target_revision,
                    }) => Ok(CommitOutcome::AlreadySatisfied {
                        revision: target_revision.into(),
                    }),
                    Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted {
                        target_revision,
                    }) => Ok(CommitOutcome::AlreadyCommitted {
                        revision: target_revision.into(),
                    }),
                    Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict {
                        ..
                    }) => Err(Error::new(
                        ErrorKind::TransactionConflict,
                        "transaction id conflicts with a different committed intent",
                    )),
                    Err(
                        kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
                            ..
                        },
                    ) => Err(Error::new(
                        ErrorKind::StaleRevision,
                        "authorized hosted schema-aware publication head is no longer current",
                    )),
                    Err(error) => Err(Error::new(
                        ErrorKind::InvariantViolation,
                        format!("hosted schema-aware publication rejected: {error:?}"),
                    )),
                }
            });
        }

        for mutation in &normalized {
            authority.require_write_relation(mutation.relation)?;
        }
        let durable_mutations = normalized
            .iter()
            .map(|mutation| kernel_durability::DurableRelationMutation {
                relation: mutation.relation.into(),
                inserted: mutation
                    .inserted
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                removed: mutation
                    .removed
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                object_field_writes: Vec::new(),
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        let requested_target = current_revision
            .raw()
            .checked_add(1)
            .map(kernel_types::RevisionId::new)
            .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
        let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
        match self.runtime.check_relation_data_retry(
            transaction_id,
            durable_mutations,
            None,
            requested_target,
        ) {
            Ok(Some(target_revision)) => {
                return Ok(CommitOutcome::AlreadyCommitted {
                    revision: target_revision.into(),
                });
            }
            Ok(None) => {}
            Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
                return Err(Error::new(
                    ErrorKind::TransactionConflict,
                    "transaction id conflicts with a different committed intent",
                ));
            }
            Err(error) => {
                return Err(Error::new(
                    ErrorKind::Recovery,
                    format!("durable transaction retry probe failed: {error:?}"),
                ));
            }
        }

        let registry = self.runtime.semantic_registry();
        let current_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("remote intent semantic snapshot failed: {error:?}"),
            )
        })?;
        if current_snapshot.revision().id().raw() != current_revision.raw() {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "database head advanced before remote intent certification",
            ));
        }
        let semantic_context = current_snapshot.revision().semantic_context();
        if semantic_context.revision() != formation_semantic_revision {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "formation semantic identity no longer matches current semantic world",
            ));
        }
        let mut deltas = Vec::with_capacity(normalized.len());
        for mutation in &normalized {
            let relation: kernel_types::SemanticId = mutation.relation.into();
            let result_type = kernel_query::RelExpr::Scan(relation)
                .typecheck(semantic_context, registry)
                .map_err(|error| query_error(&error))?;
            let delta = kernel_query::RelationDelta {
                inserted: mutation
                    .inserted
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                removed: mutation
                    .removed
                    .iter()
                    .cloned()
                    .map(|row| row.into_iter().map(Into::into).collect())
                    .collect(),
                result_type,
            };
            self.runtime
                .validate_relation_delta_at(
                    kernel_types::RevisionId::new(formation_revision.raw()),
                    relation,
                    &delta,
                )
                .map_err(|error| match error {
                    kernel_plan::RuntimeHistoricalSnapshotError::Unavailable { .. } => Error::new(
                        ErrorKind::StaleRevision,
                        format!(
                            "formation revision {} is outside exact Γ-support coverage",
                            formation_revision.raw()
                        ),
                    ),
                    kernel_plan::RuntimeHistoricalSnapshotError::Runtime(_) => Error::new(
                        ErrorKind::InvalidPlan,
                        format!(
                            "exact relation effect is invalid in its formation world: {error:?}"
                        ),
                    ),
                    other => Error::new(
                        ErrorKind::Recovery,
                        format!("formation Γ-support validation failed: {other:?}"),
                    ),
                })?;
            deltas.push((relation, delta));
        }
        let client_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: &[],
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();

        if current_revision != formation_revision {
            match self
                .runtime
                .certify_transition_rebase(
                    kernel_types::RevisionId::new(formation_revision.raw()),
                    &client_refs,
                    None,
                    None,
                )
                .map_err(|error| {
                    Error::new(
                        ErrorKind::TransactionConflict,
                        format!("remote intent rebase certification unavailable: {error:?}"),
                    )
                })? {
                kernel_plan::RuntimeTransitionRebaseOutcome::Certified(_) => {}
                kernel_plan::RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                    return Err(Error::new(
                        ErrorKind::TransactionConflict,
                        format!(
                            "remote intent conflicts at revision {} across {} semantic coordinates",
                            conflict.current_revision.raw(),
                            conflict.coordinates.len(),
                        ),
                    ));
                }
            }
        }

        let snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("remote intent publication snapshot failed: {error:?}"),
            )
        })?;
        if snapshot.revision().id().raw() != current_revision.raw() {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "database head advanced while remote intent was being certified",
            ));
        }
        let current = snapshot.revision();
        let mut realized = Vec::with_capacity(deltas.len());
        for (relation, delta) in &deltas {
            let residual = if current_revision == formation_revision {
                delta.clone()
            } else {
                let rows = current.state().model.relations.get(relation);
                let witness = snapshot.relation_base_witness(*relation).ok_or_else(|| {
                    Error::new(
                        ErrorKind::Internal,
                        format!("current Γ witness missing for relation {}", relation.raw()),
                    )
                })?;
                witness
                    .residualize_delta_against_support(delta, |position| {
                        rows.and_then(|rows| rows.get(position)).cloned()
                    })
                    .map_err(|error| query_error(&error))?
            };
            if !residual.is_empty() {
                realized.push((*relation, residual));
            }
        }
        let realized_refs = realized
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: &[],
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        let target_revision = current
            .id()
            .raw()
            .checked_add(1)
            .map(kernel_types::RevisionId::new)
            .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
        let request = kernel_plan::DerivedRelationTransitionRequest {
            source_revision: current.id(),
            target_revision,
            mutations: &realized_refs,
        };
        let outcome = if current_revision == formation_revision {
            self.runtime
                .commit_derived_relation_data_guarded(transaction_id, &request, None)
        } else {
            self.runtime.commit_derived_relation_data_residual_guarded(
                transaction_id,
                &request,
                &client_refs,
                None,
            )
        };
        match outcome {
            Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
                Ok(CommitOutcome::Committed {
                    revision: receipt.durable.target_revision().into(),
                })
            }
            Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
                Ok(CommitOutcome::AlreadySatisfied {
                    revision: target_revision.into(),
                })
            }
            Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
                Ok(CommitOutcome::AlreadyCommitted {
                    revision: target_revision.into(),
                })
            }
            Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
                Err(Error::new(
                    ErrorKind::TransactionConflict,
                    "transaction id conflicts with a different committed intent",
                ))
            }
            Err(kernel_plan::DurableRuntimeCommitError::Runtime(
                kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
            )) => Err(Error::new(
                ErrorKind::StaleRevision,
                "database head advanced before certified remote intent publication",
            )),
            Err(error) => Err(Error::new(
                ErrorKind::InvariantViolation,
                format!("exact remote relation intent rejected: {error:?}"),
            )),
        }
    }

    /// Advanced low-level publication path for tooling and bindings that intentionally construct
    /// ordinary [`Plan`] values themselves.
    pub fn commit_plan(&self, plan: &Plan, transaction: TransactionId) -> Result<CommitOutcome> {
        if plan.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "plan belongs to a different open database instance",
            ));
        }
        commit_bound_plan(&self.runtime, plan, transaction, None, None)
    }

    fn require_intent_owner(&self, transaction: &IntentJournal) -> Result<()> {
        let Some(identity) = transaction.database_identity() else {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "transaction is still unbound; add at least one database mutation first",
            ));
        };
        if identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "transaction belongs to a different open database instance",
            ));
        }
        Ok(())
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep exact publication authority footprint derivation together."
)]
fn publication_authority_footprint(plan: &Plan) -> crate::security::PublicationAuthorityFootprint {
    let mut footprint = crate::security::PublicationAuthorityFootprint::default();
    for (relation, coverage) in &plan.history_authorization {
        let authorization = coverage.authorization;
        if authorization.relation_write {
            footprint.relation_writes.insert(*relation);
        }
        for (required, action) in [
            (
                authorization.object_create,
                crate::plan::MutationAction::ObjectCreate,
            ),
            (
                authorization.object_delete,
                crate::plan::MutationAction::ObjectDelete,
            ),
            (
                authorization.relationship_attach,
                crate::plan::MutationAction::RelationshipAttach,
            ),
            (
                authorization.relationship_detach,
                crate::plan::MutationAction::RelationshipDetach,
            ),
            (
                authorization.relationship_move,
                crate::plan::MutationAction::RelationshipMove,
            ),
        ] {
            if required {
                footprint.actions.insert((*relation, action));
            }
        }
        footprint.field_writes.extend(
            coverage
                .fields
                .iter()
                .copied()
                .map(|field| (*relation, field)),
        );
    }
    for (relation, mutation) in &plan.mutations {
        for index in 0..mutation.inserted.len() {
            if plan
                .history_authorization
                .get(relation)
                .is_some_and(|coverage| index < coverage.inserted_prefix)
            {
                continue;
            }
            match plan.mutation_actions.get(&(
                *relation,
                crate::plan::MutationDirection::Insert,
                index,
            )) {
                Some(action) => {
                    footprint.actions.insert((*relation, *action));
                }
                None => {
                    footprint.relation_writes.insert(*relation);
                }
            }
        }
        for index in 0..mutation.removed.len() {
            if plan
                .history_authorization
                .get(relation)
                .is_some_and(|coverage| index < coverage.removed_prefix)
            {
                continue;
            }
            match plan.mutation_actions.get(&(
                *relation,
                crate::plan::MutationDirection::Remove,
                index,
            )) {
                Some(action) => {
                    footprint.actions.insert((*relation, *action));
                }
                None => {
                    footprint.relation_writes.insert(*relation);
                }
            }
        }
    }
    for ((relation, _), patch) in &plan.object_field_patches {
        footprint.field_writes.extend(
            patch
                .fields
                .values()
                .map(|(_, field)| (*relation, crate::RelationColumnId::new(field.raw()))),
        );
    }
    for contract in plan.owned_relations.values() {
        if contract.orphan_policy != crate::plan::OrphanPolicy::DeleteIfUnowned {
            continue;
        }
        let can_orphan = plan
            .mutation_actions
            .iter()
            .any(|((relation, _, _), action)| {
                *relation == contract.relation
                    && *action == crate::plan::MutationAction::RelationshipDetach
            });
        if can_orphan {
            footprint.actions.insert((
                contract.target_relation,
                crate::plan::MutationAction::ObjectDelete,
            ));
        }
    }
    footprint
}

fn publication_authority_footprint_from_schema_aware(
    transported: &kernel_plan::PreparedSchemaAwarePublication,
) -> crate::security::PublicationAuthorityFootprint {
    let mut footprint = crate::security::PublicationAuthorityFootprint::default();
    footprint.relation_writes.extend(
        transported
            .relation_writes
            .iter()
            .copied()
            .map(|relation| RelationId::new(relation.raw())),
    );
    footprint
        .field_writes
        .extend(transported.field_writes.iter().map(|(relation, field)| {
            (
                RelationId::new(relation.raw()),
                crate::RelationColumnId::new(field.raw()),
            )
        }));
    let model = transported.current_model_authority_footprint();
    footprint.carrier_presence.extend(
        model
            .carrier_presence
            .iter()
            .map(|id| crate::ModelSemanticId::new(id.raw())),
    );
    footprint
        .carrier_members
        .extend(model.carrier_members.iter().map(|(carrier, member)| {
            (
                crate::ModelSemanticId::new(carrier.raw()),
                crate::ModelEntityId::new(member.raw()),
            )
        }));
    footprint.lifecycle_entities.extend(
        model
            .lifecycle_entities
            .iter()
            .map(|id| crate::ModelEntityId::new(id.raw())),
    );
    footprint.lifecycle_roots.extend(
        model
            .lifecycle_roots
            .iter()
            .map(|id| crate::ModelEntityId::new(id.raw())),
    );
    footprint.keeps_alive_presence.extend(
        model
            .keeps_alive_presence
            .iter()
            .map(|id| crate::ModelEntityId::new(id.raw())),
    );
    footprint
        .keeps_alive_edges
        .extend(model.keeps_alive_edges.iter().map(|(parent, child)| {
            (
                crate::ModelEntityId::new(parent.raw()),
                crate::ModelEntityId::new(child.raw()),
            )
        }));
    for (relation, authorization) in &transported.relation_authorizations {
        let relation = RelationId::new(relation.raw());
        for (required, action) in [
            (
                authorization.object_create,
                crate::plan::MutationAction::ObjectCreate,
            ),
            (
                authorization.object_delete,
                crate::plan::MutationAction::ObjectDelete,
            ),
            (
                authorization.relationship_attach,
                crate::plan::MutationAction::RelationshipAttach,
            ),
            (
                authorization.relationship_detach,
                crate::plan::MutationAction::RelationshipDetach,
            ),
            (
                authorization.relationship_move,
                crate::plan::MutationAction::RelationshipMove,
            ),
        ] {
            if required {
                footprint.actions.insert((relation, action));
            }
        }
    }
    footprint
}

fn schema_aware_intent_readiness(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: &IntentJournal,
    base_revision: RevisionId,
) -> Result<IntentReadiness> {
    let formation_guard = validate_transaction_requirements(plan, transaction.requirements())?;
    let relational_observations = transaction.relational_causal_observations()?;
    match prepare_schema_aware_plan_kernel(
        runtime,
        plan,
        effect,
        transaction.client_guard_digest(),
        formation_guard.as_ref(),
        &relational_observations,
    ) {
        Ok(Some(prepared)) => {
            let footprint = publication_authority_footprint_from_schema_aware(&prepared);
            plan.authority.require_publication_footprint(&footprint)?;
            Ok(IntentReadiness::Rebasable {
                base_revision,
                current_revision: prepared.authorized_head_revision.into(),
                intervening_effect_count: prepared.intervening_effect_count,
            })
        }
        Ok(None) => Err(Error::new(
            ErrorKind::StaleRevision,
            "schema-aware readiness is unavailable for this effect class",
        )),
        Err(kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict)) => {
            Ok(IntentReadiness::Conflict {
                base_revision,
                current_revision: conflict.current_revision.into(),
                conflicting_effects: conflict.conflicting_effects,
                coordination_effects: conflict.coordination_effects,
                conflicting_coordinates: conflict.coordinates.len(),
                opaque_effects: conflict.opaque_effects,
            })
        }
        Err(kernel_plan::DurableRuntimeCommitError::GuardDependencyConflict(_)) => Err(Error::new(
            ErrorKind::FormationProofInvalidated,
            "formation-world observation was invalidated before schema cutover",
        )),
        Err(kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable {
            revision,
        }) => Err(Error::new(
            ErrorKind::FormationProofUnavailable,
            format!(
                "formation proof material for revision {} is no longer available",
                revision.raw()
            ),
        )),
        Err(other) => Err(Error::new(
            ErrorKind::StaleRevision,
            format!("schema-aware readiness unavailable: {other:?}"),
        )),
    }
}

fn prepare_schema_aware_plan(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<Option<kernel_plan::PreparedSchemaAwarePublication>> {
    prepare_schema_aware_plan_kernel(
        runtime,
        plan,
        effect,
        client_guard_digest,
        guard_observation,
        relational_observations,
    )
    .map_err(|error| match error {
        kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict) => {
            Error::new(
                ErrorKind::TransactionConflict,
                format!(
                    "schema-aware intent conflicts at revision {} across {} semantic coordinates",
                    conflict.current_revision.raw(),
                    conflict.coordinates.len(),
                ),
            )
        }
        kernel_plan::DurableRuntimeCommitError::GuardDependencyConflict(_) => Error::new(
            ErrorKind::FormationProofInvalidated,
            "formation-world observation was invalidated before schema cutover",
        ),
        kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable { revision } => {
            Error::new(
                ErrorKind::FormationProofUnavailable,
                format!(
                    "formation proof material for revision {} is no longer available",
                    revision.raw()
                ),
            )
        }
        other => Error::new(
            ErrorKind::StaleRevision,
            format!("schema-aware publication preparation unavailable: {other:?}"),
        ),
    })
}

#[allow(
    clippy::result_large_err,
    reason = "Preserve the exact kernel diagnostic without heap allocation on readiness paths."
)]
fn prepare_schema_aware_plan_kernel(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> std::result::Result<
    Option<kernel_plan::PreparedSchemaAwarePublication>,
    kernel_plan::DurableRuntimeCommitError,
> {
    let source_semantic_revision = plan.source.revision().semantic_context().revision();
    let current_snapshot = runtime.snapshot()?;
    if current_snapshot.revision().semantic_context().revision() == source_semantic_revision {
        return Ok(None);
    }
    drop(current_snapshot);

    let source_footprint = publication_authority_footprint(plan);
    let relation_writes = source_footprint
        .relation_writes
        .iter()
        .map(|relation| kernel_types::SemanticId::new(relation.raw()))
        .collect::<BTreeSet<_>>();
    let field_writes = source_footprint
        .field_writes
        .iter()
        .map(|(relation, field)| {
            (
                kernel_types::SemanticId::new(relation.raw()),
                kernel_types::SemanticId::new(field.raw()),
            )
        })
        .collect::<BTreeSet<_>>();
    let mutations = effect
        .deltas
        .iter()
        .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
            relation: *relation,
            delta,
            object_field_writes: &[],
            authorization: effect
                .authorizations
                .get(relation)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    runtime
        .prepare_schema_aware_publication_with_formation_proof(
            plan.source.revision().id(),
            source_semantic_revision,
            &mutations,
            &effect.model_delta,
            &relation_writes,
            &field_writes,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
        .map(Some)
}

fn current_plan_from_prepared_schema_aware(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    source_plan: &Plan,
    prepared: &kernel_plan::PreparedSchemaAwarePublication,
) -> Result<Plan> {
    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("prepared schema-aware snapshot failed: {error:?}"),
        )
    })?;
    if snapshot.revision().id() != prepared.authorized_head_revision {
        return Err(Error::new(
            ErrorKind::StaleRevision,
            "prepared schema-aware publication head is no longer current",
        ));
    }
    let mut current = Plan::new(
        runtime,
        snapshot,
        source_plan.database_identity,
        source_plan.authority.clone(),
    );
    for (relation, delta, _) in prepared.current_relation_deltas() {
        let relation = RelationId::new(relation.raw());
        for row in &delta.removed {
            current.remove(relation, row.iter().cloned().map(Into::into).collect());
        }
        for row in &delta.inserted {
            current.insert(relation, row.iter().cloned().map(Into::into).collect());
        }
    }
    if prepared.current_model_delta() != &kernel_plan::DurableModelDelta::default() {
        current.model_delta = Some(prepared.current_model_delta().clone());
    }
    Ok(current)
}

fn commit_schema_aware_plan(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<Option<CommitOutcome>> {
    let Some(prepared) = prepare_schema_aware_plan(
        runtime,
        plan,
        effect,
        client_guard_digest,
        guard_observation,
        relational_observations,
    )?
    else {
        return Ok(None);
    };
    let current_footprint = publication_authority_footprint_from_schema_aware(&prepared);
    plan.authority.with_publication_authority(&current_footprint, || {
        match runtime.commit_prepared_schema_aware_publication(
            kernel_types::ClientTransactionId::new(transaction.raw()),
            &prepared,
        ) {
            Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
                Ok(CommitOutcome::Committed {
                    revision: receipt.durable.target_revision().into(),
                })
            }
            Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
                Ok(CommitOutcome::AlreadySatisfied {
                    revision: target_revision.into(),
                })
            }
            Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
                Ok(CommitOutcome::AlreadyCommitted {
                    revision: target_revision.into(),
                })
            }
            Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => Err(
                Error::new(
                    ErrorKind::TransactionConflict,
                    "transaction id conflicts with a different committed intent",
                ),
            ),
            Err(kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict)) => {
                Err(Error::new(
                    ErrorKind::TransactionConflict,
                    format!(
                        "schema-aware intent conflicts at revision {} across {} semantic coordinates",
                        conflict.current_revision.raw(),
                        conflict.coordinates.len(),
                    ),
                ))
            }
            Err(kernel_plan::DurableRuntimeCommitError::SchemaAwareTransitionUnavailable { .. }) => {
                Err(Error::new(
                    ErrorKind::StaleRevision,
                    "authorized schema-aware publication head is no longer current",
                ))
            }
            Err(error) => Err(Error::new(
                ErrorKind::InvariantViolation,
                format!("schema-aware publication rejected: {error:?}"),
            )),
        }
    })
    .map(Some)
}

fn authorize_bound_plan(plan: &Plan) -> Result<()> {
    plan.authority
        .require_publication_footprint(&publication_authority_footprint(plan))
}

pub(crate) fn commit_bound_plan(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
) -> Result<CommitOutcome> {
    commit_bound_plan_with_relational_observations(
        runtime,
        plan,
        transaction,
        client_guard_digest,
        guard_observation,
        &[],
    )
}

fn commit_bound_plan_with_relational_observations(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<CommitOutcome> {
    let footprint = publication_authority_footprint(plan);
    plan.authority.with_publication_authority(&footprint, || {
        commit_bound_plan_authorized(
            runtime,
            plan,
            transaction,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep bound-plan authorization and publication protocol together."
)]
fn commit_bound_plan_authorized(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    guard_observation: Option<&kernel_plan::RuntimeGuardObservationFootprint>,
    relational_observations: &[kernel_plan::RuntimeRelationalCausalObservation],
) -> Result<CommitOutcome> {
    if plan.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidPlan,
            "cannot commit an empty plan",
        ));
    }
    let source_revision = plan.source.revision().id();
    let target_revision = plan_target_revision_id(plan)?;
    let field_writes = plan_object_field_writes(plan);
    let authorizations = plan_relation_authorizations(plan);
    let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
    let outcome = if let Some(model_delta) = &plan.model_delta {
        let deltas = plan_relation_deltas(plan, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: field_writes.get(relation).map_or(&[][..], Vec::as_slice),
                authorization: authorizations.get(relation).copied().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let target =
            build_explicit_model_target(plan, target_revision, runtime.semantic_registry())?;
        let model_complement =
            kernel_plan::DurableModelDelta::between(target.state(), plan.source.revision().state());
        let request = kernel_plan::MixedRevisionTransitionRequest {
            source_revision,
            target_revision: &target,
            mutations: &mutation_refs,
            model_delta,
            model_complement: &model_complement,
            registry: runtime.semantic_registry(),
        };
        runtime.commit_mixed_revision_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            &request,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    } else if plan.object_contracts.is_empty() {
        let deltas = plan_relation_deltas(plan, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: field_writes.get(relation).map_or(&[][..], Vec::as_slice),
                authorization: authorizations.get(relation).copied().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let request = kernel_plan::DerivedRelationTransitionRequest {
            source_revision,
            target_revision,
            mutations: &mutation_refs,
        };
        runtime.commit_derived_relation_data_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            &request,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    } else {
        let target = build_object_target(plan, target_revision, runtime.semantic_registry())?;
        let deltas =
            revision_relation_deltas(plan.source.revision(), &target, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: field_writes.get(relation).map_or(&[][..], Vec::as_slice),
                authorization: authorizations.get(relation).copied().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let model_delta =
            kernel_plan::DurableModelDelta::between(plan.source.revision().state(), target.state());
        let model_complement =
            kernel_plan::DurableModelDelta::between(target.state(), plan.source.revision().state());
        let request = kernel_plan::MixedRevisionTransitionRequest {
            source_revision,
            target_revision: &target,
            mutations: &mutation_refs,
            model_delta: &model_delta,
            model_complement: &model_complement,
            registry: runtime.semantic_registry(),
        };
        runtime.commit_mixed_revision_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            &request,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    };
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
            })
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }) => {
            Ok(CommitOutcome::AlreadySatisfied {
                revision: target_revision.into(),
            })
        }
        Ok(kernel_plan::DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }) => {
            Ok(CommitOutcome::AlreadyCommitted {
                revision: target_revision.into(),
            })
        }
        Err(kernel_plan::DurableRuntimeCommitError::TransactionIdConflict { .. }) => {
            Err(Error::new(
                ErrorKind::TransactionConflict,
                "transaction id conflicts with a different committed intent",
            ))
        }
        Err(kernel_plan::DurableRuntimeCommitError::Runtime(
            kernel_plan::PhysicalExecutionError::InvalidRevisionTransition,
        )) => Err(Error::new(
            ErrorKind::StaleRevision,
            "plan source revision is no longer current",
        )),
        Err(error) => Err(Error::new(
            ErrorKind::InvariantViolation,
            format!("commit rejected: {error:?}"),
        )),
    }
}

#[derive(Debug, Clone)]
pub struct ReadContext {
    runtime: Arc<kernel_plan::DurableRuntime>,
    database_identity: u64,
    revision: Option<kernel_revision::Revision>,
    factorized: Option<kernel_durability::DurableFactorizedReadSnapshot>,
    live_snapshot: Option<kernel_plan::RuntimeRevisionSnapshot>,
    contract_context: Option<kernel_schema::SemanticContext>,
    schema_bridge: Option<Arc<kernel_plan::CurrentSchemaBridge>>,
    relational_causal_capture: Option<Arc<Mutex<RelationalCausalCapture>>>,
    pub(crate) authority: RuntimeAuthority,
}

#[derive(Debug, Default)]
struct RelationalCausalCapture {
    next_observation_id: u32,
    observations: BTreeMap<
        (kernel_types::RevisionId, Vec<u8>),
        kernel_plan::RuntimeRelationalCausalObservation,
    >,
}

impl RelationalCausalCapture {
    fn record(&mut self, capsule: kernel_query::RelCausalCapsule) -> Result<()> {
        let revision = capsule.revision();
        let identity =
            kernel_durability::canonical_rel_expr_identity(capsule.query()).map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("relational causal observation identity failed: {error:?}"),
                )
            })?;
        let key = (revision, identity);
        if self.observations.contains_key(&key) {
            return Ok(());
        }
        let observation_id = self.next_observation_id;
        self.next_observation_id = self.next_observation_id.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::ResourceLimit,
                "transaction relational observation id space exhausted",
            )
        })?;
        self.observations.insert(
            key,
            kernel_plan::RuntimeRelationalCausalObservation {
                observation_id,
                observed_revision: revision,
                intent_prefix: kernel_durability::DurableIntentPrefix::empty(),
                capsule,
            },
        );
        Ok(())
    }

    fn values(&self) -> Vec<kernel_plan::RuntimeRelationalCausalObservation> {
        let mut observations = self.observations.values().cloned().collect::<Vec<_>>();
        observations.sort_by_key(|observation| observation.observation_id);
        observations
    }
}

impl ReadContext {
    pub(crate) fn with_current_schema_bridge(
        mut self,
        bridge: kernel_plan::CurrentSchemaBridge,
    ) -> Result<Self> {
        if bridge.target_context() != self.semantic_context() {
            return Err(Error::new(
                ErrorKind::ContractNotRepresentable,
                "current schema bridge target does not match the admitted authoritative world",
            ));
        }
        self.contract_context = Some(bridge.source_context().clone());
        self.schema_bridge = Some(Arc::new(bridge));
        Ok(self)
    }

    fn contract_semantic_context(&self) -> &kernel_schema::SemanticContext {
        self.contract_context
            .as_ref()
            .unwrap_or_else(|| self.semantic_context())
    }

    pub(crate) fn bridged_relation_identity(
        &self,
        relation: crate::RelationId,
    ) -> Result<crate::RelationId> {
        let Some(bridge) = &self.schema_bridge else {
            return Ok(relation);
        };
        bridge
            .transport_relation_identity_exact(relation.into())
            .map(|relation| crate::RelationId::new(relation.raw()))
            .map_err(|error| {
                let diagnostic = crate::MigrationDiagnostic::transport_failure(
                    crate::MigrationWorkflowStage::CutOver,
                    crate::MigrationDiagnosticDomain::WriteBridge,
                    &error,
                );
                Error::new(
                    ErrorKind::ContractNotRepresentable,
                    format!(
                        "relation is not exactly writable through current schema bridge: {error:?}"
                    ),
                )
                .with_migration_diagnostic(diagnostic)
            })
    }

    pub(crate) fn bridged_field_identity(
        &self,
        field: kernel_types::SemanticId,
    ) -> Result<kernel_types::SemanticId> {
        let Some(bridge) = &self.schema_bridge else {
            return Ok(field);
        };
        bridge.transport_field_identity_exact(field).map_err(|error| {
            let diagnostic = crate::MigrationDiagnostic::transport_failure(
                crate::MigrationWorkflowStage::CutOver,
                crate::MigrationDiagnosticDomain::WriteBridge,
                &error,
            );
            Error::new(
                ErrorKind::ContractNotRepresentable,
                format!("field is not definitionally writable through current schema bridge: {error:?}"),
            )
            .with_migration_diagnostic(diagnostic)
        })
    }

    pub(crate) fn bridge_rule_expression_exact(
        &self,
        expression: SemanticRuleExpr,
    ) -> Result<SemanticRuleExpr> {
        fn map_value(
            context: &ReadContext,
            value: &crate::RuleValueExpr,
        ) -> Result<crate::RuleValueExpr> {
            match value {
                crate::RuleValueExpr::Input => Ok(crate::RuleValueExpr::Input),
                crate::RuleValueExpr::Field(field) => context
                    .bridged_field_identity((*field).into())
                    .map(|field| crate::RuleValueExpr::Field(crate::FieldId::new(field.raw()))),
            }
        }

        Ok(match expression {
            SemanticRuleExpr::True => SemanticRuleExpr::True,
            SemanticRuleExpr::False => SemanticRuleExpr::False,
            SemanticRuleExpr::And(parts) => SemanticRuleExpr::And(
                parts
                    .into_iter()
                    .map(|part| self.bridge_rule_expression_exact(part))
                    .collect::<Result<Vec<_>>>()?,
            ),
            SemanticRuleExpr::Or(parts) => SemanticRuleExpr::Or(
                parts
                    .into_iter()
                    .map(|part| self.bridge_rule_expression_exact(part))
                    .collect::<Result<Vec<_>>>()?,
            ),
            SemanticRuleExpr::Not(part) => {
                SemanticRuleExpr::Not(Box::new(self.bridge_rule_expression_exact(*part)?))
            }
            SemanticRuleExpr::I64Range { value, min, max } => SemanticRuleExpr::I64Range {
                value: map_value(self, &value)?,
                min,
                max,
            },
            SemanticRuleExpr::TextLength { value, min, max } => SemanticRuleExpr::TextLength {
                value: map_value(self, &value)?,
                min,
                max,
            },
            SemanticRuleExpr::TextOneOf { value, allowed } => SemanticRuleExpr::TextOneOf {
                value: map_value(self, &value)?,
                allowed,
            },
            SemanticRuleExpr::TextMatches { value, pattern } => SemanticRuleExpr::TextMatches {
                value: map_value(self, &value)?,
                pattern,
            },
            SemanticRuleExpr::Equivalent {
                left,
                right,
                equivalence,
            } => SemanticRuleExpr::Equivalent {
                left: map_value(self, &left)?,
                right: map_value(self, &right)?,
                equivalence,
            },
            SemanticRuleExpr::Ordered {
                left,
                right,
                ordering,
                comparison,
            } => SemanticRuleExpr::Ordered {
                left: map_value(self, &left)?,
                right: map_value(self, &right)?,
                ordering,
                comparison,
            },
        })
    }

    fn bridge_owned_relations_exact(
        &self,
        bridge: &kernel_plan::CurrentSchemaBridge,
        plan: &mut Plan,
    ) -> Result<()> {
        let mut owned_relations = BTreeMap::new();
        for (_, mut contract) in std::mem::take(&mut plan.owned_relations) {
            let source_definition = self
                .contract_semantic_context()
                .schema
                .owned_relationship(contract.relation.into())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::ContractNotRepresentable,
                        "source typed ownership contract is not part of its authoritative schema",
                    )
                })?;
            let source_policy = match contract.orphan_policy {
                crate::plan::OrphanPolicy::Keep => kernel_schema::OrphanPolicyDef::Keep,
                crate::plan::OrphanPolicy::DeleteIfUnowned => {
                    kernel_schema::OrphanPolicyDef::DeleteIfUnowned
                }
            };
            if source_definition.target_relation != contract.target_relation.into()
                || source_definition.orphan_policy != source_policy
            {
                let diagnostic = crate::MigrationDiagnostic::failure(
                    crate::MigrationWorkflowStage::CutOver,
                    crate::MigrationDiagnosticDomain::Lifecycle,
                    crate::MigrationDiagnosticReason::OwnershipContractChanged,
                    vec![crate::MigrationCoordinate::Relation(contract.relation)],
                    "typed OwnedMany contract disagrees with source authoritative lifecycle semantics",
                );
                return Err(Error::new(
                    ErrorKind::ContractNotRepresentable,
                    "typed OwnedMany contract disagrees with source authoritative lifecycle semantics",
                )
                .with_migration_diagnostic(diagnostic));
            }
            let target = bridge
                .transport_owned_relationship_exact(contract.relation.into())
                .map_err(|error| {
                    let diagnostic = crate::MigrationDiagnostic::transport_failure(
                        crate::MigrationWorkflowStage::CutOver,
                        crate::MigrationDiagnosticDomain::Lifecycle,
                        &error,
                    );
                    Error::new(
                        ErrorKind::ContractNotRepresentable,
                        format!(
                            "owned relationship lifecycle contract is not exactly transportable: {error:?}"
                        ),
                    )
                    .with_migration_diagnostic(diagnostic)
                })?;
            contract.relation = RelationId::new(target.relation.raw());
            contract.target_relation = RelationId::new(target.target_relation.raw());
            if owned_relations
                .insert(contract.relation, contract)
                .is_some()
            {
                return Err(Error::new(
                    ErrorKind::ContractNotRepresentable,
                    "distinct source ownership contracts alias one current target relation",
                ));
            }
        }
        plan.owned_relations = owned_relations;
        Ok(())
    }

    pub(crate) fn bridge_plan_exact(&self, mut plan: Plan) -> Result<Plan> {
        let Some(bridge) = &self.schema_bridge else {
            return Ok(plan);
        };
        if !plan.object_field_patches.is_empty()
            || plan.model_delta.is_some()
            || !plan.history_authorization.is_empty()
        {
            let diagnostic = crate::MigrationDiagnostic::failure(
                crate::MigrationWorkflowStage::CutOver,
                crate::MigrationDiagnosticDomain::WriteBridge,
                crate::MigrationDiagnosticReason::WriteNotRepresentable,
                Vec::new(),
                "bridged mutation class has no exact current-schema transport theorem",
            );
            return Err(Error::new(
                ErrorKind::ContractNotRepresentable,
                "bridged object/relationship plan contains a mutation class without an exact current-schema transport theorem",
            )
            .with_migration_diagnostic(diagnostic));
        }

        let map_relation = |relation: RelationId| -> Result<RelationId> {
            bridge
                .transport_relation_identity_exact(relation.into())
                .map(|target| RelationId::new(target.raw()))
                .map_err(|error| Error::new(
                    ErrorKind::ContractNotRepresentable,
                    format!("object/lifecycle relation is not definitionally transportable: {error:?}"),
                ))
        };
        let map_field = |field: crate::FieldId| -> Result<crate::FieldId> {
            bridge
                .transport_field_identity_exact(field.into())
                .map(|target| crate::FieldId::new(target.raw()))
                .map_err(|error| {
                    Error::new(
                        ErrorKind::ContractNotRepresentable,
                        format!(
                            "object reference field is not definitionally transportable: {error:?}"
                        ),
                    )
                })
        };

        let mut mutations = BTreeMap::new();
        for (relation, mutation) in std::mem::take(&mut plan.mutations) {
            let target = map_relation(relation)?;
            if mutations.insert(target, mutation).is_some() {
                return Err(Error::new(
                    ErrorKind::ContractNotRepresentable,
                    "distinct source object/relationship relations alias one current target relation",
                ));
            }
        }
        plan.mutations = mutations;

        let mut actions = BTreeMap::new();
        for ((relation, direction, index), action) in std::mem::take(&mut plan.mutation_actions) {
            let target = map_relation(relation)?;
            if actions.insert((target, direction, index), action).is_some() {
                return Err(Error::new(
                    ErrorKind::ContractNotRepresentable,
                    "bridged mutation-action coordinates alias in the current schema",
                ));
            }
        }
        plan.mutation_actions = actions;

        let mut contracts = BTreeMap::new();
        for (_, mut contract) in std::mem::take(&mut plan.object_contracts) {
            contract.relation = map_relation(contract.relation)?;
            for reference in &mut contract.references {
                reference.field = map_field(reference.field)?;
            }
            if contracts.insert(contract.relation, contract).is_some() {
                return Err(Error::new(
                    ErrorKind::ContractNotRepresentable,
                    "distinct source object contracts alias one current target relation",
                ));
            }
        }
        plan.object_contracts = contracts;

        self.bridge_owned_relations_exact(bridge, &mut plan)?;
        Ok(plan)
    }

    #[cfg(test)]
    pub(crate) fn is_schema_bridged(&self) -> bool {
        self.schema_bridge.is_some()
    }

    pub(crate) fn relation_column_ids(
        &self,
        relation: crate::RelationId,
    ) -> Result<Vec<crate::RelationColumnId>> {
        self.semantic_context()
            .schema
            .relation_column_ids(relation.into())
            .map(|ids| {
                ids.iter()
                    .map(|id| crate::RelationColumnId::new(id.raw()))
                    .collect()
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!(
                        "relation {} has no stable column identities",
                        relation.raw()
                    ),
                )
            })
    }

    pub(crate) fn runtime_authority(&self) -> RuntimeAuthority {
        self.authority.clone()
    }

    pub(crate) const fn database_identity(&self) -> u64 {
        self.database_identity
    }

    pub(crate) fn same_snapshot(&self, other: &Self) -> bool {
        self.database_identity == other.database_identity && self.revision() == other.revision()
    }

    pub(crate) const fn is_live(&self) -> bool {
        self.live_snapshot.is_some()
    }

    pub(crate) const fn runtime_arc(&self) -> &Arc<kernel_plan::DurableRuntime> {
        &self.runtime
    }

    pub(crate) fn live_snapshot_ref(&self) -> Option<&kernel_plan::RuntimeRevisionSnapshot> {
        self.live_snapshot.as_ref().filter(|snapshot| {
            self.revision
                .as_ref()
                .is_some_and(|revision| revision.id() == snapshot.revision().id())
        })
    }

    pub(crate) fn speculative_from_revision(&self, revision: kernel_revision::Revision) -> Self {
        let mut context = self.clone();
        context.revision = Some(revision);
        context.factorized = None;
        // Keep `live_snapshot` as the exact formation Plan source, but `live_snapshot_ref()`
        // deliberately hides it from query scan-seed execution once the read world is speculative.
        context
    }

    pub(crate) fn with_fresh_intent_relational_causal_capture(&self) -> Self {
        self.with_intent_relational_causal_capture()
    }

    pub(crate) fn with_intent_relational_causal_capture(&self) -> Self {
        let mut context = self.clone();
        context.relational_causal_capture =
            Some(Arc::new(Mutex::new(RelationalCausalCapture::default())));
        context
    }

    pub(crate) fn without_intent_relational_causal_capture(&self) -> Self {
        let mut context = self.clone();
        context.relational_causal_capture = None;
        context
    }

    pub(crate) fn relational_causal_observations(
        &self,
    ) -> Result<Vec<kernel_plan::RuntimeRelationalCausalObservation>> {
        let Some(capture) = &self.relational_causal_capture else {
            return Ok(Vec::new());
        };
        capture
            .lock()
            .map_err(|_| {
                Error::new(
                    ErrorKind::Internal,
                    "transaction relational observation capture poisoned",
                )
            })
            .map(|capture| capture.values())
    }

    fn record_relational_causal_observation(
        &self,
        capsule: kernel_query::RelCausalCapsule,
    ) -> Result<()> {
        let Some(capture) = &self.relational_causal_capture else {
            return Ok(());
        };
        capture
            .lock()
            .map_err(|_| {
                Error::new(
                    ErrorKind::Internal,
                    "transaction relational observation capture poisoned",
                )
            })?
            .record(capsule)
    }

    pub(crate) fn kernel_revision(&self) -> &kernel_revision::Revision {
        self.revision
            .as_ref()
            .expect("live ReadContext must own a logical revision")
    }

    fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        match (&self.revision, &self.factorized) {
            (Some(revision), None) => revision.semantic_context(),
            (None, Some(factorized)) => factorized.semantic_context(),
            _ => unreachable!("ReadContext has exactly one read representation"),
        }
    }

    pub fn watch(&self, query: &Query) -> Result<crate::QueryWatch> {
        self.authority.require(Permission::Watch)?;
        crate::QueryWatch::new(self, query)
    }

    pub(crate) fn plan(&self) -> Result<Plan> {
        let snapshot = self.live_snapshot.clone().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidPlan,
                "historical snapshots are read-only; derive a transition explicitly against the live database",
            )
        })?;
        self.authority.require_write_entry()?;
        Ok(Plan::new(
            &self.runtime,
            snapshot,
            self.database_identity,
            self.authority.clone(),
        ))
    }

    pub fn objects<E: crate::Object>(&self) -> Result<crate::ObjectSet<E>> {
        let relation = self.relation::<E>(E::relation_id())?;
        crate::ObjectSet::new(self.clone(), relation)
    }

    pub(crate) fn projected_objects<E: crate::Object>(&self) -> Result<crate::ObjectSet<E>> {
        let relation = self.relation::<E>(E::relation_id())?;
        crate::ObjectSet::new_projected(self.clone(), relation)
    }

    #[must_use]
    pub fn revision(&self) -> RevisionId {
        match (&self.revision, &self.factorized) {
            (Some(revision), None) => revision.id().into(),
            (None, Some(factorized)) => factorized.revision().into(),
            _ => unreachable!("ReadContext has exactly one read representation"),
        }
    }

    pub fn history(&self) -> Result<crate::History> {
        self.authority.require(Permission::HistoryRead)?;
        crate::History::from_runtime_at(
            &self.runtime,
            self.database_identity,
            self.revision(),
            &self.authority,
        )
    }

    fn schema_unchecked(&self) -> SchemaView {
        SchemaView::from_kernel(&self.contract_semantic_context().schema)
    }

    pub fn schema(&self) -> Result<SchemaView> {
        self.authority.require(Permission::ModelRead)?;
        Ok(self.schema_unchecked())
    }

    #[must_use]
    pub fn schema_revision(&self) -> u64 {
        self.semantic_context().schema.revision.raw()
    }

    fn prepare_unchecked(&self, query: &Query) -> Result<PreparedQuery> {
        let expression = if let Some(bridge) = &self.schema_bridge {
            bridge
                .compile_read_exact(&query.inner, self.runtime.semantic_registry())
                .map_err(|error| {
                    let diagnostic = crate::MigrationDiagnostic::transport_failure(
                        crate::MigrationWorkflowStage::CutOver,
                        crate::MigrationDiagnosticDomain::ReadBridge,
                        &error,
                    );
                    Error::new(
                        ErrorKind::ContractNotRepresentable,
                        format!(
                            "read is not exactly representable in the current schema: {error:?}"
                        ),
                    )
                    .with_migration_diagnostic(diagnostic)
                    .with_query(query.node_id(), query.source())
                })?
        } else {
            query.inner.clone()
        };
        let inner = expression
            .prepare(self.semantic_context(), self.runtime.semantic_registry())
            .map_err(|error| query_error_at(query, &error))?;
        Ok(PreparedQuery {
            inner,
            node: query.node_id(),
            source: query.source(),
        })
    }

    pub fn prepare(&self, query: &Query) -> Result<PreparedQuery> {
        let prepared = self.prepare_unchecked(query)?;
        let footprint = prepared
            .inner
            .read_footprint()
            .map_err(|error| query_error_at(query, &error))?;
        self.authority.require_read_footprint(&footprint)?;
        Ok(prepared)
    }

    pub fn execute(&self, query: &Query) -> Result<RelationResult> {
        self.prepare(query)?.execute(self)
    }

    pub(crate) fn execute_for_mutation(&self, query: &Query) -> Result<RelationResult> {
        self.authority.require_write_entry()?;
        self.prepare_unchecked(query)?.execute_unchecked(self)
    }

    pub fn relation<R>(&self, id: crate::RelationId) -> Result<crate::Relation<R>> {
        let schema = self.schema_unchecked();
        let relation = schema.relation(id).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("schema has no relation {id:?}"),
            )
        })?;
        Ok(crate::Relation::from_schema(relation))
    }
}

impl PreparedQuery {
    pub fn execute(&self, context: &ReadContext) -> Result<RelationResult> {
        let footprint = self.inner.read_footprint().map_err(|error| {
            Error::new(
                ErrorKind::Query,
                format!("prepared query authorization footprint failed: {error:?}"),
            )
            .with_query(self.node, self.source)
        })?;
        context.authority.require_read_footprint(&footprint)?;
        self.execute_unchecked(context)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep query execution operator lowering together."
    )]
    fn execute_unchecked(&self, context: &ReadContext) -> Result<RelationResult> {
        let registry = context.runtime.semantic_registry();
        if let Some(factorized) = context.factorized.as_ref() {
            let rows = kernel_realization::evaluate_relation_expr_factorized(
                factorized.root(),
                factorized.atoms(),
                factorized.semantic_context(),
                registry,
                self.inner.expression(),
            )
            .map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("factorized read execution failed: {error:?}"),
                )
                .with_query(self.node, self.source)
            })?;
            let value = match &self.inner.result_type().semantics {
                kernel_schema::RelationSemantics::Bag { .. } => {
                    kernel_query::RelationValue::Bag(rows)
                }
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                } => kernel_query::RelationValue::Set {
                    rows,
                    column_equivalences: column_equivalences.clone(),
                },
            };
            return Ok(value.into());
        }
        let seeded = if let Some(snapshot) = context.live_snapshot_ref() {
            let relations = self.inner.scan_relations();
            if self
                .inner
                .emits_occurrence_certificate_with_scan_seeds(&relations)
            {
                let mut seeds = BTreeMap::new();
                for relation in &relations {
                    let seed = snapshot
                        .relation_scan_occurrence_seed(*relation)
                        .map_err(|error| {
                            Error::new(
                                ErrorKind::Internal,
                                format!("runtime scan-evidence derivation failed: {error:?}"),
                            )
                        })?
                        .ok_or_else(|| {
                            Error::new(
                                ErrorKind::Internal,
                                format!("runtime has no relation witness for {relation:?}"),
                            )
                        })?;
                    seeds.insert(*relation, seed);
                }
                Some(seeds)
            } else {
                None
            }
        } else {
            None
        };
        if context.relational_causal_capture.is_some() {
            let mut state = match seeded.as_ref() {
                Some(seeds) => kernel_query::MaterializedRelPlanState::build_with_scan_seeds(
                    self.inner.expression(),
                    &context.kernel_revision().state().model,
                    context.semantic_context(),
                    registry,
                    seeds,
                ),
                None => kernel_query::MaterializedRelPlanState::build(
                    self.inner.expression(),
                    &context.kernel_revision().state().model,
                    context.semantic_context(),
                    registry,
                ),
            }
            .map_err(|error| {
                Error::new(
                    ErrorKind::Query,
                    format!("transaction query has no exact relational causal program: {error:?}"),
                )
                .with_query(self.node, self.source)
            })?;
            state
                .bind_revision(context.kernel_revision().id())
                .map_err(|error| {
                    Error::new(
                        ErrorKind::Internal,
                        format!(
                            "transaction relational observation revision binding failed: {error:?}"
                        ),
                    )
                    .with_query(self.node, self.source)
                })?;
            let value = state
                .output_value(context.semantic_context(), registry)
                .map_err(|error| {
                    Error::new(
                        ErrorKind::Query,
                        format!("transaction relational observation output failed: {error:?}"),
                    )
                    .with_query(self.node, self.source)
                })?;
            let capsule = kernel_query::RelCausalCapsule::capture(&state).map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("transaction relational causal capsule capture failed: {error:?}"),
                )
                .with_query(self.node, self.source)
            })?;
            context.record_relational_causal_observation(capsule)?;
            return Ok(value.into());
        }
        match seeded {
            Some(seeds) => self.inner.evaluate_seeded(
                &context.kernel_revision().state().model,
                context.semantic_context(),
                registry,
                &seeds,
            ),
            None => self.inner.evaluate(
                &context.kernel_revision().state().model,
                context.semantic_context(),
                registry,
            ),
        }
        .map(Into::into)
        .map_err(|error| query_error(&error).with_query(self.node, self.source))
    }
}

#[cfg(test)]
mod pass515_model_authority_tests {
    use super::*;
    use crate::{
        ErrorKind, MigrationHistoryPolicy, MigrationModel, ModelEntityId, ModelSemanticId,
        Permission, PermissionSet, PrincipalId, Schema, Session, TransactionId, TypeId,
    };
    use kernel_durability::{DurableCarrierPatch, DurableKeepsAlivePatch, DurableModelDelta};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_PASS515_DIR: AtomicU64 = AtomicU64::new(1);

    fn pass515_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cfmd-runtime-pass515-model-authority-{}-{}",
            std::process::id(),
            NEXT_PASS515_DIR.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete stale schema-aware model-coordinate authorization scenario together."
    )]
    fn stale_schema_aware_model_effect_requires_exact_current_world_model_authority() {
        let directory = pass515_dir();
        fs::create_dir_all(&directory).expect("create PASS515 fixture directory");
        let entity_type = TypeId::new(515_001);
        let carrier = ModelSemanticId::new(entity_type.raw());
        let parent = ModelEntityId::new(515_002);
        let child = ModelEntityId::new(515_003);
        let source = Schema::builder()
            .revisions(515, 1)
            .__entity_type(entity_type)
            .build()
            .expect("PASS515 source schema");
        let database = Database::create(&directory, source).expect("create PASS515 database");

        let model_delta = DurableModelDelta {
            carriers: vec![DurableCarrierPatch {
                carrier: kernel_types::SemanticId::new(carrier.raw()),
                target_present: true,
                inserted: vec![
                    kernel_types::EntityId::new(parent.raw()),
                    kernel_types::EntityId::new(child.raw()),
                ],
                removed: Vec::new(),
            }],
            lifecycle_entities_inserted: vec![
                kernel_types::EntityId::new(parent.raw()),
                kernel_types::EntityId::new(child.raw()),
            ],
            lifecycle_roots_inserted: vec![kernel_types::EntityId::new(parent.raw())],
            lifecycle_keeps_alive: vec![DurableKeepsAlivePatch {
                parent: kernel_types::EntityId::new(parent.raw()),
                target_present: true,
                inserted: vec![kernel_types::EntityId::new(child.raw())],
                removed: Vec::new(),
            }],
            ..DurableModelDelta::default()
        };

        let broad = database.session(Session::new(
            PrincipalId::new(515_010),
            PermissionSet::from([Permission::Write]),
        ));
        let exact = database.session(Session::new(
            PrincipalId::new(515_011),
            PermissionSet::from([
                Permission::WriteCarrierPresence(carrier),
                Permission::WriteCarrierMember {
                    carrier,
                    member: parent,
                },
                Permission::WriteCarrierMember {
                    carrier,
                    member: child,
                },
                Permission::WriteLifecycleEntity(parent),
                Permission::WriteLifecycleEntity(child),
                Permission::WriteLifecycleRoot(parent),
                Permission::WriteKeepsAlivePresence(parent),
                Permission::WriteKeepsAliveEdge { parent, child },
            ]),
        ));

        let mut broad_plan = broad.plan().expect("broad formation plan");
        broad_plan.model_delta = Some(model_delta.clone());
        let mut broad_intent = IntentJournal::new()
            .with_idempotency_key(TransactionId::new(9_515_001))
            .expect("broad intent identity");
        broad_intent
            .add_plan(broad_plan)
            .expect("stage broad PASS515 intent");

        let mut exact_plan = exact.plan().expect("exact formation plan");
        exact_plan.model_delta = Some(model_delta);
        let mut exact_intent = IntentJournal::new()
            .with_idempotency_key(TransactionId::new(9_515_002))
            .expect("exact intent identity");
        exact_intent
            .add_plan(exact_plan)
            .expect("stage exact PASS515 intent");

        let target = Schema::builder()
            .revisions(516, 1)
            .__entity_type(entity_type)
            .build()
            .expect("PASS515 target schema");
        database
            .migrate(
                &MigrationModel::new(515_516, target),
                TransactionId::new(9_515_003),
                MigrationHistoryPolicy::Forget,
            )
            .expect("PASS515 identity-preserving schema migration");

        assert_eq!(
            broad
                .preview(&broad_intent)
                .expect_err("generic Write must not cover transported model coordinates")
                .kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            broad
                .intent_readiness(&broad_intent)
                .expect_err("generic Write must not certify transported model-coordinate readiness")
                .kind(),
            ErrorKind::PermissionDenied
        );
        assert_eq!(
            broad
                .commit(&broad_intent)
                .expect_err("generic Write must not publish transported model coordinates")
                .kind(),
            ErrorKind::PermissionDenied
        );

        exact
            .preview(&exact_intent)
            .expect("exact current-world model authority previews stale intent");
        assert!(matches!(
            exact
                .intent_readiness(&exact_intent)
                .expect("exact current-world model authority certifies readiness"),
            IntentReadiness::Rebasable { .. }
        ));
        assert!(matches!(
            exact
                .commit(&exact_intent)
                .expect("exact current-world model authority publishes stale intent"),
            CommitOutcome::Committed { .. }
        ));

        let head = database.runtime.snapshot().expect("PASS515 final snapshot");
        let state = head.revision().state();
        assert_eq!(
            state
                .model
                .carriers
                .get(&kernel_types::SemanticId::new(carrier.raw())),
            Some(&std::collections::BTreeSet::from([
                kernel_types::EntityId::new(parent.raw()),
                kernel_types::EntityId::new(child.raw()),
            ]))
        );
        assert!(
            state
                .lifecycle
                .entities
                .contains(&kernel_types::EntityId::new(parent.raw()))
        );
        assert!(
            state
                .lifecycle
                .entities
                .contains(&kernel_types::EntityId::new(child.raw()))
        );
        assert!(
            state
                .lifecycle
                .roots
                .contains(&kernel_types::EntityId::new(parent.raw()))
        );
        assert!(
            state
                .lifecycle
                .keeps_alive
                .get(&kernel_types::EntityId::new(parent.raw()))
                .is_some_and(
                    |children| children.contains(&kernel_types::EntityId::new(child.raw()))
                )
        );
        drop(head);
        drop(database);
        fs::remove_dir_all(directory).expect("remove PASS515 fixture directory");
    }
}

#[cfg(test)]
mod pass590_persistence_transition_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PASS590_PATH: AtomicU64 = AtomicU64::new(1);

    fn pass590_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cfmd-pass590-{name}-{}-{}",
            std::process::id(),
            NEXT_PASS590_PATH.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[test]
    fn durable_volatile_durable_roundtrip_keeps_one_runtime_identity() {
        let source = pass590_path("source");
        let target_dir = pass590_path("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("repersisted.cfmd");
        let schema = Schema::builder().revisions(590, 1).build().unwrap();
        let database = Database::create(&source, schema).unwrap();
        let identity = database.runtime_identity();

        assert!(!database.is_memory());
        database.make_volatile().unwrap();
        assert!(database.is_memory());
        assert_eq!(database.runtime_identity(), identity);
        database.persist(&target).unwrap();
        assert!(!database.is_memory());
        assert_eq!(database.runtime_identity(), identity);

        drop(database);
        std::fs::remove_file(source).unwrap();
        std::fs::remove_dir_all(target_dir).unwrap();
    }
}
