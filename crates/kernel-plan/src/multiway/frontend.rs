#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
 pub(super) struct MultiwayJoinColumnRef {
    pub(super) leaf: usize,
    pub(super) column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
 pub(super) struct MultiwayJoinLeaf {
    pub(super) plan: Plan,
    pub(super) relation: SemanticId,
    pub(super) layout: LayoutBinding,
    pub(super) width: usize,
    pub(super) rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
 pub(super) struct MultiwayJoinPredicate {
    pub(super) left: MultiwayJoinColumnRef,
    pub(super) right: MultiwayJoinColumnRef,
    pub(super) equivalence: SemanticId,
}

/// Query-local APNF coordinate program.
///
/// Each logical join predicate owns a distinct observable coordinate even when
/// multiple predicates use the same pinned Γ-equivalence. The coordinate IDs
/// themselves are allocated only when execution constructs a revision-local
/// observable catalog; this prepared form contains no semantic state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedAnchorPullbackProgram {
    pub(super) leaf_count: usize,
    pub(super) predicates: Vec<MultiwayJoinPredicate>,
}

#[derive(Debug, Clone)]
struct MultiwayJoinCandidate {
    plan: Plan,
    cost: u128,
    estimated_rows: usize,
}

#[derive(Debug, Clone, Copy)]
struct MultiwayJoinInterval {
    start: usize,
    split: usize,
    end: usize,
}

struct MultiwayPlannerContext<'a> {
    store: &'a PhysicalStore,
    leaves: &'a [MultiwayJoinLeaf],
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

 struct PreferredNwayOrderPreservingJoin {
    leaves: Vec<MultiwayJoinLeaf>,
    predicates: Vec<MultiwayJoinPredicate>,
    search_order: Vec<usize>,
}

