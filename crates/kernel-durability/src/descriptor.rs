mod artifacts;
mod revision;

pub use artifacts::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableSemanticKeyPart, PHYSICAL_ARTIFACT_RECIPE_TAG,
};
pub use revision::DurableRevisionDescriptor;

pub(crate) use artifacts::canonical_physical_artifact_specs;
