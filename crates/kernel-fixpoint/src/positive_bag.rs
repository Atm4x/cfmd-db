use crate::GroundedAtomId;
use kernel_grounded_closure::{
    GroundedClosureError, GroundedProgram, GroundedRule, solve as solve_grounded_closure,
};

pub use kernel_exact::ExactNatural as BigNatural;

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

#[derive(Debug)]
struct PositiveBagIncidence {
    rules_by_head: Vec<Vec<usize>>,
    graph: Vec<Vec<usize>>,
    reverse: Vec<Vec<usize>>,
}

impl PositiveBagIncidence {
    fn compile(
        program: &PositiveBagProgram,
        support: &kernel_grounded_closure::GroundedCertificate,
    ) -> Self {
        let mut rules_by_head = vec![Vec::new(); program.atom_count];
        let mut graph = vec![Vec::new(); program.atom_count];

        for (rule_index, rule) in program.rules.iter().enumerate() {
            if rule.coefficient == 0
                || !support.is_live(rule.head)
                || !rule.body.iter().all(|&atom| support.is_live(atom))
            {
                continue;
            }

            let head = rule.head.index();
            rules_by_head[head].push(rule_index);
            for &premise in &rule.body {
                graph[premise.index()].push(head);
            }
        }

        for targets in &mut graph {
            targets.sort_unstable();
            targets.dedup();
        }

        let mut reverse = vec![Vec::new(); program.atom_count];
        for (source, targets) in graph.iter().enumerate() {
            for &target in targets {
                reverse[target].push(source);
            }
        }

        Self {
            rules_by_head,
            graph,
            reverse,
        }
    }
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

fn cyclic_live_atoms(incidence: &PositiveBagIncidence, live: &[bool]) -> Vec<bool> {
    let mut seen = vec![false; incidence.graph.len()];
    let mut order = Vec::new();
    for (node, &is_live) in live.iter().enumerate().take(incidence.graph.len()) {
        if is_live {
            dfs_order(node, &incidence.graph, &mut seen, &mut order);
        }
    }
    seen.fill(false);
    let mut cyclic = vec![false; incidence.graph.len()];
    for &node in order.iter().rev() {
        if seen[node] || !live[node] {
            continue;
        }
        let mut component = Vec::new();
        dfs_component(node, &incidence.reverse, &mut seen, &mut component);
        let is_cycle = component.len() > 1 || incidence.graph[node].binary_search(&node).is_ok();
        if is_cycle {
            for member in component {
                cyclic[member] = true;
            }
        }
    }
    cyclic
}

fn classify_infinite(incidence: &PositiveBagIncidence, live: &[bool]) -> Vec<bool> {
    let mut infinite = cyclic_live_atoms(incidence, live);
    let mut queue = std::collections::VecDeque::new();
    for (index, &value) in infinite.iter().enumerate() {
        if value {
            queue.push_back(index);
        }
    }
    while let Some(source) = queue.pop_front() {
        for &target in &incidence.graph[source] {
            if live[target] && !infinite[target] {
                infinite[target] = true;
                queue.push_back(target);
            }
        }
    }
    infinite
}

pub fn solve_positive_bag(
    program: &PositiveBagProgram,
) -> Result<PositiveBagCertificate, PositiveBagFixpointError> {
    let support_program = program.support_program()?;
    let (support, _) = solve_grounded_closure(&support_program);
    kernel_grounded_closure::check(&support_program, &support)?;
    let live = (0..program.atom_count)
        .map(|index| support.is_live(GroundedAtomId::new(index)))
        .collect::<Vec<_>>();
    let incidence = PositiveBagIncidence::compile(program, &support);
    let infinite = classify_infinite(&incidence, &live);

    let mut indegree = vec![0_usize; program.atom_count];
    for head in 0..program.atom_count {
        if live[head] && !infinite[head] {
            indegree[head] = incidence.reverse[head]
                .iter()
                .filter(|&&premise| live[premise] && !infinite[premise])
                .count();
        }
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
        for &rule_index in &incidence.rules_by_head[atom] {
            let rule = &program.rules[rule_index];
            let mut product = BigNatural::from_u64(rule.coefficient);
            for &premise in &rule.body {
                product = product.multiplied(&finite[premise.index()]);
            }
            value.add_assign(&product);
        }
        finite[atom] = value;
        for &target in &incidence.graph[atom] {
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

    let live = (0..program.atom_count)
        .map(|index| certificate.support.is_live(GroundedAtomId::new(index)))
        .collect::<Vec<_>>();
    let incidence = PositiveBagIncidence::compile(program, &certificate.support);
    let infinite = classify_infinite(&incidence, &live);

    for atom in 0..program.atom_count {
        let claimed = &certificate.multiplicity[atom];
        if !live[atom] {
            if claimed != &NaturalInfinity::zero() {
                return Err(PositiveBagFixpointError::CertificateShapeMismatch);
            }
            continue;
        }
        if infinite[atom] {
            if !claimed.is_infinite() {
                return Err(PositiveBagFixpointError::CertificateShapeMismatch);
            }
            continue;
        }

        let NaturalInfinity::Finite(claimed_value) = claimed else {
            return Err(PositiveBagFixpointError::CertificateShapeMismatch);
        };
        let mut expected = BigNatural::from_u64(program.seed_multiplicity[atom]);
        for &rule_index in &incidence.rules_by_head[atom] {
            let rule = &program.rules[rule_index];
            let mut product = BigNatural::from_u64(rule.coefficient);
            for &premise in &rule.body {
                let NaturalInfinity::Finite(premise_value) =
                    &certificate.multiplicity[premise.index()]
                else {
                    return Err(PositiveBagFixpointError::CertificateShapeMismatch);
                };
                product = product.multiplied(premise_value);
            }
            expected.add_assign(&product);
        }
        if &expected != claimed_value {
            return Err(PositiveBagFixpointError::CertificateShapeMismatch);
        }
    }

    Ok(())
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

        let mut forged = certificate.clone();
        forged.multiplicity[2] = NaturalInfinity::finite_u64(11);
        assert_eq!(
            check_positive_bag(&program, &forged),
            Err(PositiveBagFixpointError::CertificateShapeMismatch)
        );
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
