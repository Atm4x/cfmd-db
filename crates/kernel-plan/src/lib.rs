use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock},
};

use kernel_change::{
    RevisionEffectResidualChainCertificate, RevisionEffectResidualCubeLayerCertificate,
    RevisionEffectResidualMixedChainCertificate, RevisionEffectResidualNormalizedLayerCertificate,
    RevisionEffectResidualSquareChainCertificate, RevisionEffectResidualSquareLayerCertificate,
    RewriteLawSetId, RewriteResidualCubeCertificate, RewriteSpecId,
};
use kernel_durability::{
    DurabilityError, DurableArtifactCore, DurableCommitReceipt, DurableGenerationReceipt,
    DurableMaterializationSpec, DurableMigrationComplement, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableRelationMutation, DurableRelationResolution,
    DurableRelationRewriteIntent, DurableRevisionChange, DurableRevisionDescriptor,
    DurableRevisionStore, DurableSemanticKeyPart, DurableTransactionIntent,
    DurableTransactionOutcome, IdempotencyEpoch, RecoveryScan, RevisionDurability,
};
use kernel_identity::{DenseEntityIds, LocalEntityId};
use kernel_model::Value;
use kernel_persistent::{
    PersistentOrdMap, PersistentOrdSet, PersistentVec as PersistentPhysicalVec,
};
use kernel_query::{
    AggregateSpec, Impact, MaterializedRelPlanState, OrderDirection, PreparedRelationRewrite,
    RelExpr, RelObservationGuard, RelQueryError, RelType, RelationDelta, RelationValue,
    StorageResolvedRelationDelta,
};
pub use kernel_types::StableRowHandle as PhysicalRowId;
use kernel_types::{ClientTransactionId, EqClassId, RevisionId, RevisionObservableId, SemanticId};

pub mod advisor;
pub mod algebraic_native;

pub use advisor::{
    AdvisorTelemetry, ArtifactTelemetry, PhysicalCapability, PhysicalPressurePolicy,
    PhysicalPressureSample, PhysicalWorkEstimate, ResourceFootprint, ResourceFootprintError,
    TelemetryDecayPolicy, UnifiedAdvisorPolicy,
};

// HOSTILE inventory markers remain intentionally terse and grep-able while the mechanical
// source parts are promoted into real modules. Format: HOSTILE[pass][classification][:problem].
// ACTIVE = production authority/path; COMPAT = supported compatibility surface; RECOVERY =
// deliberate reconstruction boundary; RESIDUE = structural cleanup debt; PAYER = open hot-path
// cost. These markers are temporary scaffolding for the module-boundary cleanup after Pass167.

const _: () = assert!(
    kernel_semantic_index::KEY_ENCODING_REVISION
        == kernel_semantics::CANONICAL_EQ_KEY_ENCODING_VERSION as u64
);

fn saturating_usize_sum(values: impl Iterator<Item = usize>) -> usize {
    values.fold(0_usize, usize::saturating_add)
}

// Native storage and runtime data vocabulary remain root-owned so every promoted implementation
// module shares one stable type boundary. Behavior-heavy ownership lives in real submodules below.
include!("native_storage.rs");
mod native_relation;
mod physical_delta;
mod semantic_key;
mod semantic_quotient_physical;
mod semantic_quotient_store;
mod semantic_rows;
include!("physical_recovery_types.rs");
mod storage_impl;
#[doc(inline)]
pub use storage_impl::PhysicalStore;
pub use storage_impl::*;
mod runtime_impl;
// HOSTILE[P171][ACTIVE][CLEAN]: revision/commit/runtime protocol ownership now lives in
// runtime_impl/types.rs; this root re-export preserves the established crate API only.
pub use runtime_impl::{
    DerivedRelationRewriteTransitionRequest, DerivedRelationTransitionRequest,
    DurableMaterializationConfigOutcome, DurableRuntime, DurableRuntimeCheckpointError,
    DurableRuntimeCommitError, DurableRuntimeCommitOutcome, DurableRuntimeCommitReceipt,
    DurableRuntimeHistoricalError, DurableRuntimeSupervisor, FullRevisionTransitionRequest,
    PreparedCoherentResolutionTransition, PreparedRuntimeRevisionTransition,
    RepairCandidateProvider, RepairSearchOutcome, RepairSearchPolicy, RepairSearchReport,
    RevisionAndMaterializationsTransitionRequest, RevisionCommitChange, RevisionCommitDescriptor,
    RevisionRelationMutation, RevisionRelationRewrite, RevisionRewriteTransitionRequest,
    RevisionTransitionRequest, RuntimeInvariantClosureCertificate, RuntimeMaterializationSpec,
    RuntimeObservationGuard, RuntimePublicationEffect, RuntimeRecoveryError,
    RuntimeRepairCandidate, RuntimeRepairObservationTransport, RuntimeRepairRelationMutation,
    RuntimeRevisionBundle, RuntimeRevisionCell, RuntimeRevisionSnapshot, RuntimeRewriteIntent,
    RuntimeRootVersion, RuntimeViolationState, SealedRuntimeRevisionTransition,
};

mod recovery;
pub use recovery::*;
mod plan;
pub use plan::*;
mod execution;
mod filter_shape;
mod join_access;
mod join_shape;
mod multiway;
mod preparation;
pub use preparation::*;

#[cfg(test)]
mod tests;

mod positive_recursive;
pub use positive_recursive::PreparedPositiveRecursivePlan;
