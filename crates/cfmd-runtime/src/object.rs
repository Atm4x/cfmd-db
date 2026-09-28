use crate::{
    Error, ErrorKind, Field, Plan, PrimitiveEquivalence, Projection, ReadContext, Relation,
    RelationId, RelationQuery, Result, RowCodec, SchemaBuilder, Type, TypedQuery, ValueCodec,
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
    fn role() -> ObjectFieldRole {
        ObjectFieldRole::Value
    }
}

macro_rules! scalar_object_value {
    ($rust:ty, $type_expr:expr, $equivalence:expr) => {
        impl ObjectValue for $rust {
            fn object_type() -> Type {
                $type_expr
            }

            fn equivalence() -> ObjectEquivalence {
                ObjectEquivalence::Primitive($equivalence)
            }
        }
    };
}

scalar_object_value!((), Type::unit(), PrimitiveEquivalence::UnitExact);
scalar_object_value!(bool, Type::bool(), PrimitiveEquivalence::BoolExact);
scalar_object_value!(i64, Type::i64(), PrimitiveEquivalence::I64Exact);
scalar_object_value!(f64, Type::f64(), PrimitiveEquivalence::F64Bitwise);
scalar_object_value!(String, Type::text(), PrimitiveEquivalence::TextExact);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectFieldSchema {
    name: &'static str,
    ty: Type,
    equivalence: ObjectEquivalence,
    role: ObjectFieldRole,
}

impl ObjectFieldSchema {
    #[must_use]
    pub fn of<V: ObjectValue>(name: &'static str) -> Self {
        Self {
            name,
            ty: V::object_type(),
            equivalence: V::equivalence(),
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
    let mut types = Vec::with_capacity(fields.len());
    let mut equivalences = Vec::with_capacity(fields.len());
    for field in &fields {
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
        equivalences.push(id);
        types.push(field.ty().clone());
    }
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
    builder
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
        self.relation
            .field::<V>(column)
            .expect("CFMD object relation is validated before its proxy is exposed")
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
    pub fn __many<T: Object>(&self, target_reference_name: &str) -> crate::ManyField<E, T> {
        crate::ManyField::new(&self.relation, target_reference_name)
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

    #[must_use]
    pub fn query(&self) -> ObjectQuery<E> {
        ObjectQuery {
            context: self.context.clone(),
            relation: self.relation.clone(),
            inner: self.relation.query(),
        }
    }

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
        if let Some(contract) = crate::object::object_contract::<E>()? {
            plan.register_object_contract(contract);
        }
        plan.insert_typed(&self.relation, value)?;
        Ok(plan)
    }

    /// Returns a proposed transition; it does not mutate the database.
    pub fn remove(&self, value: E) -> Result<Plan> {
        let mut plan = self.context.plan()?;
        if let Some(contract) = crate::object::object_contract::<E>()? {
            plan.register_object_contract(contract);
        }
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

    pub fn all(&self) -> Result<Vec<E>> {
        let result = self.context.execute(&self.inner.clone().raw())?;
        result.rows().iter().map(E::from_row).collect()
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
            let new = rewrite(old.clone());
            plan.remove_typed(&self.relation, old)?;
            plan.insert_typed(&self.relation, new)?;
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
                    self.inner.__many::<$many_target>(stringify!($via_field))
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
