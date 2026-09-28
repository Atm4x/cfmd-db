mod residual;

pub use residual::*;

use std::collections::{BTreeMap, BTreeSet};

use crate::rewrite::PairCoordinationDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevisionEffectId(pub u128);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffect<E> {
    pub id: RevisionEffectId,
    pub prerequisites: BTreeSet<RevisionEffectId>,
    pub payload: E,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectIdeal<E> {
    events: BTreeMap<RevisionEffectId, RevisionEffect<E>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectMergeRequirements {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_exclusive: BTreeSet<RevisionEffectId>,
    pub right_exclusive: BTreeSet<RevisionEffectId>,
    pub requires_residual: BTreeSet<(RevisionEffectId, RevisionEffectId)>,
    pub intent_conflicts: BTreeSet<(RevisionEffectId, RevisionEffectId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectCausalLayerSchedule {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_layers: Vec<BTreeSet<RevisionEffectId>>,
    pub right_layers: Vec<BTreeSet<RevisionEffectId>>,
}

impl RevisionEffectMergeRequirements {
    #[must_use]
    pub fn coordination_free(&self) -> bool {
        self.requires_residual.is_empty() && self.intent_conflicts.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectIdealError {
    DuplicateEffect(RevisionEffectId),
    MissingPrerequisite {
        effect: RevisionEffectId,
        prerequisite: RevisionEffectId,
    },
    CyclicPrerequisites(BTreeSet<RevisionEffectId>),
    EffectIdentityConflict(RevisionEffectId),
    NotSubideal,
}

impl<E: Clone + PartialEq + Eq> RevisionEffectIdeal<E> {
    pub fn new(
        events: impl IntoIterator<Item = RevisionEffect<E>>,
    ) -> Result<Self, RevisionEffectIdealError> {
        let mut by_id = BTreeMap::new();
        for event in events {
            let id = event.id;
            if by_id.insert(id, event).is_some() {
                return Err(RevisionEffectIdealError::DuplicateEffect(id));
            }
        }
        for event in by_id.values() {
            for prerequisite in &event.prerequisites {
                if !by_id.contains_key(prerequisite) {
                    return Err(RevisionEffectIdealError::MissingPrerequisite {
                        effect: event.id,
                        prerequisite: *prerequisite,
                    });
                }
            }
        }
        let mut pending = by_id
            .iter()
            .map(|(&id, event)| (id, event.prerequisites.len()))
            .collect::<BTreeMap<_, _>>();
        let mut successors = BTreeMap::<RevisionEffectId, Vec<RevisionEffectId>>::new();
        for event in by_id.values() {
            for prerequisite in &event.prerequisites {
                successors.entry(*prerequisite).or_default().push(event.id);
            }
        }
        let mut ready = pending
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some(id))
            .collect::<BTreeSet<_>>();
        while let Some(completed) = ready.pop_first() {
            let count = pending
                .remove(&completed)
                .expect("ready effects must still be pending");
            debug_assert_eq!(count, 0);
            if let Some(dependents) = successors.get(&completed) {
                for dependent in dependents {
                    let remaining = pending
                        .get_mut(dependent)
                        .expect("dependent must remain pending until all prerequisites complete");
                    *remaining -= 1;
                    if *remaining == 0 {
                        ready.insert(*dependent);
                    }
                }
            }
        }
        if !pending.is_empty() {
            return Err(RevisionEffectIdealError::CyclicPrerequisites(
                pending.into_keys().collect(),
            ));
        }
        Ok(Self { events: by_id })
    }

    #[must_use]
    pub const fn events(&self) -> &BTreeMap<RevisionEffectId, RevisionEffect<E>> {
        &self.events
    }

    /// Returns the exact-identity intersection of two already valid ideals.
    /// Down-closure is preserved by intersection, so no second DAG validation
    /// pass is required after shared identities have been checked.
    pub fn common_ideal(&self, other: &Self) -> Result<Self, RevisionEffectIdealError> {
        let mut common = BTreeMap::new();
        for (&id, event) in &self.events {
            let Some(other_event) = other.events.get(&id) else {
                continue;
            };
            if event != other_event {
                return Err(RevisionEffectIdealError::EffectIdentityConflict(id));
            }
            common.insert(id, event.clone());
        }
        Ok(Self { events: common })
    }

    pub fn exclusive_from(
        &self,
        common: &Self,
    ) -> Result<Vec<&RevisionEffect<E>>, RevisionEffectIdealError> {
        for (&id, event) in &common.events {
            if self.events.get(&id) != Some(event) {
                return Err(RevisionEffectIdealError::NotSubideal);
            }
        }
        Ok(self
            .events
            .iter()
            .filter_map(|(id, event)| (!common.events.contains_key(id)).then_some(event))
            .collect())
    }

    /// Returns the exact-identity union of two already valid ideals.
    /// Each event brings its complete prerequisite closure from its source ideal,
    /// so compatible union does not require a second DAG validation pass.
    pub fn union(&self, other: &Self) -> Result<Self, RevisionEffectIdealError> {
        let mut union = self.events.clone();
        for (&id, event) in &other.events {
            if let Some(existing) = union.get(&id) {
                if existing != event {
                    return Err(RevisionEffectIdealError::EffectIdentityConflict(id));
                }
            } else {
                union.insert(id, event.clone());
            }
        }
        Ok(Self { events: union })
    }

    pub fn merge_requirements(
        &self,
        other: &Self,
        classify: impl Fn(&E, &E) -> PairCoordinationDecision,
    ) -> Result<RevisionEffectMergeRequirements, RevisionEffectIdealError> {
        let common = self.common_ideal(other)?;
        let left = self.exclusive_from(&common)?;
        let right = other.exclusive_from(&common)?;
        let mut requires_residual = BTreeSet::new();
        let mut intent_conflicts = BTreeSet::new();
        for left_event in &left {
            for right_event in &right {
                let pair = (left_event.id, right_event.id);
                match classify(&left_event.payload, &right_event.payload) {
                    PairCoordinationDecision::CoordinationFree => {}
                    PairCoordinationDecision::RequiresCoordination => {
                        requires_residual.insert(pair);
                    }
                    PairCoordinationDecision::IntentConflict => {
                        intent_conflicts.insert(pair);
                    }
                }
            }
        }
        Ok(RevisionEffectMergeRequirements {
            common: common.events.keys().copied().collect(),
            left_exclusive: left.into_iter().map(|event| event.id).collect(),
            right_exclusive: right.into_iter().map(|event| event.id).collect(),
            requires_residual,
            intent_conflicts,
        })
    }

    fn exclusive_causal_layers_from(
        &self,
        common: &Self,
    ) -> Result<Vec<BTreeSet<RevisionEffectId>>, RevisionEffectIdealError> {
        let exclusive = self.exclusive_from(common)?;
        let exclusive_ids = exclusive
            .iter()
            .map(|event| event.id)
            .collect::<BTreeSet<_>>();
        let mut pending = BTreeMap::<RevisionEffectId, usize>::new();
        let mut successors = BTreeMap::<RevisionEffectId, Vec<RevisionEffectId>>::new();
        for event in exclusive {
            let mut exclusive_prerequisites = 0;
            for prerequisite in &event.prerequisites {
                if exclusive_ids.contains(prerequisite) {
                    exclusive_prerequisites += 1;
                    successors.entry(*prerequisite).or_default().push(event.id);
                }
            }
            pending.insert(event.id, exclusive_prerequisites);
        }

        let mut ready = pending
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some(id))
            .collect::<BTreeSet<_>>();
        let mut layers = Vec::new();
        while !ready.is_empty() {
            let layer = std::mem::take(&mut ready);
            for completed in &layer {
                let count = pending
                    .remove(completed)
                    .expect("ready effects must still be pending");
                debug_assert_eq!(count, 0);
                if let Some(dependents) = successors.get(completed) {
                    for dependent in dependents {
                        let remaining = pending.get_mut(dependent).expect(
                            "dependent must remain pending until all prerequisites complete",
                        );
                        *remaining -= 1;
                        if *remaining == 0 {
                            ready.insert(*dependent);
                        }
                    }
                }
            }
            layers.push(layer);
        }
        if !pending.is_empty() {
            return Err(RevisionEffectIdealError::CyclicPrerequisites(
                pending.into_keys().collect(),
            ));
        }
        Ok(layers)
    }

    pub fn causal_layer_schedule(
        &self,
        other: &Self,
    ) -> Result<RevisionEffectCausalLayerSchedule, RevisionEffectIdealError> {
        let common = self.common_ideal(other)?;
        Ok(RevisionEffectCausalLayerSchedule {
            common: common.events.keys().copied().collect(),
            left_layers: self.exclusive_causal_layers_from(&common)?,
            right_layers: other.exclusive_causal_layers_from(&common)?,
        })
    }
}
