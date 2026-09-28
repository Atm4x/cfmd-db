use std::collections::{BTreeMap, BTreeSet};

use super::{
    PairCoordinationDecision, PreparedRewriteIntent, RewriteFootprint, RewriteLawSetId,
    RewriteSpec, RewriteSpecId, SemanticWriteCoordinate, coordination_decision,
    infer_pair_rewrite_law,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteCoordinationRegistryError {
    SpecIdentityConflict(RewriteSpecId),
    SpecNotRegistered(RewriteSpecId),
    LawSetMismatch(RewriteSpecId),
    DuplicateNode,
    PreparedSetMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredRewriteSpec {
    law_set: RewriteLawSetId,
    footprint: RewriteFootprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteCoordinationRegistry {
    specs: BTreeMap<RewriteSpecId, RegisteredRewriteSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRewriteCoordination<K> {
    members: BTreeSet<K>,
    graph: CompiledRewriteCoordinationGraph<K>,
}

impl<K: Copy + Ord> PreparedRewriteCoordination<K> {
    #[must_use]
    pub const fn members(&self) -> &BTreeSet<K> {
        &self.members
    }

    #[must_use]
    pub const fn graph(&self) -> &CompiledRewriteCoordinationGraph<K> {
        &self.graph
    }

    #[must_use]
    pub fn decision(&self, left: K, right: K) -> PairCoordinationDecision {
        self.graph.decision(left, right)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRewriteCoordinationGraph<K> {
    requires_coordination: BTreeSet<(K, K)>,
    intent_conflicts: BTreeSet<(K, K)>,
}

impl<K: Copy + Ord> CompiledRewriteCoordinationGraph<K> {
    #[must_use]
    pub const fn requires_coordination(&self) -> &BTreeSet<(K, K)> {
        &self.requires_coordination
    }

    #[must_use]
    pub const fn intent_conflicts(&self) -> &BTreeSet<(K, K)> {
        &self.intent_conflicts
    }

    #[must_use]
    pub fn decision(&self, left: K, right: K) -> PairCoordinationDecision {
        let pair = ordered_pair(left, right);
        if self.intent_conflicts.contains(&pair) {
            PairCoordinationDecision::IntentConflict
        } else if self.requires_coordination.contains(&pair) {
            PairCoordinationDecision::RequiresCoordination
        } else {
            PairCoordinationDecision::CoordinationFree
        }
    }

    #[must_use]
    pub fn coordination_free(&self) -> bool {
        self.requires_coordination.is_empty() && self.intent_conflicts.is_empty()
    }
}

impl RewriteCoordinationRegistry {
    pub fn register(&mut self, spec: &RewriteSpec) -> Result<(), RewriteCoordinationRegistryError> {
        let registered = RegisteredRewriteSpec {
            law_set: spec.law_set,
            footprint: spec.footprint.clone(),
        };
        match self.specs.get(&spec.id) {
            Some(existing) if existing == &registered => Ok(()),
            Some(_) => Err(RewriteCoordinationRegistryError::SpecIdentityConflict(
                spec.id,
            )),
            None => {
                self.specs.insert(spec.id, registered);
                Ok(())
            }
        }
    }

    /// Binds one concrete prepared Rewrite set to a single compiled coordination
    /// graph. The resulting object can be reused by multiple certification
    /// passes without rebuilding the semantic-coordinate inverted index.
    pub fn prepare<'a, K, R: PreparedRewriteIntent + ?Sized + 'a>(
        &self,
        rewrites: impl IntoIterator<Item = (K, &'a R)>,
    ) -> Result<PreparedRewriteCoordination<K>, RewriteCoordinationRegistryError>
    where
        K: Copy + Ord,
    {
        let rewrites = rewrites.into_iter().collect::<Vec<_>>();
        let mut members = BTreeSet::new();
        for (key, _) in &rewrites {
            if !members.insert(*key) {
                return Err(RewriteCoordinationRegistryError::DuplicateNode);
            }
        }
        let graph = self.compile(rewrites.iter().map(|(key, rewrite)| (*key, *rewrite)))?;
        Ok(PreparedRewriteCoordination { members, graph })
    }

    /// Compiles exact coordination obligations from authoritative Rewrite
    /// footprints. Candidate pairs are generated from an inverted semantic
    /// coordinate index: disjoint read/write footprints never enter the pair
    /// classifier because `infer_pair_rewrite_law` proves them `StrongCommute`.
    /// Rewrites carrying invariant obligations remain connected to every other
    /// rewrite, preserving the current fail-closed law exactly.
    pub fn compile<'a, K, R: PreparedRewriteIntent + ?Sized + 'a>(
        &self,
        rewrites: impl IntoIterator<Item = (K, &'a R)>,
    ) -> Result<CompiledRewriteCoordinationGraph<K>, RewriteCoordinationRegistryError>
    where
        K: Copy + Ord,
    {
        struct Node<'a, K> {
            key: K,
            footprint: &'a RewriteFootprint,
            has_invariant_obligations: bool,
        }

        let mut seen = BTreeSet::new();
        let mut nodes = Vec::new();
        for (key, rewrite) in rewrites {
            if !seen.insert(key) {
                return Err(RewriteCoordinationRegistryError::DuplicateNode);
            }
            let rewrite_spec = rewrite.rewrite_spec();
            let rewrite_law_set = rewrite.rewrite_law_set();
            let registered = self.specs.get(&rewrite_spec).ok_or(
                RewriteCoordinationRegistryError::SpecNotRegistered(rewrite_spec),
            )?;
            if registered.law_set != rewrite_law_set {
                return Err(RewriteCoordinationRegistryError::LawSetMismatch(
                    rewrite_spec,
                ));
            }
            nodes.push(Node {
                key,
                footprint: &registered.footprint,
                has_invariant_obligations: !registered.footprint.invariant_obligations.is_empty(),
            });
        }

        let mut readers: BTreeMap<SemanticWriteCoordinate, BTreeSet<usize>> = BTreeMap::new();
        let mut writers: BTreeMap<SemanticWriteCoordinate, BTreeSet<usize>> = BTreeMap::new();
        let mut invariant_nodes = BTreeSet::new();
        for (index, node) in nodes.iter().enumerate() {
            if node.has_invariant_obligations {
                invariant_nodes.insert(index);
            }
            for coordinate in &node.footprint.reads {
                readers.entry(coordinate.clone()).or_default().insert(index);
            }
            for coordinate in node.footprint.writes.keys() {
                writers.entry(coordinate.clone()).or_default().insert(index);
            }
        }

        let mut candidates = BTreeSet::new();
        for (coordinate, coordinate_writers) in &writers {
            if let Some(coordinate_readers) = readers.get(coordinate) {
                for &writer in coordinate_writers {
                    for &reader in coordinate_readers {
                        insert_index_pair(&mut candidates, writer, reader);
                    }
                }
            }
            let writers = coordinate_writers.iter().copied().collect::<Vec<_>>();
            for left in 0..writers.len() {
                for right in (left + 1)..writers.len() {
                    insert_index_pair(&mut candidates, writers[left], writers[right]);
                }
            }
        }

        // An unresolved invariant obligation blocks every coordination-free
        // claim under the existing law, including otherwise-disjoint pairs.
        for &invariant in &invariant_nodes {
            for other in 0..nodes.len() {
                insert_index_pair(&mut candidates, invariant, other);
            }
        }

        let mut requires_coordination = BTreeSet::new();
        let mut intent_conflicts = BTreeSet::new();
        for (left_index, right_index) in candidates {
            let left = &nodes[left_index];
            let right = &nodes[right_index];
            let pair = ordered_pair(left.key, right.key);
            match coordination_decision(infer_pair_rewrite_law(left.footprint, right.footprint)) {
                PairCoordinationDecision::CoordinationFree => {}
                PairCoordinationDecision::RequiresCoordination => {
                    requires_coordination.insert(pair);
                }
                PairCoordinationDecision::IntentConflict => {
                    intent_conflicts.insert(pair);
                }
            }
        }

        Ok(CompiledRewriteCoordinationGraph {
            requires_coordination,
            intent_conflicts,
        })
    }
}

fn insert_index_pair(pairs: &mut BTreeSet<(usize, usize)>, left: usize, right: usize) {
    if left == right {
        return;
    }
    pairs.insert(ordered_pair(left, right));
}

fn ordered_pair<K: Ord>(left: K, right: K) -> (K, K) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

#[cfg(test)]
mod tests {
    use kernel_types::SemanticId;

    use crate::rewrite::{
        RewriteActionLaw, RewriteEffect, RewriteFootprint, RewriteLawSetId, RewriteSpec,
        RewriteSpecId, SemanticWriteCoordinate,
    };

    use super::RewriteCoordinationRegistry;

    fn spec(id: u128, coordinate: u128) -> RewriteSpec {
        RewriteSpec {
            id: RewriteSpecId(SemanticId(id)),
            law_set: RewriteLawSetId(SemanticId(99)),
            footprint: RewriteFootprint {
                writes: [(
                    SemanticWriteCoordinate::ProductField(SemanticId(coordinate)),
                    RewriteActionLaw::Opaque,
                )]
                .into_iter()
                .collect(),
                ..RewriteFootprint::default()
            },
        }
    }

    #[test]
    fn prepared_coordination_retains_compiled_membership_and_graph() {
        let a = spec(1, 10);
        let b = spec(2, 10);
        let c = spec(3, 11);
        let mut registry = RewriteCoordinationRegistry::default();
        for spec in [&a, &b, &c] {
            registry.register(spec).unwrap();
        }
        let rewrites = [
            a.prepare(Vec::<()>::new(), RewriteEffect::Replace(1_i32)),
            b.prepare(Vec::<()>::new(), RewriteEffect::Replace(2_i32)),
            c.prepare(Vec::<()>::new(), RewriteEffect::Replace(3_i32)),
        ];
        let prepared = registry.prepare(rewrites.iter().enumerate()).unwrap();

        assert_eq!(prepared.members().len(), 3);
        assert_eq!(
            prepared.decision(0, 1),
            crate::rewrite::PairCoordinationDecision::RequiresCoordination
        );
        assert_eq!(
            prepared.decision(0, 2),
            crate::rewrite::PairCoordinationDecision::CoordinationFree
        );
    }
}
