use std::{hash::Hash, marker::PhantomData};

use crate::{
    EqPredicate, EquivalenceId, Object, ObjectEquivalence, ObjectPredicate, OrderComparison,
    PrimitiveEquivalence, Query, Relation, Result, ScalarType, Type, Value, ValueCodec,
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

impl<E: Object> crate::OrderedObjectValue for Id<E> {
    fn __into_ordered_statistic_bound(self) -> crate::OrderedStatisticBound {
        crate::OrderedStatisticBound::HistoricalEntityRef(crate::EntityRef {
            entity_type: E::type_id(),
            id: self.raw(),
        })
    }
}
impl<E: Object> crate::OrderedObjectValue for Ref<E> {
    fn __into_ordered_statistic_bound(self) -> crate::OrderedStatisticBound {
        self.id().__into_ordered_statistic_bound()
    }
}

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
    source_equivalences: Vec<EquivalenceId>,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> crate::ObjectPatchField<S, Ref<T>> for RefField<S, T> {
    fn into_patch_field(self) -> Result<crate::Field<S, Ref<T>>> {
        Ok(crate::Field::__from_parts(
            self.source_relation,
            self.source_column,
            self.equivalence,
        ))
    }
}

impl<S: Object, T: Object> RefField<S, T> {
    pub(crate) fn new(relation: &Relation<S>, column: usize, equivalence: EquivalenceId) -> Self {
        Self {
            source_relation: relation.id(),
            source_width: relation.width(),
            source_column: column,
            equivalence,
            source_equivalences: relation.equivalences().to_vec(),
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

    #[doc(hidden)]
    #[must_use]
    pub fn __path(self) -> RefPath<S, T> {
        RefPath::from_field(self)
    }
}

impl<S: Object, T: Object> crate::ObjectPatchField<S, Ref<T>> for RefPath<S, T> {
    fn into_patch_field(self) -> Result<crate::Field<S, Ref<T>>> {
        let [hop] = self.hops.as_slice() else {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "only a direct reference field can be patched",
            ));
        };
        Ok(crate::Field::__from_parts(
            hop.source_relation,
            hop.source_column,
            hop.equivalence,
        ))
    }
}

#[derive(Debug, Clone)]
struct RefPathHop {
    source_relation: crate::RelationId,
    source_width: usize,
    source_column: usize,
    source_equivalences: Vec<EquivalenceId>,
    target_identity_column: usize,
    equivalence: EquivalenceId,
}

/// Symbolic strong-reference path. Building or extending a path performs no object I/O.
#[derive(Debug)]
pub struct RefPath<S: Object, T: Object> {
    hops: Vec<RefPathHop>,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> Clone for RefPath<S, T> {
    fn clone(&self) -> Self {
        Self {
            hops: self.hops.clone(),
            marker: PhantomData,
        }
    }
}

impl<S: Object, T: Object> RefPath<S, T> {
    fn from_field(field: RefField<S, T>) -> Self {
        let target_identity_column =
            T::identity_column().expect("referenced CFMD object must have an identity field");
        Self {
            hops: vec![RefPathHop {
                source_relation: field.source_relation,
                source_width: field.source_width,
                source_column: field.source_column,
                source_equivalences: field.source_equivalences,
                target_identity_column,
                equivalence: field.equivalence,
            }],
            marker: PhantomData,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __field<V: crate::ObjectValue>(&self, name: &str) -> PathField<S, T, V> {
        let relation = crate::object::symbolic_relation::<T>();
        let fields = T::fields();
        let column = fields
            .iter()
            .position(|field| field.name() == name)
            .expect("generated CFMD path field must exist in its descriptor");
        let equivalence = relation
            .equivalence_at(column)
            .expect("validated CFMD path field has equivalence semantics");
        let ordering = fields[column].ordering().map(|_| {
            crate::OrderingId::new(crate::object::__semantic_id(
                "cfmd.object.field-ordering.v1",
                T::KEY,
                name,
            ))
        });
        PathField {
            path: self.clone(),
            field: crate::Field::__from_semantics(relation.id(), column, equivalence, ordering),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __ref<U: Object>(&self, name: &str) -> RefPath<S, U> {
        let relation = crate::object::symbolic_relation::<T>();
        let fields = T::fields();
        let column = fields
            .iter()
            .position(|field| field.name() == name)
            .expect("generated CFMD path reference must exist in its descriptor");
        match fields[column].role() {
            crate::ObjectFieldRole::Reference { target_type, .. }
                if target_type == U::type_id() => {}
            _ => panic!("generated CFMD path reference has incompatible target type"),
        }
        let equivalence = relation
            .equivalence_at(column)
            .expect("validated CFMD path reference has equivalence semantics");
        let mut hops = self.hops.clone();
        hops.push(RefPathHop {
            source_relation: relation.id(),
            source_width: relation.width(),
            source_column: column,
            source_equivalences: relation.equivalences().to_vec(),
            target_identity_column: U::identity_column()
                .expect("referenced CFMD object must have an identity field"),
            equivalence,
        });
        RefPath {
            hops,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn matches<F, P>(self, predicate: F) -> PathPredicate<S, T, P>
    where
        F: FnOnce(&T::Proxy) -> P,
        P: ObjectPredicate<T>,
    {
        let relation = crate::object::symbolic_relation::<T>();
        let proxy = T::proxy(relation);
        PathPredicate {
            path: self,
            target: predicate(&proxy),
        }
    }

    #[must_use]
    pub fn eq(self, target: Id<T>) -> PathPredicate<S, T, EqPredicate<T>> {
        let relation = crate::object::symbolic_relation::<T>();
        let identity = T::identity_column().expect("referenced CFMD object must have identity");
        let equivalence = relation
            .equivalence_at(identity)
            .expect("referenced CFMD identity has equivalence semantics");
        let field = crate::Field::<T, Id<T>>::__from_parts(relation.id(), identity, equivalence);
        PathPredicate {
            path: self,
            target: field.eq(target),
        }
    }
}

/// Leaf value reached through one or more strong-reference hops.
#[derive(Debug, Clone)]
pub struct PathField<S: Object, T: Object, V> {
    path: RefPath<S, T>,
    field: crate::Field<T, V>,
}

impl<S: Object, T: Object, V: ValueCodec> PathField<S, T, V> {
    #[must_use]
    pub fn eq(self, value: V) -> PathPredicate<S, T, EqPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.eq(value),
        }
    }

    #[must_use]
    pub fn ne(self, value: V) -> PathPredicate<S, T, crate::NotPredicate<T, EqPredicate<T>>> {
        PathPredicate {
            path: self.path,
            target: self.field.ne(value),
        }
    }
}

impl<S: Object, T: Object, V: crate::OrderedObjectValue> PathField<S, T, V> {
    #[must_use]
    pub fn greater_than(self, value: V) -> PathPredicate<S, T, crate::OrderPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.greater_than(value),
        }
    }

    #[must_use]
    pub fn greater_than_or_equal(self, value: V) -> PathPredicate<S, T, crate::OrderPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.greater_than_or_equal(value),
        }
    }

    #[must_use]
    pub fn less_than(self, value: V) -> PathPredicate<S, T, crate::OrderPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.less_than(value),
        }
    }

    #[must_use]
    pub fn less_than_or_equal(self, value: V) -> PathPredicate<S, T, crate::OrderPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.less_than_or_equal(value),
        }
    }

    #[must_use]
    pub fn between(self, lower: V, upper: V) -> PathPredicate<S, T, crate::BetweenPredicate<T>> {
        PathPredicate {
            path: self.path,
            target: self.field.between(lower, upper),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PathPredicate<S: Object, T: Object, P: ObjectPredicate<T>> {
    path: RefPath<S, T>,
    target: P,
}

impl<S: Object, T: Object, P: ObjectPredicate<T>> ObjectPredicate<S> for PathPredicate<S, T, P> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        let Some(first) = self.path.hops.first() else {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "empty reference path cannot form a predicate",
            ));
        };
        if first.source_relation != root.id() || first.source_width != root.width() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "reference path belongs to a different root relation",
            ));
        }

        let target_relation = crate::object::symbolic_relation::<T>();
        let mut selected = self
            .target
            .apply(Query::scan(T::relation_id()), &target_relation)?;
        let mut root_input = Some(input);
        for (index, hop) in self.path.hops.iter().enumerate().rev() {
            let source = if index == 0 {
                root_input.take().expect("root path input is consumed once")
            } else {
                Query::scan(hop.source_relation)
            };
            selected = source
                .join_eq(
                    selected,
                    hop.source_column,
                    hop.target_identity_column,
                    hop.equivalence,
                )
                .project((0..hop.source_width).collect::<Vec<_>>())
                .distinct(hop.source_equivalences.clone());
        }
        Ok(selected)
    }
}

