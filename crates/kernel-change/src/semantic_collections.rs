use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{EqClassId, RevisionObservableId};

/// Revision-local Γ class coordinate used by structural collection changes.
///
/// The observable pins the semantic quotient used to classify values. The
/// class id alone is intentionally insufficient because `EqClassId` is local
/// to one revision observable catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticClassCoordinate {
    pub observable: RevisionObservableId,
    pub class: EqClassId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticCollectionChangeError {
    ObservableMismatch,
    DuplicateSourceClass(SemanticClassCoordinate),
    MissingRemovedClass(SemanticClassCoordinate),
    ConflictingSetClass(SemanticClassCoordinate),
    BagRemovalExceedsMultiplicity {
        coordinate: SemanticClassCoordinate,
        available: u64,
        removed: u64,
    },
    BagMultiplicityOverflow(SemanticClassCoordinate),
    DuplicateMapKeyClass(SemanticClassCoordinate),
}

/// Γ-aware Set patch over revision-local semantic classes.
///
/// Values are carried only for newly inserted classes. Existing values are
/// addressed by semantic class, never by Rust equality/hash identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSetChange<T> {
    pub observable: RevisionObservableId,
    pub inserted: BTreeMap<EqClassId, T>,
    pub removed: BTreeSet<EqClassId>,
}

impl<T: Clone> SemanticSetChange<T> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, T)>,
    ) -> Result<BTreeMap<EqClassId, T>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::new();
        for (coordinate, value) in old {
            if coordinate.observable != self.observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if next.insert(coordinate.class, value).is_some() {
                return Err(SemanticCollectionChangeError::DuplicateSourceClass(
                    coordinate,
                ));
            }
        }
        for class in &self.removed {
            let coordinate = SemanticClassCoordinate {
                observable: self.observable,
                class: *class,
            };
            if self.inserted.contains_key(class) {
                return Err(SemanticCollectionChangeError::ConflictingSetClass(
                    coordinate,
                ));
            }
            if next.remove(class).is_none() {
                return Err(SemanticCollectionChangeError::MissingRemovedClass(
                    coordinate,
                ));
            }
        }
        for (&class, value) in &self.inserted {
            next.entry(class).or_insert_with(|| value.clone());
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBagClassChange<T> {
    pub representative: Option<T>,
    pub inserted: u64,
    pub removed: u64,
}

/// Γ-aware Bag patch. Multiplicity is changed per semantic class; a concrete
/// representative is required only when an insertion can create a new class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBagChange<T> {
    pub observable: RevisionObservableId,
    pub classes: BTreeMap<EqClassId, SemanticBagClassChange<T>>,
}

impl<T: Clone> SemanticBagChange<T> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, T, u64)>,
    ) -> Result<BTreeMap<EqClassId, (T, u64)>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::<EqClassId, (T, u64)>::new();
        for (coordinate, value, count) in old {
            if coordinate.observable != self.observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if let Some((_, existing_count)) = next.get_mut(&coordinate.class) {
                *existing_count = existing_count.checked_add(count).ok_or(
                    SemanticCollectionChangeError::BagMultiplicityOverflow(coordinate),
                )?;
            } else {
                next.insert(coordinate.class, (value, count));
            }
        }

        for (&class, change) in &self.classes {
            let coordinate = SemanticClassCoordinate {
                observable: self.observable,
                class,
            };
            let available = next.get(&class).map_or(0, |(_, count)| *count);
            if change.removed > available {
                return Err(
                    SemanticCollectionChangeError::BagRemovalExceedsMultiplicity {
                        coordinate,
                        available,
                        removed: change.removed,
                    },
                );
            }
            let after_remove = available - change.removed;
            let after_insert = after_remove.checked_add(change.inserted).ok_or(
                SemanticCollectionChangeError::BagMultiplicityOverflow(coordinate),
            )?;
            if after_insert == 0 {
                next.remove(&class);
                continue;
            }
            let representative = match (next.get(&class), &change.representative) {
                (Some((existing, _)), _) => existing.clone(),
                (None, Some(representative)) => representative.clone(),
                (None, None) => {
                    return Err(SemanticCollectionChangeError::MissingRemovedClass(
                        coordinate,
                    ));
                }
            };
            next.insert(class, (representative, after_insert));
        }
        Ok(next)
    }
}

/// Γ-aware Map patch keyed by semantic key classes. Upsert is the only way to
/// replace a value for an existing class; remove+upsert of the same class is
/// rejected so intent does not depend on operation ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticMapChange<K, V> {
    pub key_observable: RevisionObservableId,
    pub upserted: BTreeMap<EqClassId, (K, V)>,
    pub removed: BTreeSet<EqClassId>,
}

impl<K: Clone, V: Clone> SemanticMapChange<K, V> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, K, V)>,
    ) -> Result<BTreeMap<EqClassId, (K, V)>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::new();
        for (coordinate, key, value) in old {
            if coordinate.observable != self.key_observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if next.insert(coordinate.class, (key, value)).is_some() {
                return Err(SemanticCollectionChangeError::DuplicateMapKeyClass(
                    coordinate,
                ));
            }
        }
        for class in &self.removed {
            let coordinate = SemanticClassCoordinate {
                observable: self.key_observable,
                class: *class,
            };
            if self.upserted.contains_key(class) {
                return Err(SemanticCollectionChangeError::DuplicateMapKeyClass(
                    coordinate,
                ));
            }
            if next.remove(class).is_none() {
                return Err(SemanticCollectionChangeError::MissingRemovedClass(
                    coordinate,
                ));
            }
        }
        for (&class, pair) in &self.upserted {
            next.insert(class, pair.clone());
        }
        Ok(next)
    }
}
