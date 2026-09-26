use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use kernel_persistent::{PersistentOrdMap, PersistentVec as PersistentPhysicalVec};
use kernel_types::SemanticId;

use crate::physical_delta::PhysicalRelationDelta;
use crate::semantic_key::{
    semantic_key_dependencies_for_equivalences,
    semantic_key_structural_definitions_for_equivalences,
};
use crate::semantic_quotient_store::SemanticQuotientStoreView;
use crate::{
    ExecutionStats, LayoutBinding, PhysicalExecutionError, PhysicalRowId, RelQueryError,
    SemanticIndexBinding,
};

// HOSTILE[P191][ACTIVE]: persisted semantic-quotient/QCN physical state and maintenance live in a
// capability domain independent of both multiway planning and PhysicalStore representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SemanticQuotientEndpoint {
    pub(super) leaf: usize,
    pub(super) column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SemanticQuotientPhysicalLeaf {
    pub(super) relation: SemanticId,
    pub(super) layout: LayoutBinding,
    pub(super) width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SemanticQuotientSupportBinding {
    pub(super) leaves: Vec<SemanticQuotientPhysicalLeaf>,
    pub(super) specs: Vec<(SemanticId, Vec<SemanticQuotientEndpoint>)>,
}

include!("semantic_quotient_physical/stable_keys.rs");
include!("semantic_quotient_physical/support_state.rs");
include!("semantic_quotient_physical/bfc_maintenance.rs");
include!("semantic_quotient_physical/build.rs");
include!("semantic_quotient_physical/bfc.rs");
include!("semantic_quotient_physical/component_refresh.rs");
fn mask_ordinals(mask: &[u64]) -> impl Iterator<Item = usize> + '_ {
    mask.iter().enumerate().flat_map(|(word_index, word)| {
        let mut bits = *word;
        std::iter::from_fn(move || {
            if bits == 0 {
                return None;
            }
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            Some(word_index * 64 + bit)
        })
    })
}

include!("semantic_quotient_physical/support.rs");

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
