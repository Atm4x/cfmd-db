use std::marker::PhantomData;

use crate::{
    EquivalenceId, Error, ErrorKind, OrderComparison, OrderDirection, OrderingId, PreparedQuery,
    Query, ReadContext, RelationId, RelationResult, Result, Row, Type, Value,
};

/// Converts one scalar product value between Rust and the stable CFMD value protocol.
///
/// Domain/generated APIs may implement this trait for newtypes without exposing any kernel type.
pub trait ValueCodec: Sized {
    fn into_value(self) -> Value;
    fn from_value(value: &Value) -> Result<Self>;
    fn accepts(ty: &Type) -> bool;
}

macro_rules! scalar_codec {
    ($rust:ty, $variant:ident, $accept:pat) => {
        impl ValueCodec for $rust {
            fn into_value(self) -> Value {
                Value::$variant(self)
            }

            fn from_value(value: &Value) -> Result<Self> {
                match value {
                    Value::$variant(value) => Ok(value.clone()),
                    _ => Err(type_mismatch(stringify!($rust))),
                }
            }

            fn accepts(ty: &Type) -> bool {
                matches!(ty, $accept)
            }
        }
    };
}

impl ValueCodec for () {
    fn into_value(self) -> Value {
        Value::Unit
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Unit => Ok(()),
            _ => Err(type_mismatch("()")),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty, Type::Scalar(crate::ScalarType::Unit))
    }
}

scalar_codec!(bool, Bool, Type::Scalar(crate::ScalarType::Bool));
scalar_codec!(i64, I64, Type::Scalar(crate::ScalarType::I64));
scalar_codec!(String, Text, Type::Scalar(crate::ScalarType::Text));

impl ValueCodec for f64 {
    fn into_value(self) -> Value {
        Value::F64Bits(self.to_bits())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::F64Bits(bits) => Ok(Self::from_bits(*bits)),
            _ => Err(type_mismatch("f64")),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty, Type::Scalar(crate::ScalarType::F64))
    }
}

impl ValueCodec for Value {
    fn into_value(self) -> Value {
        self
    }

    fn from_value(value: &Value) -> Result<Self> {
        Ok(value.clone())
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

fn type_mismatch(expected: &str) -> Error {
    Error::new(
        ErrorKind::TypeMismatch,
        format!("CFMD value does not decode as {expected}"),
    )
}

/// Encodes/decodes one relation row without exposing the universal `Vec<Value>` representation.
/// Generated domain structs can implement this trait directly.
pub trait RowCodec: Sized {
    fn into_row(self) -> Row;
    fn from_row(row: &Row) -> Result<Self>;
    fn accepts(types: &[Type]) -> bool;
}

impl<A: ValueCodec> RowCodec for (A,) {
    fn into_row(self) -> Row {
        vec![self.0.into_value()]
    }

    fn from_row(row: &Row) -> Result<Self> {
        let value = row.first().ok_or_else(|| type_mismatch("one-column row"))?;
        Ok((A::from_value(value)?,))
    }

    fn accepts(types: &[Type]) -> bool {
        matches!(types, [first] if A::accepts(first))
    }
}

impl<A: ValueCodec, B: ValueCodec> RowCodec for (A, B) {
    fn into_row(self) -> Row {
        vec![self.0.into_value(), self.1.into_value()]
    }

    fn from_row(row: &Row) -> Result<Self> {
        let [first, second] = row.as_slice() else {
            return Err(type_mismatch("two-column row"));
        };
        Ok((A::from_value(first)?, B::from_value(second)?))
    }

    fn accepts(types: &[Type]) -> bool {
        matches!(types, [first, second] if A::accepts(first) && B::accepts(second))
    }
}

impl<A: ValueCodec, B: ValueCodec, C: ValueCodec> RowCodec for (A, B, C) {
    fn into_row(self) -> Row {
        vec![
            self.0.into_value(),
            self.1.into_value(),
            self.2.into_value(),
        ]
    }

    fn from_row(row: &Row) -> Result<Self> {
        let [first, second, third] = row.as_slice() else {
            return Err(type_mismatch("three-column row"));
        };
        Ok((
            A::from_value(first)?,
            B::from_value(second)?,
            C::from_value(third)?,
        ))
    }

    fn accepts(types: &[Type]) -> bool {
        matches!(
            types,
            [first, second, third]
                if A::accepts(first) && B::accepts(second) && C::accepts(third)
        )
    }
}

/// Typed handle for one declared CFMD relation.
///
/// `R` is a zero-sized domain marker. Generated/manual domain modules should use one distinct
/// marker type per relation; the marker never crosses the runtime protocol boundary.
#[derive(Debug)]
pub struct Relation<R> {
    id: RelationId,
    columns: Vec<Type>,
    equivalences: Vec<EquivalenceId>,
    marker: PhantomData<fn() -> R>,
}

impl<R> Clone for Relation<R> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            columns: self.columns.clone(),
            equivalences: self.equivalences.clone(),
            marker: PhantomData,
        }
    }
}

