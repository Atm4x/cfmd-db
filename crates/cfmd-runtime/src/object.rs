use crate::{
    Error, ErrorKind, Field, Plan, PrimitiveEquivalence, PrimitiveOrdering, Projection, Query,
    ReadContext, Relation, RelationId, RelationQuery, Result, RowCodec, SchemaBuilder, Type,
    TypedQuery, Value, ValueCodec,
};

const FNV128_OFFSET: u128 = 144_066_263_297_769_815_596_495_629_667_062_367_629;
const FNV128_PRIME: u128 = 309_485_009_821_345_068_724_781_371;

const fn hash_bytes(mut hash: u128, bytes: &[u8]) -> u128 {
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u128;
        hash = hash.wrapping_mul(FNV128_PRIME);
        index += 1;
    }
    hash
}

/// Stable deterministic product-layer semantic identity derived from explicit textual keys.
///
/// The mapping is intentionally simple and language-portable. Collisions remain fail-closed at
/// schema construction; callers should use stable, application-qualified object keys.
#[doc(hidden)]
#[must_use]
pub const fn __semantic_id(domain: &str, owner: &str, member: &str) -> u128 {
    let hash = hash_bytes(FNV128_OFFSET, domain.as_bytes());
    let hash = hash_bytes(hash, &[0]);
    let hash = hash_bytes(hash, owner.as_bytes());
    let hash = hash_bytes(hash, &[0]);
    hash_bytes(hash, member.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectEquivalence {
    Primitive(PrimitiveEquivalence),
    OptionOf(PrimitiveEquivalence),
}

#[must_use]
pub(crate) fn count_equivalence_id() -> crate::EquivalenceId {
    crate::EquivalenceId::new(__semantic_id("cfmd.runtime.aggregate.v1", "count", "i64"))
}

/// A Rust value that can appear as an object field with a default CFMD semantic equality.
pub trait ObjectValue: ValueCodec {
    fn object_type() -> Type;
    fn equivalence() -> ObjectEquivalence;
    #[must_use]
    fn ordering() -> Option<PrimitiveOrdering> {
        None
    }
    #[must_use]
    fn role() -> ObjectFieldRole {
        ObjectFieldRole::Value
    }
}

pub trait OrderedObjectValue: ObjectValue {}

macro_rules! scalar_object_value {
    ($rust:ty, $type_expr:expr, $equivalence:expr, $ordering:expr) => {
        impl ObjectValue for $rust {
            fn object_type() -> Type {
                $type_expr
            }

            fn equivalence() -> ObjectEquivalence {
                ObjectEquivalence::Primitive($equivalence)
            }
            fn ordering() -> Option<PrimitiveOrdering> {
                Some($ordering)
            }
        }
        impl OrderedObjectValue for $rust {}
    };
}

scalar_object_value!(
    (),
    Type::unit(),
    PrimitiveEquivalence::UnitExact,
    PrimitiveOrdering::UnitExact
);
scalar_object_value!(
    bool,
    Type::bool(),
    PrimitiveEquivalence::BoolExact,
    PrimitiveOrdering::BoolAscending
);
scalar_object_value!(
    i64,
    Type::i64(),
    PrimitiveEquivalence::I64Exact,
    PrimitiveOrdering::I64Ascending
);
scalar_object_value!(
    f64,
    Type::f64(),
    PrimitiveEquivalence::F64Bitwise,
    PrimitiveOrdering::F64Total
);
scalar_object_value!(
    String,
    Type::text(),
    PrimitiveEquivalence::TextExact,
    PrimitiveOrdering::TextBinary
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRelationshipCardinality {
    RequiredOne,
    OptionalOne,
    Many,
}

/// First-class object relationship value for a zero-to-many edge.
///
/// The relationship itself is stored by CFMD as an internal edge relation, not as an embedded
/// collection column. A materialized `Many<T>` is bound to the snapshot that produced its owner.
/// Merely accessing the field performs no I/O; [`Many::query`] and [`Many::load`] make reads
/// explicit and therefore keep accidental N+1 behavior visible in application code.
pub struct Many<T> {
    binding: Option<ManyBinding>,
    pending: Option<Vec<T>>,
}

#[derive(Clone)]
struct ManyBinding {
    context: ReadContext,
    relation: RelationId,
    source_id: u128,
    source_type: crate::TypeId,
    source_equivalence: crate::EquivalenceId,
    target_equivalence: crate::EquivalenceId,
}

impl<T> std::fmt::Debug for Many<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Many")
            .field("bound", &self.binding.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: Clone> Clone for Many<T> {
    fn clone(&self) -> Self {
        Self {
            binding: self.binding.clone(),
            pending: self.pending.clone(),
        }
    }
}

impl<T> Default for Many<T> {
    fn default() -> Self {
        Self::__new()
    }
}

impl<T: PartialEq> PartialEq for Many<T> {
    fn eq(&self, other: &Self) -> bool {
        self.pending == other.pending
            && match (&self.binding, &other.binding) {
                (None, None) => true,
                (Some(left), Some(right)) => {
                    left.relation == right.relation
                        && left.source_id == right.source_id
                        && left.context.revision() == right.context.revision()
                }
                _ => false,
            }
    }
}

impl<T: Eq> Eq for Many<T> {}

impl<T> Many<T> {
    #[doc(hidden)]
    #[must_use]
    pub const fn __new() -> Self {
        Self {
            binding: None,
            pending: None,
        }
    }

    /// Creates an explicit detached relationship value for a new object graph.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            binding: None,
            pending: Some(values.into_iter().collect()),
        }
    }

    /// Creates an explicit empty relationship for a new object graph.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    #[doc(hidden)]
    pub fn __into_pending(self) -> Result<Vec<T>> {
        if self.binding.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "cannot insert a snapshot-bound Many<T> as a detached object graph",
            ));
        }
        Ok(self.pending.unwrap_or_default())
    }

    #[doc(hidden)]
    pub fn __bind(
        &mut self,
        context: ReadContext,
        relation: RelationId,
        source_id: u128,
        source_type: crate::TypeId,
        source_equivalence: crate::EquivalenceId,
        target_equivalence: crate::EquivalenceId,
    ) {
        self.binding = Some(ManyBinding {
            context,
            relation,
            source_id,
            source_type,
            source_equivalence,
            target_equivalence,
        });
        self.pending = None;
    }

    #[doc(hidden)]
    pub fn __preserves_binding(
        &self,
        context: &ReadContext,
        relation: RelationId,
        source_id: u128,
    ) -> Result<bool> {
        let Some(binding) = &self.binding else {
            return Ok(false);
        };
        if binding.relation != relation
            || binding.source_id != source_id
            || !binding.context.same_snapshot(context)
        {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "cannot reuse a Many<T> relationship value from a different owner or snapshot",
            ));
        }
        Ok(true)
    }

    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.binding.is_some()
    }
}

impl<T: Object> Many<T> {
    /// Builds an object query for this concrete relationship value. No I/O occurs until the
    /// returned query is evaluated.
    pub fn query(&self) -> Result<ObjectQuery<T>> {
        let binding = self.binding.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidPlan,
                "Many<T> is not bound to a database snapshot; materialize its owner through CFMD before querying the relationship",
            )
        })?;
        let set = binding.context.objects::<T>()?;
        let target_identity = T::identity_column().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("relationship target {} has no identity", T::KEY),
            )
        })?;
        let edges = Query::scan(binding.relation).filter_eq(
            0,
            Value::HistoricalEntityRef(crate::EntityRef {
                entity_type: binding.source_type,
                id: binding.source_id,
            }),
            binding.source_equivalence,
        );
        let target = Query::scan(T::relation_id())
            .join_eq(edges, target_identity, 1, binding.target_equivalence)
            .project((0..set.relation.width()).collect::<Vec<_>>());
        Ok(ObjectQuery {
            context: binding.context.clone(),
            relation: set.relation.clone(),
            inner: RelationQuery::__from_raw(set.relation.clone(), target),
        })
    }

    /// Narrows this concrete relationship using ordinary object-first predicates. The returned
    /// selection remains a relationship value: it can be read or mutated without materializing
    /// target objects merely to discover their identities.
    pub fn where_<F, P>(&self, predicate: F) -> Result<ManySelection<T>>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: crate::ObjectPredicate<T>,
    {
        let binding = many_binding(self)?.clone();
        Ok(ManySelection {
            binding,
            query: self.query()?.where_(predicate),
        })
    }

    /// Explicitly materializes this relationship at the owner's snapshot.
    pub fn load(&self) -> Result<Vec<T>> {
        self.query()?.all()
    }

    /// Object-first alias for explicit full relationship materialization.
    pub fn all(&self) -> Result<Vec<T>> {
        self.load()
    }

    pub fn first_or_none(&self) -> Result<Option<T>> {
        self.query()?.first_or_none()
    }

    pub fn one_or_none(&self) -> Result<Option<T>> {
        self.query()?.one_or_none()
    }

    pub fn one(&self) -> Result<T> {
        self.query()?.one()
    }

    /// Counts relationship edges without materializing target objects.
    pub fn count(&self) -> Result<usize> {
        let binding = self.binding.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidPlan,
                "Many<T> is not bound to a database snapshot",
            )
        })?;
        let query = Query::scan(binding.relation).filter_eq(
            0,
            Value::HistoricalEntityRef(crate::EntityRef {
                entity_type: binding.source_type,
                id: binding.source_id,
            }),
            binding.source_equivalence,
        );
        Ok(binding.context.execute(&query)?.rows().len())
    }
}

