#[derive(Debug, Clone)]
struct SemanticIndexAdviceObservation {
    binding: SemanticIndexBinding,
    savings_per_execution: u128,
    key_cells: usize,
    estimated_bytes: usize,
}

#[derive(Debug, Clone)]
struct SemanticIndexAdviceScore {
    binding: SemanticIndexBinding,
    gross_savings: u128,
    key_cells: usize,
    estimated_bytes: usize,
}

#[derive(Debug, Clone)]
struct SemanticIndexAdviceChoice {
    binding: SemanticIndexBinding,
    work: PhysicalWorkEstimate,
    key_cells: usize,
    estimated_bytes: usize,
    compatible_existing: bool,
    advisor_managed: bool,
    replaced_fixed_bytes: usize,
}

#[derive(Debug)]
struct SemanticQuotientFactorAdviceScore {
    binding: SemanticIndexBinding,
    work: PhysicalWorkEstimate,
    row_count: usize,
    estimated_bytes: usize,
    compatible_existing: bool,
    advisor_managed: bool,
    prepared_state: Option<MaterializedSemanticQuotientFactorState>,
    replaced_fixed_bytes: usize,
}

struct SemanticQuotientFactorAdvisorSelection {
    selected: BTreeSet<SemanticIndexBinding>,
    prepared_states: Vec<(
        SemanticIndexBinding,
        MaterializedSemanticQuotientFactorState,
    )>,
    managed_estimated_bytes: usize,
    report: SemanticQuotientFactorAdvisorReport,
}

#[derive(Debug)]
struct I64IndexAdviceChoice {
    binding: I64IndexBinding,
    work: PhysicalWorkEstimate,
    row_count: usize,
    estimated_bytes: usize,
    existing_manual: bool,
    advisor_managed: bool,
    prepared_state: Option<MaterializedI64IndexState>,
}

struct I64IndexAdvisorSelection {
    selected: BTreeSet<I64IndexBinding>,
    prepared_states: Vec<(I64IndexBinding, MaterializedI64IndexState)>,
    managed_estimated_bytes: usize,
    report: I64IndexAdvisorReport,
}

struct SemanticStatisticsAdvisorSelection {
    selected: BTreeSet<SemanticIndexBinding>,
    prepared_states: Vec<(SemanticIndexBinding, MaterializedSemanticStatisticsState)>,
    managed_estimated_bytes: usize,
    report: SemanticStatisticsAdvisorReport,
}

fn canonical_eq_key_heap_bytes(key: &kernel_semantics::CanonicalEqKey) -> usize {
    use kernel_semantics::CanonicalEqKey;
    match key {
        CanonicalEqKey::Unit
        | CanonicalEqKey::Bool(_)
        | CanonicalEqKey::I64(_)
        | CanonicalEqKey::F64Bits(_)
        | CanonicalEqKey::OptionNone
        | CanonicalEqKey::LiveEntityId { .. }
        | CanonicalEqKey::HistoricalEntityId { .. } => 0,
        CanonicalEqKey::TextExact(value) | CanonicalEqKey::TextAsciiCaseInsensitive(value) => {
            value.capacity()
        }
        CanonicalEqKey::Product(fields) => fields
            .capacity()
            .saturating_mul(std::mem::size_of::<(SemanticId, CanonicalEqKey)>())
            .saturating_add(saturating_usize_sum(
                fields
                    .iter()
                    .map(|(_, value)| canonical_eq_key_heap_bytes(value)),
            )),
        CanonicalEqKey::OptionSome(value) | CanonicalEqKey::Variant { value, .. } => {
            std::mem::size_of::<CanonicalEqKey>().saturating_add(canonical_eq_key_heap_bytes(value))
        }
        CanonicalEqKey::Seq(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<CanonicalEqKey>())
            .saturating_add(saturating_usize_sum(
                values.iter().map(canonical_eq_key_heap_bytes),
            )),
        CanonicalEqKey::Set(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<
                kernel_semantics::FiniteMeasureEntry<CanonicalEqKey>,
            >())
            .saturating_add(saturating_usize_sum(
                values
                    .iter()
                    .map(|entry| canonical_eq_key_heap_bytes(&entry.atom)),
            )),
        CanonicalEqKey::Bag(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<
                kernel_semantics::FiniteMeasureEntry<kernel_semantics::CanonicalBagAtom>,
            >())
            .saturating_add(saturating_usize_sum(
                values
                    .iter()
                    .map(|entry| canonical_eq_key_heap_bytes(&entry.atom.value)),
            )),
        CanonicalEqKey::Map(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<
                kernel_semantics::FiniteMeasureEntry<kernel_semantics::CanonicalMapAtom>,
            >())
            .saturating_add(saturating_usize_sum(values.iter().map(|entry| {
                canonical_eq_key_heap_bytes(&entry.atom.key)
                    .saturating_add(canonical_eq_key_heap_bytes(&entry.atom.value))
            }))),
    }
}

