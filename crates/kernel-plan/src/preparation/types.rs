use super::{
    BTreeMap, BTreeSet, ExecutionStats,
    OrderDirection, PhysicalCatalog, PhysicalExecutionError, PhysicalStore, Plan,
    RelExpr, RelQueryError,
    RelType, RelationDelta, RelationValue, RevisionId, RevisionObservableId, RuntimeRevisionBundle,
    SemanticId, SemanticIndexBinding, };
use crate::multiway::{
    PreparedAnchorPullbackProgram, PreparedSemanticQuotientProgram,
    flatten_multiway_join_tree, preferred_nway_order_preserving_join_available,
    prepare_anchor_pullback_program, prepare_semantic_quotient_program,
    semantic_quotient_support_binding,
};

#[must_use]
pub fn logical_node_count(expr: &RelExpr) -> usize {
    match expr {
        RelExpr::Scan(_) => 1,
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::TopKWithTies { input, .. }
        | RelExpr::PromoteToBag(input) => 1 + logical_node_count(input),
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            1 + logical_node_count(left) + logical_node_count(right)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweringSpec {
    pub logical: RelExpr,
    pub physical: Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringCertificate {
    ExactLogicalRoundTrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringError {
    LogicalMeaningChanged,
    HiddenPlanExpansion,
}

pub struct LoweringChecker;

impl kernel_proof::CertificateChecker for LoweringChecker {
    type Spec = LoweringSpec;
    type Certificate = LoweringCertificate;
    type Error = LoweringError;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        match certificate {
            LoweringCertificate::ExactLogicalRoundTrip => {
                if spec.physical.to_logical_expr() != spec.logical {
                    return Err(LoweringError::LogicalMeaningChanged);
                }
                if spec.physical.shape().nodes != logical_node_count(&spec.logical) {
                    return Err(LoweringError::HiddenPlanExpansion);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanPrepareError {
    Query(RelQueryError),
    Lowering(LoweringError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableCoordinatePrepareError {
    Query(RelQueryError),
    Observable(kernel_semantics::observable::ObservableError),
    AnchorPullback(kernel_semantics::anchor_pullback::AnchorPullbackError),
    UnknownRelation(SemanticId),
    RowArityMismatch {
        relation: SemanticId,
        expected: usize,
        actual: usize,
    },
    ColumnOutOfBounds {
        relation: SemanticId,
        column: usize,
        arity: usize,
    },
    CoordinateOverflow,
}

impl From<RelQueryError> for WritableCoordinatePrepareError {
    fn from(value: RelQueryError) -> Self {
        Self::Query(value)
    }
}

impl From<kernel_semantics::observable::ObservableError> for WritableCoordinatePrepareError {
    fn from(value: kernel_semantics::observable::ObservableError) -> Self {
        Self::Observable(value)
    }
}

impl From<kernel_semantics::anchor_pullback::AnchorPullbackError>
    for WritableCoordinatePrepareError
{
    fn from(value: kernel_semantics::anchor_pullback::AnchorPullbackError) -> Self {
        Self::AnchorPullback(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelWritableCoordinates {
    catalog: kernel_semantics::observable::RevisionObservableCatalog,
    relation_columns: BTreeMap<SemanticId, Vec<RevisionObservableId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevalidatedRelationDeterminant {
    relation: SemanticId,
    source: Vec<RevisionObservableId>,
    target: Vec<RevisionObservableId>,
    before: kernel_semantics::observable::CertifiedSemanticMorphism,
    after: kernel_semantics::observable::CertifiedSemanticMorphism,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationDeterminantColumns<'a> {
    pub source: &'a [usize],
    pub target: &'a [usize],
}

impl RevalidatedRelationDeterminant {
    #[must_use]
    pub const fn relation(&self) -> SemanticId {
        self.relation
    }

    #[must_use]
    pub fn source(&self) -> &[RevisionObservableId] {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &[RevisionObservableId] {
        &self.target
    }

    #[must_use]
    pub const fn before(&self) -> &kernel_semantics::observable::CertifiedSemanticMorphism {
        &self.before
    }

    #[must_use]
    pub const fn after(&self) -> &kernel_semantics::observable::CertifiedSemanticMorphism {
        &self.after
    }

    #[must_use]
    pub fn after_domain_is_covered_by_before(&self) -> bool {
        self.after
            .materialized_mapping()
            .keys()
            .all(|source| self.before.image(source).is_some())
    }

    #[must_use]
    pub fn shared_domain_images_are_stable(&self) -> bool {
        self.before
            .materialized_mapping()
            .iter()
            .all(|(source, target)| {
                self.after
                    .image(source)
                    .is_none_or(|after_target| after_target == target.as_slice())
            })
    }
}

impl PreparedRelWritableCoordinates {
    #[must_use]
    pub const fn catalog(&self) -> &kernel_semantics::observable::RevisionObservableCatalog {
        &self.catalog
    }

    #[must_use]
    pub const fn relation_columns(&self) -> &BTreeMap<SemanticId, Vec<RevisionObservableId>> {
        &self.relation_columns
    }

    pub fn determinant_theory(
        &self,
        morphisms: Vec<kernel_semantics::observable::CertifiedSemanticMorphism>,
    ) -> Result<
        kernel_semantics::anchor_pullback::DeterminantTheory,
        kernel_semantics::anchor_pullback::AnchorPullbackError,
    > {
        kernel_semantics::anchor_pullback::DeterminantTheory::new(
            &self.catalog,
            self.relation_columns.values().flatten().copied(),
            morphisms,
        )
    }

    pub fn relation_measure(
        &mut self,
        relation: SemanticId,
        value: &RelationValue,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<
        kernel_semantics::anchor_pullback::RevisionFiniteMeasure,
        WritableCoordinatePrepareError,
    > {
        let coordinates = self
            .relation_columns
            .get(&relation)
            .cloned()
            .ok_or(WritableCoordinatePrepareError::UnknownRelation(relation))?;
        let mut rows = Vec::with_capacity(value.rows().len());
        for row in value.rows() {
            if row.len() != coordinates.len() {
                return Err(WritableCoordinatePrepareError::RowArityMismatch {
                    relation,
                    expected: coordinates.len(),
                    actual: row.len(),
                });
            }
            let classes = coordinates
                .iter()
                .zip(row)
                .map(|(&observable, cell)| {
                    self.catalog
                        .observe_value(registry, semantic, observable, cell)
                })
                .collect::<Result<Vec<_>, _>>()?;
            rows.push((classes, 1_u64));
        }
        Ok(
            kernel_semantics::anchor_pullback::RevisionFiniteMeasure::new(
                &self.catalog,
                coordinates,
                rows,
            )?,
        )
    }

    fn relation_observables_for_columns(
        &self,
        relation: SemanticId,
        columns: &[usize],
    ) -> Result<Vec<RevisionObservableId>, WritableCoordinatePrepareError> {
        let observables = self
            .relation_columns
            .get(&relation)
            .ok_or(WritableCoordinatePrepareError::UnknownRelation(relation))?;
        columns
            .iter()
            .map(|&column| {
                observables.get(column).copied().ok_or(
                    WritableCoordinatePrepareError::ColumnOutOfBounds {
                        relation,
                        column,
                        arity: observables.len(),
                    },
                )
            })
            .collect()
    }

    pub fn relation_determinant_morphism(
        &mut self,
        relation: SemanticId,
        value: &RelationValue,
        source_columns: &[usize],
        target_columns: &[usize],
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<
        Option<kernel_semantics::observable::CertifiedSemanticMorphism>,
        WritableCoordinatePrepareError,
    > {
        let source = self.relation_observables_for_columns(relation, source_columns)?;
        let target = self.relation_observables_for_columns(relation, target_columns)?;
        let measure = self.relation_measure(relation, value, semantic, registry)?;
        Ok(measure.determinant_morphism(&self.catalog, source, target)?)
    }

    pub fn revalidate_relation_determinant(
        &mut self,
        relation: SemanticId,
        before: &RelationValue,
        after: &RelationValue,
        columns: RelationDeterminantColumns<'_>,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<RevalidatedRelationDeterminant>, WritableCoordinatePrepareError> {
        let source = self.relation_observables_for_columns(relation, columns.source)?;
        let target = self.relation_observables_for_columns(relation, columns.target)?;
        let before_measure = self.relation_measure(relation, before, semantic, registry)?;
        let after_measure = self.relation_measure(relation, after, semantic, registry)?;
        let Some(before) =
            before_measure.determinant_morphism(&self.catalog, source.clone(), target.clone())?
        else {
            return Ok(None);
        };
        let Some(after) =
            after_measure.determinant_morphism(&self.catalog, source.clone(), target.clone())?
        else {
            return Ok(None);
        };
        Ok(Some(RevalidatedRelationDeterminant {
            relation,
            source,
            target,
            before,
            after,
        }))
    }
}

impl From<RelQueryError> for PlanPrepareError {
    fn from(value: RelQueryError) -> Self {
        Self::Query(value)
    }
}

impl From<LoweringError> for PlanPrepareError {
    fn from(value: LoweringError) -> Self {
        Self::Lowering(value)
    }
}

// HOSTILE[P165][ACTIVE][PRIMARY]: pinned logical+Γ lowering certificate; runtime-dependent access paths stay outside the prepared plan.
pub struct PreparedPlan {
    lowering: kernel_proof::CheckedCertificate<LoweringChecker>,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    pub(super) anchor_pullback_program: Option<PreparedAnchorPullbackProgram>,
    pub(super) semantic_quotient_program: Option<PreparedSemanticQuotientProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedViewSpec {
    pub column: usize,
    pub ordering: SemanticId,
    pub direction: OrderDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedViewCursor {
    revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    semantic_context: kernel_schema::SemanticContext,
    logical: RelExpr,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    order_class: kernel_semantics::CanonicalOrderClassKey,
    row_key: Vec<kernel_semantics::CanonicalEqKey>,
    pub(super) occurrence: u64,
}

impl OrderedViewCursor {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn semantic_revision(&self) -> kernel_types::SemanticRevision {
        self.semantic_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedViewPage {
    pub rows: Vec<kernel_query::Row>,
    pub next_cursor: Option<OrderedViewCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderedViewError {
    Query(RelQueryError),
    Execution(PhysicalExecutionError),
    UnboundPhysicalRevision,
    UnknownMaterialization(kernel_types::MaterializationId),
    MaterializationQueryMismatch,
    SnapshotBindingMismatch,
    SnapshotDeltaTypeMismatch,
    SnapshotMultiplicityUnderflow,
    CursorBindingMismatch,
    CursorPositionMismatch,
    ZeroPageSize,
}

impl From<RelQueryError> for OrderedViewError {
    fn from(value: RelQueryError) -> Self {
        Self::Query(value)
    }
}

impl From<PhysicalExecutionError> for OrderedViewError {
    fn from(value: PhysicalExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl From<kernel_semantics::SemanticError> for OrderedViewError {
    fn from(value: kernel_semantics::SemanticError) -> Self {
        Self::Query(RelQueryError::Semantic(value))
    }
}

// HOSTILE[P163][ACTIVE][CLEAN:P160.O]: ordered paging is snapshot-bound; query execution and
// canonical sorting happen once per exact physical revision snapshot, never once per page.
pub struct PreparedOrderedView<'a> {
    plan: &'a PreparedPlan,
    spec: OrderedViewSpec,
    equivalences: Vec<SemanticId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
 struct OrderedViewRun {
    row: kernel_query::Row,
    count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedViewSnapshot {
    pub(super) revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    pub(super) semantic_context: kernel_schema::SemanticContext,
    pub(super) logical: RelExpr,
    result_type: RelType,
    equivalences: Vec<SemanticId>,
    spec: OrderedViewSpec,
    runs: BTreeMap<
        kernel_semantics::CanonicalOrderClassKey,
        BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, OrderedViewRun>,
    >,
}

