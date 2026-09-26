use super::{
    BTreeMap, BTreeSet, PhysicalExecutionError, RelQueryError, SemanticId, SemanticIndexBinding,
};

// HOSTILE[P185][ACTIVE][CLEAN]: semantic-key resolution is revision-semantic capability,
// not physical storage representation. Storage, execution, recovery, and multiway share this owner.
pub(super) fn resolve_semantic_key_binding(
    binding: &SemanticIndexBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<
    (
        Vec<ResolvedSemanticIndexKeyPart>,
        Vec<kernel_semantic_index::SemanticModuleBinding>,
    ),
    PhysicalExecutionError,
> {
    if binding.key_parts.is_empty() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    let definition = context
        .schema
        .relation(binding.relation)
        .ok_or(RelQueryError::UnknownRelation(binding.relation))?;
    let mut resolved = Vec::with_capacity(binding.key_parts.len());
    let mut dependencies = Vec::new();
    for part in &binding.key_parts {
        if part.column >= definition.columns.len() {
            return Err(RelQueryError::ColumnOutOfBounds.into());
        }
        if let Some(module) = registry.resolve_primitive_equivalence(context, part.equivalence)? {
            resolved.push(ResolvedSemanticIndexKeyPart::Primitive(module));
        } else {
            resolved.push(ResolvedSemanticIndexKeyPart::Structural(
                registry.compile_equivalence(context, part.equivalence)?,
            ));
        }
        dependencies.extend(
            registry
                .canonical_equivalence_dependencies(context, part.equivalence)?
                .into_iter()
                .map(|dependency| kernel_semantic_index::SemanticModuleBinding {
                    semantic_id: dependency.semantic_id,
                    module_digest: dependency.module_digest,
                }),
        );
    }
    dependencies.sort_unstable();
    dependencies.dedup();
    Ok((resolved, dependencies))
}

pub(super) fn resolve_primitive_semantic_index_binding(
    binding: &SemanticIndexBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<
    (
        Vec<kernel_semantics::ResolvedPrimitiveEquivalence>,
        Vec<kernel_semantic_index::SemanticModuleBinding>,
    ),
    PhysicalExecutionError,
> {
    if binding.key_parts.is_empty() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    let definition = context
        .schema
        .relation(binding.relation)
        .ok_or(RelQueryError::UnknownRelation(binding.relation))?;
    let mut resolved = Vec::with_capacity(binding.key_parts.len());
    let mut dependencies = Vec::with_capacity(binding.key_parts.len());
    for part in &binding.key_parts {
        if part.column >= definition.columns.len() {
            return Err(RelQueryError::ColumnOutOfBounds.into());
        }
        let Some(module) = registry.resolve_primitive_equivalence(context, part.equivalence)?
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        dependencies.push(kernel_semantic_index::SemanticModuleBinding {
            semantic_id: part.equivalence,
            module_digest: module.module_digest(),
        });
        resolved.push(module);
    }
    dependencies.sort_unstable();
    dependencies.dedup();
    Ok((resolved, dependencies))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedSemanticIndexKeyPart {
    Primitive(kernel_semantics::ResolvedPrimitiveEquivalence),
    Structural(kernel_semantics::CompiledEquivalence),
}

pub(super) fn semantic_key_dependencies_for_equivalences(
    equivalences: impl IntoIterator<Item = SemanticId>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_semantic_index::SemanticModuleBinding>, PhysicalExecutionError> {
    let mut dependencies = BTreeSet::new();
    for equivalence in equivalences {
        dependencies.extend(
            registry
                .canonical_equivalence_dependencies(context, equivalence)?
                .into_iter()
                .map(|dependency| kernel_semantic_index::SemanticModuleBinding {
                    semantic_id: dependency.semantic_id,
                    module_digest: dependency.module_digest,
                }),
        );
    }
    Ok(dependencies.into_iter().collect())
}

pub(super) fn semantic_key_structural_definitions_for_equivalences(
    equivalences: impl IntoIterator<Item = SemanticId>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_semantic_index::StructuralEquivalenceBinding>, PhysicalExecutionError> {
    let mut definitions = BTreeMap::new();
    for equivalence in equivalences {
        for dependency in
            registry.canonical_structural_equivalence_dependencies(context, equivalence)?
        {
            definitions.insert(dependency.semantic_id, dependency.definition);
        }
    }
    Ok(definitions
        .into_iter()
        .map(
            |(semantic_id, definition)| kernel_semantic_index::StructuralEquivalenceBinding {
                semantic_id,
                definition,
            },
        )
        .collect())
}
// HOSTILE[P186][ACTIVE][CLEAN]: canonical semantic-index row-key construction is shared by
// materialized indexes and durable recovery; it depends only on resolved semantic key parts.
pub(super) fn resolved_semantic_index_row_key(
    binding: &SemanticIndexBinding,
    resolved: &[ResolvedSemanticIndexKeyPart],
    row: &kernel_query::Row,
) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
    binding
        .key_parts
        .iter()
        .zip(resolved)
        .map(|(part, resolved)| {
            let value = row
                .get(part.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            match resolved {
                ResolvedSemanticIndexKeyPart::Primitive(module) => {
                    module.canonical_key(value).map_err(Into::into)
                }
                ResolvedSemanticIndexKeyPart::Structural(compiled) => {
                    compiled.canonical_key(value).map_err(Into::into)
                }
            }
        })
        .collect()
}
