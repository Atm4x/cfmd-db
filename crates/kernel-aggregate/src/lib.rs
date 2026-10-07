use std::cmp::Ordering;

use kernel_exact::ExactNatural;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateError {
    NonFiniteInput,
    CountOverflow,
    CountUnderflow,
}

/// Aggregate-domain wrapper around the shared exact natural coefficient.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ExactCount(ExactNatural);

impl ExactCount {
    pub fn add_one(&mut self) {
        self.0.add_u128(1);
    }

    pub fn add_many(&mut self, amount: u128) {
        self.0.add_u128(amount);
    }

    pub fn add_exact(&mut self, amount: &ExactNatural) {
        self.0.add_assign(amount);
    }

    pub fn merge(&mut self, other: &Self) {
        self.0.add_assign(&other.0);
    }

    pub fn remove_one(&mut self) -> Result<(), AggregateError> {
        self.remove_many(1)
    }

    pub fn remove_many(&mut self, amount: u128) -> Result<(), AggregateError> {
        self.remove_exact(&ExactNatural::from_u128(amount))
    }

    pub fn remove_exact(&mut self, amount: &ExactNatural) -> Result<(), AggregateError> {
        if self.0.checked_sub_assign(amount) {
            Ok(())
        } else {
            Err(AggregateError::CountUnderflow)
        }
    }

    pub fn remove_count(&mut self, amount: &Self) -> Result<(), AggregateError> {
        self.remove_exact(&amount.0)
    }

    pub fn scale_floor_ratio(&mut self, numerator: u64, denominator: u64) -> Option<()> {
        if denominator == 0 {
            return None;
        }
        self.0.multiply_u64_assign(numerator);
        self.0.div_rem_u64_assign(denominator)?;
        Some(())
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    #[must_use]
    pub fn is_one(&self) -> bool {
        self.0.is_one()
    }

    #[must_use]
    pub fn from_u128(value: u128) -> Self {
        Self(ExactNatural::from_u128(value))
    }

    pub fn finish_u64(&self) -> Result<u64, AggregateError> {
        self.0.to_u64().ok_or(AggregateError::CountOverflow)
    }

    pub fn finish_i64(&self) -> Result<i64, AggregateError> {
        self.finish_u64()
            .and_then(|value| i64::try_from(value).map_err(|_| AggregateError::CountOverflow))
    }
}

/// Exact multiplicity index over an already-canonical ordered key.
///
/// The key type owns ordering semantics. This AVL order-statistics tree is the single
/// multiplicity authority: each node stores its exact local multiplicity and the exact
/// multiplicity of its subtree. Insert/remove/rank/select are O(log D) in the number of
/// distinct canonical keys, including deletion of the current extrema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactOrderedMultiset<K> {
    root: Option<Box<ExactOrderNode<K>>>,
    distinct_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactOrderNode<K> {
    key: K,
    multiplicity: ExactCount,
    subtree_count: ExactCount,
    height: i16,
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
}

type ExactOrderMutation<K> = Result<(Option<Box<ExactOrderNode<K>>>, bool), AggregateError>;

impl<K> ExactOrderNode<K> {
    fn new(key: K) -> Self {
        let multiplicity = ExactCount::from_u128(1);
        Self {
            key,
            subtree_count: multiplicity.clone(),
            multiplicity,
            height: 1,
            left: None,
            right: None,
        }
    }

    fn height(node: Option<&Self>) -> i16 {
        node.map_or(0, |node| node.height)
    }

    fn subtree_count(node: Option<&Self>) -> ExactCount {
        node.map_or_else(ExactCount::default, |node| node.subtree_count.clone())
    }

    fn refresh(&mut self) {
        self.height =
            1 + Self::height(self.left.as_deref()).max(Self::height(self.right.as_deref()));
        self.subtree_count = self.multiplicity.clone();
        self.subtree_count
            .merge(&Self::subtree_count(self.left.as_deref()));
        self.subtree_count
            .merge(&Self::subtree_count(self.right.as_deref()));
    }

