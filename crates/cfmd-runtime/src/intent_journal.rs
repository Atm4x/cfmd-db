use crate::{
    Id, Object, ObjectSet, Plan, ReadContext, Result, RevisionId, SemanticRuleExpr, TransactionId,
    Value, ValueCodec,
};

/// Whether a transaction may be semantically transported to a newer live head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntentBasis {
    Adaptive,
    Snapshot,
}

/// One passive atomic set of write intentions.
///
/// `IntentJournal::new()` is intentionally not attached to a database or revision. Its first
/// object/relationship mutation binds the exact effect construction to that database's then-current
/// read world, but that revision is only provenance: [`crate::Database::commit`] may transport the
/// already-formed effect through later compatible history using the kernel change algebra.
///
/// `IntentJournal::from(snapshot)` is different: the supplied snapshot is part of the caller's
/// intent. A newer head therefore invalidates publication rather than being transported silently.
pub struct IntentJournal {
    id: Option<TransactionId>,
    context: Option<ReadContext>,
    plan: Option<Plan>,
    requirements: Vec<IntentRequirement>,
    scoped_relational_observations: Vec<kernel_plan::RuntimeRelationalCausalObservation>,
    basis: IntentBasis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntentRequirement {
    pub(crate) relation: crate::RelationId,
    pub(crate) entity: u128,
    pub(crate) identity_column: usize,
    pub(crate) identity_value: Value,
    pub(crate) expression: SemanticRuleExpr,
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u64::try_from(len).unwrap_or(u64::MAX).to_le_bytes());
}

fn encode_requirement(requirement: &IntentRequirement) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&requirement.relation.raw().to_le_bytes());
    out.extend_from_slice(&requirement.entity.to_le_bytes());
    out.extend_from_slice(
        &u64::try_from(requirement.identity_column)
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    let kernel_expression = crate::schema::semantic_rule_to_kernel(requirement.expression.clone());
    out.extend_from_slice(&kernel_schema::canonical_semantic_rule_bytes(
        &kernel_expression,
    ));
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentReadiness {
    Unbound,
    Ready {
        revision: RevisionId,
    },
    Rebasable {
        base_revision: RevisionId,
        current_revision: RevisionId,
        intervening_effect_count: usize,
    },
    SnapshotChanged {
        snapshot_revision: RevisionId,
        current_revision: RevisionId,
    },
    Conflict {
        base_revision: RevisionId,
        current_revision: RevisionId,
        conflicting_effects: Vec<u128>,
        coordination_effects: Vec<u128>,
        conflicting_coordinates: usize,
        opaque_effects: Vec<u128>,
    },
}

impl std::fmt::Debug for IntentJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntentJournal")
            .field("id", &self.id)
            .field("origin_revision", &self.origin_revision())
            .field("snapshot_bound", &self.is_snapshot_bound())
            .field("requirements", &self.requirements.len())
            .field("is_empty", &self.is_empty())
            .finish_non_exhaustive()
    }
}