/// A snapshot-bound subset of one concrete `Many<T>` relationship.
///
/// Selection evaluation is identity-first: edge mutation methods project only target identities
/// and never construct target Rust objects. Object materialization remains explicit through
/// [`ManySelection::all`] or [`ManySelection::load`].
pub struct ManySelection<T: Object> {
    binding: ManyBinding,
    query: ObjectQuery<T>,
}

impl<T: Object> std::fmt::Debug for ManySelection<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManySelection")
            .field("relation", &self.binding.relation)
            .field("source_id", &self.binding.source_id)
            .field("node_id", &self.query.node_id())
            .finish_non_exhaustive()
    }
}

impl<T: Object> ManySelection<T> {
    #[must_use]
    pub fn query(&self) -> ObjectQuery<T> {
        ObjectQuery {
            context: self.query.context.clone(),
            relation: self.query.relation.clone(),
            inner: self.query.inner.clone(),
        }
    }

    /// Returns selected identities without decoding/materializing target objects.
    pub fn ids(&self) -> Result<Vec<crate::Id<T>>> {
        self.query.identity_ids()
    }

    pub fn count(&self) -> Result<usize> {
        Ok(self.ids()?.len())
    }

    pub fn load(&self) -> Result<Vec<T>> {
        self.query.all()
    }

    pub fn all(&self) -> Result<Vec<T>> {
        self.load()
    }

    pub fn first_or_none(&self) -> Result<Option<T>> {
        self.query.first_or_none()
    }

    pub fn one_or_none(&self) -> Result<Option<T>> {
        self.query.one_or_none()
    }

    pub fn one(&self) -> Result<T> {
        self.query.one()
    }

    /// Removes all selected relationship edges while leaving target objects alive.
    pub fn detach_all(&self) -> Result<Plan> {
        plan_detach_ids::<T>(&self.binding, self.ids()?)
    }

    /// Atomically moves all selected edges to another owner of the same relationship.
    pub fn move_to(&self, destination: &Many<T>) -> Result<Plan> {
        let dest = many_binding(destination)?;
        plan_move_ids::<T>(&self.binding, dest, self.ids()?)
    }

    /// Deletes selected target objects. CFMD reads exact stored rows but does not materialize
    /// Rust target objects; lifecycle normalization removes every now-dangling relationship edge.
    pub fn delete_all(&self) -> Result<Plan> {
        let rows = self
            .binding
            .context
            .execute(&self.query.inner.clone().raw())?
            .rows()
            .to_vec();
        let mut plan = self.binding.context.plan()?;
        if let Some(contract) = object_contract::<T>()? {
            plan.register_object_contract(contract);
        }
        register_object_relationship_contracts::<T>(&mut plan)?;
        for row in rows {
            plan.remove(T::relation_id(), row);
        }
        Ok(plan)
    }
}

/// Ownership-preserving counterpart of [`ManySelection`].
pub struct OwnedManySelection<T: Object> {
    inner: ManySelection<T>,
    orphan_policy: crate::plan::OrphanPolicy,
}

impl<T: Object> std::fmt::Debug for OwnedManySelection<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedManySelection")
            .field("selection", &self.inner)
            .field("orphan_policy", &self.orphan_policy)
            .finish()
    }
}

impl<T: Object> OwnedManySelection<T> {
    #[must_use]
    pub fn query(&self) -> ObjectQuery<T> {
        self.inner.query()
    }
    pub fn ids(&self) -> Result<Vec<crate::Id<T>>> {
        self.inner.ids()
    }
    pub fn count(&self) -> Result<usize> {
        self.inner.count()
    }
    pub fn load(&self) -> Result<Vec<T>> {
        self.inner.load()
    }
    pub fn all(&self) -> Result<Vec<T>> {
        self.inner.all()
    }
    pub fn first_or_none(&self) -> Result<Option<T>> {
        self.inner.first_or_none()
    }
    pub fn one_or_none(&self) -> Result<Option<T>> {
        self.inner.one_or_none()
    }
    pub fn one(&self) -> Result<T> {
        self.inner.one()
    }
    pub fn detach_all(&self) -> Result<Plan> {
        let mut plan = self.inner.detach_all()?;
        __register_owned_contract::<T>(
            &many_from_binding::<T>(&self.inner.binding),
            &mut plan,
            self.orphan_policy,
        )?;
        Ok(plan)
    }
    pub fn move_to(&self, destination: &OwnedMany<T>) -> Result<Plan> {
        let mut plan = plan_move_ids::<T>(
            &self.inner.binding,
            many_binding(&destination.inner)?,
            self.inner.ids()?,
        )?;
        __register_owned_contract::<T>(&destination.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }
    pub fn delete_all(&self) -> Result<Plan> {
        let mut plan = self.inner.delete_all()?;
        __register_owned_contract::<T>(
            &many_from_binding::<T>(&self.inner.binding),
            &mut plan,
            self.orphan_policy,
        )?;
        Ok(plan)
    }
}

fn many_from_binding<T>(binding: &ManyBinding) -> Many<T> {
    Many {
        binding: Some(binding.clone()),
        pending: None,
    }
}

/// Exclusive object-first zero-to-many ownership relation.
///
/// Unlike [`Many<T>`], a target may have at most one owner in this relationship. Moving ownership
/// is an atomic edge rewrite; the target object itself is not rewritten. Orphan handling is a
/// schema policy and defaults to [`crate::OrphanPolicy::Keep`].
pub struct OwnedMany<T> {
    inner: Many<T>,
    orphan_policy: crate::plan::OrphanPolicy,
}

impl<T> std::fmt::Debug for OwnedMany<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedMany")
            .field("bound", &self.inner.is_bound())
            .field("orphan_policy", &self.orphan_policy)
            .finish_non_exhaustive()
    }
}

impl<T: Clone> Clone for OwnedMany<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            orphan_policy: self.orphan_policy,
        }
    }
}

impl<T> Default for OwnedMany<T> {
    fn default() -> Self {
        Self::__new()
    }
}

impl<T: PartialEq> PartialEq for OwnedMany<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner && self.orphan_policy == other.orphan_policy
    }
}

impl<T: Eq> Eq for OwnedMany<T> {}

impl<T> OwnedMany<T> {
    #[doc(hidden)]
    #[must_use]
    pub const fn __new() -> Self {
        Self {
            inner: Many::__new(),
            orphan_policy: crate::plan::OrphanPolicy::Keep,
        }
    }

