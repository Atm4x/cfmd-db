use super::{
    BTreeSet, GroundedAtomId, GroundedCertificate, GroundedProgram, GroundedRule, GroundedRuleId,
    GroundedWitness, PersistentVec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundedIncidenceIndex {
    pub(super) dependents: PersistentVec<Vec<GroundedRuleId>>,
    pub(super) by_head: PersistentVec<Vec<GroundedRuleId>>,
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

    pub(super) fn resize_atoms(&mut self, atom_count: usize) {
        self.dependents.resize_with(atom_count, Vec::new);
        self.by_head.resize_with(atom_count, Vec::new);
    }

    pub(super) fn append_rule(&mut self, rule_id: GroundedRuleId, rule: &GroundedRule) {
        self.by_head[rule.head.index()].push(rule_id);
        for &atom in &rule.body {
            self.dependents[atom.index()].push(rule_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GroundedWitnessIndex {
    pub(super) children: PersistentVec<BTreeSet<GroundedAtomId>>,
}

impl GroundedWitnessIndex {
    pub(super) fn compile(program: &GroundedProgram, certificate: &GroundedCertificate) -> Self {
        let mut index = Self {
            children: PersistentVec::from_vec(vec![BTreeSet::new(); program.atom_count]),
        };
        for atom in certificate.live_atoms() {
            index.install_from_certificate(program, certificate, atom);
        }
        index
    }

    pub(super) fn resize_atoms(&mut self, atom_count: usize) {
        self.children.resize_with(atom_count, BTreeSet::new);
    }

    pub(super) fn install_from_certificate(
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

    pub(super) fn remove_from_certificate(
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
