use std::{hash::Hash, marker::PhantomData};

use crate::{
    EqPredicate, EquivalenceId, Object, ObjectEquivalence, ObjectPredicate, PrimitiveEquivalence,
    Query, Relation, Result, ScalarType, Type, Value, ValueCodec,
};

/// Stable typed identity of one object-first entity.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id<E: Object> {
    raw: u128,
    marker: PhantomData<fn() -> E>,
}

impl<E: Object> Copy for Id<E> {}
impl<E: Object> Clone for Id<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: Object> Id<E> {
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.raw
    }
    #[must_use]
    pub const fn reference(self) -> Ref<E> {
        Ref {
            id: self,
            context: None,
        }
    }
}

/// Strong object-first relationship value for exactly one target object.
///
/// A detached `Ref<T>` contains only identity. When its owner is materialized by CFMD the
/// reference is bound to that exact snapshot. Merely reading the field performs no I/O;
/// [`Ref::query`] and [`Ref::load`] make relationship traversal explicit.
#[derive(Clone)]
pub struct Ref<E: Object> {
    id: Id<E>,
    context: Option<crate::ReadContext>,
}

impl<E: Object> std::fmt::Debug for Ref<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Ref")
            .field("id", &self.id.raw())
            .field("bound", &self.context.is_some())
            .finish()
    }
}

impl<E: Object> PartialEq for Ref<E> {
    fn eq(&self, other: &Self) -> bool {
        self.id.raw() == other.id.raw()
    }
}

impl<E: Object> Eq for Ref<E> {}

impl<E: Object> PartialOrd for Ref<E> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<E: Object> Ord for Ref<E> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.raw().cmp(&other.id.raw())
    }
}

impl<E: Object> Hash for Ref<E> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.raw().hash(state);
    }
}

impl<E: Object> Ref<E> {
    #[must_use]
    pub const fn new(id: Id<E>) -> Self {
        Self { id, context: None }
    }

    #[must_use]
    pub const fn id(&self) -> Id<E> {
        self.id
    }

    #[must_use]
    pub const fn is_bound(&self) -> bool {
        self.context.is_some()
    }

    #[doc(hidden)]
    pub fn __bind(&mut self, context: crate::ReadContext) {
        self.context = Some(context);
    }

    /// Builds a query for this exact relationship target without performing I/O yet.
    pub fn query(&self) -> Result<crate::ObjectQuery<E>> {
        let context = self.context.as_ref().ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "Ref<T> is not bound to a database snapshot; materialize its owner through CFMD before traversing the relationship",
            )
        })?;
        let set = context.objects::<E>()?;
        let column = E::identity_column().ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!("relationship target {} has no identity", E::KEY),
            )
        })?;
        let equivalence = set.relation().equivalence_at(column).ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!("relationship target {} identity has no equivalence", E::KEY),
            )
        })?;
        let field =
            crate::Field::<E, Id<E>>::__from_parts(set.relation().id(), column, equivalence);
        Ok(set.query().where_(|_| field.eq(self.id)))
    }

    /// Explicitly materializes this relationship target at the owner's snapshot.
    pub fn load(&self) -> Result<E> {
        self.query()?.one()
    }
}

fn entity_value<E: Object>(id: u128) -> Value {
    Value::HistoricalEntityRef(crate::EntityRef {
        entity_type: E::type_id(),
        id,
    })
}

fn decode_entity_value<E: Object>(value: &Value) -> Result<u128> {
    match value {
        Value::HistoricalEntityRef(value) if value.entity_type == E::type_id() => Ok(value.id),
        _ => Err(crate::Error::new(
            crate::ErrorKind::TypeMismatch,
            format!("CFMD value is not an identity/reference for {}", E::KEY),
        )),
    }
}

impl<E: Object> ValueCodec for Id<E> {
    fn into_value(self) -> Value {
        entity_value::<E>(self.raw)
    }
    fn from_value(value: &Value) -> Result<Self> {
        decode_entity_value::<E>(value).map(Self::new)
    }
    fn accepts(ty: &Type) -> bool {
        matches!(ty, Type::Scalar(ScalarType::HistoricalEntityRef(id)) if *id == E::type_id())
    }
}