    #[must_use]
    pub fn new(values: impl IntoIterator<Item = T>) -> Self {
        Self {
            inner: Many::new(values),
            orphan_policy: crate::plan::OrphanPolicy::Keep,
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.inner.is_bound()
    }

    #[doc(hidden)]
    pub fn __into_pending(self) -> Result<Vec<T>> {
        self.inner.__into_pending()
    }

    #[doc(hidden)]
    pub fn __bind(
        &mut self,
        context: ReadContext,
        relation: RelationId,
        source_id: u128,
        source_type: crate::TypeId,
        source_equivalence: crate::EquivalenceId,
        target_equivalence: crate::EquivalenceId,
    ) {
        self.inner.__bind(
            context,
            relation,
            source_id,
            source_type,
            source_equivalence,
            target_equivalence,
        );
    }

    #[doc(hidden)]
    pub fn __set_orphan_policy(&mut self, orphan_policy: crate::plan::OrphanPolicy) {
        self.orphan_policy = orphan_policy;
    }

    #[doc(hidden)]
    pub fn __preserves_binding(
        &self,
        context: &ReadContext,
        relation: RelationId,
        source_id: u128,
    ) -> Result<bool> {
        self.inner.__preserves_binding(context, relation, source_id)
    }
}

impl<T: Object> OwnedMany<T> {
    pub fn query(&self) -> Result<ObjectQuery<T>> {
        self.inner.query()
    }
    pub fn where_<F, P>(&self, predicate: F) -> Result<OwnedManySelection<T>>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: crate::ObjectPredicate<T>,
    {
        Ok(OwnedManySelection {
            inner: self.inner.where_(predicate)?,
            orphan_policy: self.orphan_policy,
        })
    }
    pub fn load(&self) -> Result<Vec<T>> {
        self.inner.load()
    }
    pub fn all(&self) -> Result<Vec<T>> {
        self.inner.all()
    }
    pub fn first_or_none(&self) -> Result<Option<T>> {
        self.inner.first_or_none()
    }
    pub fn one_or_none(&self) -> Result<Option<T>> {
        self.inner.one_or_none()
    }
    pub fn one(&self) -> Result<T> {
        self.inner.one()
    }
    pub fn count(&self) -> Result<usize> {
        self.inner.count()
    }

    /// Proposes attaching an existing target. Fails during candidate construction/commit if that
    /// target already has a different owner in this ownership relation.
    pub fn attach(&self, target: crate::Id<T>) -> Result<Plan> {
        let mut plan = self.inner.__attach(target)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }

    pub fn detach(&self, target: crate::Id<T>) -> Result<Plan> {
        let mut plan = self.inner.__detach(target)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }

    /// Atomically transfers one target from this owner to `destination` without rewriting the
    /// target object row.
    pub fn move_to(&self, target: crate::Id<T>, destination: &Self) -> Result<Plan> {
        let mut plan = self.inner.__move_to(target, &destination.inner)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }

