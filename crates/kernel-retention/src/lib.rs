use std::collections::BTreeSet;

use kernel_types::SemanticId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErasureDomain(pub SemanticId);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionLabel(BTreeSet<ErasureDomain>);

impl RetentionLabel {
    #[must_use]
    pub fn public() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_domain(domain: ErasureDomain) -> Self {
        Self(BTreeSet::from([domain]))
    }

    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        Self(self.0.union(&other.0).copied().collect())
    }

    #[must_use]
    pub fn contains(&self, domain: ErasureDomain) -> bool {
        self.0.contains(&domain)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked<T> {
    value: T,
    label: RetentionLabel,
}

impl<T> Tracked<T> {
    #[must_use]
    pub fn public(value: T) -> Self {
        Self {
            value,
            label: RetentionLabel::public(),
        }
    }

    #[must_use]
    pub fn protected(value: T, domain: ErasureDomain) -> Self {
        Self {
            value,
            label: RetentionLabel::from_domain(domain),
        }
    }

    #[must_use]
    pub fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub fn label(&self) -> &RetentionLabel {
        &self.label
    }
}

#[must_use]
pub fn add_i64(left: &Tracked<i64>, right: &Tracked<i64>) -> Option<Tracked<i64>> {
    let label = left.label.join(&right.label);
    Some(Tracked {
        value: left.value.checked_add(right.value)?,
        label,
    })
}

#[must_use]
pub fn select<T>(
    condition: Tracked<bool>,
    when_true: Tracked<T>,
    when_false: Tracked<T>,
) -> Tracked<T> {
    let pc_label = condition.label;
    let chosen = if condition.value {
        when_true
    } else {
        when_false
    };
    Tracked {
        value: chosen.value,
        label: pc_label.join(&chosen.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(raw: u128) -> ErasureDomain {
        ErasureDomain(SemanticId::new(raw))
    }

    #[test]
    fn explicit_dataflow_joins_retention_labels() {
        let a = Tracked::protected(2_i64, domain(1));
        let b = Tracked::protected(3_i64, domain(2));
        let sum = add_i64(&a, &b).unwrap();
        assert_eq!(*sum.value(), 5);
        assert!(sum.label().contains(domain(1)));
        assert!(sum.label().contains(domain(2)));
    }

    #[test]
    fn branch_condition_taints_result_through_pc_label() {
        let secret_bit = Tracked::protected(true, domain(9));
        let result = select(secret_bit, Tracked::public(1), Tracked::public(0));
        assert_eq!(*result.value(), 1);
        assert!(result.label().contains(domain(9)));
    }

    #[test]
    fn overflow_is_explicit_in_tracked_arithmetic() {
        let result = add_i64(&Tracked::public(i64::MAX), &Tracked::public(1));
        assert_eq!(result, None);
    }
}
