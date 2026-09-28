#[cfg(test)]
use super::{AdaptiveDelta, DeltaSink, DeltaView, ExactDelta, exact_integer_to_i64};
use super::{
    BTreeMap, CanonicalRowKey, ExactDeltaSink, ExactDeltaView, RelQueryError, RelType,
    RelationDelta, Row, canonical_row_key, relation_column_equivalences,
};
use crate::MaintainedDelta;

#[cfg(test)]
pub(super) fn materialize_delta_view<D: DeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    #[cfg(test)]
    RELATION_DELTA_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
    materialize_delta_view_uncounted(delta, result_type)
}

#[cfg(test)]
pub(super) fn materialize_delta_view_uncounted<D: DeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    let mut error = None;
    delta.visit(|weight, row| {
        if weight == 0 || error.is_some() {
            return;
        }
        let magnitude = if weight < 0 {
            let Some(value) = weight.checked_neg() else {
                error = Some(RelQueryError::InconsistentIncrementalDelta);
                return;
            };
            value
        } else {
            weight
        };
        let Ok(magnitude) = usize::try_from(magnitude) else {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        };
        let target = if weight < 0 {
            &mut removed
        } else {
            &mut inserted
        };
        if target.try_reserve(magnitude).is_err() {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        }
        target.extend(std::iter::repeat_n(row.clone(), magnitude));
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(RelationDelta {
        inserted,
        removed,
        result_type,
    })
}

#[cfg(test)]
std::thread_local! {
    static RELATION_DELTA_MATERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_relation_delta_materialization_count() {
    RELATION_DELTA_MATERIALIZATIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn relation_delta_materialization_count() -> usize {
    RELATION_DELTA_MATERIALIZATIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn exact_delta_from_legacy<D: DeltaView<Row>>(delta: &D) -> ExactDelta<Row> {
    let mut exact = ExactDelta::with_capacity(delta.support_len());
    delta.visit(|weight, row| {
        exact.push_exact(kernel_exact::ExactInteger::from_i64(weight), row.clone());
    });
    exact
}

#[cfg(test)]
pub(super) fn exact_delta_to_legacy_checked<D: ExactDeltaView<Row>>(
    delta: &D,
) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
    let mut legacy = AdaptiveDelta::default();
    let mut error = None;
    delta.visit_exact(|weight, row| {
        if error.is_some() {
            return;
        }
        let Some(weight) = exact_integer_to_i64(weight) else {
            error = Some(RelQueryError::DerivedIdentityExhausted);
            return;
        };
        legacy.push_weighted(weight, row.clone());
    });
    error.map_or(Ok(legacy), Err)
}

pub(super) fn materialize_exact_delta_view<D: ExactDeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    #[cfg(test)]
    RELATION_DELTA_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
    materialize_exact_delta_view_uncounted(delta, result_type)
}

pub(super) fn materialize_exact_delta_view_uncounted<D: ExactDeltaView<Row>>(
    delta: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    let mut error = None;
    delta.visit_exact(|weight, row| {
        if error.is_some() || weight.is_zero() {
            return;
        }
        let Some(magnitude) = weight
            .magnitude()
            .to_u64()
            .and_then(|value| usize::try_from(value).ok())
        else {
            error = Some(RelQueryError::DerivedIdentityExhausted);
            return;
        };
        if weight.is_negative() {
            removed.extend(std::iter::repeat_n(row.clone(), magnitude));
        } else {
            inserted.extend(std::iter::repeat_n(row.clone(), magnitude));
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(RelationDelta {
        inserted,
        removed,
        result_type,
    })
}

pub(super) fn maintained_delta_from_relation_delta(delta: RelationDelta) -> MaintainedDelta {
    maintained_delta_from_rows(delta.inserted, delta.removed)
}

pub(super) fn maintained_delta_from_rows(inserted: Vec<Row>, removed: Vec<Row>) -> MaintainedDelta {
    let mut delta = MaintainedDelta::default();
    for row in removed {
        delta.push_exact(kernel_exact::ExactInteger::from_i64(-1), row);
    }
    for row in inserted {
        delta.push_exact(kernel_exact::ExactInteger::from_i64(1), row);
    }
    delta
}

pub(super) fn materialize_exact_quotient_delta_view<D: ExactDeltaView<Row>>(
    delta: &D,
    result_type: RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    struct SignedClass {
        weight: kernel_exact::ExactInteger,
        positive_representative: Option<Row>,
        negative_representative: Option<Row>,
    }

    let equivalences = relation_column_equivalences(&result_type);
    let mut classes = BTreeMap::<CanonicalRowKey, SignedClass>::new();
    let mut error = None;
    delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        let key = match canonical_row_key(row, equivalences, context, registry) {
            Ok(key) => key,
            Err(next) => {
                error = Some(next);
                return;
            }
        };
        let class = classes.entry(key).or_insert_with(|| SignedClass {
            weight: kernel_exact::ExactInteger::default(),
            positive_representative: None,
            negative_representative: None,
        });
        class.weight.add_assign(weight);
        if weight.is_negative() {
            class
                .negative_representative
                .get_or_insert_with(|| row.clone());
        } else {
            class
                .positive_representative
                .get_or_insert_with(|| row.clone());
        }
    });
    if let Some(error) = error {
        return Err(error);
    }

    let is_set = matches!(
        result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    for class in classes.into_values() {
        if class.weight.is_zero() {
            continue;
        }
        if is_set && !class.weight.magnitude().is_one() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let magnitude = class
            .weight
            .magnitude()
            .to_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(RelQueryError::DerivedIdentityExhausted)?;
        if class.weight.is_negative() {
            let representative = class
                .negative_representative
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            removed.extend(std::iter::repeat_n(representative, magnitude));
        } else {
            let representative = class
                .positive_representative
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            inserted.extend(std::iter::repeat_n(representative, magnitude));
        }
    }
    Ok(RelationDelta {
        inserted,
        removed,
        result_type,
    })
}
