use std::sync::Arc;

use kernel_persistent::PersistentOrdMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKeyMassRetentionError {
    DuplicateRow,
    MissingRow,
    InconsistentDelta,
    CountOverflow,
}

/// Protected direct realizer for `{RowCanonicalKey, JointMass}`.
///
/// Unlike [`SharedRowKeyMassRetention`], the row route owns one canonical tuple per
/// row. This intentionally preserves the low-indirection direct lookup point
/// for low-reuse/small-key workloads while shared retention remains a separate
/// candidate physical engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectRowKeyMassRetention<RowId: Ord + Clone, Key: Ord + Clone> {
    joint_masses: PersistentOrdMap<Vec<Key>, usize>,
    row_routes: PersistentOrdMap<RowId, Vec<Key>>,
}

impl<RowId: Ord + Clone, Key: Ord + Clone> Default for DirectRowKeyMassRetention<RowId, Key> {
    fn default() -> Self {
        Self {
            joint_masses: PersistentOrdMap::default(),
            row_routes: PersistentOrdMap::default(),
        }
    }
}

impl<RowId: Ord + Clone, Key: Ord + Clone> DirectRowKeyMassRetention<RowId, Key> {
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_routes.len()
    }

    #[must_use]
    pub fn distinct_joint_key_count(&self) -> usize {
        self.joint_masses.len()
    }

    #[must_use]
    pub fn row_key(&self, row: &RowId) -> Option<&[Key]> {
        self.row_routes.get(row).map(Vec::as_slice)
    }

    #[must_use]
    pub fn joint_mass(&self, key: &[Key]) -> usize {
        self.joint_masses.get(key).copied().unwrap_or(0)
    }

    pub fn insert(&mut self, row: RowId, key: Vec<Key>) -> Result<(), RowKeyMassRetentionError> {
        if self.row_routes.contains_key(&row) {
            return Err(RowKeyMassRetentionError::DuplicateRow);
        }
        let next_mass = self
            .joint_masses
            .get(key.as_slice())
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(RowKeyMassRetentionError::CountOverflow)?;
        self.joint_masses.insert(key.clone(), next_mass);
        self.row_routes.insert(row, key);
        Ok(())
    }

    pub fn remove(
        &mut self,
        row: &RowId,
        expected_key: &[Key],
    ) -> Result<(), RowKeyMassRetentionError> {
        let retained_key = self
            .row_routes
            .get(row)
            .cloned()
            .ok_or(RowKeyMassRetentionError::MissingRow)?;
        if retained_key.as_slice() != expected_key {
            return Err(RowKeyMassRetentionError::InconsistentDelta);
        }
        let mass = self
            .joint_masses
            .get(&retained_key)
            .copied()
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?
            .checked_sub(1)
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        if mass == 0 {
            self.joint_masses.remove(&retained_key);
        } else {
            self.joint_masses.insert(retained_key, mass);
        }
        self.row_routes
            .remove(row)
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        Ok(())
    }

    /// Owned heap only; inline `Self` bytes are intentionally excluded.
    #[must_use]
    pub fn estimated_heap_bytes_with(&self, key_heap_bytes: impl Fn(&Key) -> usize) -> usize {
        let key_payload_bytes = self
            .joint_masses
            .keys()
            .chain(self.row_routes.values())
            .map(|key| {
                key.len()
                    .saturating_mul(std::mem::size_of::<Key>())
                    .saturating_add(
                        key.iter()
                            .map(&key_heap_bytes)
                            .fold(0_usize, usize::saturating_add),
                    )
            })
            .fold(0_usize, usize::saturating_add);
        self.joint_masses
            .estimated_heap_bytes()
            .saturating_add(self.row_routes.estimated_heap_bytes())
            .saturating_add(key_payload_bytes)
    }
}

/// Canonical joint-key interning plus exact live mass.
///
/// This is the shared payload owner for exact joint-key retention. Dependent
/// row routes may clone the returned `Arc` identity without duplicating the
/// canonical tuple payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalJointKeyPool<Key: Ord + Clone> {
    joint_masses: PersistentOrdMap<Arc<[Key]>, usize>,
    row_count: usize,
}

impl<Key: Ord + Clone> Default for CanonicalJointKeyPool<Key> {
    fn default() -> Self {
        Self {
            joint_masses: PersistentOrdMap::default(),
            row_count: 0,
        }
    }
}

impl<Key: Ord + Clone> CanonicalJointKeyPool<Key> {
    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    #[must_use]
    pub fn distinct_joint_key_count(&self) -> usize {
        self.joint_masses.len()
    }

