use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    CommitOutcome, Error, ErrorKind, Plan, PreparedQuery, Query, RelationResult, Result,
    RevisionId, Schema, SchemaView, TransactionId,
    query::{query_error, query_error_at},
    schema::{
        PrimitiveEquivalence, PrimitiveOrdering, RelationSemantics, StructuralEquivalence,
        type_to_kernel,
    },
    security::{Permission, RuntimeAuthority, Session, SessionDatabase},
};

#[derive(Debug, Clone)]
pub struct Database {
    runtime: Arc<kernel_plan::DurableRuntime>,
    path: Arc<PathBuf>,
    identity: u64,
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

fn compile_schema(
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
    for relation in relations.values() {
        schema
            .define_relation(kernel_schema::RelationDef {
                id: relation.id().into(),
                columns: relation.columns().iter().map(type_to_kernel).collect(),
                semantics: relation_semantics(relation.semantics()),
            })
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidSchema,
                    format!("invalid relation schema: {error:?}"),
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
    let live = |value: crate::EntityRef| kernel_model::Value::LiveEntityRef {
        entity_type: reference.target_type.into(),
        id: lifecycle_entity_id(reference.target_type, value.id),
    };
    match (value, reference.optional) {
        (crate::Value::HistoricalEntityRef(value), false)
            if value.entity_type == reference.target_type =>
        {
            Ok(live(*value))
        }
        (crate::Value::Option(None), true) => Ok(kernel_model::Value::Option(None)),
        (crate::Value::Option(Some(value)), true) => match value.as_ref() {
            crate::Value::HistoricalEntityRef(value)
                if value.entity_type == reference.target_type =>
            {
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
            .ok_or_else(|| Error::new(ErrorKind::InvalidPlan, "object relation is missing"))?
            .clone();
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
                .get(&relation)
                .cloned()
                .unwrap_or_default(),
        );
        let next = relation_value(
            target
                .state()
                .model
                .relations
                .get(&relation)
                .cloned()
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
    if plan.model_delta.is_some() {
        build_explicit_model_target(plan, target_revision, registry)
    } else if plan.object_contracts.is_empty() {
        let deltas = plan_relation_deltas(plan, registry)?;
        let mutations = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
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

    pub fn snapshot(&self) -> Result<ReadContext> {
        let snapshot = self.runtime.snapshot().map_err(|error| {
            Error::new(ErrorKind::Internal, format!("snapshot failed: {error:?}"))
        })?;
        let revision = snapshot.revision().clone();
        Ok(ReadContext {
            runtime: Arc::clone(&self.runtime),
            database_identity: self.identity,
            revision,
            live_snapshot: Some(snapshot),
            authority: RuntimeAuthority::Unrestricted,
        })
    }

    /// Returns one exact immutable committed world at `revision`.
    ///
    /// Historical worlds share the ordinary read/query vocabulary but do not
    /// carry a write capability. Reconstruction uses the durable causal effect
    /// authority; unavailable/non-reversible history fails closed.
    pub fn at(&self, revision: RevisionId) -> Result<ReadContext> {
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
            revision,
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

    /// Starts one write intent pinned to the current exact live snapshot.
    pub fn transaction(&self, transaction: TransactionId) -> Result<crate::Transaction> {
        crate::Transaction::new(self.snapshot()?, transaction)
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
            revision,
            live_snapshot: Some(snapshot),
            authority,
        })
    }

    pub(crate) fn at_with_authority(
        &self,
        revision: RevisionId,
        authority: RuntimeAuthority,
    ) -> Result<ReadContext> {
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
            revision,
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

    pub fn commit(&self, plan: &Plan, transaction: TransactionId) -> Result<CommitOutcome> {
        if plan.database_identity != self.identity {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "plan belongs to a different open database instance",
            ));
        }
        commit_bound_plan(&self.runtime, plan, transaction)
    }
}

pub(crate) fn commit_bound_plan(
    runtime: &kernel_plan::DurableRuntime,
    plan: &Plan,
    transaction: TransactionId,
) -> Result<CommitOutcome> {
    plan.authority.require(Permission::Write)?;
    if plan.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidPlan,
            "cannot commit an empty plan",
        ));
    }
    let source_revision = plan.source.revision().id();
    let target_revision = plan_target_revision_id(plan)?;
    let transaction_id = kernel_types::ClientTransactionId::new(transaction.raw());
    let outcome = if let Some(model_delta) = &plan.model_delta {
        let deltas = plan_relation_deltas(plan, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
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
        runtime.commit_mixed_revision(transaction_id, &request)
    } else if plan.object_contracts.is_empty() {
        let deltas = plan_relation_deltas(plan, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
            })
            .collect::<Vec<_>>();
        let request = kernel_plan::DerivedRelationTransitionRequest {
            source_revision,
            target_revision,
            mutations: &mutation_refs,
        };
        runtime.commit_derived_relation_data(transaction_id, &request)
    } else {
        let target = build_object_target(plan, target_revision, runtime.semantic_registry())?;
        let deltas =
            revision_relation_deltas(plan.source.revision(), &target, runtime.semantic_registry())?;
        let mutation_refs = deltas
            .iter()
            .map(|(relation, delta)| kernel_plan::RevisionRelationMutation {
                relation: *relation,
                delta,
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
        runtime.commit_mixed_revision(transaction_id, &request)
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
    revision: kernel_revision::Revision,
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

    pub(crate) const fn kernel_revision(&self) -> &kernel_revision::Revision {
        &self.revision
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
        self.authority.require(Permission::Write)?;
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

    #[must_use]
    pub fn revision(&self) -> RevisionId {
        self.revision.id().into()
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

    #[must_use]
    pub fn schema(&self) -> SchemaView {
        SchemaView::from_kernel(&self.revision.semantic_context().schema)
    }

    pub fn prepare(&self, query: &Query) -> Result<PreparedQuery> {
        query
            .inner
            .prepare(
                self.revision.semantic_context(),
                self.runtime.semantic_registry(),
            )
            .map(|inner| PreparedQuery {
                inner,
                node: query.node_id(),
                source: query.source(),
            })
            .map_err(|error| query_error_at(query, &error))
    }

    pub fn execute(&self, query: &Query) -> Result<RelationResult> {
        self.prepare(query)?.execute(self)
    }

    pub fn relation<R>(&self, id: crate::RelationId) -> Result<crate::Relation<R>> {
        let schema = self.schema();
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
        self.inner
            .evaluate(
                &context.revision.state().model,
                context.revision.semantic_context(),
                context.runtime.semantic_registry(),
            )
            .map(Into::into)
            .map_err(|error| query_error(&error).with_query(self.node, self.source))
    }
}
