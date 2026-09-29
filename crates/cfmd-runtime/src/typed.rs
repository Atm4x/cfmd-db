use std::marker::PhantomData;

use crate::{
    EquivalenceId, Error, ErrorKind, OrderComparison, OrderingId, PreparedQuery, Query,
    ReadContext, RelationId, RelationResult, Result, Row, Type, Value,
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
}

impl<R, V: ValueCodec> Field<R, V> {
    #[must_use]
    pub fn eq(self, value: V) -> EqPredicate<R> {
        EqPredicate {
            relation: self.relation,
            column: self.column,
            equivalence: self.equivalence,
            value: value.into_value(),
            marker: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EqPredicate<R> {
    relation: RelationId,
    column: usize,
    equivalence: EquivalenceId,
    value: Value,
    marker: PhantomData<fn() -> R>,
}

/// Predicate that can transform a root object query while preserving its root row shape.
pub trait ObjectPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query>;
}

impl<R> ObjectPredicate<R> for EqPredicate<R> {
    #[track_caller]
    fn apply(self, input: Query, root: &Relation<R>) -> Result<Query> {
        if self.relation != root.id() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "predicate belongs to a different relation handle",
            ));
        }
        Ok(input.filter_eq(self.column, self.value, self.equivalence))
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
    #[must_use]
    pub fn select<P: Projection<R>>(self, projection: P) -> TypedQuery<R, P> {
        let mut error = self.error;
        if !projection.belongs_to(self.relation.id()) {
            error = Some(Error::new(
                ErrorKind::InvalidPlan,
                "typed projection mixes fields from a different relation handle",
            ));
        }
        let inner = self.inner.project(projection.columns());
        TypedQuery {
            inner,
            projection,
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

impl<R, A, B> Projection<R> for (Field<R, A>, Field<R, B>)
where
    A: ValueCodec,
    B: ValueCodec,
{
    type Output = (A, B);

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.0.relation == relation && self.1.relation == relation
    }

    fn columns(&self) -> Vec<usize> {
        vec![self.0.column, self.1.column]
    }

    fn decode(&self, row: &Row) -> Result<Self::Output> {
        let left = row.first().ok_or_else(|| {
            Error::new(ErrorKind::TypeMismatch, "projected row is missing column 0")
        })?;
        let right = row.get(1).ok_or_else(|| {
            Error::new(ErrorKind::TypeMismatch, "projected row is missing column 1")
        })?;
        Ok((A::from_value(left)?, B::from_value(right)?))
    }
}

impl<R, A, B, C> Projection<R> for (Field<R, A>, Field<R, B>, Field<R, C>)
where
    A: ValueCodec,
    B: ValueCodec,
    C: ValueCodec + Clone,
{
    type Output = (A, B, C);

    fn belongs_to(&self, relation: RelationId) -> bool {
        self.0.relation == relation && self.1.relation == relation && self.2.relation == relation
    }

    fn columns(&self) -> Vec<usize> {
        vec![self.0.column, self.1.column, self.2.column]
    }

    fn decode(&self, row: &Row) -> Result<Self::Output> {
        let first = row.first().ok_or_else(|| {
            Error::new(ErrorKind::TypeMismatch, "projected row is missing column 0")
        })?;
        let second = row.get(1).ok_or_else(|| {
            Error::new(ErrorKind::TypeMismatch, "projected row is missing column 1")
        })?;
        let third = row.get(2).ok_or_else(|| {
            Error::new(ErrorKind::TypeMismatch, "projected row is missing column 2")
        })?;
        Ok((
            A::from_value(first)?,
            B::from_value(second)?,
            C::from_value(third)?,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct TypedQuery<R, P> {
    inner: Query,
    projection: P,
    error: Option<Error>,
    marker: PhantomData<fn() -> R>,
}

impl<R, P: Projection<R>> TypedQuery<R, P> {
    pub(crate) const fn projection(&self) -> &P {
        &self.projection
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
