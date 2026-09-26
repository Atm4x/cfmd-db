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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedRule {
    body: Vec<GroundedAtomId>,
    head: GroundedAtomId,
    enabled: bool,
}

type GroundedRuleKey = (GroundedAtomId, Vec<GroundedAtomId>, bool);

fn grounded_rule_key(rule: &GroundedRule) -> GroundedRuleKey {
    (rule.head, rule.body.clone(), rule.enabled)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BipolarSupportRequirement {
    pub dependent: GroundedAtomId,
    pub supporters: Vec<GroundedAtomId>,
}

impl BipolarSupportRequirement {
    #[must_use]
    pub fn new(
        dependent: GroundedAtomId,
        supporters: impl IntoIterator<Item = GroundedAtomId>,
    ) -> Self {
        let mut supporters = supporters.into_iter().collect::<Vec<_>>();
        supporters.sort_unstable();
        supporters.dedup();
        Self {
            dependent,
            supporters,
        }
    }
}

/// Finite coinductive support program lowered to the grounded death calculus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BipolarSupportProgram {
    atom_count: usize,
    unavailable: BTreeSet<GroundedAtomId>,
    requirements: Vec<BipolarSupportRequirement>,
}

impl BipolarSupportProgram {
    pub fn new(
        atom_count: usize,
        unavailable: impl IntoIterator<Item = GroundedAtomId>,
        requirements: Vec<BipolarSupportRequirement>,
    ) -> Result<Self, GroundedClosureError> {
        let unavailable = unavailable.into_iter().collect::<BTreeSet<_>>();
        let program = Self {
            atom_count,
            unavailable,
            requirements,
        };
        let death = program.death_program()?;
        death.validate()?;
        Ok(program)
    }

    #[must_use]
    pub const fn atom_count(&self) -> usize {
        self.atom_count
    }

    #[must_use]
    pub fn unavailable(&self) -> &BTreeSet<GroundedAtomId> {
        &self.unavailable
    }

    #[must_use]
    pub fn requirements(&self) -> &[BipolarSupportRequirement] {
        &self.requirements
    }

    /// Approximate retained bytes owned by the finite greatest-support model.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(
                self.unavailable
                    .len()
                    .saturating_mul(std::mem::size_of::<GroundedAtomId>()),
            )
            .saturating_add(
                self.requirements
                    .capacity()
                    .saturating_mul(std::mem::size_of::<BipolarSupportRequirement>()),
            );
        for requirement in &self.requirements {
            bytes = bytes.saturating_add(
                requirement
                    .supporters
                    .capacity()
                    .saturating_mul(std::mem::size_of::<GroundedAtomId>()),
            );
        }
        bytes
    }

    pub fn death_program(&self) -> Result<GroundedProgram, GroundedClosureError> {
        let rules = self
            .requirements
            .iter()
            .map(|requirement| {
                GroundedRule::new(
                    requirement.supporters.iter().copied(),
                    requirement.dependent,
                )
            })
            .collect();
        GroundedProgram::new(self.atom_count, self.unavailable.iter().copied(), rules)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BipolarSupportCertificate {
    death: GroundedCertificate,
}

impl BipolarSupportCertificate {
    #[must_use]
    pub fn is_supported(&self, atom: GroundedAtomId) -> bool {
        self.death.live.get(atom.index()).is_some_and(|dead| !*dead)
    }

    #[must_use]
    pub const fn death_certificate(&self) -> &GroundedCertificate {
        &self.death
    }

    pub fn supported_atoms(&self) -> impl Iterator<Item = GroundedAtomId> + '_ {
        self.death
            .live
            .iter()
            .enumerate()
            .filter_map(|(index, &dead)| (!dead).then_some(GroundedAtomId::new(index)))
    }

    /// Approximate retained bytes owned by the support/death certificate.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.death.estimated_retained_bytes())
    }
}

pub fn solve_bipolar_support(
    program: &BipolarSupportProgram,
) -> Result<(BipolarSupportCertificate, GroundedWorkStats), GroundedClosureError> {
    let death_program = program.death_program()?;
    let (death, stats) = solve(&death_program);
    let certificate = BipolarSupportCertificate { death };
    check_bipolar_support_with_death(program, &death_program, &certificate)?;
    Ok((certificate, stats))
}