fn semantic_key_retained_bytes(key: &Vec<kernel_semantics::CanonicalEqKey>) -> usize {
    std::mem::size_of::<Vec<kernel_semantics::CanonicalEqKey>>()
        .saturating_add(
            key.capacity()
                .saturating_mul(std::mem::size_of::<kernel_semantics::CanonicalEqKey>()),
        )
        .saturating_add(saturating_usize_sum(
            key.iter().map(canonical_eq_key_heap_bytes),
        ))
}

fn semantic_key_heap_bytes(key: &Vec<kernel_semantics::CanonicalEqKey>) -> usize {
    key.capacity()
        .saturating_mul(std::mem::size_of::<kernel_semantics::CanonicalEqKey>())
        .saturating_add(saturating_usize_sum(
            key.iter().map(canonical_eq_key_heap_bytes),
        ))
}

fn semantic_index_binding_heap_bytes(binding: &SemanticIndexBinding) -> usize {
    binding
        .key_parts
        .capacity()
        .saturating_mul(std::mem::size_of::<SemanticIndexKeyPart>())
}

fn semantic_index_estimated_retained_bytes(state: &MaterializedSemanticIndexState) -> usize {
    std::mem::size_of::<SemanticIndexBinding>()
        .saturating_add(semantic_index_binding_heap_bytes(&state.binding))
        .saturating_add(
            state
                .resolved
                .capacity()
                .saturating_mul(std::mem::size_of::<
                    kernel_semantics::ResolvedPrimitiveEquivalence,
                >()),
        )
        .saturating_add(
            state
                .index
                .estimated_retained_bytes(semantic_key_heap_bytes),
        )
}

fn observable_atom_estimated_retained_bytes(state: &MaterializedObservableAtomState) -> usize {
    let class_bytes = std::mem::size_of::<kernel_types::EqClassId>();
    let row_bytes = std::mem::size_of::<PhysicalRowId>();
    let atom_rows = state.fabric.row_count();
    let atoms = state.fabric.atom_count();
    let coordinates = state.observables.len();
    let projection_refs = state.fabric.projection_atom_reference_count();
    let projected_classes = state.fabric.projected_class_count();
    let projection_mapping_bytes =
        state
            .projection
            .materialized_mapping()
            .iter()
            .fold(0_usize, |bytes, (source, target)| {
                bytes
                    .saturating_add(std::mem::size_of::<Vec<kernel_types::EqClassId>>() * 2)
                    .saturating_add(source.capacity().saturating_mul(class_bytes))
                    .saturating_add(target.capacity().saturating_mul(class_bytes))
            });

    std::mem::size_of::<MaterializedObservableAtomState>()
        .saturating_add(semantic_index_binding_heap_bytes(&state.binding))
        .saturating_add(
            state
                .observables
                .capacity()
                .saturating_mul(std::mem::size_of::<kernel_types::RevisionObservableId>()),
        )
        .saturating_add(atom_rows.saturating_mul(row_bytes.saturating_mul(2)))
        .saturating_add(atom_rows.saturating_mul(class_bytes))
        .saturating_add(
            atoms
                .saturating_mul(coordinates)
                .saturating_mul(class_bytes),
        )
        .saturating_add(projection_refs.saturating_mul(class_bytes))
        .saturating_add(projected_classes.saturating_mul(class_bytes))
        .saturating_add(projection_mapping_bytes)
}

fn i64_index_estimated_retained_bytes(state: &MaterializedI64IndexState) -> usize {
    state.buckets.iter().fold(
        std::mem::size_of::<MaterializedI64IndexState>(),
        |bytes, (_, identities)| {
            bytes
                .saturating_add(std::mem::size_of::<i64>())
                .saturating_add(identities.estimated_retained_bytes())
        },
    )
}

fn quotient_factor_estimated_retained_bytes(
    state: &MaterializedSemanticQuotientFactorState,
) -> usize {
    let mut bytes = std::mem::size_of::<MaterializedSemanticQuotientFactorState>()
        .saturating_add(semantic_index_binding_heap_bytes(&state.binding));
    for (key, identities) in &state.buckets {
        bytes = bytes
            .saturating_add(semantic_key_retained_bytes(key))
            .saturating_add(identities.estimated_retained_bytes());
    }
    for key in state.reverse.values() {
        bytes = bytes
            .saturating_add(std::mem::size_of::<PhysicalRowId>())
            .saturating_add(semantic_key_retained_bytes(key));
    }
    bytes
}

fn semantic_statistics_estimated_retained_bytes(
    state: &MaterializedSemanticStatisticsState,
) -> usize {
    state.counts.iter().fold(
        std::mem::size_of::<MaterializedSemanticStatisticsState>()
            .saturating_add(semantic_index_binding_heap_bytes(&state.binding))
            .saturating_add(
                state
                    .resolved
                    .capacity()
                    .saturating_mul(std::mem::size_of::<
                        kernel_semantics::ResolvedPrimitiveEquivalence,
                    >()),
            ),
        |bytes, (key, _)| {
            bytes
                .saturating_add(semantic_key_retained_bytes(key))
                .saturating_add(std::mem::size_of::<usize>())
        },
    )
}

fn quotient_support_estimated_retained_bytes(
    state: &MaterializedSemanticQuotientSupportState,
) -> usize {
    state.estimated_retained_bytes(canonical_eq_key_heap_bytes)
}

