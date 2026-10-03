use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    CandidatePreview, CommitOutcome, Error, ErrorKind, Plan, PreparedQuery, Query, RelationId,
    RelationResult, Result, RevisionId, Row, Schema, SchemaView, SemanticRuleExpr, Transaction,
    TransactionId, TransactionReadiness,
    query::{query_error, query_error_at},
    schema::{
        PrimitiveEquivalence, PrimitiveOrdering, RelationSemantics, StructuralEquivalence,
        semantic_rule_to_kernel, text_pattern_to_kernel, type_to_kernel,
    },
    security::{Permission, RuntimeAuthority, Session, SessionDatabase},
};

#[derive(Debug, Clone)]
pub struct Database {
    runtime: Arc<kernel_plan::DurableRuntime>,
    path: Arc<PathBuf>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Storage {
    #[default]
    Auto,
    SingleFile,
    Directory,
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
            )),
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

    pub fn create(self) -> Result<Database> {
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
        Ok(Database::from_runtime(runtime, self.path))
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
        let runtime = if let Some(notifier) = self.publication_notifier {
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
        .map_err(|error| Error::new(ErrorKind::Recovery, format!("open failed: {error:?}")))?;
        Ok(Database::from_runtime(runtime, self.path))
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
        entity_fields,
        field_rules,
        relation_column_rules,
        entity_rules,
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
                if row.get(patch.identity_column) == Some(&identity) {
                    if matching_position.replace(position).is_some() {
                        return Err(Error::new(
                            ErrorKind::Cardinality,
                            "object field patch identity matched more than one row",
                        ));
                    }
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
                authorization: Default::default(),
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
    }
}

fn validate_transaction_requirements(
    plan: &Plan,
    requirements: &[crate::transaction::TransactionRequirement],
) -> Result<()> {
    if requirements.is_empty() {
        return Ok(());
    }
    let target = build_plan_target(plan)?;
    let context = target.semantic_context();
    for requirement in requirements {
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
            for field in fields {
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
        let matches =
            kernel_validation::relation_row_rule_matches(&expression, relation, row, context)
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
    Ok(())
}

fn plan_object_field_writes(
    plan: &Plan,
) -> BTreeMap<kernel_types::SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>> {
    let mut field_writes =
        BTreeMap::<kernel_types::SemanticId, Vec<kernel_durability::DurableObjectFieldWrite>>::new(
        );
    for ((relation, _), patch) in &plan.object_field_patches {
        let writes = field_writes.entry((*relation).into()).or_default();
        for (_column, (value, field)) in &patch.fields {
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
        rebased.model_delta = plan.model_delta.clone();
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

fn commit_certified_relation_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
    let outcome = runtime.commit_derived_relation_data_residual_guarded(
        transaction_id,
        &request,
        &client_refs,
        client_guard_digest,
    );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
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

fn commit_certified_field_reapply_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    plan: &Plan,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
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
    rebased.model_delta = plan.model_delta.clone();
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
    let outcome = runtime.commit_mixed_revision_residual_guarded(
        transaction_id,
        &request,
        &client_refs,
        &effect.model_delta,
        client_guard_digest,
    );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
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

fn commit_certified_mixed_residual(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    effect: &RebasablePlanEffect,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
                .map(Vec::as_slice)
                .unwrap_or(&[]),
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
    let outcome = runtime.commit_mixed_revision_residual_guarded(
        transaction_id,
        &request,
        &client_refs,
        &effect.model_delta,
        client_guard_digest,
    );
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(Some(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
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
    #[must_use]
    pub fn builder(path: impl Into<PathBuf>) -> DatabaseBuilder {
        DatabaseBuilder {
            path: path.into(),
            storage: Storage::Auto,
            schema: None,
            encryption: Encryption::None,
            publication_notifier: None,
        }
    }

    fn from_runtime(runtime: kernel_plan::DurableRuntime, path: PathBuf) -> Self {
        Self {
            runtime: Arc::new(runtime),
            path: Arc::new(path),
            identity: next_database_identity(),
        }
    }

    pub fn create(path: impl Into<PathBuf>, definition: Schema) -> Result<Self> {
        Self::builder(path).schema(definition).create()
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::builder(path).open()
    }

    /// Applies one already-declared deterministic schema migration model.
    ///
    /// Frontends are expected to compile their own DSL/codegen into `MigrationModel`; runtime
    /// never executes arbitrary host callbacks during the migration. Historical inversion is
    /// currently available only as an explicit `Forget` policy until complement compilation is
    /// generalized for structural migrations.
    pub fn migrate(
        &self,
        model: &crate::MigrationModel,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        self.migrate_with_authority(model, transaction, history, &RuntimeAuthority::Unrestricted)
    }

    pub(crate) fn migrate_with_authority(
        &self,
        model: &crate::MigrationModel,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
        authority: &RuntimeAuthority,
    ) -> Result<CommitOutcome> {
        authority.require(Permission::SchemaMigrate)?;
        let source_snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("migration snapshot failed: {error:?}"),
            )
        })?;
        let source = source_snapshot.revision();
        let (target_context, _, _) = compile_schema(model.target())?;
        let registry = self.runtime.semantic_registry();

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
            .map(|rule| -> crate::Result<kernel_transport::MigrationRelationRewrite> {
                Ok(match rule {
                    crate::MigrationRelationRule::Query { target, query } => {
                        kernel_transport::MigrationRelationRewrite::Query(
                            kernel_transport::RelationRewrite {
                                target_relation: (*target).into(),
                                transform: query.inner.clone(),
                            },
                        )
                    }
                    crate::MigrationRelationRule::Rows {
                        source: source_relation,
                        target: target_relation,
                        columns,
                    } => {
                        let source_relation_id: kernel_types::SemanticId = (*source_relation).into();
                        let target_relation_id: kernel_types::SemanticId = (*target_relation).into();
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
                        let target_column_ids = target_context
                            .schema
                            .relation_column_ids(target_relation_id)
                            .ok_or_else(|| {
                                Error::new(
                                    ErrorKind::InvalidSchema,
                                    "migration references unknown target relation",
                                )
                            })?;
                        kernel_transport::MigrationRelationRewrite::Rows(
                            kernel_transport::MigrationRowRewrite {
                                source_relation: source_relation_id,
                                target_relation: target_relation_id,
                                columns: columns
                                    .iter()
                                    .map(|column| {
                                        let source_columns = column
                                            .source_columns
                                            .iter()
                                            .map(|ordinal| {
                                                source_column_ids.get(*ordinal).copied().ok_or_else(|| {
                                                    Error::new(
                                                        ErrorKind::InvalidSchema,
                                                        format!(
                                                            "migration references unknown source column {ordinal}"
                                                        ),
                                                    )
                                                })
                                            })
                                            .collect::<crate::Result<Vec<_>>>()?;
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
                                    })
                                    .collect::<crate::Result<Vec<_>>>()?,
                            },
                        )
                    }
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let migration_program = kernel_transport::SchemaMigrationProgram::new(
            target_context,
            field_rewrites,
            relation_rewrites,
        );
        let transport = migration_program
            .verify(source.semantic_context(), registry)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("migration model is not valid for the current schema: {error:?}"),
                )
            })?;
        let target_id = source
            .id()
            .raw()
            .checked_add(1)
            .map(kernel_types::RevisionId::new)
            .ok_or_else(|| Error::new(ErrorKind::ResourceLimit, "revision id space exhausted"))?;
        let target = transport
            .transport_revision(source, target_id, registry)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvariantViolation,
                    format!("migration execution rejected: {error:?}"),
                )
            })?;

        let complement = match history {
            crate::MigrationHistoryPolicy::Forget => {
                kernel_durability::DurableMigrationComplement::from_capsule(
                    kernel_lens::ComplementCapsule {
                        source_schema: source.semantic_revision().schema,
                        target_schema: target.semantic_revision().schema,
                        lens_spec: kernel_lens::LensSpecId(kernel_types::SemanticId::new(
                            model.id(),
                        )),
                        semantic_pins: kernel_lens::SemanticManifestId(
                            kernel_types::SemanticId::new(
                                model.id() ^ 0x4346_4d44_4d49_4752_4154_494f_4e00_0001,
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
            target_revision: &target,
            registry,
        };
        authority.with_permission(Permission::SchemaMigrate, || {
            match self.runtime.migrate_schema(
                kernel_types::ClientTransactionId::new(transaction.raw()),
                &request,
                &migration_program,
                &complement,
            ) {
                Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
                    Ok(CommitOutcome::Committed {
                        revision: receipt.durable.target_revision().into(),
                    })
                }
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

    pub fn rewrap_encryption(&self, next: &Encryption) -> Result<u64> {
        let Encryption::Aes256GcmSivProvider { provider } = next else {
            return Err(Error::new(
                ErrorKind::Recovery,
                "database master key rewrap requires a provider-backed encryption policy",
            ));
        };
        let (key, metadata) = resolve_provider_key(
            provider.as_ref(),
            &self.path,
            EncryptionKeyOperation::Rewrap,
        )?;
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
                    Error::new(
                        ErrorKind::Recovery,
                        format!("database master key rewrap failed: {error:?}"),
                    )
                })?;
        provider
            .acknowledge_database_key_epoch(
                &self.path,
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
                        "external database-key acknowledgement failed after local durable handoff; retry is idempotent: {error}"
                    ),
                )
            })?;
        self.runtime
            .retire_previous_storage_encryption_key(database_key_epoch)
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("database master key predecessor retirement failed: {error:?}"),
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
            authority: RuntimeAuthority::Unrestricted,
        })
    }

    pub fn current_revision(&self) -> Result<RevisionId> {
        self.snapshot().map(|snapshot| snapshot.revision())
    }

    pub fn history(&self) -> Result<crate::History> {
        self.snapshot()?.history()
    }

    /// Adds the exact compensating inverse of one durable history entry to an ordinary
    /// transaction. The database remains the visible authority; the history entry only describes
    /// which committed effect is being inverted.
    pub fn undo(&self, transaction: &mut Transaction, entry: &crate::HistoryEntry) -> Result<()> {
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
    pub fn undo_latest(&self, transaction: &mut Transaction) -> Result<()> {
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

    pub(crate) fn projected_objects<E: crate::Object>(&self) -> Result<crate::ObjectSet<E>> {
        self.snapshot()?.projected_objects::<E>()
    }

    /// Reports whether this transaction can be applied to the current head without changing its
    /// exact semantic effect. A newer global revision is not itself a conflict: the kernel proves
    /// transport across intervening exact effects by Γ-canonical write coordinates. Overlap or
    /// opaque history fails closed.
    pub fn transaction_readiness(&self, transaction: &Transaction) -> Result<TransactionReadiness> {
        self.require_transaction_owner(transaction)?;
        let Some(base_revision) = transaction.origin_revision() else {
            return Ok(TransactionReadiness::Unbound);
        };
        let current_revision = self.current_revision()?;
        if current_revision == base_revision {
            return Ok(TransactionReadiness::Ready {
                revision: current_revision,
            });
        }
        if transaction.is_snapshot_bound() {
            return Ok(TransactionReadiness::SnapshotChanged {
                snapshot_revision: base_revision,
                current_revision,
            });
        }
        let plan = transaction.plan()?;
        let effect = plan_rebasable_effect(plan)?;
        match certify_plan_rebase(&self.runtime, plan, &effect)? {
            kernel_plan::RuntimeTransitionRebaseOutcome::Certified(certificate) => {
                Ok(TransactionReadiness::Rebasable {
                    base_revision,
                    current_revision: certificate.current_revision.into(),
                    intervening_effect_count: certificate.intervening_effect_count,
                })
            }
            kernel_plan::RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                Ok(TransactionReadiness::Conflict {
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
    pub fn preview(&self, transaction: &Transaction) -> Result<CandidatePreview> {
        self.require_transaction_owner(transaction)?;
        let plan = transaction.plan()?;
        let base_revision = plan.base_revision();
        let current_revision = self.current_revision()?;
        if current_revision == base_revision {
            validate_transaction_requirements(plan, transaction.requirements())?;
            return Ok(plan.candidate()?.preview());
        }
        if transaction.is_snapshot_bound() {
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
            authority,
        })
    }

    pub(crate) fn at_with_authority(
        &self,
        revision: RevisionId,
        authority: RuntimeAuthority,
    ) -> Result<ReadContext> {
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
    pub fn commit(&self, transaction: &Transaction) -> Result<CommitOutcome> {
        self.require_transaction_owner(transaction)?;
        let plan = transaction.plan()?;
        let transaction_id = transaction.transaction_id()?;
        let client_guard_digest = transaction.client_guard_digest();
        // A stale adaptive transaction cannot publish its original Candidate. Avoid building that
        // obsolete future world solely to evaluate requirements: requirements are checked on the
        // exact rebased Candidate below. We still try the original durable intent first so an
        // uncertain retry can return `AlreadyCommitted` with its original client identity.
        if self.current_revision()? == plan.base_revision() {
            validate_transaction_requirements(plan, transaction.requirements())?;
        }
        match commit_bound_plan(&self.runtime, plan, transaction_id, client_guard_digest) {
            Ok(outcome) => return Ok(outcome),
            Err(error) if error.kind() == ErrorKind::StaleRevision => {}
            Err(error) => return Err(error),
        }
        if transaction.is_snapshot_bound() {
            return Err(Error::new(
                ErrorKind::StaleRevision,
                "snapshot-bound transaction cannot be transported to a newer database revision",
            ));
        }

        let effect = plan_rebasable_effect(plan)?;
        match certify_plan_rebase(&self.runtime, plan, &effect)? {
            kernel_plan::RuntimeTransitionRebaseOutcome::Certified(_) => {
                if !transaction.requirements().is_empty() {
                    let rebased_for_requirements =
                        exact_rebased_plan(&self.runtime, plan, plan_rebasable_effect(plan)?)?;
                    validate_transaction_requirements(
                        &rebased_for_requirements,
                        transaction.requirements(),
                    )?;
                }
                // Recheck the original semantic intent immediately before publication. Residual
                // rows are an internal realization of this already-authorized action.
                authorize_bound_plan(plan)?;
                if let Some(outcome) = commit_certified_field_reapply_residual(
                    &self.runtime,
                    plan,
                    &effect,
                    transaction_id,
                    client_guard_digest,
                )? {
                    return Ok(outcome);
                }
                if let Some(outcome) = commit_certified_relation_residual(
                    &self.runtime,
                    plan,
                    &effect,
                    transaction_id,
                    client_guard_digest,
                )? {
                    return Ok(outcome);
                }
                if let Some(outcome) = commit_certified_mixed_residual(
                    &self.runtime,
                    &effect,
                    transaction_id,
                    client_guard_digest,
                )? {
                    return Ok(outcome);
                }
                let rebased = exact_rebased_plan(&self.runtime, plan, effect)?;
                commit_bound_plan_authorized(
                    &self.runtime,
                    &rebased,
                    transaction_id,
                    client_guard_digest,
                )
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
    pub(crate) fn commit_exact_relation_intent(
        &self,
        formation_revision: RevisionId,
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

        let mut normalized = BTreeMap::<RelationId, (Vec<Row>, Vec<Row>)>::new();
        for mutation in mutations {
            authority.require_write_relation(mutation.relation)?;
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
                authorization: Default::default(),
            })
            .collect::<Vec<_>>();
        let current_revision = self.current_revision()?;
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
                authorization: Default::default(),
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
                authorization: Default::default(),
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
        commit_bound_plan(&self.runtime, plan, transaction, None)
    }

    fn require_transaction_owner(&self, transaction: &Transaction) -> Result<()> {
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

fn authorize_bound_plan(plan: &Plan) -> Result<()> {
    plan.authority.require_write_entry()?;
    for (relation, coverage) in &plan.history_authorization {
        let authorization = coverage.authorization;
        if authorization.relation_write {
            plan.authority.require_write_relation(*relation)?;
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
                plan.authority.require_mutation_action(*relation, action)?;
            }
        }
        for field in &coverage.fields {
            plan.authority.require_write_field(*relation, *field)?;
        }
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
                Some(action) => plan.authority.require_mutation_action(*relation, *action)?,
                None => plan.authority.require_write_relation(*relation)?,
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
                Some(action) => plan.authority.require_mutation_action(*relation, *action)?,
                None => plan.authority.require_write_relation(*relation)?,
            }
        }
    }
    for ((relation, _), patch) in &plan.object_field_patches {
        for (_, field) in patch.fields.values() {
            plan.authority
                .require_write_field(*relation, crate::RelationColumnId::new(field.raw()))?;
        }
    }
    // Detaching an exclusively-owned edge under DeleteIfUnowned can delete the target object.
    // That induced lifecycle effect must never be a route around explicit object-delete authority.
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
            plan.authority.require_mutation_action(
                contract.target_relation,
                crate::plan::MutationAction::ObjectDelete,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn commit_bound_plan(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
) -> Result<CommitOutcome> {
    authorize_bound_plan(plan)?;
    commit_bound_plan_authorized(runtime, plan, transaction, client_guard_digest)
}

fn commit_bound_plan_authorized(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
    client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
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
                object_field_writes: field_writes.get(relation).map(Vec::as_slice).unwrap_or(&[]),
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
        runtime.commit_mixed_revision_guarded(transaction_id, &request, client_guard_digest)
    } else if plan.object_contracts.is_empty() {
        let deltas = plan_relation_deltas(plan, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: field_writes.get(relation).map(Vec::as_slice).unwrap_or(&[]),
                authorization: authorizations.get(relation).copied().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let request = kernel_plan::DerivedRelationTransitionRequest {
            source_revision,
            target_revision,
            mutations: &mutation_refs,
        };
        runtime.commit_derived_relation_data_guarded(transaction_id, &request, client_guard_digest)
    } else {
        let target = build_object_target(plan, target_revision, runtime.semantic_registry())?;
        let deltas =
            revision_relation_deltas(plan.source.revision(), &target, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
                object_field_writes: field_writes.get(relation).map(Vec::as_slice).unwrap_or(&[]),
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
        runtime.commit_mixed_revision_guarded(transaction_id, &request, client_guard_digest)
    };
    match outcome {
        Ok(kernel_plan::DurableRuntimeCommitOutcome::Committed(receipt)) => {
            Ok(CommitOutcome::Committed {
                revision: receipt.durable.target_revision().into(),
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
    pub(crate) authority: RuntimeAuthority,
}

impl ReadContext {
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
        self.live_snapshot.as_ref()
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
        SchemaView::from_kernel(&self.semantic_context().schema)
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
        let inner = query
            .inner
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