impl<R> Relation<R> {
    pub(crate) fn from_parts(
        id: RelationId,
        columns: Vec<Type>,
        equivalences: Vec<EquivalenceId>,
    ) -> Self {
        Self {
            id,
            columns,
            equivalences,
            marker: PhantomData,
        }
    }

    pub(crate) fn from_schema(schema: &crate::RelationSchema) -> Self {
        let equivalences = match schema.semantics() {
            crate::RelationSemantics::Bag {
                column_equivalences,
            }
            | crate::RelationSemantics::Set {
                column_equivalences,
            } => column_equivalences.clone(),
        };
        Self {
            id: schema.id(),
            columns: schema.columns().to_vec(),
            equivalences,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn id(&self) -> RelationId {
        self.id
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.columns.len()
    }

    pub(crate) fn column_types(&self) -> &[Type] {
        &self.columns
    }

    pub(crate) fn equivalence_at(&self, column: usize) -> Option<EquivalenceId> {
        self.equivalences.get(column).copied()
    }

    pub(crate) fn equivalences(&self) -> &[EquivalenceId] {
        &self.equivalences
    }

    pub fn field<V: ValueCodec>(&self, column: usize) -> Result<Field<R, V>> {
        let ty = self.columns.get(column).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("relation {:?} has no column {column}", self.id),
            )
        })?;
        if !V::accepts(ty) {
            return Err(Error::new(
                ErrorKind::TypeMismatch,
                format!(
                    "column {column} of relation {:?} is incompatible with {}",
                    self.id,
                    std::any::type_name::<V>()
                ),
            ));
        }
        let equivalence = self.equivalences.get(column).copied().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!(
                    "relation {:?} has no equivalence for column {column}",
                    self.id
                ),
            )
        })?;
        Ok(Field {
            relation: self.id,
            column,
            equivalence,
            ordering: None,
            marker: PhantomData,
        })
    }

    pub(crate) fn encode_row<T: RowCodec>(&self, row: T) -> Result<Row> {
        if !T::accepts(&self.columns) {
            return Err(Error::new(
                ErrorKind::TypeMismatch,
                format!(
                    "Rust row type {} is incompatible with relation {:?}",
                    std::any::type_name::<T>(),
                    self.id
                ),
            ));
        }
        Ok(row.into_row())
    }

    #[track_caller]
    #[must_use]
    pub fn query(&self) -> RelationQuery<R> {
        RelationQuery {
            relation: self.clone(),
            inner: Query::scan(self.id),
            error: None,
            marker: PhantomData,
        }
    }
}

/// Typed column handle. Its equality semantics are inherited from the relation schema.
#[derive(Debug)]
pub struct Field<R, V> {
    relation: RelationId,
    column: usize,
    equivalence: EquivalenceId,
    ordering: Option<OrderingId>,
    marker: PhantomData<fn() -> (R, V)>,
}

impl<R, V> Copy for Field<R, V> {}

impl<R, V> Clone for Field<R, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R, V> Field<R, V> {
    pub(crate) const fn __from_parts(
        relation: RelationId,
        column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        Self {
            relation,
            column,
            equivalence,
            ordering: None,
            marker: PhantomData,
        }
    }

    pub(crate) const fn __from_semantics(
        relation: RelationId,
        column: usize,
        equivalence: EquivalenceId,
        ordering: Option<OrderingId>,
    ) -> Self {
        Self {
            relation,
            column,
            equivalence,
            ordering,
            marker: PhantomData,
        }
    }
    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }

    #[must_use]
    pub const fn equivalence(self) -> EquivalenceId {
        self.equivalence
    }

    pub(crate) const fn relation_id(self) -> RelationId {
        self.relation
    }
}

