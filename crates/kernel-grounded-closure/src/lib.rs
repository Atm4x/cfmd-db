use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kernel_persistent::PersistentVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundedAtomId(usize);

impl GroundedAtomId {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundedRuleId(usize);

impl GroundedRuleId {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroundedRule {
    body: Vec<GroundedAtomId>,
    head: GroundedAtomId,
    enabled: bool,
}

impl GroundedRule {
    #[must_use]
    pub fn new(body: impl IntoIterator<Item = GroundedAtomId>, head: GroundedAtomId) -> Self {
        let mut body = body.into_iter().collect::<Vec<_>>();
        body.sort_unstable();
        body.dedup();
        Self {
            body,
            head,
            enabled: true,
        }
    }

    #[must_use]
    pub fn disabled(body: impl IntoIterator<Item = GroundedAtomId>, head: GroundedAtomId) -> Self {
        let mut rule = Self::new(body, head);
        rule.enabled = false;
        rule
    }

    #[must_use]
    pub fn body(&self) -> &[GroundedAtomId] {
        &self.body
    }

    #[must_use]
    pub const fn head(&self) -> GroundedAtomId {
        self.head
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroundedClosureError {
    AtomOutsideUniverse(GroundedAtomId),
    RuleOutsideProgram(GroundedRuleId),
    CertificateShapeMismatch,
    LiveAtomMissingRank(GroundedAtomId),
    LiveAtomMissingWitness(GroundedAtomId),
    DeadAtomHasProofState(GroundedAtomId),
    SeedWitnessForNonSeed(GroundedAtomId),
    SeedHasNonZeroRank(GroundedAtomId),
    WitnessRuleDisabled(GroundedRuleId),
    WitnessRuleHeadMismatch {
        atom: GroundedAtomId,
        rule: GroundedRuleId,
    },
    WitnessPremiseDead {
        atom: GroundedAtomId,
        premise: GroundedAtomId,
    },
    WitnessRankNotStrictlySmaller {
        atom: GroundedAtomId,
        premise: GroundedAtomId,
    },
    MissingSeed(GroundedAtomId),
    NotClosedUnderRule(GroundedRuleId),
    StructuralUniverseShrank {
        old_atom_count: usize,
        new_atom_count: usize,
    },
    StructuralUniverseOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroundedSeedSet {
    members: PersistentVec<bool>,
    count: usize,
}

impl GroundedSeedSet {
    fn from_atoms(
        atom_count: usize,
        atoms: impl IntoIterator<Item = GroundedAtomId>,
    ) -> Result<Self, GroundedClosureError> {
        let mut set = Self {
            members: PersistentVec::from_vec(vec![false; atom_count]),
            count: 0,
        };
        for atom in atoms {
            if atom.index() >= atom_count {
                return Err(GroundedClosureError::AtomOutsideUniverse(atom));
            }
            set.insert(atom);
        }
        Ok(set)
    }

    fn contains(&self, atom: GroundedAtomId) -> bool {
        self.members.get(atom.index()).copied().unwrap_or(false)
    }

    fn insert(&mut self, atom: GroundedAtomId) -> bool {
        if atom.index() >= self.members.len() {
            self.members.resize(atom.index() + 1, false);
        }
        if self.members[atom.index()] {
            return false;
        }
        self.members.set(atom.index(), true);
        self.count += 1;
        true
    }

    fn remove(&mut self, atom: GroundedAtomId) -> bool {
        if !self.contains(atom) {
            return false;
        }
        self.members.set(atom.index(), false);
        self.count -= 1;
        true
    }

    fn resize_atoms(&mut self, atom_count: usize) {
        debug_assert!(atom_count >= self.members.len());
        self.members.resize(atom_count, false);
    }

    fn iter(&self) -> impl Iterator<Item = GroundedAtomId> + '_ {
        self.members
            .iter()
            .enumerate()
            .filter_map(|(index, present)| present.then_some(GroundedAtomId::new(index)))
    }

    fn difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = GroundedAtomId> + 'a {
        self.iter().filter(|atom| !other.contains(*atom))
    }

    fn estimated_heap_bytes(&self) -> usize {
        self.members.estimated_heap_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedProgram {
    atom_count: usize,
    seeds: GroundedSeedSet,
    rules: PersistentVec<GroundedRule>,
}

impl GroundedProgram {
    pub fn new(
        atom_count: usize,
        seeds: impl IntoIterator<Item = GroundedAtomId>,
        rules: Vec<GroundedRule>,
    ) -> Result<Self, GroundedClosureError> {
        let seeds = GroundedSeedSet::from_atoms(atom_count, seeds)?;
        let program = Self {
            atom_count,
            seeds,
            rules: PersistentVec::from_vec(rules),
        };
        program.validate()?;
        Ok(program)
    }

    pub fn validate(&self) -> Result<(), GroundedClosureError> {
        for seed in self.seeds.iter() {
            self.validate_atom(seed)?;
        }
        for rule in &self.rules {
            self.validate_atom(rule.head)?;
            for &atom in &rule.body {
                self.validate_atom(atom)?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn atom_count(&self) -> usize {
        self.atom_count
    }

    pub fn seeds(&self) -> impl Iterator<Item = GroundedAtomId> + '_ {
        self.seeds.iter()
    }

    #[must_use]
    pub fn rules(&self) -> &[GroundedRule] {
        &self.rules
    }

    /// Approximate retained bytes owned by this reconstructed finite program.
    ///
    /// This intentionally accounts for the explicit semantic data structures
    /// only. Allocator/node overhead for `BTreeSet` remains a physical/runtime
    /// concern rather than a semantic accounting contract.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(self.seeds.estimated_heap_bytes())
            .saturating_add(
                self.rules
                    .capacity()
                    .saturating_mul(std::mem::size_of::<GroundedRule>()),
            );
        for rule in &self.rules {
            bytes = bytes.saturating_add(
                rule.body
                    .capacity()
                    .saturating_mul(std::mem::size_of::<GroundedAtomId>()),
            );
        }
        bytes
    }

    fn validate_atom(&self, atom: GroundedAtomId) -> Result<(), GroundedClosureError> {
        if atom.index() < self.atom_count {
            Ok(())
        } else {
            Err(GroundedClosureError::AtomOutsideUniverse(atom))
        }
    }

    fn validate_rule(&self, rule: GroundedRuleId) -> Result<(), GroundedClosureError> {
        if rule.index() < self.rules.len() {
            Ok(())
        } else {
            Err(GroundedClosureError::RuleOutsideProgram(rule))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundedWitness {
    Seed,
    Rule(GroundedRuleId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedCertificate {
    live: PersistentVec<bool>,
    rank: PersistentVec<Option<usize>>,
    witness: PersistentVec<Option<GroundedWitness>>,
}

impl GroundedCertificate {
    #[must_use]
    pub fn is_live(&self, atom: GroundedAtomId) -> bool {
        self.live.get(atom.index()).copied().unwrap_or(false)
    }

    #[must_use]
    pub fn rank(&self, atom: GroundedAtomId) -> Option<usize> {
        self.rank.get(atom.index()).copied().flatten()
    }

    #[must_use]
    pub fn witness(&self, atom: GroundedAtomId) -> Option<GroundedWitness> {
        self.witness.get(atom.index()).copied().flatten()
    }

    pub fn live_atoms(&self) -> impl Iterator<Item = GroundedAtomId> + '_ {
        self.live
            .iter()
            .enumerate()
            .filter_map(|(index, &live)| live.then_some(GroundedAtomId::new(index)))
    }

    /// Approximate retained bytes owned by this reconstructible certificate.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.live
                    .capacity()
                    .saturating_mul(std::mem::size_of::<bool>()),
            )
            .saturating_add(
                self.rank
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<usize>>()),
            )
            .saturating_add(
                self.witness
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<GroundedWitness>>()),
            )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroundedWorkStats {
    pub incidence_updates: usize,
    pub dependency_walks: usize,
    pub rule_fires: usize,
    pub affected_atoms: usize,
}

/// One conjunctive support obligation in a greatest-support problem.
///
/// `dependent` is supported only when at least one atom from `supporters`
/// remains supported. Under death dualization the obligation becomes one
/// grounded rule `all supporters dead -> dependent dead`.
mod index;
pub use index::GroundedIncidenceIndex;

mod solver;
pub use solver::*;

mod maintenance;
pub use maintenance::*;

mod bipolar;
pub use bipolar::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[derive(Clone)]
    struct Rng(usize);

    impl Rng {
        fn next(&mut self) -> usize {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.0
        }

        fn usize(&mut self, n: usize) -> usize {
            self.next() % n.max(1)
        }

        fn bool(&mut self, numerator: usize, denominator: usize) -> bool {
            self.usize(denominator) < numerator
        }
    }

    fn atom(index: usize) -> GroundedAtomId {
        GroundedAtomId::new(index)
    }

    fn random_program(rng: &mut Rng, atoms: usize, rules: usize) -> GroundedProgram {
        let seeds = (0..atoms)
            .filter(|_| rng.bool(1, 5))
            .map(atom)
            .collect::<Vec<_>>();
        let rules = (0..rules)
            .map(|_| {
                let arity = rng.usize(4);
                let body = (0..arity)
                    .map(|_| atom(rng.usize(atoms)))
                    .collect::<Vec<_>>();
                let head = atom(rng.usize(atoms));
                GroundedRule::new(body, head)
            })
            .collect::<Vec<_>>();
        GroundedProgram::new(atoms, seeds, rules).unwrap()
    }

    fn naive_live(program: &GroundedProgram) -> Vec<bool> {
        let mut live = vec![false; program.atom_count];
        for seed in program.seeds.iter() {
            live[seed.index()] = true;
        }
        loop {
            let mut changed = false;
            for rule in &program.rules {
                if !rule.enabled || live[rule.head.index()] {
                    continue;
                }
                if rule.body.iter().all(|premise| live[premise.index()]) {
                    live[rule.head.index()] = true;
                    changed = true;
                }
            }
            if !changed {
                return live;
            }
        }
    }

    #[test]
    fn randomized_solver_matches_naive_and_certificate() {
        let mut rng = Rng(1);
        for _ in 0..2_000 {
            let atoms = 1 + rng.usize(18);
            let rules = rng.usize(50);
            let program = random_program(&mut rng, atoms, rules);
            let (certificate, _) = solve(&program);
            assert_eq!(certificate.live.as_slice(), naive_live(&program).as_slice());
            assert_eq!(check(&program, &certificate), Ok(()));
        }
    }

    #[test]
    fn groundless_cycle_remains_dead() {
        let program = GroundedProgram::new(
            2,
            [],
            vec![
                GroundedRule::new([atom(0)], atom(1)),
                GroundedRule::new([atom(1)], atom(0)),
            ],
        )
        .unwrap();
        let (certificate, _) = solve(&program);
        assert!(!certificate.is_live(atom(0)));
        assert!(!certificate.is_live(atom(1)));
        assert_eq!(check(&program, &certificate), Ok(()));
    }

    #[test]
    fn genuine_hyperrule_requires_all_premises() {
        let rules = vec![GroundedRule::new([atom(0), atom(1)], atom(2))];
        let one = GroundedProgram::new(3, [atom(0)], rules.clone()).unwrap();
        assert!(!solve(&one).0.is_live(atom(2)));
        let both = GroundedProgram::new(3, [atom(0), atom(1)], rules).unwrap();
        assert!(solve(&both).0.is_live(atom(2)));
    }

    #[test]
    fn randomized_mixed_updates_match_full_rebuild() {
        let mut rng = Rng(7);
        let mut program = random_program(&mut rng, 40, 100);
        let (mut certificate, _) = solve(&program);
        for _ in 0..5_000 {
            let mut update = GroundedUpdate::default();
            match rng.usize(4) {
                0 => update.add_seeds.push(atom(rng.usize(program.atom_count()))),
                1 => update
                    .remove_seeds
                    .push(atom(rng.usize(program.atom_count()))),
                2 => update
                    .enable_rules
                    .push(GroundedRuleId::new(rng.usize(program.rules().len()))),
                _ => update
                    .disable_rules
                    .push(GroundedRuleId::new(rng.usize(program.rules().len()))),
            }
            let (next, _) = apply_update(&mut program, &certificate, &update).unwrap();
            let full = solve(&program).0;
            assert_eq!(
                next.live_atoms().collect::<Vec<_>>(),
                full.live_atoms().collect::<Vec<_>>()
            );
            certificate = next;
        }
    }

    #[test]
    fn generic_maintenance_matches_full_rebuild_across_mixed_updates() {
        let mut rng = Rng(17);
        let program = random_program(&mut rng, 32, 72);
        let mut maintenance = GroundedMaintenance::new(program).unwrap();
        for _ in 0..2_000 {
            let mut update = GroundedUpdate::default();
            match rng.usize(4) {
                0 => update
                    .add_seeds
                    .push(atom(rng.usize(maintenance.program().atom_count()))),
                1 => update
                    .remove_seeds
                    .push(atom(rng.usize(maintenance.program().atom_count()))),
                2 => update.enable_rules.push(GroundedRuleId::new(
                    rng.usize(maintenance.program().rules().len()),
                )),
                _ => update.disable_rules.push(GroundedRuleId::new(
                    rng.usize(maintenance.program().rules().len()),
                )),
            }
            maintenance.apply_update(&update).unwrap();
            let rebuilt = solve(maintenance.program()).0;
            assert_eq!(
                maintenance.certificate().live_atoms().collect::<Vec<_>>(),
                rebuilt.live_atoms().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn deleting_non_witness_rule_does_no_invalidation_work() {
        let mut program = GroundedProgram::new(
            4,
            [atom(0), atom(1)],
            vec![
                GroundedRule::new([atom(0)], atom(2)),
                GroundedRule::new([atom(1)], atom(2)),
                GroundedRule::new([atom(2)], atom(3)),
            ],
        )
        .unwrap();
        let (certificate, _) = solve(&program);
        assert_eq!(
            certificate.witness(atom(2)),
            Some(GroundedWitness::Rule(GroundedRuleId::new(0)))
        );
        let update = GroundedUpdate {
            disable_rules: vec![GroundedRuleId::new(1)],
            ..GroundedUpdate::default()
        };
        let (same, stats) = apply_update(&mut program, &certificate, &update).unwrap();
        assert_eq!(
            same.live_atoms().collect::<Vec<_>>(),
            certificate.live_atoms().collect::<Vec<_>>()
        );
        assert_eq!(stats.affected_atoms, 0);
    }

    #[test]
    fn deleting_grounding_seed_kills_self_supported_cycle() {
        let mut program = GroundedProgram::new(
            3,
            [atom(0)],
            vec![
                GroundedRule::new([atom(0)], atom(1)),
                GroundedRule::new([atom(1)], atom(2)),
                GroundedRule::new([atom(2)], atom(1)),
            ],
        )
        .unwrap();
        let (certificate, _) = solve(&program);
        let update = GroundedUpdate {
            remove_seeds: vec![atom(0)],
            ..GroundedUpdate::default()
        };
        let (next, stats) = apply_update(&mut program, &certificate, &update).unwrap();
        assert!(next.live_atoms().next().is_none());
        assert_eq!(stats.affected_atoms, 3);
    }

    #[test]
    fn structural_body_extension_resurrects_only_selected_witness_cone() {
        let old_program = GroundedProgram::new(
            5,
            [atom(0)],
            vec![
                GroundedRule::new([atom(0)], atom(2)),
                GroundedRule::new([atom(2)], atom(3)),
                GroundedRule::new([atom(4)], atom(1)),
            ],
        )
        .unwrap();
        let (old, _) = solve(&old_program);
        assert!(old.is_live(atom(2)));
        assert!(old.is_live(atom(3)));

        let new_program = GroundedProgram::new(
            5,
            [atom(0)],
            vec![
                GroundedRule::new([atom(0), atom(1)], atom(2)),
                GroundedRule::new([atom(2)], atom(3)),
                GroundedRule::new([atom(4)], atom(1)),
            ],
        )
        .unwrap();
        let (reconciled, stats) =
            reconcile_structural_change(&old_program, &new_program, &old).unwrap();
        let rebuilt = solve(&new_program).0;
        assert_eq!(
            reconciled.live_atoms().collect::<Vec<_>>(),
            rebuilt.live_atoms().collect::<Vec<_>>()
        );
        assert!(!reconciled.is_live(atom(2)));
        assert!(!reconciled.is_live(atom(3)));
        assert_eq!(stats.affected_atoms, 2);
    }

    #[test]
    fn randomized_structural_reconciliation_matches_full_rebuild() {
        let mut rng = Rng(31);
        let mut program = random_program(&mut rng, 24, 48);
        let (mut certificate, _) = solve(&program);
        for _ in 0..2_000 {
            let mut next = program.clone();
            match rng.usize(6) {
                0 => {
                    next.atom_count += 1;
                    let new_atom = atom(next.atom_count - 1);
                    if rng.bool(1, 2) {
                        next.seeds.insert(new_atom);
                    }
                    next.rules.push(GroundedRule::new(
                        [atom(rng.usize(next.atom_count))],
                        new_atom,
                    ));
                }
                1 => {
                    let candidate = atom(rng.usize(next.atom_count));
                    if next.seeds.contains(candidate) {
                        next.seeds.remove(candidate);
                    } else {
                        next.seeds.insert(candidate);
                    }
                }
                2 if !next.rules.is_empty() => {
                    let index = rng.usize(next.rules.len());
                    let arity = rng.usize(4);
                    let body = (0..arity)
                        .map(|_| atom(rng.usize(next.atom_count)))
                        .collect::<Vec<_>>();
                    let head = atom(rng.usize(next.atom_count));
                    next.rules[index] = GroundedRule::new(body, head);
                }
                3 if next.rules.len() > 1 => {
                    let left = rng.usize(next.rules.len());
                    let right = rng.usize(next.rules.len());
                    next.rules.swap(left, right);
                }
                4 => next.rules.push(GroundedRule::new(
                    [atom(rng.usize(next.atom_count))],
                    atom(rng.usize(next.atom_count)),
                )),
                _ if !next.rules.is_empty() => {
                    let index = rng.usize(next.rules.len());
                    next.rules.remove(index);
                }
                _ => {}
            }
            next.validate().unwrap();
            let (reconciled, _) =
                reconcile_structural_change(&program, &next, &certificate).unwrap();
            let rebuilt = solve(&next).0;
            assert_eq!(
                reconciled.live_atoms().collect::<Vec<_>>(),
                rebuilt.live_atoms().collect::<Vec<_>>()
            );
            assert_eq!(check(&next, &reconciled), Ok(()));
            program = next;
            certificate = reconciled;
        }
    }

    #[test]
    fn append_structural_patch_preserves_ids_and_matches_full_recompute() {
        let mut program =
            GroundedProgram::new(2, [atom(0)], vec![GroundedRule::new([atom(0)], atom(1))])
                .unwrap();
        let mut index = GroundedIncidenceIndex::compile(&program);
        let (old, _) = solve_indexed(&program, &index);

        let patch = GroundedStructuralPatch {
            append_atoms: 2,
            append_rules: vec![
                GroundedRule::new([atom(0), atom(2)], atom(1)),
                GroundedRule::new([atom(1)], atom(3)),
            ],
            add_seeds: vec![atom(2)],
            disable_rules: vec![GroundedRuleId::new(0)],
            ..GroundedStructuralPatch::default()
        };
        let (patched, _, appended) =
            apply_structural_patch_indexed(&mut program, &mut index, &old, patch).unwrap();

        assert_eq!(
            appended,
            vec![GroundedRuleId::new(1), GroundedRuleId::new(2)]
        );
        assert!(!program.rules()[0].enabled());
        assert_eq!(
            patched.live_atoms().collect::<Vec<_>>(),
            vec![atom(0), atom(1), atom(2), atom(3)]
        );
        let (recomputed, _) = solve(&program);
        assert_eq!(
            patched.live_atoms().collect::<Vec<_>>(),
            recomputed.live_atoms().collect::<Vec<_>>()
        );
        assert_eq!(check(&program, &patched), Ok(()));
    }

    #[test]
    fn bipolar_support_is_exact_complement_of_grounded_death() {
        // 0 and 1 mutually support each other and are therefore live under ν;
        // 2 has no supporter and dies immediately; 3 depends only on 2 and dies.
        let program = BipolarSupportProgram::new(
            4,
            [],
            vec![
                BipolarSupportRequirement::new(atom(0), [atom(1)]),
                BipolarSupportRequirement::new(atom(1), [atom(0)]),
                BipolarSupportRequirement::new(atom(2), []),
                BipolarSupportRequirement::new(atom(3), [atom(2)]),
            ],
        )
        .unwrap();
        let (certificate, _) = solve_bipolar_support(&program).unwrap();
        assert!(certificate.is_supported(atom(0)));
        assert!(certificate.is_supported(atom(1)));
        assert!(!certificate.is_supported(atom(2)));
        assert!(!certificate.is_supported(atom(3)));
        assert_eq!(check_bipolar_support(&program, &certificate), Ok(()));
    }

    #[test]
    fn bipolar_maintenance_versions_requirement_without_full_rebuild() {
        let old_program = BipolarSupportProgram::new(
            3,
            [atom(0)],
            vec![BipolarSupportRequirement::new(atom(2), [atom(0)])],
        )
        .unwrap();
        let mut maintenance = BipolarSupportMaintenance::new(&old_program).unwrap();
        assert!(!maintenance.certificate().is_supported(atom(2)));

        let (stats, appended) = maintenance
            .apply_structural_patch(BipolarSupportStructuralPatch {
                append_requirements: vec![BipolarSupportRequirement::new(
                    atom(2),
                    [atom(0), atom(1)],
                )],
                disable_requirements: vec![GroundedRuleId::new(0)],
                ..BipolarSupportStructuralPatch::default()
            })
            .unwrap();
        assert_eq!(appended, vec![GroundedRuleId::new(1)]);
        assert!(maintenance.certificate().is_supported(atom(2)));
        assert!(stats.affected_atoms <= 1);

        let rebuilt = BipolarSupportProgram::new(
            3,
            [atom(0)],
            vec![BipolarSupportRequirement::new(atom(2), [atom(0), atom(1)])],
        )
        .unwrap();
        let rebuilt = solve_bipolar_support(&rebuilt).unwrap().0;
        assert_eq!(
            maintenance
                .certificate()
                .supported_atoms()
                .collect::<Vec<_>>(),
            rebuilt.supported_atoms().collect::<Vec<_>>()
        );
    }

    #[test]
    fn bipolar_maintenance_small_replacement_keeps_large_universe_in_place() {
        let program = BipolarSupportProgram::new(
            100_000,
            [atom(0)],
            vec![BipolarSupportRequirement::new(atom(1), [atom(0)])],
        )
        .unwrap();
        let mut maintenance = BipolarSupportMaintenance::new(&program).unwrap();
        let snapshot = maintenance.clone();
        assert!(
            maintenance
                .certificate
                .death
                .live
                .shares_storage_with(&snapshot.certificate.death.live)
        );
        assert!(
            maintenance
                .death_index
                .dependents
                .shares_storage_with(&snapshot.death_index.dependents)
        );
        assert!(
            maintenance
                .witness_index
                .children
                .shares_storage_with(&snapshot.witness_index.children)
        );

        let outcome = maintenance
            .apply_structural_patch_tracked(BipolarSupportStructuralPatch {
                append_requirements: vec![BipolarSupportRequirement::new(
                    atom(1),
                    [atom(0), atom(2)],
                )],
                disable_requirements: vec![GroundedRuleId::new(0)],
                ..BipolarSupportStructuralPatch::default()
            })
            .unwrap();

        assert_eq!(outcome.appended_requirements, vec![GroundedRuleId::new(1)]);
        assert_eq!(outcome.work.affected_atoms, 1);
        assert_eq!(outcome.changed_atoms, vec![atom(1)]);
        assert!(maintenance.certificate().is_supported(atom(1)));
        assert!(!snapshot.certificate().is_supported(atom(1)));
        assert!(
            maintenance
                .certificate
                .death
                .live
                .shares_page_with(&snapshot.certificate.death.live, 99_999)
        );
        assert!(
            maintenance
                .certificate
                .death
                .rank
                .shares_page_with(&snapshot.certificate.death.rank, 99_999)
        );
        assert!(
            maintenance
                .certificate
                .death
                .witness
                .shares_page_with(&snapshot.certificate.death.witness, 99_999)
        );
        assert!(
            maintenance
                .witness_index
                .children
                .shares_page_with(&snapshot.witness_index.children, 99_999)
        );

        let rebuilt = BipolarSupportProgram::new(
            100_000,
            [atom(0)],
            vec![BipolarSupportRequirement::new(atom(1), [atom(0), atom(2)])],
        )
        .unwrap();
        let rebuilt = solve_bipolar_support(&rebuilt).unwrap().0;
        assert_eq!(
            maintenance.certificate().is_supported(atom(1)),
            rebuilt.is_supported(atom(1))
        );
    }

    #[test]
    fn randomized_bipolar_maintenance_matches_full_rebuild() {
        let atom_count = 32;
        let mut rng = Rng(0x51a7_2026);
        let mut unavailable = BTreeSet::from([atom(0), atom(7)]);
        let mut requirements = (0..16)
            .map(|index| {
                BipolarSupportRequirement::new(
                    atom(index + 8),
                    [atom(rng.usize(atom_count)), atom(rng.usize(atom_count))],
                )
            })
            .collect::<Vec<_>>();
        let initial = BipolarSupportProgram::new(
            atom_count,
            unavailable.iter().copied(),
            requirements.clone(),
        )
        .unwrap();
        let mut maintenance = BipolarSupportMaintenance::new(&initial).unwrap();
        let mut rule_ids = (0..requirements.len())
            .map(GroundedRuleId::new)
            .collect::<Vec<_>>();

        for step in 0..120 {
            let requirement_index = rng.usize(requirements.len());
            let replacement = BipolarSupportRequirement::new(
                requirements[requirement_index].dependent,
                [atom(rng.usize(atom_count)), atom(rng.usize(atom_count))],
            );
            let toggle = atom(rng.usize(atom_count));
            let (add_unavailable, remove_unavailable) = if step % 3 == 0 {
                if unavailable.insert(toggle) {
                    (vec![toggle], vec![])
                } else {
                    unavailable.remove(&toggle);
                    (vec![], vec![toggle])
                }
            } else {
                (vec![], vec![])
            };
            let (_, appended) = maintenance
                .apply_structural_patch(BipolarSupportStructuralPatch {
                    append_requirements: vec![replacement.clone()],
                    add_unavailable,
                    remove_unavailable,
                    disable_requirements: vec![rule_ids[requirement_index]],
                    ..BipolarSupportStructuralPatch::default()
                })
                .unwrap();
            requirements[requirement_index] = replacement;
            rule_ids[requirement_index] = appended[0];

            let rebuilt = BipolarSupportProgram::new(
                atom_count,
                unavailable.iter().copied(),
                requirements.clone(),
            )
            .unwrap();
            let rebuilt = solve_bipolar_support(&rebuilt).unwrap().0;
            assert_eq!(
                maintenance
                    .certificate()
                    .supported_atoms()
                    .collect::<Vec<_>>(),
                rebuilt.supported_atoms().collect::<Vec<_>>(),
                "mismatch after maintenance step {step}"
            );
        }
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_sparse_bipolar_replacement_against_dense_structural_update() {
        for atom_count in [10_000, 100_000, 300_000] {
            let support = BipolarSupportProgram::new(
                atom_count,
                [atom(0)],
                vec![BipolarSupportRequirement::new(atom(1), [atom(0)])],
            )
            .unwrap();

            let mut dense_program = support.death_program().unwrap();
            let mut dense_index = GroundedIncidenceIndex::compile(&dense_program);
            let dense_old = solve_indexed(&dense_program, &dense_index).0;
            let dense_patch = GroundedStructuralPatch {
                append_rules: vec![GroundedRule::new([atom(0), atom(2)], atom(1))],
                disable_rules: vec![GroundedRuleId::new(0)],
                ..GroundedStructuralPatch::default()
            };
            let dense_start = Instant::now();
            let _ = apply_structural_patch_indexed(
                &mut dense_program,
                &mut dense_index,
                &dense_old,
                dense_patch,
            )
            .unwrap();
            let dense = dense_start.elapsed();

            let mut sparse = BipolarSupportMaintenance::new(&support).unwrap();
            let sparse_start = Instant::now();
            let _ = sparse
                .apply_structural_patch(BipolarSupportStructuralPatch {
                    append_requirements: vec![BipolarSupportRequirement::new(
                        atom(1),
                        [atom(0), atom(2)],
                    )],
                    disable_requirements: vec![GroundedRuleId::new(0)],
                    ..BipolarSupportStructuralPatch::default()
                })
                .unwrap();
            let sparse = sparse_start.elapsed();

            eprintln!("atoms={atom_count} dense={dense:?} sparse={sparse:?}");
        }
    }

    #[test]
    fn unavailable_atom_grounds_death_through_support_requirements() {
        let program = BipolarSupportProgram::new(
            3,
            [atom(0)],
            vec![
                BipolarSupportRequirement::new(atom(1), [atom(0)]),
                BipolarSupportRequirement::new(atom(2), [atom(1)]),
            ],
        )
        .unwrap();
        let (certificate, _) = solve_bipolar_support(&program).unwrap();
        assert!(certificate.supported_atoms().next().is_none());
    }

    #[test]
    fn bipolar_body_extension_resurrects_support_through_death_witness_cone() {
        let old_program = BipolarSupportProgram::new(
            4,
            [atom(0)],
            vec![
                BipolarSupportRequirement::new(atom(2), [atom(0)]),
                BipolarSupportRequirement::new(atom(3), [atom(2)]),
            ],
        )
        .unwrap();
        let (old, _) = solve_bipolar_support(&old_program).unwrap();
        assert!(!old.is_supported(atom(2)));
        assert!(!old.is_supported(atom(3)));

        let new_program = BipolarSupportProgram::new(
            4,
            [atom(0)],
            vec![
                BipolarSupportRequirement::new(atom(2), [atom(0), atom(1)]),
                BipolarSupportRequirement::new(atom(3), [atom(2)]),
            ],
        )
        .unwrap();
        let (reconciled, stats) =
            reconcile_bipolar_support(&old_program, &new_program, &old).unwrap();
        let rebuilt = solve_bipolar_support(&new_program).unwrap().0;
        assert_eq!(
            reconciled.supported_atoms().collect::<Vec<_>>(),
            rebuilt.supported_atoms().collect::<Vec<_>>()
        );
        assert!(reconciled.is_supported(atom(2)));
        assert!(reconciled.is_supported(atom(3)));
        assert_eq!(stats.affected_atoms, 2);
    }
}
