use std::collections::BTreeMap;

use kernel_types::{SemanticEnvId, SemanticId, SemanticRevision};

use crate::Schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleDigest(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticEnvironment {
    pub revision: SemanticEnvId,
    modules: BTreeMap<SemanticId, ModuleDigest>,
}

impl SemanticEnvironment {
    #[must_use]
    pub fn new(revision: SemanticEnvId) -> Self {
        Self {
            revision,
            modules: BTreeMap::new(),
        }
    }

    pub fn pin_module(&mut self, module: SemanticId, digest: ModuleDigest) {
        self.modules.insert(module, digest);
    }

    #[must_use]
    pub fn definitionally_equivalent(&self, other: &Self) -> bool {
        self.modules == other.modules
    }

    #[must_use]
    pub fn module(&self, module: SemanticId) -> Option<ModuleDigest> {
        self.modules.get(&module).copied()
    }

    #[must_use]
    pub fn has_module(&self, module: SemanticId) -> bool {
        self.modules.contains_key(&module)
    }

    pub fn modules(&self) -> impl Iterator<Item = (SemanticId, ModuleDigest)> + '_ {
        self.modules.iter().map(|(&id, &digest)| (id, digest))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticContext {
    pub schema: Schema,
    pub environment: SemanticEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    MissingSemanticModule(SemanticId),
}

impl SemanticContext {
    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        SemanticRevision::new(self.schema.revision, self.environment.revision)
    }

    #[must_use]
    pub fn definitionally_equivalent(&self, other: &Self) -> bool {
        self.schema.definitionally_equivalent(&other.schema)
            && self
                .environment
                .definitionally_equivalent(&other.environment)
    }

    pub fn validate(&self) -> Result<(), ContextError> {
        for dependency in self.schema.semantic_dependencies() {
            if !self.environment.has_module(dependency) {
                return Err(ContextError::MissingSemanticModule(dependency));
            }
        }
        Ok(())
    }
}
