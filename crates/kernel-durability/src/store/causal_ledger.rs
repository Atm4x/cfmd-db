use std::collections::{BTreeMap, BTreeSet};

use kernel_change::{RevisionEffect, RevisionEffectId, RevisionEffectIdeal};
use kernel_revision::Revision;
use kernel_types::RevisionId;

use crate::replication::authority::ReplicationAuthorityJournal;

use super::DurableRevisionStore;
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{DurableRevisionEffectRecord, DurableTransactionIntent};
use crate::replication::ReplicationBranchId;
use crate::runtime::{DurabilityError, RecoveryScan};

pub(super) fn allocate_local_revision_effect_id(
    next: &mut u128,
) -> Result<RevisionEffectId, DurabilityError> {
    if *next > u128::from(u64::MAX) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local revision effect namespace is exhausted",
        });
    }
    let id = RevisionEffectId(*next);
    *next = next.checked_add(1).ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "revision effect identity space is exhausted",
    })?;
    Ok(id)
}

type RecoveredRevisionEffectState = (
    RevisionId,
    BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
);

fn append_revision_effect(
    effects: &mut BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &mut BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    record: DurableRevisionEffectRecord,
) -> Result<(), DurabilityError> {
    record
        .validate_identity()
        .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
    if record
        .prerequisites
        .iter()
        .any(|prerequisite| !effects.contains_key(prerequisite))
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "revision effect prerequisite is missing from durable causal ledger",
        });
    }
    if let Some(existing) = effects.get(&record.id) {
        if existing == &record {
            return Ok(());
        }
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "revision effect identity conflicts with durable causal ledger",
        });
    }
    if frontiers.contains_key(&record.target_revision) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "target revision already has a durable causal frontier",
        });
    }
    let target_revision = record.target_revision;
    let id = record.id;
    effects.insert(id, record);
    frontiers.insert(target_revision, BTreeSet::from([id]));
    Ok(())
}

fn causal_prerequisites_with_lookup(
    descriptor: &DurableRevisionDescriptor,
    mut frontier: impl FnMut(RevisionId) -> Option<BTreeSet<RevisionEffectId>>,
) -> Result<BTreeSet<RevisionEffectId>, DurabilityError> {
    match &descriptor.intent {
        DurableTransactionIntent::RelationResolution { causal_parents, .. } => {
            if causal_parents.len() < 2
                || causal_parents.windows(2).any(|pair| pair[0] >= pair[1])
                || causal_parents
                    .binary_search(&descriptor.source_revision)
                    .is_err()
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "resolution causal parents are not canonical",
                });
            }
            let mut prerequisites = BTreeSet::new();
            for parent in causal_parents {
                let Some(parent_frontier) = frontier(*parent) else {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "resolution causal parent is outside durable causal coverage",
                    });
                };
                prerequisites.extend(parent_frontier);
            }
            Ok(prerequisites)
        }
        _ => frontier(descriptor.source_revision).ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "committed revision source is outside durable causal coverage",
        }),
    }
}

pub(super) fn causal_prerequisites_for_descriptor(
    frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    descriptor: &DurableRevisionDescriptor,
) -> Result<BTreeSet<RevisionEffectId>, DurabilityError> {
    causal_prerequisites_with_lookup(descriptor, |revision| frontiers.get(&revision).cloned())
}

pub(super) struct CausalCommitOverlay<'a> {
    base_effects: &'a BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    base_frontiers: &'a BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
}

impl<'a> CausalCommitOverlay<'a> {
    pub(super) fn new(
        effects: &'a BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
        frontiers: &'a BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    ) -> Self {
        Self {
            base_effects: effects,
            base_frontiers: frontiers,
            effects: BTreeMap::new(),
            frontiers: BTreeMap::new(),
        }
    }

