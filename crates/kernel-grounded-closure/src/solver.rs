use std::collections::VecDeque;

use super::{
    GroundedAtomId, GroundedCertificate, GroundedClosureError, GroundedProgram, GroundedRuleId,
    GroundedWitness, GroundedWorkStats, PersistentVec,
};
use crate::index::GroundedIncidenceIndex;

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
