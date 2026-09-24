use std::collections::{BTreeMap, BTreeSet};

/// Semantic/physical capability supplied by a reconstructible candidate.
///
/// The enum is intentionally family-neutral: a legacy index, a SAMF overlay,
/// or a future backend may supply the same capability without becoming a new
/// semantic ontology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhysicalCapability {
    PointLookup,
    ExactCardinality,
    QuotientFiber,
    ObservableFiber,
    Annotation,
    OrderedCut,
}

/// Deterministic policy work estimate. These are planning units, never
/// correctness authority or wall-clock/RSS claims.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicalWorkEstimate {
    pub read_work_saved: u128,
    pub maintenance_work: u128,
    pub build_work: u128,
}

impl PhysicalWorkEstimate {
    #[must_use]
    pub const fn total_cost(self) -> u128 {
        self.maintenance_work.saturating_add(self.build_work)
    }

    #[must_use]
    pub const fn net_benefit(self) -> u128 {
        self.read_work_saved.saturating_sub(self.total_cost())
    }

    #[must_use]
    pub const fn profitable_above(self, threshold: u128) -> bool {
        self.read_work_saved > self.total_cost().saturating_add(threshold)
    }
}

/// Weighted union of shared reconstructible resource atoms.
///
/// Two candidates may name the same resource atom. Union cost counts that atom
/// once, which is the production hook needed by the Γ-SRE shared-resource
/// model. Current legacy adapters use unique atoms, preserving Pass80 byte
/// accounting until SAMF overlays begin sharing backing structures explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceFootprint<R: Ord> {
    atoms: BTreeMap<R, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceFootprintError<R> {
    InconsistentAtomWeight {
        atom: R,
        existing_bytes: usize,
        incoming_bytes: usize,
    },
}

impl<R: Ord> Default for ResourceFootprint<R> {
    fn default() -> Self {
        Self {
            atoms: BTreeMap::new(),
        }
    }
}

impl<R: Ord> ResourceFootprint<R> {
    #[must_use]
    pub fn from_atom(atom: R, estimated_bytes: usize) -> Self {
        Self {
            atoms: BTreeMap::from([(atom, estimated_bytes)]),
        }
    }

    #[must_use]
    pub fn estimated_bytes(&self) -> usize {
        self.atoms
            .values()
            .copied()
            .fold(0_usize, usize::saturating_add)
    }

    pub fn marginal_bytes_against(
        &self,
        retained: &Self,
    ) -> Result<usize, ResourceFootprintError<R>>
    where
        R: Clone,
    {
        let mut total = 0_usize;
        for (atom, bytes) in &self.atoms {
            match retained.atoms.get(atom) {
                Some(existing) if existing != bytes => {
                    return Err(ResourceFootprintError::InconsistentAtomWeight {
                        atom: atom.clone(),
                        existing_bytes: *existing,
                        incoming_bytes: *bytes,
                    });
                }
                Some(_) => {}
                None => total = total.saturating_add(*bytes),
            }
        }
        Ok(total)
    }

    pub fn union_in_place(&mut self, other: &Self) -> Result<(), ResourceFootprintError<R>>
    where
        R: Clone,
    {
        for (atom, bytes) in &other.atoms {
            match self.atoms.get(atom) {
                Some(existing) if existing != bytes => {
                    return Err(ResourceFootprintError::InconsistentAtomWeight {
                        atom: atom.clone(),
                        existing_bytes: *existing,
                        incoming_bytes: *bytes,
                    });
                }
                Some(_) => {}
                None => {
                    self.atoms.insert(atom.clone(), *bytes);
                }
            }
        }
        Ok(())
    }
}

/// Host/platform memory observations are deliberately separate from exact
/// kernel-owned resource atoms. RSS and available memory are admission signals,
/// never per-artifact accounting authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicalPressureSample {
    pub process_rss_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicalPressurePolicy {
    pub max_process_rss_bytes: Option<u64>,
    pub min_available_memory_bytes: Option<u64>,
}

impl PhysicalPressurePolicy {
    #[must_use]
    pub fn admits(self, sample: PhysicalPressureSample) -> bool {
        let rss_ok = match (self.max_process_rss_bytes, sample.process_rss_bytes) {
            (Some(limit), Some(rss)) => rss <= limit,
            _ => true,
        };
        let available_ok = match (
            self.min_available_memory_bytes,
            sample.available_memory_bytes,
        ) {
            (Some(minimum), Some(available)) => available >= minimum,
            _ => true,
        };
        rss_ok && available_ok
    }
}

