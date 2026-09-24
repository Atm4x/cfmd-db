use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use std::collections::VecDeque;

pub use kernel_grounded_closure::GroundedAtomId;
use kernel_grounded_closure::{
    GroundedClosureError, GroundedProgram, GroundedRule, GroundedWitness,
    solve as solve_grounded_closure,
};
use kernel_types::EntityId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachabilityProgram {
    pub universe: BTreeSet<EntityId>,
    pub seeds: BTreeSet<EntityId>,
    pub edges: BTreeSet<(EntityId, EntityId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachabilityCertificate {
    pub reachable: BTreeSet<EntityId>,
    pub rank: BTreeMap<EntityId, usize>,
    pub parent: BTreeMap<EntityId, EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixpointError {
    SeedOutsideUniverse(EntityId),
    EdgeOutsideUniverse(EntityId),
    MissingRank(EntityId),
    SeedHasNonZeroRank(EntityId),
    MissingParent(EntityId),
    ParentNotReachable(EntityId),
    ParentRankNotSmaller(EntityId),
    MissingWitnessEdge { parent: EntityId, child: EntityId },
    NotClosedUnderEdge { parent: EntityId, child: EntityId },
    CertificateContainsOutsideUniverse(EntityId),
    MissingSeed(EntityId),
    GroundedClosure(GroundedClosureError),
}

impl From<GroundedClosureError> for FixpointError {
    fn from(value: GroundedClosureError) -> Self {
        Self::GroundedClosure(value)
    }
}

impl ReachabilityProgram {
    pub fn validate(&self) -> Result<(), FixpointError> {
        for &seed in &self.seeds {
            if !self.universe.contains(&seed) {
                return Err(FixpointError::SeedOutsideUniverse(seed));
            }
        }
        for &(left, right) in &self.edges {
            if !self.universe.contains(&left) {
                return Err(FixpointError::EdgeOutsideUniverse(left));
            }
            if !self.universe.contains(&right) {
                return Err(FixpointError::EdgeOutsideUniverse(right));
            }
        }
        Ok(())
    }
}

pub fn solve(program: &ReachabilityProgram) -> Result<ReachabilityCertificate, FixpointError> {
    program.validate()?;
    let entity_by_atom = program.universe.iter().copied().collect::<Vec<_>>();
    let atom_by_entity = entity_by_atom
        .iter()
        .enumerate()
        .map(|(index, &entity)| (entity, GroundedAtomId::new(index)))
        .collect::<BTreeMap<_, _>>();
    let rule_edges = program.edges.iter().copied().collect::<Vec<_>>();
    let rules = rule_edges
        .iter()
        .map(|&(parent, child)| {
            GroundedRule::new([atom_by_entity[&parent]], atom_by_entity[&child])
        })
        .collect::<Vec<_>>();
    let seeds = program
        .seeds
        .iter()
        .map(|seed| atom_by_entity[seed])
        .collect::<Vec<_>>();
    let grounded = GroundedProgram::new(entity_by_atom.len(), seeds, rules)?;
    let (grounded_certificate, _) = solve_grounded_closure(&grounded);

    let mut reachable = BTreeSet::new();
    let mut rank = BTreeMap::new();
    let mut parent = BTreeMap::new();
    for atom in grounded_certificate.live_atoms() {
        let entity = entity_by_atom[atom.index()];
        reachable.insert(entity);
        rank.insert(
            entity,
            grounded_certificate
                .rank(atom)
                .ok_or(FixpointError::MissingRank(entity))?,
        );
        if let Some(GroundedWitness::Rule(rule_id)) = grounded_certificate.witness(atom) {
            let (witness_parent, witness_child) = rule_edges[rule_id.index()];
            debug_assert_eq!(witness_child, entity);
            parent.insert(entity, witness_parent);
        }
    }

    Ok(ReachabilityCertificate {
        reachable,
        rank,
        parent,
    })
}

pub fn check(
    program: &ReachabilityProgram,
    certificate: &ReachabilityCertificate,
) -> Result<(), FixpointError> {
    program.validate()?;

    for &value in &certificate.reachable {
        if !program.universe.contains(&value) {
            return Err(FixpointError::CertificateContainsOutsideUniverse(value));
        }
    }
    for &seed in &program.seeds {
        if !certificate.reachable.contains(&seed) {
            return Err(FixpointError::MissingSeed(seed));
        }
        let rank = certificate
            .rank
            .get(&seed)
            .ok_or(FixpointError::MissingRank(seed))?;
        if *rank != 0 {
            return Err(FixpointError::SeedHasNonZeroRank(seed));
        }
    }

    for &value in &certificate.reachable {
        let value_rank = *certificate
            .rank
            .get(&value)
            .ok_or(FixpointError::MissingRank(value))?;
        if program.seeds.contains(&value) {
            continue;
        }
        let witness_parent = *certificate
            .parent
            .get(&value)
            .ok_or(FixpointError::MissingParent(value))?;
        if !certificate.reachable.contains(&witness_parent) {
            return Err(FixpointError::ParentNotReachable(witness_parent));
        }
        let parent_rank = *certificate
            .rank
            .get(&witness_parent)
            .ok_or(FixpointError::MissingRank(witness_parent))?;
        if parent_rank >= value_rank {
            return Err(FixpointError::ParentRankNotSmaller(value));
        }
        if !program.edges.contains(&(witness_parent, value)) {
            return Err(FixpointError::MissingWitnessEdge {
                parent: witness_parent,
                child: value,
            });
        }
    }

    for &(left, right) in &program.edges {
        if certificate.reachable.contains(&left) && !certificate.reachable.contains(&right) {
            return Err(FixpointError::NotClosedUnderEdge {
                parent: left,
                child: right,
            });
        }
    }

    Ok(())
}

pub trait CertifiedFixpointSolver {
    type Program;
    type Certificate;
    type Output;
    type Error;

    fn solve(program: &Self::Program) -> Result<Self::Certificate, Self::Error>;
    fn check(program: &Self::Program, certificate: &Self::Certificate) -> Result<(), Self::Error>;
    fn output(certificate: &Self::Certificate) -> &Self::Output;
}

pub use kernel_proof::CheckedCertificate;

pub struct FixpointCertificateChecker<S>(std::marker::PhantomData<S>);

impl<S: CertifiedFixpointSolver> kernel_proof::CertificateChecker
    for FixpointCertificateChecker<S>
{
    type Spec = S::Program;
    type Certificate = S::Certificate;
    type Error = S::Error;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        S::check(spec, certificate)
    }
}

pub fn verify<S: CertifiedFixpointSolver>(
    program: &S::Program,
    certificate: S::Certificate,
) -> Result<CheckedCertificate<FixpointCertificateChecker<S>>, S::Error>
where
    S::Program: Clone,
{
    kernel_proof::verify_certificate::<FixpointCertificateChecker<S>>(program, certificate)
}

pub struct ReachabilitySolver;

impl CertifiedFixpointSolver for ReachabilitySolver {
    type Program = ReachabilityProgram;
    type Certificate = ReachabilityCertificate;
    type Output = BTreeSet<EntityId>;
    type Error = FixpointError;

    fn solve(program: &Self::Program) -> Result<Self::Certificate, Self::Error> {
        solve(program)
    }

    fn check(program: &Self::Program, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        check(program, certificate)
    }

    fn output(certificate: &Self::Certificate) -> &Self::Output {
        &certificate.reachable
    }
}

const BIG_NAT_BASE: u64 = 1_000_000_000;

/// Exact non-negative integer used by positive recursive Bag evaluation.
/// Limbs are little-endian base 1e9 so finite proof-tree multiplicity never
/// silently saturates into the semantic `Infinite` value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BigNatural {
    limbs: Vec<u32>,
}

impl BigNatural {
    #[must_use]
    pub fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    #[must_use]
    pub fn one() -> Self {
        Self::from_u64(1)
    }

    #[must_use]
    pub fn from_u64(mut value: u64) -> Self {
        let mut limbs = Vec::new();
        while value != 0 {
            limbs.push((value % BIG_NAT_BASE) as u32);
            value /= BIG_NAT_BASE;
        }
        Self { limbs }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    pub fn add_assign(&mut self, rhs: &Self) {
        let len = self.limbs.len().max(rhs.limbs.len());
        self.limbs.resize(len, 0);
        let mut carry = 0_u64;
        for index in 0..len {
            let left = u64::from(self.limbs[index]);
            let right = rhs.limbs.get(index).copied().map_or(0, u64::from);
            let sum = left + right + carry;
            self.limbs[index] = (sum % BIG_NAT_BASE) as u32;
            carry = sum / BIG_NAT_BASE;
        }
        if carry != 0 {
            self.limbs
                .push(u32::try_from(carry).expect("carry is below BigNatural base"));
        }
    }

    #[must_use]
    pub fn multiplied(&self, rhs: &Self) -> Self {
        if self.is_zero() || rhs.is_zero() {
            return Self::zero();
        }
        let mut accum = vec![0_u128; self.limbs.len() + rhs.limbs.len()];
        for (left_index, &left) in self.limbs.iter().enumerate() {
            for (right_index, &right) in rhs.limbs.iter().enumerate() {
                accum[left_index + right_index] += u128::from(left) * u128::from(right);
            }
        }
        let base = u128::from(BIG_NAT_BASE);
        let mut limbs = Vec::with_capacity(accum.len() + 1);
        let mut carry = 0_u128;
        for value in accum {
            let value = value + carry;
            limbs.push(u32::try_from(value % base).expect("limb is below BigNatural base"));
            carry = value / base;
        }
        while carry != 0 {
            limbs.push(u32::try_from(carry % base).expect("limb is below BigNatural base"));
            carry /= base;
        }
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self { limbs }
    }

    #[must_use]
    pub fn to_decimal_string(&self) -> String {
        let Some((&last, rest)) = self.limbs.split_last() else {
            return "0".to_owned();
        };
        let mut out = last.to_string();
        for limb in rest.iter().rev() {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{limb:09}");
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NaturalInfinity {
    Finite(BigNatural),
    Infinite,
}

impl NaturalInfinity {
    #[must_use]
    pub fn zero() -> Self {
        Self::Finite(BigNatural::zero())
    }

    #[must_use]
    pub fn finite_u64(value: u64) -> Self {
        Self::Finite(BigNatural::from_u64(value))
    }

    #[must_use]
    pub fn is_infinite(&self) -> bool {
        matches!(self, Self::Infinite)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveBagRule {
    pub body: Vec<GroundedAtomId>,
    pub head: GroundedAtomId,
    pub coefficient: u64,
}

impl PositiveBagRule {
    #[must_use]
    pub fn new(
        body: impl IntoIterator<Item = GroundedAtomId>,
        head: GroundedAtomId,
        coefficient: u64,
    ) -> Self {
        Self {
            body: body.into_iter().collect(),
            head,
            coefficient,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveBagProgram {
    atom_count: usize,
    seed_multiplicity: Vec<u64>,
    rules: Vec<PositiveBagRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositiveBagFixpointError {
    AtomOutsideUniverse(GroundedAtomId),
    SeedShapeMismatch,
    CertificateShapeMismatch,
    Support(GroundedClosureError),
    FiniteDependencyCycle,
}

impl From<GroundedClosureError> for PositiveBagFixpointError {
    fn from(value: GroundedClosureError) -> Self {
        Self::Support(value)
    }
}

impl PositiveBagProgram {
    pub fn new(
        atom_count: usize,
        seed_multiplicity: Vec<u64>,
        rules: Vec<PositiveBagRule>,
    ) -> Result<Self, PositiveBagFixpointError> {
        if seed_multiplicity.len() != atom_count {
            return Err(PositiveBagFixpointError::SeedShapeMismatch);
        }
        for rule in &rules {
            if rule.head.index() >= atom_count {
                return Err(PositiveBagFixpointError::AtomOutsideUniverse(rule.head));
            }
            for &atom in &rule.body {
                if atom.index() >= atom_count {
                    return Err(PositiveBagFixpointError::AtomOutsideUniverse(atom));
                }
            }
        }
        Ok(Self {
            atom_count,
            seed_multiplicity,
            rules,
        })
    }

    #[must_use]
    pub const fn atom_count(&self) -> usize {
        self.atom_count
    }

    #[must_use]
    pub fn rules(&self) -> &[PositiveBagRule] {
        &self.rules
    }

    #[must_use]
    pub fn seed_multiplicity(&self) -> &[u64] {
        &self.seed_multiplicity
    }

    fn support_program(&self) -> Result<GroundedProgram, GroundedClosureError> {
        let seeds = self
            .seed_multiplicity
            .iter()
            .enumerate()
            .filter_map(|(index, &weight)| (weight != 0).then_some(GroundedAtomId::new(index)));
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.coefficient != 0)
            .map(|rule| GroundedRule::new(rule.body.iter().copied(), rule.head));
        GroundedProgram::new(self.atom_count, seeds, rules.collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveBagCertificate {
    support: kernel_grounded_closure::GroundedCertificate,
    multiplicity: Vec<NaturalInfinity>,
}

impl PositiveBagCertificate {
    #[must_use]
    pub fn multiplicity(&self, atom: GroundedAtomId) -> Option<&NaturalInfinity> {
        self.multiplicity.get(atom.index())
    }

    #[must_use]
    pub fn multiplicities(&self) -> &[NaturalInfinity] {
        &self.multiplicity
    }

    #[must_use]
    pub const fn support(&self) -> &kernel_grounded_closure::GroundedCertificate {
        &self.support
    }
}

fn active_dependency_graph(
    program: &PositiveBagProgram,
    support: &kernel_grounded_closure::GroundedCertificate,
) -> Vec<Vec<usize>> {
    let mut graph = vec![Vec::new(); program.atom_count];
    for rule in &program.rules {
        if rule.coefficient == 0
            || !support.is_live(rule.head)
            || !rule.body.iter().all(|&atom| support.is_live(atom))
        {
            continue;
        }
        for &premise in &rule.body {
            let edges = &mut graph[premise.index()];
            if !edges.contains(&rule.head.index()) {
                edges.push(rule.head.index());
            }
        }
    }
    graph
}

fn dfs_order(start: usize, graph: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
    if seen[start] {
        return;
    }
    let mut stack = vec![(start, false)];
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            order.push(node);
            continue;
        }
        if seen[node] {
            continue;
        }
        seen[node] = true;
        stack.push((node, true));
        for &next in graph[node].iter().rev() {
            if !seen[next] {
                stack.push((next, false));
            }
        }
    }
}

fn dfs_component(start: usize, reverse: &[Vec<usize>], seen: &mut [bool], out: &mut Vec<usize>) {
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        if seen[node] {
            continue;
        }
        seen[node] = true;
        out.push(node);
        for &next in &reverse[node] {
            if !seen[next] {
                stack.push(next);
            }
        }
    }
}

fn cyclic_live_atoms(graph: &[Vec<usize>], live: &[bool]) -> Vec<bool> {
    let mut reverse = vec![Vec::new(); graph.len()];
    for (source, edges) in graph.iter().enumerate() {
        for &target in edges {
            reverse[target].push(source);
        }
    }
    let mut seen = vec![false; graph.len()];
    let mut order = Vec::new();
    for (node, &is_live) in live.iter().enumerate().take(graph.len()) {
        if is_live {
            dfs_order(node, graph, &mut seen, &mut order);
        }
    }
    seen.fill(false);
    let mut cyclic = vec![false; graph.len()];
    for &node in order.iter().rev() {
        if seen[node] || !live[node] {
            continue;
        }
        let mut component = Vec::new();
        dfs_component(node, &reverse, &mut seen, &mut component);
        let is_cycle = component.len() > 1 || graph[node].contains(&node);
        if is_cycle {
            for member in component {
                cyclic[member] = true;
            }
        }
    }
    cyclic
}

fn classify_infinite(
    program: &PositiveBagProgram,
    support: &kernel_grounded_closure::GroundedCertificate,
) -> (Vec<Vec<usize>>, Vec<bool>) {
    let graph = active_dependency_graph(program, support);
    let live = (0..program.atom_count)
        .map(|index| support.is_live(GroundedAtomId::new(index)))
        .collect::<Vec<_>>();
    let mut infinite = cyclic_live_atoms(&graph, &live);
    let mut queue = std::collections::VecDeque::new();
    for (index, &value) in infinite.iter().enumerate() {
        if value {
            queue.push_back(index);
        }
    }
    while let Some(source) = queue.pop_front() {
        for &target in &graph[source] {
            if live[target] && !infinite[target] {
                infinite[target] = true;
                queue.push_back(target);
            }
        }
    }
    (graph, infinite)
}

pub fn solve_positive_bag(
    program: &PositiveBagProgram,
) -> Result<PositiveBagCertificate, PositiveBagFixpointError> {
    let support_program = program.support_program()?;
    let (support, _) = solve_grounded_closure(&support_program);
    kernel_grounded_closure::check(&support_program, &support)?;
    let (graph, infinite) = classify_infinite(program, &support);
    let live = (0..program.atom_count)
        .map(|index| support.is_live(GroundedAtomId::new(index)))
        .collect::<Vec<_>>();

    let mut indegree = vec![0_usize; program.atom_count];
    for head in 0..program.atom_count {
        if !live[head] || infinite[head] {
            continue;
        }
        let mut prerequisites = BTreeSet::new();
        for rule in &program.rules {
            if rule.coefficient == 0 || rule.head.index() != head {
                continue;
            }
            if !rule.body.iter().all(|&atom| support.is_live(atom)) {
                continue;
            }
            for &atom in &rule.body {
                if !infinite[atom.index()] {
                    prerequisites.insert(atom.index());
                }
            }
        }
        indegree[head] = prerequisites.len();
    }

    let mut queue = std::collections::VecDeque::new();
    for index in 0..program.atom_count {
        if live[index] && !infinite[index] && indegree[index] == 0 {
            queue.push_back(index);
        }
    }
    let mut finite = vec![BigNatural::zero(); program.atom_count];
    let mut processed = 0_usize;
    while let Some(atom) = queue.pop_front() {
        processed += 1;
        let mut value = BigNatural::from_u64(program.seed_multiplicity[atom]);
        for rule in &program.rules {
            if rule.coefficient == 0 || rule.head.index() != atom {
                continue;
            }
            if !rule.body.iter().all(|&premise| support.is_live(premise)) {
                continue;
            }
            let mut product = BigNatural::from_u64(rule.coefficient);
            for &premise in &rule.body {
                product = product.multiplied(&finite[premise.index()]);
            }
            value.add_assign(&product);
        }
        finite[atom] = value;
        for &target in &graph[atom] {
            if !live[target] || infinite[target] {
                continue;
            }
            indegree[target] = indegree[target].saturating_sub(1);
            if indegree[target] == 0 {
                queue.push_back(target);
            }
        }
    }
    let expected = (0..program.atom_count)
        .filter(|&index| live[index] && !infinite[index])
        .count();
    if processed != expected {
        return Err(PositiveBagFixpointError::FiniteDependencyCycle);
    }

    let multiplicity = (0..program.atom_count)
        .map(|index| {
            if !live[index] {
                NaturalInfinity::zero()
            } else if infinite[index] {
                NaturalInfinity::Infinite
            } else {
                NaturalInfinity::Finite(finite[index].clone())
            }
        })
        .collect();
    Ok(PositiveBagCertificate {
        support,
        multiplicity,
    })
}

pub fn check_positive_bag(
    program: &PositiveBagProgram,
    certificate: &PositiveBagCertificate,
) -> Result<(), PositiveBagFixpointError> {
    if certificate.multiplicity.len() != program.atom_count {
        return Err(PositiveBagFixpointError::CertificateShapeMismatch);
    }
    let support_program = program.support_program()?;
    kernel_grounded_closure::check(&support_program, &certificate.support)?;
    let expected = solve_positive_bag(program)?;
    if expected.multiplicity != certificate.multiplicity {
        return Err(PositiveBagFixpointError::CertificateShapeMismatch);
    }
    for index in 0..program.atom_count {
        let atom = GroundedAtomId::new(index);
        if expected.support.is_live(atom) != certificate.support.is_live(atom) {
            return Err(PositiveBagFixpointError::CertificateShapeMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    #[test]
    fn solver_emits_checkable_least_fixpoint_certificate() {
        let program = ReachabilityProgram {
            universe: BTreeSet::from([id(1), id(2), id(3), id(4)]),
            seeds: BTreeSet::from([id(1)]),
            edges: BTreeSet::from([(id(1), id(2)), (id(2), id(3))]),
        };
        let certificate = solve(&program).unwrap();
        assert_eq!(certificate.reachable, BTreeSet::from([id(1), id(2), id(3)]));
        assert_eq!(check(&program, &certificate), Ok(()));
    }

    #[test]
    fn checker_rejects_closed_but_nonleast_superset_without_witness() {
        let program = ReachabilityProgram {
            universe: BTreeSet::from([id(1), id(2), id(9)]),
            seeds: BTreeSet::from([id(1)]),
            edges: BTreeSet::from([(id(1), id(2))]),
        };
        let certificate = ReachabilityCertificate {
            reachable: BTreeSet::from([id(1), id(2), id(9)]),
            rank: BTreeMap::from([(id(1), 0), (id(2), 1), (id(9), 1)]),
            parent: BTreeMap::from([(id(2), id(1))]),
        };
        assert_eq!(
            check(&program, &certificate),
            Err(FixpointError::MissingParent(id(9)))
        );
    }

    #[test]
    fn checker_rejects_nonclosed_candidate() {
        let program = ReachabilityProgram {
            universe: BTreeSet::from([id(1), id(2)]),
            seeds: BTreeSet::from([id(1)]),
            edges: BTreeSet::from([(id(1), id(2))]),
        };
        let certificate = ReachabilityCertificate {
            reachable: BTreeSet::from([id(1)]),
            rank: BTreeMap::from([(id(1), 0)]),
            parent: BTreeMap::new(),
        };
        assert_eq!(
            check(&program, &certificate),
            Err(FixpointError::NotClosedUnderEdge {
                parent: id(1),
                child: id(2)
            })
        );
    }
    #[test]
    fn generic_solver_boundary_exposes_only_checked_certificate_after_verification() {
        let program = ReachabilityProgram {
            universe: BTreeSet::from([id(1), id(2)]),
            seeds: BTreeSet::from([id(1)]),
            edges: BTreeSet::from([(id(1), id(2))]),
        };
        let certificate = ReachabilitySolver::solve(&program).unwrap();
        let checked = verify::<ReachabilitySolver>(&program, certificate).unwrap();
        assert_eq!(
            ReachabilitySolver::output(checked.certificate()),
            &BTreeSet::from([id(1), id(2)])
        );
    }

    #[test]
    fn adjacency_lowering_preserves_relation_scan_certificate_exactly() {
        fn reference(program: &ReachabilityProgram) -> ReachabilityCertificate {
            let mut reachable = BTreeSet::new();
            let mut rank = BTreeMap::new();
            let mut parent = BTreeMap::new();
            let mut queue = VecDeque::new();
            for &seed in &program.seeds {
                reachable.insert(seed);
                rank.insert(seed, 0);
                queue.push_back(seed);
            }
            while let Some(current) = queue.pop_front() {
                let current_rank = rank[&current];
                for &(left, right) in &program.edges {
                    if left == current && reachable.insert(right) {
                        rank.insert(right, current_rank + 1);
                        parent.insert(right, current);
                        queue.push_back(right);
                    }
                }
            }
            ReachabilityCertificate {
                reachable,
                rank,
                parent,
            }
        }

        for salt in 0_u64..32 {
            let universe = (0_u64..24)
                .map(|raw| EntityId::new(u128::from(raw)))
                .collect::<BTreeSet<_>>();
            let seeds = BTreeSet::from([EntityId::new(u128::from(salt % 5))]);
            let mut edges = BTreeSet::new();
            for left in 0_u64..24 {
                for right in 0_u64..24 {
                    if left != right && (left * 17 + right * 31 + salt * 13) % 19 == 0 {
                        edges.insert((
                            EntityId::new(u128::from(left)),
                            EntityId::new(u128::from(right)),
                        ));
                    }
                }
            }
            let program = ReachabilityProgram {
                universe,
                seeds,
                edges,
            };
            assert_eq!(solve(&program).unwrap(), reference(&program), "salt={salt}");
        }
    }
}

#[cfg(test)]
mod positive_bag_tests {
    use super::*;

    fn atom(index: usize) -> GroundedAtomId {
        GroundedAtomId::new(index)
    }

    #[test]
    fn finite_positive_recursion_counts_exact_proof_trees_without_row_expansion() {
        // a has two base witnesses; b derives once from a; c derives from a*b.
        let program = PositiveBagProgram::new(
            3,
            vec![2, 0, 0],
            vec![
                PositiveBagRule::new([atom(0)], atom(1), 3),
                PositiveBagRule::new([atom(0), atom(1)], atom(2), 1),
            ],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert_eq!(
            certificate.multiplicity(atom(0)),
            Some(&NaturalInfinity::finite_u64(2))
        );
        assert_eq!(
            certificate.multiplicity(atom(1)),
            Some(&NaturalInfinity::finite_u64(6))
        );
        assert_eq!(
            certificate.multiplicity(atom(2)),
            Some(&NaturalInfinity::finite_u64(12))
        );
        check_positive_bag(&program, &certificate).unwrap();
    }

    #[test]
    fn grounded_productive_cycle_is_infinite_and_propagates_downstream() {
        let program = PositiveBagProgram::new(
            3,
            vec![1, 0, 0],
            vec![
                PositiveBagRule::new([atom(0)], atom(1), 1),
                PositiveBagRule::new([atom(1)], atom(0), 1),
                PositiveBagRule::new([atom(1)], atom(2), 1),
            ],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert!(certificate.multiplicity(atom(0)).unwrap().is_infinite());
        assert!(certificate.multiplicity(atom(1)).unwrap().is_infinite());
        assert!(certificate.multiplicity(atom(2)).unwrap().is_infinite());
    }

    #[test]
    fn ungrounded_cycle_stays_zero_in_least_fixpoint() {
        let program = PositiveBagProgram::new(
            2,
            vec![0, 0],
            vec![
                PositiveBagRule::new([atom(0)], atom(1), 1),
                PositiveBagRule::new([atom(1)], atom(0), 1),
            ],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert_eq!(
            certificate.multiplicity(atom(0)),
            Some(&NaturalInfinity::zero())
        );
        assert_eq!(
            certificate.multiplicity(atom(1)),
            Some(&NaturalInfinity::zero())
        );
    }

    #[test]
    fn duplicate_recursive_occurrence_multiplies_proof_counts() {
        let program = PositiveBagProgram::new(
            2,
            vec![3, 0],
            vec![PositiveBagRule::new([atom(0), atom(0)], atom(1), 2)],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert_eq!(
            certificate.multiplicity(atom(1)),
            Some(&NaturalInfinity::finite_u64(18))
        );
    }

    #[test]
    fn hostile_cycle_with_dead_conjunctive_premise_is_not_productive() {
        // atom 0 is grounded. atom 2 is dead, so 0*2 -> 1 never fires and
        // the apparent 0 <-> 1 graph cycle is not part of the live program.
        let program = PositiveBagProgram::new(
            3,
            vec![1, 0, 0],
            vec![
                PositiveBagRule::new([atom(0), atom(2)], atom(1), 1),
                PositiveBagRule::new([atom(1)], atom(0), 1),
            ],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert_eq!(
            certificate.multiplicity(atom(0)),
            Some(&NaturalInfinity::finite_u64(1))
        );
        assert_eq!(
            certificate.multiplicity(atom(1)),
            Some(&NaturalInfinity::zero())
        );
        assert_eq!(
            certificate.multiplicity(atom(2)),
            Some(&NaturalInfinity::zero())
        );
    }

    #[test]
    fn randomized_acyclic_programs_match_independent_u128_oracle() {
        fn next(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *state
        }
        let mut state = 0x4d59_5df4_d0f3_3173_u64;
        for _case in 0..2_000 {
            let count = 2 + usize::try_from(next(&mut state) % 7).unwrap();
            let seeds = (0..count).map(|_| next(&mut state) % 4).collect::<Vec<_>>();
            let mut rules = Vec::new();
            for head in 0..count {
                let rule_count = usize::try_from(next(&mut state) % 3).unwrap();
                for _ in 0..rule_count {
                    let body_len = if head == 0 {
                        0
                    } else {
                        usize::try_from(next(&mut state) % 3).unwrap()
                    };
                    let body = (0..body_len)
                        .map(|_| {
                            let premise = usize::try_from(next(&mut state) % head as u64).unwrap();
                            atom(premise)
                        })
                        .collect::<Vec<_>>();
                    rules.push(PositiveBagRule::new(
                        body,
                        atom(head),
                        1 + next(&mut state) % 3,
                    ));
                }
            }
            let program = PositiveBagProgram::new(count, seeds.clone(), rules.clone()).unwrap();
            let certificate = solve_positive_bag(&program).unwrap();

            let mut oracle = vec![0_u128; count];
            for head in 0..count {
                let mut value = u128::from(seeds[head]);
                for rule in rules.iter().filter(|rule| rule.head.index() == head) {
                    let mut product = u128::from(rule.coefficient);
                    for premise in &rule.body {
                        product *= oracle[premise.index()];
                    }
                    value += product;
                }
                oracle[head] = value;
                let NaturalInfinity::Finite(actual) = certificate.multiplicity(atom(head)).unwrap()
                else {
                    panic!("acyclic program classified as infinite");
                };
                assert_eq!(actual.to_decimal_string(), value.to_string());
            }
        }
    }

    #[test]
    fn long_finite_carrier_uses_iterative_scc_walk_without_stack_recursion() {
        let count = 20_000;
        let mut rules = Vec::with_capacity(count - 1);
        for head in 1..count {
            rules.push(PositiveBagRule::new([atom(head - 1)], atom(head), 1));
        }
        let mut seeds = vec![0; count];
        seeds[0] = 1;
        let program = PositiveBagProgram::new(count, seeds, rules).unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        assert_eq!(
            certificate.multiplicity(atom(count - 1)),
            Some(&NaturalInfinity::finite_u64(1))
        );
    }

    #[test]
    fn arbitrary_precision_finite_counts_do_not_turn_into_infinity() {
        let program = PositiveBagProgram::new(
            3,
            vec![u64::MAX, 0, 0],
            vec![
                PositiveBagRule::new([atom(0), atom(0)], atom(1), u64::MAX),
                PositiveBagRule::new([atom(1), atom(1)], atom(2), u64::MAX),
            ],
        )
        .unwrap();
        let certificate = solve_positive_bag(&program).unwrap();
        let NaturalInfinity::Finite(value) = certificate.multiplicity(atom(2)).unwrap() else {
            panic!("finite acyclic proof count must remain finite");
        };
        assert!(value.to_decimal_string().len() > 38);
    }
}