impl Default for IntentJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl IntentJournal {
    /// Creates an empty adaptive transaction.
    ///
    /// It has no database/revision authority until its first mutation. `IntentJournal` identity is
    /// generated lazily from the operating system CSPRNG when the first exact effect is added.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            id: None,
            context: None,
            plan: None,
            requirements: Vec::new(),
            scoped_relational_observations: Vec::new(),
            basis: IntentBasis::Adaptive,
        }
    }

    pub(crate) fn from_scoped_formation(formation: ReadContext) -> Self {
        Self {
            id: None,
            context: Some(formation),
            plan: None,
            requirements: Vec::new(),
            scoped_relational_observations: Vec::new(),
            basis: IntentBasis::Adaptive,
        }
    }

    pub(crate) fn from_scoped_snapshot(formation: ReadContext) -> Self {
        Self {
            id: None,
            context: Some(formation),
            plan: None,
            requirements: Vec::new(),
            scoped_relational_observations: Vec::new(),
            basis: IntentBasis::Snapshot,
        }
    }

    pub(crate) fn set_idempotency_key(&mut self, id: TransactionId) -> Result<()> {
        if self.id.is_some() || self.plan.is_some() || !self.requirements.is_empty() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction idempotency key must be selected before effects or requirements are added",
            ));
        }
        self.id = Some(id);
        Ok(())
    }

    /// Selects a caller-stable durable idempotency key before semantic intent is formed.
    ///
    /// The key is an orthogonal namespace identity: exact effects and passive requirements remain
    /// independently compared under it. Configuring a key does not bind an adaptive transaction to
    /// a database or revision; its first mutation still establishes formation provenance. The same
    /// configurator is valid for `IntentJournal::from(snapshot)` before any effect/requirement is added.
    pub fn with_idempotency_key(mut self, id: TransactionId) -> Result<Self> {
        self.set_idempotency_key(id)?;
        Ok(self)
    }

    /// Returns the durable idempotency key once selected/generated.
    #[must_use]
    pub const fn id(&self) -> Option<TransactionId> {
        self.id
    }

    /// Revision from which the exact effect was originally formed, if the transaction is bound.
    /// For an adaptive transaction this is provenance, not a global staleness condition.
    #[must_use]
    pub fn origin_revision(&self) -> Option<RevisionId> {
        self.plan
            .as_ref()
            .map(Plan::base_revision)
            .or_else(|| self.context.as_ref().map(ReadContext::revision))
    }

    #[must_use]
    pub const fn is_snapshot_bound(&self) -> bool {
        matches!(self.basis, IntentBasis::Snapshot)
    }

    /// Adds a passive semantic precondition evaluated against this transaction's proposed future world.
    ///
    /// The same expression substrate as persisted Semantic Rules is used; no host callback is stored.
    /// If an adaptive transaction is later rebased, the condition is evaluated again against the
    /// certified rebased Candidate before publication.
    pub fn require<E: Object>(
        &mut self,
        entity: Id<E>,
        expression: SemanticRuleExpr,
    ) -> Result<&mut Self> {
        self.require_on_relation(entity, E::relation_id(), expression)
    }

    pub(crate) fn require_on_relation<E: Object>(
        &mut self,
        entity: Id<E>,
        relation: crate::RelationId,
        expression: SemanticRuleExpr,
    ) -> Result<&mut Self> {
        let identity_column = E::identity_column().ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidSchema,
                format!(
                    "object {} has no identity field for transaction requirement",
                    E::KEY
                ),
            )
        })?;
        self.requirements.push(IntentRequirement {
            relation,
            entity: entity.raw(),
            identity_column,
            identity_value: entity.into_value(),
            expression,
        });
        Ok(self)
    }

    pub(crate) fn requirements(&self) -> &[IntentRequirement] {
        &self.requirements
    }

    pub(crate) fn client_guard_digest(&self) -> Option<kernel_durability::ClientIntentGuardDigest> {
        if self.requirements.is_empty() {
            return None;
        }
        let mut encoded = self
            .requirements
            .iter()
            .map(encode_requirement)
            .collect::<Vec<_>>();
        encoded.sort();
        encoded.dedup();
        let mut canonical = b"CFMD-TX-REQUIREMENTS-v1\0".to_vec();
        push_len(&mut canonical, encoded.len());
        for requirement in encoded {
            push_len(&mut canonical, requirement.len());
            canonical.extend_from_slice(&requirement);
        }
        Some(kernel_durability::ClientIntentGuardDigest::canonical(
            &canonical,
        ))
    }

    /// Object-first access to the transaction's bound formation world.
    ///
    /// Normal application code should prefer `db.objects::<T>()?.add(&mut tx, value)` so the
    /// database/collection stays visible at the mutation call site.
    pub fn objects<E: Object>(&self) -> Result<ObjectSet<E>> {
        self.context
            .as_ref()
            .ok_or_else(|| crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction is not bound yet; mutate through a database object collection first",
            ))?
            .objects::<E>()
    }

    /// Advanced composition escape hatch for an explicitly constructed plan.
    #[doc(hidden)]
    pub fn add_plan(&mut self, plan: Plan) -> Result<()> {
        if let Some(existing) = &mut self.plan {
            existing.extend(plan)?;
        } else {
            if let Some(context) = &self.context
                && (context.database_identity() != plan.database_identity
                    || context.revision() != plan.base_revision()
                    || context.authority != plan.authority)
            {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidPlan,
                    "plan does not belong to the transaction formation world",
                ));
            }
            self.plan = Some(plan);
        }
        self.ensure_identity()?;
        Ok(())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.as_ref().is_none_or(Plan::is_empty)
    }

    pub(crate) fn operation_context(&mut self, proposed: &ReadContext) -> Result<ReadContext> {
        if let Some(context) = &self.context {
            if context.database_identity() != proposed.database_identity()
                || context.authority != proposed.authority
            {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidPlan,
                    "transaction cannot mix database or session authorities",
                ));
            }
            if proposed.revision() != context.revision() {
                return Ok(proposed.without_intent_relational_causal_capture());
            }
            Ok(context.without_intent_relational_causal_capture())
        } else {
            let context = proposed.with_intent_relational_causal_capture();
            self.context = Some(context.clone());
            Ok(context.without_intent_relational_causal_capture())
        }
    }

    pub(crate) fn append_scoped_relational_causal_observations(
        &mut self,
        observations: impl IntoIterator<Item = kernel_plan::RuntimeRelationalCausalObservation>,
    ) {
        self.scoped_relational_observations.extend(observations);
    }

    pub(crate) fn relational_causal_observations(
        &self,
    ) -> Result<Vec<kernel_plan::RuntimeRelationalCausalObservation>> {
        let mut observations = self
            .context
            .as_ref()
            .map(ReadContext::relational_causal_observations)
            .transpose()?
            .unwrap_or_default();
        observations.extend(self.scoped_relational_observations.iter().cloned());
        for (index, observation) in observations.iter_mut().enumerate() {
            observation.observation_id = u32::try_from(index).map_err(|_| {
                crate::Error::new(
                    crate::ErrorKind::ResourceLimit,
                    "transaction relational observation id space exhausted",
                )
            })?;
        }
        Ok(observations)
    }

    pub(crate) fn plan(&self) -> Result<&Plan> {
        self.plan.as_ref().ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction has no exact changes to preview or publish",
            )
        })
    }

    pub(crate) fn transaction_id(&self) -> Result<TransactionId> {
        self.id.ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction has no durable identity before its first exact change",
            )
        })
    }

    pub(crate) fn database_identity(&self) -> Option<u64> {
        self.plan
            .as_ref()
            .map(|plan| plan.database_identity)
            .or_else(|| self.context.as_ref().map(ReadContext::database_identity))
    }

    pub(crate) fn authority(&self) -> Option<&crate::security::RuntimeAuthority> {
        self.plan
            .as_ref()
            .map(|plan| &plan.authority)
            .or_else(|| self.context.as_ref().map(|context| &context.authority))
    }

    fn ensure_identity(&mut self) -> Result<()> {
        if self.id.is_some() {
            return Ok(());
        }
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            crate::Error::new(
                crate::ErrorKind::Internal,
                format!("operating-system transaction identity generation failed: {error}"),
            )
        })?;
        let mut raw = u128::from_le_bytes(bytes);
        if raw == 0 {
            raw = 1;
        }
        self.id = Some(TransactionId::new(raw));
        Ok(())
    }

    /// Drops the transaction wrapper while preserving its advanced Plan representation.
    pub fn into_plan(self) -> Result<Plan> {
        if !self.requirements.is_empty() {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction requirements cannot be discarded into a bare Plan",
            ));
        }
        self.plan.ok_or_else(|| {
            crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "transaction contains no plan",
            )
        })
    }
}

impl From<ReadContext> for IntentJournal {
    fn from(snapshot: ReadContext) -> Self {
        Self {
            id: None,
            context: Some(snapshot.with_intent_relational_causal_capture()),
            plan: None,
            requirements: Vec::new(),
            scoped_relational_observations: Vec::new(),
            basis: IntentBasis::Snapshot,
        }
    }
}
