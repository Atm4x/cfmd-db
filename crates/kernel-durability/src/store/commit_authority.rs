use std::collections::BTreeMap;

use kernel_types::RevisionId;

use super::DurableRevisionStore;
use super::causal_ledger::{CausalCommitOverlay, publish_prepared_revision_effects};
use super::migration_history::{MigrationCommitOverlay, publish_prepared_migration_complements};
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{
    DurableCommittedTransaction, DurableMigrationComplement, DurableRevisionEffectRecord,
    DurableTransactionIntent, DurableTransactionKey, HistoricalEpochAnchor,
};
use crate::runtime::DurabilityError;

#[derive(Debug)]
pub(super) struct PreparedCommitAuthority {
    durable_head: RevisionId,
    migration_appends: Vec<DurableMigrationComplement>,
    historical_anchor_appends: Vec<HistoricalEpochAnchor>,
    retry_appends: Vec<(DurableTransactionKey, DurableCommittedTransaction)>,
    revision_effect_appends: Vec<DurableRevisionEffectRecord>,
}

impl PreparedCommitAuthority {
    pub(super) fn prepare(
        store: &DurableRevisionStore,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<Self, DurabilityError> {
        let mut migrations = MigrationCommitOverlay::new(
            &store.migration_complements,
            &store.migration_complement_index,
            store.checkpoint.semantic_revision().schema,
        );
        let mut causal =
            CausalCommitOverlay::new(&store.revision_effects, &store.revision_effect_frontiers);
        let mut retry_overlay = BTreeMap::new();
        let mut durable_head = store.durable_head;
        let mut historical_anchor_appends = Vec::new();

        for descriptor in descriptors {
            if let DurableTransactionIntent::SchemaMigration {
                migration_complement,
                ..
            } = &descriptor.intent
            {
                migrations.prepare_append(migration_complement)?;
                let effect_id =
                    descriptor
                        .revision_effect_id
                        .unwrap_or(kernel_change::RevisionEffectId(
                            descriptor.transaction_id.raw(),
                        ));
                historical_anchor_appends.push(HistoricalEpochAnchor {
                    effect_id,
                    source_revision: descriptor.source_revision,
                    source_schema: migration_complement.source_schema,
                    generation: store.generation,
                });
            }

            let key =
                DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
            if let Some(existing) = store
                .committed_transactions
                .get(&key)
                .or_else(|| retry_overlay.get(&key))
            {
                if !existing.same_client_intent(&descriptor.intent) {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "committed transaction id changed exact intent",
                    });
                }
            } else {
                retry_overlay.insert(
                    key,
                    DurableCommittedTransaction::from_descriptor_intent(
                        descriptor.target_revision,
                        &descriptor.intent,
                    ),
                );
            }

            causal.prepare_append(descriptor)?;
            durable_head = descriptor.target_revision;
        }

        Ok(Self {
            durable_head,
            migration_appends: migrations.into_appends(),
            historical_anchor_appends,
            retry_appends: retry_overlay.into_iter().collect(),
            revision_effect_appends: causal.into_effects(),
        })
    }

    pub(super) fn publish(self, store: &mut DurableRevisionStore) {
        publish_prepared_migration_complements(
            &mut store.migration_complements,
            &mut store.migration_complement_index,
            self.migration_appends,
        );
        for anchor in self.historical_anchor_appends {
            if let Some(existing) = store
                .historical_epoch_anchors
                .insert(anchor.effect_id, anchor)
            {
                debug_assert_eq!(existing, anchor);
            }
        }
        for (key, intent) in self.retry_appends {
            debug_assert!(!store.committed_transactions.contains_key(&key));
            store.committed_transactions.insert(key, intent);
        }
        publish_prepared_revision_effects(
            &mut store.revision_effects,
            &mut store.revision_effect_frontiers,
            self.revision_effect_appends,
        );
        store.durable_head = self.durable_head;
    }
}