    /// Atomically transfers every edge owned by this object. Only edge rows are read; target
    /// objects are not materialized into Rust.
    pub fn move_all_to(&self, destination: &Self) -> Result<Plan> {
        let mut plan = self.inner.__move_all_to(&destination.inner)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }
    pub fn detach_all(&self) -> Result<Plan> {
        let mut plan = self.inner.detach_all()?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }
    pub fn detach_ids(&self, ids: impl IntoIterator<Item = crate::Id<T>>) -> Result<Plan> {
        let mut plan = self.inner.detach_ids(ids)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }
    pub fn move_ids_to(
        &self,
        ids: impl IntoIterator<Item = crate::Id<T>>,
        destination: &Self,
    ) -> Result<Plan> {
        let mut plan = self.inner.move_ids_to(ids, &destination.inner)?;
        __register_owned_contract::<T>(&self.inner, &mut plan, self.orphan_policy)?;
        Ok(plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFieldRole {
    Value,
    Identity(crate::TypeId),
    Reference {
        target_type: crate::TypeId,
        target_relation: RelationId,
        target_identity_column: usize,
    },
    OptionalReference {
        target_type: crate::TypeId,
        target_relation: RelationId,
        target_identity_column: usize,
    },
}

impl ObjectFieldRole {
    #[must_use]
    pub const fn relationship_cardinality(self) -> Option<ObjectRelationshipCardinality> {
        match self {
            Self::Reference { .. } => Some(ObjectRelationshipCardinality::RequiredOne),
            Self::OptionalReference { .. } => Some(ObjectRelationshipCardinality::OptionalOne),
            Self::Value | Self::Identity(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct ObjectManyFieldSchema {
    name: &'static str,
    target_type: crate::TypeId,
    target_relation: RelationId,
    relation: RelationId,
    source_equivalence: crate::EquivalenceId,
    target_equivalence: crate::EquivalenceId,
    source_live_equivalence: crate::EquivalenceId,
    target_live_equivalence: crate::EquivalenceId,
    // Retained only as a pre-1.0 compatibility hint while old declarations migrate. The edge
    // relation no longer depends on a target-side Ref backlink.
    via_field: Option<&'static str>,
    ownership: Option<crate::plan::OrphanPolicy>,
    register_owned: Option<fn(&ObjectManyFieldSchema, &mut Plan) -> Result<()>>,
    validate: fn() -> Result<()>,
}

impl std::fmt::Debug for ObjectManyFieldSchema {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObjectManyFieldSchema")
            .field("name", &self.name)
            .field("target_type", &self.target_type)
            .field("target_relation", &self.target_relation)
            .field("relation", &self.relation)
            .field("via_field", &self.via_field)
            .field("ownership", &self.ownership)
            .finish_non_exhaustive()
    }
}

impl ObjectManyFieldSchema {
    #[must_use]
    pub fn of<S: Object, T: Object>(name: &'static str, via_field: &'static str) -> Self {
        Self::new::<S, T>(name, Some(via_field))
    }

    #[must_use]
    pub fn inferred<S: Object, T: Object>(name: &'static str) -> Self {
        Self::new::<S, T>(name, None)
    }

    fn new<S: Object, T: Object>(name: &'static str, via_field: Option<&'static str>) -> Self {
        Self {
            name,
            target_type: T::type_id(),
            target_relation: T::relation_id(),
            relation: __many_relation_id::<S>(name),
            source_equivalence: __identity_equivalence_id::<S>()
                .unwrap_or_else(|_| crate::EquivalenceId::new(0)),
            target_equivalence: __identity_equivalence_id::<T>()
                .unwrap_or_else(|_| crate::EquivalenceId::new(0)),
            source_live_equivalence: crate::EquivalenceId::new(__semantic_id(
                "cfmd.object.many-live-equivalence.v1",
                S::KEY,
                name,
            )),
            target_live_equivalence: crate::EquivalenceId::new(__semantic_id(
                "cfmd.object.many-live-target-equivalence.v1",
                S::KEY,
                name,
            )),
            via_field,
            ownership: None,
            register_owned: None,
            validate: validate_many_field::<S, T>,
        }
    }

    #[must_use]
    pub fn owned<S: Object, T: Object>(
        name: &'static str,
        orphan_policy: crate::plan::OrphanPolicy,
    ) -> Self {
        let mut value = Self::new::<S, T>(name, None);
        value.ownership = Some(orphan_policy);
        value.register_owned = Some(register_owned_schema::<S, T>);
        value
    }

    #[must_use]
    pub const fn orphan_policy(&self) -> Option<crate::plan::OrphanPolicy> {
        self.ownership
    }

    #[must_use]
    pub const fn is_owned(&self) -> bool {
        self.ownership.is_some()
    }

    pub(crate) fn register_contract(&self, plan: &mut Plan) -> Result<()> {
        if let Some(register) = self.register_owned {
            register(self, plan)?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn target_type(&self) -> crate::TypeId {
        self.target_type
    }

    #[must_use]
    pub const fn target_relation(&self) -> RelationId {
        self.target_relation
    }

    /// Internal relation created by the object-schema compiler for this object relationship.
    #[must_use]
    pub const fn relation(&self) -> RelationId {
        self.relation
    }

    #[must_use]
    pub const fn via_field(&self) -> Option<&'static str> {
        self.via_field
    }

    #[must_use]
    pub const fn cardinality(&self) -> ObjectRelationshipCardinality {
        ObjectRelationshipCardinality::Many
    }

    pub(crate) fn source_equivalence(&self) -> crate::EquivalenceId {
        self.source_equivalence
    }

    pub(crate) fn target_equivalence(&self) -> crate::EquivalenceId {
        self.target_equivalence
    }

    pub(crate) fn source_live_equivalence(&self) -> crate::EquivalenceId {
        self.source_live_equivalence
    }

    pub(crate) fn target_live_equivalence(&self) -> crate::EquivalenceId {
        self.target_live_equivalence
    }

    pub(crate) fn validate(&self) -> Result<()> {
        (self.validate)()
    }
}

#[must_use]
#[doc(hidden)]
pub fn __many_relation_id<S: Object>(name: &str) -> RelationId {
    RelationId::new(__semantic_id("cfmd.object.many-relation.v1", S::KEY, name))
}

#[doc(hidden)]
pub fn __identity_equivalence_id<E: Object>() -> Result<crate::EquivalenceId> {
    let field = E::fields()
        .into_iter()
        .find(|field| field.role() == ObjectFieldRole::Identity(E::type_id()))
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("object relationship endpoint {} has no identity", E::KEY),
            )
        })?;
    Ok(crate::EquivalenceId::new(__semantic_id(
        "cfmd.object.field-equivalence.v1",
        E::KEY,
        field.name(),
    )))
}

fn register_owned_schema<S: Object, T: Object>(
    schema: &ObjectManyFieldSchema,
    plan: &mut Plan,
) -> Result<()> {
    let policy = schema.ownership.ok_or_else(|| {
        Error::new(
            ErrorKind::Internal,
            "owned relationship registration lost its policy",
        )
    })?;
    __register_owned_many::<S, T>(plan, schema.name, policy)
}

pub(crate) fn register_object_relationship_contracts<E: Object>(plan: &mut Plan) -> Result<()> {
    for relationship in E::many_fields() {
        relationship.register_contract(plan)?;
    }
    Ok(())
}

fn validate_many_field<S: Object, T: Object>() -> Result<()> {
    __identity_equivalence_id::<S>()?;
    __identity_equivalence_id::<T>()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectFieldSchema {
    name: &'static str,
    ty: Type,
    equivalence: ObjectEquivalence,
    ordering: Option<PrimitiveOrdering>,
    role: ObjectFieldRole,
}

impl ObjectFieldSchema {
    #[must_use]
    pub fn of<V: ObjectValue>(name: &'static str) -> Self {
        Self {
            name,
            ty: V::object_type(),
            equivalence: V::equivalence(),
            ordering: V::ordering(),
            role: V::role(),
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn ty(&self) -> &Type {
        &self.ty
    }

    #[must_use]
    pub const fn equivalence(&self) -> ObjectEquivalence {
        self.equivalence
    }
    #[must_use]
    pub const fn ordering(&self) -> Option<PrimitiveOrdering> {
        self.ordering
    }
    #[must_use]
    pub const fn role(&self) -> ObjectFieldRole {
        self.role
    }
}

/// Compile-time mapping between one Rust domain object and the universal CFMD relation protocol.
///
/// The textual key is the durable product identity. Numeric semantic identifiers are derived from
/// it deterministically and never need to appear in application code.
pub trait Object: RowCodec + Sized + 'static {
    type Proxy: Clone;

    const KEY: &'static str;

    fn fields() -> Vec<ObjectFieldSchema>;

    #[must_use]
    fn many_fields() -> Vec<ObjectManyFieldSchema> {
        Vec::new()
    }

    #[doc(hidden)]
    fn __bind_relations(&mut self, _context: &ReadContext) -> Result<()> {
        Ok(())
    }

    #[doc(hidden)]
    fn __identity_raw(&self) -> Result<u128> {
        Err(Error::new(
            ErrorKind::InvalidSchema,
            format!(
                "object {} does not expose a generated identity accessor",
                Self::KEY
            ),
        ))
    }

    #[doc(hidden)]
    fn __append_relationships(&mut self, _context: &ReadContext, _plan: &mut Plan) -> Result<()> {
        Ok(())
    }

    #[doc(hidden)]
    fn __append_update_relationships(
        &mut self,
        _context: &ReadContext,
        _plan: &mut Plan,
    ) -> Result<()> {
        Ok(())
    }

    #[doc(hidden)]
    fn __append_insert(mut self, context: &ReadContext, plan: &mut Plan) -> Result<()> {
        self.__append_relationships(context, plan)?;
        __append_flat_insert(context, plan, self)
    }

    fn proxy(relation: Relation<Self>) -> Self::Proxy;

    #[must_use]
    fn type_id() -> crate::TypeId {
        crate::TypeId::new(__semantic_id("cfmd.object.type.v1", Self::KEY, "type"))
    }

    #[must_use]
    fn identity_column() -> Option<usize> {
        Self::fields()
            .iter()
            .position(|field| field.role() == ObjectFieldRole::Identity(Self::type_id()))
    }

    #[must_use]
    fn relation_id() -> RelationId {
        RelationId::new(__semantic_id(
            "cfmd.object.relation.v1",
            Self::KEY,
            "relation",
        ))
    }
}

pub(crate) fn register_object<E: Object>(mut builder: SchemaBuilder) -> SchemaBuilder {
    let fields = E::fields();
    let (registered, types, equivalences) = register_object_field_semantics::<E>(builder, &fields);
    builder = registered;
    builder = builder
        .relation(crate::RelationSchema::set(
            E::relation_id(),
            types,
            equivalences,
        ))
        .__entity_type(E::type_id());
    for field in &fields {
        if matches!(
            field.role(),
            ObjectFieldRole::Reference { .. } | ObjectFieldRole::OptionalReference { .. }
        ) {
            let kernel_type = match field.role() {
                ObjectFieldRole::Reference { target_type, .. } => {
                    Type::Scalar(crate::ScalarType::LiveEntityRef(target_type))
                }
                ObjectFieldRole::OptionalReference { target_type, .. } => Type::Option(Box::new(
                    Type::Scalar(crate::ScalarType::LiveEntityRef(target_type)),
                )),
                _ => unreachable!("only reference fields are mirrored into kernel entity fields"),
            };
            builder = builder.__entity_field(
                crate::FieldId::new(__semantic_id(
                    "cfmd.object.kernel-field.v1",
                    E::KEY,
                    field.name(),
                )),
                E::type_id(),
                kernel_type,
            );
        }
    }
    for many in E::many_fields() {
        if let Err(error) = many.validate() {
            builder = builder.__invalid_schema(error.to_string());
        }
        builder = builder
            .equivalence(
                many.source_live_equivalence(),
                PrimitiveEquivalence::LiveEntityIdExact(E::type_id()),
            )
            .equivalence(
                many.target_live_equivalence(),
                PrimitiveEquivalence::LiveEntityIdExact(many.target_type()),
            )
            .relation(crate::RelationSchema::set(
                many.relation(),
                vec![
                    Type::Scalar(crate::ScalarType::HistoricalEntityRef(E::type_id())),
                    Type::Scalar(crate::ScalarType::HistoricalEntityRef(many.target_type())),
                    Type::Scalar(crate::ScalarType::LiveEntityRef(E::type_id())),
                    Type::Scalar(crate::ScalarType::LiveEntityRef(many.target_type())),
                ],
                vec![
                    many.source_equivalence(),
                    many.target_equivalence(),
                    many.source_live_equivalence(),
                    many.target_live_equivalence(),
                ],
            ));
        builder = builder.__require_relation(
            many.target_relation(),
            format!(
                "object relationship {}.{} requires target object relation {} to be registered",
                E::KEY,
                many.name(),
                many.target_relation().raw()
            ),
        );
    }
    builder
}

fn register_object_field_semantics<E: Object>(
    mut builder: SchemaBuilder,
    fields: &[ObjectFieldSchema],
) -> (SchemaBuilder, Vec<Type>, Vec<crate::EquivalenceId>) {
    let mut types = Vec::with_capacity(fields.len());
    let mut equivalences = Vec::with_capacity(fields.len());
    for field in fields {
        let id = crate::EquivalenceId::new(__semantic_id(
            "cfmd.object.field-equivalence.v1",
            E::KEY,
            field.name(),
        ));
        builder = match field.equivalence() {
            ObjectEquivalence::Primitive(module) => builder.equivalence(id, module),
            ObjectEquivalence::OptionOf(inner_module) => {
                let inner = crate::EquivalenceId::new(__semantic_id(
                    "cfmd.object.field-equivalence-inner.v1",
                    E::KEY,
                    field.name(),
                ));
                builder
                    .equivalence(inner, inner_module)
                    .structural_equivalence(id, crate::StructuralEquivalence::Option { inner })
            }
        };
        if let Some(ordering) = field.ordering() {
            builder = builder.ordering(
                crate::OrderingId::new(__semantic_id(
                    "cfmd.object.field-ordering.v1",
                    E::KEY,
                    field.name(),
                )),
                ordering,
            );
        }
        equivalences.push(id);
        types.push(field.ty().clone());
    }
    (builder, types, equivalences)
}

#[doc(hidden)]
pub fn __append_flat_insert<E: Object>(
    context: &ReadContext,
    plan: &mut Plan,
    value: E,
) -> Result<()> {
    let relation = context.relation::<E>(E::relation_id())?;
    if let Some(contract) = object_contract::<E>()? {
        plan.register_object_contract(contract);
    }
    register_object_relationship_contracts::<E>(plan)?;
    plan.insert_typed(&relation, value)?;
    Ok(())
}

#[doc(hidden)]
pub fn __append_many_edge<S: Object, T: Object>(
    plan: &mut Plan,
    name: &str,
    source_id: u128,
    target_id: u128,
) -> Result<()> {
    __identity_equivalence_id::<S>()?;
    __identity_equivalence_id::<T>()?;
    plan.insert(
        __many_relation_id::<S>(name),
        vec![
            Value::HistoricalEntityRef(crate::EntityRef {
                entity_type: S::type_id(),
                id: source_id,
            }),
            Value::HistoricalEntityRef(crate::EntityRef {
                entity_type: T::type_id(),
                id: target_id,
            }),
            Value::LiveEntityRef(crate::EntityRef {
                entity_type: S::type_id(),
                id: crate::runtime::lifecycle_entity_id(S::type_id(), source_id).raw(),
            }),
            Value::LiveEntityRef(crate::EntityRef {
                entity_type: T::type_id(),
                id: crate::runtime::lifecycle_entity_id(T::type_id(), target_id).raw(),
            }),
        ],
    );
    Ok(())
}

#[doc(hidden)]
pub fn __append_remove_many_edges<E: Object>(
    context: &ReadContext,
    plan: &mut Plan,
    name: &str,
    source_id: u128,
) -> Result<()> {
    let many = E::many_fields()
        .into_iter()
        .find(|field| field.name() == name)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("object {} has no Many field {name}", E::KEY),
            )
        })?;
    let query = Query::scan(many.relation()).filter_eq(
        0,
        Value::HistoricalEntityRef(crate::EntityRef {
            entity_type: E::type_id(),
            id: source_id,
        }),
        many.source_equivalence(),
    );
    for row in context.execute(&query)?.rows() {
        plan.remove(many.relation(), row.clone());
    }
    Ok(())
}

fn many_binding<T>(many: &Many<T>) -> Result<&ManyBinding> {
    many.binding.as_ref().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidPlan,
            "relationship value is not bound to a database snapshot",
        )
    })
}

fn edge_row<T: Object>(binding: &ManyBinding, target: crate::Id<T>) -> crate::Row {
    vec![
        Value::HistoricalEntityRef(crate::EntityRef {
            entity_type: binding.source_type,
            id: binding.source_id,
        }),
        Value::HistoricalEntityRef(crate::EntityRef {
            entity_type: T::type_id(),
            id: target.raw(),
        }),
        Value::LiveEntityRef(crate::EntityRef {
            entity_type: binding.source_type,
            id: crate::runtime::lifecycle_entity_id(binding.source_type, binding.source_id).raw(),
        }),
        Value::LiveEntityRef(crate::EntityRef {
            entity_type: T::type_id(),
            id: crate::runtime::lifecycle_entity_id(T::type_id(), target.raw()).raw(),
        }),
    ]
}

fn same_relationship(source: &ManyBinding, destination: &ManyBinding) -> Result<()> {
    if source.relation != destination.relation
        || !source.context.same_snapshot(&destination.context)
    {
        return Err(Error::new(
            ErrorKind::InvalidPlan,
            "relationship mutation requires the same relationship and snapshot",
        ));
    }
    Ok(())
}

fn bound_target_ids<T: Object>(binding: &ManyBinding) -> Result<std::collections::BTreeSet<u128>> {
    let query = Query::scan(binding.relation).filter_eq(
        0,
        Value::HistoricalEntityRef(crate::EntityRef {
            entity_type: binding.source_type,
            id: binding.source_id,
        }),
        binding.source_equivalence,
    );
    binding
        .context
        .execute(&query)?
        .rows()
        .iter()
        .map(|row| match row.get(1) {
            Some(Value::HistoricalEntityRef(reference))
                if reference.entity_type == T::type_id() =>
            {
                Ok(reference.id)
            }
            _ => Err(Error::new(
                ErrorKind::InvariantViolation,
                "relationship edge target has invalid shape",
            )),
        })
        .collect()
}

fn plan_detach_ids<T: Object>(
    binding: &ManyBinding,
    ids: impl IntoIterator<Item = crate::Id<T>>,
) -> Result<Plan> {
    let ids = ids.into_iter().collect::<Vec<_>>();
    let existing = bound_target_ids::<T>(binding)?;
    if let Some(missing) = ids.iter().find(|id| !existing.contains(&id.raw())) {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "relationship target {} is not attached to this owner",
                missing.raw()
            ),
        ));
    }
    let mut plan = binding.context.plan()?;
    for id in ids {
        plan.remove(binding.relation, edge_row(binding, id));
    }
    Ok(plan)
}