impl<E: Object> ValueCodec for Ref<E> {
    fn into_value(self) -> Value {
        self.id.into_value()
    }
    fn from_value(value: &Value) -> Result<Self> {
        Id::<E>::from_value(value).map(Self::new)
    }
    fn accepts(ty: &Type) -> bool {
        Id::<E>::accepts(ty)
    }
}

impl<E: Object> crate::ObjectValue for Id<E> {
    fn object_type() -> Type {
        Type::Scalar(ScalarType::HistoricalEntityRef(E::type_id()))
    }
    fn equivalence() -> ObjectEquivalence {
        ObjectEquivalence::Primitive(PrimitiveEquivalence::HistoricalEntityIdExact(E::type_id()))
    }
    fn ordering() -> Option<crate::PrimitiveOrdering> {
        Some(crate::PrimitiveOrdering::HistoricalEntityIdAscending(
            E::type_id(),
        ))
    }
    fn role() -> crate::ObjectFieldRole {
        crate::ObjectFieldRole::Identity(E::type_id())
    }
}

impl<E: Object> crate::ObjectValue for Ref<E> {
    fn object_type() -> Type {
        Type::Scalar(ScalarType::HistoricalEntityRef(E::type_id()))
    }
    fn equivalence() -> ObjectEquivalence {
        ObjectEquivalence::Primitive(PrimitiveEquivalence::HistoricalEntityIdExact(E::type_id()))
    }
    fn ordering() -> Option<crate::PrimitiveOrdering> {
        Some(crate::PrimitiveOrdering::HistoricalEntityIdAscending(
            E::type_id(),
        ))
    }
    fn role() -> crate::ObjectFieldRole {
        crate::ObjectFieldRole::Reference {
            target_type: E::type_id(),
            target_relation: E::relation_id(),
            target_identity_column: E::identity_column()
                .expect("referenced entity must have identity"),
        }
    }
}

impl<E: Object> crate::OrderedObjectValue for Id<E> {}
impl<E: Object> crate::OrderedObjectValue for Ref<E> {}

impl<E: Object> ValueCodec for Option<Ref<E>> {
    fn into_value(self) -> Value {
        Value::Option(self.map(|value| Box::new(value.into_value())))
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Option(None) => Ok(None),
            Value::Option(Some(value)) => Ref::<E>::from_value(value).map(Some),
            _ => Err(crate::Error::new(
                crate::ErrorKind::TypeMismatch,
                format!("CFMD value is not an optional reference for {}", E::KEY),
            )),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty, Type::Option(inner) if Ref::<E>::accepts(inner))
    }
}

impl<E: Object> crate::ObjectValue for Option<Ref<E>> {
    fn object_type() -> Type {
        Type::Option(Box::new(Type::Scalar(ScalarType::HistoricalEntityRef(
            E::type_id(),
        ))))
    }

    fn equivalence() -> ObjectEquivalence {
        ObjectEquivalence::OptionOf(PrimitiveEquivalence::HistoricalEntityIdExact(E::type_id()))
    }

    fn role() -> crate::ObjectFieldRole {
        crate::ObjectFieldRole::OptionalReference {
            target_type: E::type_id(),
            target_relation: E::relation_id(),
            target_identity_column: E::identity_column()
                .expect("referenced entity must have identity"),
        }
    }
}

/// Symbolic strong-reference field. `matches` follows it only in query space.
#[derive(Debug, Clone)]
pub struct RefField<S: Object, T: Object> {
    source_relation: crate::RelationId,
    source_width: usize,
    source_column: usize,
    equivalence: EquivalenceId,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> RefField<S, T> {
    pub(crate) fn new(relation: &Relation<S>, column: usize, equivalence: EquivalenceId) -> Self {
        Self {
            source_relation: relation.id(),
            source_width: relation.width(),
            source_column: column,
            equivalence,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn eq(self, target: Id<T>) -> EqPredicate<S> {
        crate::Field::<S, Ref<T>>::__from_parts(
            self.source_relation,
            self.source_column,
            self.equivalence,
        )
        .eq(target.reference())
    }

    /// Builds a deep predicate. Missing targets simply do not match; no object I/O occurs.
    #[must_use]
    pub fn matches<F, P>(self, predicate: F) -> RefPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        let relation = crate::object::symbolic_relation::<T>();
        let proxy = T::proxy(relation);
        RefPredicate {
            source_width: self.source_width,
            source_column: self.source_column,
            equivalence: self.equivalence,
            target: predicate(&proxy),
            marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefPredicate<S: Object, T: Object, P: ObjectPredicate<T>> {
    source_width: usize,
    source_column: usize,
    equivalence: EquivalenceId,
    target: P,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object, P: ObjectPredicate<T>> ObjectPredicate<S> for RefPredicate<S, T, P> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if root.width() != self.source_width {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "reference predicate root shape does not match its relation handle",
            ));
        }
        let identity = T::identity_column().ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!("referenced object {} has no identity field", T::KEY),
            )
        })?;
        let target_relation = crate::object::symbolic_relation::<T>();
        let target = self
            .target
            .apply(Query::scan(T::relation_id()), &target_relation)?;
        let joined = input.join_eq(target, self.source_column, identity, self.equivalence);
        Ok(joined
            .project((0..self.source_width).collect::<Vec<_>>())
            .distinct(root.equivalences().to_vec()))
    }
}

