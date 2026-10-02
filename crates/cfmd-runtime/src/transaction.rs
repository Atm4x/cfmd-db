use crate::{Id, Object, ObjectSet, Plan, ReadContext, Result, RevisionId, SemanticRuleExpr, TransactionId, Value, ValueCodec};

/// Whether a transaction may be semantically transported to a newer live head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionBasis {
    Adaptive,
    Snapshot,
}

/// One passive atomic set of write intentions.
///
/// `Transaction::new()` is intentionally not attached to a database or revision. Its first
/// object/relationship mutation binds the exact effect construction to that database's then-current
/// read world, but that revision is only provenance: [`crate::Database::commit`] may transport the
/// already-formed effect through later compatible history using the kernel change algebra.
///
/// `Transaction::from(snapshot)` is different: the supplied snapshot is part of the caller's
/// intent. A newer head therefore invalidates publication rather than being transported silently.
pub struct Transaction {
    id: Option<TransactionId>,
    context: Option<ReadContext>,
    plan: Option<Plan>,
    requirements: Vec<TransactionRequirement>,
    basis: TransactionBasis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransactionRequirement {
    pub(crate) relation: crate::RelationId,
    pub(crate) entity: u128,
    pub(crate) identity_column: usize,
    pub(crate) identity_value: Value,
    pub(crate) expression: SemanticRuleExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionReadiness {
    Unbound,
    Ready {
        revision: RevisionId,
    },
    Rebasable {
        base_revision: RevisionId,
        current_revision: RevisionId,
        intervening_effects: Vec<u128>,
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

impl std::fmt::Debug for Transaction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transaction")
            .field("id", &self.id)
            .field("origin_revision", &self.origin_revision())
            .field("snapshot_bound", &self.is_snapshot_bound())
            .field("requirements", &self.requirements.len())
            .field("is_empty", &self.is_empty())
            .finish_non_exhaustive()
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Transaction {
    /// Creates an empty adaptive transaction.
    ///
    /// It has no database/revision authority until its first mutation. Transaction identity is
    /// generated lazily from the operating system CSPRNG when the first exact effect is added.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            id: None,
            context: None,
            plan: None,
            requirements: Vec::new(),
            basis: TransactionBasis::Adaptive,
        }
    }

    pub(crate) fn from_context_with_id(context: ReadContext, id: TransactionId) -> Result<Self> {
        let plan = context.plan()?;
        Ok(Self {
            id: Some(id),
            context: Some(context),
            plan: Some(plan),
            requirements: Vec::new(),
            basis: TransactionBasis::Adaptive,
        })
    }

    /// Returns the durable client identity once this transaction contains an exact effect.
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
        matches!(self.basis, TransactionBasis::Snapshot)
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
        let identity_column = E::identity_column().ok_or_else(|| crate::Error::new(
            crate::ErrorKind::InvalidSchema,
            format!("object {} has no identity field for transaction requirement", E::KEY),
        ))?;
        self.requirements.push(TransactionRequirement {
            relation: E::relation_id(),
            entity: entity.raw(),
            identity_column,
            identity_value: entity.into_value(),
            expression,
        });
        // Requirements are part of client intent even though they are publication guards rather
        // than durable effects. Rotating the opaque identity prevents a post-build requirement
        // change from aliasing an earlier retry identity.
        self.id = None;
        self.ensure_identity()?;
        Ok(self)
    }

    pub(crate) fn requirements(&self) -> &[TransactionRequirement] {
        &self.requirements
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
            Ok(context.clone())
        } else {
            self.context = Some(proposed.clone());
            Ok(proposed.clone())
        }
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

impl From<ReadContext> for Transaction {
    fn from(snapshot: ReadContext) -> Self {
        Self {
            id: None,
            context: Some(snapshot),
            plan: None,
            requirements: Vec::new(),
            basis: TransactionBasis::Snapshot,
        }
    }
}
