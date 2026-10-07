use std::sync::Arc;

use kernel_persistent::{PersistentOrdMap, PersistentOrdSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FiniteFiberProfile {
    Measure,
    ExactFibers,
    ProjectedFibers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiniteFiberCarrierError {
    CoordinateArityMismatch,
    ClassIdOverflow,
    DuplicateRow,
    MissingRow,
    InconsistentDelta,
}

type JointClassKey = Arc<[u32]>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum RetainedFibers<RowId: Ord + Clone> {
    Measure {
        counts: PersistentOrdMap<JointClassKey, usize>,
    },
    Exact {
        fibers: PersistentOrdMap<JointClassKey, PersistentOrdSet<RowId>>,
        reverse: PersistentOrdMap<RowId, JointClassKey>,
    },
    Projected {
        fibers: PersistentOrdMap<JointClassKey, PersistentOrdSet<RowId>>,
        reverse: PersistentOrdMap<RowId, JointClassKey>,
        projected: Vec<PersistentOrdMap<u32, PersistentOrdSet<JointClassKey>>>,
    },
}

/// Finite retained materialization of one canonical map `kappa : X -> K1 x ... x Kn`.
///
/// Canonicalization is deliberately external. The carrier receives already-canonical
/// coordinate keys and interns each coordinate class exactly once per slot. Profiles
/// retain successively stronger derived indexes over the same `kappa` transition law.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteFiberCarrier<RowId: Ord + Clone, Key: Ord + Clone> {
    profile: FiniteFiberProfile,
    coordinate_classes: Vec<PersistentOrdMap<Key, u32>>,
    coordinate_masses: Vec<PersistentOrdMap<u32, usize>>,
    next_class: Vec<u32>,
    retained: RetainedFibers<RowId>,
    row_count: usize,
}

impl<RowId: Ord + Clone, Key: Ord + Clone> FiniteFiberCarrier<RowId, Key> {
    #[must_use]
    pub fn new(profile: FiniteFiberProfile, coordinate_count: usize) -> Self {
        let retained = match profile {
            FiniteFiberProfile::Measure => RetainedFibers::Measure {
                counts: PersistentOrdMap::default(),
            },
            FiniteFiberProfile::ExactFibers => RetainedFibers::Exact {
                fibers: PersistentOrdMap::default(),
                reverse: PersistentOrdMap::default(),
            },
            FiniteFiberProfile::ProjectedFibers => RetainedFibers::Projected {
                fibers: PersistentOrdMap::default(),
                reverse: PersistentOrdMap::default(),
                projected: vec![PersistentOrdMap::default(); coordinate_count],
            },
        };
        Self {
            profile,
            coordinate_classes: vec![PersistentOrdMap::default(); coordinate_count],
            coordinate_masses: vec![PersistentOrdMap::default(); coordinate_count],
            next_class: vec![0; coordinate_count],
            retained,
            row_count: 0,
        }
    }

    #[must_use]
    pub const fn profile(&self) -> FiniteFiberProfile {
        self.profile
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    #[must_use]
    pub fn distinct_joint_key_count(&self) -> usize {
        match &self.retained {
            RetainedFibers::Measure { counts } => counts.len(),
            RetainedFibers::Exact { fibers, .. } | RetainedFibers::Projected { fibers, .. } => {
                fibers.len()
            }
        }
    }

    #[must_use]
    pub fn coordinate_class_count(&self, slot: usize) -> Option<usize> {
        self.coordinate_classes.get(slot).map(PersistentOrdMap::len)
    }

    fn intern_signature(&mut self, keys: &[Key]) -> Result<JointClassKey, FiniteFiberCarrierError> {
        if keys.len() != self.coordinate_classes.len() {
            return Err(FiniteFiberCarrierError::CoordinateArityMismatch);
        }
        let mut classes = Vec::with_capacity(keys.len());
        for (slot, key) in keys.iter().enumerate() {
            let class = if let Some(class) = self.coordinate_classes[slot].get(key).copied() {
                class
            } else {
                let class = self.next_class[slot];
                self.next_class[slot] = class
                    .checked_add(1)
                    .ok_or(FiniteFiberCarrierError::ClassIdOverflow)?;
                self.coordinate_classes[slot].insert(key.clone(), class);
                class
            };
            let mass = self.coordinate_masses[slot]
                .get(&class)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(FiniteFiberCarrierError::ClassIdOverflow)?;
            self.coordinate_masses[slot].insert(class, mass);
            classes.push(class);
        }
        Ok(Arc::from(classes))
    }

    fn lookup_signature(&self, keys: &[Key]) -> Result<JointClassKey, FiniteFiberCarrierError> {
        if keys.len() != self.coordinate_classes.len() {
            return Err(FiniteFiberCarrierError::CoordinateArityMismatch);
        }
        keys.iter()
            .enumerate()
            .map(|(slot, key)| {
                self.coordinate_classes[slot]
                    .get(key)
                    .copied()
                    .ok_or(FiniteFiberCarrierError::InconsistentDelta)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Arc::from)
    }

    fn release_signature_classes(
        &mut self,
        keys: &[Key],
        signature: &[u32],
    ) -> Result<(), FiniteFiberCarrierError> {
        for (slot, (key, class)) in keys.iter().zip(signature).enumerate() {
            let mass = self.coordinate_masses[slot]
                .get(class)
                .copied()
                .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
            if mass == 1 {
                self.coordinate_masses[slot].remove(class);
                let removed = self.coordinate_classes[slot].remove(key);
                if removed != Some(*class) {
                    return Err(FiniteFiberCarrierError::InconsistentDelta);
                }
            } else {
                self.coordinate_masses[slot].insert(*class, mass - 1);
            }
        }
        Ok(())
    }

    pub fn insert(&mut self, row: RowId, keys: &[Key]) -> Result<(), FiniteFiberCarrierError> {
        match &self.retained {
            RetainedFibers::Exact { reverse, .. } | RetainedFibers::Projected { reverse, .. }
                if reverse.contains_key(&row) =>
            {
                return Err(FiniteFiberCarrierError::DuplicateRow);
            }
            _ => {}
        }
        let signature = self.intern_signature(keys)?;
        match &mut self.retained {
            RetainedFibers::Measure { counts } => {
                let count = counts.get(&signature).copied().unwrap_or(0) + 1;
                counts.insert(signature, count);
            }
            RetainedFibers::Exact { fibers, reverse } => {
                let mut fiber = fibers.get(&signature).cloned().unwrap_or_default();
                if !fiber.insert(row.clone()) {
                    return Err(FiniteFiberCarrierError::DuplicateRow);
                }
                fibers.insert(Arc::clone(&signature), fiber);
                reverse.insert(row, signature);
            }
            RetainedFibers::Projected {
                fibers,
                reverse,
                projected,
            } => {
                let mut fiber = fibers.get(&signature).cloned().unwrap_or_default();
                let new_joint_fiber = fiber.is_empty();
                if !fiber.insert(row.clone()) {
                    return Err(FiniteFiberCarrierError::DuplicateRow);
                }
                fibers.insert(Arc::clone(&signature), fiber);
                if new_joint_fiber {
                    for (slot, class) in signature.iter().copied().enumerate() {
                        let mut projected_joints =
                            projected[slot].get(&class).cloned().unwrap_or_default();
                        projected_joints.insert(Arc::clone(&signature));
                        projected[slot].insert(class, projected_joints);
                    }
                }
                reverse.insert(row, signature);
            }
        }
        self.row_count += 1;
        Ok(())
    }

    pub fn remove(&mut self, row: &RowId, keys: &[Key]) -> Result<(), FiniteFiberCarrierError> {
        let signature = self.lookup_signature(keys)?;
        match &mut self.retained {
            RetainedFibers::Measure { counts } => {
                let count = counts
                    .get(&signature)
                    .copied()
                    .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
                if count == 1 {
                    counts.remove(&signature);
                } else {
                    counts.insert(Arc::clone(&signature), count - 1);
                }
            }
            RetainedFibers::Exact { fibers, reverse } => {
                if reverse.get(row) != Some(&signature) {
                    return Err(FiniteFiberCarrierError::MissingRow);
                }
                let mut fiber = fibers
                    .get(&signature)
                    .cloned()
                    .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
                if !fiber.remove(row) {
                    return Err(FiniteFiberCarrierError::InconsistentDelta);
                }
                if fiber.is_empty() {
                    fibers.remove(&signature);
                } else {
                    fibers.insert(Arc::clone(&signature), fiber);
                }
                reverse.remove(row);
            }
            RetainedFibers::Projected {
                fibers,
                reverse,
                projected,
            } => {
                if reverse.get(row) != Some(&signature) {
                    return Err(FiniteFiberCarrierError::MissingRow);
                }
                let mut fiber = fibers
                    .get(&signature)
                    .cloned()
                    .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
                if !fiber.remove(row) {
                    return Err(FiniteFiberCarrierError::InconsistentDelta);
                }
                let joint_fiber_removed = fiber.is_empty();
                if joint_fiber_removed {
                    fibers.remove(&signature);
                } else {
                    fibers.insert(Arc::clone(&signature), fiber);
                }
                if joint_fiber_removed {
                    for (slot, class) in signature.iter().copied().enumerate() {
                        let mut projected_joints = projected[slot]
                            .get(&class)
                            .cloned()
                            .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
                        if !projected_joints.remove(&signature) {
                            return Err(FiniteFiberCarrierError::InconsistentDelta);
                        }
                        if projected_joints.is_empty() {
                            projected[slot].remove(&class);
                        } else {
                            projected[slot].insert(class, projected_joints);
                        }
                    }
                }
                reverse.remove(row);
            }
        }
        self.release_signature_classes(keys, &signature)?;
        self.row_count = self
            .row_count
            .checked_sub(1)
            .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
        Ok(())
    }

    pub fn joint_count(&self, keys: &[Key]) -> Result<usize, FiniteFiberCarrierError> {
        let signature = match self.lookup_signature(keys) {
            Ok(signature) => signature,
            Err(FiniteFiberCarrierError::InconsistentDelta) => return Ok(0),
            Err(error) => return Err(error),
        };
        Ok(match &self.retained {
            RetainedFibers::Measure { counts } => counts.get(&signature).copied().unwrap_or(0),
            RetainedFibers::Exact { fibers, .. } | RetainedFibers::Projected { fibers, .. } => {
                fibers.get(&signature).map_or(0, PersistentOrdSet::len)
            }
        })
    }

    pub fn joint_fiber(
        &self,
        keys: &[Key],
    ) -> Result<Option<&PersistentOrdSet<RowId>>, FiniteFiberCarrierError> {
        let signature = match self.lookup_signature(keys) {
            Ok(signature) => signature,
            Err(FiniteFiberCarrierError::InconsistentDelta) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(match &self.retained {
            RetainedFibers::Measure { .. } => None,
            RetainedFibers::Exact { fibers, .. } | RetainedFibers::Projected { fibers, .. } => {
                fibers.get(&signature)
            }
        })
    }

    #[must_use]
    pub fn estimated_retained_bytes_with(&self, key_heap_bytes: impl Fn(&Key) -> usize) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(
                self.coordinate_classes
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PersistentOrdMap<Key, u32>>()),
            )
            .saturating_add(
                self.coordinate_masses
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PersistentOrdMap<u32, usize>>()),
            )
            .saturating_add(
                self.next_class
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            );
        for classes in &self.coordinate_classes {
            bytes = bytes.saturating_add(classes.estimated_heap_bytes());
            bytes = bytes.saturating_add(
                classes
                    .keys()
                    .map(&key_heap_bytes)
                    .fold(0_usize, usize::saturating_add),
            );
        }
        bytes = bytes.saturating_add(
            self.coordinate_masses
                .iter()
                .map(PersistentOrdMap::estimated_heap_bytes)
                .fold(0_usize, usize::saturating_add),
        );
        let joint_allocation =
            |signature: &JointClassKey| signature.len().saturating_mul(std::mem::size_of::<u32>());
        match &self.retained {
            RetainedFibers::Measure { counts } => {
                bytes = bytes.saturating_add(counts.estimated_heap_bytes());
                bytes = bytes.saturating_add(
                    counts
                        .keys()
                        .map(joint_allocation)
                        .fold(0_usize, usize::saturating_add),
                );
            }
            RetainedFibers::Exact { fibers, reverse } => {
                bytes = bytes
                    .saturating_add(fibers.estimated_heap_bytes())
                    .saturating_add(reverse.estimated_heap_bytes());
                for (signature, rows) in fibers {
                    bytes = bytes
                        .saturating_add(joint_allocation(signature))
                        .saturating_add(rows.estimated_heap_bytes());
                }
            }
            RetainedFibers::Projected {
                fibers,
                reverse,
                projected,
            } => {
                bytes = bytes
                    .saturating_add(fibers.estimated_heap_bytes())
                    .saturating_add(reverse.estimated_heap_bytes())
                    .saturating_add(projected.capacity().saturating_mul(std::mem::size_of::<
                        PersistentOrdMap<u32, PersistentOrdSet<JointClassKey>>,
                    >()));
                for (signature, rows) in fibers {
                    bytes = bytes
                        .saturating_add(joint_allocation(signature))
                        .saturating_add(rows.estimated_heap_bytes());
                }
                for projection in projected {
                    bytes = bytes.saturating_add(projection.estimated_heap_bytes());
                    bytes = bytes.saturating_add(
                        projection
                            .values()
                            .map(PersistentOrdSet::estimated_heap_bytes)
                            .fold(0_usize, usize::saturating_add),
                    );
                }
            }
        }
        bytes
    }

    pub fn projected_rows(
        &self,
        slot: usize,
        key: &Key,
    ) -> Result<Vec<RowId>, FiniteFiberCarrierError> {
        let Some(classes) = self.coordinate_classes.get(slot) else {
            return Err(FiniteFiberCarrierError::CoordinateArityMismatch);
        };
        let Some(class) = classes.get(key) else {
            return Ok(Vec::new());
        };
        let RetainedFibers::Projected {
            fibers, projected, ..
        } = &self.retained
        else {
            return Ok(Vec::new());
        };
        let Some(joints) = projected[slot].get(class) else {
            return Ok(Vec::new());
        };
        let mut rows = Vec::new();
        for joint in joints.iter() {
            let fiber = fibers
                .get(joint)
                .ok_or(FiniteFiberCarrierError::InconsistentDelta)?;
            rows.extend(fiber.iter().cloned());
        }
        rows.sort();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carriers() -> [FiniteFiberCarrier<u32, u32>; 3] {
        [
            FiniteFiberCarrier::new(FiniteFiberProfile::Measure, 2),
            FiniteFiberCarrier::new(FiniteFiberProfile::ExactFibers, 2),
            FiniteFiberCarrier::new(FiniteFiberProfile::ProjectedFibers, 2),
        ]
    }

    #[test]
    fn forgetful_levels_agree_after_insert_and_remove() {
        let mut carriers = carriers();
        let rows = [(1, [10, 20]), (2, [10, 21]), (3, [11, 20]), (4, [10, 20])];
        for (row, key) in rows {
            for carrier in &mut carriers {
                carrier.insert(row, &key).unwrap();
            }
        }
        for key in [[10, 20], [10, 21], [11, 20], [99, 99]] {
            let expected = carriers[0].joint_count(&key).unwrap();
            assert_eq!(carriers[1].joint_count(&key).unwrap(), expected);
            assert_eq!(carriers[2].joint_count(&key).unwrap(), expected);
        }
        assert_eq!(carriers[2].projected_rows(0, &10).unwrap().len(), 3);
        for carrier in &mut carriers {
            carrier.remove(&2, &[10, 21]).unwrap();
        }
        assert_eq!(carriers[0].joint_count(&[10, 21]).unwrap(), 0);
        assert_eq!(carriers[1].joint_count(&[10, 21]).unwrap(), 0);
        assert_eq!(carriers[2].joint_count(&[10, 21]).unwrap(), 0);
        assert_eq!(carriers[2].coordinate_class_count(1), Some(1));
    }

    #[test]
    fn finite_hostile_state_space_preserves_forgetful_laws() {
        let universe = [
            (0_u32, [0_u32, 0_u32]),
            (1, [0, 1]),
            (2, [1, 0]),
            (3, [1, 1]),
        ];
        for mask in 0_u32..(1 << universe.len()) {
            let mut carriers = carriers();
            for (index, (row, key)) in universe.iter().enumerate() {
                if mask & (1 << index) != 0 {
                    for carrier in &mut carriers {
                        carrier.insert(*row, key).unwrap();
                    }
                }
            }
            for key in [[0, 0], [0, 1], [1, 0], [1, 1]] {
                let measure = carriers[0].joint_count(&key).unwrap();
                assert_eq!(carriers[1].joint_count(&key).unwrap(), measure);
                assert_eq!(carriers[2].joint_count(&key).unwrap(), measure);
            }
            for (index, (row, key)) in universe.iter().enumerate() {
                if mask & (1 << index) != 0 {
                    for carrier in &mut carriers {
                        carrier.remove(row, key).unwrap();
                    }
                    for probe in [[0, 0], [0, 1], [1, 0], [1, 1]] {
                        let measure = carriers[0].joint_count(&probe).unwrap();
                        assert_eq!(carriers[1].joint_count(&probe).unwrap(), measure);
                        assert_eq!(carriers[2].joint_count(&probe).unwrap(), measure);
                    }
                }
            }
        }
    }
}