    fn balance_factor(&self) -> i16 {
        Self::height(self.left.as_deref()) - Self::height(self.right.as_deref())
    }
}

impl<K> Default for ExactOrderedMultiset<K> {
    fn default() -> Self {
        Self {
            root: None,
            distinct_len: 0,
        }
    }
}

impl<K: Ord + Clone> ExactOrderedMultiset<K> {
    fn rotate_left(mut root: Box<ExactOrderNode<K>>) -> Box<ExactOrderNode<K>> {
        let mut pivot = root
            .right
            .take()
            .expect("AVL left rotation requires right child");
        root.right = pivot.left.take();
        root.refresh();
        pivot.left = Some(root);
        pivot.refresh();
        pivot
    }

    fn rotate_right(mut root: Box<ExactOrderNode<K>>) -> Box<ExactOrderNode<K>> {
        let mut pivot = root
            .left
            .take()
            .expect("AVL right rotation requires left child");
        root.left = pivot.right.take();
        root.refresh();
        pivot.right = Some(root);
        pivot.refresh();
        pivot
    }

    fn rebalance(mut node: Box<ExactOrderNode<K>>) -> Box<ExactOrderNode<K>> {
        node.refresh();
        if node.balance_factor() > 1 {
            if node
                .left
                .as_ref()
                .is_some_and(|left| left.balance_factor() < 0)
            {
                let left = node.left.take().map(Self::rotate_left);
                node.left = left;
            }
            return Self::rotate_right(node);
        }
        if node.balance_factor() < -1 {
            if node
                .right
                .as_ref()
                .is_some_and(|right| right.balance_factor() > 0)
            {
                let right = node.right.take().map(Self::rotate_right);
                node.right = right;
            }
            return Self::rotate_left(node);
        }
        node
    }

    fn insert_node(
        node: Option<Box<ExactOrderNode<K>>>,
        key: K,
    ) -> (Option<Box<ExactOrderNode<K>>>, bool) {
        let Some(mut node) = node else {
            return (Some(Box::new(ExactOrderNode::new(key))), true);
        };
        let inserted_distinct = match key.cmp(&node.key) {
            Ordering::Less => {
                let (left, inserted) = Self::insert_node(node.left.take(), key);
                node.left = left;
                inserted
            }
            Ordering::Greater => {
                let (right, inserted) = Self::insert_node(node.right.take(), key);
                node.right = right;
                inserted
            }
            Ordering::Equal => {
                node.multiplicity.add_one();
                false
            }
        };
        (Some(Self::rebalance(node)), inserted_distinct)
    }

    fn take_min(
        mut node: Box<ExactOrderNode<K>>,
    ) -> (Option<Box<ExactOrderNode<K>>>, K, ExactCount) {
        let Some(left) = node.left.take() else {
            return (node.right.take(), node.key, node.multiplicity);
        };
        let (new_left, key, multiplicity) = Self::take_min(left);
        node.left = new_left;
        (Some(Self::rebalance(node)), key, multiplicity)
    }

    fn remove_node(node: Option<Box<ExactOrderNode<K>>>, key: &K) -> ExactOrderMutation<K> {
        let Some(mut node) = node else {
            return Err(AggregateError::CountUnderflow);
        };
        match key.cmp(&node.key) {
            Ordering::Less => {
                let (left, removed_distinct) = Self::remove_node(node.left.take(), key)?;
                node.left = left;
                Ok((Some(Self::rebalance(node)), removed_distinct))
            }
            Ordering::Greater => {
                let (right, removed_distinct) = Self::remove_node(node.right.take(), key)?;
                node.right = right;
                Ok((Some(Self::rebalance(node)), removed_distinct))
            }
            Ordering::Equal => {
                if !node.multiplicity.is_one() {
                    node.multiplicity.remove_one()?;
                    return Ok((Some(Self::rebalance(node)), false));
                }
                match (node.left.take(), node.right.take()) {
                    (None, None) => Ok((None, true)),
                    (Some(left), None) => Ok((Some(left), true)),
                    (None, Some(right)) => Ok((Some(right), true)),
                    (Some(left), Some(right)) => {
                        let (new_right, successor_key, successor_multiplicity) =
                            Self::take_min(right);
                        node.key = successor_key;
                        node.multiplicity = successor_multiplicity;
                        node.left = Some(left);
                        node.right = new_right;
                        Ok((Some(Self::rebalance(node)), true))
                    }
                }
            }
        }
    }

