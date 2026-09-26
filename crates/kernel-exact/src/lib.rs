use std::cmp::Ordering;

fn low_u64(value: u128) -> u64 {
    let bytes = value.to_le_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Exact non-negative integer used as the coefficient domain for finite CFMD measures.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactNatural {
    limbs: Vec<u64>,
}

impl ExactNatural {
    #[must_use]
    pub const fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    #[must_use]
    pub fn one() -> Self {
        Self::from_u64(1)
    }

    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        Self::from_u128(u128::from(value))
    }

    #[must_use]
    pub fn from_u128(value: u128) -> Self {
        if value == 0 {
            return Self::default();
        }
        let low = low_u64(value);
        let high = (value >> 64) as u64;
        let mut limbs = vec![low];
        if high != 0 {
            limbs.push(high);
        }
        Self { limbs }
    }

    #[must_use]
    pub fn from_shifted_u64(value: u64, shift: usize) -> Self {
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

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    #[must_use]
    pub fn is_one(&self) -> bool {
        self.limbs.len() == 1 && self.limbs[0] == 1
    }

    pub fn add_assign(&mut self, other: &Self) {
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

    pub fn add_u128(&mut self, amount: u128) {
        self.add_assign(&Self::from_u128(amount));
    }

    pub fn multiply_u64_assign(&mut self, factor: u64) {
        if factor == 0 {
            self.limbs.clear();
            return;
        }
        if factor == 1 {
            return;
        }
        let mut carry = 0_u128;
        for limb in &mut self.limbs {
            let product = u128::from(*limb) * u128::from(factor) + carry;
            *limb = low_u64(product);
            carry = product >> 64;
        }
        if carry != 0 {
            self.limbs.push(low_u64(carry));
        }
    }

    pub fn multiply_assign(&mut self, other: &Self) {
        if self.is_zero() || other.is_zero() {
            self.limbs.clear();
            return;
        }
        let left = self.limbs.clone();
        let mut result = vec![0_u64; left.len() + other.limbs.len()];
        for (left_index, left_limb) in left.into_iter().enumerate() {
            let mut carry = 0_u128;
            for (right_index, right_limb) in other.limbs.iter().copied().enumerate() {
                let index = left_index + right_index;
                let product = u128::from(left_limb) * u128::from(right_limb)
                    + u128::from(result[index])
                    + carry;
                result[index] = low_u64(product);
                carry = product >> 64;
            }
            let mut index = left_index + other.limbs.len();
            while carry != 0 {
                if index == result.len() {
                    result.push(0);
                }
                let sum = u128::from(result[index]) + carry;
                result[index] = low_u64(sum);
                carry = sum >> 64;
                index += 1;
            }
        }
        self.limbs = result;
        self.normalize();
    }

    #[must_use]
    pub fn multiplied(&self, other: &Self) -> Self {
        let mut result = self.clone();
        result.multiply_assign(other);
        result
    }

    /// Subtracts `other` when `self >= other`; returns `false` without mutation otherwise.
    pub fn checked_sub_assign(&mut self, other: &Self) -> bool {
        if ExactNatural::cmp(&*self, other) == Ordering::Less {
            return false;
        }
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
        true
    }

    #[must_use]
    pub fn bit_len(&self) -> usize {
        self.limbs.last().map_or(0, |last| {
            (self.limbs.len() - 1) * 64 + (64 - last.leading_zeros() as usize)
        })
    }

    #[must_use]
    pub fn bit(&self, index: usize) -> bool {
        let limb = index / 64;
        let bit = index % 64;
        self.limbs
            .get(limb)
            .is_some_and(|value| (value & (1_u64 << bit)) != 0)
    }

    #[must_use]
    pub fn any_bits_below(&self, exclusive: usize) -> bool {
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

    #[must_use]
    pub fn shr_to_u64(&self, shift: usize) -> u64 {
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

    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        (self.bit_len() <= 64).then(|| self.shr_to_u64(0))
    }

    #[must_use]
    pub fn to_decimal_string(&self) -> String {
        if self.is_zero() {
            return "0".to_owned();
        }
        let mut work = self.clone();
        let mut chunks = Vec::new();
        while !work.is_zero() {
            chunks.push(work.div_rem_u32(1_000_000_000));
        }
        let mut chunks = chunks.into_iter().rev();
        let mut out = chunks
            .next()
            .expect("non-zero natural has a decimal chunk")
            .to_string();
        for chunk in chunks {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{chunk:09}");
        }
        out
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    fn div_rem_u32(&mut self, divisor: u32) -> u32 {
        let divisor = u128::from(divisor);
        let mut remainder = 0_u128;
        for limb in self.limbs.iter_mut().rev() {
            let value = (remainder << 64) | u128::from(*limb);
            *limb = u64::try_from(value / divisor).expect("base division quotient fits limb");
            remainder = value % divisor;
        }
        self.normalize();
        u32::try_from(remainder).expect("division remainder fits divisor")
    }
}

impl Ord for ExactNatural {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl PartialOrd for ExactNatural {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Exact signed integer used as the coefficient domain for CFMD deltas.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactInteger {
    negative: bool,
    magnitude: ExactNatural,
}

impl ExactInteger {
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        if value == 0 {
            return Self::default();
        }
        Self {
            negative: value < 0,
            magnitude: ExactNatural::from_u64(value.unsigned_abs()),
        }
    }

    #[must_use]
    pub fn from_parts(negative: bool, magnitude: ExactNatural) -> Self {
        Self {
            negative: negative && !magnitude.is_zero(),
            magnitude,
        }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.magnitude.is_zero()
    }

    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    #[must_use]
    pub fn magnitude(&self) -> &ExactNatural {
        &self.magnitude
    }

    pub fn add_assign(&mut self, other: &Self) {
        if other.is_zero() {
            return;
        }
        if self.is_zero() {
            *self = other.clone();
            return;
        }
        if self.negative == other.negative {
            self.magnitude.add_assign(&other.magnitude);
            return;
        }
        match self.magnitude.cmp(&other.magnitude) {
            Ordering::Greater => {
                let subtracted = self.magnitude.checked_sub_assign(&other.magnitude);
                debug_assert!(subtracted);
            }
            Ordering::Less => {
                let mut magnitude = other.magnitude.clone();
                let subtracted = magnitude.checked_sub_assign(&self.magnitude);
                debug_assert!(subtracted);
                self.magnitude = magnitude;
                self.negative = other.negative;
            }
            Ordering::Equal => {
                self.magnitude = ExactNatural::default();
                self.negative = false;
            }
        }
    }

    #[must_use]
    pub fn multiply(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::default();
        }
        Self {
            negative: self.negative != other.negative,
            magnitude: self.magnitude.multiplied(&other.magnitude),
        }
    }

    #[must_use]
    pub fn scale_by_natural(&self, multiplicity: &ExactNatural) -> Self {
        if self.is_zero() || multiplicity.is_zero() {
            return Self::default();
        }
        Self {
            negative: self.negative,
            magnitude: self.magnitude.multiplied(multiplicity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_promotes_multiplies_and_formats_without_bound() {
        let left = ExactNatural::from_u128(u128::MAX);
        let right = ExactNatural::from_u128(u128::MAX);
        let product = left.multiplied(&right);
        assert_eq!(
            product.to_decimal_string(),
            "115792089237316195423570985008687907852589419931798687112530834793049593217025"
        );
    }

    #[test]
    fn integer_addition_cancels_exactly() {
        let magnitude = ExactNatural::from_u128(u128::MAX);
        let mut positive = ExactInteger::from_parts(false, magnitude.clone());
        positive.add_assign(&ExactInteger::from_parts(true, magnitude));
        assert!(positive.is_zero());
        assert!(!positive.is_negative());
    }

    #[test]
    fn integer_scaling_stays_one_exact_coefficient() {
        let coefficient = ExactInteger::from_i64(i64::MAX);
        let multiplicity = ExactNatural::from_u128(u128::MAX);
        let scaled = coefficient.scale_by_natural(&multiplicity);
        assert!(!scaled.is_zero());
        assert_eq!(scaled.magnitude().bit_len(), 191);
    }
}
