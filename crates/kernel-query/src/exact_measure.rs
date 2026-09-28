use super::RelQueryError;

pub(super) fn apply_exact_to_natural(
    value: &mut kernel_exact::ExactNatural,
    weight: &kernel_exact::ExactInteger,
) -> Result<(), RelQueryError> {
    if weight.is_negative() {
        if !value.checked_sub_assign(weight.magnitude()) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
    } else {
        value.add_assign(weight.magnitude());
    }
    Ok(())
}

pub(super) fn exact_natural_difference(
    after: &kernel_exact::ExactNatural,
    before: &kernel_exact::ExactNatural,
) -> kernel_exact::ExactInteger {
    match after.cmp(before) {
        std::cmp::Ordering::Equal => kernel_exact::ExactInteger::default(),
        std::cmp::Ordering::Greater => {
            let mut magnitude = after.clone();
            let subtracted = magnitude.checked_sub_assign(before);
            debug_assert!(subtracted);
            kernel_exact::ExactInteger::from_parts(false, magnitude)
        }
        std::cmp::Ordering::Less => {
            let mut magnitude = before.clone();
            let subtracted = magnitude.checked_sub_assign(after);
            debug_assert!(subtracted);
            kernel_exact::ExactInteger::from_parts(true, magnitude)
        }
    }
}

#[cfg(test)]
pub(super) fn exact_integer_to_i64(weight: &kernel_exact::ExactInteger) -> Option<i64> {
    let magnitude = weight.magnitude().to_u64()?;
    if weight.is_negative() {
        if magnitude == (1_u64 << 63) {
            Some(i64::MIN)
        } else {
            i64::try_from(magnitude).ok().map(|value| -value)
        }
    } else {
        i64::try_from(magnitude).ok()
    }
}