    pub(super) fn prepare_append(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<(), DurabilityError> {
        let prerequisites = causal_prerequisites_with_lookup(descriptor, |revision| {
            self.frontiers
                .get(&revision)
                .or_else(|| self.base_frontiers.get(&revision))
                .cloned()
        })?;
        let record = DurableRevisionEffectRecord {
            id: descriptor
                .revision_effect_id
                .unwrap_or(RevisionEffectId(descriptor.transaction_id.raw())),
            prerequisites,
            transaction_epoch: descriptor.idempotency_epoch,
            transaction_id: descriptor.transaction_id,
            intent: descriptor.intent.clone(),
            change: descriptor.change.clone(),
            source_revision: descriptor.source_revision,
            target_revision: descriptor.target_revision,
        };
        record
            .validate_identity()
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
        if record.prerequisites.iter().any(|prerequisite| {
            !self.effects.contains_key(prerequisite)
                && !self.base_effects.contains_key(prerequisite)
        }) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "revision effect prerequisite is missing from durable causal ledger",
            });
        }
        if let Some(existing) = self
            .effects
            .get(&record.id)
            .or_else(|| self.base_effects.get(&record.id))
        {
            if existing == &record {
                return Ok(());
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "revision effect identity conflicts with durable causal ledger",
            });
        }
        if self.frontiers.contains_key(&record.target_revision)
            || self.base_frontiers.contains_key(&record.target_revision)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "target revision already has a durable causal frontier",
            });
        }
        let target_revision = record.target_revision;
        let id = record.id;
        self.effects.insert(id, record);
        self.frontiers.insert(target_revision, BTreeSet::from([id]));
        Ok(())
    }

    pub(super) fn into_effects(self) -> Vec<DurableRevisionEffectRecord> {
        self.effects.into_values().collect()
    }
}

pub(super) fn publish_prepared_revision_effects(
    effects: &mut BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &mut BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    prepared: Vec<DurableRevisionEffectRecord>,
) {
    for record in prepared {
        let id = record.id;
        let target = record.target_revision;
        debug_assert!(!effects.contains_key(&id));
        debug_assert!(!frontiers.contains_key(&target));
        effects.insert(id, record);
        frontiers.insert(target, BTreeSet::from([id]));
    }
}

fn causal_frontier_for_revision<'a>(
    local_frontiers: &'a BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    replication: &'a ReplicationAuthorityJournal,
    revision: RevisionId,
) -> Option<&'a BTreeSet<RevisionEffectId>> {
    local_frontiers
        .get(&revision)
        .or_else(|| replication.revision_frontier(revision))
}

pub(super) fn causal_prerequisites_for_replicated_effect(
    local_frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    replication: &ReplicationAuthorityJournal,
    record: &DurableRevisionEffectRecord,
) -> Result<BTreeSet<RevisionEffectId>, DurabilityError> {
    match &record.intent {
        DurableTransactionIntent::RelationResolution { causal_parents, .. } => {
            if causal_parents.len() < 2
                || causal_parents.windows(2).any(|pair| pair[0] >= pair[1])
                || causal_parents
                    .binary_search(&record.source_revision)
                    .is_err()
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replicated resolution causal parents are not canonical",
                });
            }
            let mut prerequisites = BTreeSet::new();
            for parent in causal_parents {
                let Some(frontier) =
                    causal_frontier_for_revision(local_frontiers, replication, *parent)
                else {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replicated resolution parent is outside durable causal coverage",
                    });
                };
                prerequisites.extend(frontier.iter().copied());
            }
            Ok(prerequisites)
        }
        _ => causal_frontier_for_revision(local_frontiers, replication, record.source_revision)
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect source is outside durable causal coverage",
            }),
    }
}

fn append_committed_revision_effect(
    effects: &mut BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &mut BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    descriptor: &DurableRevisionDescriptor,
) -> Result<(), DurabilityError> {
    let prerequisites = causal_prerequisites_for_descriptor(frontiers, descriptor)?;
    append_revision_effect(
        effects,
        frontiers,
        DurableRevisionEffectRecord {
            id: descriptor
                .revision_effect_id
                .unwrap_or(RevisionEffectId(descriptor.transaction_id.raw())),
            prerequisites,
            transaction_epoch: descriptor.idempotency_epoch,
            transaction_id: descriptor.transaction_id,
            intent: descriptor.intent.clone(),
            change: descriptor.change.clone(),
            source_revision: descriptor.source_revision,
            target_revision: descriptor.target_revision,
        },
    )
}

