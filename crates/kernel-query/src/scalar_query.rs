use super::value_shape_matches_type;
use kernel_change::{Change, SeqChangeError, SeqSplice};
use kernel_model::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impact {
    Unaffected,
    Changed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    TypeMismatch,
    IndexOutOfBounds,
    ArithmeticOverflow,
    LengthOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryTypeError {
    TypeMismatch,
    UnknownField(kernel_types::SemanticId),
    AmbiguousConstantType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Input,
    Const(Value),
    TypedConst {
        value: Value,
        ty: kernel_schema::TypeExpr,
    },
    ProductField {
        input: Box<Self>,
        field: kernel_types::SemanticId,
    },
    SeqLength(Box<Self>),
    SeqSumI64(Box<Self>),
    AddI64(Box<Self>, Box<Self>),
    If {
        condition: Box<Self>,
        when_true: Box<Self>,
        when_false: Box<Self>,
    },
}

enum EvalValue<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl EvalValue<'_> {
    fn as_ref(&self) -> &Value {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }

    fn into_owned(self) -> Value {
        match self {
            Self::Borrowed(value) => value.clone(),
            Self::Owned(value) => value,
        }
    }
}

impl Expr {
    pub fn evaluate(&self, input: &Value) -> Result<Value, QueryError> {
        Ok(self.evaluate_internal(input)?.into_owned())
    }