    fn contains_key(&self, key: &K) -> bool {
        let mut node = self.root.as_deref();
        while let Some(current) = node {
            match key.cmp(&current.key) {
                Ordering::Less => node = current.left.as_deref(),
                Ordering::Greater => node = current.right.as_deref(),
                Ordering::Equal => return true,
            }
        }
        false
    }

    pub fn add_one(&mut self, key: K) {
        let (root, inserted_distinct) = Self::insert_node(self.root.take(), key);
        self.root = root;
        if inserted_distinct {
            self.distinct_len += 1;
        }
    }

    pub fn remove_one(&mut self, key: &K) -> Result<(), AggregateError> {
        if !self.contains_key(key) {
            return Err(AggregateError::CountUnderflow);
        }
        let (root, removed_distinct) = Self::remove_node(self.root.take(), key)
            .expect("prechecked ordered-multiset key must remain removable");
        self.root = root;
        if removed_distinct {
            self.distinct_len -= 1;
        }
        Ok(())
    }

    #[must_use]
    pub fn min_key(&self) -> Option<&K> {
        let mut node = self.root.as_deref()?;
        while let Some(left) = node.left.as_deref() {
            node = left;
        }
        Some(&node.key)
    }

    #[must_use]
    pub fn max_key(&self) -> Option<&K> {
        let mut node = self.root.as_deref()?;
        while let Some(right) = node.right.as_deref() {
            node = right;
        }
        Some(&node.key)
    }

    #[must_use]
    pub fn total_count(&self) -> ExactCount {
        self.root
            .as_ref()
            .map_or_else(ExactCount::default, |root| root.subtree_count.clone())
    }

    #[must_use]
    pub fn select_zero_based(&self, rank: &ExactCount) -> Option<&K> {
        let root = self.root.as_deref()?;
        if rank >= &root.subtree_count {
            return None;
        }
        let mut remaining = rank.clone();
        let mut node = root;
        loop {
            let left_count = ExactOrderNode::subtree_count(node.left.as_deref());
            if remaining < left_count {
                node = node.left.as_deref()?;
                continue;
            }
            remaining.remove_count(&left_count).ok()?;
            if remaining < node.multiplicity {
                return Some(&node.key);
            }
            remaining.remove_count(&node.multiplicity).ok()?;
            node = node.right.as_deref()?;
        }
    }

    #[must_use]
    pub fn select_from_start(&self, rank: u64) -> Option<&K> {
        self.select_zero_based(&ExactCount::from_u128(u128::from(rank)))
    }

    #[must_use]
    pub fn select_from_end(&self, rank: u64) -> Option<&K> {
        let mut zero_based = self.total_count();
        zero_based.remove_one().ok()?;
        zero_based
            .remove_count(&ExactCount::from_u128(u128::from(rank)))
            .ok()?;
        self.select_zero_based(&zero_based)
    }