/// Reconcile a greatest-support certificate after the finite support program
/// changes structurally.
///
/// The implementation dualizes both programs to grounded death, reuses the
/// selected old death witnesses through [`reconcile_structural_change`], and
/// complements the repaired death certificate back to support. This is the
/// exact insertion/resurrection operation for support programs whose rule
/// bodies gain or lose potential supporters.
pub fn reconcile_bipolar_support(
    old_program: &BipolarSupportProgram,
    new_program: &BipolarSupportProgram,
    old: &BipolarSupportCertificate,
) -> Result<(BipolarSupportCertificate, GroundedWorkStats), GroundedClosureError> {
    let old_death = old_program.death_program()?;
    check_bipolar_support_with_death(old_program, &old_death, old)?;
    let new_death = new_program.death_program()?;
    let (death, stats) = reconcile_structural_change(&old_death, &new_death, &old.death)?;
    let certificate = BipolarSupportCertificate { death };
    check_bipolar_support_with_death(new_program, &new_death, &certificate)?;
    Ok((certificate, stats))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BipolarSupportStructuralPatch {
    pub append_atoms: usize,
    pub append_requirements: Vec<BipolarSupportRequirement>,
    pub add_unavailable: Vec<GroundedAtomId>,
    pub remove_unavailable: Vec<GroundedAtomId>,
    pub enable_requirements: Vec<GroundedRuleId>,
    pub disable_requirements: Vec<GroundedRuleId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BipolarSupportPatchOutcome {
    pub work: GroundedWorkStats,
    pub appended_requirements: Vec<GroundedRuleId>,
    pub changed_atoms: Vec<GroundedAtomId>,
}

/// Incremental exact maintenance state for a greatest-support program.
///
/// It keeps the dual grounded incidence graph compiled across updates. Changed
/// requirements are versioned as `disable old + append replacement`, so stable
/// rule identities survive and a small structural patch does not require a
/// whole-program incidence rebuild.
#[derive(Debug, Clone)]
pub struct BipolarSupportMaintenance {
    death_program: GroundedProgram,
    death_index: GroundedIncidenceIndex,
    certificate: BipolarSupportCertificate,
    witness_index: GroundedWitnessIndex,
}

impl BipolarSupportMaintenance {
    pub fn new(program: &BipolarSupportProgram) -> Result<Self, GroundedClosureError> {
        let death_program = program.death_program()?;
        let death_index = GroundedIncidenceIndex::compile(&death_program);
        let (death, _) = solve_indexed(&death_program, &death_index);
        let witness_index = GroundedWitnessIndex::compile(&death_program, &death);
        Ok(Self {
            death_program,
            death_index,
            certificate: BipolarSupportCertificate { death },
            witness_index,
        })
    }

    #[must_use]
    pub const fn certificate(&self) -> &BipolarSupportCertificate {
        &self.certificate
    }

    #[must_use]
    pub const fn atom_count(&self) -> usize {
        self.death_program.atom_count
    }

    #[must_use]
    pub fn requirement_count(&self) -> usize {
        self.death_program.rules.len()
    }

    /// Approximate retained bytes of the snapshot-friendly maintained support state.
    #[must_use]
    pub fn estimated_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.death_program.estimated_retained_bytes())
            .saturating_add(self.certificate.estimated_retained_bytes())
    }

    pub fn apply_structural_patch(
        &mut self,
        patch: BipolarSupportStructuralPatch,
    ) -> Result<(GroundedWorkStats, Vec<GroundedRuleId>), GroundedClosureError> {
        let outcome = self.apply_structural_patch_tracked(patch)?;
        Ok((outcome.work, outcome.appended_requirements))
    }

    pub fn apply_structural_patch_tracked(
        &mut self,
        patch: BipolarSupportStructuralPatch,
    ) -> Result<BipolarSupportPatchOutcome, GroundedClosureError> {
        let old_live = self.certificate.death.live.clone();
        let grounded = GroundedStructuralPatch {
            append_atoms: patch.append_atoms,
            append_rules: patch
                .append_requirements
                .into_iter()
                .map(|requirement| GroundedRule::new(requirement.supporters, requirement.dependent))
                .collect(),
            add_seeds: patch.add_unavailable,
            remove_seeds: patch.remove_unavailable,
            enable_rules: patch.enable_requirements,
            disable_rules: patch.disable_requirements,
        };
        let (stats, appended) = apply_structural_patch_maintained(
            &mut self.death_program,
            &mut self.death_index,
            &mut self.certificate.death,
            &mut self.witness_index,
            grounded,
        )?;
        let changed_atoms = old_live
            .changed_indices(&self.certificate.death.live)
            .into_iter()
            .map(GroundedAtomId::new)
            .collect();
        Ok(BipolarSupportPatchOutcome {
            work: stats,
            appended_requirements: appended,
            changed_atoms,
        })
    }
}

pub fn check_bipolar_support(
    program: &BipolarSupportProgram,
    certificate: &BipolarSupportCertificate,
) -> Result<(), GroundedClosureError> {
    let death_program = program.death_program()?;
    check_bipolar_support_with_death(program, &death_program, certificate)
}

