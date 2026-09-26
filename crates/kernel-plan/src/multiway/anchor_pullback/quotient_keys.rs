pub(super) fn try_execute_anchor_pullback_join(
    plan: &Plan,
    program: &PreparedAnchorPullbackProgram,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(None);
    };
    if leaves.len() != program.leaf_count || predicates != program.predicates {
        return Err(PhysicalExecutionError::AnchorPullbackInvariant);
    }

    let mut catalog = kernel_semantics::observable::RevisionObservableCatalog::new(context)
        .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)?;
    let predicate_observables =
        register_anchor_pullback_observables(&predicates, &mut catalog, registry, context)?;
    let mut finite_factors = Vec::with_capacity(leaves.len());
    let mut row_buckets = Vec::with_capacity(leaves.len());
    {
        let mut builder = AnchorPullbackFactorBuilder {
            predicates: &predicates,
            predicate_observables: &predicate_observables,
            catalog: &mut catalog,
            store,
            context,
            registry,
            stats,
        };
        for (leaf_index, leaf) in leaves.iter().enumerate() {
            let built = builder.build(leaf_index, leaf)?;
            finite_factors.push(built.measure);
            row_buckets.push(built.rows_by_tuple);
        }
    }
    let normal_form = kernel_semantics::anchor_pullback::AnchorPullbackNormalForm::new(
        &catalog,
        finite_factors,
        Vec::new(),
        predicate_observables,
    )
    .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)?;
    let factors = anchor_pullback_runtime_factors(&normal_form, row_buckets)?;
    let (search_order, has_branch_free) = anchor_pullback_search_order(&normal_form, &factors)?;
    if has_branch_free {
        stats.multiway_join_apnf_branch_free =
            stats.multiway_join_apnf_branch_free.saturating_add(1);
    }
    let mut matches = AnchorPullbackExecution {
        factors: &factors,
        search_order: &search_order,
        stats,
        assignments: BTreeMap::new(),
        selected: vec![None; factors.len()],
        matches: Vec::new(),
    }
    .run();
    matches.sort_by(|left, right| left.0.cmp(&right.0));
    stats.multiway_join_apnf_executions = stats.multiway_join_apnf_executions.saturating_add(1);
    Ok(Some(matches.into_iter().map(|(_, row)| row).collect()))
}

struct SemanticQuotientBuildContext<'a> {
    leaves: &'a [MultiwayJoinLeaf],
    predicates: &'a [MultiwayJoinPredicate],
    handles: &'a [Vec<PhysicalRowId>],
    store: &'a dyn SemanticQuotientStoreView,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn semantic_quotient_physical_leaves(
    leaves: &[MultiwayJoinLeaf],
) -> Vec<SemanticQuotientPhysicalLeaf> {
    leaves
        .iter()
        .map(|leaf| SemanticQuotientPhysicalLeaf {
            relation: leaf.relation,
            layout: leaf.layout,
            width: leaf.width,
        })
        .collect()
}

fn prepared_semantic_quotient_specs(
    specs: &[(SemanticId, Vec<MultiwayJoinColumnRef>)],
) -> Vec<(SemanticId, Vec<SemanticQuotientEndpoint>)> {
    specs
        .iter()
        .map(|(equivalence, endpoints)| {
            (
                *equivalence,
                endpoints
                    .iter()
                    .map(|endpoint| SemanticQuotientEndpoint {
                        leaf: endpoint.leaf,
                        column: endpoint.column,
                    })
                    .collect(),
            )
        })
        .collect()
}

fn build_semantic_quotient_constraints(
    build: &SemanticQuotientBuildContext<'_>,
    prepared_specs: Option<&[(SemanticId, Vec<MultiwayJoinColumnRef>)]>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<SemanticQuotientConstraint>>, PhysicalExecutionError> {
    let owned_planner_specs;
    let planner_specs = if let Some(prepared) = prepared_specs {
        prepared
    } else {
        owned_planner_specs = semantic_quotient_specs(build.predicates, build.context, build.registry)?;
        &owned_planner_specs
    };
    let specs = prepared_semantic_quotient_specs(planner_specs);
    let leaves = semantic_quotient_physical_leaves(build.leaves);
    let prepared = PreparedSemanticQuotientBuildContext {
        leaves: &leaves,
        handles: build.handles,
        store: build.store,
        context: build.context,
        registry: build.registry,
    };
    build_semantic_quotient_constraints_for_prepared_specs(&prepared, &specs, stats)
}

