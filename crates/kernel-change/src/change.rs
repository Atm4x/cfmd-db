use std::collections::BTreeSet;

use kernel_types::SemanticId;

/// Universal logical change. `Fine` is an extensional refinement of the same
/// endpoint semantics; it does not become authority for semantic equality by
/// virtue of its structural tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<T> {
    NoChange,
    Replace(T),
    Fine(FineChange<T>),
}

impl<T: Clone> Change<T> {
    #[must_use]
    pub fn apply(&self, old: &T) -> T {
        match self {
            Self::NoChange => old.clone(),
            Self::Replace(new) => new.clone(),
            Self::Fine(fine) => fine.apply(old),
        }
    }

    /// Extensional composition. Fine changes carry an absolute endpoint, so a
    /// later non-empty change supersedes the earlier endpoint without needing
    /// representation-specific delta algebra.
    #[must_use]
    pub fn compose(self, later: Self) -> Self {
        match later {
            Self::NoChange => self,
            other => other,
        }
    }
}

impl<T: Clone + PartialEq> Change<T> {
    /// Removes a semantically *representation-level* no-op against an exact
    /// endpoint. Callers using coarser Γ equality must canonicalize/prove that
    /// equivalence before invoking this helper.
    #[must_use]
    pub fn normalize_exact(self, old: &T) -> Self {
        match &self {
            Self::NoChange => Self::NoChange,
            Self::Replace(next) if next == old => Self::NoChange,
            Self::Fine(fine) if fine.endpoint() == old => Self::NoChange,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FineChangeKind {
    Scalar,
    Product,
    Sum,
    Option,
    Set,
    Bag,
    Seq,
    Map,
    Relation,
    Recursive,
    Other(SemanticId),
}

/// First-class extensional fine effect.
///
/// `endpoint` is authoritative for universal `apply`; `kind` records which
/// structural calculus produced the refinement. This makes `FineChange` total
/// today while allowing Γ-aware Set/Bag/Map/Relation payloads to migrate in
/// without changing `Change<T>` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FineChange<T> {
    kind: FineChangeKind,
    endpoint: T,
}

impl<T> FineChange<T> {
    #[must_use]
    pub const fn new(kind: FineChangeKind, endpoint: T) -> Self {
        Self { kind, endpoint }
    }

    #[must_use]
    pub const fn kind(&self) -> FineChangeKind {
        self.kind
    }

    #[must_use]
    pub const fn endpoint(&self) -> &T {
        &self.endpoint
    }

    #[must_use]
    pub fn into_endpoint(self) -> T {
        self.endpoint
    }
}

impl<T: Clone> FineChange<T> {
    #[must_use]
    pub fn apply(&self, _old: &T) -> T {
        self.endpoint.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetChange<T> {
    pub inserted: BTreeSet<T>,
    pub removed: BTreeSet<T>,
}

impl<T: Clone + Ord> SetChange<T> {
    #[must_use]
    pub fn apply(&self, old: &BTreeSet<T>) -> BTreeSet<T> {
        let mut next = old.clone();
        for value in &self.removed {
            next.remove(value);
        }
        next.extend(self.inserted.iter().cloned());
        next
    }

    #[must_use]
    pub fn between(old: &BTreeSet<T>, new: &BTreeSet<T>) -> Self {
        Self {
            inserted: new.difference(old).cloned().collect(),
            removed: old.difference(new).cloned().collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }

    /// Compatibility adapter. This adapter is only for exact Rust-ordered
    /// sets; Γ-class-aware collections must construct their endpoint through
    /// the pinned semantic module before creating `FineChange`.
    #[must_use]
    pub fn into_fine(&self, old: &BTreeSet<T>) -> FineChange<BTreeSet<T>> {
        FineChange::new(FineChangeKind::Set, self.apply(old))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqSplice<T> {
    pub start: usize,
    pub delete_count: usize,
    pub insert: Vec<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqChangeError {
    StartOutOfBounds,
    DeleteOutOfBounds,
}

impl<T: Clone> SeqSplice<T> {
    pub fn apply(&self, old: &[T]) -> Result<Vec<T>, SeqChangeError> {
        if self.start > old.len() {
            return Err(SeqChangeError::StartOutOfBounds);
        }
        let end = self
            .start
            .checked_add(self.delete_count)
            .ok_or(SeqChangeError::DeleteOutOfBounds)?;
        if end > old.len() {
            return Err(SeqChangeError::DeleteOutOfBounds);
        }

        let mut next = Vec::with_capacity(old.len() - self.delete_count + self.insert.len());
        next.extend_from_slice(&old[..self.start]);
        next.extend(self.insert.iter().cloned());
        next.extend_from_slice(&old[end..]);
        Ok(next)
    }

    pub fn into_fine(&self, old: &[T]) -> Result<FineChange<Vec<T>>, SeqChangeError> {
        Ok(FineChange::new(FineChangeKind::Seq, self.apply(old)?))
    }
}
