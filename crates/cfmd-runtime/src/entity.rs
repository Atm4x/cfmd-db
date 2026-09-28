use std::marker::PhantomData;

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
        Ref { id: self }
    }
}

/// Strong object-first reference. It never performs I/O when materialized.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref<E: Object> {
    id: Id<E>,
}
impl<E: Object> Copy for Ref<E> {}
impl<E: Object> Clone for Ref<E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E: Object> Ref<E> {
    #[must_use]
    pub const fn new(id: Id<E>) -> Self {
        Self { id }
    }
    #[must_use]
    pub const fn id(self) -> Id<E> {
        self.id
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
    fn role() -> crate::ObjectFieldRole {
        crate::ObjectFieldRole::Reference {
            target_type: E::type_id(),
            target_relation: E::relation_id(),
            target_identity_column: E::identity_column()
                .expect("referenced entity must have identity"),
        }
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

/// Symbolic reverse-many relationship. It is not stored inside materialized objects.
#[derive(Debug, Clone)]
pub struct ManyField<S: Object, T: Object> {
    source_width: usize,
    source_identity_column: usize,
    target_reference_column: usize,
    join_equivalence: EquivalenceId,
    error: Option<crate::Error>,
    marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Object, T: Object> ManyField<S, T> {
    pub(crate) fn new(source: &Relation<S>, target_reference_name: &str) -> Self {
        let mut error = None;
        let source_identity_column = S::identity_column().unwrap_or_else(|| {
            error = Some(crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!("many relationship source {} has no identity", S::KEY),
            ));
            0
        });
        let target_fields = T::fields();
        let target_reference_column = target_fields
            .iter()
            .position(|field| field.name() == target_reference_name)
            .unwrap_or_else(|| {
                error = Some(crate::Error::new(
                    crate::ErrorKind::InvalidSchema,
                    format!(
                        "many relationship {} -> {} names unknown reference {target_reference_name}",
                        S::KEY,
                        T::KEY
                    ),
                ));
                0
            });
        if error.is_none() {
            match target_fields[target_reference_column].role() {
                crate::ObjectFieldRole::Reference { target_type, .. }
                    if target_type == S::type_id() => {}
                _ => {
                    error = Some(crate::Error::new(
                        crate::ErrorKind::InvalidSchema,
                        format!(
                            "many relationship {} -> {} must point through a required Ref<{}>",
                            S::KEY,
                            T::KEY,
                            S::KEY
                        ),
                    ));
                }
            }
        }
        let join_equivalence = source
            .equivalence_at(source_identity_column)
            .unwrap_or_else(|| crate::EquivalenceId::new(0));
        Self {
            source_width: source.width(),
            source_identity_column,
            target_reference_column,
            join_equivalence,
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
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if let Some(error) = self.field.error {
            return Err(error);
        }
        if root.width() != self.field.source_width {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "many predicate root shape does not match its relation handle",
            ));
        }
        let target_relation = crate::object::symbolic_relation::<T>();
        let all_targets = Query::scan(T::relation_id());
        let matching = self.target.apply(all_targets.clone(), &target_relation)?;
        let query = match self.mode {
            ManyMode::Any => input
                .join_eq(
                    matching,
                    self.field.source_identity_column,
                    self.field.target_reference_column,
                    self.field.join_equivalence,
                )
                .project((0..self.field.source_width).collect::<Vec<_>>())
                .distinct(root.equivalences().to_vec()),
            ManyMode::None => input.anti_join(
                matching,
                self.field.source_identity_column,
                self.field.target_reference_column,
                self.field.join_equivalence,
            ),
            ManyMode::All => input.anti_join(
                all_targets.difference(matching),
                self.field.source_identity_column,
                self.field.target_reference_column,
                self.field.join_equivalence,
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
    fn apply(self, input: Query, root: &Relation<S>) -> Result<Query> {
        if let Some(error) = self.field.error {
            return Err(error);
        }
        if self.expected < 0 {
            return Ok(input.clone().difference(input));
        }
        let all_targets = Query::scan(T::relation_id());
        if self.expected == 0 {
            return Ok(input.anti_join(
                all_targets,
                self.field.source_identity_column,
                self.field.target_reference_column,
                self.field.join_equivalence,
            ));
        }
        let grouped = all_targets
            .group_count(
                self.field.target_reference_column,
                self.field.join_equivalence,
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
                self.field.join_equivalence,
            )
            .project((0..self.field.source_width).collect::<Vec<_>>())
            .distinct(root.equivalences().to_vec()))
    }
}