macro_rules! impl_entity_predicate_operators {
    ([$($gen:ident),*] $self_ty:ty => $root:ty where [$($bounds:tt)*]) => {
        impl<$($gen,)* Rhs> std::ops::BitAnd<Rhs> for $self_ty
        where
            Rhs: ObjectPredicate<$root>,
            $($bounds)*
        {
            type Output = crate::AndPredicate<$root, Self, Rhs>;
            fn bitand(self, rhs: Rhs) -> Self::Output { ObjectPredicate::and(self, rhs) }
        }

        impl<$($gen,)* Rhs> std::ops::BitOr<Rhs> for $self_ty
        where
            Rhs: ObjectPredicate<$root>,
            $($bounds)*
        {
            type Output = crate::OrPredicate<$root, Self, Rhs>;
            fn bitor(self, rhs: Rhs) -> Self::Output { ObjectPredicate::or(self, rhs) }
        }

        impl<$($gen),*> std::ops::Not for $self_ty
        where
            $($bounds)*
        {
            type Output = crate::NotPredicate<$root, Self>;
            fn not(self) -> Self::Output { ObjectPredicate::not(self) }
        }
    };
}

impl_entity_predicate_operators!([S, T, P] RefPredicate<S, T, P> => S where [
    S: Object,
    T: Object,
    P: ObjectPredicate<T>,
]);
impl_entity_predicate_operators!([S, T, P] PathPredicate<S, T, P> => S where [
    S: Object,
    T: Object,
    P: ObjectPredicate<T>,
]);
impl_entity_predicate_operators!([S, T] OptionalRefIsSome<S, T> => S where [
    S: Object,
    T: Object,
]);
impl_entity_predicate_operators!([S, T, P] ManyPredicate<S, T, P> => S where [
    S: Object,
    T: Object,
    P: ObjectPredicate<T>,
]);
impl_entity_predicate_operators!([S, T] ManyCountPredicate<S, T> => S where [
    S: Object,
    T: Object,
]);
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