/// Symbolic optional-reference field. Materialized `Option<Ref<T>>` remains ordinary Rust data.
#[derive(Debug, Clone)]
pub struct OptionalRefField<S: Object, T: Object> {
    source_relation: crate::RelationId,
    source_column: usize,
    equivalence: EquivalenceId,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> OptionalRefField<S, T> {
    pub(crate) fn new(relation: &Relation<S>, column: usize, equivalence: EquivalenceId) -> Self {
        Self {
            source_relation: relation.id(),
            source_column: column,
            equivalence,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn eq(self, target: Id<T>) -> EqPredicate<S> {
        crate::Field::<S, Option<Ref<T>>>::__from_parts(
            self.source_relation,
            self.source_column,
            self.equivalence,
        )
        .eq(Some(target.reference()))
    }

    #[must_use]
    pub fn is_none(self) -> EqPredicate<S> {
        crate::Field::<S, Option<Ref<T>>>::__from_parts(
            self.source_relation,
            self.source_column,
            self.equivalence,
        )
        .eq(None)
    }

    #[must_use]
    pub fn is_some(self) -> OptionalRefIsSome<S, T> {
        OptionalRefIsSome { field: self }
    }
}

#[derive(Debug, Clone)]
pub struct OptionalRefIsSome<S: Object, T: Object> {
    field: OptionalRefField<S, T>,
}

impl<S: Object, T: Object> ObjectPredicate<S> for OptionalRefIsSome<S, T> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        let none = crate::Field::<S, Option<Ref<T>>>::__from_parts(
            self.field.source_relation,
            self.field.source_column,
            self.field.equivalence,
        )
        .eq(None)
        .apply(input.clone(), root)?;
        Ok(input.difference(none))
    }
}

/// Symbolic zero-to-many object relationship. The public object model owns the relationship;
/// query lowering follows the internal edge relation created by the schema compiler.
#[derive(Debug, Clone)]
pub struct ManyField<S: Object, T: Object> {
    source_width: usize,
    source_identity_column: usize,
    target_identity_column: usize,
    relation: crate::RelationId,
    source_equivalence: EquivalenceId,
    target_equivalence: EquivalenceId,
    error: Option<crate::Error>,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> ManyField<S, T> {
    pub(crate) fn new(source: &Relation<S>, name: &str) -> Self {
        let source_identity_column = S::identity_column();
        let target_identity_column = T::identity_column();
        let source_equivalence = crate::object::__identity_equivalence_id::<S>();
        let target_equivalence = crate::object::__identity_equivalence_id::<T>();
        let error = source_identity_column
            .is_none()
            .then(|| {
                crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    format!("relationship source {} has no identity", S::KEY),
                )
            })
            .or_else(|| {
                target_identity_column.is_none().then(|| {
                    crate::Error::new(
                        crate::ErrorKind::InvalidSchema,
                        format!("relationship target {} has no identity", T::KEY),
                    )
                })
            })
            .or_else(|| source_equivalence.as_ref().err().cloned())
            .or_else(|| target_equivalence.as_ref().err().cloned());
        Self {
            source_width: source.width(),
            source_identity_column: source_identity_column.unwrap_or(0),
            target_identity_column: target_identity_column.unwrap_or(0),
            relation: crate::object::__many_relation_id::<S>(name),
            source_equivalence: source_equivalence.unwrap_or_else(|_| crate::EquivalenceId::new(0)),
            target_equivalence: target_equivalence.unwrap_or_else(|_| crate::EquivalenceId::new(0)),
            error,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn any<F, P>(self, predicate: F) -> ManyPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        self.predicate(ManyMode::Any, predicate)
    }

    #[must_use]
    pub fn none<F, P>(self, predicate: F) -> ManyPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        self.predicate(ManyMode::None, predicate)
    }

