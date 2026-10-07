use std::collections::{BTreeMap, BTreeSet};

use crate::advisor::{ResourceFootprint, ResourceFootprintError, SemanticFiberDemand};

/// Exact observation of one pinned semantic-fiber canonical map.
///
/// Global cardinality is intentionally distinct from keyed joint mass: the former
/// is `(row_count, distinct_key_count)`, while the latter answers `mass(k)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FiberObservationKey {
    GlobalCardinality,
    JointMass,
    RowCanonicalKey,
    JointRows,
    SlotMass(usize),
    SlotRows(usize),
}

impl From<SemanticFiberDemand> for FiberObservationKey {
    fn from(value: SemanticFiberDemand) -> Self {
        match value {
            SemanticFiberDemand::GlobalCardinality => Self::GlobalCardinality,
            SemanticFiberDemand::JointMass => Self::JointMass,
            SemanticFiberDemand::RowCanonicalKey => Self::RowCanonicalKey,
            SemanticFiberDemand::JointRows => Self::JointRows,
            SemanticFiberDemand::SlotMass(slot) => Self::SlotMass(slot),
            SemanticFiberDemand::SlotRows(slot) => Self::SlotRows(slot),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiberObservationDemand {
    pub observation: FiberObservationKey,
    pub expected_reads: u128,
    pub baseline_read_work: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiberReadWorkEnvelope {
    pub lower: u128,
    pub upper: u128,
}

impl FiberReadWorkEnvelope {
    #[must_use]
    pub const fn new(lower: u128, upper: u128) -> Option<Self> {
        if lower <= upper {
            Some(Self { lower, upper })
        } else {
            None
        }
    }
}

/// Lifecycle contribution of one reconstructible physical resource atom.
///
/// Peak fields are additive marginal contributions for the owning atom model. Shared
/// atoms are unioned before these costs are summed, so one shared allocation is paid once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FiberResourceCost {
    pub retained_bytes: usize,
    pub build_work: u128,
    pub maintenance_work: u128,
    pub snapshot_peak_bytes: usize,
    pub transition_peak_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiberResourceSpec<R: Ord> {
    pub atom: R,
    pub cost: FiberResourceCost,
    /// Exact maintenance prerequisites of this atom.
    ///
    /// Dependencies are physical only: closure never changes the semantic
    /// observation realized by a plan. The compiler closes every candidate over
    /// this relation before costing or witness selection.
    pub requires: BTreeSet<R>,
}

/// One certified exact derivation of an observation from physical atoms.
///
/// Read cost belongs to the derivation, not the observation globally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiberRealizerRule<R: Ord> {
    pub observation: FiberObservationKey,
    pub resources: BTreeSet<R>,
    pub read_work: FiberReadWorkEnvelope,
}

/// Existing or otherwise protected physical point admitted to the same feasible set.
///
/// A protected point is not semantic authority. It is a performance reference that
/// synthesis must conservatively dominate before it may be displaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedFiberPlan<K: Ord, R: Ord> {
    pub key: K,
    pub resources: BTreeSet<R>,
    pub read_work: BTreeMap<FiberObservationKey, FiberReadWorkEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiberPlanOrigin<K> {
    Synthesized,
    Protected(K),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFiberRetentionPlan<K, R: Ord> {
    pub origin: FiberPlanOrigin<K>,
    pub resources: BTreeSet<R>,
    pub footprint: ResourceFootprint<R>,
    pub witnesses: BTreeMap<FiberObservationKey, FiberReadWorkEnvelope>,
    pub build_work: u128,
    pub maintenance_work: u128,
    pub snapshot_peak_bytes: usize,
    pub transition_peak_bytes: usize,
    pub conservative_net_work: u128,
    pub optimistic_net_work: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiberRetentionCompiler<K: Ord, R: Ord> {
    pub demands: Vec<FiberObservationDemand>,
    pub resources: Vec<FiberResourceSpec<R>>,
    pub realizers: Vec<FiberRealizerRule<R>>,
    pub protected: Vec<ProtectedFiberPlan<K, R>>,
    pub max_retained_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiberRetentionCompileError<R> {
    InvalidReadEnvelope,
    DuplicateDemand(FiberObservationKey),
    DuplicateResource(R),
    UnknownResource(R),
    MissingRealizer(FiberObservationKey),
    ResourceConflict(ResourceFootprintError<R>),
    CyclicResourceDependency(R),
    NoFeasiblePlan,
}

#[derive(Debug, Clone)]
struct EvaluatedPlan<K, R: Ord> {
    plan: CompiledFiberRetentionPlan<K, R>,
    total_upper_read_work: u128,
}

impl<K, R> FiberRetentionCompiler<K, R>
where
    K: Clone + Ord,
    R: Clone + Ord,
{
    pub fn compile(
        self,
    ) -> Result<CompiledFiberRetentionPlan<K, R>, FiberRetentionCompileError<R>> {
        let demands = canonical_demands(self.demands)?;
        let resources = canonical_resources(self.resources)?;
        validate_resource_dependencies(&resources)?;
        validate_realizers(&self.realizers, &resources)?;
        validate_protected(&self.protected, &resources)?;

        let synthesized = synthesize_candidates(
            &demands,
            &resources,
            &self.realizers,
            self.max_retained_bytes,
        )?;
        let protected = evaluate_protected(
            &demands,
            &resources,
            self.protected,
            self.max_retained_bytes,
        )?;

        let guarded_synthesized = if protected.is_empty() {
            synthesized
        } else {
            apply_competitive_firewall(&demands, synthesized, &protected)
        };

        select_plan(guarded_synthesized, protected)
            .map(|candidate| candidate.plan)
            .ok_or(FiberRetentionCompileError::NoFeasiblePlan)
    }
}

fn canonical_demands<R>(
    demands: Vec<FiberObservationDemand>,
) -> Result<BTreeMap<FiberObservationKey, FiberObservationDemand>, FiberRetentionCompileError<R>> {
    let mut canonical = BTreeMap::new();
    for demand in demands {
        if canonical.insert(demand.observation, demand).is_some() {
            return Err(FiberRetentionCompileError::DuplicateDemand(
                demand.observation,
            ));
        }
    }
    Ok(canonical)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FiberResourceNode<R: Ord> {
    cost: FiberResourceCost,
    requires: BTreeSet<R>,
}

fn canonical_resources<R: Clone + Ord>(
    resources: Vec<FiberResourceSpec<R>>,
) -> Result<BTreeMap<R, FiberResourceNode<R>>, FiberRetentionCompileError<R>> {
    let mut canonical = BTreeMap::new();
    for spec in resources {
        let node = FiberResourceNode {
            cost: spec.cost,
            requires: spec.requires,
        };
        if canonical.insert(spec.atom.clone(), node).is_some() {
            return Err(FiberRetentionCompileError::DuplicateResource(spec.atom));
        }
    }
    Ok(canonical)
}

fn validate_resource_dependencies<R: Clone + Ord>(
    resources: &BTreeMap<R, FiberResourceNode<R>>,
) -> Result<(), FiberRetentionCompileError<R>> {
    for (atom, node) in resources {
        for required in &node.requires {
            if !resources.contains_key(required) {
                return Err(FiberRetentionCompileError::UnknownResource(
                    required.clone(),
                ));
            }
        }
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        validate_dependency_dfs(atom, atom, resources, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn validate_dependency_dfs<R: Clone + Ord>(
    root: &R,
    atom: &R,
    resources: &BTreeMap<R, FiberResourceNode<R>>,
    visiting: &mut BTreeSet<R>,
    visited: &mut BTreeSet<R>,
) -> Result<(), FiberRetentionCompileError<R>> {
    if visited.contains(atom) {
        return Ok(());
    }
    if !visiting.insert(atom.clone()) {
        return Err(FiberRetentionCompileError::CyclicResourceDependency(
            root.clone(),
        ));
    }
    let node = resources
        .get(atom)
        .ok_or_else(|| FiberRetentionCompileError::UnknownResource(atom.clone()))?;
    for required in &node.requires {
        validate_dependency_dfs(root, required, resources, visiting, visited)?;
    }
    visiting.remove(atom);
    visited.insert(atom.clone());
    Ok(())
}

fn close_resources<R: Clone + Ord>(
    seed: &BTreeSet<R>,
    resources: &BTreeMap<R, FiberResourceNode<R>>,
) -> Result<BTreeSet<R>, FiberRetentionCompileError<R>> {
    let mut closed = seed.clone();
    let mut pending = seed.iter().cloned().collect::<Vec<_>>();
    while let Some(atom) = pending.pop() {
        let node = resources
            .get(&atom)
            .ok_or_else(|| FiberRetentionCompileError::UnknownResource(atom.clone()))?;
        for required in &node.requires {
            if closed.insert(required.clone()) {
                pending.push(required.clone());
            }
        }
    }
    Ok(closed)
}

fn validate_realizers<R: Clone + Ord>(
    realizers: &[FiberRealizerRule<R>],
    resources: &BTreeMap<R, FiberResourceNode<R>>,
) -> Result<(), FiberRetentionCompileError<R>> {
    for realizer in realizers {
        if realizer.read_work.lower > realizer.read_work.upper {
            return Err(FiberRetentionCompileError::InvalidReadEnvelope);
        }
        for atom in &realizer.resources {
            if !resources.contains_key(atom) {
                return Err(FiberRetentionCompileError::UnknownResource(atom.clone()));
            }
        }
    }
    Ok(())
}

fn validate_protected<K: Ord, R: Clone + Ord>(
    protected: &[ProtectedFiberPlan<K, R>],
    resources: &BTreeMap<R, FiberResourceNode<R>>,
) -> Result<(), FiberRetentionCompileError<R>> {
    for reference in protected {
        for envelope in reference.read_work.values() {
            if envelope.lower > envelope.upper {
                return Err(FiberRetentionCompileError::InvalidReadEnvelope);
            }
        }
        for atom in &reference.resources {
            if !resources.contains_key(atom) {
                return Err(FiberRetentionCompileError::UnknownResource(atom.clone()));
            }
        }
    }
    Ok(())
}

fn synthesize_candidates<K, R>(
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    resources: &BTreeMap<R, FiberResourceNode<R>>,
    realizers: &[FiberRealizerRule<R>],
    max_retained_bytes: usize,
) -> Result<Vec<EvaluatedPlan<K, R>>, FiberRetentionCompileError<R>>
where
    R: Clone + Ord,
{
    let mut shapes = BTreeSet::from([BTreeSet::<R>::new()]);
    for observation in demands.keys().copied() {
        let rules = realizers
            .iter()
            .filter(|rule| rule.observation == observation)
            .collect::<Vec<_>>();
        if rules.is_empty() {
            return Ok(Vec::new());
        }
        let mut next = BTreeSet::new();
        for shape in &shapes {
            for rule in &rules {
                let mut union = shape.clone();
                union.extend(rule.resources.iter().cloned());
                next.insert(close_resources(&union, resources)?);
            }
        }
        shapes = next;
    }

    let mut candidates = Vec::new();
    for shape in shapes {
        let witnesses = select_witnesses(demands, realizers, &shape)?;
        let candidate = evaluate_plan(
            FiberPlanOrigin::Synthesized,
            shape,
            witnesses,
            demands,
            resources,
        )?;
        if candidate.plan.footprint.estimated_bytes() <= max_retained_bytes {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn select_witnesses<R: Clone + Ord>(
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    realizers: &[FiberRealizerRule<R>],
    resources: &BTreeSet<R>,
) -> Result<BTreeMap<FiberObservationKey, FiberReadWorkEnvelope>, FiberRetentionCompileError<R>> {
    let mut selected = BTreeMap::new();
    for observation in demands.keys().copied() {
        let best = realizers
            .iter()
            .filter(|rule| rule.observation == observation && rule.resources.is_subset(resources))
            .min_by(|left, right| {
                left.read_work
                    .upper
                    .cmp(&right.read_work.upper)
                    .then_with(|| left.read_work.lower.cmp(&right.read_work.lower))
                    .then_with(|| left.resources.len().cmp(&right.resources.len()))
                    .then_with(|| left.resources.cmp(&right.resources))
            })
            .ok_or(FiberRetentionCompileError::MissingRealizer(observation))?;
        selected.insert(observation, best.read_work);
    }
    Ok(selected)
}

fn evaluate_protected<K, R>(
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    resources: &BTreeMap<R, FiberResourceNode<R>>,
    protected: Vec<ProtectedFiberPlan<K, R>>,
    max_retained_bytes: usize,
) -> Result<Vec<EvaluatedPlan<K, R>>, FiberRetentionCompileError<R>>
where
    K: Clone + Ord,
    R: Clone + Ord,
{
    let mut candidates = Vec::new();
    for reference in protected {
        if !demands
            .keys()
            .all(|observation| reference.read_work.contains_key(observation))
        {
            continue;
        }
        let witnesses = demands
            .keys()
            .filter_map(|observation| {
                reference
                    .read_work
                    .get(observation)
                    .copied()
                    .map(|work| (*observation, work))
            })
            .collect();
        let closed_resources = close_resources(&reference.resources, resources)?;
        let candidate = evaluate_plan(
            FiberPlanOrigin::Protected(reference.key),
            closed_resources,
            witnesses,
            demands,
            resources,
        )?;
        if candidate.plan.footprint.estimated_bytes() <= max_retained_bytes {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn evaluate_plan<K, R>(
    origin: FiberPlanOrigin<K>,
    atoms: BTreeSet<R>,
    witnesses: BTreeMap<FiberObservationKey, FiberReadWorkEnvelope>,
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    resources: &BTreeMap<R, FiberResourceNode<R>>,
) -> Result<EvaluatedPlan<K, R>, FiberRetentionCompileError<R>>
where
    R: Clone + Ord,
{
    let mut footprint = ResourceFootprint::default();
    let mut build_work = 0_u128;
    let mut maintenance_work = 0_u128;
    let mut snapshot_peak_bytes = 0_usize;
    let mut transition_peak_bytes = 0_usize;
    for atom in &atoms {
        let node = resources
            .get(atom)
            .ok_or_else(|| FiberRetentionCompileError::UnknownResource(atom.clone()))?;
        let cost = node.cost;
        footprint
            .union_in_place(&ResourceFootprint::from_atom(
                atom.clone(),
                cost.retained_bytes,
            ))
            .map_err(FiberRetentionCompileError::ResourceConflict)?;
        build_work = build_work.saturating_add(cost.build_work);
        maintenance_work = maintenance_work.saturating_add(cost.maintenance_work);
        snapshot_peak_bytes = snapshot_peak_bytes.saturating_add(cost.snapshot_peak_bytes);
        transition_peak_bytes = transition_peak_bytes.saturating_add(cost.transition_peak_bytes);
    }

    let optimistic_saved = saved_read_work(demands, &witnesses, false);
    let conservative_saved = saved_read_work(demands, &witnesses, true);
    let lifecycle_cost = build_work.saturating_add(maintenance_work);
    let total_upper_read_work = demands
        .iter()
        .map(|(observation, demand)| {
            witnesses.get(observation).map_or(u128::MAX, |work| {
                work.upper.saturating_mul(demand.expected_reads)
            })
        })
        .fold(0_u128, u128::saturating_add);

    Ok(EvaluatedPlan {
        plan: CompiledFiberRetentionPlan {
            origin,
            resources: atoms,
            footprint,
            witnesses,
            build_work,
            maintenance_work,
            snapshot_peak_bytes,
            transition_peak_bytes,
            conservative_net_work: conservative_saved.saturating_sub(lifecycle_cost),
            optimistic_net_work: optimistic_saved.saturating_sub(lifecycle_cost),
        },
        total_upper_read_work,
    })
}

fn saved_read_work(
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    witnesses: &BTreeMap<FiberObservationKey, FiberReadWorkEnvelope>,
    conservative: bool,
) -> u128 {
    demands
        .iter()
        .map(|(observation, demand)| {
            let read_work = witnesses.get(observation).map_or(u128::MAX, |work| {
                if conservative { work.upper } else { work.lower }
            });
            demand
                .baseline_read_work
                .saturating_sub(read_work)
                .saturating_mul(demand.expected_reads)
        })
        .fold(0_u128, u128::saturating_add)
}

fn apply_competitive_firewall<K, R>(
    demands: &BTreeMap<FiberObservationKey, FiberObservationDemand>,
    synthesized: Vec<EvaluatedPlan<K, R>>,
    protected: &[EvaluatedPlan<K, R>],
) -> Vec<EvaluatedPlan<K, R>>
where
    R: Ord,
{
    let best_optimistic_protected = protected
        .iter()
        .map(|reference| reference.plan.optimistic_net_work)
        .max()
        .unwrap_or(0);
    let protected_read_floor = demands
        .keys()
        .copied()
        .map(|observation| {
            let lower = protected
                .iter()
                .filter_map(|reference| reference.plan.witnesses.get(&observation))
                .map(|work| work.lower)
                .min()
                .unwrap_or(u128::MAX);
            (observation, lower)
        })
        .collect::<BTreeMap<_, _>>();

    synthesized
        .into_iter()
        .filter(|candidate| {
            let read_guard = demands.keys().all(|observation| {
                candidate
                    .plan
                    .witnesses
                    .get(observation)
                    .is_some_and(|work| work.upper <= protected_read_floor[observation])
            });
            let physical_guard = protected.iter().any(|reference| {
                candidate.plan.build_work <= reference.plan.build_work
                    && candidate.plan.maintenance_work <= reference.plan.maintenance_work
                    && candidate.plan.snapshot_peak_bytes <= reference.plan.snapshot_peak_bytes
                    && candidate.plan.transition_peak_bytes <= reference.plan.transition_peak_bytes
            });
            read_guard
                && physical_guard
                && candidate.plan.conservative_net_work >= best_optimistic_protected
        })
        .collect()
}

fn select_plan<K, R>(
    synthesized: Vec<EvaluatedPlan<K, R>>,
    protected: Vec<EvaluatedPlan<K, R>>,
) -> Option<EvaluatedPlan<K, R>>
where
    K: Ord,
    R: Ord,
{
    synthesized
        .into_iter()
        .chain(protected)
        .min_by(compare_evaluated_plans)
}

fn compare_evaluated_plans<K: Ord, R: Ord>(
    left: &EvaluatedPlan<K, R>,
    right: &EvaluatedPlan<K, R>,
) -> std::cmp::Ordering {
    right
        .plan
        .conservative_net_work
        .cmp(&left.plan.conservative_net_work)
        .then_with(|| {
            left.plan
                .footprint
                .estimated_bytes()
                .cmp(&right.plan.footprint.estimated_bytes())
        })
        .then_with(|| left.total_upper_read_work.cmp(&right.total_upper_read_work))
        .then_with(|| match (&left.plan.origin, &right.plan.origin) {
            (FiberPlanOrigin::Protected(left_key), FiberPlanOrigin::Protected(right_key)) => {
                left_key.cmp(right_key)
            }
            (FiberPlanOrigin::Protected(_), FiberPlanOrigin::Synthesized) => {
                std::cmp::Ordering::Less
            }
            (FiberPlanOrigin::Synthesized, FiberPlanOrigin::Protected(_)) => {
                std::cmp::Ordering::Greater
            }
            (FiberPlanOrigin::Synthesized, FiberPlanOrigin::Synthesized) => {
                left.plan.resources.cmp(&right.plan.resources)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Atom {
        GlobalCount,
        JointMass,
        DirectRowKey,
        InternedRows,
        ClassDecode,
        Projection,
        ProtectedRows,
        CanonicalKeyPool,
        SharedRowRoute,
        JointRows,
    }

    fn resource(atom: Atom, retained: usize, read_axes: u128) -> FiberResourceSpec<Atom> {
        FiberResourceSpec {
            atom,
            cost: FiberResourceCost {
                retained_bytes: retained,
                build_work: read_axes,
                maintenance_work: read_axes,
                snapshot_peak_bytes: retained,
                transition_peak_bytes: retained,
            },
            requires: BTreeSet::new(),
        }
    }

    fn resource_with_dependencies(
        atom: Atom,
        retained: usize,
        read_axes: u128,
        requires: BTreeSet<Atom>,
    ) -> FiberResourceSpec<Atom> {
        FiberResourceSpec {
            atom,
            cost: FiberResourceCost {
                retained_bytes: retained,
                build_work: read_axes,
                maintenance_work: read_axes,
                snapshot_peak_bytes: retained,
                transition_peak_bytes: retained,
            },
            requires,
        }
    }

    fn demand(observation: FiberObservationKey) -> FiberObservationDemand {
        FiberObservationDemand {
            observation,
            expected_reads: 100,
            baseline_read_work: 20,
        }
    }

    fn envelope(lower: u128, upper: u128) -> FiberReadWorkEnvelope {
        FiberReadWorkEnvelope { lower, upper }
    }

    #[test]
    fn global_cardinality_is_not_keyed_joint_mass() {
        let compiler = FiberRetentionCompiler::<u8, _> {
            demands: vec![demand(FiberObservationKey::GlobalCardinality)],
            resources: vec![
                resource(Atom::GlobalCount, 8, 0),
                resource(Atom::JointMass, 64, 0),
            ],
            realizers: vec![
                FiberRealizerRule {
                    observation: FiberObservationKey::GlobalCardinality,
                    resources: BTreeSet::from([Atom::GlobalCount]),
                    read_work: envelope(1, 1),
                },
                FiberRealizerRule {
                    observation: FiberObservationKey::JointMass,
                    resources: BTreeSet::from([Atom::JointMass]),
                    read_work: envelope(1, 1),
                },
            ],
            protected: vec![],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert_eq!(plan.resources, BTreeSet::from([Atom::GlobalCount]));
    }

    #[test]
    fn shared_atoms_are_charged_once_and_fastest_retained_witness_is_used() {
        let compiler = FiberRetentionCompiler::<u8, _> {
            demands: vec![
                demand(FiberObservationKey::RowCanonicalKey),
                demand(FiberObservationKey::SlotRows(0)),
            ],
            resources: vec![
                resource(Atom::InternedRows, 100, 1),
                resource(Atom::ClassDecode, 20, 1),
                resource(Atom::Projection, 30, 1),
                resource(Atom::DirectRowKey, 25, 1),
            ],
            realizers: vec![
                FiberRealizerRule {
                    observation: FiberObservationKey::RowCanonicalKey,
                    resources: BTreeSet::from([Atom::InternedRows, Atom::ClassDecode]),
                    read_work: envelope(4, 5),
                },
                FiberRealizerRule {
                    observation: FiberObservationKey::RowCanonicalKey,
                    resources: BTreeSet::from([Atom::DirectRowKey]),
                    read_work: envelope(1, 1),
                },
                FiberRealizerRule {
                    observation: FiberObservationKey::SlotRows(0),
                    resources: BTreeSet::from([Atom::InternedRows, Atom::Projection]),
                    read_work: envelope(2, 2),
                },
            ],
            protected: vec![],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert!(plan.resources.contains(&Atom::InternedRows));
        assert!(plan.resources.contains(&Atom::Projection));
        assert!(plan.resources.contains(&Atom::DirectRowKey));
        assert_eq!(plan.footprint.estimated_bytes(), 155);
        assert_eq!(
            plan.witnesses[&FiberObservationKey::RowCanonicalKey].upper,
            1
        );
    }

    #[test]
    fn overlapping_latency_envelope_keeps_protected_fast_path() {
        let compiler = FiberRetentionCompiler {
            demands: vec![demand(FiberObservationKey::SlotRows(0))],
            resources: vec![
                resource(Atom::Projection, 10, 0),
                resource(Atom::ProtectedRows, 100, 0),
            ],
            realizers: vec![FiberRealizerRule {
                observation: FiberObservationKey::SlotRows(0),
                resources: BTreeSet::from([Atom::Projection]),
                read_work: envelope(1, 3),
            }],
            protected: vec![ProtectedFiberPlan {
                key: 7_u8,
                resources: BTreeSet::from([Atom::ProtectedRows]),
                read_work: BTreeMap::from([(FiberObservationKey::SlotRows(0), envelope(1, 1))]),
            }],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert_eq!(plan.origin, FiberPlanOrigin::Protected(7));
    }

    #[test]
    fn protected_plan_remains_reachable_without_any_synthesized_realizer() {
        let compiler = FiberRetentionCompiler {
            demands: vec![demand(FiberObservationKey::JointRows)],
            resources: vec![resource(Atom::ProtectedRows, 100, 0)],
            realizers: vec![],
            protected: vec![ProtectedFiberPlan {
                key: 11_u8,
                resources: BTreeSet::from([Atom::ProtectedRows]),
                read_work: BTreeMap::from([(FiberObservationKey::JointRows, envelope(1, 1))]),
            }],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert_eq!(plan.origin, FiberPlanOrigin::Protected(11));
    }

    #[test]
    fn strict_measured_dominance_allows_synthesized_replacement() {
        let compiler = FiberRetentionCompiler {
            demands: vec![demand(FiberObservationKey::SlotRows(0))],
            resources: vec![
                resource(Atom::Projection, 10, 1),
                resource(Atom::ProtectedRows, 100, 8),
            ],
            realizers: vec![FiberRealizerRule {
                observation: FiberObservationKey::SlotRows(0),
                resources: BTreeSet::from([Atom::Projection]),
                read_work: envelope(2, 3),
            }],
            protected: vec![ProtectedFiberPlan {
                key: 9_u8,
                resources: BTreeSet::from([Atom::ProtectedRows]),
                read_work: BTreeMap::from([(FiberObservationKey::SlotRows(0), envelope(5, 5))]),
            }],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert_eq!(plan.origin, FiberPlanOrigin::Synthesized);
        assert_eq!(plan.resources, BTreeSet::from([Atom::Projection]));
    }

    #[test]
    fn aggregate_protected_read_floor_blocks_cross_reference_regression() {
        let compiler = FiberRetentionCompiler {
            demands: vec![
                demand(FiberObservationKey::RowCanonicalKey),
                demand(FiberObservationKey::SlotRows(0)),
            ],
            resources: vec![
                resource(Atom::DirectRowKey, 10, 0),
                resource(Atom::Projection, 10, 0),
                resource(Atom::ProtectedRows, 100, 0),
                resource(Atom::ClassDecode, 100, 0),
            ],
            realizers: vec![
                FiberRealizerRule {
                    observation: FiberObservationKey::RowCanonicalKey,
                    resources: BTreeSet::from([Atom::DirectRowKey]),
                    read_work: envelope(2, 2),
                },
                FiberRealizerRule {
                    observation: FiberObservationKey::SlotRows(0),
                    resources: BTreeSet::from([Atom::Projection]),
                    read_work: envelope(2, 2),
                },
            ],
            protected: vec![
                ProtectedFiberPlan {
                    key: 1_u8,
                    resources: BTreeSet::from([Atom::ProtectedRows]),
                    read_work: BTreeMap::from([
                        (FiberObservationKey::RowCanonicalKey, envelope(1, 1)),
                        (FiberObservationKey::SlotRows(0), envelope(5, 5)),
                    ]),
                },
                ProtectedFiberPlan {
                    key: 2_u8,
                    resources: BTreeSet::from([Atom::ClassDecode]),
                    read_work: BTreeMap::from([
                        (FiberObservationKey::RowCanonicalKey, envelope(5, 5)),
                        (FiberObservationKey::SlotRows(0), envelope(1, 1)),
                    ]),
                },
            ],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert!(matches!(plan.origin, FiberPlanOrigin::Protected(_)));
    }

    #[test]
    fn retained_budget_can_fail_closed_when_no_reference_fits() {
        let compiler = FiberRetentionCompiler::<u8, _> {
            demands: vec![demand(FiberObservationKey::RowCanonicalKey)],
            resources: vec![resource(Atom::DirectRowKey, 100, 0)],
            realizers: vec![FiberRealizerRule {
                observation: FiberObservationKey::RowCanonicalKey,
                resources: BTreeSet::from([Atom::DirectRowKey]),
                read_work: envelope(1, 1),
            }],
            protected: vec![],
            max_retained_bytes: 99,
        };
        assert_eq!(
            compiler.compile(),
            Err(FiberRetentionCompileError::NoFeasiblePlan)
        );
    }
    #[test]
    fn maintenance_closure_adds_shared_key_pool_once_across_independent_atoms() {
        let compiler = FiberRetentionCompiler::<u8, _> {
            demands: vec![
                demand(FiberObservationKey::RowCanonicalKey),
                demand(FiberObservationKey::JointRows),
            ],
            resources: vec![
                resource(Atom::CanonicalKeyPool, 100, 1),
                resource_with_dependencies(
                    Atom::SharedRowRoute,
                    40,
                    1,
                    BTreeSet::from([Atom::CanonicalKeyPool]),
                ),
                resource_with_dependencies(
                    Atom::JointRows,
                    60,
                    1,
                    BTreeSet::from([Atom::CanonicalKeyPool]),
                ),
            ],
            realizers: vec![
                FiberRealizerRule {
                    observation: FiberObservationKey::RowCanonicalKey,
                    resources: BTreeSet::from([Atom::SharedRowRoute]),
                    read_work: envelope(1, 1),
                },
                FiberRealizerRule {
                    observation: FiberObservationKey::JointRows,
                    resources: BTreeSet::from([Atom::JointRows]),
                    read_work: envelope(1, 1),
                },
            ],
            protected: vec![],
            max_retained_bytes: usize::MAX,
        };
        let plan = compiler.compile().unwrap();
        assert_eq!(
            plan.resources,
            BTreeSet::from([
                Atom::CanonicalKeyPool,
                Atom::SharedRowRoute,
                Atom::JointRows,
            ])
        );
        assert_eq!(plan.footprint.estimated_bytes(), 200);
    }

    #[test]
    fn cyclic_physical_dependencies_fail_closed() {
        let compiler = FiberRetentionCompiler::<u8, _> {
            demands: vec![demand(FiberObservationKey::RowCanonicalKey)],
            resources: vec![
                resource_with_dependencies(
                    Atom::CanonicalKeyPool,
                    100,
                    1,
                    BTreeSet::from([Atom::SharedRowRoute]),
                ),
                resource_with_dependencies(
                    Atom::SharedRowRoute,
                    40,
                    1,
                    BTreeSet::from([Atom::CanonicalKeyPool]),
                ),
            ],
            realizers: vec![FiberRealizerRule {
                observation: FiberObservationKey::RowCanonicalKey,
                resources: BTreeSet::from([Atom::SharedRowRoute]),
                read_work: envelope(1, 1),
            }],
            protected: vec![],
            max_retained_bytes: usize::MAX,
        };
        assert!(matches!(
            compiler.compile(),
            Err(FiberRetentionCompileError::CyclicResourceDependency(_))
        ));
    }
}
