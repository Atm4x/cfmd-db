use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use std::collections::VecDeque;

pub use kernel_grounded_closure::GroundedAtomId;
use kernel_grounded_closure::{
    GroundedClosureError, GroundedProgram, GroundedRule, GroundedWitness,
    solve as solve_grounded_closure,
};
use kernel_types::EntityId;

mod positive_bag;
pub use positive_bag::*;

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
    UnexpectedRank(EntityId),
    UnexpectedParent(EntityId),
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
    for &value in certificate.rank.keys() {
        if !certificate.reachable.contains(&value) {
            return Err(FixpointError::UnexpectedRank(value));
        }
    }
    for &value in certificate.parent.keys() {
        if !certificate.reachable.contains(&value) || program.seeds.contains(&value) {
            return Err(FixpointError::UnexpectedParent(value));
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
    fn checker_rejects_unverified_certificate_payload_outside_reachable_shape() {
        let program = ReachabilityProgram {
            universe: BTreeSet::from([id(1), id(2), id(3)]),
            seeds: BTreeSet::from([id(1)]),
            edges: BTreeSet::from([(id(1), id(2))]),
        };
        let mut certificate = solve(&program).unwrap();
        certificate.rank.insert(id(3), 99);
        assert_eq!(
            check(&program, &certificate),
            Err(FixpointError::UnexpectedRank(id(3)))
        );

        let mut certificate = solve(&program).unwrap();
        certificate.parent.insert(id(1), id(2));
        assert_eq!(
            check(&program, &certificate),
            Err(FixpointError::UnexpectedParent(id(1)))
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