fn plan_move_ids<T: Object>(
    source: &ManyBinding,
    destination: &ManyBinding,
    ids: impl IntoIterator<Item = crate::Id<T>>,
) -> Result<Plan> {
    same_relationship(source, destination)?;
    let ids = ids.into_iter().collect::<Vec<_>>();
    let existing = bound_target_ids::<T>(source)?;
    if let Some(missing) = ids.iter().find(|id| !existing.contains(&id.raw())) {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "relationship target {} is not attached to the source owner",
                missing.raw()
            ),
        ));
    }
    let mut plan = source.context.plan()?;
    for id in ids {
        plan.remove(source.relation, edge_row(source, id));
        plan.insert(destination.relation, edge_row(destination, id));
    }
    Ok(plan)
}

impl<T: Object> Many<T> {
    #[doc(hidden)]
    pub fn __attach(&self, target: crate::Id<T>) -> Result<Plan> {
        let binding = many_binding(self)?;
        let mut plan = binding.context.plan()?;
        plan.insert(binding.relation, edge_row(binding, target));
        Ok(plan)
    }

    #[doc(hidden)]
    pub fn __detach(&self, target: crate::Id<T>) -> Result<Plan> {
        let binding = many_binding(self)?;
        let mut plan = binding.context.plan()?;
        let row = edge_row(binding, target);
        let exists = binding
            .context
            .execute(&Query::scan(binding.relation))?
            .rows()
            .contains(&row);
        if !exists {
            return Err(Error::new(
                ErrorKind::NotFound,
                "relationship edge is not present",
            ));
        }
        plan.remove(binding.relation, row);
        Ok(plan)
    }