    /// Selects the lower exact quantile at `p = numerator / denominator`.
    ///
    /// The selected zero-based rank is `floor(p * (n - 1))`; therefore `0/1` is the
    /// minimum, `1/1` is the maximum, and `1/2` is the lower median. Invalid fractions
    /// and empty multisets return `None`.
    #[must_use]
    pub fn select_lower_quantile(&self, numerator: u64, denominator: u64) -> Option<&K> {
        if denominator == 0 || numerator > denominator || self.is_empty() {
            return None;
        }
        let mut rank = self.total_count();
        rank.remove_one().ok()?;
        rank.scale_floor_ratio(numerator, denominator)?;
        self.select_zero_based(&rank)
    }

    #[must_use]
    pub fn rank_lt(&self, key: &K) -> ExactCount {
        let mut result = ExactCount::default();
        let mut node = self.root.as_deref();
        while let Some(current) = node {
            match key.cmp(&current.key) {
                Ordering::Less | Ordering::Equal => node = current.left.as_deref(),
                Ordering::Greater => {
                    result.merge(&ExactOrderNode::subtree_count(current.left.as_deref()));
                    result.merge(&current.multiplicity);
                    node = current.right.as_deref();
                }
            }
        }
        result
    }

    #[must_use]
    pub const fn distinct_len(&self) -> usize {
        self.distinct_len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactF64Sum {
    negative: bool,
    magnitude: ExactNatural,
}

impl ExactF64Sum {
    pub fn add(&mut self, value: f64) -> Result<(), AggregateError> {
        self.add_many(value, 1)
    }

    pub fn add_many(&mut self, value: f64, multiplicity: u64) -> Result<(), AggregateError> {
        self.add_exact(value, &ExactNatural::from_u64(multiplicity))
    }

    pub fn add_exact(
        &mut self,
        value: f64,
        multiplicity: &ExactNatural,
    ) -> Result<(), AggregateError> {
        if multiplicity.is_zero() {
            return Ok(());
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as u16;
        let fraction = bits & ((1_u64 << 52) - 1);
        if exponent == 0x7ff {
            return Err(AggregateError::NonFiniteInput);
        }
        if exponent == 0 && fraction == 0 {
            return Ok(());
        }

        let (mantissa, shift) = if exponent == 0 {
            (fraction, 0_usize)
        } else {
            (
                (1_u64 << 52) | fraction,
                usize::from(exponent.saturating_sub(1)),
            )
        };
        let mut term = ExactNatural::from_shifted_u64(mantissa, shift);
        term.multiply_assign(multiplicity);
        let term_negative = (bits >> 63) != 0;
        self.add_signed(term_negative, &term);
        Ok(())
    }

    pub fn remove(&mut self, value: f64) -> Result<(), AggregateError> {
        self.remove_many(value, 1)
    }

    pub fn remove_many(&mut self, value: f64, multiplicity: u64) -> Result<(), AggregateError> {
        self.remove_exact(value, &ExactNatural::from_u64(multiplicity))
    }

    pub fn remove_exact(
        &mut self,
        value: f64,
        multiplicity: &ExactNatural,
    ) -> Result<(), AggregateError> {
        self.add_exact(-value, multiplicity)
    }

    pub fn merge(&mut self, other: &Self) {
        self.add_signed(other.negative, &other.magnitude);
    }

    pub fn cmp_f64_exact(&self, rhs: f64) -> Result<Ordering, AggregateError> {
        if !rhs.is_finite() {
            return Err(AggregateError::NonFiniteInput);
        }
        let mut delta = self.clone();
        delta.remove(rhs)?;
        if delta.magnitude.is_zero() {
            Ok(Ordering::Equal)
        } else if delta.negative {
            Ok(Ordering::Less)
        } else {
            Ok(Ordering::Greater)
        }
    }

    #[must_use]
    pub fn cmp_exact(&self, rhs: &Self) -> Ordering {
        if self.magnitude.is_zero() && rhs.magnitude.is_zero() {
            return Ordering::Equal;
        }
        match (self.negative, rhs.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => self.magnitude.cmp(&rhs.magnitude),
            (true, true) => rhs.magnitude.cmp(&self.magnitude),
        }
    }

    #[must_use]
    pub fn finish(&self) -> f64 {
        if self.magnitude.is_zero() {
            return 0.0;
        }

        let bit_len = self.magnitude.bit_len();
        if bit_len <= 52 {
            let fraction = self.magnitude.shr_to_u64(0);
            let sign = u64::from(self.negative) << 63;
            return f64::from_bits(sign | fraction);
        }

        if bit_len > 2098 {
            return if self.negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        let Ok(bit_len_i32) = i32::try_from(bit_len) else {
            return if self.negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        };
        let mut unbiased_exponent = bit_len_i32 - 1 - 1074;
        if unbiased_exponent > 1023 {
            return if self.negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }

        let shift = bit_len - 53;
        let mut significand = self.magnitude.shr_to_u64(shift);
        if shift != 0 {
            let half = self.magnitude.bit(shift - 1);
            let lower = self.magnitude.any_bits_below(shift - 1);
            if half && (lower || significand & 1 != 0) {
                significand += 1;
                if significand == (1_u64 << 53) {
                    significand >>= 1;
                    unbiased_exponent += 1;
                    if unbiased_exponent > 1023 {
                        return if self.negative {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        };
                    }
                }
            }
        }

        let sign = u64::from(self.negative) << 63;
        let Ok(exponent_field) = u16::try_from(unbiased_exponent + 1023) else {
            return if self.negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        };
        let exponent_bits = u64::from(exponent_field) << 52;
        let fraction = significand & ((1_u64 << 52) - 1);
        f64::from_bits(sign | exponent_bits | fraction)
    }

    fn add_signed(&mut self, negative: bool, magnitude: &ExactNatural) {
        if magnitude.is_zero() {
            return;
        }
        if self.magnitude.is_zero() {
            self.negative = negative;
            self.magnitude = magnitude.clone();
            return;
        }
        if self.negative == negative {
            self.magnitude.add_assign(magnitude);
            return;
        }
        match self.magnitude.cmp(magnitude) {
            Ordering::Greater => {
                let subtracted = self.magnitude.checked_sub_assign(magnitude);
                debug_assert!(subtracted);
            }
            Ordering::Equal => {
                self.magnitude = ExactNatural::default();
                self.negative = false;
            }
            Ordering::Less => {
                let mut next = magnitude.clone();
                let subtracted = next.checked_sub_assign(&self.magnitude);
                debug_assert!(subtracted);
                self.magnitude = next;
                self.negative = negative;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_sum(values: &[f64]) -> f64 {
        let mut sum = ExactF64Sum::default();
        for &value in values {
            sum.add(value).unwrap();
        }
        sum.finish()
    }

    #[test]
    fn catastrophic_cancellation_is_exact_and_order_independent() {
        assert_eq!(exact_sum(&[1e16, 1.0, -1e16]).to_bits(), 1.0_f64.to_bits());
        assert_eq!(exact_sum(&[-1e16, 1e16, 1.0]).to_bits(), 1.0_f64.to_bits());
        assert_eq!(exact_sum(&[1.0, -1e16, 1e16]).to_bits(), 1.0_f64.to_bits());
    }

    #[test]
    fn merge_is_reproducible_across_partitions() {
        let mut left = ExactF64Sum::default();
        left.add(1e16).unwrap();
        left.add(1.0).unwrap();
        let mut right = ExactF64Sum::default();
        right.add(-1e16).unwrap();
        left.merge(&right);
        assert_eq!(left.finish().to_bits(), 1.0_f64.to_bits());
    }

    #[test]
    fn subnormal_values_sum_exactly() {
        let min = f64::from_bits(1);
        assert_eq!(
            exact_sum(&[min, min]).to_bits(),
            f64::from_bits(2).to_bits()
        );
    }

    #[test]
    fn non_finite_inputs_are_explicitly_rejected() {
        let mut sum = ExactF64Sum::default();
        assert_eq!(sum.add(f64::NAN), Err(AggregateError::NonFiniteInput));
        assert_eq!(sum.add(f64::INFINITY), Err(AggregateError::NonFiniteInput));
    }
    #[test]
    fn exact_count_bulk_updates_cross_representation_boundaries_exactly() {
        let mut count = ExactCount::default();
        count.add_many(u128::from(u64::MAX) + 1);
        count.remove_many(u128::from(u64::MAX)).unwrap();
        assert_eq!(count.finish_i64(), Ok(1));
        assert_eq!(count.remove_many(2), Err(AggregateError::CountUnderflow));
        assert_eq!(count.finish_i64(), Ok(1));
    }

    #[test]
    fn exact_f64_bulk_update_matches_repeated_algebra_without_repetition() {
        let mut bulk = ExactF64Sum::default();
        bulk.add_many(0.25, u64::MAX).unwrap();
        bulk.remove_many(0.25, u64::MAX - 3).unwrap();
        assert_eq!(bulk.finish().to_bits(), 0.75_f64.to_bits());
    }

    #[test]
    fn exact_count_is_mergeable_and_checks_result_range() {
        let mut left = ExactCount::default();
        left.add_one();
        left.add_one();
        let mut right = ExactCount::default();
        right.add_one();
        left.merge(&right);
        assert_eq!(left.finish_i64(), Ok(3));
        let overflow = ExactCount::from_u128(1_u128 << 63);
        assert_eq!(overflow.finish_i64(), Err(AggregateError::CountOverflow));
    }

    #[test]
    fn exact_count_promotes_losslessly_when_small_representation_overflows() {
        let mut by_increment = ExactCount::from_u128(u128::from(u64::MAX));
        by_increment.add_one();
        assert_eq!(
            by_increment.finish_i64(),
            Err(AggregateError::CountOverflow)
        );

        let mut by_merge = ExactCount::from_u128(u128::from(u64::MAX));
        by_merge.merge(&ExactCount::from_u128(1));
        assert_eq!(by_increment, by_merge);
    }

    #[test]
    fn exact_count_decrement_is_checked_and_demotes_after_big_boundary() {
        let mut count = ExactCount::from_u128(u128::from(u64::MAX));
        count.add_one();
        count.remove_one().unwrap();
        assert_eq!(count, ExactCount::from_u128(u128::from(u64::MAX)));

        let mut zero = ExactCount::default();
        assert_eq!(zero.remove_one(), Err(AggregateError::CountUnderflow));
        assert!(zero.is_zero());
    }

    #[test]
    fn exact_f64_sum_removal_is_an_exact_inverse() {
        let mut sum = ExactF64Sum::default();
        for value in [0.1_f64, 0.2, -3.5, f64::MIN_POSITIVE] {
            sum.add(value).unwrap();
            sum.remove(value).unwrap();
            assert_eq!(sum.finish().to_bits(), 0.0_f64.to_bits());
        }
    }
    #[test]
    fn exact_sum_compares_against_finite_bound_without_rounding_loss() {
        let mut sum = ExactF64Sum::default();
        sum.add(1.0e16).unwrap();
        sum.add(1.0).unwrap();
        sum.add(-1.0e16).unwrap();
        assert_eq!(sum.cmp_f64_exact(1.0), Ok(Ordering::Equal));
        assert_eq!(sum.cmp_f64_exact(2.0), Ok(Ordering::Less));
        assert_eq!(sum.cmp_f64_exact(0.0), Ok(Ordering::Greater));
    }

    #[test]
    fn exact_sums_compare_without_rounding_through_f64() {
        let mut left = ExactF64Sum::default();
        left.add(1.0e16).unwrap();
        left.add(1.0).unwrap();
        left.add(-1.0e16).unwrap();
        let mut right = ExactF64Sum::default();
        right.add(1.0).unwrap();
        assert_eq!(left.cmp_exact(&right), Ordering::Equal);

        right.add(f64::MIN_POSITIVE).unwrap();
        assert_eq!(left.cmp_exact(&right), Ordering::Less);

        let mut negative = ExactF64Sum::default();
        negative.add(-2.0).unwrap();
        assert_eq!(negative.cmp_exact(&left), Ordering::Less);
    }

    #[test]
    fn exact_ordered_multiset_rank_select_and_quantile_are_logarithmic_state_queries() {
        let mut values = ExactOrderedMultiset::default();
        for value in [10_i64, 5, 5, 20, 30, 30, 30] {
            values.add_one(value);
        }
        assert_eq!(values.total_count(), ExactCount::from_u128(7));
        assert_eq!(values.rank_lt(&20), ExactCount::from_u128(3));
        assert_eq!(values.select_from_start(0), Some(&5));
        assert_eq!(values.select_from_start(2), Some(&10));
        assert_eq!(values.select_from_end(0), Some(&30));
        assert_eq!(values.select_lower_quantile(1, 2), Some(&20));
        assert_eq!(values.select_lower_quantile(0, 1), Some(&5));
        assert_eq!(values.select_lower_quantile(1, 1), Some(&30));
        assert_eq!(values.select_lower_quantile(2, 1), None);

        values.remove_one(&20).unwrap();
        assert_eq!(values.select_lower_quantile(1, 2), Some(&10));
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_ordered_multiset_rank_select_against_scan_sort() {
        use std::hint::black_box;
        use std::time::Instant;

        const N: i64 = 200_000;
        const QUERIES: usize = 50_000;
        const MUTATIONS: usize = 20_000;

        let mut values = ExactOrderedMultiset::default();
        for value in (0..N).rev() {
            values.add_one(value);
        }

        let query_start = Instant::now();
        for i in 0..QUERIES {
            let key = ((i as u64).wrapping_mul(48_271) % N as u64).cast_signed();
            black_box(values.rank_lt(&key));
            black_box(values.select_from_start(key.cast_unsigned()));
        }
        let query_elapsed = query_start.elapsed();

        let mutation_start = Instant::now();
        for i in 0..MUTATIONS {
            let key = ((i as u64).wrapping_mul(69_069) % N as u64).cast_signed();
            values.remove_one(&key).unwrap();
            values.add_one(key);
        }
        let mutation_elapsed = mutation_start.elapsed();

        let mut source = (0..N).rev().collect::<Vec<_>>();
        let scan_sort_start = Instant::now();
        source.sort_unstable();
        black_box(source[source.len() / 2]);
        let scan_sort_elapsed = scan_sort_start.elapsed();

        println!(
            "PASS529 ordered-stat N={N}: rank+select {query_elapsed:?}/{QUERIES}, delete+insert {mutation_elapsed:?}/{MUTATIONS}, one scan+sort median {scan_sort_elapsed:?}"
        );
    }

    #[test]
    fn exact_ordered_multiset_deletes_current_extrema_without_rescan_state() {
        let mut values = ExactOrderedMultiset::default();
        values.add_one(10_i64);
        values.add_one(5_i64);
        values.add_one(5_i64);
        values.add_one(20_i64);
        assert_eq!(values.min_key(), Some(&5));
        assert_eq!(values.max_key(), Some(&20));
        assert_eq!(values.distinct_len(), 3);

        values.remove_one(&5).unwrap();
        assert_eq!(values.min_key(), Some(&5));
        values.remove_one(&5).unwrap();
        assert_eq!(values.min_key(), Some(&10));
        values.remove_one(&20).unwrap();
        assert_eq!(values.max_key(), Some(&10));
        assert_eq!(values.remove_one(&20), Err(AggregateError::CountUnderflow));
        assert_eq!(values.min_key(), Some(&10));
        assert_eq!(values.max_key(), Some(&10));
    }
}
