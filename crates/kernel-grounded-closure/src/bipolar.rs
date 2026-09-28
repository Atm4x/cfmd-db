use super::{
    BTreeSet, GroundedAtomId, GroundedCertificate, GroundedClosureError, GroundedProgram,
    GroundedRule, GroundedRuleId, GroundedStructuralPatch, GroundedWorkStats, check,
    reconcile_structural_change, solve, solve_indexed,
};
use crate::index::{GroundedIncidenceIndex, GroundedWitnessIndex};
use crate::maintenance::apply_structural_patch_maintained;

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
    pub(super) death: GroundedCertificate,
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
    pub(super) death_index: GroundedIncidenceIndex,
    pub(super) certificate: BipolarSupportCertificate,
    pub(super) witness_index: GroundedWitnessIndex,
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
