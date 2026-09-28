use std::collections::BTreeMap;

use kernel_change::{FineChange, FineChangeKind, PreparedRewrite, RewriteEffect};
use kernel_model::Value;
use kernel_types::SemanticId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensError {
    TypeMismatch,
    FieldMissing(SemanticId),
    ComplementMismatch,
}

/// Explicit dependent complement captured while projecting a source value.
/// Complements contain logical values only; physical row/index handles cannot
/// enter this representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensComplement {
    Identity,
    ProductRemainder {
        field: SemanticId,
        remainder: BTreeMap<SemanticId, Value>,
    },
    Compose {
        outer: Box<Self>,
        inner: Box<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensExpr {
    Identity,
    ProductField(SemanticId),
    Compose(Box<Self>, Box<Self>),
}

impl LensExpr {
    pub fn split(&self, source: &Value) -> Result<(Value, LensComplement), LensError> {
        match self {
            Self::Identity => Ok((source.clone(), LensComplement::Identity)),
            Self::ProductField(field) => {
                let Value::Product(fields) = source else {
                    return Err(LensError::TypeMismatch);
                };
                let view = fields
                    .get(field)
                    .cloned()
                    .ok_or(LensError::FieldMissing(*field))?;
                let mut remainder = fields.clone();
                remainder.remove(field);
                Ok((
                    view,
                    LensComplement::ProductRemainder {
                        field: *field,
                        remainder,
                    },
                ))
            }
            Self::Compose(outer, inner) => {
                let (middle, outer_complement) = outer.split(source)?;
                let (view, inner_complement) = inner.split(&middle)?;
                Ok((
                    view,
                    LensComplement::Compose {
                        outer: Box::new(outer_complement),
                        inner: Box::new(inner_complement),
                    },
                ))
            }
        }
    }

    pub fn restore(&self, view: &Value, complement: &LensComplement) -> Result<Value, LensError> {
        match (self, complement) {
            (Self::Identity, LensComplement::Identity) => Ok(view.clone()),
            (
                Self::ProductField(expected_field),
                LensComplement::ProductRemainder { field, remainder },
            ) if expected_field == field => {
                let mut fields = remainder.clone();
                fields.insert(*field, view.clone());
                Ok(Value::Product(fields))
            }
            (
                Self::Compose(outer, inner),
                LensComplement::Compose {
                    outer: outer_complement,
                    inner: inner_complement,
                },
            ) => {
                let middle = inner.restore(view, inner_complement)?;
                outer.restore(&middle, outer_complement)
            }
            _ => Err(LensError::ComplementMismatch),
        }
    }

    pub fn get(&self, source: &Value) -> Result<Value, LensError> {
        self.split(source).map(|(view, _)| view)
    }

    pub fn complement(&self, source: &Value) -> Result<LensComplement, LensError> {
        self.split(source).map(|(_, complement)| complement)
    }

    pub fn put(&self, source: &Value, view: &Value) -> Result<Value, LensError> {
        let (_, complement) = self.split(source)?;
        self.restore(view, &complement)
    }

    /// Lifts an intent-bearing rewrite on the view back to an intent-bearing
    /// rewrite on the source. Rewrite identity/law identity are preserved;
    /// only the extensional effect is re-derived through the complement.
    pub fn lift_rewrite<I: Clone>(
        &self,
        source: &Value,
        rewrite: &PreparedRewrite<Value, I>,
    ) -> Result<PreparedRewrite<Value, I>, LensError> {
        let (old_view, complement) = self.split(source)?;
        let new_view = rewrite.apply(&old_view);
        let new_source = self.restore(&new_view, &complement)?;
        let effect = match self {
            Self::Identity => rewrite.effect.clone(),
            Self::ProductField(_) | Self::Compose(_, _) => {
                RewriteEffect::Fine(FineChange::new(FineChangeKind::Product, new_source))
            }
        };
        Ok(PreparedRewrite {
            spec: rewrite.spec,
            explicit_inputs: rewrite.explicit_inputs.clone(),
            effect,
            law_set: rewrite.law_set,
        })
    }

    pub fn check_get_put(&self, source: &Value) -> Result<bool, LensError> {
        let view = self.get(source)?;
        Ok(self.put(source, &view)? == *source)
    }

    pub fn check_put_get(&self, source: &Value, view: &Value) -> Result<bool, LensError> {
        let updated = self.put(source, view)?;
        Ok(self.get(&updated)? == *view)
    }

    pub fn check_put_put(
        &self,
        source: &Value,
        first: &Value,
        second: &Value,
    ) -> Result<bool, LensError> {
        let after_first = self.put(source, first)?;
        let after_both = self.put(&after_first, second)?;
        Ok(after_both == self.put(source, second)?)
    }

    pub fn check_complement_round_trip(&self, source: &Value) -> Result<bool, LensError> {
        let (view, complement) = self.split(source)?;
        Ok(self.restore(&view, &complement)? == *source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_change::{RewriteLawSetId, RewriteSpecId};

    const LEFT: SemanticId = SemanticId(1);
    const NESTED: SemanticId = SemanticId(2);
    const RIGHT: SemanticId = SemanticId(3);
    const NAME: SemanticId = SemanticId(4);
    const FLAG: SemanticId = SemanticId(5);

    fn source() -> Value {
        Value::Product(BTreeMap::from([
            (LEFT, Value::I64(10)),
            (
                NESTED,
                Value::Product(BTreeMap::from([
                    (NAME, Value::Text("name".into())),
                    (FLAG, Value::Bool(true)),
                ])),
            ),
            (RIGHT, Value::I64(30)),
        ]))
    }

    #[test]
    fn product_field_lens_preserves_explicit_complement_and_laws() {
        let lens = LensExpr::ProductField(NESTED);
        let source = source();
        let new_view = Value::Product(BTreeMap::from([
            (NAME, Value::Text("new".into())),
            (FLAG, Value::Bool(false)),
        ]));
        assert_eq!(lens.check_complement_round_trip(&source), Ok(true));
        assert_eq!(lens.check_get_put(&source), Ok(true));
        assert_eq!(lens.check_put_get(&source, &new_view), Ok(true));
        assert_eq!(
            lens.check_put_put(
                &source,
                &Value::Product(BTreeMap::from([
                    (NAME, Value::Text("first".into())),
                    (FLAG, Value::Bool(true)),
                ])),
                &new_view,
            ),
            Ok(true)
        );
        let updated = lens.put(&source, &new_view).unwrap();
        let Value::Product(fields) = updated else {
            unreachable!();
        };
        assert_eq!(fields[&LEFT], Value::I64(10));
        assert_eq!(fields[&RIGHT], Value::I64(30));
    }

    #[test]
    fn composed_lens_composes_dependent_complements() {
        let lens = LensExpr::Compose(
            Box::new(LensExpr::ProductField(NESTED)),
            Box::new(LensExpr::ProductField(NAME)),
        );
        let source = source();
        let new_name = Value::Text("renamed".into());
        assert_eq!(lens.check_complement_round_trip(&source), Ok(true));
        assert_eq!(lens.check_get_put(&source), Ok(true));
        assert_eq!(lens.check_put_get(&source, &new_name), Ok(true));
        let (_, complement) = lens.split(&source).unwrap();
        assert!(matches!(complement, LensComplement::Compose { .. }));
    }

    #[test]
    fn complement_from_another_lens_is_rejected() {
        let source = source();
        let lens = LensExpr::ProductField(NESTED);
        let wrong = LensExpr::ProductField(LEFT).complement(&source).unwrap();
        assert_eq!(
            lens.restore(&Value::I64(9), &wrong),
            Err(LensError::ComplementMismatch)
        );
    }

    #[test]
    fn view_rewrite_lifts_to_source_without_losing_intent_identity() {
        let lens = LensExpr::Compose(
            Box::new(LensExpr::ProductField(NESTED)),
            Box::new(LensExpr::ProductField(NAME)),
        );
        let source = source();
        let rewrite = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(100)),
            explicit_inputs: vec![Value::Text("requested".into())],
            effect: RewriteEffect::Fine(FineChange::new(
                FineChangeKind::Scalar,
                Value::Text("renamed".into()),
            )),
            law_set: RewriteLawSetId(SemanticId(101)),
        };
        let lifted = lens.lift_rewrite(&source, &rewrite).unwrap();
        assert_eq!(lifted.spec, rewrite.spec);
        assert_eq!(lifted.law_set, rewrite.law_set);
        let updated = lifted.apply(&source);
        assert_eq!(lens.get(&updated), Ok(Value::Text("renamed".into())));
        let Value::Product(root) = updated else {
            unreachable!()
        };
        assert_eq!(root[&LEFT], Value::I64(10));
        assert_eq!(root[&RIGHT], Value::I64(30));
    }
}