/// Deterministic reconstructible workload observations for one physical artifact.
///
/// Values are abstract work units rather than elapsed time. They may influence only
/// reconstructible physical choices; losing them on restart cannot change logical state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtifactTelemetry {
    pub read_work_saved: u128,
    pub maintenance_work: u128,
    pub rebuild_work: u128,
}

impl ArtifactTelemetry {
    #[must_use]
    pub const fn apply_to(
        self,
        baseline: PhysicalWorkEstimate,
        already_present: bool,
    ) -> PhysicalWorkEstimate {
        PhysicalWorkEstimate {
            read_work_saved: baseline
                .read_work_saved
                .saturating_add(self.read_work_saved),
            maintenance_work: baseline
                .maintenance_work
                .saturating_add(self.maintenance_work),
            build_work: if already_present || baseline.build_work > self.rebuild_work {
                baseline.build_work
            } else {
                self.rebuild_work
            },
        }
    }
}

/// Epoch-based decay. Advancing an epoch once is independent of observation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryDecayPolicy {
    /// Retain `value - value / 2^shift`. `shift=0` forgets the previous epoch.
    pub shift: u8,
}

impl Default for TelemetryDecayPolicy {
    fn default() -> Self {
        Self { shift: 3 }
    }
}

impl TelemetryDecayPolicy {
    #[must_use]
    const fn decay(self, value: u128) -> u128 {
        if self.shift == 0 {
            0
        } else if self.shift >= 127 {
            value
        } else {
            value.saturating_sub(value >> self.shift)
        }
    }
}

/// Non-authoritative, reconstructible telemetry keyed by the production artifact identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorTelemetry<K: Ord> {
    entries: BTreeMap<K, ArtifactTelemetry>,
}

impl<K: Ord> Default for AdvisorTelemetry<K> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<K: Clone + Ord> AdvisorTelemetry<K> {
    pub fn observe(&mut self, key: K, sample: ArtifactTelemetry) {
        let current = self.entries.entry(key).or_default();
        current.read_work_saved = current
            .read_work_saved
            .saturating_add(sample.read_work_saved);
        current.maintenance_work = current
            .maintenance_work
            .saturating_add(sample.maintenance_work);
        current.rebuild_work = current.rebuild_work.saturating_add(sample.rebuild_work);
    }

    #[must_use]
    pub fn get(&self, key: &K) -> ArtifactTelemetry {
        self.entries.get(key).copied().unwrap_or_default()
    }

    /// Decays exactly once per maintenance epoch, not once per observation.
    pub fn advance_epoch(&mut self, policy: TelemetryDecayPolicy) {
        for current in self.entries.values_mut() {
            current.read_work_saved = policy.decay(current.read_work_saved);
            current.maintenance_work = policy.decay(current.maintenance_work);
            current.rebuild_work = policy.decay(current.rebuild_work);
        }
    }
}

/// Cross-family policy knobs. Hysteresis is explicit because immediate
/// build/evict at the same utility threshold can thrash around break-even.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedAdvisorPolicy {
    pub max_managed_units: usize,
    pub max_managed_estimated_bytes: usize,
    pub max_total_estimated_bytes: usize,
    pub build_threshold: u128,
    pub retain_threshold: u128,
}

