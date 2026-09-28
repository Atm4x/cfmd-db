use std::collections::BTreeSet;

use kernel_change::RevisionEffectId;
use kernel_types::{ClientTransactionId, RevisionId};

use super::transaction::{DurableEffectKind, DurableTransactionIntent, IdempotencyEpoch};

/// Durable causal identity for one exact committed transition.
///
/// Event identity is deliberately independent from the client retry id: raw
/// transaction ids may be reused in a later idempotency epoch.  The canonical
/// intent is retained by the causal record itself so retry-payload GC cannot
/// tear a Γ-REIC ideal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRevisionEffectRecord {
    pub id: RevisionEffectId,
    pub prerequisites: BTreeSet<RevisionEffectId>,
    pub transaction_epoch: IdempotencyEpoch,
    pub transaction_id: ClientTransactionId,
    pub intent: DurableTransactionIntent,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableEffectCoordinationClass {
    /// No durable confluence/coherence certificate is attached to this effect.
    /// It must therefore be ordered, explicitly resolved, or coordinated; it
    /// is never eligible for automatic REIC union merely because its payload
    /// happens to be a Rewrite.
    OpaqueNonConfluent,
}

impl DurableRevisionEffectRecord {
    #[must_use]
    pub const fn kind(&self) -> DurableEffectKind {
        self.intent.effect_kind()
    }

    #[must_use]
    pub const fn coordination_class(&self) -> DurableEffectCoordinationClass {
        DurableEffectCoordinationClass::OpaqueNonConfluent
    }

    pub fn validate_identity(&self) -> Result<(), &'static str> {
        if self.source_revision == self.target_revision {
            return Err("revision effect does not advance revision identity");
        }
        if self.intent.target_revision() != self.target_revision {
            return Err("revision effect canonical intent targets another revision");
        }
        if self
            .intent
            .source_revision()
            .is_some_and(|source| source != self.source_revision)
        {
            return Err("revision effect canonical intent starts from another revision");
        }
        if self.prerequisites.contains(&self.id) {
            return Err("revision effect depends on itself");
        }
        Ok(())
    }
}