    #[must_use]
    pub fn all<F, P>(self, predicate: F) -> ManyPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        self.predicate(ManyMode::All, predicate)
    }

    fn predicate<F, P>(self, mode: ManyMode, predicate: F) -> ManyPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        let relation = crate::object::symbolic_relation::<T>();
        let proxy = T::proxy(relation);
        ManyPredicate {
            field: self,
            mode,
            target: predicate(&proxy),
        }
    }

    #[must_use]
    pub fn count(self) -> ManyCount<S, T> {
        ManyCount { field: self }
    }

    fn matching_sources<P: ObjectPredicate<T>>(&self, target: P, matching: bool) -> Result<Query> {
        let target_relation = crate::object::symbolic_relation::<T>();
        let all_targets = Query::scan(T::relation_id());
        let selected = target.apply(all_targets.clone(), &target_relation)?;
        let selected = if matching {
            selected
        } else {
            all_targets.difference(selected)
        };
        Ok(Query::scan(self.relation)
            .join_eq(
                selected,
                1,
                self.target_identity_column,
                self.target_equivalence,
            )
            .project(vec![0]))
    }
}

#[derive(Debug, Clone, Copy)]
enum ManyMode {
    Any,
    None,
    All,
}

#[derive(Debug, Clone)]
pub struct ManyPredicate<S: Object, T: Object, P: ObjectPredicate<T>> {
    field: ManyField<S, T>,
    mode: ManyMode,
    target: P,
}

impl<S: Object, T: Object, P: ObjectPredicate<T>> ObjectPredicate<S> for ManyPredicate<S, T, P> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if let Some(error) = self.field.error.clone() {
            return Err(error);
        }
        if root.width() != self.field.source_width {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "many predicate root shape does not match its relationship handle",
            ));
        }
        let sources = self
            .field
            .matching_sources(self.target, !matches!(self.mode, ManyMode::All))?;
        let query = match self.mode {
            ManyMode::Any => input
                .join_eq(
                    sources,
                    self.field.source_identity_column,
                    0,
                    self.field.source_equivalence,
                )
                .project((0..self.field.source_width).collect::<Vec<_>>())
                .distinct(root.equivalences().to_vec()),
            ManyMode::None | ManyMode::All => input.anti_join(
                sources,
                self.field.source_identity_column,
                0,
                self.field.source_equivalence,
            ),
        };
        Ok(query)
    }
}

#[derive(Debug, Clone)]
pub struct ManyCount<S: Object, T: Object> {
    field: ManyField<S, T>,
}

impl<S: Object, T: Object> ManyCount<S, T> {
    #[must_use]
    pub fn eq(self, expected: i64) -> ManyCountEq<S, T> {
        ManyCountEq {
            field: self.field,
            expected,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManyCountEq<S: Object, T: Object> {
    field: ManyField<S, T>,
    expected: i64,
}

impl<S: Object, T: Object> ObjectPredicate<S> for ManyCountEq<S, T> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if let Some(error) = self.field.error {
            return Err(error);
        }
        if self.expected < 0 {
            return Ok(input.clone().difference(input));
        }
        let edges = Query::scan(self.field.relation);
        if self.expected == 0 {
            return Ok(input.anti_join(
                edges,
                self.field.source_identity_column,
                0,
                self.field.source_equivalence,
            ));
        }
        let grouped = edges
            .group_count(
                0,
                self.field.source_equivalence,
                crate::object::count_equivalence_id(),
            )
            .filter_eq(
                1,
                Value::I64(self.expected),
                crate::object::count_equivalence_id(),
            );
        Ok(input
            .join_eq(
                grouped,
                self.field.source_identity_column,
                0,
                self.field.source_equivalence,
            )
            .project((0..self.field.source_width).collect::<Vec<_>>())
            .distinct(root.equivalences().to_vec()))
    }
}
