use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationMeasureError {
    MultiplicityOverflow,
    MissingWitness,
    RetractionExceedsMass,
}

/// Finite non-negative measure over exact violation witnesses.
///
/// Zero mass is exactly validity for the represented invariant. This value is
/// reconstructible derived state; callers must never treat it as semantic
/// authority independently of the revision and invariant recipe that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViolationMeasure<W> {
    masses: BTreeMap<W, u64>,
}

impl<W: Ord> ViolationMeasure<W> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            masses: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.masses.is_empty()
    }

    #[must_use]
    pub fn witness_count(&self) -> usize {
        self.masses.len()
    }

    #[must_use]
    pub fn mass(&self, witness: &W) -> u64 {
        self.masses.get(witness).copied().unwrap_or(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&W, u64)> {
        self.masses.iter().map(|(witness, &mass)| (witness, mass))
    }

    pub fn add(&mut self, witness: W, mass: u64) -> Result<(), ViolationMeasureError> {
        if mass == 0 {
            return Ok(());
        }
        let next = self
            .mass(&witness)
            .checked_add(mass)
            .ok_or(ViolationMeasureError::MultiplicityOverflow)?;
        self.masses.insert(witness, next);
        Ok(())
    }

    pub fn retract(&mut self, witness: &W, mass: u64) -> Result<(), ViolationMeasureError> {
        if mass == 0 {
            return Ok(());
        }
        let current = self
            .masses
            .get(witness)
            .copied()
            .ok_or(ViolationMeasureError::MissingWitness)?;
        if mass > current {
            return Err(ViolationMeasureError::RetractionExceedsMass);
        }
        if mass == current {
            self.masses.remove(witness);
        } else {
            *self
                .masses
                .get_mut(witness)
                .expect("witness existence was checked above") = current - mass;
        }
        Ok(())
    }
}

/// Namespace-preserving witness used for the direct sum of independent
/// invariant measures. Tagged witnesses cannot cancel because all masses are
/// non-negative and tags are part of identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaggedViolationWitness<Tag, W> {
    pub tag: Tag,
    pub witness: W,
}

pub fn tagged_direct_sum<Tag, W>(
    measures: impl IntoIterator<Item = (Tag, ViolationMeasure<W>)>,
) -> Result<ViolationMeasure<TaggedViolationWitness<Tag, W>>, ViolationMeasureError>
where
    Tag: Ord + Clone,
    W: Ord,
{
    let mut out = ViolationMeasure::new();
    for (tag, measure) in measures {
        for (witness, mass) in measure.masses {
            out.add(
                TaggedViolationWitness {
                    tag: tag.clone(),
                    witness,
                },
                mass,
            )?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_non_negative_measure_has_exact_zero_law() {
        let mut measure = ViolationMeasure::new();
        assert!(measure.is_zero());
        measure.add("a", 2).unwrap();
        measure.add("b", 1).unwrap();
        assert_eq!(measure.mass(&"a"), 2);
        assert!(!measure.is_zero());
        measure.retract(&"a", 2).unwrap();
        measure.retract(&"b", 1).unwrap();
        assert!(measure.is_zero());
    }

    #[test]
    fn tagged_direct_sum_preserves_equal_witnesses_from_distinct_invariants() {
        let mut left = ViolationMeasure::new();
        left.add(7, 1).unwrap();
        let mut right = ViolationMeasure::new();
        right.add(7, 3).unwrap();
        let sum = tagged_direct_sum([("left", left), ("right", right)]).unwrap();
        assert_eq!(sum.witness_count(), 2);
        assert_eq!(
            sum.mass(&TaggedViolationWitness {
                tag: "left",
                witness: 7
            }),
            1
        );
        assert_eq!(
            sum.mass(&TaggedViolationWitness {
                tag: "right",
                witness: 7
            }),
            3
        );
    }
}
