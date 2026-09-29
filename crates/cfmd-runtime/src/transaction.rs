use crate::{
    Candidate, CandidatePreview, CommitOutcome, Object, ObjectSet, Plan, ReadContext, Result,
    RevisionId, TransactionId,
};

/// One explicit write intent pinned to one exact live database snapshot.
///
/// A `Transaction` is a product-side plan composer, not a second mutation engine. Object mutations
/// still produce ordinary [`Plan`] values; [`Transaction::apply`] accepts only plans derived from
/// the same snapshot and authority. Publication still goes through the ordinary Candidate/commit
/// path, so stale HEADs fail closed instead of being retried or rebased implicitly.
pub struct Transaction {
    id: TransactionId,
    context: ReadContext,
    plan: Plan,
}

impl std::fmt::Debug for Transaction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transaction")
            .field("id", &self.id)
            .field("base_revision", &self.base_revision())
            .field("is_empty", &self.is_empty())
            .finish_non_exhaustive()
    }
}

impl Transaction {
    pub(crate) fn new(context: ReadContext, id: TransactionId) -> Result<Self> {
        let plan = context.plan()?;
        Ok(Self { id, context, plan })
    }

    #[must_use]
    pub const fn id(&self) -> TransactionId {
        self.id
    }

    #[must_use]
    pub fn base_revision(&self) -> RevisionId {
        self.context.revision()
    }

    /// Exact read view used by this transaction. Reads and mutation plans derived through this
    /// context therefore share the same base revision.
    #[must_use]
    pub const fn read(&self) -> &ReadContext {
        &self.context
    }

    /// Object-first access pinned to the transaction's exact base snapshot.
    pub fn objects<E: Object>(&self) -> Result<ObjectSet<E>> {
        self.context.objects::<E>()
    }

    /// Adds an ordinary CFMD mutation plan to this transaction.
    ///
    /// The plan must come from this transaction's exact snapshot and authority. There is no hidden
    /// refresh/rebase path when it does not.
    pub fn apply(&mut self, plan: Plan) -> Result<&mut Self> {
        self.plan.extend(plan)?;
        Ok(self)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    pub fn candidate(&self) -> Result<Candidate> {
        self.plan.candidate()
    }

    pub fn preview(&self) -> Result<CandidatePreview> {
        Ok(self.candidate()?.preview())
    }

    /// Publishes this transaction through the ordinary exact Candidate/commit pipeline.
    /// The transaction remains available after publication so the exact same `(plan, id)` may be
    /// retried idempotently after an uncertain caller-side failure. A changed plan with the same
    /// transaction id is still rejected by the durable transaction-id authority.
    pub fn commit(&self) -> Result<CommitOutcome> {
        self.plan.candidate()?.commit(self.id)
    }

    /// Drops the transaction wrapper while preserving its ordinary Plan representation.
    #[must_use]
    pub fn into_plan(self) -> Plan {
        self.plan
    }
}