    #[must_use]
    pub fn joint_mass(&self, key: &[Key]) -> usize {
        self.joint_masses.get(key).copied().unwrap_or(0)
    }

    pub fn retain(&mut self, key: Vec<Key>) -> Result<Arc<[Key]>, RowKeyMassRetentionError> {
        let (retained_key, mass) =
            if let Some((retained_key, mass)) = self.joint_masses.get_key_value(key.as_slice()) {
                (
                    Arc::clone(retained_key),
                    mass.checked_add(1)
                        .ok_or(RowKeyMassRetentionError::CountOverflow)?,
                )
            } else {
                (Arc::<[Key]>::from(key), 1)
            };
        self.row_count = self
            .row_count
            .checked_add(1)
            .ok_or(RowKeyMassRetentionError::CountOverflow)?;
        self.joint_masses.insert(Arc::clone(&retained_key), mass);
        Ok(retained_key)
    }

    pub fn release(&mut self, expected_key: &[Key]) -> Result<(), RowKeyMassRetentionError> {
        let (retained_key, mass) = self
            .joint_masses
            .get_key_value(expected_key)
            .map(|(key, mass)| (Arc::clone(key), *mass))
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        let next_mass = mass
            .checked_sub(1)
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        self.row_count = self
            .row_count
            .checked_sub(1)
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        if next_mass == 0 {
            self.joint_masses.remove(&retained_key);
        } else {
            self.joint_masses.insert(retained_key, next_mass);
        }
        Ok(())
    }

    /// Owned heap only; inline `Self` bytes are intentionally excluded.
    #[must_use]
    pub fn estimated_heap_bytes_with(&self, key_heap_bytes: impl Fn(&Key) -> usize) -> usize {
        let key_allocations = self
            .joint_masses
            .keys()
            .map(|key| {
                key.len()
                    .saturating_mul(std::mem::size_of::<Key>())
                    .saturating_add(
                        key.iter()
                            .map(&key_heap_bytes)
                            .fold(0_usize, usize::saturating_add),
                    )
            })
            .fold(0_usize, usize::saturating_add);
        self.joint_masses
            .estimated_heap_bytes()
            .saturating_add(key_allocations)
    }
}

/// Exact row -> canonical-joint-key route whose key payload is owned elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRowKeyRoute<RowId: Ord + Clone, Key: Ord + Clone> {
    row_routes: PersistentOrdMap<RowId, Arc<[Key]>>,
}

impl<RowId: Ord + Clone, Key: Ord + Clone> Default for SharedRowKeyRoute<RowId, Key> {
    fn default() -> Self {
        Self {
            row_routes: PersistentOrdMap::default(),
        }
    }
}

impl<RowId: Ord + Clone, Key: Ord + Clone> SharedRowKeyRoute<RowId, Key> {
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_routes.len()
    }

    #[must_use]
    pub fn row_key(&self, row: &RowId) -> Option<&[Key]> {
        self.row_routes.get(row).map(AsRef::as_ref)
    }

    pub fn insert(
        &mut self,
        row: RowId,
        retained_key: Arc<[Key]>,
    ) -> Result<(), RowKeyMassRetentionError> {
        if self.row_routes.contains_key(&row) {
            return Err(RowKeyMassRetentionError::DuplicateRow);
        }
        self.row_routes.insert(row, retained_key);
        Ok(())
    }

    pub fn remove(
        &mut self,
        row: &RowId,
        expected_key: &[Key],
    ) -> Result<(), RowKeyMassRetentionError> {
        let retained_key = self
            .row_routes
            .get(row)
            .ok_or(RowKeyMassRetentionError::MissingRow)?;
        if retained_key.as_ref() != expected_key {
            return Err(RowKeyMassRetentionError::InconsistentDelta);
        }
        self.row_routes
            .remove(row)
            .ok_or(RowKeyMassRetentionError::InconsistentDelta)?;
        Ok(())
    }

    #[must_use]
    pub fn estimated_heap_bytes(&self) -> usize {
        self.row_routes.estimated_heap_bytes()
    }
}

/// Exact retained realizer for `{RowCanonicalKey, JointMass}` of one canonical map `kappa`.
///
/// The implementation is deliberately decomposed into a canonical key/mass pool
/// plus an independent shared row route. This exposes the production resource
/// atoms required by retention synthesis while preserving the existing API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRowKeyMassRetention<RowId: Ord + Clone, Key: Ord + Clone> {
    pool: CanonicalJointKeyPool<Key>,
    row_route: SharedRowKeyRoute<RowId, Key>,
}

