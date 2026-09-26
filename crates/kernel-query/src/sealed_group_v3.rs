//! R&D V3: pre-publication sealing for Group state patches.
//!
//! This module deliberately lives beside the production implementation in the
//! disposable Pass103 R&D workspace.  It proves that all semantic/index/shape
//! failure discovery can happen before publication, leaving a structural
//! commit that does not return a recoverable error.

use super::{
    BTreeMap, BTreeSet, GenericGroupPatch, GroupDeltaPatch, I64CountGroupPatch,
    MaintainedGroupBucket, MaterializedGroupDeltaState, RelQueryError, Row, Value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SealedLookupKey {
    I64(i64),
    Semantic(Vec<kernel_semantics::CanonicalEqKey>),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SealedRemoval {
    index: usize,
    removed_lookup: SealedLookupKey,
    moved_lookup: Option<SealedLookupKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SealedMove {
    index: usize,
    old_lookup: SealedLookupKey,
    new_lookup: SealedLookupKey,
    bucket: MaintainedGroupBucket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SealedDenseAction {
    Untouched,
    Drop,
    Assign(Vec<(usize, kernel_aggregate::ExactCount)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SealedGroupPatch {
    moves: Vec<SealedMove>,
    replacements: Vec<(usize, MaintainedGroupBucket)>,
    removals: Vec<SealedRemoval>,
    insertions: Vec<(MaintainedGroupBucket, SealedLookupKey)>,
    dense: SealedDenseAction,
}

fn lookup_key_for(
    state: &MaterializedGroupDeltaState,
    key: &Row,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<SealedLookupKey, RelQueryError> {
    if state.i64_lookup.is_some() {
        let [Value::I64(value)] = key.as_slice() else {
            return Err(RelQueryError::TypeMismatch);
        };
        return Ok(SealedLookupKey::I64(*value));
    }
    if state.semantic_lookup.is_some() {
        let canonical = state.canonical_group_key(key, registry)?;
        return Ok(SealedLookupKey::Semantic(canonical));
    }
    Ok(SealedLookupKey::None)
}

fn seal_removals(
    state: &MaterializedGroupDeltaState,
    mut indices: Vec<usize>,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<SealedRemoval>, RelQueryError> {
    // Descending indices make swap-remove relocation deterministic.  The small
    // virtual source map tracks which original bucket occupies a slot after
    // earlier removals without cloning the whole group vector.
    indices.sort_unstable_by(|left, right| right.cmp(left));
    if indices.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    let mut current_len = state.groups.len();
    let mut virtual_sources = BTreeMap::<usize, usize>::new();
    let mut sealed = Vec::with_capacity(indices.len());
    for index in indices {
        if index >= current_len {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let source = virtual_sources.get(&index).copied().unwrap_or(index);
        let removed_lookup = lookup_key_for(state, &state.groups[source].key, registry)?;
        let tail = current_len - 1;
        let tail_source = virtual_sources.get(&tail).copied().unwrap_or(tail);
        let moved_lookup = if index == tail {
            None
        } else {
            let key = lookup_key_for(state, &state.groups[tail_source].key, registry)?;
            virtual_sources.insert(index, tail_source);
            Some(key)
        };
        virtual_sources.remove(&tail);
        sealed.push(SealedRemoval {
            index,
            removed_lookup,
            moved_lookup,
        });
        current_len -= 1;
    }
    Ok(sealed)
}

#[allow(clippy::too_many_lines)]
fn seal_i64_group_patch(
    state: &MaterializedGroupDeltaState,
    patch: I64CountGroupPatch,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<SealedGroupPatch, RelQueryError> {
    if !state.fast_i64_count || state.i64_lookup.is_none() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    if !patch.retain_dense && patch.dense_move.is_some() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }

    let dense = if patch.retain_dense {
        let dense = state
            .dense_i64_count
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut assignments = Vec::with_capacity(patch.changes.len());
        for (key, count) in &patch.changes {
            let index = dense
                .index(*key)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let dense_present = !dense.counts[index].is_zero();
            let lookup_present = state
                .i64_lookup
                .as_ref()
                .is_some_and(|lookup| lookup.contains_key(key));
            if dense_present != lookup_present {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            assignments.push((index, count.clone()));
        }
        SealedDenseAction::Assign(assignments)
    } else {
        SealedDenseAction::Drop
    };

    let lookup = state
        .i64_lookup
        .as_ref()
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
    if let Some((removed, inserted)) = patch.dense_move {
        let index = lookup
            .get(&removed)
            .copied()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if lookup.contains_key(&inserted) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let inserted_count = patch
            .changes
            .iter()
            .find(|(key, _)| *key == inserted)
            .map(|(_, count)| count.clone())
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let [Value::I64(actual_key)] = state.groups[index].key.as_slice() else {
            return Err(RelQueryError::TypeMismatch);
        };
        if *actual_key != removed || !state.groups[index].count.is_one() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let mut bucket = state.groups[index].clone();
        bucket.key = vec![Value::I64(inserted)];
        bucket.count = inserted_count;
        return Ok(SealedGroupPatch {
            moves: vec![SealedMove {
                index,
                old_lookup: SealedLookupKey::I64(removed),
                new_lookup: SealedLookupKey::I64(inserted),
                bucket,
            }],
            replacements: Vec::new(),
            removals: Vec::new(),
            insertions: Vec::new(),
            dense,
        });
    }
    let mut replacements = Vec::new();
    let mut removal_indices = Vec::new();
    let mut insertions = Vec::new();
    let mut touched_indices = BTreeSet::new();

    for (key, count) in patch.changes {
        match (lookup.get(&key).copied(), count.is_zero()) {
            (Some(index), false) => {
                if !touched_indices.insert(index) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                let [Value::I64(actual_key)] = state.groups[index].key.as_slice() else {
                    return Err(RelQueryError::TypeMismatch);
                };
                if *actual_key != key {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                let mut bucket = state.groups[index].clone();
                bucket.count = count;
                replacements.push((index, bucket));
            }
            (Some(index), true) => {
                if !touched_indices.insert(index) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                removal_indices.push(index);
            }
            (None, false) => {
                let mut bucket = MaterializedGroupDeltaState::empty_bucket(vec![Value::I64(key)]);
                bucket.count = count;
                insertions.push((bucket, SealedLookupKey::I64(key)));
            }
            (None, true) => {}
        }
    }

    let removals = seal_removals(state, removal_indices, registry)?;
    Ok(SealedGroupPatch {
        moves: Vec::new(),
        replacements,
        removals,
        insertions,
        dense,
    })
}

fn seal_generic_group_patch(
    state: &MaterializedGroupDeltaState,
    patch: GenericGroupPatch,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<SealedGroupPatch, RelQueryError> {
    let mut replacements = Vec::new();
    let mut removal_indices = Vec::new();
    let mut insertions = Vec::new();
    let mut touched_indices = BTreeSet::new();

    for (key, next_bucket) in patch.planned {
        let current = state.find_group(&key, context, registry)?;
        match (current, next_bucket) {
            (Some(index), Some(bucket)) => {
                if !touched_indices.insert(index) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                replacements.push((index, bucket));
            }
            (Some(index), None) => {
                if !touched_indices.insert(index) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                removal_indices.push(index);
            }
            (None, Some(bucket)) => {
                let lookup = lookup_key_for(state, &bucket.key, registry)?;
                insertions.push((bucket, lookup));
            }
            (None, None) => {}
        }
    }

    let removals = seal_removals(state, removal_indices, registry)?;
    let final_len = state.groups.len() - removals.len() + insertions.len();
    if state.group_columns.is_empty() && final_len == 0 {
        let bucket = MaterializedGroupDeltaState::empty_bucket(Vec::new());
        let lookup = lookup_key_for(state, &bucket.key, registry)?;
        insertions.push((bucket, lookup));
    }

    Ok(SealedGroupPatch {
        moves: Vec::new(),
        replacements,
        removals,
        insertions,
        dense: SealedDenseAction::Untouched,
    })
}

pub(super) fn seal_group_patch(
    state: &MaterializedGroupDeltaState,
    patch: GroupDeltaPatch,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<SealedGroupPatch, RelQueryError> {
    match patch {
        GroupDeltaPatch::I64Count(patch) => seal_i64_group_patch(state, patch, registry),
        GroupDeltaPatch::Generic(patch) => {
            seal_generic_group_patch(state, patch, context, registry)
        }
    }
}

fn remove_lookup_total(state: &mut MaterializedGroupDeltaState, key: &SealedLookupKey) {
    match key {
        SealedLookupKey::I64(key) => {
            let lookup = state
                .i64_lookup
                .as_mut()
                .expect("sealed I64 lookup backend");
            let removed = lookup.remove(key);
            debug_assert!(removed.is_some());
        }
        SealedLookupKey::Semantic(key) => {
            let lookup = state
                .semantic_lookup
                .as_mut()
                .expect("sealed semantic lookup backend");
            let removed = lookup.remove(key);
            debug_assert!(removed.is_some());
        }
        SealedLookupKey::None => {}
    }
}

fn insert_lookup_total(
    state: &mut MaterializedGroupDeltaState,
    key: &SealedLookupKey,
    index: usize,
) {
    match key {
        SealedLookupKey::I64(key) => {
            state
                .i64_lookup
                .as_mut()
                .expect("sealed I64 lookup backend")
                .insert(*key, index);
        }
        SealedLookupKey::Semantic(key) => {
            state
                .semantic_lookup
                .as_mut()
                .expect("sealed semantic lookup backend")
                .insert(key.clone(), index);
        }
        SealedLookupKey::None => {}
    }
}

/// Total over the exact state generation against which the patch was sealed.
/// Every recoverable semantic/index/shape failure belongs to `seal_group_patch`.
pub(super) fn commit_sealed_group_patch(
    state: &mut MaterializedGroupDeltaState,
    patch: SealedGroupPatch,
) {
    for movement in patch.moves {
        remove_lookup_total(state, &movement.old_lookup);
        state.groups[movement.index] = movement.bucket;
        insert_lookup_total(state, &movement.new_lookup, movement.index);
    }
    for (index, bucket) in patch.replacements {
        state.groups[index] = bucket;
    }

    match patch.dense {
        SealedDenseAction::Untouched => {}
        SealedDenseAction::Drop => state.dense_i64_count = None,
        SealedDenseAction::Assign(assignments) => {
            let dense = state
                .dense_i64_count
                .as_mut()
                .expect("sealed dense backend");
            for (index, count) in assignments {
                dense.counts[index] = count;
            }
        }
    }

    for removal in patch.removals {
        debug_assert!(removal.index < state.groups.len());
        remove_lookup_total(state, &removal.removed_lookup);
        let last = state.groups.len() - 1;
        state.groups.swap_remove(removal.index);
        if removal.index == last {
            debug_assert!(removal.moved_lookup.is_none());
        } else {
            insert_lookup_total(
                state,
                removal
                    .moved_lookup
                    .as_ref()
                    .expect("sealed moved lookup key"),
                removal.index,
            );
        }
    }

    for (bucket, lookup_key) in patch.insertions {
        let index = state.groups.len();
        insert_lookup_total(state, &lookup_key, index);
        state.groups.push(bucket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AggregateSpec, CompactDelta, RelExpr};
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    #[test]
    fn sealed_i64_outlier_roundtrip_matches_legacy_commit() {
        let relation = SemanticId::new(390_001);
        let eq = SemanticId::new(390_002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(390_001));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(390_001));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: eq,
            },
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, (0..8).map(|i| vec![Value::I64(i)]).collect());
        let original = MaterializedGroupDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();

        let forward = CompactDelta::replace(vec![Value::I64(0)], vec![Value::I64(10_000)]);
        let planned = original
            .plan_delta_view(&forward, &context, &registry)
            .unwrap();
        let mut legacy = original.clone();
        legacy
            .commit_group_patch(planned.patch.clone(), &context, &registry)
            .unwrap();
        let sealed = seal_group_patch(&original, planned.patch, &context, &registry).unwrap();
        let mut total = original.clone();
        commit_sealed_group_patch(&mut total, sealed);
        assert_eq!(total, legacy, "sealed forward state differs from legacy");

        let backward = CompactDelta::replace(vec![Value::I64(10_000)], vec![Value::I64(0)]);
        let planned = total
            .plan_delta_view(&backward, &context, &registry)
            .unwrap();
        let mut legacy_back = total.clone();
        legacy_back
            .commit_group_patch(planned.patch.clone(), &context, &registry)
            .unwrap();
        let sealed = seal_group_patch(&total, planned.patch, &context, &registry).unwrap();
        commit_sealed_group_patch(&mut total, sealed);
        assert_eq!(
            total, legacy_back,
            "sealed backward state differs from legacy"
        );
    }
}