impl<S: Object, T: Object> crate::ObjectPatchField<S, Option<Ref<T>>> for OptionalRefField<S, T> {
    fn into_patch_field(self) -> Result<crate::Field<S, Option<Ref<T>>>> {
        Ok(crate::Field::__from_parts(
            self.source_relation,
            self.source_column,
            self.equivalence,
        ))
    }
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

    #[doc(hidden)]
    #[must_use]
    pub const fn __relation_id(&self) -> crate::RelationId {
        self.relation
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
    pub fn eq(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Eq(expected))
    }

    #[must_use]
    pub fn ne(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::NotEq(expected))
    }

    #[must_use]
    pub fn greater_than(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Order {
            expected,
            comparison: OrderComparison::Greater,
        })
    }

    #[must_use]
    pub fn greater_than_or_equal(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Order {
            expected,
            comparison: OrderComparison::GreaterOrEqual,
        })
    }

    #[must_use]
    pub fn less_than(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Order {
            expected,
            comparison: OrderComparison::Less,
        })
    }

    #[must_use]
    pub fn less_than_or_equal(self, expected: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Order {
            expected,
            comparison: OrderComparison::LessOrEqual,
        })
    }

    #[must_use]
    pub fn between(self, lower: i64, upper: i64) -> ManyCountPredicate<S, T> {
        self.predicate(ManyCountCondition::Between { lower, upper })
    }

    fn predicate(self, condition: ManyCountCondition) -> ManyCountPredicate<S, T> {
        ManyCountPredicate {
            field: self.field,
            condition,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManyCountPredicate<S: Object, T: Object> {
    field: ManyField<S, T>,
    condition: ManyCountCondition,
}

#[derive(Debug, Clone, Copy)]
enum ManyCountCondition {
    Eq(i64),
    NotEq(i64),
    Order {
        expected: i64,
        comparison: OrderComparison,
    },
    Between {
        lower: i64,
        upper: i64,
    },
}

impl ManyCountCondition {
    fn matches_zero(self) -> bool {
        match self {
            Self::Eq(expected) => expected == 0,
            Self::NotEq(expected) => expected != 0,
            Self::Order {
                expected,
                comparison,
            } => match comparison {
                OrderComparison::Less => 0 < expected,
                OrderComparison::LessOrEqual => 0 <= expected,
                OrderComparison::Greater => 0 > expected,
                OrderComparison::GreaterOrEqual => 0 >= expected,
            },
            Self::Between { lower, upper } => lower <= 0 && 0 <= upper,
        }
    }

    fn matching_positive(self, grouped: Query) -> Query {
        match self {
            Self::Eq(expected) => grouped.filter_eq(
                1,
                Value::I64(expected),
                crate::object::count_equivalence_id(),
            ),
            Self::NotEq(expected) => {
                let equal = grouped.clone().filter_eq(
                    1,
                    Value::I64(expected),
                    crate::object::count_equivalence_id(),
                );
                grouped.difference(equal)
            }
            Self::Order {
                expected,
                comparison,
            } => grouped.filter_order(
                1,
                Value::I64(expected),
                crate::object::count_ordering_id(),
                comparison,
            ),
            Self::Between { lower, upper } => grouped
                .filter_order(
                    1,
                    Value::I64(lower),
                    crate::object::count_ordering_id(),
                    OrderComparison::GreaterOrEqual,
                )
                .filter_order(
                    1,
                    Value::I64(upper),
                    crate::object::count_ordering_id(),
                    OrderComparison::LessOrEqual,
                ),
        }
    }

    fn nonmatching_positive(self, grouped: Query) -> Query {
        debug_assert!(self.matches_zero());
        match self {
            Self::Eq(0) => grouped,
            Self::Eq(_) => unreachable!("non-zero equality does not match zero"),
            Self::NotEq(expected) => grouped.filter_eq(
                1,
                Value::I64(expected),
                crate::object::count_equivalence_id(),
            ),
            Self::Order {
                expected,
                comparison,
            } => {
                let complement = match comparison {
                    OrderComparison::Less => OrderComparison::GreaterOrEqual,
                    OrderComparison::LessOrEqual => OrderComparison::Greater,
                    OrderComparison::Greater => OrderComparison::LessOrEqual,
                    OrderComparison::GreaterOrEqual => OrderComparison::Less,
                };
                grouped.filter_order(
                    1,
                    Value::I64(expected),
                    crate::object::count_ordering_id(),
                    complement,
                )
            }
            Self::Between { upper, .. } => grouped.filter_order(
                1,
                Value::I64(upper),
                crate::object::count_ordering_id(),
                OrderComparison::Greater,
            ),
        }
    }
}

impl<S: Object, T: Object> ObjectPredicate<S> for ManyCountPredicate<S, T> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if let Some(error) = self.field.error {
            return Err(error);
        }
        let edges = Query::scan(self.field.relation);
        let grouped = edges.group_count(
            vec![0],
            vec![self.field.source_equivalence],
            crate::object::count_equivalence_id(),
        );

        if self.condition.matches_zero() {
            let rejected = self.condition.nonmatching_positive(grouped);
            return Ok(input.anti_join(
                rejected,
                self.field.source_identity_column,
                0,
                self.field.source_equivalence,
            ));
        }

        let accepted = self.condition.matching_positive(grouped);
        Ok(input
            .join_eq(
                accepted,
                self.field.source_identity_column,
                0,
                self.field.source_equivalence,
            )
            .project((0..self.field.source_width).collect::<Vec<_>>())
            .distinct(root.equivalences().to_vec()))
    }
}