impl<R, V: ValueCodec> Field<R, V> {
    #[must_use]
    pub fn eq<O>(self, other: O) -> EqPredicate<R>
    where
        O: EqOperand<R, V>,
    {
        other.into_predicate(self)
    }

    #[must_use]
    pub fn ne<O>(self, other: O) -> NotPredicate<R, EqPredicate<R>>
    where
        O: EqOperand<R, V>,
    {
        self.eq(other).not()
    }
}

#[doc(hidden)]
pub trait EqOperand<R, V: ValueCodec> {
    fn into_predicate(self, left: Field<R, V>) -> EqPredicate<R>;
}

impl<R, V: ValueCodec> EqOperand<R, V> for V {
    fn into_predicate(self, left: Field<R, V>) -> EqPredicate<R> {
        EqPredicate {
            relation: left.relation,
            kind: EqPredicateKind::Const {
                column: left.column,
                equivalence: left.equivalence,
                value: self.into_value(),
            },
            marker: PhantomData,
        }
    }
}

impl<R, V: ValueCodec> EqOperand<R, V> for Field<R, V> {
    fn into_predicate(self, left: Field<R, V>) -> EqPredicate<R> {
        EqPredicate {
            relation: left.relation,
            kind: EqPredicateKind::Columns {
                right_relation: self.relation,
                left_column: left.column,
                right_column: self.column,
                equivalence: left.equivalence,
            },
            marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
enum EqPredicateKind {
    Const {
        column: usize,
        equivalence: EquivalenceId,
        value: Value,
    },
    Columns {
        right_relation: RelationId,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    },
}

#[derive(Debug, Clone)]
pub struct EqPredicate<R> {
    relation: RelationId,
    kind: EqPredicateKind,
    marker: PhantomData<fn() -> R>,
}

/// Predicate that can transform a root object query while preserving its root row shape.
pub trait ObjectPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query>;

    #[must_use]
    fn and<P: ObjectPredicate<R>>(self, other: P) -> AndPredicate<R, Self, P>
    where
        Self: Sized,
    {
        AndPredicate {
            left: self,
            right: other,
            marker: PhantomData,
        }
    }

    #[must_use]
    fn or<P: ObjectPredicate<R>>(self, other: P) -> OrPredicate<R, Self, P>
    where
        Self: Sized,
    {
        OrPredicate {
            left: self,
            right: other,
            marker: PhantomData,
        }
    }

    #[must_use]
    fn not(self) -> NotPredicate<R, Self>
    where
        Self: Sized,
    {
        NotPredicate {
            inner: self,
            marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndPredicate<R, L, P> {
    left: L,
    right: P,
    marker: PhantomData<fn() -> R>,
}

#[derive(Debug, Clone)]
pub struct OrPredicate<R, L, P> {
    left: L,
    right: P,
    marker: PhantomData<fn() -> R>,
}

#[derive(Debug, Clone)]
pub struct NotPredicate<R, P> {
    inner: P,
    marker: PhantomData<fn() -> R>,
}

impl<R, P> ObjectPredicate<R> for NotPredicate<R, P>
where
    P: ObjectPredicate<R>,
{
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        let selected = self.inner.apply(input.clone(), root)?;
        Ok(input.difference(selected))
    }
}

impl<R, L, P> ObjectPredicate<R> for AndPredicate<R, L, P>
where
    L: ObjectPredicate<R>,
    P: ObjectPredicate<R>,
{
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        let input = self.left.apply(input, root)?;
        self.right.apply(input, root)
    }
}

impl<R, L, P> ObjectPredicate<R> for OrPredicate<R, L, P>
where
    L: ObjectPredicate<R>,
    P: ObjectPredicate<R>,
{
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        let left = self.left.apply(input.clone(), root)?;
        let right = self.right.apply(input, root)?;
        Ok(left.union(right))
    }
}

macro_rules! impl_predicate_operators {
    ([$($gen:ident),*] $self_ty:ty => $root:ty where [$($bounds:tt)*]) => {
        impl<$($gen,)* Rhs> std::ops::BitAnd<Rhs> for $self_ty
        where
            Rhs: ObjectPredicate<$root>,
            $($bounds)*
        {
            type Output = AndPredicate<$root, Self, Rhs>;

            fn bitand(self, rhs: Rhs) -> Self::Output {
                ObjectPredicate::and(self, rhs)
            }
        }

        impl<$($gen,)* Rhs> std::ops::BitOr<Rhs> for $self_ty
        where
            Rhs: ObjectPredicate<$root>,
            $($bounds)*
        {
            type Output = OrPredicate<$root, Self, Rhs>;

            fn bitor(self, rhs: Rhs) -> Self::Output {
                ObjectPredicate::or(self, rhs)
            }
        }

        impl<$($gen),*> std::ops::Not for $self_ty
        where
            $($bounds)*
        {
            type Output = NotPredicate<$root, Self>;

            fn not(self) -> Self::Output {
                ObjectPredicate::not(self)
            }
        }
    };
}

impl_predicate_operators!([R] EqPredicate<R> => R where []);
impl_predicate_operators!([R] OrderPredicate<R> => R where []);
impl_predicate_operators!([R] BetweenPredicate<R> => R where []);
impl_predicate_operators!([R, L, P] AndPredicate<R, L, P> => R where [
    L: ObjectPredicate<R>,
    P: ObjectPredicate<R>,
]);
impl_predicate_operators!([R, L, P] OrPredicate<R, L, P> => R where [
    L: ObjectPredicate<R>,
    P: ObjectPredicate<R>,
]);
impl_predicate_operators!([R, P] NotPredicate<R, P> => R where [
    P: ObjectPredicate<R>,
]);
impl<R> ObjectPredicate<R> for EqPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        if self.relation != root.id() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "predicate belongs to a different relation handle",
            ));
        }
        match self.kind {
            EqPredicateKind::Const {
                column,
                equivalence,
                value,
            } => Ok(input.filter_eq(column, value, equivalence)),
            EqPredicateKind::Columns {
                right_relation,
                left_column,
                right_column,
                equivalence,
            } => {
                if right_relation != root.id() {
                    return Err(Error::new(
                        ErrorKind::InvalidPlan,
                        "field equality compares columns from different relation handles",
                    ));
                }
                Ok(input.filter_eq_columns(left_column, right_column, equivalence))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrderPredicate<R> {
    relation: RelationId,
    column: usize,
    ordering: Option<OrderingId>,
    value: Value,
    comparison: OrderComparison,
    marker: PhantomData<fn() -> R>,
}

impl<R> ObjectPredicate<R> for OrderPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        if self.relation != root.id() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "predicate belongs to a different relation handle",
            ));
        }
        let ordering = self.ordering.ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                "field has no declared canonical ordering",
            )
        })?;
        Ok(input.filter_order(self.column, self.value, ordering, self.comparison))
    }
}