fn check_bipolar_support_with_death(
    program: &BipolarSupportProgram,
    death_program: &GroundedProgram,
    certificate: &BipolarSupportCertificate,
) -> Result<(), GroundedClosureError> {
    if certificate.death.live.len() != program.atom_count
        || death_program.atom_count() != program.atom_count
    {
        return Err(GroundedClosureError::CertificateShapeMismatch);
    }
    check(death_program, &certificate.death)?;
    for index in 0..program.atom_count {
        let atom = GroundedAtomId::new(index);
        if certificate.is_supported(atom) == certificate.death.is_live(atom) {
            return Err(GroundedClosureError::CertificateShapeMismatch);
        }
    }
    for &atom in &program.unavailable {
        if certificate.is_supported(atom) {
            return Err(GroundedClosureError::MissingSeed(atom));
        }
    }
    for requirement in &program.requirements {
        if certificate.is_supported(requirement.dependent)
            && !requirement
                .supporters
                .iter()
                .any(|supporter| certificate.is_supported(*supporter))
        {
            return Err(GroundedClosureError::NotClosedUnderRule(
                GroundedRuleId::new(0),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedIncidenceIndex {
    dependents: PersistentVec<Vec<GroundedRuleId>>,
    by_head: PersistentVec<Vec<GroundedRuleId>>,
}

impl GroundedIncidenceIndex {
    #[must_use]
    pub fn compile(program: &GroundedProgram) -> Self {
        let mut dependents = PersistentVec::from_vec(vec![Vec::new(); program.atom_count]);
        let mut by_head = PersistentVec::from_vec(vec![Vec::new(); program.atom_count]);
        for (index, rule) in program.rules.iter().enumerate() {
            let rule_id = GroundedRuleId::new(index);
            by_head[rule.head.index()].push(rule_id);
            for &atom in &rule.body {
                dependents[atom.index()].push(rule_id);
            }
        }
        Self {
            dependents,
            by_head,
        }
    }

    fn resize_atoms(&mut self, atom_count: usize) {
        self.dependents.resize_with(atom_count, Vec::new);
        self.by_head.resize_with(atom_count, Vec::new);
    }

    fn append_rule(&mut self, rule_id: GroundedRuleId, rule: &GroundedRule) {
        self.by_head[rule.head.index()].push(rule_id);
        for &atom in &rule.body {
            self.dependents[atom.index()].push(rule_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroundedWitnessIndex {
    children: PersistentVec<BTreeSet<GroundedAtomId>>,
}

impl GroundedWitnessIndex {
    fn compile(program: &GroundedProgram, certificate: &GroundedCertificate) -> Self {
        let mut index = Self {
            children: PersistentVec::from_vec(vec![BTreeSet::new(); program.atom_count]),
        };
        for atom_index in 0..program.atom_count {
            let atom = GroundedAtomId::new(atom_index);
            index.install_from_certificate(program, certificate, atom);
        }
        index
    }

    fn resize_atoms(&mut self, atom_count: usize) {
        self.children.resize_with(atom_count, BTreeSet::new);
    }

    fn install_from_certificate(
        &mut self,
        program: &GroundedProgram,
        certificate: &GroundedCertificate,
        atom: GroundedAtomId,
    ) {
        let Some(GroundedWitness::Rule(rule_id)) = certificate.witness(atom) else {
            return;
        };
        for &premise in &program.rules[rule_id.index()].body {
            self.children[premise.index()].insert(atom);
        }
    }

    fn remove_from_certificate(
        &mut self,
        program: &GroundedProgram,
        certificate: &GroundedCertificate,
        atom: GroundedAtomId,
    ) {
        let Some(GroundedWitness::Rule(rule_id)) = certificate.witness(atom) else {
            return;
        };
        for &premise in &program.rules[rule_id.index()].body {
            self.children[premise.index()].remove(&atom);
        }
    }
}

struct SolverState {
    live: Vec<bool>,
    rank: Vec<Option<usize>>,
    witness: Vec<Option<GroundedWitness>>,
    fired: Vec<bool>,
    queue: VecDeque<GroundedAtomId>,
    stats: GroundedWorkStats,
}

impl SolverState {
    fn fire_rule(&mut self, program: &GroundedProgram, rule_id: GroundedRuleId) {
        if self.fired[rule_id.index()] {
            return;
        }
        self.fired[rule_id.index()] = true;
        self.stats.rule_fires = self.stats.rule_fires.saturating_add(1);
        let rule = &program.rules[rule_id.index()];
        if self.live[rule.head.index()] {
            return;
        }
        let max_rank = rule
            .body
            .iter()
            .filter_map(|atom| self.rank[atom.index()])
            .max();
        let rank = max_rank.map_or(0, |value| value.saturating_add(1));
        self.live[rule.head.index()] = true;
        self.rank[rule.head.index()] = Some(rank);
        self.witness[rule.head.index()] = Some(GroundedWitness::Rule(rule_id));
        self.queue.push_back(rule.head);
    }
}

#[must_use]
pub fn solve(program: &GroundedProgram) -> (GroundedCertificate, GroundedWorkStats) {
    let index = GroundedIncidenceIndex::compile(program);
    solve_indexed(program, &index)
}

#[must_use]
pub fn solve_indexed(
    program: &GroundedProgram,
    index: &GroundedIncidenceIndex,
) -> (GroundedCertificate, GroundedWorkStats) {
    let mut remaining = vec![0_usize; program.rules.len()];
    let mut state = SolverState {
        live: vec![false; program.atom_count],
        rank: vec![None; program.atom_count],
        witness: vec![None; program.atom_count],
        fired: vec![false; program.rules.len()],
        queue: VecDeque::new(),
        stats: GroundedWorkStats::default(),
    };

    for seed in program.seeds.iter() {
        if !state.live[seed.index()] {
            state.live[seed.index()] = true;
            state.rank[seed.index()] = Some(0);
            state.witness[seed.index()] = Some(GroundedWitness::Seed);
            state.queue.push_back(seed);
        }
    }
    for (index, rule) in program.rules.iter().enumerate() {
        if rule.enabled {
            remaining[index] = rule.body.len();
        }
    }
    for (index, &left) in remaining.iter().enumerate() {
        if program.rules[index].enabled && left == 0 {
            state.fire_rule(program, GroundedRuleId::new(index));
        }
    }
    while let Some(atom) = state.queue.pop_front() {
        for &rule_id in &index.dependents[atom.index()] {
            if state.fired[rule_id.index()] || remaining[rule_id.index()] == 0 {
                continue;
            }
            state.stats.incidence_updates = state.stats.incidence_updates.saturating_add(1);
            remaining[rule_id.index()] -= 1;
            if remaining[rule_id.index()] == 0 {
                state.fire_rule(program, rule_id);
            }
        }
    }

    (
        GroundedCertificate {
            live: PersistentVec::from_vec(state.live),
            rank: PersistentVec::from_vec(state.rank),
            witness: PersistentVec::from_vec(state.witness),
        },
        state.stats,
    )
}

pub fn check(
    program: &GroundedProgram,
    certificate: &GroundedCertificate,
) -> Result<(), GroundedClosureError> {
    if certificate.live.len() != program.atom_count
        || certificate.rank.len() != program.atom_count
        || certificate.witness.len() != program.atom_count
    {
        return Err(GroundedClosureError::CertificateShapeMismatch);
    }

    for index in 0..program.atom_count {
        let atom = GroundedAtomId::new(index);
        if !certificate.live[index] {
            if certificate.rank[index].is_some() || certificate.witness[index].is_some() {
                return Err(GroundedClosureError::DeadAtomHasProofState(atom));
            }
            continue;
        }
        let rank =
            certificate.rank[index].ok_or(GroundedClosureError::LiveAtomMissingRank(atom))?;
        let witness =
            certificate.witness[index].ok_or(GroundedClosureError::LiveAtomMissingWitness(atom))?;
        match witness {
            GroundedWitness::Seed => {
                if !program.seeds.contains(atom) {
                    return Err(GroundedClosureError::SeedWitnessForNonSeed(atom));
                }
                if rank != 0 {
                    return Err(GroundedClosureError::SeedHasNonZeroRank(atom));
                }
            }
            GroundedWitness::Rule(rule_id) => {
                let rule = program
                    .rules
                    .get(rule_id.index())
                    .ok_or(GroundedClosureError::RuleOutsideProgram(rule_id))?;
                if !rule.enabled {
                    return Err(GroundedClosureError::WitnessRuleDisabled(rule_id));
                }
                if rule.head != atom {
                    return Err(GroundedClosureError::WitnessRuleHeadMismatch {
                        atom,
                        rule: rule_id,
                    });
                }
                for &premise in &rule.body {
                    if !certificate.live[premise.index()] {
                        return Err(GroundedClosureError::WitnessPremiseDead { atom, premise });
                    }
                    let premise_rank = certificate.rank[premise.index()]
                        .ok_or(GroundedClosureError::LiveAtomMissingRank(premise))?;
                    if premise_rank >= rank {
                        return Err(GroundedClosureError::WitnessRankNotStrictlySmaller {
                            atom,
                            premise,
                        });
                    }
                }
            }
        }
    }

    for seed in program.seeds.iter() {
        if !certificate.live[seed.index()] {
            return Err(GroundedClosureError::MissingSeed(seed));
        }
    }
    for (index, rule) in program.rules.iter().enumerate() {
        if rule.enabled
            && rule.body.iter().all(|atom| certificate.live[atom.index()])
            && !certificate.live[rule.head.index()]
        {
            return Err(GroundedClosureError::NotClosedUnderRule(
                GroundedRuleId::new(index),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroundedUpdate {
    pub add_seeds: Vec<GroundedAtomId>,
    pub remove_seeds: Vec<GroundedAtomId>,
    pub enable_rules: Vec<GroundedRuleId>,
    pub disable_rules: Vec<GroundedRuleId>,
}

/// Append-oriented structural mutation of a grounded program.
///
/// Existing atom and rule ids remain stable. New atoms occupy the next
/// contiguous ids and appended rules occupy the next rule ids. Existing rules
/// may be disabled/enabled, which lets clients version a changed rule as
/// `disable old + append replacement` without rebuilding the whole program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroundedStructuralPatch {
    pub append_atoms: usize,
    pub append_rules: Vec<GroundedRule>,
    pub add_seeds: Vec<GroundedAtomId>,
    pub remove_seeds: Vec<GroundedAtomId>,
    pub enable_rules: Vec<GroundedRuleId>,
    pub disable_rules: Vec<GroundedRuleId>,
}

pub fn apply_update(
    program: &mut GroundedProgram,
    old: &GroundedCertificate,
    update: &GroundedUpdate,
) -> Result<(GroundedCertificate, GroundedWorkStats), GroundedClosureError> {
    validate_update(program, update)?;
    check(program, old)?;
    let index = GroundedIncidenceIndex::compile(program);
    apply_update_indexed(program, &index, old, update)
}

pub fn apply_update_indexed(
    program: &mut GroundedProgram,
    index: &GroundedIncidenceIndex,
    old: &GroundedCertificate,
    update: &GroundedUpdate,
) -> Result<(GroundedCertificate, GroundedWorkStats), GroundedClosureError> {
    validate_update(program, update)?;
    check(program, old)?;
    let result = apply_update_indexed_validated(program, index, old, update);
    check(program, &result.0)?;
    Ok(result)
}

fn apply_update_indexed_validated(
    program: &mut GroundedProgram,
    index: &GroundedIncidenceIndex,
    old: &GroundedCertificate,
    update: &GroundedUpdate,
) -> (GroundedCertificate, GroundedWorkStats) {
    // Capture the only old-state facts needed after mutation. Cloning the
    // complete program here made a tiny rule toggle pay O(program size).
    let mut direct = BTreeSet::new();
    for &seed in &update.remove_seeds {
        if old.is_live(seed) && old.witness(seed) == Some(GroundedWitness::Seed) {
            direct.insert(seed);
        }
    }
    for &rule_id in &update.disable_rules {
        let rule = &program.rules[rule_id.index()];
        if rule.enabled
            && old.is_live(rule.head)
            && old.witness(rule.head) == Some(GroundedWitness::Rule(rule_id))
        {
            direct.insert(rule.head);
        }
    }

    for &seed in &update.remove_seeds {
        program.seeds.remove(seed);
    }
    for &rule_id in &update.disable_rules {
        program.rules[rule_id.index()].enabled = false;
    }

    let (mut certificate, mut stats) = if direct.is_empty() {
        (old.clone(), GroundedWorkStats::default())
    } else {
        local_recompute_after_deletion_indexed(program, index, old, &direct)
    };

    for &seed in &update.add_seeds {
        program.seeds.insert(seed);
    }
    for &rule_id in &update.enable_rules {
        program.rules[rule_id.index()].enabled = true;
    }
    incremental_insertions(program, index, &mut certificate, update, &mut stats);
    (certificate, stats)
}

/// Apply an append-oriented structural patch while preserving existing ids and
/// extending the already-compiled incidence index only for the new structure.
///
/// The returned rule ids correspond one-for-one to `patch.append_rules`.
pub fn apply_structural_patch_indexed(
    program: &mut GroundedProgram,
    index: &mut GroundedIncidenceIndex,
    old: &GroundedCertificate,
    patch: GroundedStructuralPatch,
) -> Result<(GroundedCertificate, GroundedWorkStats, Vec<GroundedRuleId>), GroundedClosureError> {
    apply_structural_patch_indexed_impl(program, index, old, patch, true)
}

fn apply_structural_patch_indexed_impl(
    program: &mut GroundedProgram,
    index: &mut GroundedIncidenceIndex,
    old: &GroundedCertificate,
    patch: GroundedStructuralPatch,
    verify_full_certificate: bool,
) -> Result<(GroundedCertificate, GroundedWorkStats, Vec<GroundedRuleId>), GroundedClosureError> {
    if verify_full_certificate {
        check(program, old)?;
    }

    let old_rule_count = program.rules.len();
    let new_atom_count = program
        .atom_count
        .checked_add(patch.append_atoms)
        .ok_or(GroundedClosureError::StructuralUniverseOverflow)?;

    let validate_future_atom = |atom: GroundedAtomId| {
        if atom.index() < new_atom_count {
            Ok(())
        } else {
            Err(GroundedClosureError::AtomOutsideUniverse(atom))
        }
    };
    for &atom in patch.add_seeds.iter().chain(&patch.remove_seeds) {
        validate_future_atom(atom)?;
    }
    for rule in &patch.append_rules {
        validate_future_atom(rule.head)?;
        for &atom in &rule.body {
            validate_future_atom(atom)?;
        }
    }
    for &rule in patch.enable_rules.iter().chain(&patch.disable_rules) {
        if rule.index() >= old_rule_count {
            return Err(GroundedClosureError::RuleOutsideProgram(rule));
        }
    }

    program.atom_count = new_atom_count;
    program.seeds.resize_atoms(new_atom_count);
    index.resize_atoms(new_atom_count);

    let mut certificate = old.clone();
    certificate.live.resize(new_atom_count, false);
    certificate.rank.resize(new_atom_count, None);
    certificate.witness.resize(new_atom_count, None);

    let mut appended_rule_ids = Vec::with_capacity(patch.append_rules.len());
    let mut appended_enabled = Vec::new();
    for (offset, mut rule) in patch.append_rules.into_iter().enumerate() {
        let desired_enabled = rule.enabled;
        rule.enabled = false;
        let rule_id = GroundedRuleId::new(old_rule_count + offset);
        index.append_rule(rule_id, &rule);
        program.rules.push(rule);
        appended_rule_ids.push(rule_id);
        if desired_enabled {
            appended_enabled.push(rule_id);
        }
    }

    let mut enable_rules = patch.enable_rules;
    enable_rules.extend(appended_enabled);
    let update = GroundedUpdate {
        add_seeds: patch.add_seeds,
        remove_seeds: patch.remove_seeds,
        enable_rules,
        disable_rules: patch.disable_rules,
    };
    validate_update(program, &update)?;
    let (certificate, stats) =
        apply_update_indexed_validated(program, index, &certificate, &update);
    if verify_full_certificate {
        check(program, &certificate)?;
    }
    Ok((certificate, stats, appended_rule_ids))
}

fn apply_structural_patch_maintained(
    program: &mut GroundedProgram,
    index: &mut GroundedIncidenceIndex,
    certificate: &mut GroundedCertificate,
    witness_index: &mut GroundedWitnessIndex,
    patch: GroundedStructuralPatch,
) -> Result<(GroundedWorkStats, Vec<GroundedRuleId>), GroundedClosureError> {
    let old_rule_count = program.rules.len();
    let new_atom_count = program
        .atom_count
        .checked_add(patch.append_atoms)
        .ok_or(GroundedClosureError::StructuralUniverseOverflow)?;

    let validate_future_atom = |atom: GroundedAtomId| {
        if atom.index() < new_atom_count {
            Ok(())
        } else {
            Err(GroundedClosureError::AtomOutsideUniverse(atom))
        }
    };
    for &atom in patch.add_seeds.iter().chain(&patch.remove_seeds) {
        validate_future_atom(atom)?;
    }
    for rule in &patch.append_rules {
        validate_future_atom(rule.head)?;
        for &atom in &rule.body {
            validate_future_atom(atom)?;
        }
    }
    for &rule in patch.enable_rules.iter().chain(&patch.disable_rules) {
        if rule.index() >= old_rule_count {
            return Err(GroundedClosureError::RuleOutsideProgram(rule));
        }
    }

    program.atom_count = new_atom_count;
    program.seeds.resize_atoms(new_atom_count);
    index.resize_atoms(new_atom_count);
    witness_index.resize_atoms(new_atom_count);
    certificate.live.resize(new_atom_count, false);
    certificate.rank.resize(new_atom_count, None);
    certificate.witness.resize(new_atom_count, None);

    let mut appended_rule_ids = Vec::with_capacity(patch.append_rules.len());
    let mut appended_enabled = Vec::new();
    for (offset, mut rule) in patch.append_rules.into_iter().enumerate() {
        let desired_enabled = rule.enabled;
        rule.enabled = false;
        let rule_id = GroundedRuleId::new(old_rule_count + offset);
        index.append_rule(rule_id, &rule);
        program.rules.push(rule);
        appended_rule_ids.push(rule_id);
        if desired_enabled {
            appended_enabled.push(rule_id);
        }
    }

    let mut enable_rules = patch.enable_rules;
    enable_rules.extend(appended_enabled);
    let update = GroundedUpdate {
        add_seeds: patch.add_seeds,
        remove_seeds: patch.remove_seeds,
        enable_rules,
        disable_rules: patch.disable_rules,
    };
    validate_update(program, &update)?;

    let direct = invalidate_update_sources(program, certificate, &update);

    let mut stats = if direct.is_empty() {
        GroundedWorkStats::default()
    } else {
        local_recompute_after_deletion_sparse_indexed(
            program,
            index,
            certificate,
            witness_index,
            &direct,
        )
    };

    for &seed in &update.add_seeds {
        program.seeds.insert(seed);
    }
    for &rule_id in &update.enable_rules {
        program.rules[rule_id.index()].enabled = true;
    }
    incremental_insertions_with_witness_index(
        program,
        index,
        certificate,
        witness_index,
        &update,
        &mut stats,
    );
    Ok((stats, appended_rule_ids))
}

fn invalidate_update_sources(
    program: &mut GroundedProgram,
    certificate: &GroundedCertificate,
    update: &GroundedUpdate,
) -> BTreeSet<GroundedAtomId> {
    let mut direct = BTreeSet::new();
    for &seed in &update.remove_seeds {
        if certificate.is_live(seed) && certificate.witness(seed) == Some(GroundedWitness::Seed) {
            direct.insert(seed);
        }
        program.seeds.remove(seed);
    }
    for &rule_id in &update.disable_rules {
        let rule = &program.rules[rule_id.index()];
        if rule.enabled
            && certificate.is_live(rule.head)
            && certificate.witness(rule.head) == Some(GroundedWitness::Rule(rule_id))
        {
            direct.insert(rule.head);
        }
        program.rules[rule_id.index()].enabled = false;
    }
    direct
}

/// Reconcile an exact grounded certificate after a structural program change.
///
/// Unlike [`GroundedUpdate`], this path permits an append-only atom universe,
/// rule insertion/removal/reordering and rule-body replacement. Exact old
/// witnesses whose rule survives semantically are remapped to their new rule
/// id. Only atoms whose selected proof disappeared are invalidated, after
/// which the ordinary witness cone is recomputed. New seeds and new/changed
/// enabled rules then drive the forward closure.
pub fn reconcile_structural_change(
    old_program: &GroundedProgram,
    new_program: &GroundedProgram,
    old: &GroundedCertificate,
) -> Result<(GroundedCertificate, GroundedWorkStats), GroundedClosureError> {
    old_program.validate()?;
    new_program.validate()?;
    check(old_program, old)?;
    if new_program.atom_count < old_program.atom_count {
        return Err(GroundedClosureError::StructuralUniverseShrank {
            old_atom_count: old_program.atom_count,
            new_atom_count: new_program.atom_count,
        });
    }

    let mut old_to_new_rule = vec![None; old_program.rules.len()];
    let mut matched_new_rule = vec![false; new_program.rules.len()];

    // QCN and other append-oriented clients preserve rule identities for the
    // overwhelming majority of structural updates. Match those stable slots
    // without cloning every rule body into the fallback content map. Only the
    // genuinely changed/reordered rules pay canonical-key allocation.
    for index in 0..old_program.rules.len().min(new_program.rules.len()) {
        if old_program.rules[index] == new_program.rules[index] {
            let rule = GroundedRuleId::new(index);
            old_to_new_rule[index] = Some(rule);
            matched_new_rule[index] = true;
        }
    }

    let mut new_by_key = BTreeMap::<GroundedRuleKey, VecDeque<GroundedRuleId>>::new();
    for (index, rule) in new_program.rules.iter().enumerate() {
        if matched_new_rule[index] {
            continue;
        }
        new_by_key
            .entry(grounded_rule_key(rule))
            .or_default()
            .push_back(GroundedRuleId::new(index));
    }

    for (index, rule) in old_program.rules.iter().enumerate() {
        if old_to_new_rule[index].is_some() {
            continue;
        }
        let Some(ids) = new_by_key.get_mut(&grounded_rule_key(rule)) else {
            continue;
        };
        let Some(new_id) = ids.pop_front() else {
            continue;
        };
        old_to_new_rule[index] = Some(new_id);
        matched_new_rule[new_id.index()] = true;
    }

    let mut remapped = old.clone();
    remapped.live.resize(new_program.atom_count, false);
    remapped.rank.resize(new_program.atom_count, None);
    remapped.witness.resize(new_program.atom_count, None);
    let mut direct = BTreeSet::new();
    for atom_index in 0..old_program.atom_count {
        let atom = GroundedAtomId::new(atom_index);
        if !old.is_live(atom) {
            continue;
        }
        match old.witness(atom) {
            Some(GroundedWitness::Seed) => {
                if !new_program.seeds.contains(atom) {
                    direct.insert(atom);
                    remapped.witness[atom_index] = None;
                }
            }
            Some(GroundedWitness::Rule(old_rule)) => {
                if let Some(new_rule) = old_to_new_rule[old_rule.index()] {
                    remapped.witness[atom_index] = Some(GroundedWitness::Rule(new_rule));
                } else {
                    direct.insert(atom);
                    remapped.witness[atom_index] = None;
                }
            }
            None => return Err(GroundedClosureError::LiveAtomMissingWitness(atom)),
        }
    }

    let index = GroundedIncidenceIndex::compile(new_program);
    let (mut certificate, mut stats) = if direct.is_empty() {
        (remapped, GroundedWorkStats::default())
    } else {
        local_recompute_after_deletion_indexed(new_program, &index, &remapped, &direct)
    };

    let update = GroundedUpdate {
        add_seeds: new_program.seeds.difference(&old_program.seeds).collect(),
        enable_rules: new_program
            .rules
            .iter()
            .enumerate()
            .filter_map(|(index, rule)| {
                (rule.enabled && !matched_new_rule[index]).then_some(GroundedRuleId::new(index))
            })
            .collect(),
        ..GroundedUpdate::default()
    };
    incremental_insertions(new_program, &index, &mut certificate, &update, &mut stats);
    check(new_program, &certificate)?;
    Ok((certificate, stats))
}

fn validate_update(
    program: &GroundedProgram,
    update: &GroundedUpdate,
) -> Result<(), GroundedClosureError> {
    for &atom in update.add_seeds.iter().chain(&update.remove_seeds) {
        program.validate_atom(atom)?;
    }
    for &rule in update.enable_rules.iter().chain(&update.disable_rules) {
        program.validate_rule(rule)?;
    }
    Ok(())
}

fn incremental_insertions(
    program: &GroundedProgram,
    index: &GroundedIncidenceIndex,
    certificate: &mut GroundedCertificate,
    update: &GroundedUpdate,
    stats: &mut GroundedWorkStats,
) {
    let mut queue = VecDeque::new();
    for &seed in &update.add_seeds {
        if !certificate.is_live(seed) {
            certificate.live[seed.index()] = true;
            certificate.rank[seed.index()] = Some(0);
            certificate.witness[seed.index()] = Some(GroundedWitness::Seed);
            queue.push_back(seed);
        }
    }

    for &rule_id in &update.enable_rules {
        let rule = &program.rules[rule_id.index()];
        if !rule.enabled || certificate.is_live(rule.head) {
            continue;
        }
        if rule.body.iter().all(|atom| certificate.is_live(*atom)) {
            stats.rule_fires = stats.rule_fires.saturating_add(1);
            derive_from_rule(certificate, program, rule_id, &mut queue);
        }
    }

    while let Some(atom) = queue.pop_front() {
        for &rule_id in &index.dependents[atom.index()] {
            let rule = &program.rules[rule_id.index()];
            if !rule.enabled || certificate.is_live(rule.head) {
                continue;
            }
            stats.incidence_updates = stats.incidence_updates.saturating_add(1);
            if rule
                .body
                .iter()
                .all(|premise| certificate.is_live(*premise))
            {
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                derive_from_rule(certificate, program, rule_id, &mut queue);
            }
        }
    }
}

fn incremental_insertions_with_witness_index(
    program: &GroundedProgram,
    index: &GroundedIncidenceIndex,
    certificate: &mut GroundedCertificate,
    witness_index: &mut GroundedWitnessIndex,
    update: &GroundedUpdate,
    stats: &mut GroundedWorkStats,
) {
    let mut queue = VecDeque::new();
    for &seed in &update.add_seeds {
        if !certificate.is_live(seed) {
            certificate.live[seed.index()] = true;
            certificate.rank[seed.index()] = Some(0);
            certificate.witness[seed.index()] = Some(GroundedWitness::Seed);
            queue.push_back(seed);
        }
    }

    for &rule_id in &update.enable_rules {
        let rule = &program.rules[rule_id.index()];
        if !rule.enabled || certificate.is_live(rule.head) {
            continue;
        }
        if rule.body.iter().all(|atom| certificate.is_live(*atom)) {
            stats.rule_fires = stats.rule_fires.saturating_add(1);
            derive_from_rule_with_witness_index(
                certificate,
                witness_index,
                program,
                rule_id,
                &mut queue,
            );
        }
    }

    while let Some(atom) = queue.pop_front() {
        for &rule_id in &index.dependents[atom.index()] {
            let rule = &program.rules[rule_id.index()];
            if !rule.enabled || certificate.is_live(rule.head) {
                continue;
            }
            stats.incidence_updates = stats.incidence_updates.saturating_add(1);
            if rule
                .body
                .iter()
                .all(|premise| certificate.is_live(*premise))
            {
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                derive_from_rule_with_witness_index(
                    certificate,
                    witness_index,
                    program,
                    rule_id,
                    &mut queue,
                );
            }
        }
    }
}

fn derive_from_rule(
    certificate: &mut GroundedCertificate,
    program: &GroundedProgram,
    rule_id: GroundedRuleId,
    queue: &mut VecDeque<GroundedAtomId>,
) {
    let rule = &program.rules[rule_id.index()];
    if certificate.is_live(rule.head) {
        return;
    }
    let max_rank = rule
        .body
        .iter()
        .filter_map(|atom| certificate.rank(*atom))
        .max();
    certificate.live[rule.head.index()] = true;
    certificate.rank[rule.head.index()] = Some(max_rank.map_or(0, |rank| rank.saturating_add(1)));
    certificate.witness[rule.head.index()] = Some(GroundedWitness::Rule(rule_id));
    queue.push_back(rule.head);
}

fn derive_from_rule_with_witness_index(
    certificate: &mut GroundedCertificate,
    witness_index: &mut GroundedWitnessIndex,
    program: &GroundedProgram,
    rule_id: GroundedRuleId,
    queue: &mut VecDeque<GroundedAtomId>,
) {
    let rule = &program.rules[rule_id.index()];
    if certificate.is_live(rule.head) {
        return;
    }
    let max_rank = rule
        .body
        .iter()
        .filter_map(|atom| certificate.rank(*atom))
        .max();
    certificate.live[rule.head.index()] = true;
    certificate.rank[rule.head.index()] = Some(max_rank.map_or(0, |rank| rank.saturating_add(1)));
    certificate.witness[rule.head.index()] = Some(GroundedWitness::Rule(rule_id));
    witness_index.install_from_certificate(program, certificate, rule.head);
    queue.push_back(rule.head);
}

fn local_recompute_after_deletion_sparse_indexed(
    program: &GroundedProgram,
    index: &GroundedIncidenceIndex,
    certificate: &mut GroundedCertificate,
    witness_index: &mut GroundedWitnessIndex,
    direct: &BTreeSet<GroundedAtomId>,
) -> GroundedWorkStats {
    let mut stats = GroundedWorkStats::default();
    let affected = collect_sparse_witness_cone(certificate, witness_index, direct, &mut stats);
    stats.affected_atoms = affected.len();

    for &atom in &affected {
        witness_index.remove_from_certificate(program, certificate, atom);
    }
    for &atom in &affected {
        certificate.live[atom.index()] = false;
        certificate.rank[atom.index()] = None;
        certificate.witness[atom.index()] = None;
    }
    for seed in program.seeds.iter() {
        if affected.contains(&seed) {
            certificate.live[seed.index()] = true;
            certificate.rank[seed.index()] = Some(0);
            certificate.witness[seed.index()] = Some(GroundedWitness::Seed);
        }
    }

    let mut remaining = BTreeMap::<GroundedRuleId, usize>::new();
    let mut local_dependents = BTreeMap::<GroundedAtomId, Vec<GroundedRuleId>>::new();
    let mut seen_rules = BTreeSet::new();
    let mut fired = BTreeSet::new();
    let mut work = VecDeque::new();

    for &head in &affected {
        for &rule_id in &index.by_head[head.index()] {
            if !seen_rules.insert(rule_id) {
                continue;
            }
            let rule = &program.rules[rule_id.index()];
            if !rule.enabled {
                continue;
            }
            let missing = rule
                .body
                .iter()
                .filter(|atom| !certificate.is_live(**atom))
                .count();
            remaining.insert(rule_id, missing);
            for &premise in &rule.body {
                if affected.contains(&premise) {
                    local_dependents.entry(premise).or_default().push(rule_id);
                }
            }
            if missing == 0 {
                fired.insert(rule_id);
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                if !certificate.is_live(rule.head) {
                    derive_from_rule_with_witness_index(
                        certificate,
                        witness_index,
                        program,
                        rule_id,
                        &mut work,
                    );
                }
            }
        }
    }

    while let Some(atom) = work.pop_front() {
        let Some(dependents) = local_dependents.get(&atom) else {
            continue;
        };
        for &rule_id in dependents {
            if fired.contains(&rule_id) {
                continue;
            }
            let Some(left) = remaining.get_mut(&rule_id) else {
                continue;
            };
            if *left == 0 {
                continue;
            }
            stats.incidence_updates = stats.incidence_updates.saturating_add(1);
            *left -= 1;
            if *left == 0 {
                fired.insert(rule_id);
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                let head = program.rules[rule_id.index()].head;
                if !certificate.is_live(head) {
                    derive_from_rule_with_witness_index(
                        certificate,
                        witness_index,
                        program,
                        rule_id,
                        &mut work,
                    );
                }
            }
        }
    }
    stats
}

fn collect_sparse_witness_cone(
    certificate: &GroundedCertificate,
    witness_index: &GroundedWitnessIndex,
    direct: &BTreeSet<GroundedAtomId>,
    stats: &mut GroundedWorkStats,
) -> BTreeSet<GroundedAtomId> {
    let mut affected = BTreeSet::new();
    let mut queue = VecDeque::new();
    for &atom in direct {
        if certificate.is_live(atom) && affected.insert(atom) {
            queue.push_back(atom);
        }
    }
    while let Some(atom) = queue.pop_front() {
        for child in witness_index.children[atom.index()].iter().copied() {
            stats.dependency_walks = stats.dependency_walks.saturating_add(1);
            if certificate.is_live(child) && affected.insert(child) {
                queue.push_back(child);
            }
        }
    }
    affected
}

#[must_use]
pub fn local_recompute_after_deletion(
    program: &GroundedProgram,
    old: &GroundedCertificate,
    direct: &BTreeSet<GroundedAtomId>,
) -> (GroundedCertificate, GroundedWorkStats) {
    let index = GroundedIncidenceIndex::compile(program);
    local_recompute_after_deletion_indexed(program, &index, old, direct)
}

#[must_use]
pub fn local_recompute_after_deletion_indexed(
    program: &GroundedProgram,
    index: &GroundedIncidenceIndex,
    old: &GroundedCertificate,
    direct: &BTreeSet<GroundedAtomId>,
) -> (GroundedCertificate, GroundedWorkStats) {
    let mut witness_children = vec![Vec::new(); program.atom_count];
    for atom_index in 0..program.atom_count {
        let atom = GroundedAtomId::new(atom_index);
        let Some(GroundedWitness::Rule(rule_id)) = old.witness(atom) else {
            continue;
        };
        for &premise in &program.rules[rule_id.index()].body {
            witness_children[premise.index()].push(atom);
        }
    }

    let mut affected = vec![false; program.atom_count];
    let mut queue = VecDeque::new();
    let mut stats = GroundedWorkStats::default();
    for &atom in direct {
        if old.is_live(atom) && !affected[atom.index()] {
            affected[atom.index()] = true;
            queue.push_back(atom);
        }
    }
    while let Some(atom) = queue.pop_front() {
        for &child in &witness_children[atom.index()] {
            stats.dependency_walks = stats.dependency_walks.saturating_add(1);
            if old.is_live(child) && !affected[child.index()] {
                affected[child.index()] = true;
                queue.push_back(child);
            }
        }
    }

    let mut certificate = old.clone();
    for (index, &is_affected) in affected.iter().enumerate() {
        if is_affected {
            certificate.live[index] = false;
            certificate.rank[index] = None;
            certificate.witness[index] = None;
        }
    }
    for seed in program.seeds.iter() {
        if affected[seed.index()] {
            certificate.live[seed.index()] = true;
            certificate.rank[seed.index()] = Some(0);
            certificate.witness[seed.index()] = Some(GroundedWitness::Seed);
        }
    }

    let mut local_dependents = vec![Vec::new(); program.atom_count];
    let mut remaining = vec![usize::MAX; program.rules.len()];
    let mut fired = vec![false; program.rules.len()];
    let mut work = VecDeque::new();
    stats.affected_atoms = affected.iter().filter(|&&value| value).count();

    let mut seen_rule = vec![false; program.rules.len()];
    for (head_index, &is_affected) in affected.iter().enumerate() {
        if !is_affected {
            continue;
        }
        for &rule_id in &index.by_head[head_index] {
            if seen_rule[rule_id.index()] {
                continue;
            }
            seen_rule[rule_id.index()] = true;
            let rule = &program.rules[rule_id.index()];
            if !rule.enabled {
                continue;
            }
            remaining[rule_id.index()] = rule
                .body
                .iter()
                .filter(|atom| !certificate.is_live(**atom))
                .count();
            for &premise in &rule.body {
                if affected[premise.index()] {
                    local_dependents[premise.index()].push(rule_id);
                }
            }
            if remaining[rule_id.index()] == 0 {
                fired[rule_id.index()] = true;
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                if !certificate.is_live(rule.head) {
                    derive_from_rule(&mut certificate, program, rule_id, &mut work);
                }
            }
        }
    }

    while let Some(atom) = work.pop_front() {
        for &rule_id in &local_dependents[atom.index()] {
            if fired[rule_id.index()] || remaining[rule_id.index()] == 0 {
                continue;
            }
            stats.incidence_updates = stats.incidence_updates.saturating_add(1);
            remaining[rule_id.index()] -= 1;
            if remaining[rule_id.index()] == 0 {
                fired[rule_id.index()] = true;
                stats.rule_fires = stats.rule_fires.saturating_add(1);
                let head = program.rules[rule_id.index()].head;
                if !certificate.is_live(head) {
                    derive_from_rule(&mut certificate, program, rule_id, &mut work);
                }
            }
        }
    }
    (certificate, stats)
}

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
