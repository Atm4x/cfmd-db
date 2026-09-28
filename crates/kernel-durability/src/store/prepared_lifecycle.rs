use std::collections::{BTreeMap, BTreeSet};

use kernel_types::RevisionId;

use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::DurableTransactionKey;
use crate::runtime::{DurablePrepareToken, RecoveryScan};

#[derive(Debug, Clone)]
struct PreparedTransactionEntry {
    descriptor: DurableRevisionDescriptor,
    payload_crc32c: u32,
    commit_capable: bool,
}

#[derive(Debug, Default)]
pub(super) struct PreparedTransactionLedger {
    by_lsn: BTreeMap<u64, PreparedTransactionEntry>,
    lsns_by_retry_key: BTreeMap<DurableTransactionKey, BTreeSet<u64>>,
    retry_key_by_target_revision: BTreeMap<RevisionId, DurableTransactionKey>,
}

impl PreparedTransactionLedger {
    pub(super) fn from_recovery_scan(scan: &RecoveryScan) -> Self {
        let mut ledger = Self::default();
        for &(prepare_lsn, ref descriptor, payload_crc32c) in scan.unresolved_prepares() {
            ledger.insert_recovered(prepare_lsn, descriptor.clone(), payload_crc32c);
        }
        ledger
    }

    fn insert_recovered(
        &mut self,
        prepare_lsn: u64,
        descriptor: DurableRevisionDescriptor,
        payload_crc32c: u32,
    ) {
        let key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        if let Some(existing) = self.descriptor_by_retry_key(key) {
            debug_assert_eq!(existing, &descriptor);
        }
        if let Some(existing) = self.descriptor_by_target_revision(descriptor.target_revision) {
            debug_assert_eq!(existing, &descriptor);
        }
        self.lsns_by_retry_key
            .entry(key)
            .or_default()
            .insert(prepare_lsn);
        self.retry_key_by_target_revision
            .entry(descriptor.target_revision)
            .or_insert(key);
        let previous = self.by_lsn.insert(
            prepare_lsn,
            PreparedTransactionEntry {
                descriptor,
                payload_crc32c,
                commit_capable: false,
            },
        );
        debug_assert!(previous.is_none());
    }

    fn descriptor_by_lsn(&self, prepare_lsn: u64) -> Option<&DurableRevisionDescriptor> {
        self.by_lsn.get(&prepare_lsn).map(|entry| &entry.descriptor)
    }

    pub(super) fn commit_descriptor_by_lsn(
        &self,
        prepare_lsn: u64,
    ) -> Option<&DurableRevisionDescriptor> {
        self.by_lsn
            .get(&prepare_lsn)
            .filter(|entry| entry.commit_capable)
            .map(|entry| &entry.descriptor)
    }

    pub(super) fn descriptor_by_retry_key(
        &self,
        key: DurableTransactionKey,
    ) -> Option<&DurableRevisionDescriptor> {
        self.lsns_by_retry_key
            .get(&key)
            .and_then(|lsns| lsns.first())
            .and_then(|lsn| self.descriptor_by_lsn(*lsn))
    }

    pub(super) fn descriptor_by_target_revision(
        &self,
        target_revision: RevisionId,
    ) -> Option<&DurableRevisionDescriptor> {
        self.retry_key_by_target_revision
            .get(&target_revision)
            .and_then(|key| self.descriptor_by_retry_key(*key))
    }

    pub(super) fn publish_prepare(
        &mut self,
        token: DurablePrepareToken,
        descriptor: DurableRevisionDescriptor,
    ) {
        let prepare_lsn = token.prepare_lsn();
        let key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        debug_assert!(!self.by_lsn.contains_key(&prepare_lsn));
        debug_assert!(!self.lsns_by_retry_key.contains_key(&key));
        debug_assert!(
            !self
                .retry_key_by_target_revision
                .contains_key(&descriptor.target_revision)
        );
        self.lsns_by_retry_key
            .entry(key)
            .or_default()
            .insert(prepare_lsn);
        self.retry_key_by_target_revision
            .insert(descriptor.target_revision, key);
        self.by_lsn.insert(
            prepare_lsn,
            PreparedTransactionEntry {
                descriptor,
                payload_crc32c: token.prepare_payload_crc32c(),
                commit_capable: true,
            },
        );
    }

    pub(super) fn retire_committed(&mut self, prepare_lsn: u64) {
        let Some(entry) = self.by_lsn.get(&prepare_lsn) else {
            debug_assert!(false, "committed prepare must exist in lifecycle ledger");
            return;
        };
        let key = DurableTransactionKey::new(
            entry.descriptor.idempotency_epoch,
            entry.descriptor.transaction_id,
        );
        let target_revision = entry.descriptor.target_revision;
        let Some(alias_lsns) = self.lsns_by_retry_key.remove(&key) else {
            debug_assert!(false, "prepared retry owner must exist");
            return;
        };
        for alias_lsn in alias_lsns {
            self.by_lsn.remove(&alias_lsn);
        }
        if self.retry_key_by_target_revision.get(&target_revision) == Some(&key) {
            self.retry_key_by_target_revision.remove(&target_revision);
        }
    }

    pub(super) fn checkpoint_entries(
        &self,
        durable_head: RevisionId,
    ) -> impl Iterator<Item = (u64, &DurableRevisionDescriptor, u32)> {
        self.by_lsn.iter().filter_map(move |(&prepare_lsn, entry)| {
            (entry.commit_capable && entry.descriptor.source_revision == durable_head).then_some((
                prepare_lsn,
                &entry.descriptor,
                entry.payload_crc32c,
            ))
        })
    }

    pub(super) fn retain_published_generation(
        &mut self,
        wal_first_lsn: u64,
        seeded_prepare_lsns: impl IntoIterator<Item = u64>,
    ) {
        let seeded = seeded_prepare_lsns.into_iter().collect::<BTreeSet<_>>();
        self.by_lsn
            .retain(|lsn, _| *lsn >= wal_first_lsn || seeded.contains(lsn));
        self.rebuild_indices();
    }

    fn rebuild_indices(&mut self) {
        self.lsns_by_retry_key.clear();
        self.retry_key_by_target_revision.clear();
        for (&lsn, entry) in &self.by_lsn {
            let key = DurableTransactionKey::new(
                entry.descriptor.idempotency_epoch,
                entry.descriptor.transaction_id,
            );
            self.lsns_by_retry_key.entry(key).or_default().insert(lsn);
            self.retry_key_by_target_revision
                .entry(entry.descriptor.target_revision)
                .or_insert(key);
        }
    }

    pub(super) fn clear(&mut self) {
        self.by_lsn.clear();
        self.lsns_by_retry_key.clear();
        self.retry_key_by_target_revision.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_lsn.len()
    }
}
