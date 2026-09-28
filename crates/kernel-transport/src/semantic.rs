use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::IdentityTransport;
use kernel_model::{DatabaseState, FiniteModel};
use kernel_query::RelExpr;
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;

use crate::{
    TransportError,
    core::{
        check_rel_impact_identity_context_law, transport_database_state, transport_lifecycle_graph,
        transport_lifecycle_intent,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BijectiveIdentityRevisionTransport {
    source: SemanticContext,
    target: SemanticContext,
    identity: IdentityTransport,
}

impl BijectiveIdentityRevisionTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        identity: IdentityTransport,
    ) -> Result<Self, TransportError> {
        if !registry
            .contexts_semantically_equivalent(source, target)
            .map_err(TransportError::TargetSemantics)?
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            identity,
        })
    }

    fn transport_state(&self, source: &DatabaseState) -> Result<DatabaseState, TransportError> {
        transport_database_state(&self.identity, source)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state())?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn identity(&self) -> &IdentityTransport {
        &self.identity
    }

    pub fn transport_lifecycle_intent(
        &self,
        intent: &kernel_lifecycle::LifecycleIntent,
    ) -> Result<kernel_lifecycle::LifecycleIntent, TransportError> {
        transport_lifecycle_intent(&self.identity, intent)
    }

    pub fn transport_lifecycle_graph(
        &self,
        graph: &kernel_lifecycle::LifecycleGraph,
    ) -> Result<kernel_lifecycle::LifecycleGraph, TransportError> {
        transport_lifecycle_graph(&self.identity, graph)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConservativeSemanticEnvironmentExtension {
    source: SemanticContext,
    target: SemanticContext,
}

impl ConservativeSemanticEnvironmentExtension {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        if !registry
            .context_conservatively_extends(source, target)
            .map_err(TransportError::TargetSemantics)?
        {
            return Err(TransportError::NotConservativeSemanticExtension);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivalentSemanticEnvironmentTransport {
    source: SemanticContext,
    target: SemanticContext,
}

impl EquivalentSemanticEnvironmentTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }
        let source_modules: BTreeMap<_, _> = source.environment.modules().collect();
        let target_modules: BTreeMap<_, _> = target.environment.modules().collect();
        if source_modules.keys().collect::<BTreeSet<_>>()
            != target_modules.keys().collect::<BTreeSet<_>>()
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        for (dependency, source_digest) in source_modules {
            let target_digest = target_modules[&dependency];
            if !registry
                .equivalent_implementation_contract(source_digest, target_digest)
                .map_err(TransportError::TargetSemantics)?
            {
                return Err(TransportError::SemanticContractChanged(dependency));
            }
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticLawMigration {
    source: SemanticContext,
    target: SemanticContext,
}

impl SemanticLawMigration {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }
        let mut changed = false;
        let source_modules: BTreeMap<_, _> = source.environment.modules().collect();
        let target_modules: BTreeMap<_, _> = target.environment.modules().collect();
        if source_modules.keys().collect::<BTreeSet<_>>()
            != target_modules.keys().collect::<BTreeSet<_>>()
        {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        for (dependency, source_digest) in source_modules {
            let target_digest = target_modules[&dependency];
            changed |= !registry
                .equivalent_implementation_contract(source_digest, target_digest)
                .map_err(TransportError::TargetSemantics)?;
        }
        if !changed {
            return Err(TransportError::NoSemanticLawChange);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        kernel_revision::Revision::build(target_id, &self.target, registry, source.state().clone())
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionalTransport {
    source: SemanticContext,
    target: SemanticContext,
}

impl DefinitionalTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.definitionally_equivalent(target) {
            return Err(TransportError::NotDefinitionallyEquivalent);
        }
        Ok(Self {
            source: source.clone(),
            target: target.clone(),
        })
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state());
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    pub fn check_rel_impact_law(
        &self,
        query: &RelExpr,
        old: &FiniteModel,
        change: &kernel_change::Change<FiniteModel>,
        registry: &SemanticRegistry,
    ) -> Result<bool, TransportError> {
        check_rel_impact_identity_context_law(
            query,
            old,
            change,
            &self.source,
            &self.target,
            registry,
        )
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn transport_state(&self, state: &DatabaseState) -> DatabaseState {
        state.clone()
    }
}
