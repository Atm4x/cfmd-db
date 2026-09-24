use std::collections::{BTreeMap, BTreeSet};

use kernel_types::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalEntityId(u32);

impl LocalEntityId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenseIdentityError {
    CapacityExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DenseEntitySet {
    words: Vec<u64>,
}

impl DenseEntitySet {
    #[must_use]
    pub fn with_capacity(entity_capacity: usize) -> Self {
        Self {
            words: vec![0; entity_capacity.div_ceil(64)],
        }
    }

    pub fn insert(&mut self, entity: LocalEntityId) {
        let word = entity.index() / 64;
        let bit = entity.index() % 64;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1_u64 << bit;
    }

    pub fn remove(&mut self, entity: LocalEntityId) -> bool {
        let word = entity.index() / 64;
        let bit = entity.index() % 64;
        let Some(bits) = self.words.get_mut(word) else {
            return false;
        };
        let mask = 1_u64 << bit;
        let existed = *bits & mask != 0;
        *bits &= !mask;
        existed
    }

    #[must_use]
    pub fn contains(&self, entity: LocalEntityId) -> bool {
        let word = entity.index() / 64;
        let bit = entity.index() % 64;
        self.words
            .get(word)
            .is_some_and(|bits| bits & (1_u64 << bit) != 0)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub fn iter(&self) -> impl Iterator<Item = LocalEntityId> + '_ {
        self.words
            .iter()
            .enumerate()
            .flat_map(|(word_index, word)| {
                let word = *word;
                (0_usize..64)
                    .filter(move |&bit| word & (1_u64 << bit) != 0)
                    .map(move |bit| {
                        let index = word_index * 64 + bit;
                        LocalEntityId(
                            u32::try_from(index).expect("dense entity set index fits u32"),
                        )
                    })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DenseEntityIds {
    external_by_local: Vec<EntityId>,
    local_by_external: BTreeMap<EntityId, LocalEntityId>,
}

impl DenseEntityIds {
    pub fn compile(entities: &BTreeSet<EntityId>) -> Result<Self, DenseIdentityError> {
        if entities.len() > u32::MAX as usize {
            return Err(DenseIdentityError::CapacityExceeded);
        }
        let external_by_local = entities.iter().copied().collect::<Vec<_>>();
        let mut local_by_external = BTreeMap::new();
        for (index, external) in external_by_local.iter().copied().enumerate() {
            let index = u32::try_from(index).map_err(|_| DenseIdentityError::CapacityExceeded)?;
            local_by_external.insert(external, LocalEntityId(index));
        }
        Ok(Self {
            external_by_local,
            local_by_external,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.external_by_local.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.external_by_local.is_empty()
    }

    /// Deterministic retained-size estimate for physical budgeting.
    /// `BTree` allocator/node overhead is intentionally not part of this value.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.external_by_local
                    .capacity()
                    .saturating_mul(std::mem::size_of::<EntityId>()),
            )
            .saturating_add(
                self.local_by_external
                    .len()
                    .saturating_mul(std::mem::size_of::<(EntityId, LocalEntityId)>()),
            )
    }

    #[must_use]
    pub fn local(&self, external: EntityId) -> Option<LocalEntityId> {
        self.local_by_external.get(&external).copied()
    }

    #[must_use]
    pub fn external(&self, local: LocalEntityId) -> Option<EntityId> {
        self.external_by_local.get(local.index()).copied()
    }

    #[must_use]
    pub fn external_at(&self, index: usize) -> Option<EntityId> {
        self.external_by_local.get(index).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityTransport {
    forward: BTreeMap<EntityId, EntityId>,
    backward: BTreeMap<EntityId, EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityTransportError {
    SourceCoverageMismatch,
    TargetCoverageMismatch,
    NotInjective,
    CompositionDomainMismatch,
}

impl IdentityTransport {
    #[must_use]
    pub fn identity(ids: &BTreeSet<EntityId>) -> Self {
        let forward = ids.iter().copied().map(|id| (id, id)).collect();
        Self {
            forward,
            backward: ids.iter().copied().map(|id| (id, id)).collect(),
        }
    }

    pub fn new(
        source: &BTreeSet<EntityId>,
        target: &BTreeSet<EntityId>,
        forward: BTreeMap<EntityId, EntityId>,
    ) -> Result<Self, IdentityTransportError> {
        if forward.keys().copied().collect::<BTreeSet<_>>() != *source {
            return Err(IdentityTransportError::SourceCoverageMismatch);
        }

        let image: BTreeSet<_> = forward.values().copied().collect();
        if image.len() != forward.len() {
            return Err(IdentityTransportError::NotInjective);
        }
        if image != *target {
            return Err(IdentityTransportError::TargetCoverageMismatch);
        }

        let backward = forward.iter().map(|(&a, &b)| (b, a)).collect();
        Ok(Self { forward, backward })
    }

    #[must_use]
    pub fn transport(&self, source: EntityId) -> Option<EntityId> {
        self.forward.get(&source).copied()
    }

    #[must_use]
    pub fn source_ids(&self) -> BTreeSet<EntityId> {
        self.forward.keys().copied().collect()
    }

    #[must_use]
    pub fn target_ids(&self) -> BTreeSet<EntityId> {
        self.backward.keys().copied().collect()
    }

    #[must_use]
    pub fn inverse(&self) -> Self {
        Self {
            forward: self.backward.clone(),
            backward: self.forward.clone(),
        }
    }

    pub fn then(&self, next: &Self) -> Result<Self, IdentityTransportError> {
        let self_target: BTreeSet<_> = self.backward.keys().copied().collect();
        let next_source: BTreeSet<_> = next.forward.keys().copied().collect();
        if self_target != next_source {
            return Err(IdentityTransportError::CompositionDomainMismatch);
        }

        let source: BTreeSet<_> = self.forward.keys().copied().collect();
        let target: BTreeSet<_> = next.backward.keys().copied().collect();
        let mut composed = BTreeMap::new();
        for (&old, &middle) in &self.forward {
            let new = next
                .forward
                .get(&middle)
                .copied()
                .ok_or(IdentityTransportError::CompositionDomainMismatch)?;
            composed.insert(old, new);
        }
        Self::new(&source, &target, composed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    #[test]
    fn dense_entity_set_is_a_compact_exact_local_id_extent() {
        let entities = BTreeSet::from([id(10), id(20), id(30), id(40)]);
        let dense = DenseEntityIds::compile(&entities).unwrap();
        let mut extent = DenseEntitySet::with_capacity(dense.len());
        extent.insert(dense.local(id(10)).unwrap());
        extent.insert(dense.local(id(30)).unwrap());
        assert!(extent.contains(dense.local(id(10)).unwrap()));
        assert!(!extent.contains(dense.local(id(20)).unwrap()));
        assert!(extent.contains(dense.local(id(30)).unwrap()));
        assert_eq!(extent.len(), 2);
    }

    #[test]
    fn dense_entity_ids_round_trip_external_identity_without_reusing_semantics() {
        let entities = BTreeSet::from([id(30), id(10), id(20)]);
        let dense = DenseEntityIds::compile(&entities).unwrap();
        assert_eq!(dense.len(), 3);
        assert_eq!(dense.local(id(10)).unwrap().index(), 0);
        assert_eq!(dense.local(id(20)).unwrap().index(), 1);
        assert_eq!(dense.external(dense.local(id(30)).unwrap()), Some(id(30)));
        assert_eq!(dense.local(id(99)), None);
    }

    #[test]
    fn bijective_transports_form_a_groupoid_under_composition() {
        let a = BTreeSet::from([id(1), id(2)]);
        let b = BTreeSet::from([id(11), id(12)]);
        let c = BTreeSet::from([id(21), id(22)]);
        let ab = IdentityTransport::new(&a, &b, BTreeMap::from([(id(1), id(11)), (id(2), id(12))]))
            .unwrap();
        let bc =
            IdentityTransport::new(&b, &c, BTreeMap::from([(id(11), id(22)), (id(12), id(21))]))
                .unwrap();
        let ac = ab.then(&bc).unwrap();

        assert_eq!(ac.transport(id(1)), Some(id(22)));
        assert_eq!(ac.inverse().transport(id(22)), Some(id(1)));
    }

    #[test]
    fn split_is_not_identity_transport() {
        let source = BTreeSet::from([id(1)]);
        let target = BTreeSet::from([id(10), id(11)]);
        let attempt = IdentityTransport::new(&source, &target, BTreeMap::from([(id(1), id(10))]));
        assert_eq!(attempt, Err(IdentityTransportError::TargetCoverageMismatch));
    }

    #[test]
    fn merge_is_not_identity_transport() {
        let source = BTreeSet::from([id(1), id(2)]);
        let target = BTreeSet::from([id(10)]);
        let attempt = IdentityTransport::new(
            &source,
            &target,
            BTreeMap::from([(id(1), id(10)), (id(2), id(10))]),
        );
        assert_eq!(attempt, Err(IdentityTransportError::NotInjective));
    }
}