#[derive(Debug, Clone)]
pub struct BetweenPredicate<R> {
    relation: RelationId,
    column: usize,
    ordering: Option<OrderingId>,
    lower: Value,
    upper: Value,
    marker: PhantomData<fn() -> R>,
}

impl<R> ObjectPredicate<R> for BetweenPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        if self.relation != root.id() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "predicate belongs to a different relation handle",
            ));
        }
        let ordering = self.ordering.ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                "field has no declared canonical ordering",
            )
        })?;
        Ok(input
            .filter_order(
                self.column,
                self.lower,
                ordering,
                OrderComparison::GreaterOrEqual,
            )
            .filter_order(
                self.column,
                self.upper,
                ordering,
                OrderComparison::LessOrEqual,
            ))
    }
}

impl<R, V: crate::OrderedObjectValue> Field<R, V> {
    fn order_predicate(self, value: V, comparison: OrderComparison) -> OrderPredicate<R> {
        OrderPredicate {
            relation: self.relation,
            column: self.column,
            ordering: self.ordering,
            value: value.into_value(),
            comparison,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn greater_than(self, value: V) -> OrderPredicate<R> {
        self.order_predicate(value, OrderComparison::Greater)
    }

    #[must_use]
    pub fn greater_than_or_equal(self, value: V) -> OrderPredicate<R> {
        self.order_predicate(value, OrderComparison::GreaterOrEqual)
    }

    #[must_use]
    pub fn less_than(self, value: V) -> OrderPredicate<R> {
        self.order_predicate(value, OrderComparison::Less)
    }

    #[must_use]
    pub fn less_than_or_equal(self, value: V) -> OrderPredicate<R> {
        self.order_predicate(value, OrderComparison::LessOrEqual)
    }

    /// Inclusive canonical-order range: `lower <= field <= upper`.
    #[must_use]
    pub fn between(self, lower: V, upper: V) -> BetweenPredicate<R> {
        BetweenPredicate {
            relation: self.relation,
            column: self.column,
            ordering: self.ordering,
            lower: lower.into_value(),
            upper: upper.into_value(),
            marker: PhantomData,
        }
    }
}

/// Fluent typed query over one relation. It lowers to the stable untyped `Query` IR.
#[derive(Debug)]
pub struct RelationQuery<R> {
    relation: Relation<R>,
    inner: Query,
    error: Option<Error>,
    marker: PhantomData<fn() -> R>,
}

impl<R> Clone for RelationQuery<R> {
    fn clone(&self) -> Self {
        Self {
            relation: self.relation.clone(),
            inner: self.inner.clone(),
            error: self.error.clone(),
            marker: PhantomData,
        }
    }
}

impl<R> RelationQuery<R> {
    #[doc(hidden)]
    #[must_use]
    pub(crate) fn __from_raw(relation: Relation<R>, inner: Query) -> Self {
        Self {
            relation,
            inner,
            error: None,
            marker: PhantomData,
        }
    }

    #[track_caller]
    #[must_use]
    pub fn filter<P: ObjectPredicate<R>>(mut self, predicate: P) -> Self {
        if self.error.is_some() {
            return self;
        }
        match predicate.apply(self.inner.clone(), &self.relation) {
            Ok(inner) => self.inner = inner,
            Err(error) => self.error = Some(error),
        }
        self
    }

    #[track_caller]
    fn boundary_with_ties<V: crate::OrderedObjectValue>(
        mut self,
        field: Field<R, V>,
        direction: OrderDirection,
        k: usize,
    ) -> Self {
        if self.error.is_some() {
            return self;
        }
        if field.relation != self.relation.id() {
            self.error = Some(Error::new(
                ErrorKind::InvalidPlan,
                "ordered boundary field belongs to a different relation handle",
            ));
            return self;
        }
        let Some(ordering) = field.ordering else {
            self.error = Some(Error::new(
                ErrorKind::InvalidSchema,
                "field has no declared canonical ordering",
            ));
            return self;
        };
        self.inner = self
            .inner
            .top_k_with_ties(field.column, ordering, direction, k);
        self
    }

    #[track_caller]
    #[must_use]
    pub fn top<V: crate::OrderedObjectValue>(self, field: Field<R, V>, k: usize) -> Self {
        self.boundary_with_ties(field, OrderDirection::Descending, k)
    }

    #[track_caller]
    #[must_use]
    pub fn bottom<V: crate::OrderedObjectValue>(self, field: Field<R, V>, k: usize) -> Self {
        self.boundary_with_ties(field, OrderDirection::Ascending, k)
    }

    #[track_caller]
    #[must_use]
    pub fn select<P: Projection<R>>(self, projection: P) -> TypedQuery<R, P> {
        let mut error = self.error;
        if !projection.belongs_to(self.relation.id()) {
            error = Some(Error::new(
                ErrorKind::InvalidPlan,
                "typed projection mixes fields from a different relation handle",
            ));
        }
        let columns = projection.columns();
        let equivalences = columns
            .iter()
            .filter_map(|column| self.relation.equivalence_at(*column))
            .collect::<Vec<_>>();
        if equivalences.len() != columns.len() {
            error = Some(Error::new(
                ErrorKind::InvalidSchema,
                "typed projection references a column without semantic equivalence",
            ));
        }
        let inner = self.inner.project_preserving_multiplicity(columns);
        TypedQuery {
            inner,
            projection,
            equivalences,
            error,
            marker: PhantomData,
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

    #[must_use]
    pub fn raw(self) -> Query {
        self.inner
    }
}

/// A typed projection can decode the projected CFMD row into one Rust value.
pub trait Projection<R>: Clone {
    type Output;

    fn belongs_to(&self, relation: RelationId) -> bool;
    fn columns(&self) -> Vec<usize>;
    fn decode(&self, row: &Row) -> Result<Self::Output>;
}

/// Typed key description for kernel-native grouping.
///
/// Application code normally obtains this implicitly by returning a field or tuple of fields
/// from `group_by`; the trait keeps composite Γ keys in the query algebra rather than routing
/// grouping through host-language maps.
pub trait GroupKey<R>: Clone {
    type Output;

    fn belongs_to(&self, relation: RelationId) -> bool;
    fn columns(&self) -> Vec<usize>;
    fn equivalences(&self) -> Vec<EquivalenceId>;
    fn decode(values: &[Value]) -> Result<Self::Output>;
}

impl<R, V: ValueCodec> GroupKey<R> for Field<R, V> {
    type Output = V;

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.relation == relation
    }

    fn columns(&self) -> Vec<usize> {
        vec![self.column]
    }

    fn equivalences(&self) -> Vec<EquivalenceId> {
        vec![self.equivalence]
    }

    fn decode(values: &[Value]) -> Result<Self::Output> {
        let [value] = values else {
            return Err(type_mismatch("one-column group key"));
        };
        V::from_value(value)
    }
}

macro_rules! impl_group_key_tuple {
    ($arity:literal; $( $ty:ident : $index:tt ),+ $(,)?) => {
        impl<R, $( $ty: ValueCodec ),+> GroupKey<R> for ($( Field<R, $ty>, )+) {
            type Output = ($( $ty, )+);

            fn belongs_to(&self, relation: RelationId) -> bool {
                true $( && self.$index.relation == relation )+
            }

            fn columns(&self) -> Vec<usize> {
                vec![$( self.$index.column ),+]
            }

            fn equivalences(&self) -> Vec<EquivalenceId> {
                vec![$( self.$index.equivalence ),+]
            }

            fn decode(values: &[Value]) -> Result<Self::Output> {
                if values.len() != $arity {
                    return Err(type_mismatch(concat!(stringify!($arity), "-column group key")));
                }
                Ok(($( $ty::from_value(&values[$index])?, )+))
            }
        }
    };
}

impl_group_key_tuple!(2; A:0, B:1);
impl_group_key_tuple!(3; A:0, B:1, C:2);
impl_group_key_tuple!(4; A:0, B:1, C:2, D:3);
impl_group_key_tuple!(5; A:0, B:1, C:2, D:3, E:4);
impl_group_key_tuple!(6; A:0, B:1, C:2, D:3, E:4, F:5);
impl_group_key_tuple!(7; A:0, B:1, C:2, D:3, E:4, F:5, G:6);
impl_group_key_tuple!(8; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
impl_group_key_tuple!(9; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
impl_group_key_tuple!(10; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
impl_group_key_tuple!(11; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
impl_group_key_tuple!(12; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

impl<R, V: ValueCodec, const N: usize> GroupKey<R> for [Field<R, V>; N] {
    type Output = [V; N];

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.iter().all(|field| field.relation == relation)
    }

    fn columns(&self) -> Vec<usize> {
        self.iter().map(|field| field.column).collect()
    }

    fn equivalences(&self) -> Vec<EquivalenceId> {
        self.iter().map(|field| field.equivalence).collect()
    }

    fn decode(values: &[Value]) -> Result<Self::Output> {
        if values.len() != N {
            return Err(type_mismatch("fixed-width array group key"));
        }
        values
            .iter()
            .map(V::from_value)
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| type_mismatch("fixed-width array group key"))
    }
}

impl<R, V: ValueCodec> Projection<R> for Field<R, V> {
    type Output = V;

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.relation == relation
    }

    fn columns(&self) -> Vec<usize> {
        vec![self.column]
    }

    fn decode(&self, row: &Row) -> Result<Self::Output> {
        let value = row.first().ok_or_else(|| {
            Error::new(
                ErrorKind::TypeMismatch,
                "projected row is missing its only value",
            )
        })?;
        V::from_value(value)
    }
}

macro_rules! impl_projection_tuple {
    ($arity:literal; $( $ty:ident : $index:tt ),+ $(,)?) => {
        impl<R, $( $ty: ValueCodec ),+> Projection<R> for ($( Field<R, $ty>, )+) {
            type Output = ($( $ty, )+);

            fn belongs_to(&self, relation: RelationId) -> bool {
                true $( && self.$index.relation == relation )+
            }

            fn columns(&self) -> Vec<usize> {
                vec![$( self.$index.column ),+]
            }

            fn decode(&self, row: &Row) -> Result<Self::Output> {
                if row.len() != $arity {
                    return Err(type_mismatch(concat!(stringify!($arity), "-column projection")));
                }
                Ok(($( $ty::from_value(&row[$index])?, )+))
            }
        }
    };
}

impl_projection_tuple!(2; A:0, B:1);
impl_projection_tuple!(3; A:0, B:1, C:2);
impl_projection_tuple!(4; A:0, B:1, C:2, D:3);
impl_projection_tuple!(5; A:0, B:1, C:2, D:3, E:4);
impl_projection_tuple!(6; A:0, B:1, C:2, D:3, E:4, F:5);
impl_projection_tuple!(7; A:0, B:1, C:2, D:3, E:4, F:5, G:6);
impl_projection_tuple!(8; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
impl_projection_tuple!(9; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
impl_projection_tuple!(10; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
impl_projection_tuple!(11; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
impl_projection_tuple!(12; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

impl<R, V: ValueCodec, const N: usize> Projection<R> for [Field<R, V>; N] {
    type Output = [V; N];

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.iter().all(|field| field.relation == relation)
    }

    fn columns(&self) -> Vec<usize> {
        self.iter().map(|field| field.column).collect()
    }

    fn decode(&self, row: &Row) -> Result<Self::Output> {
        if row.len() != N {
            return Err(type_mismatch("fixed-width array projection"));
        }
        row.iter()
            .map(V::from_value)
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| type_mismatch("fixed-width array projection"))
    }
}

#[derive(Debug, Clone)]
pub struct TypedQuery<R, P> {
    inner: Query,
    projection: P,
    equivalences: Vec<EquivalenceId>,
    error: Option<Error>,
    marker: PhantomData<fn() -> R>,
}

impl<R, P: Projection<R>> TypedQuery<R, P> {
    pub(crate) const fn projection(&self) -> &P {
        &self.projection
    }

    #[must_use]
    pub(crate) fn distinct(mut self) -> Self {
        self.inner = self.inner.distinct(self.equivalences.clone());
        self
    }

    pub fn prepare(&self, context: &ReadContext) -> Result<PreparedTypedQuery<R, P>> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(PreparedTypedQuery {
            inner: context.prepare(&self.inner)?,
            projection: self.projection.clone(),
            marker: PhantomData,
        })
    }

    pub fn all(&self, context: &ReadContext) -> Result<Vec<P::Output>> {
        self.prepare(context)?.all(context)
    }

    pub fn first_or_none(&self, context: &ReadContext) -> Result<Option<P::Output>> {
        self.prepare(context)?.first_or_none(context)
    }

    pub fn one(&self, context: &ReadContext) -> Result<P::Output> {
        self.prepare(context)?.one(context)
    }

    pub fn one_or_none(&self, context: &ReadContext) -> Result<Option<P::Output>> {
        self.prepare(context)?.one_or_none(context)
    }

    #[must_use]
    pub const fn node_id(&self) -> crate::QueryNodeId {
        self.inner.node_id()
    }

    #[must_use]
    pub fn source(&self) -> crate::QuerySource {
        self.inner.source()
    }

    #[must_use]
    pub fn raw(&self) -> &Query {
        &self.inner
    }

    pub(crate) fn decode_result(&self, result: &RelationResult) -> Result<Vec<P::Output>> {
        decode_rows(result, &self.projection)
    }
}

#[derive(Debug, Clone)]
pub struct PreparedTypedQuery<R, P> {
    inner: PreparedQuery,
    projection: P,
    marker: PhantomData<fn() -> R>,
}

impl<R, P: Projection<R>> PreparedTypedQuery<R, P> {
    pub fn all(&self, context: &ReadContext) -> Result<Vec<P::Output>> {
        decode_rows(&self.inner.execute(context)?, &self.projection)
    }

    pub fn first_or_none(&self, context: &ReadContext) -> Result<Option<P::Output>> {
        let mut values = self.all(context)?;
        Ok(values.drain(..).next())
    }

    pub fn one(&self, context: &ReadContext) -> Result<P::Output> {
        self.one_or_none(context)?.ok_or_else(|| {
            Error::new(
                ErrorKind::Cardinality,
                "expected exactly one row, query returned none",
            )
        })
    }

    pub fn one_or_none(&self, context: &ReadContext) -> Result<Option<P::Output>> {
        let mut values = self.all(context)?;
        match values.len() {
            0 => Ok(None),
            1 => Ok(values.pop()),
            count => Err(Error::new(
                ErrorKind::Cardinality,
                format!("expected at most one row, query returned {count}"),
            )),
        }
    }
}

fn decode_rows<R, P: Projection<R>>(
    result: &RelationResult,
    projection: &P,
) -> Result<Vec<P::Output>> {
    result
        .rows()
        .iter()
        .map(|row| projection.decode(row))
        .collect()
}