    #[doc(hidden)]
    pub fn __move_to(&self, target: crate::Id<T>, destination: &Self) -> Result<Plan> {
        let source = many_binding(self)?;
        let dest = many_binding(destination)?;
        if source.relation != dest.relation || !source.context.same_snapshot(&dest.context) {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "relationship move requires the same relationship and snapshot",
            ));
        }
        let mut plan = self.__detach(target)?;
        plan.insert(dest.relation, edge_row(dest, target));
        Ok(plan)
    }

    #[doc(hidden)]
    pub fn __move_all_to(&self, destination: &Self) -> Result<Plan> {
        let source = many_binding(self)?;
        let dest = many_binding(destination)?;
        if source.relation != dest.relation || !source.context.same_snapshot(&dest.context) {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "relationship move requires the same relationship and snapshot",
            ));
        }
        let query = Query::scan(source.relation).filter_eq(
            0,
            Value::HistoricalEntityRef(crate::EntityRef {
                entity_type: source.source_type,
                id: source.source_id,
            }),
            source.source_equivalence,
        );
        let rows = source.context.execute(&query)?.rows().to_vec();
        let mut plan = source.context.plan()?;
        for row in rows {
            let target_id = match row.get(1) {
                Some(Value::HistoricalEntityRef(reference))
                    if reference.entity_type == T::type_id() =>
                {
                    reference.id
                }
                _ => {
                    return Err(Error::new(
                        ErrorKind::InvariantViolation,
                        "relationship edge target has invalid shape",
                    ));
                }
            };
            plan.remove(source.relation, row);
            plan.insert(
                dest.relation,
                edge_row(dest, crate::Id::<T>::new(target_id)),
            );
        }
        Ok(plan)
    }

    pub fn attach(&self, target: crate::Id<T>) -> Result<Plan> {
        self.__attach(target)
    }
    pub fn detach(&self, target: crate::Id<T>) -> Result<Plan> {
        self.__detach(target)
    }
    pub fn move_to(&self, target: crate::Id<T>, destination: &Self) -> Result<Plan> {
        self.__move_to(target, destination)
    }
    pub fn move_all_to(&self, destination: &Self) -> Result<Plan> {
        self.__move_all_to(destination)
    }
    pub fn detach_all(&self) -> Result<Plan> {
        let binding = many_binding(self)?;
        let ids = bound_target_ids::<T>(binding)?
            .into_iter()
            .map(crate::Id::<T>::new);
        plan_detach_ids::<T>(binding, ids)
    }
    pub fn detach_ids(&self, ids: impl IntoIterator<Item = crate::Id<T>>) -> Result<Plan> {
        plan_detach_ids::<T>(many_binding(self)?, ids)
    }
    pub fn move_ids_to(
        &self,
        ids: impl IntoIterator<Item = crate::Id<T>>,
        destination: &Self,
    ) -> Result<Plan> {
        plan_move_ids::<T>(many_binding(self)?, many_binding(destination)?, ids)
    }
}

fn __register_owned_contract<T: Object>(
    many: &Many<T>,
    plan: &mut Plan,
    orphan_policy: crate::plan::OrphanPolicy,
) -> Result<()> {
    let binding = many_binding(many)?;
    let identity = T::identity_column().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("owned target {} has no identity", T::KEY),
        )
    })?;
    if let Some(contract) = object_contract::<T>()? {
        plan.register_object_contract(contract);
    }
    plan.register_owned_relation(crate::plan::OwnedRelationContract {
        relation: binding.relation,
        target_relation: T::relation_id(),
        target_identity_column: identity,
        orphan_policy,
    });
    Ok(())
}

#[doc(hidden)]
pub fn __register_owned_many<S: Object, T: Object>(
    plan: &mut Plan,
    name: &str,
    orphan_policy: crate::plan::OrphanPolicy,
) -> Result<()> {
    let identity = T::identity_column().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("owned target {} has no identity", T::KEY),
        )
    })?;
    if let Some(contract) = object_contract::<T>()? {
        plan.register_object_contract(contract);
    }
    plan.register_owned_relation(crate::plan::OwnedRelationContract {
        relation: __many_relation_id::<S>(name),
        target_relation: T::relation_id(),
        target_identity_column: identity,
        orphan_policy,
    });
    Ok(())
}

pub(crate) fn object_contract<E: Object>() -> Result<Option<crate::plan::ObjectContract>> {
    let fields = E::fields();
    let identity_column = E::identity_column();
    let has_references = fields.iter().any(|field| {
        matches!(
            field.role(),
            ObjectFieldRole::Reference { .. } | ObjectFieldRole::OptionalReference { .. }
        )
    });
    let Some(identity_column) = identity_column else {
        if has_references {
            return Err(Error::new(
                ErrorKind::InvalidSchema,
                format!(
                    "object {} declares references but has no identity field",
                    E::KEY
                ),
            ));
        }
        return Ok(None);
    };
    let mut references = Vec::new();
    for (column, field) in fields.iter().enumerate() {
        match field.role() {
            ObjectFieldRole::Reference {
                target_type,
                target_relation,
                target_identity_column,
            } => references.push(crate::plan::ReferenceContract {
                column,
                field: crate::FieldId::new(__semantic_id(
                    "cfmd.object.kernel-field.v1",
                    E::KEY,
                    field.name(),
                )),
                target_type,
                target_relation,
                target_identity_column,
                optional: false,
            }),
            ObjectFieldRole::OptionalReference {
                target_type,
                target_relation,
                target_identity_column,
            } => references.push(crate::plan::ReferenceContract {
                column,
                field: crate::FieldId::new(__semantic_id(
                    "cfmd.object.kernel-field.v1",
                    E::KEY,
                    field.name(),
                )),
                target_type,
                target_relation,
                target_identity_column,
                optional: true,
            }),
            _ => {}
        }
    }
    Ok(Some(crate::plan::ObjectContract {
        relation: E::relation_id(),
        entity_type: E::type_id(),
        identity_column,
        identity_type: E::type_id(),
        references,
    }))
}

pub(crate) fn symbolic_relation<E: Object>() -> Relation<E> {
    let fields = E::fields();
    let columns = fields.iter().map(|field| field.ty().clone()).collect();
    let equivalences = fields
        .iter()
        .map(|field| {
            crate::EquivalenceId::new(__semantic_id(
                "cfmd.object.field-equivalence.v1",
                E::KEY,
                field.name(),
            ))
        })
        .collect();
    Relation::from_parts(E::relation_id(), columns, equivalences)
}

/// Symbolic object proxy used only while constructing a query expression.
#[derive(Debug, Clone)]
pub struct ObjectProxy<E: Object> {
    relation: Relation<E>,
}