impl Default for UnifiedAdvisorPolicy {
    fn default() -> Self {
        Self {
            max_managed_units: usize::MAX,
            max_managed_estimated_bytes: usize::MAX,
            max_total_estimated_bytes: usize::MAX,
            build_threshold: 0,
            retain_threshold: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AdmissionCandidate<K: Ord, R: Ord> {
    pub key: K,
    pub capabilities: BTreeSet<PhysicalCapability>,
    pub work: PhysicalWorkEstimate,
    pub managed_units: usize,
    pub footprint: ResourceFootprint<R>,
    pub replaced_fixed_bytes: usize,
    pub existing_manual: bool,
    pub existing_advisor_managed: bool,
    pub tie_break_work: usize,
}

impl<K: Ord, R: Ord> AdmissionCandidate<K, R> {
    fn threshold(&self, policy: UnifiedAdvisorPolicy) -> u128 {
        if self.existing_advisor_managed || self.existing_manual {
            policy.retain_threshold
        } else {
            policy.build_threshold
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdmissionSelection<K: Ord> {
    pub selected: BTreeSet<K>,
    pub rejected_unprofitable: Vec<K>,
    pub rejected_budget: Vec<K>,
    pub rejected_pressure: Vec<K>,
    pub rejected_resource_conflict: Vec<K>,
    pub managed_units: usize,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

impl<K: Ord> Default for AdmissionSelection<K> {
    fn default() -> Self {
        Self {
            selected: BTreeSet::new(),
            rejected_unprofitable: Vec::new(),
            rejected_budget: Vec::new(),
            rejected_pressure: Vec::new(),
            rejected_resource_conflict: Vec::new(),
            managed_units: 0,
            managed_estimated_bytes: 0,
            fixed_estimated_bytes: 0,
            total_estimated_bytes_after: 0,
        }
    }
}

fn compare_candidates<K: Ord, R: Ord>(
    left: &AdmissionCandidate<K, R>,
    right: &AdmissionCandidate<K, R>,
) -> std::cmp::Ordering {
    let left_cost = if left.existing_manual {
        0
    } else {
        left.footprint.estimated_bytes()
    };
    let right_cost = if right.existing_manual {
        0
    } else {
        right.footprint.estimated_bytes()
    };
    let left_benefit = left.work.net_benefit();
    let right_benefit = right.work.net_benefit();
    match (left_cost, right_cost) {
        (0, 0) => right_benefit
            .cmp(&left_benefit)
            .then_with(|| left.key.cmp(&right.key)),
        (0, _) => std::cmp::Ordering::Less,
        (_, 0) => std::cmp::Ordering::Greater,
        _ => {
            let left_density = left_benefit.saturating_mul(right_cost as u128);
            let right_density = right_benefit.saturating_mul(left_cost as u128);
            right_density
                .cmp(&left_density)
                .then_with(|| right_benefit.cmp(&left_benefit))
                .then_with(|| right.tie_break_work.cmp(&left.tie_break_work))
                .then_with(|| left.key.cmp(&right.key))
        }
    }
}

/// Family-neutral hard-budget selector. Correctness is independent of this
/// choice: all candidates are reconstructible derivatives.
pub(crate) fn select_candidates<K, R>(
    candidates: Vec<AdmissionCandidate<K, R>>,
    fixed_estimated_bytes: usize,
    policy: UnifiedAdvisorPolicy,
) -> AdmissionSelection<K>
where
    K: Clone + Ord,
    R: Clone + Ord,
{
    select_candidates_with_pressure(
        candidates,
        fixed_estimated_bytes,
        policy,
        PhysicalPressurePolicy::default(),
        PhysicalPressureSample::default(),
    )
}

pub(crate) fn select_candidates_with_pressure<K, R>(
    mut candidates: Vec<AdmissionCandidate<K, R>>,
    fixed_estimated_bytes: usize,
    policy: UnifiedAdvisorPolicy,
    pressure_policy: PhysicalPressurePolicy,
    pressure_sample: PhysicalPressureSample,
) -> AdmissionSelection<K>
where
    K: Clone + Ord,
    R: Clone + Ord,
{
    candidates.sort_by(compare_candidates);
    let mut result = AdmissionSelection {
        fixed_estimated_bytes,
        ..AdmissionSelection::default()
    };
    let mut retained_resources = ResourceFootprint::<R>::default();

    for candidate in candidates {
        if candidate.capabilities.is_empty()
            || !candidate.work.profitable_above(candidate.threshold(policy))
        {
            result.rejected_unprofitable.push(candidate.key);
            continue;
        }
        if !candidate.existing_manual && !pressure_policy.admits(pressure_sample) {
            result.rejected_pressure.push(candidate.key);
            continue;
        }
        let managed_units = if candidate.existing_manual {
            0
        } else {
            candidate.managed_units
        };
        let marginal_bytes = if candidate.existing_manual {
            0
        } else if let Ok(bytes) = candidate
            .footprint
            .marginal_bytes_against(&retained_resources)
        {
            bytes
        } else {
            result.rejected_resource_conflict.push(candidate.key);
            continue;
        };
        let projected_fixed = result
            .fixed_estimated_bytes
            .saturating_sub(candidate.replaced_fixed_bytes);
        if result.managed_units.saturating_add(managed_units) > policy.max_managed_units
            || result
                .managed_estimated_bytes
                .saturating_add(marginal_bytes)
                > policy.max_managed_estimated_bytes
            || projected_fixed
                .saturating_add(result.managed_estimated_bytes)
                .saturating_add(marginal_bytes)
                > policy.max_total_estimated_bytes
        {
            result.rejected_budget.push(candidate.key);
            continue;
        }
        result.fixed_estimated_bytes = projected_fixed;
        result.managed_units = result.managed_units.saturating_add(managed_units);
        result.managed_estimated_bytes = result
            .managed_estimated_bytes
            .saturating_add(marginal_bytes);
        if !candidate.existing_manual
            && retained_resources
                .union_in_place(&candidate.footprint)
                .is_err()
        {
            result.rejected_resource_conflict.push(candidate.key);
            continue;
        }
        result.selected.insert(candidate.key);
    }
    result.total_estimated_bytes_after = result
        .fixed_estimated_bytes
        .saturating_add(result.managed_estimated_bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        key: u8,
        resource: u8,
        bytes: usize,
        read: u128,
        maintenance: u128,
        build: u128,
        managed: bool,
    ) -> AdmissionCandidate<u8, u8> {
        AdmissionCandidate {
            key,
            capabilities: BTreeSet::from([PhysicalCapability::PointLookup]),
            work: PhysicalWorkEstimate {
                read_work_saved: read,
                maintenance_work: maintenance,
                build_work: build,
            },
            managed_units: 1,
            footprint: ResourceFootprint::from_atom(resource, bytes),
            replaced_fixed_bytes: 0,
            existing_manual: false,
            existing_advisor_managed: managed,
            tie_break_work: 0,
        }
    }

    #[test]
    fn resource_footprint_counts_shared_atom_once() {
        let mut retained = ResourceFootprint::from_atom(7_u8, 100);
        let same = ResourceFootprint::from_atom(7_u8, 100);
        let larger = ResourceFootprint::from_atom(7_u8, 140);
        assert_eq!(same.marginal_bytes_against(&retained), Ok(0));
        assert!(matches!(
            larger.marginal_bytes_against(&retained),
            Err(ResourceFootprintError::InconsistentAtomWeight {
                atom: 7,
                existing_bytes: 100,
                incoming_bytes: 140,
            })
        ));
        assert!(retained.union_in_place(&larger).is_err());
        assert_eq!(retained.estimated_bytes(), 100);
    }

    #[test]
    fn write_maintenance_can_make_read_profitable_candidate_unprofitable() {
        let selected = select_candidates(
            vec![candidate(1, 1, 10, 100, 95, 10, false)],
            0,
            UnifiedAdvisorPolicy::default(),
        );
        assert!(selected.selected.is_empty());
        assert_eq!(selected.rejected_unprofitable, vec![1]);
    }

    #[test]
    fn hysteresis_retains_existing_candidate_below_build_threshold() {
        let policy = UnifiedAdvisorPolicy {
            build_threshold: 20,
            retain_threshold: 5,
            ..UnifiedAdvisorPolicy::default()
        };
        let new_selection =
            select_candidates(vec![candidate(1, 1, 10, 115, 0, 100, false)], 0, policy);
        assert!(new_selection.selected.is_empty());
        let retained_selection =
            select_candidates(vec![candidate(1, 1, 10, 115, 0, 100, true)], 0, policy);
        assert_eq!(retained_selection.selected, BTreeSet::from([1]));
    }

    #[test]
    fn shared_resource_budget_uses_marginal_union_cost() {
        let selected = select_candidates(
            vec![
                candidate(1, 9, 100, 500, 0, 1, false),
                candidate(2, 9, 100, 400, 0, 1, false),
            ],
            0,
            UnifiedAdvisorPolicy {
                max_managed_units: 2,
                max_managed_estimated_bytes: 100,
                max_total_estimated_bytes: 100,
                ..UnifiedAdvisorPolicy::default()
            },
        );
        assert_eq!(selected.selected, BTreeSet::from([1, 2]));
        assert_eq!(selected.managed_estimated_bytes, 100);
    }

    #[test]
    fn external_pressure_blocks_optional_builds_but_not_manual_state() {
        let mut manual = candidate(1, 1, 100, 500, 0, 1, false);
        manual.existing_manual = true;
        let optional = candidate(2, 2, 100, 500, 0, 1, false);
        let selected = select_candidates_with_pressure(
            vec![manual, optional],
            0,
            UnifiedAdvisorPolicy::default(),
            PhysicalPressurePolicy {
                max_process_rss_bytes: Some(1_000),
                min_available_memory_bytes: Some(500),
            },
            PhysicalPressureSample {
                process_rss_bytes: Some(2_000),
                available_memory_bytes: Some(100),
            },
        );
        assert_eq!(selected.selected, BTreeSet::from([1]));
        assert_eq!(selected.rejected_pressure, vec![2]);
    }

    #[test]
    fn telemetry_is_order_independent_within_epoch_and_decays_once() {
        let mut left = AdvisorTelemetry::<u8>::default();
        let mut right = AdvisorTelemetry::<u8>::default();
        let a = ArtifactTelemetry {
            read_work_saved: 80,
            maintenance_work: 20,
            rebuild_work: 8,
        };
        let b = ArtifactTelemetry {
            read_work_saved: 40,
            maintenance_work: 4,
            rebuild_work: 4,
        };
        left.observe(1, a);
        left.observe(1, b);
        right.observe(1, b);
        right.observe(1, a);
        assert_eq!(left, right);
        left.advance_epoch(TelemetryDecayPolicy { shift: 1 });
        right.advance_epoch(TelemetryDecayPolicy { shift: 1 });
        assert_eq!(left, right);
        assert_eq!(
            left.get(&1),
            ArtifactTelemetry {
                read_work_saved: 60,
                maintenance_work: 12,
                rebuild_work: 6,
            }
        );
    }

    #[test]
    fn telemetry_adds_write_cost_and_reuses_existing_without_rebuild_cost() {
        let sample = ArtifactTelemetry {
            read_work_saved: 50,
            maintenance_work: 25,
            rebuild_work: 100,
        };
        let baseline = PhysicalWorkEstimate {
            read_work_saved: 10,
            maintenance_work: 5,
            build_work: 20,
        };
        assert_eq!(
            sample.apply_to(baseline, false),
            PhysicalWorkEstimate {
                read_work_saved: 60,
                maintenance_work: 30,
                build_work: 100,
            }
        );
        assert_eq!(sample.apply_to(baseline, true).build_work, 20);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecoveryCandidate<K: Ord, R: Ord, D> {
    pub key: K,
    pub capabilities: BTreeSet<PhysicalCapability>,
    pub work: PhysicalWorkEstimate,
    pub footprint: ResourceFootprint<R>,
    pub manual_pin: bool,
    pub secondary_build_work: usize,
    pub detail: D,
}

/// Recovery uses the same candidate ontology as the online advisor, but with a
/// startup horizon: manual durable intent is first, then advisor candidates are
/// ordered by deterministic rebuild work. Budget admission remains in the
/// recovery loop because retained bytes are measured after reconstructing the
/// concrete backend state.
pub(crate) fn order_recovery_candidates<K, R, D>(
    mut candidates: Vec<RecoveryCandidate<K, R, D>>,
) -> Vec<RecoveryCandidate<K, R, D>>
where
    K: Ord,
    R: Ord,
{
    candidates.sort_by(|left, right| {
        right
            .manual_pin
            .cmp(&left.manual_pin)
            .then_with(|| {
                if left.manual_pin || right.manual_pin {
                    return std::cmp::Ordering::Equal;
                }
                let left_denominator = left.work.build_work.max(1);
                let right_denominator = right.work.build_work.max(1);
                let left_density = left.work.read_work_saved.saturating_mul(right_denominator);
                let right_density = right.work.read_work_saved.saturating_mul(left_denominator);
                right_density.cmp(&left_density)
            })
            .then_with(|| left.work.build_work.cmp(&right.work.build_work))
            .then_with(|| left.secondary_build_work.cmp(&right.secondary_build_work))
            .then_with(|| {
                left.footprint
                    .estimated_bytes()
                    .cmp(&right.footprint.estimated_bytes())
            })
            .then_with(|| left.capabilities.len().cmp(&right.capabilities.len()))
            .then_with(|| left.key.cmp(&right.key))
    });
    candidates
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn recovery(key: u8, read: u128, build: u128, manual: bool) -> RecoveryCandidate<u8, u8, ()> {
        RecoveryCandidate {
            key,
            capabilities: BTreeSet::from([PhysicalCapability::PointLookup]),
            work: PhysicalWorkEstimate {
                read_work_saved: read,
                maintenance_work: 0,
                build_work: build,
            },
            footprint: ResourceFootprint::from_atom(key, 1),
            manual_pin: manual,
            secondary_build_work: 0,
            detail: (),
        }
    }

    #[test]
    fn recovery_keeps_manual_first_then_uses_benefit_density() {
        let ordered = order_recovery_candidates(vec![
            recovery(1, 0, 1, false),
            recovery(2, 1_000, 100, false),
            recovery(3, 0, 1_000, true),
        ]);
        assert_eq!(
            ordered.into_iter().map(|item| item.key).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }
}
