use super::{
    BTreeMap, BTreeSet, GroundedAtomId, GroundedCertificate, GroundedClosureError, GroundedProgram,
    GroundedRule, GroundedRuleId, GroundedWitness, GroundedWorkStats, VecDeque, check,
    solve_indexed,
};
use crate::index::{GroundedIncidenceIndex, GroundedWitnessIndex};

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

/// First-class incremental state for the grounded least-fixpoint calculus.
///
/// The incidence graph and selected-witness reverse index are maintained across
/// updates. Deletions therefore repair only the selected witness cone instead
/// of reconstructing dependency state from the whole finite universe.
#[derive(Debug, Clone)]
pub struct GroundedMaintenance {
    program: GroundedProgram,
    index: GroundedIncidenceIndex,
    certificate: GroundedCertificate,
    witness_index: GroundedWitnessIndex,
}

impl GroundedMaintenance {
    pub fn new(program: GroundedProgram) -> Result<Self, GroundedClosureError> {
        program.validate()?;
        let index = GroundedIncidenceIndex::compile(&program);
        let (certificate, _) = solve_indexed(&program, &index);
        let witness_index = GroundedWitnessIndex::compile(&program, &certificate);
        Ok(Self {
            program,
            index,
            certificate,
            witness_index,
        })
    }

    #[must_use]
    pub const fn program(&self) -> &GroundedProgram {
        &self.program
    }

    #[must_use]
    pub const fn certificate(&self) -> &GroundedCertificate {
        &self.certificate
    }

    pub fn apply_update(
        &mut self,
        update: &GroundedUpdate,
    ) -> Result<GroundedWorkStats, GroundedClosureError> {
        validate_update(&self.program, update)?;
        Ok(apply_update_maintained_validated(
            &mut self.program,
            &self.index,
            &mut self.certificate,
            &mut self.witness_index,
            update,
        ))
    }

    pub fn apply_structural_patch(
        &mut self,
        patch: GroundedStructuralPatch,
    ) -> Result<(GroundedWorkStats, Vec<GroundedRuleId>), GroundedClosureError> {
        apply_structural_patch_maintained(
            &mut self.program,
            &mut self.index,
            &mut self.certificate,
            &mut self.witness_index,
            patch,
        )
    }
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
    let mut certificate = old.clone();
    let mut witness_index = GroundedWitnessIndex::compile(program, old);
    let stats = apply_update_maintained_validated(
        program,
        index,
        &mut certificate,
        &mut witness_index,
        update,
    );
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

pub(super) fn apply_structural_patch_maintained(
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

    let stats =
        apply_update_maintained_validated(program, index, certificate, witness_index, &update);
    Ok((stats, appended_rule_ids))
}

fn apply_update_maintained_validated(
    program: &mut GroundedProgram,
    index: &GroundedIncidenceIndex,
    certificate: &mut GroundedCertificate,
    witness_index: &mut GroundedWitnessIndex,
    update: &GroundedUpdate,
) -> GroundedWorkStats {
    let direct = invalidate_update_sources(program, certificate, update);
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
        update,
        &mut stats,
    );
    stats
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

    let (old_to_new_rule, matched_new_rule) = correlate_rules(old_program, new_program);

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
    let mut certificate = remapped;
    let mut witness_index = GroundedWitnessIndex::compile(new_program, &certificate);
    let mut stats = if direct.is_empty() {
        GroundedWorkStats::default()
    } else {
        local_recompute_after_deletion_sparse_indexed(
            new_program,
            &index,
            &mut certificate,
            &mut witness_index,
            &direct,
        )
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
    incremental_insertions_with_witness_index(
        new_program,
        &index,
        &mut certificate,
        &mut witness_index,
        &update,
        &mut stats,
    );
    check(new_program, &certificate)?;
    Ok((certificate, stats))
}

fn correlate_rules(
    old_program: &GroundedProgram,
    new_program: &GroundedProgram,
) -> (Vec<Option<GroundedRuleId>>, Vec<bool>) {
    let mut old_to_new_rule = vec![None; old_program.rules.len()];
    let mut matched_new_rule = vec![false; new_program.rules.len()];

    // Rule correspondence is extensional equality plus occurrence order.
    // Stable slots are consumed first as an allocation-free positional fast path;
    // the residual correspondence uses borrowed rule values, so reordered rules
    // never clone bodies merely to establish identity.
    for index in 0..old_program.rules.len().min(new_program.rules.len()) {
        if old_program.rules[index] == new_program.rules[index] {
            let rule = GroundedRuleId::new(index);
            old_to_new_rule[index] = Some(rule);
            matched_new_rule[index] = true;
        }
    }

    let mut new_by_rule = BTreeMap::<&GroundedRule, VecDeque<GroundedRuleId>>::new();
    for (index, rule) in new_program.rules.iter().enumerate() {
        if !matched_new_rule[index] {
            new_by_rule
                .entry(rule)
                .or_default()
                .push_back(GroundedRuleId::new(index));
        }
    }
    for (index, rule) in old_program.rules.iter().enumerate() {
        if old_to_new_rule[index].is_some() {
            continue;
        }
        let Some(new_id) = new_by_rule.get_mut(rule).and_then(VecDeque::pop_front) else {
            continue;
        };
        old_to_new_rule[index] = Some(new_id);
        matched_new_rule[new_id.index()] = true;
    }
    (old_to_new_rule, matched_new_rule)
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
    let mut certificate = old.clone();
    let mut witness_index = GroundedWitnessIndex::compile(program, old);
    let stats = local_recompute_after_deletion_sparse_indexed(
        program,
        index,
        &mut certificate,
        &mut witness_index,
        direct,
    );
    (certificate, stats)
}