pub(super) fn recover_revision_effect_state(
    coverage_root: Option<RevisionId>,
    mut effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    mut frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    scan: &RecoveryScan,
) -> Result<RecoveredRevisionEffectState, DurabilityError> {
    let Some(coverage_root) = coverage_root else {
        let root = scan.durable_revision();
        effects.clear();
        frontiers.clear();
        frontiers.insert(root, BTreeSet::new());
        return Ok((root, effects, frontiers));
    };
    if !frontiers.contains_key(&coverage_root) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal coverage root has no frontier",
        });
    }
    for committed in scan.committed() {
        append_committed_revision_effect(&mut effects, &mut frontiers, &committed.descriptor)?;
    }
    Ok((coverage_root, effects, frontiers))
}

pub(super) fn validate_revision_effect_state(
    coverage_root: RevisionId,
    effects: &BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
) -> Result<(), DurabilityError> {
    if frontiers.get(&coverage_root) != Some(&BTreeSet::new()) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal coverage root is not an empty frontier",
        });
    }
    for (&id, effect) in effects {
        effect
            .validate_identity()
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
        if id != effect.id {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal ledger map key disagrees with effect identity",
            });
        }
        let intent = &effect.intent;
        if intent.target_revision() != effect.target_revision {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect target disagrees with canonical effect intent",
            });
        }
        let expected_prerequisites = match intent {
            DurableTransactionIntent::RelationResolution { causal_parents, .. } => {
                let mut cut = BTreeSet::new();
                for parent in causal_parents {
                    let Some(frontier) = frontiers.get(parent) else {
                        return Err(DurabilityError::Protocol {
                            offset: 0,
                            reason: "durable resolution parent frontier is missing",
                        });
                    };
                    cut.extend(frontier.iter().copied());
                }
                cut
            }
            _ => frontiers.get(&effect.source_revision).cloned().ok_or(
                DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable causal effect source frontier is missing",
                },
            )?,
        };
        if expected_prerequisites != effect.prerequisites {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect prerequisites disagree with exact causal cut",
            });
        }
        if frontiers.get(&effect.target_revision) != Some(&BTreeSet::from([id])) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect target frontier is not canonical",
            });
        }
    }
    if frontiers
        .values()
        .flatten()
        .any(|effect| !effects.contains_key(effect))
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal frontier references a missing effect",
        });
    }
    Ok(())
}

