use std::collections::{BTreeMap, BTreeSet};

use kernel_model::{FiniteModel, Value};
use kernel_schema::{ModuleDigest, SemanticContext};
use kernel_types::SemanticId;

use crate::contracts::{
    BUILTIN_ORDERING_COMPATIBILITIES, OrderingCompatibilityArtifact, OrderingCompatibilityChecker,
    OrderingCompatibilitySpec, SemanticImplementationArtifact, SemanticImplementationChecker,
    certify_ordering_compatibility,
};
use crate::equivalence::{EquivalenceImplementation, EquivalenceModule, domain_for_type};
use crate::error::SemanticError;
use crate::implementation_descriptor::BuiltinSemanticModuleSpec;
use crate::ordering::{OrderingImplementation, OrderingModule};
use crate::tokenizer::{TokenizerImplementation, TokenizerModule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRegistry {
    equivalences: BTreeMap<ModuleDigest, EquivalenceImplementation>,
    tokenizers: BTreeMap<ModuleDigest, TokenizerImplementation>,
    orderings: BTreeMap<ModuleDigest, OrderingImplementation>,
    ordering_compatibilities: BTreeSet<OrderingCompatibilitySpec>,
}

impl Default for SemanticRegistry {
    fn default() -> Self {
        let mut registry = Self {
            equivalences: BTreeMap::new(),
            tokenizers: BTreeMap::new(),
            orderings: BTreeMap::new(),
            ordering_compatibilities: BTreeSet::new(),
        };
        for spec in BUILTIN_ORDERING_COMPATIBILITIES {
            if let Ok(checked) =
                certify_ordering_compatibility(&spec, OrderingCompatibilityArtifact::BuiltinLaw)
            {
                registry.install_certified_ordering_compatibility(checked);
            }
        }
        registry
    }
}

impl SemanticRegistry {
    /// Installs one builtin implementation from its durable descriptor and
    /// returns the digest that callers must compare with the pinned semantic
    /// environment.
    pub fn install_builtin_module_spec(&mut self, spec: BuiltinSemanticModuleSpec) -> ModuleDigest {
        match spec {
            BuiltinSemanticModuleSpec::Equivalence {
                module,
                implementation_revision,
            } => self.install_equivalence_revision(module, implementation_revision),
            BuiltinSemanticModuleSpec::Tokenizer {
                module,
                implementation_revision,
            } => self.install_tokenizer_revision(module, implementation_revision),
            BuiltinSemanticModuleSpec::Ordering {
                module,
                implementation_revision,
            } => self.install_ordering_revision(module, implementation_revision),
        }
    }

    /// Returns the builtin descriptor for an installed digest.  At present all
    /// executable semantic modules supported by CFMD are builtin contracts, so
    /// a pinned digest without a descriptor is a deployment error.
    #[must_use]
    pub fn builtin_module_spec(&self, digest: ModuleDigest) -> Option<BuiltinSemanticModuleSpec> {
        if let Some(implementation) = self.equivalences.get(&digest) {
            return Some(BuiltinSemanticModuleSpec::Equivalence {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            });
        }
        if let Some(implementation) = self.tokenizers.get(&digest) {
            return Some(BuiltinSemanticModuleSpec::Tokenizer {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            });
        }
        self.orderings
            .get(&digest)
            .map(|implementation| BuiltinSemanticModuleSpec::Ordering {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            })
    }

    /// Collects the exact builtin implementations pinned by one semantic
    /// context, sorted by digest and deduplicated.  This is the deployment
    /// manifest persisted by the durability layer.
    pub fn builtin_modules_for_context(
        &self,
        context: &SemanticContext,
    ) -> Result<Vec<BuiltinSemanticModuleSpec>, SemanticError> {
        self.validate_context(context)?;
        let mut by_digest = BTreeMap::new();
        for (_, digest) in context.environment.modules() {
            let spec = self
                .builtin_module_spec(digest)
                .ok_or(SemanticError::ModuleUnavailable(digest))?;
            by_digest.insert(digest, spec);
        }
        Ok(by_digest.into_values().collect())
    }

    pub fn install_certified_ordering_compatibility(
        &mut self,
        checked: kernel_proof::CheckedCertificate<OrderingCompatibilityChecker>,
    ) {
        let spec = *checked.spec();
        let _ = checked.into_inner();
        self.ordering_compatibilities.insert(spec);
    }

    pub fn install_certified_implementation(
        &mut self,
        checked: kernel_proof::CheckedCertificate<SemanticImplementationChecker>,
        implementation_revision: u64,
    ) -> ModuleDigest {
        match checked.into_inner() {
            SemanticImplementationArtifact::BuiltinEquivalence(module) => {
                self.install_equivalence_revision(module, implementation_revision)
            }
            SemanticImplementationArtifact::BuiltinTokenizer(module) => {
                self.install_tokenizer_revision(module, implementation_revision)
            }
            SemanticImplementationArtifact::BuiltinOrdering(module) => {
                self.install_ordering_revision(module, implementation_revision)
            }
        }
    }
    pub fn contexts_semantically_equivalent(
        &self,
        left: &SemanticContext,
        right: &SemanticContext,
    ) -> Result<bool, SemanticError> {
        self.validate_context(left)?;
        self.validate_context(right)?;
        if !left.schema.definitionally_equivalent(&right.schema) {
            return Ok(false);
        }
        let left_count = left.environment.modules().count();
        let right_count = right.environment.modules().count();
        if left_count != right_count {
            return Ok(false);
        }
        for (symbol, left_digest) in left.environment.modules() {
            let Some(right_digest) = right.environment.module(symbol) else {
                return Ok(false);
            };
            if !self.equivalent_implementation_contract(left_digest, right_digest)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn context_conservatively_extends(
        &self,
        source: &SemanticContext,
        target: &SemanticContext,
    ) -> Result<bool, SemanticError> {
        self.validate_context(source)?;
        self.validate_context(target)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Ok(false);
        }
        for (symbol, source_digest) in source.environment.modules() {
            let Some(target_digest) = target.environment.module(symbol) else {
                return Ok(false);
            };
            if !self.equivalent_implementation_contract(source_digest, target_digest)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn validate_context(&self, context: &SemanticContext) -> Result<(), SemanticError> {
        for (_, digest) in context.environment.modules() {
            if !self.module_available(digest) {
                return Err(SemanticError::ModuleUnavailable(digest));
            }
        }
        for dependency in context.schema.semantic_dependencies() {
            let digest = context
                .environment
                .module(dependency)
                .ok_or(SemanticError::WrongModuleKind(dependency))?;
            if !self.module_available(digest) {
                return Err(SemanticError::ModuleUnavailable(digest));
            }
            if !self.equivalences.contains_key(&digest) {
                return Err(SemanticError::WrongModuleKind(dependency));
            }
        }
        self.validate_structural_equivalence_graph(context)?;
        self.validate_structural_ordering_graph(context)?;
        for relation in context.schema.relations() {
            let column_equivalences = match &relation.semantics {
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                }
                | kernel_schema::RelationSemantics::Bag {
                    column_equivalences,
                } => column_equivalences,
            };
            for (column, equivalence) in relation.columns.iter().zip(column_equivalences) {
                let expected =
                    domain_for_type(column).ok_or(SemanticError::TypeMismatch(*equivalence))?;
                let actual = self.equivalence_domain(context, *equivalence)?;
                if expected != actual {
                    return Err(SemanticError::EquivalenceDomainMismatch {
                        equivalence: *equivalence,
                        expected,
                        actual,
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn equivalence_implementation(
        &self,
        digest: ModuleDigest,
    ) -> Option<EquivalenceImplementation> {
        self.equivalences.get(&digest).copied()
    }

    pub(super) fn ordering_implementation(
        &self,
        digest: ModuleDigest,
    ) -> Option<OrderingImplementation> {
        self.orderings.get(&digest).copied()
    }

    pub(super) fn ordering_compatibility_installed(
        &self,
        spec: &OrderingCompatibilitySpec,
    ) -> bool {
        self.ordering_compatibilities.contains(spec)
    }

    pub(super) fn module_available(&self, digest: ModuleDigest) -> bool {
        self.equivalences.contains_key(&digest)
            || self.tokenizers.contains_key(&digest)
            || self.orderings.contains_key(&digest)
    }

    pub fn install_ordering(&mut self, module: OrderingModule) -> ModuleDigest {
        self.install_ordering_revision(module, 0)
    }

    pub fn install_ordering_revision(
        &mut self,
        module: OrderingModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = OrderingImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.orderings.insert(digest, implementation);
        digest
    }

    pub fn install_tokenizer(&mut self, module: TokenizerModule) -> ModuleDigest {
        self.install_tokenizer_revision(module, 0)
    }

    pub fn install_tokenizer_revision(
        &mut self,
        module: TokenizerModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = TokenizerImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.tokenizers.insert(digest, implementation);
        digest
    }

    pub fn tokenize(
        &self,
        context: &SemanticContext,
        tokenizer: SemanticId,
        text: &str,
    ) -> Result<Vec<String>, SemanticError> {
        let digest = context
            .environment
            .module(tokenizer)
            .ok_or(SemanticError::WrongModuleKind(tokenizer))?;
        let implementation = self
            .tokenizers
            .get(&digest)
            .ok_or(SemanticError::WrongModuleKind(tokenizer))?;
        Ok(implementation.contract.tokenize(text))
    }

    pub fn install_equivalence(&mut self, module: EquivalenceModule) -> ModuleDigest {
        self.install_equivalence_revision(module, 0)
    }

    pub fn install_equivalence_revision(
        &mut self,
        module: EquivalenceModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = EquivalenceImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.equivalences.insert(digest, implementation);
        digest
    }

    pub fn equivalent_implementation_contract(
        &self,
        left: ModuleDigest,
        right: ModuleDigest,
    ) -> Result<bool, SemanticError> {
        if let (Some(left), Some(right)) =
            (self.equivalences.get(&left), self.equivalences.get(&right))
        {
            return Ok(left.contract == right.contract);
        }
        if let (Some(left), Some(right)) = (self.tokenizers.get(&left), self.tokenizers.get(&right))
        {
            return Ok(left.contract == right.contract);
        }
        if let (Some(left), Some(right)) = (self.orderings.get(&left), self.orderings.get(&right)) {
            return Ok(left.contract == right.contract);
        }
        if !self.module_available(left) {
            return Err(SemanticError::ModuleUnavailable(left));
        }
        if !self.module_available(right) {
            return Err(SemanticError::ModuleUnavailable(right));
        }
        Ok(false)
    }

    pub fn validate_model(
        &self,
        context: &SemanticContext,
        model: &FiniteModel,
    ) -> Result<(), SemanticError> {
        for value in model.fields.values() {
            self.validate_value(context, value)?;
        }
        for tuples in model.relations.values() {
            for tuple in tuples {
                for value in tuple {
                    self.validate_value(context, value)?;
                }
            }
        }
        Ok(())
    }

    fn validate_value(
        &self,
        context: &SemanticContext,
        value: &Value,
    ) -> Result<(), SemanticError> {
        match value {
            Value::Product(values) => {
                for child in values.values() {
                    self.validate_value(context, child)?;
                }
            }
            Value::Seq(values) => {
                for child in values {
                    self.validate_value(context, child)?;
                }
            }
            Value::Variant { value, .. } => self.validate_value(context, value)?,
            Value::Option(value) => {
                if let Some(value) = value.as_deref() {
                    self.validate_value(context, value)?;
                }
            }
            Value::Set {
                equivalence,
                elements,
            } => {
                self.ensure_unique(context, *equivalence, elements, |equivalence| {
                    SemanticError::DuplicateSetElement { equivalence }
                })?;
                for child in elements {
                    self.validate_value(context, child)?;
                }
            }
            Value::Bag {
                equivalence,
                entries,
            } => {
                let elements: Vec<_> = entries.iter().map(|(value, _)| value.clone()).collect();
                self.ensure_unique(context, *equivalence, &elements, |equivalence| {
                    SemanticError::DuplicateBagElement { equivalence }
                })?;
                for (child, _) in entries {
                    self.validate_value(context, child)?;
                }
            }
            Value::Map {
                key_equivalence,
                entries,
            } => {
                let keys: Vec<_> = entries.iter().map(|(key, _)| key.clone()).collect();
                self.ensure_unique(context, *key_equivalence, &keys, |equivalence| {
                    SemanticError::DuplicateMapKey { equivalence }
                })?;
                for (key, mapped) in entries {
                    self.validate_value(context, key)?;
                    self.validate_value(context, mapped)?;
                }
            }
            Value::Unit
            | Value::Bool(_)
            | Value::I64(_)
            | Value::F64Bits(_)
            | Value::Text(_)
            | Value::LiveEntityRef { .. }
            | Value::HistoricalEntityId { .. } => {}
        }
        Ok(())
    }

    fn ensure_unique<F>(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        values: &[Value],
        duplicate: F,
    ) -> Result<(), SemanticError>
    where
        F: Fn(SemanticId) -> SemanticError,
    {
        let mut seen = BTreeSet::new();
        for value in values {
            let key = self.canonical_equivalence_key(context, equivalence, value)?;
            if !seen.insert(key) {
                return Err(duplicate(equivalence));
            }
        }
        Ok(())
    }
}