    fn evaluate_internal<'a>(&self, input: &'a Value) -> Result<EvalValue<'a>, QueryError> {
        match self {
            Self::Input => Ok(EvalValue::Borrowed(input)),
            Self::Const(value) | Self::TypedConst { value, .. } => {
                Ok(EvalValue::Owned(value.clone()))
            }
            Self::ProductField {
                input: source,
                field,
            } => match source.evaluate_internal(input)? {
                EvalValue::Borrowed(Value::Product(fields)) => fields
                    .get(field)
                    .map(EvalValue::Borrowed)
                    .ok_or(QueryError::IndexOutOfBounds),
                EvalValue::Owned(Value::Product(mut fields)) => fields
                    .remove(field)
                    .map(EvalValue::Owned)
                    .ok_or(QueryError::IndexOutOfBounds),
                EvalValue::Borrowed(_) | EvalValue::Owned(_) => Err(QueryError::TypeMismatch),
            },
            Self::SeqLength(source) => {
                let source = source.evaluate_internal(input)?;
                let Value::Seq(values) = source.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let len = i64::try_from(values.len()).map_err(|_| QueryError::LengthOverflow)?;
                Ok(EvalValue::Owned(Value::I64(len)))
            }
            Self::SeqSumI64(source) => {
                let source = source.evaluate_internal(input)?;
                let Value::Seq(values) = source.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let mut sum = 0_i64;
                for value in values {
                    let Value::I64(value) = value else {
                        return Err(QueryError::TypeMismatch);
                    };
                    sum = sum
                        .checked_add(*value)
                        .ok_or(QueryError::ArithmeticOverflow)?;
                }
                Ok(EvalValue::Owned(Value::I64(sum)))
            }
            Self::AddI64(left, right) => {
                let left = left.evaluate_internal(input)?;
                let right = right.evaluate_internal(input)?;
                let Value::I64(left) = left.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                let Value::I64(right) = right.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                left.checked_add(*right)
                    .map(|value| EvalValue::Owned(Value::I64(value)))
                    .ok_or(QueryError::ArithmeticOverflow)
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                let condition = condition.evaluate_internal(input)?;
                let Value::Bool(condition) = condition.as_ref() else {
                    return Err(QueryError::TypeMismatch);
                };
                if *condition {
                    when_true.evaluate_internal(input)
                } else {
                    when_false.evaluate_internal(input)
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactQuery {
    root: Expr,
}

impl ExactQuery {
    #[must_use]
    pub const fn new(root: Expr) -> Self {
        Self { root }
    }

    #[must_use]
    pub const fn root(&self) -> &Expr {
        &self.root
    }

    pub fn evaluate(&self, input: &Value) -> Result<Value, QueryError> {
        self.root.evaluate(input)
    }

    pub fn typecheck(
        &self,
        input: &kernel_schema::TypeExpr,
    ) -> Result<kernel_schema::TypeExpr, QueryTypeError> {
        self.root.typecheck(input)
    }
}

impl Expr {
    pub fn typecheck(
        &self,
        input_type: &kernel_schema::TypeExpr,
    ) -> Result<kernel_schema::TypeExpr, QueryTypeError> {
        use kernel_schema::{ScalarType, TypeExpr};
        match self {
            Self::Input => Ok(input_type.clone()),
            Self::Const(value) => {
                infer_value_type(value).ok_or(QueryTypeError::AmbiguousConstantType)
            }
            Self::TypedConst { value, ty } => {
                if value_shape_matches_type(value, ty) {
                    Ok(ty.clone())
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::ProductField { input, field } => {
                let TypeExpr::Product(fields) = input.typecheck(input_type)? else {
                    return Err(QueryTypeError::TypeMismatch);
                };
                fields
                    .get(field)
                    .cloned()
                    .ok_or(QueryTypeError::UnknownField(*field))
            }
            Self::SeqLength(source) => {
                if matches!(source.typecheck(input_type)?, TypeExpr::Seq(_)) {
                    Ok(TypeExpr::Scalar(ScalarType::I64))
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::SeqSumI64(source) => {
                let TypeExpr::Seq(element) = source.typecheck(input_type)? else {
                    return Err(QueryTypeError::TypeMismatch);
                };
                if *element == TypeExpr::Scalar(ScalarType::I64) {
                    Ok(TypeExpr::Scalar(ScalarType::I64))
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::AddI64(left, right) => {
                let expected = TypeExpr::Scalar(ScalarType::I64);
                if left.typecheck(input_type)? == expected
                    && right.typecheck(input_type)? == expected
                {
                    Ok(expected)
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                if condition.typecheck(input_type)? != TypeExpr::Scalar(ScalarType::Bool) {
                    return Err(QueryTypeError::TypeMismatch);
                }
                let left = when_true.typecheck(input_type)?;
                let right = when_false.typecheck(input_type)?;
                if left == right {
                    Ok(left)
                } else {
                    Err(QueryTypeError::TypeMismatch)
                }
            }
        }
    }
}

fn infer_value_type(value: &Value) -> Option<kernel_schema::TypeExpr> {
    use kernel_schema::{ScalarType, TypeExpr};
    match value {
        Value::Unit => Some(TypeExpr::Scalar(ScalarType::Unit)),
        Value::Bool(_) => Some(TypeExpr::Scalar(ScalarType::Bool)),
        Value::I64(_) => Some(TypeExpr::Scalar(ScalarType::I64)),
        Value::F64Bits(_) => Some(TypeExpr::Scalar(ScalarType::F64)),
        Value::Text(_) => Some(TypeExpr::Scalar(ScalarType::Text)),
        Value::LiveEntityRef { entity_type, .. } => {
            Some(TypeExpr::Scalar(ScalarType::LiveEntityRef(*entity_type)))
        }
        Value::HistoricalEntityId { entity_type, .. } => Some(TypeExpr::Scalar(
            ScalarType::HistoricalEntityId(*entity_type),
        )),
        Value::Product(fields) => fields
            .iter()
            .map(|(field, value)| infer_value_type(value).map(|ty| (*field, ty)))
            .collect::<Option<std::collections::BTreeMap<_, _>>>()
            .map(TypeExpr::Product),
        Value::Seq(values) if !values.is_empty() => {
            let first = infer_value_type(&values[0])?;
            if values
                .iter()
                .all(|value| infer_value_type(value).as_ref() == Some(&first))
            {
                Some(TypeExpr::Seq(Box::new(first)))
            } else {
                None
            }
        }
        Value::Option(_)
        | Value::Variant { .. }
        | Value::Seq(_)
        | Value::Set { .. }
        | Value::Bag { .. }
        | Value::Map { .. } => None,
    }
}

pub type QueryResult = Result<Value, QueryError>;

#[must_use]
pub fn derivative_by_recompute(
    query: &ExactQuery,
    old: &Value,
    change: &Change<Value>,
) -> Change<QueryResult> {
    let old_output = query.evaluate(old);
    let new_input = change.apply(old);
    let new_output = query.evaluate(&new_input);

    if old_output == new_output {
        Change::NoChange
    } else {
        Change::Replace(new_output)
    }
}

pub fn derivative_seq_splice(
    query: &ExactQuery,
    old: &Value,
    splice: &SeqSplice<Value>,
) -> Result<Option<Change<QueryResult>>, SeqChangeError> {
    let Expr::SeqLength(source) = &query.root else {
        return Ok(None);
    };
    if !matches!(source.as_ref(), Expr::Input) {
        return Ok(None);
    }
    let Value::Seq(values) = old else {
        return Ok(None);
    };

    let next = splice.apply(values)?;
    let old_len = i64::try_from(values.len()).map_err(|_| SeqChangeError::DeleteOutOfBounds);
    let next_len = i64::try_from(next.len()).map_err(|_| SeqChangeError::DeleteOutOfBounds);
    let (Ok(old_len), Ok(next_len)) = (old_len, next_len) else {
        return Ok(None);
    };
    if old_len == next_len {
        Ok(Some(Change::NoChange))
    } else {
        Ok(Some(Change::Replace(Ok(Value::I64(next_len)))))
    }
}

#[must_use]
pub fn impact_by_recompute(query: &ExactQuery, old: &Value, change: &Change<Value>) -> Impact {
    let old_output = query.evaluate(old);
    let new_input = change.apply(old);
    let new_output = query.evaluate(&new_input);
    if old_output == new_output {
        Impact::Unaffected
    } else {
        Impact::Changed
    }
}