impl DurableRevisionStore {
    /// Irreversibly expires the locally retained causal prefix at the current
    /// durable head and publishes that new coverage root in a checkpoint.
    ///
    /// This is the causal-history analogue of retry-history expiry: semantic
    /// state is unchanged, but revisions older than the new root are no longer
    /// available for causal replay/rebase. Retained migration history and
    /// unresolved prepares are explicit blockers rather than hidden fallbacks.
    pub fn release_causal_history_before_head(
        &mut self,
        revision: &Revision,
    ) -> Result<Option<super::DurableGenerationReceipt>, DurabilityError> {
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "causal-history release revision does not match durable head",
            });
        }
        if !self.prepared_transactions.is_empty() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "causal-history release is blocked by unresolved prepared transactions",
            });
        }
        if !self.historical_epoch_anchors.is_empty() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "causal-history release is blocked by retained historical epoch authority",
            });
        }
        let local_effects = self
            .revision_effects
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if self.replication.effects_iter().any(|(_, envelope)| {
            envelope
                .effect
                .prerequisites
                .iter()
                .any(|id| local_effects.contains(id))
        }) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "causal-history release is blocked by replicated causal prerequisites",
            });
        }
        if self.causal_coverage_root == self.durable_head && self.revision_effects.is_empty() {
            return Ok(None);
        }

        let previous_root = self.causal_coverage_root;
        let previous_effects = std::mem::take(&mut self.revision_effects);
        let previous_frontiers = std::mem::take(&mut self.revision_effect_frontiers);
        self.causal_coverage_root = self.durable_head;
        self.revision_effect_frontiers
            .insert(self.durable_head, BTreeSet::new());

        let result = self.rotate_checkpoint_current_specs_with_hook(
            revision,
            &mut super::publication_protocol::NoStoreFault,
        );
        if result.is_err() && !self.poisoned {
            self.causal_coverage_root = previous_root;
            self.revision_effects = previous_effects;
            self.revision_effect_frontiers = previous_frontiers;
        }
        result.map(Some)
    }

    /// Returns the exact primary revision-transition chain from `target` back
    /// to `source`, newest transition first.
    ///
    /// This is a projection of the authoritative causal ledger, not a second
    /// history index. Every committed target revision owns exactly one local
    /// transition effect; following that effect's `source_revision` therefore
    /// recovers the same state-transition lineage without materializing or
    /// topologically validating the complete causal ideal at each endpoint.
    pub fn revision_transition_records_back_to(
        &self,
        source: RevisionId,
        target: RevisionId,
    ) -> Result<Option<Vec<DurableRevisionEffectRecord>>, DurabilityError> {
        if source == target {
            return Ok(Some(Vec::new()));
        }
        let mut cursor = target;
        let mut seen = BTreeSet::new();
        let mut records = Vec::new();
        while cursor != source {
            if !seen.insert(cursor) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision transition lineage is cyclic",
                });
            }
            let Some(frontier) = self.revision_effect_frontiers.get(&cursor) else {
                return Ok(None);
            };
            if frontier.len() != 1 {
                return Ok(None);
            }
            let id = *frontier.first().expect("singleton frontier");
            let record = self
                .revision_effects
                .get(&id)
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision frontier references a missing effect",
                })?;
            if record.target_revision != cursor {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision frontier targets the wrong revision",
                });
            }
            records.push(record.clone());
            cursor = record.source_revision;
        }
        Ok(Some(records))
    }

    #[must_use]
    pub fn revision_effect_record(
        &self,
        id: RevisionEffectId,
    ) -> Option<&DurableRevisionEffectRecord> {
        self.revision_effects.get(&id)
    }

    #[must_use]
    pub fn revision_effect_frontier(
        &self,
        revision: RevisionId,
    ) -> Option<&BTreeSet<RevisionEffectId>> {
        self.revision_effect_frontiers.get(&revision)
    }

    pub fn revision_effect_ideal(
        &self,
        revision: RevisionId,
    ) -> Result<Option<RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError> {
        let Some(frontier) = self.revision_effect_frontiers.get(&revision) else {
            return Ok(None);
        };
        let mut pending = frontier.iter().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let mut events = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let record = self
                .revision_effects
                .get(&id)
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision frontier references a missing effect",
                })?;
            let payload = record.intent.clone();
            pending.extend(record.prerequisites.iter().copied());
            events.push(RevisionEffect {
                id,
                prerequisites: record.prerequisites.clone(),
                payload,
            });
        }
        RevisionEffectIdeal::new(events)
            .map(Some)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "durable revision effect ledger is not a valid causal ideal",
            })
    }

    /// Returns the exact durable effect records in one revision's causal ideal.
    ///
    /// This is a projection over the existing causal ledger, not a second history
    /// authority. Every returned record has already passed the same ideal closure
    /// validation used by `revision_effect_ideal`.
    pub fn revision_effect_records_for_revision(
        &self,
        revision: RevisionId,
    ) -> Result<Option<Vec<DurableRevisionEffectRecord>>, DurabilityError> {
        let Some(ideal) = self.revision_effect_ideal(revision)? else {
            return Ok(None);
        };
        let mut records = Vec::with_capacity(ideal.events().len());
        for id in ideal.events().keys() {
            let record = self
                .revision_effects
                .get(id)
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision ideal references a missing effect record",
                })?;
            records.push(record.clone());
        }
        Ok(Some(records))
    }

    pub fn replicated_branch_effect_ideal(
        &self,
        branch: ReplicationBranchId,
    ) -> Result<Option<RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError> {
        let Some(head) = self.replication.branch_head(branch) else {
            return Ok(None);
        };
        let mut pending = vec![head.head_effect];
        let mut seen = BTreeSet::new();
        let mut events = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let record = self
                .revision_effects
                .get(&id)
                .or_else(|| self.replication.effect(id))
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replicated branch ideal references a missing causal effect",
                })?;
            pending.extend(record.prerequisites.iter().copied());
            events.push(RevisionEffect {
                id,
                prerequisites: record.prerequisites.clone(),
                payload: record.intent.clone(),
            });
        }
        RevisionEffectIdeal::new(events)
            .map(Some)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "replicated branch effect ledger is not a valid causal ideal",
            })
    }
}
