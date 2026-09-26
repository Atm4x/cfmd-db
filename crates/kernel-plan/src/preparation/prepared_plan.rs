impl PreparedPlan {
    #[must_use]
    pub fn physical(&self) -> &Plan {
        &self.lowering.spec().physical
    }

    #[must_use]
    pub fn logical(&self) -> &RelExpr {
        &self.lowering.spec().logical
    }

    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
    }

    pub fn ordered_view(
        &self,
        spec: OrderedViewSpec,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedOrderedView<'_>, OrderedViewError> {
        let equivalences = match &self.result_type.semantics {
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            }
            | kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        let equivalence = equivalences
            .get(spec.column)
            .copied()
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !registry.ordering_congruent_with_equivalence(
            &self.semantic_context,
            spec.ordering,
            equivalence,
        )? {
            return Err(RelQueryError::OrderingNotCongruentWithEquality.into());
        }
        Ok(PreparedOrderedView {
            plan: self,
            spec,
            equivalences: equivalences.clone(),
        })
    }

    /// Builds the revision-local relation-column coordinate catalog used by
    /// writable-view compilation from this already typechecked prepared plan.
    /// Coordinates are planner-owned and distinct even when several columns
    /// share the same pinned Γ-equivalence.
    pub fn writable_coordinates(
        &self,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRelWritableCoordinates, WritableCoordinatePrepareError> {
        let mut catalog =
            kernel_semantics::observable::RevisionObservableCatalog::new(&self.semantic_context)?;
        let mut relation_columns = BTreeMap::new();
        let mut coordinate = 1_u64;
        for relation in self.logical().scan_relations() {
            let definition = self
                .semantic_context
                .schema
                .relation(relation)
                .ok_or(RelQueryError::UnknownRelation(relation))?;
            let equivalences = match &definition.semantics {
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                }
                | kernel_schema::RelationSemantics::Bag {
                    column_equivalences,
                } => column_equivalences,
            };
            let mut columns = Vec::with_capacity(equivalences.len());
            for &equivalence in equivalences {
                columns.push(catalog.register_equivalence_coordinate(
                    registry,
                    &self.semantic_context,
                    equivalence,
                    coordinate,
                )?);
                coordinate = coordinate
                    .checked_add(1)
                    .ok_or(WritableCoordinatePrepareError::CoordinateOverflow)?;
            }
            relation_columns.insert(relation, columns);
        }
        Ok(PreparedRelWritableCoordinates {
            catalog,
            relation_columns,
        })
    }

    pub fn reference_execute(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        self.physical().reference_execute(model, context, registry)
    }

    pub fn execute_native_pinned(
        &self,
        store: &PhysicalStore,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(RelationValue, ExecutionStats), PhysicalExecutionError> {
        self.physical().execute_native_with_prepared_programs(
            store,
            &self.result_type,
            &self.semantic_context,
            registry,
            self.anchor_pullback_program.as_ref(),
            self.semantic_quotient_program.as_ref(),
        )
    }

    pub fn execute_native(
        &self,
        store: &PhysicalStore,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(RelationValue, ExecutionStats), PhysicalExecutionError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch.into());
        }
        self.execute_native_pinned(store, registry)
    }

    pub(super) fn semantic_quotient_factor_bindings(
        &self,
        store: &PhysicalStore,
    ) -> Result<BTreeSet<SemanticIndexBinding>, PhysicalExecutionError> {
        let Some(program) = &self.semantic_quotient_program else {
            return Ok(BTreeSet::new());
        };
        let mut leaves = Vec::new();
        let mut predicates = Vec::new();
        let Some(_) =
            flatten_multiway_join_tree(self.physical(), store, &mut leaves, &mut predicates)?
        else {
            return Ok(BTreeSet::new());
        };
        let mut bindings = BTreeSet::new();
        for (equivalence, endpoints) in &program.specs {
            for endpoint in endpoints {
                let leaf = leaves
                    .get(endpoint.leaf)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                bindings.insert(SemanticIndexBinding::single(
                    leaf.relation,
                    leaf.layout,
                    endpoint.column,
                    *equivalence,
                ));
            }
        }
        Ok(bindings)
    }

    // HOSTILE[P190][ACTIVE][CLEAN]: storage advisor asks the prepared plan for planner-eligible
    // quotient bindings instead of importing multiway availability policy.
    pub(super) fn semantic_quotient_advisor_factor_bindings(
        &self,
        store: &PhysicalStore,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<BTreeSet<SemanticIndexBinding>, PhysicalExecutionError> {
        if !preferred_nway_order_preserving_join_available(
            self.physical(),
            self.semantic_quotient_program.as_ref(),
            store,
            &self.semantic_context,
            registry,
            true,
        )? {
            return Ok(BTreeSet::new());
        }
        self.semantic_quotient_factor_bindings(store)
    }


    /// Materializes the exact canonical-key factors used by this prepared Γ-quotient
    /// program as dedicated reconstructible physical state. Storage owns the atomic
    /// representation transition; preparation supplies only the semantic bindings.
    pub fn materialize_semantic_quotient_factors(
        &self,
        store: &mut PhysicalStore,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<usize, PhysicalExecutionError> {
        let bindings = self.semantic_quotient_factor_bindings(store)?;
        store.materialize_semantic_quotient_factors(
            &bindings,
            &self.semantic_context,
            registry,
        )
    }

    /// Materializes the exact Γ-QCN common-domain/support fixed point as reconstructible
    /// physical state. Factor preparation, support construction, advisor-ownership release,
    /// and publication are staged and committed atomically by storage.
    pub fn materialize_semantic_quotient_support(
        &self,
        store: &mut PhysicalStore,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let Some(program) = &self.semantic_quotient_program else {
            return Ok(false);
        };
        let factor_bindings = self.semantic_quotient_factor_bindings(store)?;
        let mut leaves = Vec::new();
        let mut predicates = Vec::new();
        let Some(_) =
            flatten_multiway_join_tree(self.physical(), store, &mut leaves, &mut predicates)?
        else {
            return Ok(false);
        };
        let support_binding = semantic_quotient_support_binding(&leaves, program);
        store.materialize_semantic_quotient_support(
            &factor_bindings,
            support_binding,
            &self.semantic_context,
            registry,
        )
    }

}

pub fn prepare_baseline(
    logical: RelExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<PreparedPlan, PlanPrepareError> {
    let prepared = logical.prepare(context, registry)?;
    let result_type = prepared.result_type().clone();
    let lowering = certify_baseline_lowering(logical)?;
    let anchor_pullback_program =
        prepare_anchor_pullback_program(&lowering.spec().physical, context)?;
    let semantic_quotient_program =
        prepare_semantic_quotient_program(&lowering.spec().physical, context, registry)?;
    Ok(PreparedPlan {
        lowering,
        result_type,
        semantic_context: context.clone(),
        anchor_pullback_program,
        semantic_quotient_program,
    })
}

pub fn certify_baseline_lowering(
    logical: RelExpr,
) -> Result<kernel_proof::CheckedCertificate<LoweringChecker>, LoweringError> {
    let physical = Plan::lower_baseline(&logical);
    let spec = LoweringSpec { logical, physical };
    kernel_proof::verify_certificate::<LoweringChecker>(
        &spec,
        LoweringCertificate::ExactLogicalRoundTrip,
    )
}

pub fn certify_lowering_with_catalog(
    logical: RelExpr,
    catalog: &PhysicalCatalog,
) -> Result<kernel_proof::CheckedCertificate<LoweringChecker>, LoweringError> {
    let physical = Plan::lower_with_catalog(&logical, catalog);
    let spec = LoweringSpec { logical, physical };
    kernel_proof::verify_certificate::<LoweringChecker>(
        &spec,
        LoweringCertificate::ExactLogicalRoundTrip,
    )
}

pub fn prepare_with_catalog(
    logical: RelExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    catalog: &PhysicalCatalog,
) -> Result<PreparedPlan, PlanPrepareError> {
    let prepared = logical.prepare(context, registry)?;
    let result_type = prepared.result_type().clone();
    let lowering = certify_lowering_with_catalog(logical, catalog)?;
    let anchor_pullback_program =
        prepare_anchor_pullback_program(&lowering.spec().physical, context)?;
    let semantic_quotient_program =
        prepare_semantic_quotient_program(&lowering.spec().physical, context, registry)?;
    Ok(PreparedPlan {
        lowering,
        result_type,
        semantic_context: context.clone(),
        anchor_pullback_program,
        semantic_quotient_program,
    })
}
