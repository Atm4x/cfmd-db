use std::collections::BTreeMap;

use kernel_types::{MaterializationId, RevisionId, SemanticId};

pub const PHYSICAL_ARTIFACT_RECIPE_TAG: u16 = 0xCF52;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableMaterializationSpec {
    pub id: MaterializationId,
    pub query: kernel_query::RelExpr,
}

/// Layout-independent recipe for reconstructible physical state retained across
/// checkpoint/reopen. Payload row handles are intentionally not durable; recovery
/// rebuilds each artifact against the fresh runtime layout from authoritative data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurableSemanticKeyPart {
    pub column: usize,
    pub equivalence: SemanticId,
}

/// Reconstructible native representation for one active relation layout.
///
/// This is intentionally a logical lowering recipe, not a serialized physical
/// payload: row handles, dense local ids and allocator representation are rebuilt
/// against the recovered authoritative Revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurableRelationLayoutKind {
    RowStore,
    ValueColumnar,
    I64Columnar,
    TypedColumnar,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurablePhysicalArtifactSpec {
    RelationLayout {
        relation: SemanticId,
        layout_id: u128,
        kind: DurableRelationLayoutKind,
    },
    I64Index {
        relation: SemanticId,
        key_column: usize,
        equivalence: SemanticId,
        advisor_managed: bool,
    },
    SemanticQuotientFactor {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
    SemanticStatistics {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
    ObservableAtom {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
}

/// Optional reconstructible semantic payload persisted beside a physical-artifact recipe.
///
/// Cores are never semantic authority. They are generation-local acceleration data and contain
/// no revision/process-local physical row handles. A stale or incompatible core is discarded and
/// the recipe falls back to a full exact rebuild.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurableArtifactCore {
    ObservableAtom {
        source_revision: RevisionId,
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        /// One canonical-key tuple per durable relation occurrence ordinal.
        encoded_keys_by_ordinal: Vec<Vec<u8>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DurablePhysicalArtifactKey {
    RelationLayout {
        relation: SemanticId,
        layout_id: u128,
        kind: DurableRelationLayoutKind,
    },
    I64Index {
        relation: SemanticId,
        key_column: usize,
        equivalence: SemanticId,
    },
    QuotientFactor {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
    Statistics {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
    ObservableAtom {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
}

fn physical_artifact_key(spec: &DurablePhysicalArtifactSpec) -> (DurablePhysicalArtifactKey, bool) {
    match spec {
        DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id,
            kind,
        } => (
            DurablePhysicalArtifactKey::RelationLayout {
                relation: *relation,
                layout_id: *layout_id,
                kind: *kind,
            },
            false,
        ),
        DurablePhysicalArtifactSpec::I64Index {
            relation,
            key_column,
            equivalence,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::I64Index {
                relation: *relation,
                key_column: *key_column,
                equivalence: *equivalence,
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::QuotientFactor {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::SemanticStatistics {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::Statistics {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::ObservableAtom {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::ObservableAtom {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
    }
}

fn physical_artifact_spec_from_key(
    key: DurablePhysicalArtifactKey,
    advisor_managed: bool,
) -> DurablePhysicalArtifactSpec {
    match key {
        DurablePhysicalArtifactKey::RelationLayout {
            relation,
            layout_id,
            kind,
        } => DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id,
            kind,
        },
        DurablePhysicalArtifactKey::I64Index {
            relation,
            key_column,
            equivalence,
        } => DurablePhysicalArtifactSpec::I64Index {
            relation,
            key_column,
            equivalence,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::QuotientFactor {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            relation,
            key_parts,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::Statistics {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::SemanticStatistics {
            relation,
            key_parts,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::ObservableAtom {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::ObservableAtom {
            relation,
            key_parts,
            advisor_managed,
        },
    }
}

pub(crate) fn canonical_physical_artifact_specs(
    specs: &[DurablePhysicalArtifactSpec],
) -> Vec<DurablePhysicalArtifactSpec> {
    let mut ownership = BTreeMap::<DurablePhysicalArtifactKey, bool>::new();
    for spec in specs {
        let (key, advisor_managed) = physical_artifact_key(spec);
        ownership
            .entry(key)
            .and_modify(|managed| *managed &= advisor_managed)
            .or_insert(advisor_managed);
    }
    ownership
        .into_iter()
        .map(|(key, advisor_managed)| physical_artifact_spec_from_key(key, advisor_managed))
        .collect()
}