impl<E: Object> ObjectProxy<E> {
    #[doc(hidden)]
    #[must_use]
    pub fn __new(relation: Relation<E>) -> Self {
        Self { relation }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __field<V: ObjectValue>(&self, name: &str) -> Field<E, V> {
        let column = E::fields()
            .iter()
            .position(|field| field.name() == name)
            .expect("generated CFMD object field must exist in its descriptor");
        let equivalence = self
            .relation
            .equivalence_at(column)
            .expect("validated CFMD object field has equivalence semantics");
        let ordering = E::fields()[column].ordering().map(|_| {
            crate::OrderingId::new(__semantic_id("cfmd.object.field-ordering.v1", E::KEY, name))
        });
        Field::__from_semantics(self.relation.id(), column, equivalence, ordering)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __ref<T: Object>(&self, name: &str) -> crate::RefField<E, T> {
        let column = E::fields()
            .iter()
            .position(|field| field.name() == name)
            .expect("generated CFMD object reference must exist in its descriptor");
        let equivalence = self
            .relation
            .equivalence_at(column)
            .expect("validated object reference has equivalence semantics");
        crate::RefField::new(&self.relation, column, equivalence)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __optional_ref<T: Object>(&self, name: &str) -> crate::OptionalRefField<E, T> {
        let column = E::fields()
            .iter()
            .position(|field| field.name() == name)
            .expect("generated CFMD optional reference must exist in its descriptor");
        let equivalence = self
            .relation
            .equivalence_at(column)
            .expect("validated object optional reference has equivalence semantics");
        crate::OptionalRefField::new(&self.relation, column, equivalence)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __many<T: Object>(&self, name: &str) -> crate::ManyField<E, T> {
        crate::ManyField::new(&self.relation, name)
    }
}

#[derive(Debug, Clone)]
pub struct ObjectSet<E: Object> {
    context: ReadContext,
    relation: Relation<E>,
}

impl<E: Object> ObjectSet<E> {
    pub(crate) fn new(context: ReadContext, relation: Relation<E>) -> Result<Self> {
        let fields = E::fields();
        if relation.width() != fields.len() || !E::accepts(relation.column_types()) {
            return Err(Error::new(
                ErrorKind::TypeMismatch,
                format!(
                    "stored relation for object {} does not match its Rust object shape",
                    E::KEY
                ),
            ));
        }
        for (index, field) in fields.iter().enumerate() {
            let expected = crate::EquivalenceId::new(__semantic_id(
                "cfmd.object.field-equivalence.v1",
                E::KEY,
                field.name(),
            ));
            if relation.equivalence_at(index) != Some(expected) {
                return Err(Error::new(
                    ErrorKind::InvalidSchema,
                    format!(
                        "stored relation for object {} has incompatible semantics for field {}",
                        E::KEY,
                        field.name()
                    ),
                ));
            }
        }
        Ok(Self { context, relation })
    }

    #[must_use]
    pub const fn relation(&self) -> &Relation<E> {
        &self.relation
    }

    #[track_caller]
    #[must_use]
    pub fn query(&self) -> ObjectQuery<E> {
        ObjectQuery {
            context: self.context.clone(),
            relation: self.relation.clone(),
            inner: self.relation.query(),
        }
    }

    #[track_caller]
    #[must_use]
    pub fn where_<F, P>(&self, predicate: F) -> ObjectQuery<E>
    where
        F: FnOnce(&E::Proxy) -> P,
        P: crate::ObjectPredicate<E>,
    {
        self.query().where_(predicate)
    }

    pub fn all(&self) -> Result<Vec<E>> {
        self.query().all()
    }

    pub fn get(&self, id: crate::Id<E>) -> Result<Option<E>> {
        let column = E::identity_column().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("object {} has no identity field", E::KEY),
            )
        })?;
        let equivalence = self.relation.equivalence_at(column).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("object {} identity has no equivalence", E::KEY),
            )
        })?;
        let field = Field::<E, crate::Id<E>>::__from_parts(self.relation.id(), column, equivalence);
        let mut query = self.query();
        query.inner = query.inner.filter(field.eq(id));
        query.one_or_none()
    }

    pub fn require(&self, id: crate::Id<E>) -> Result<E> {
        self.get(id)?.ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("{} identity {} was not found", E::KEY, id.raw()),
            )
        })
    }

    /// Returns a proposed transition; it does not mutate the database.
    pub fn insert(&self, value: E) -> Result<Plan> {
        let mut plan = self.context.plan()?;
        E::__append_insert(value, &self.context, &mut plan)?;
        Ok(plan)
    }

    /// Returns a proposed transition; it does not mutate the database.
    pub fn remove(&self, value: E) -> Result<Plan> {
        let mut plan = self.context.plan()?;
        if let Some(contract) = crate::object::object_contract::<E>()? {
            plan.register_object_contract(contract);
        }
        crate::object::register_object_relationship_contracts::<E>(&mut plan)?;
        plan.remove_typed(&self.relation, value)?;
        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct ObjectQuery<E: Object> {
    pub(crate) context: ReadContext,
    relation: Relation<E>,
    pub(crate) inner: RelationQuery<E>,
}

impl<E: Object> ObjectQuery<E> {
    pub fn watch(&self) -> Result<crate::ObjectWatch<E>> {
        crate::ObjectWatch::new(self)
    }

    #[track_caller]
    #[must_use]
    pub fn where_<F, P>(mut self, predicate: F) -> Self
    where
        F: FnOnce(&E::Proxy) -> P,
        P: crate::ObjectPredicate<E>,
    {
        let proxy = E::proxy(self.relation.clone());
        self.inner = self.inner.filter(predicate(&proxy));
        self
    }

    #[track_caller]
    #[must_use]
    pub fn select<F, P>(self, projection: F) -> ObjectProjectionQuery<E, P>
    where
        F: FnOnce(&E::Proxy) -> P,
        P: Projection<E>,
    {
        let proxy = E::proxy(self.relation.clone());
        ObjectProjectionQuery {
            context: self.context,
            inner: self.inner.select(projection(&proxy)),
        }
    }

    #[must_use]
    pub const fn node_id(&self) -> crate::QueryNodeId {
        self.inner.node_id()
    }

    #[must_use]
    pub fn source(&self) -> crate::QuerySource {
        self.inner.source()
    }

    pub(crate) fn identity_ids(&self) -> Result<Vec<crate::Id<E>>> {
        let identity = E::identity_column().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("object {} has no identity field", E::KEY),
            )
        })?;
        let query = self.inner.clone().raw().project(vec![identity]);
        self.context
            .execute(&query)?
            .rows()
            .iter()
            .map(|row| {
                let value = row.first().ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvariantViolation,
                        "identity projection returned an empty row",
                    )
                })?;
                crate::Id::<E>::from_value(value)
            })
            .collect()
    }

    pub fn all(&self) -> Result<Vec<E>> {
        let result = self.context.execute(&self.inner.clone().raw())?;
        result
            .rows()
            .iter()
            .map(|row| {
                let mut value = E::from_row(row)?;
                value.__bind_relations(&self.context)?;
                Ok(value)
            })
            .collect()
    }

    pub fn first_or_none(&self) -> Result<Option<E>> {
        let mut values = self.all()?;
        Ok(values.drain(..).next())
    }

    pub fn one_or_none(&self) -> Result<Option<E>> {
        let mut values = self.all()?;
        match values.len() {
            0 => Ok(None),
            1 => Ok(values.pop()),
            count => Err(Error::new(
                ErrorKind::Cardinality,
                format!("expected at most one object, query returned {count}"),
            )),
        }
    }

    pub fn one(&self) -> Result<E> {
        self.one_or_none()?.ok_or_else(|| {
            Error::new(
                ErrorKind::Cardinality,
                "expected exactly one object, query returned none",
            )
        })
    }

    /// Evaluates this snapshot-bound query and returns its exact deletion as a proposed `Plan`.
    pub fn delete(&self) -> Result<Plan> {
        let values = self.all()?;
        let mut plan = self.context.plan()?;
        if let Some(contract) = crate::object::object_contract::<E>()? {
            plan.register_object_contract(contract);
        }
        crate::object::register_object_relationship_contracts::<E>(&mut plan)?;
        for value in values {
            plan.remove_typed(&self.relation, value)?;
        }
        Ok(plan)
    }

    /// Evaluates this snapshot-bound query and returns exact remove+insert rewrites as a `Plan`.
    pub fn update<F>(&self, mut rewrite: F) -> Result<Plan>
    where
        F: FnMut(E) -> E,
        E: Clone,
    {
        let values = self.all()?;
        let mut plan = self.context.plan()?;
        if let Some(contract) = crate::object::object_contract::<E>()? {
            plan.register_object_contract(contract);
        }
        for old in values {
            let mut new = rewrite(old.clone());
            let old_row = self.relation.encode_row(old)?;
            let new_row = self.relation.encode_row(new.clone())?;
            let stored_changed = old_row != new_row;
            if stored_changed {
                plan.remove(self.relation.id(), old_row);
            }
            new.__append_update_relationships(&self.context, &mut plan)?;
            if stored_changed {
                plan.insert(self.relation.id(), new_row);
            }
        }
        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct ObjectProjectionQuery<E: Object, P: Projection<E>> {
    pub(crate) context: ReadContext,
    pub(crate) inner: TypedQuery<E, P>,
}

impl<E: Object, P: Projection<E>> ObjectProjectionQuery<E, P> {
    #[must_use]
    pub const fn node_id(&self) -> crate::QueryNodeId {
        self.inner.node_id()
    }

    #[must_use]
    pub fn source(&self) -> crate::QuerySource {
        self.inner.source()
    }

    pub fn watch(&self) -> Result<crate::ProjectionWatch<E, P>> {
        crate::ProjectionWatch::new(&self.context, &self.inner)
    }

    pub fn all(&self) -> Result<Vec<P::Output>> {
        self.inner.all(&self.context)
    }

    pub fn first_or_none(&self) -> Result<Option<P::Output>> {
        self.inner.first_or_none(&self.context)
    }

    pub fn one_or_none(&self) -> Result<Option<P::Output>> {
        self.inner.one_or_none(&self.context)
    }

    pub fn one(&self) -> Result<P::Output> {
        self.inner.one(&self.context)
    }
}

#[doc(hidden)]
#[must_use]
pub fn __row_shape_error(object: &str) -> Error {
    Error::new(
        ErrorKind::TypeMismatch,
        format!("CFMD row does not match Rust object {object}"),
    )
}

/// Defines a small Rust-first CFMD object without relation ids or column indices in application code.
///
/// A future derive macro can target the same `Object` contract; this macro intentionally keeps the
/// runtime foundation dependency-free and transparent.
#[macro_export]
macro_rules! cfmd_object {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident => $proxy:ident ($key:literal) {
            $( $field_vis:vis $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $( $field_vis $field: $ty ),*
        }

        impl $crate::RowCodec for $name {
            fn into_row(self) -> $crate::Row {
                vec![$($crate::ValueCodec::into_value(self.$field)),*]
            }

            fn from_row(row: &$crate::Row) -> $crate::Result<Self> {
                let mut values = row.iter();
                let value = Self {
                    $(
                        $field: <$ty as $crate::ValueCodec>::from_value(
                            values.next().ok_or_else(|| $crate::__row_shape_error(stringify!($name)))?
                        )?,
                    )*
                };
                if values.next().is_some() {
                    return Err($crate::__row_shape_error(stringify!($name)));
                }
                Ok(value)
            }

            fn accepts(types: &[$crate::Type]) -> bool {
                let mut types = types.iter();
                $(
                    match types.next() {
                        Some(ty) if <$ty as $crate::ValueCodec>::accepts(ty) => {}
                        _ => return false,
                    }
                )*
                types.next().is_none()
            }
        }

        #[derive(Debug, Clone)]
        $vis struct $proxy {
            inner: $crate::ObjectProxy<$name>,
        }

        impl $proxy {
            $(
                #[must_use]
                $field_vis fn $field(&self) -> $crate::Field<$name, $ty> {
                    self.inner.__field::<$ty>(stringify!($field))
                }
            )*
        }

        impl $crate::Object for $name {
            type Proxy = $proxy;

            const KEY: &'static str = $key;

            fn fields() -> Vec<$crate::ObjectFieldSchema> {
                vec![$($crate::ObjectFieldSchema::of::<$ty>(stringify!($field))),*]
            }

            fn proxy(relation: $crate::Relation<Self>) -> Self::Proxy {
                $proxy { inner: $crate::ObjectProxy::__new(relation) }
            }
        }
    };
}

/// Defines an object-first entity with a stable typed identity and strong references.
///
/// Identity is deliberately explicit and first. Scalar fields and references remain ordinary Rust
/// values after materialization; reference traversal exists only on the symbolic proxy.
#[macro_export]
macro_rules! cfmd_entity {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident => $proxy:ident ($key:literal) {
            id $id_vis:vis $id_field:ident;
            fields { $( $field_vis:vis $field:ident : $ty:ty ),* $(,)? }
            refs { $( $ref_vis:vis $ref_field:ident : $target:ty ),* $(,)? }
            optional_refs { $( $opt_vis:vis $opt_field:ident : $opt_target:ty ),* $(,)? }
            many { $( $many_vis:vis $many_field:ident via $via_field:ident : $many_target:ty ),* $(,)? }
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $id_vis $id_field: $crate::Id<$name>,
            $( $field_vis $field: $ty, )*
            $( $ref_vis $ref_field: $crate::Ref<$target>, )*
            $( $opt_vis $opt_field: Option<$crate::Ref<$opt_target>>, )*
        }

        impl $crate::RowCodec for $name {
            fn into_row(self) -> $crate::Row {
                vec![
                    $crate::ValueCodec::into_value(self.$id_field),
                    $( $crate::ValueCodec::into_value(self.$field), )*
                    $( $crate::ValueCodec::into_value(self.$ref_field), )*
                    $( $crate::ValueCodec::into_value(self.$opt_field), )*
                ]
            }

            fn from_row(row: &$crate::Row) -> $crate::Result<Self> {
                let mut values = row.iter();
                let value = Self {
                    $id_field: <$crate::Id<$name> as $crate::ValueCodec>::from_value(
                        values.next().ok_or_else(|| $crate::__row_shape_error(stringify!($name)))?
                    )?,
                    $(
                        $field: <$ty as $crate::ValueCodec>::from_value(
                            values.next().ok_or_else(|| $crate::__row_shape_error(stringify!($name)))?
                        )?,
                    )*
                    $(
                        $ref_field: <$crate::Ref<$target> as $crate::ValueCodec>::from_value(
                            values.next().ok_or_else(|| $crate::__row_shape_error(stringify!($name)))?
                        )?,
                    )*
                    $(
                        $opt_field: <Option<$crate::Ref<$opt_target>> as $crate::ValueCodec>::from_value(
                            values.next().ok_or_else(|| $crate::__row_shape_error(stringify!($name)))?
                        )?,
                    )*
                };
                if values.next().is_some() { return Err($crate::__row_shape_error(stringify!($name))); }
                Ok(value)
            }

            fn accepts(types: &[$crate::Type]) -> bool {
                let mut types = types.iter();
                match types.next() {
                    Some(ty) if <$crate::Id<$name> as $crate::ValueCodec>::accepts(ty) => {}
                    _ => return false,
                }
                $(
                    match types.next() {
                        Some(ty) if <$ty as $crate::ValueCodec>::accepts(ty) => {}
                        _ => return false,
                    }
                )*
                $(
                    match types.next() {
                        Some(ty) if <$crate::Ref<$target> as $crate::ValueCodec>::accepts(ty) => {}
                        _ => return false,
                    }
                )*
                $(
                    match types.next() {
                        Some(ty) if <Option<$crate::Ref<$opt_target>> as $crate::ValueCodec>::accepts(ty) => {}
                        _ => return false,
                    }
                )*
                types.next().is_none()
            }
        }

        #[derive(Debug, Clone)]
        $vis struct $proxy { inner: $crate::ObjectProxy<$name> }

        impl $proxy {
            #[must_use]
            $id_vis fn $id_field(&self) -> $crate::Field<$name, $crate::Id<$name>> {
                self.inner.__field::<$crate::Id<$name>>(stringify!($id_field))
            }
            $(
                #[must_use]
                $field_vis fn $field(&self) -> $crate::Field<$name, $ty> {
                    self.inner.__field::<$ty>(stringify!($field))
                }
            )*
            $(
                #[must_use]
                $ref_vis fn $ref_field(&self) -> $crate::RefField<$name, $target> {
                    self.inner.__ref::<$target>(stringify!($ref_field))
                }
            )*
            $(
                #[must_use]
                $opt_vis fn $opt_field(&self) -> $crate::OptionalRefField<$name, $opt_target> {
                    self.inner.__optional_ref::<$opt_target>(stringify!($opt_field))
                }
            )*
            $(
                #[must_use]
                $many_vis fn $many_field(&self) -> $crate::ManyField<$name, $many_target> {
                    self.inner.__many::<$many_target>(stringify!($many_field))
                }
            )*
        }

        impl $crate::Object for $name {
            type Proxy = $proxy;
            const KEY: &'static str = $key;
            fn fields() -> Vec<$crate::ObjectFieldSchema> {
                vec![
                    $crate::ObjectFieldSchema::of::<$crate::Id<$name>>(stringify!($id_field)),
                    $( $crate::ObjectFieldSchema::of::<$ty>(stringify!($field)), )*
                    $( $crate::ObjectFieldSchema::of::<$crate::Ref<$target>>(stringify!($ref_field)), )*
                    $( $crate::ObjectFieldSchema::of::<Option<$crate::Ref<$opt_target>>>(stringify!($opt_field)), )*
                ]
            }
            fn many_fields() -> Vec<$crate::ObjectManyFieldSchema> {
                vec![
                    $( $crate::ObjectManyFieldSchema::of::<$name, $many_target>(
                        stringify!($many_field),
                        stringify!($via_field),
                    ), )*
                ]
            }
            fn proxy(relation: $crate::Relation<Self>) -> Self::Proxy {
                $proxy { inner: $crate::ObjectProxy::__new(relation) }
            }
        }
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident => $proxy:ident ($key:literal) {
            id $id_vis:vis $id_field:ident;
            fields { $( $field_vis:vis $field:ident : $ty:ty ),* $(,)? }
            refs { $( $ref_vis:vis $ref_field:ident : $target:ty ),* $(,)? }
        }
    ) => {
        $crate::cfmd_entity! {
            $(#[$meta])*
            $vis struct $name => $proxy ($key) {
                id $id_vis $id_field;
                fields { $( $field_vis $field : $ty ),* }
                refs { $( $ref_vis $ref_field : $target ),* }
                optional_refs { }
                many { }
            }
        }
    };
}
