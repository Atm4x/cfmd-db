use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateError {
    NonFiniteInput,
    CountOverflow,
    CountUnderflow,
}

fn low_u64(value: u128) -> u64 {
    let bytes = value.to_le_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct BigUnsigned {
    limbs: Vec<u64>,
}

impl BigUnsigned {
    fn from_shifted_u64(value: u64, shift: usize) -> Self {
        if value == 0 {
            return Self::default();
        }
        let limb_shift = shift / 64;
        let bit_shift = shift % 64;
        let mut limbs = vec![0; limb_shift + 2];
        limbs[limb_shift] = value << bit_shift;
        if bit_shift != 0 {
            limbs[limb_shift + 1] = value >> (64 - bit_shift);
        }
        let mut result = Self { limbs };
        result.normalize();
        result
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    fn add_assign(&mut self, other: &Self) {
        let max_len = self.limbs.len().max(other.limbs.len());
        self.limbs.resize(max_len, 0);
        let mut carry = 0_u128;
        for index in 0..max_len {
            let sum = u128::from(self.limbs[index])
                + u128::from(other.limbs.get(index).copied().unwrap_or(0))
                + carry;
            self.limbs[index] = low_u64(sum);
            carry = sum >> 64;
        }
        if carry != 0 {
            self.limbs.push(1);
        }
    }

    fn sub_assign(&mut self, other: &Self) {
        debug_assert_ne!(BigUnsigned::cmp(&*self, other), Ordering::Less);
        let mut borrow = 0_u128;
        for index in 0..self.limbs.len() {
            let left = u128::from(self.limbs[index]);
            let right = u128::from(other.limbs.get(index).copied().unwrap_or(0)) + borrow;
            if left >= right {
                self.limbs[index] = low_u64(left - right);
                borrow = 0;
            } else {
                self.limbs[index] = low_u64((1_u128 << 64) + left - right);
                borrow = 1;
            }
        }
        debug_assert_eq!(borrow, 0);
        self.normalize();
    }

    fn bit_len(&self) -> usize {
        self.limbs.last().map_or(0, |last| {
            (self.limbs.len() - 1) * 64 + (64 - last.leading_zeros() as usize)
        })
    }

    fn bit(&self, index: usize) -> bool {
        let limb = index / 64;
        let bit = index % 64;
        self.limbs
            .get(limb)
            .is_some_and(|value| (value & (1_u64 << bit)) != 0)
    }

    fn any_bits_below(&self, exclusive: usize) -> bool {
        if exclusive == 0 {
            return false;
        }
        let full_limbs = exclusive / 64;
        if self.limbs.iter().take(full_limbs).any(|&value| value != 0) {
            return true;
        }
        let remaining = exclusive % 64;
        if remaining == 0 {
            return false;
        }
        self.limbs
            .get(full_limbs)
            .is_some_and(|value| (*value & ((1_u64 << remaining) - 1)) != 0)
    }

    fn shr_to_u64(&self, shift: usize) -> u64 {
        let limb = shift / 64;
        let bits = shift % 64;
        let low = self.limbs.get(limb).copied().unwrap_or(0) >> bits;
        if bits == 0 {
            low
        } else {
            let high = self.limbs.get(limb + 1).copied().unwrap_or(0) << (64 - bits);
            low | high
        }
    }
}

impl Ord for BigUnsigned {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl PartialOrd for BigUnsigned {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CountRepr {
    Small(u64),
    Big(BigUnsigned),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactCount(CountRepr);

impl Default for ExactCount {
    fn default() -> Self {
        Self(CountRepr::Small(0))
    }
}

impl ExactCount {
    pub fn add_one(&mut self) {
        match &mut self.0 {
            CountRepr::Small(value) => {
                if let Some(next) = value.checked_add(1) {
                    *value = next;
                } else {
                    let mut promoted = BigUnsigned::from_shifted_u64(*value, 0);
                    promoted.add_assign(&BigUnsigned::from_shifted_u64(1, 0));
                    self.0 = CountRepr::Big(promoted);
                }
            }
            CountRepr::Big(value) => {
                value.add_assign(&BigUnsigned::from_shifted_u64(1, 0));
            }
        }
    }

    pub fn merge(&mut self, other: &Self) {
        match (&mut self.0, &other.0) {
            (CountRepr::Small(left), CountRepr::Small(right)) => {
                if let Some(sum) = left.checked_add(*right) {
                    *left = sum;
                } else {
                    let mut promoted = BigUnsigned::from_shifted_u64(*left, 0);
                    promoted.add_assign(&BigUnsigned::from_shifted_u64(*right, 0));
                    self.0 = CountRepr::Big(promoted);
                }
            }
            (CountRepr::Big(left), CountRepr::Small(right)) => {
                left.add_assign(&BigUnsigned::from_shifted_u64(*right, 0));
            }
            (CountRepr::Big(left), CountRepr::Big(right)) => left.add_assign(right),
            (CountRepr::Small(left), CountRepr::Big(right)) => {
                let mut promoted = BigUnsigned::from_shifted_u64(*left, 0);
                promoted.add_assign(right);
                self.0 = CountRepr::Big(promoted);
            }
        }
    }

    pub fn remove_one(&mut self) -> Result<(), AggregateError> {
        match &mut self.0 {
            CountRepr::Small(value) => {
                let Some(next) = value.checked_sub(1) else {
                    return Err(AggregateError::CountUnderflow);
                };
                *value = next;
            }
            CountRepr::Big(value) => {
                if value.is_zero() {
                    return Err(AggregateError::CountUnderflow);
                }
                value.sub_assign(&BigUnsigned::from_shifted_u64(1, 0));
                if value.bit_len() <= 64 {
                    self.0 = CountRepr::Small(value.shr_to_u64(0));
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        match &self.0 {
            CountRepr::Small(value) => *value == 0,
            CountRepr::Big(value) => value.is_zero(),
        }
    }

    #[must_use]
    pub fn is_one(&self) -> bool {
        match &self.0 {
            CountRepr::Small(value) => *value == 1,
            CountRepr::Big(value) => value.bit_len() == 1 && value.shr_to_u64(0) == 1,
        }
    }

    pub fn finish_i64(&self) -> Result<i64, AggregateError> {
        match &self.0 {
            CountRepr::Small(value) => {
                i64::try_from(*value).map_err(|_| AggregateError::CountOverflow)
            }
            CountRepr::Big(value) => {
                if value.bit_len() > 63 {
                    return Err(AggregateError::CountOverflow);
                }
                i64::try_from(value.shr_to_u64(0)).map_err(|_| AggregateError::CountOverflow)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactF64Sum {
    negative: bool,
    magnitude: BigUnsigned,
}

impl ExactF64Sum {
    pub fn add(&mut self, value: f64) -> Result<(), AggregateError> {
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
        let term = BigUnsigned::from_shifted_u64(mantissa, shift);
        let term_negative = (bits >> 63) != 0;
        self.add_signed(term_negative, &term);
        Ok(())
    }

    pub fn remove(&mut self, value: f64) -> Result<(), AggregateError> {
        self.add(-value)
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

    fn add_signed(&mut self, negative: bool, magnitude: &BigUnsigned) {
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
            Ordering::Greater => self.magnitude.sub_assign(magnitude),
            Ordering::Equal => {
                self.magnitude = BigUnsigned::default();
                self.negative = false;
            }
            Ordering::Less => {
                let mut next = magnitude.clone();
                next.sub_assign(&self.magnitude);
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
    fn exact_count_is_mergeable_and_checks_result_range() {
        let mut left = ExactCount::default();
        left.add_one();
        left.add_one();
        let mut right = ExactCount::default();
        right.add_one();
        left.merge(&right);
        assert_eq!(left.finish_i64(), Ok(3));
        let overflow = ExactCount(CountRepr::Big(BigUnsigned::from_shifted_u64(1, 63)));
        assert_eq!(overflow.finish_i64(), Err(AggregateError::CountOverflow));
    }

    #[test]
    fn exact_count_promotes_losslessly_when_small_representation_overflows() {
        let mut by_increment = ExactCount(CountRepr::Small(u64::MAX));
        by_increment.add_one();
        assert!(matches!(by_increment.0, CountRepr::Big(_)));
        assert_eq!(
            by_increment.finish_i64(),
            Err(AggregateError::CountOverflow)
        );

        let mut by_merge = ExactCount(CountRepr::Small(u64::MAX));
        by_merge.merge(&ExactCount(CountRepr::Small(1)));
        assert_eq!(by_increment, by_merge);
    }

    #[test]
    fn exact_count_decrement_is_checked_and_demotes_after_big_boundary() {
        let mut count = ExactCount(CountRepr::Small(u64::MAX));
        count.add_one();
        assert!(matches!(count.0, CountRepr::Big(_)));
        count.remove_one().unwrap();
        assert_eq!(count, ExactCount(CountRepr::Small(u64::MAX)));

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
