use std::cmp::Ordering;

use kernel_exact::ExactNatural;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateError {
    NonFiniteInput,
    CountOverflow,
    CountUnderflow,
}

/// Aggregate-domain wrapper around the shared exact natural coefficient.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

    pub fn finish_i64(&self) -> Result<i64, AggregateError> {
        self.0
            .to_u64()
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(AggregateError::CountOverflow)
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
}