impl<RowId: Ord + Clone, Key: Ord + Clone> Default for SharedRowKeyMassRetention<RowId, Key> {
    fn default() -> Self {
        Self {
            pool: CanonicalJointKeyPool::default(),
            row_route: SharedRowKeyRoute::default(),
        }
    }
}

impl<RowId: Ord + Clone, Key: Ord + Clone> SharedRowKeyMassRetention<RowId, Key> {
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_route.row_count()
    }

    #[must_use]
    pub fn distinct_joint_key_count(&self) -> usize {
        self.pool.distinct_joint_key_count()
    }

    #[must_use]
    pub fn row_key(&self, row: &RowId) -> Option<&[Key]> {
        self.row_route.row_key(row)
    }

    #[must_use]
    pub fn joint_mass(&self, key: &[Key]) -> usize {
        self.pool.joint_mass(key)
    }

    pub fn insert(&mut self, row: RowId, key: Vec<Key>) -> Result<(), RowKeyMassRetentionError> {
        if self.row_route.row_key(&row).is_some() {
            return Err(RowKeyMassRetentionError::DuplicateRow);
        }
        let retained_key = self.pool.retain(key)?;
        if let Err(error) = self.row_route.insert(row, Arc::clone(&retained_key)) {
            self.pool.release(&retained_key)?;
            return Err(error);
        }
        Ok(())
    }

    pub fn remove(
        &mut self,
        row: &RowId,
        expected_key: &[Key],
    ) -> Result<(), RowKeyMassRetentionError> {
        self.row_route.remove(row, expected_key)?;
        self.pool.release(expected_key)
    }

    /// Owned heap only; inline `Self` bytes are intentionally excluded.
    #[must_use]
    pub fn estimated_heap_bytes_with(&self, key_heap_bytes: impl Fn(&Key) -> usize) -> usize {
        self.pool
            .estimated_heap_bytes_with(key_heap_bytes)
            .saturating_add(self.row_route.estimated_heap_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_row_route_preserves_owned_fast_path_semantics() {
        let mut retained = DirectRowKeyMassRetention::<u32, String>::default();
        retained.insert(1, vec!["a".into()]).unwrap();
        retained.insert(2, vec!["a".into()]).unwrap();
        assert_eq!(retained.joint_mass(&["a".into()]), 2);
        assert_eq!(retained.row_key(&2), Some(["a".to_owned()].as_slice()));
        retained.remove(&1, &["a".into()]).unwrap();
        assert_eq!(retained.joint_mass(&["a".into()]), 1);
    }

    #[test]
    fn row_route_and_joint_mass_share_one_exact_transition_law() {
        let mut retained = SharedRowKeyMassRetention::<u32, String>::default();
        retained.insert(1, vec!["a".into()]).unwrap();
        retained.insert(2, vec!["a".into()]).unwrap();
        retained.insert(3, vec!["b".into()]).unwrap();
        assert_eq!(retained.joint_mass(&["a".into()]), 2);
        assert_eq!(retained.row_key(&2), Some(["a".to_owned()].as_slice()));
        retained.remove(&2, &["a".into()]).unwrap();
        assert_eq!(retained.joint_mass(&["a".into()]), 1);
        retained.remove(&1, &["a".into()]).unwrap();
        assert_eq!(retained.joint_mass(&["a".into()]), 0);
        assert_eq!(retained.distinct_joint_key_count(), 1);
    }

    #[test]
    fn canonical_pool_is_a_standalone_joint_mass_realizer() {
        let mut pool = CanonicalJointKeyPool::<String>::default();
        let first = pool.retain(vec!["shared".into()]).unwrap();
        let second = pool.retain(vec!["shared".into()]).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(pool.row_count(), 2);
        assert_eq!(pool.joint_mass(&["shared".into()]), 2);
        pool.release(&["shared".into()]).unwrap();
        assert_eq!(pool.joint_mass(&["shared".into()]), 1);
    }

    #[test]
    fn row_route_does_not_own_canonical_payload() {
        let mut pool = CanonicalJointKeyPool::<String>::default();
        let retained_key = pool.retain(vec!["x".repeat(256)]).unwrap();
        let mut route = SharedRowKeyRoute::<u32, String>::default();
        route.insert(7, Arc::clone(&retained_key)).unwrap();
        assert_eq!(route.row_key(&7), Some(retained_key.as_ref()));
        assert!(Arc::strong_count(&retained_key) >= 3);
    }
}
