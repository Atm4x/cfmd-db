use kernel_model::Value;

pub type Row = Vec<Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationValue {
    Bag(Vec<Row>),
    Set {
        rows: Vec<Row>,
        column_equivalences: Vec<kernel_types::SemanticId>,
    },
}

impl RelationValue {
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }

    #[must_use]
    pub fn into_rows(self) -> Vec<Row> {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelQueryError {
    UnknownRelation(kernel_types::SemanticId),
    ColumnOutOfBounds,
    EquivalenceArityMismatch,
    TypeMismatch,
    EquivalenceNotCongruentWithInputEquality,
    OrderingNotCongruentWithEquality,
    InconsistentIncrementalDelta,
    StructuralRewriteBaseMismatch,
    RewriteFootprintMismatch,
    SemanticRevisionMismatch,
    RevisionBindingMismatch,
    InvalidRevisionTransition,
    RevisionBoundMutationRequiresPreparedTransition,
    StalePreparedTransition,
    TransitionEpochExhausted,
    DerivedIdentityExhausted,
    CanonicalObservationUnavailable,
    RecursiveAtomOutsideCarrier,
    NonFiniteRecursiveMultiplicity,
    PositiveBagFixpoint(kernel_fixpoint::PositiveBagFixpointError),
    Semantic(kernel_semantics::SemanticError),
    Aggregate(kernel_aggregate::AggregateError),
}

impl From<kernel_aggregate::AggregateError> for RelQueryError {
    fn from(value: kernel_aggregate::AggregateError) -> Self {
        Self::Aggregate(value)
    }
}

impl From<kernel_fixpoint::PositiveBagFixpointError> for RelQueryError {
    fn from(value: kernel_fixpoint::PositiveBagFixpointError) -> Self {
        Self::PositiveBagFixpoint(value)
    }
}

impl From<kernel_semantics::SemanticError> for RelQueryError {
    fn from(value: kernel_semantics::SemanticError) -> Self {
        Self::Semantic(value)
    }
}

pub type RelQueryResult = Result<RelationValue, RelQueryError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelType {
    pub columns: Vec<kernel_schema::TypeExpr>,
    pub semantics: kernel_schema::RelationSemantics,
}

pub(super) fn relation_column_equivalences(relation_type: &RelType) -> &[kernel_types::SemanticId] {
    match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    }
}

pub(super) fn relation_value_from_rows(rows: Vec<Row>, relation_type: &RelType) -> RelationValue {
    match &relation_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AggregateSpec {
    Count {
        result_equivalence: kernel_types::SemanticId,
    },
    ExactF64Sum {
        value_column: usize,
        result_equivalence: kernel_types::SemanticId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderComparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RelExpr {
    Scan(kernel_types::SemanticId),
    FilterEqConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterOrderConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        ordering: kernel_types::SemanticId,
        comparison: OrderComparison,
    },
    FilterEqColumns {
        input: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Project {
        input: Box<Self>,
        columns: Vec<usize>,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
    },
    Union {
        left: Box<Self>,
        right: Box<Self>,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Distinct {
        input: Box<Self>,
        column_equivalences: Vec<kernel_types::SemanticId>,
    },
    Group {
        input: Box<Self>,
        group_columns: Vec<usize>,
        group_equivalences: Vec<kernel_types::SemanticId>,
        aggregate: AggregateSpec,
    },
    TopKWithTies {
        input: Box<Self>,
        column: usize,
        ordering: kernel_types::SemanticId,
        direction: OrderDirection,
        k: usize,
    },
    PromoteToBag(Box<Self>),
}
