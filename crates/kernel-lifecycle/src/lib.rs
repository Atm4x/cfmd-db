use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kernel_identity::{DenseEntityIds, DenseEntitySet, DenseIdentityError, LocalEntityId};
use kernel_types::EntityId;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LifecycleFact {
    Root(EntityId),
    KeepsAlive { parent: EntityId, child: EntityId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LifecycleIntent {
    edits: BTreeMap<LifecycleFact, bool>,
}

impl LifecycleIntent {
    pub fn set(&mut self, fact: LifecycleFact, present: bool) {
        self.edits.insert(fact, present);
    }

    #[must_use]
    pub fn edits(&self) -> &BTreeMap<LifecycleFact, bool> {
        &self.edits
    }

    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        let mut composed = self.clone();
        composed.edits.extend(next.edits.clone());
        composed
    }

    pub fn merge(left: &Self, right: &Self) -> Result<Self, LifecycleConflict> {
        let mut merged = left.clone();
        for (fact, &present) in &right.edits {
            if let Some(&existing) = merged.edits.get(fact)
                && existing != present
            {
                return Err(LifecycleConflict { fact: fact.clone() });
            }
            merged.edits.insert(fact.clone(), present);
        }
        Ok(merged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleConflict {
    pub fact: LifecycleFact,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LifecycleGraph {
    pub entities: BTreeSet<EntityId>,
    pub roots: BTreeSet<EntityId>,
    pub keeps_alive: BTreeMap<EntityId, BTreeSet<EntityId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenseLifecycleProjection {
    ids: DenseEntityIds,
    roots: DenseEntitySet,
    keeps_alive: Vec<Vec<LocalEntityId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintainedDenseLifecycle {
    ids: DenseEntityIds,
    roots: DenseEntitySet,
    keeps_alive: Vec<Vec<LocalEntityId>>,
    kept_by: Vec<Vec<LocalEntityId>>,
    live: DenseEntitySet,
}

impl MaintainedDenseLifecycle {
    pub fn compile(graph: &LifecycleGraph) -> Result<Self, DenseIdentityError> {
        let projection = DenseLifecycleProjection::compile(graph)?;
        Ok(Self::from_projection(projection))
    }

    #[must_use]
    pub fn compile_with_ids(graph: &LifecycleGraph, ids: &DenseEntityIds) -> Self {
        Self::from_projection(DenseLifecycleProjection::compile_with_ids(graph, ids))
    }

    fn from_projection(projection: DenseLifecycleProjection) -> Self {
        let mut kept_by = vec![Vec::new(); projection.ids.len()];
        for (parent_index, children) in projection.keeps_alive.iter().enumerate() {
            let Some(parent_external) = projection.ids.external_at(parent_index) else {
                continue;
            };
            let Some(parent) = projection.ids.local(parent_external) else {
                continue;
            };
            for &child in children {
                kept_by[child.index()].push(parent);
            }
        }
        for parents in &mut kept_by {
            parents.sort_unstable();
            parents.dedup();
        }
        let live = projection.live_set();
        Self {
            ids: projection.ids,
            roots: projection.roots,
            keeps_alive: projection.keeps_alive,
            kept_by,
            live,
        }
    }

    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    #[must_use]
    pub fn live_entities(&self) -> BTreeSet<EntityId> {
        self.live
            .iter()
            .filter_map(|local| self.ids.external(local))
            .collect()
    }

    pub fn apply_intent(&mut self, intent: &LifecycleIntent) {
        let mut decrease_starts = Vec::new();
        let mut additions = Vec::new();

        for (fact, &present) in intent.edits() {
            match (fact, present) {
                (LifecycleFact::Root(entity), false) => {
                    if let Some(local) = self.ids.local(*entity)
                        && self.roots.remove(local)
                    {
                        decrease_starts.push(local);
                    }
                }
                (LifecycleFact::KeepsAlive { parent, child }, false) => {
                    let (Some(parent), Some(child)) =
                        (self.ids.local(*parent), self.ids.local(*child))
                    else {
                        continue;
                    };
                    if remove_sorted(&mut self.keeps_alive[parent.index()], child) {
                        remove_sorted(&mut self.kept_by[child.index()], parent);
                        decrease_starts.push(child);
                    }
                }
                (_, true) => additions.push(fact.clone()),
            }
        }

        if !decrease_starts.is_empty() {
            self.recompute_decrease_region(&decrease_starts);
        }

        let mut increase_starts = Vec::new();
        for fact in additions {
            match fact {
                LifecycleFact::Root(entity) => {
                    if let Some(local) = self.ids.local(entity) {
                        self.roots.insert(local);
                        increase_starts.push(local);
                    }
                }
                LifecycleFact::KeepsAlive { parent, child } => {
                    let (Some(parent), Some(child)) =
                        (self.ids.local(parent), self.ids.local(child))
                    else {
                        continue;
                    };
                    insert_sorted_unique(&mut self.keeps_alive[parent.index()], child);
                    insert_sorted_unique(&mut self.kept_by[child.index()], parent);
                    if self.live.contains(parent) {
                        increase_starts.push(child);
                    }
                }
            }
        }
        self.activate_from(&increase_starts);
    }

    fn recompute_decrease_region(&mut self, starts: &[LocalEntityId]) {
        let mut candidate = DenseEntitySet::with_capacity(self.ids.len());
        let mut queue = VecDeque::from(starts.to_vec());
        while let Some(entity) = queue.pop_front() {
            if candidate.contains(entity) {
                continue;
            }
            candidate.insert(entity);
            queue.extend(self.keeps_alive[entity.index()].iter().copied());
        }

        let mut surviving = DenseEntitySet::with_capacity(self.ids.len());
        let mut queue = VecDeque::new();
        for entity in candidate.iter() {
            let externally_supported = self.roots.contains(entity)
                || self.kept_by[entity.index()]
                    .iter()
                    .any(|&parent| self.live.contains(parent) && !candidate.contains(parent));
            if externally_supported {
                surviving.insert(entity);
                queue.push_back(entity);
            }
        }
        while let Some(parent) = queue.pop_front() {
            for &child in &self.keeps_alive[parent.index()] {
                if candidate.contains(child) && !surviving.contains(child) {
                    surviving.insert(child);
                    queue.push_back(child);
                }
            }
        }
        for entity in candidate.iter() {
            self.live.remove(entity);
        }
        for entity in surviving.iter() {
            self.live.insert(entity);
        }
    }

    fn activate_from(&mut self, starts: &[LocalEntityId]) {
        let mut queue = VecDeque::from(starts.to_vec());
        while let Some(entity) = queue.pop_front() {
            if self.live.contains(entity) {
                continue;
            }
            self.live.insert(entity);
            queue.extend(self.keeps_alive[entity.index()].iter().copied());
        }
    }
}

fn insert_sorted_unique(values: &mut Vec<LocalEntityId>, value: LocalEntityId) -> bool {
    match values.binary_search(&value) {
        Ok(_) => false,
        Err(index) => {
            values.insert(index, value);
            true
        }
    }
}

fn remove_sorted(values: &mut Vec<LocalEntityId>, value: LocalEntityId) -> bool {
    let Ok(index) = values.binary_search(&value) else {
        return false;
    };
    values.remove(index);
    true
}

impl DenseLifecycleProjection {
    pub fn compile(graph: &LifecycleGraph) -> Result<Self, DenseIdentityError> {
        let ids = DenseEntityIds::compile(&graph.entities)?;
        Ok(Self::compile_with_ids(graph, &ids))
    }

    #[must_use]
    pub fn compile_with_ids(graph: &LifecycleGraph, ids: &DenseEntityIds) -> Self {
        let mut roots = DenseEntitySet::with_capacity(ids.len());
        for root in graph.roots.iter().filter_map(|entity| ids.local(*entity)) {
            roots.insert(root);
        }
        let mut keeps_alive = vec![Vec::new(); ids.len()];
        for (parent, children) in &graph.keeps_alive {
            let Some(parent) = ids.local(*parent) else {
                continue;
            };
            keeps_alive[parent.index()]
                .extend(children.iter().filter_map(|child| ids.local(*child)));
        }
        Self {
            ids: ids.clone(),
            roots,
            keeps_alive,
        }
    }

    fn live_set(&self) -> DenseEntitySet {
        let mut live = DenseEntitySet::with_capacity(self.ids.len());
        let mut queue = VecDeque::new();
        for root in self.roots.iter() {
            if !live.contains(root) {
                live.insert(root);
                queue.push_back(root);
            }
        }
        while let Some(parent) = queue.pop_front() {
            for &child in &self.keeps_alive[parent.index()] {
                if !live.contains(child) {
                    live.insert(child);
                    queue.push_back(child);
                }
            }
        }
        live
    }

    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live_set().len()
    }

    #[must_use]
    pub fn live_entities(&self) -> BTreeSet<EntityId> {
        self.live_set()
            .iter()
            .filter_map(|local| self.ids.external(local))
            .collect()
    }
}

impl LifecycleGraph {
    pub fn apply_intent(&mut self, intent: &LifecycleIntent) {
        for (fact, present) in intent.edits() {
            match fact {
                LifecycleFact::Root(entity) => {
                    if *present {
                        self.roots.insert(*entity);
                    } else {
                        self.roots.remove(entity);
                    }
                }
                LifecycleFact::KeepsAlive { parent, child } => {
                    if *present {
                        self.keeps_alive.entry(*parent).or_default().insert(*child);
                    } else if let Some(children) = self.keeps_alive.get_mut(parent) {
                        children.remove(child);
                        if children.is_empty() {
                            self.keeps_alive.remove(parent);
                        }
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn live_entities(&self) -> BTreeSet<EntityId> {
        let mut live = BTreeSet::new();
        let mut queue = VecDeque::new();

        for &root in &self.roots {
            if self.entities.contains(&root) && live.insert(root) {
                queue.push_back(root);
            }
        }

        while let Some(parent) = queue.pop_front() {
            if let Some(children) = self.keeps_alive.get(&parent) {
                for &child in children {
                    if self.entities.contains(&child) && live.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
        }

        live
    }

    #[must_use]
    pub fn normalize(&self) -> Self {
        let live = self.live_entities();
        let roots = self.roots.intersection(&live).copied().collect();
        let mut keeps_alive = BTreeMap::new();

        for (&parent, children) in &self.keeps_alive {
            if !live.contains(&parent) {
                continue;
            }
            let live_children: BTreeSet<_> = children.intersection(&live).copied().collect();
            if !live_children.is_empty() {
                keeps_alive.insert(parent, live_children);
            }
        }

        Self {
            entities: live,
            roots,
            keeps_alive,
        }
    }

    pub fn merge_from_lca(
        lca: &Self,
        left: &LifecycleIntent,
        right: &LifecycleIntent,
    ) -> Result<Self, LifecycleConflict> {
        let intent = LifecycleIntent::merge(left, right)?;
        let mut candidate = lca.clone();
        candidate.apply_intent(&intent);
        Ok(candidate.normalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    fn grounded_live_entities(graph: &LifecycleGraph) -> BTreeSet<EntityId> {
        use kernel_grounded_closure::{GroundedAtomId, GroundedProgram, GroundedRule, solve};

        let entity_by_atom = graph.entities.iter().copied().collect::<Vec<_>>();
        let atom_by_entity = entity_by_atom
            .iter()
            .enumerate()
            .map(|(index, &entity)| (entity, GroundedAtomId::new(index)))
            .collect::<BTreeMap<_, _>>();
        let seeds = graph
            .roots
            .iter()
            .filter_map(|root| atom_by_entity.get(root).copied())
            .collect::<Vec<_>>();
        let mut rules = Vec::new();
        for (parent, children) in &graph.keeps_alive {
            let Some(&parent_atom) = atom_by_entity.get(parent) else {
                continue;
            };
            for child in children {
                let Some(&child_atom) = atom_by_entity.get(child) else {
                    continue;
                };
                rules.push(GroundedRule::new([parent_atom], child_atom));
            }
        }
        let program = GroundedProgram::new(entity_by_atom.len(), seeds, rules).unwrap();
        solve(&program)
            .0
            .live_atoms()
            .map(|atom| entity_by_atom[atom.index()])
            .collect()
    }

    #[test]
    fn dense_local_id_projection_matches_reference_reachability_including_dead_cycle() {
        let mut graph = LifecycleGraph::default();
        graph.entities.extend((1..=8).map(id));
        graph.roots.extend([id(1), id(5)]);
        graph.keeps_alive.insert(id(1), BTreeSet::from([id(2)]));
        graph.keeps_alive.insert(id(2), BTreeSet::from([id(3)]));
        graph.keeps_alive.insert(id(3), BTreeSet::from([id(2)]));
        graph.keeps_alive.insert(id(5), BTreeSet::from([id(6)]));
        graph.keeps_alive.insert(id(7), BTreeSet::from([id(8)]));
        graph.keeps_alive.insert(id(8), BTreeSet::from([id(7)]));

        let dense = DenseLifecycleProjection::compile(&graph).unwrap();
        assert_eq!(dense.live_entities(), graph.live_entities());
        assert_eq!(dense.live_count(), 5);
    }

    #[test]
    fn dense_local_id_projection_matches_reference_on_all_three_node_graphs() {
        let nodes = [id(1), id(2), id(3)];
        let directed_edges = [
            (nodes[0], nodes[1]),
            (nodes[0], nodes[2]),
            (nodes[1], nodes[0]),
            (nodes[1], nodes[2]),
            (nodes[2], nodes[0]),
            (nodes[2], nodes[1]),
        ];

        for root_mask in 0_u8..8 {
            for edge_mask in 0_u8..64 {
                let mut graph = LifecycleGraph::default();
                graph.entities.extend(nodes);
                for (index, node) in nodes.iter().enumerate() {
                    if root_mask & (1 << index) != 0 {
                        graph.roots.insert(*node);
                    }
                }
                for (index, (parent, child)) in directed_edges.iter().enumerate() {
                    if edge_mask & (1 << index) != 0 {
                        graph.keeps_alive.entry(*parent).or_default().insert(*child);
                    }
                }

                let dense = DenseLifecycleProjection::compile(&graph).unwrap();
                let expected = graph.live_entities();
                assert_eq!(
                    dense.live_entities(),
                    expected,
                    "root_mask={root_mask}, edge_mask={edge_mask}"
                );
                assert_eq!(dense.live_count(), expected.len());
            }
        }
    }

    #[test]
    fn lifecycle_reference_is_unary_grounded_closure_on_all_three_node_graphs() {
        let nodes = [id(1), id(2), id(3)];
        let directed_edges = [
            (nodes[0], nodes[1]),
            (nodes[0], nodes[2]),
            (nodes[1], nodes[0]),
            (nodes[1], nodes[2]),
            (nodes[2], nodes[0]),
            (nodes[2], nodes[1]),
        ];

        for root_mask in 0_u8..8 {
            for edge_mask in 0_u8..64 {
                let mut graph = LifecycleGraph::default();
                graph.entities.extend(nodes);
                for (index, node) in nodes.iter().enumerate() {
                    if root_mask & (1 << index) != 0 {
                        graph.roots.insert(*node);
                    }
                }
                for (index, (parent, child)) in directed_edges.iter().enumerate() {
                    if edge_mask & (1 << index) != 0 {
                        graph.keeps_alive.entry(*parent).or_default().insert(*child);
                    }
                }
                assert_eq!(
                    grounded_live_entities(&graph),
                    graph.live_entities(),
                    "root_mask={root_mask}, edge_mask={edge_mask}"
                );
            }
        }
    }

    #[test]
    fn maintained_dense_lifecycle_matches_exact_lfp_through_cycle_death_and_resurrection() {
        let mut graph = LifecycleGraph::default();
        graph.entities.extend((1..=6).map(id));
        graph.roots.insert(id(1));
        graph.keeps_alive.insert(id(1), BTreeSet::from([id(2)]));
        graph.keeps_alive.insert(id(2), BTreeSet::from([id(3)]));
        graph
            .keeps_alive
            .insert(id(3), BTreeSet::from([id(2), id(4)]));
        graph.keeps_alive.insert(id(5), BTreeSet::from([id(6)]));
        graph.keeps_alive.insert(id(6), BTreeSet::from([id(5)]));

        let mut maintained = MaintainedDenseLifecycle::compile(&graph).unwrap();
        assert_eq!(maintained.live_entities(), graph.live_entities());

        let mut cut = LifecycleIntent::default();
        cut.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        graph.apply_intent(&cut);
        maintained.apply_intent(&cut);
        assert_eq!(maintained.live_entities(), graph.live_entities());
        assert_eq!(maintained.live_entities(), BTreeSet::from([id(1)]));

        let mut root_cycle = LifecycleIntent::default();
        root_cycle.set(LifecycleFact::Root(id(3)), true);
        graph.apply_intent(&root_cycle);
        maintained.apply_intent(&root_cycle);
        assert_eq!(maintained.live_entities(), graph.live_entities());
        assert_eq!(maintained.live_count(), 4);

        let mut bridge = LifecycleIntent::default();
        bridge.set(
            LifecycleFact::KeepsAlive {
                parent: id(4),
                child: id(5),
            },
            true,
        );
        graph.apply_intent(&bridge);
        maintained.apply_intent(&bridge);
        assert_eq!(maintained.live_entities(), graph.live_entities());
        assert_eq!(maintained.live_count(), 6);

        let mut remove_root = LifecycleIntent::default();
        remove_root.set(LifecycleFact::Root(id(3)), false);
        graph.apply_intent(&remove_root);
        maintained.apply_intent(&remove_root);
        assert_eq!(maintained.live_entities(), graph.live_entities());
        assert_eq!(maintained.live_entities(), BTreeSet::from([id(1)]));
    }

    #[test]
    fn maintained_dense_lifecycle_matches_reference_for_all_single_fact_toggles_on_three_nodes() {
        let nodes = [id(1), id(2), id(3)];
        let directed_edges = [
            (nodes[0], nodes[1]),
            (nodes[0], nodes[2]),
            (nodes[1], nodes[0]),
            (nodes[1], nodes[2]),
            (nodes[2], nodes[0]),
            (nodes[2], nodes[1]),
        ];

        for root_mask in 0_u8..8 {
            for edge_mask in 0_u8..64 {
                let mut base = LifecycleGraph::default();
                base.entities.extend(nodes);
                for (index, node) in nodes.iter().enumerate() {
                    if root_mask & (1 << index) != 0 {
                        base.roots.insert(*node);
                    }
                }
                for (index, (parent, child)) in directed_edges.iter().enumerate() {
                    if edge_mask & (1 << index) != 0 {
                        base.keeps_alive.entry(*parent).or_default().insert(*child);
                    }
                }

                let facts = nodes.iter().copied().map(LifecycleFact::Root).chain(
                    directed_edges
                        .iter()
                        .copied()
                        .map(|(parent, child)| LifecycleFact::KeepsAlive { parent, child }),
                );
                for fact in facts {
                    let currently_present = match &fact {
                        LifecycleFact::Root(entity) => base.roots.contains(entity),
                        LifecycleFact::KeepsAlive { parent, child } => base
                            .keeps_alive
                            .get(parent)
                            .is_some_and(|children| children.contains(child)),
                    };
                    let mut intent = LifecycleIntent::default();
                    intent.set(fact, !currently_present);

                    let mut reference = base.clone();
                    reference.apply_intent(&intent);
                    let mut maintained = MaintainedDenseLifecycle::compile(&base).unwrap();
                    maintained.apply_intent(&intent);
                    assert_eq!(
                        maintained.live_entities(),
                        reference.live_entities(),
                        "root_mask={root_mask}, edge_mask={edge_mask}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_maintained_dense_lifecycle_local_cut_against_full_tree_recompute() {
        use std::hint::black_box;
        use std::time::Instant;

        let n = 50_000_u128;
        let cut_parent = 49_000_u128;
        let mut graph = LifecycleGraph::default();
        graph.entities.extend((1..=n).map(id));
        graph.roots.insert(id(1));
        for raw in 1..n {
            graph
                .keeps_alive
                .entry(id(raw))
                .or_default()
                .insert(id(raw + 1));
        }
        let mut maintained = MaintainedDenseLifecycle::compile(&graph).unwrap();

        let mut remove = LifecycleIntent::default();
        remove.set(
            LifecycleFact::KeepsAlive {
                parent: id(cut_parent),
                child: id(cut_parent + 1),
            },
            false,
        );
        let mut add = LifecycleIntent::default();
        add.set(
            LifecycleFact::KeepsAlive {
                parent: id(cut_parent),
                child: id(cut_parent + 1),
            },
            true,
        );

        let start = Instant::now();
        for _ in 0..20 {
            graph.apply_intent(&remove);
            black_box(graph.live_entities());
            graph.apply_intent(&add);
            black_box(graph.live_entities());
        }
        let tree_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        for _ in 0..20 {
            maintained.apply_intent(&remove);
            black_box(maintained.live_count());
            maintained.apply_intent(&add);
            black_box(maintained.live_count());
        }
        let maintained_ns = start.elapsed().as_nanos();
        assert_eq!(maintained.live_entities(), graph.live_entities());
        let ratio_milli = tree_ns.saturating_mul(1_000) / maintained_ns.max(1);
        println!("tree_ns={tree_ns} maintained_ns={maintained_ns} ratio_milli={ratio_milli}");
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_dense_local_id_lifecycle_against_entity_tree_walk() {
        use std::hint::black_box;
        use std::time::Instant;

        let n = 50_000_u128;
        let mut graph = LifecycleGraph::default();
        graph.entities.extend((1..=n).map(id));
        graph.roots.insert(id(1));
        for raw in 1..n {
            graph
                .keeps_alive
                .entry(id(raw))
                .or_default()
                .insert(id(raw + 1));
        }
        let dense = DenseLifecycleProjection::compile(&graph).unwrap();
        assert_eq!(dense.live_count(), n as usize);

        let start = Instant::now();
        for _ in 0..10 {
            black_box(graph.live_entities());
        }
        let tree_ns = start.elapsed().as_nanos();
        let start = Instant::now();
        for _ in 0..10 {
            black_box(dense.live_count());
        }
        let dense_ns = start.elapsed().as_nanos();
        let ratio_milli = tree_ns.saturating_mul(1_000) / dense_ns.max(1);
        println!("tree_ns={tree_ns} dense_ns={dense_ns} ratio_milli={ratio_milli}");
    }

    #[test]
    fn normalization_is_idempotent() {
        let mut graph = LifecycleGraph::default();
        graph.entities.extend([id(1), id(2), id(3), id(4)]);
        graph.roots.insert(id(1));
        graph.keeps_alive.insert(id(1), BTreeSet::from([id(2)]));
        graph.keeps_alive.insert(id(3), BTreeSet::from([id(4)]));
        graph.keeps_alive.insert(id(4), BTreeSet::from([id(3)]));

        let once = graph.normalize();
        assert_eq!(once.normalize(), once);
        assert_eq!(once.entities, BTreeSet::from([id(1), id(2)]));
    }

    #[test]
    fn merge_composes_intents_before_gc() {
        let mut lca = LifecycleGraph::default();
        lca.entities.extend([id(1), id(2), id(3)]);
        lca.roots.insert(id(1));
        lca.keeps_alive.insert(id(1), BTreeSet::from([id(2)]));

        let mut left = LifecycleIntent::default();
        left.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );

        let mut right = LifecycleIntent::default();
        right.set(LifecycleFact::Root(id(3)), true);
        right.set(
            LifecycleFact::KeepsAlive {
                parent: id(3),
                child: id(2),
            },
            true,
        );

        let merged = LifecycleGraph::merge_from_lca(&lca, &left, &right).unwrap();
        assert!(merged.entities.contains(&id(2)));
    }

    #[test]
    fn exhaustive_small_graphs_normalize_idempotently() {
        let nodes = [id(1), id(2), id(3)];
        let directed_edges = [
            (nodes[0], nodes[1]),
            (nodes[0], nodes[2]),
            (nodes[1], nodes[0]),
            (nodes[1], nodes[2]),
            (nodes[2], nodes[0]),
            (nodes[2], nodes[1]),
        ];

        for root_mask in 0_u8..8 {
            for edge_mask in 0_u8..64 {
                let mut graph = LifecycleGraph::default();
                graph.entities.extend(nodes);
                for (index, node) in nodes.iter().enumerate() {
                    if root_mask & (1 << index) != 0 {
                        graph.roots.insert(*node);
                    }
                }
                for (index, (parent, child)) in directed_edges.iter().enumerate() {
                    if edge_mask & (1 << index) != 0 {
                        graph.keeps_alive.entry(*parent).or_default().insert(*child);
                    }
                }

                let once = graph.normalize();
                let twice = once.normalize();
                assert_eq!(once, twice, "root_mask={root_mask}, edge_mask={edge_mask}");
            }
        }
    }

    #[test]
    fn non_conflicting_intent_merge_is_commutative() {
        let mut lca = LifecycleGraph::default();
        lca.entities.extend([id(1), id(2), id(3)]);
        lca.roots.insert(id(1));

        let mut left = LifecycleIntent::default();
        left.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            true,
        );
        let mut right = LifecycleIntent::default();
        right.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(3),
            },
            true,
        );

        let lr = LifecycleGraph::merge_from_lca(&lca, &left, &right).unwrap();
        let rl = LifecycleGraph::merge_from_lca(&lca, &right, &left).unwrap();
        assert_eq!(lr, rl);
    }

    #[test]
    fn contradictory_fact_edits_are_typed_conflicts() {
        let mut left = LifecycleIntent::default();
        let mut right = LifecycleIntent::default();
        let fact = LifecycleFact::Root(id(7));
        left.set(fact.clone(), true);
        right.set(fact.clone(), false);
        assert_eq!(
            LifecycleIntent::merge(&left, &right),
            Err(LifecycleConflict { fact })
        );
    }
}
