use kernel_persistent::{PersistentOrdMap, PersistentOrdMapIter};
use kernel_schema::{ModuleDigest, SemanticContext, StructuralEquivalenceDef};
use kernel_types::{SemanticId, SemanticRevision};

pub const KEY_ENCODING_REVISION: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticIndexCompatibility {
    Compatible,
    RebuildSemanticRevision,
    RebuildStructuralDefinitions,
    RebuildDependencies,
    RebuildKeyEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticModuleBinding {
    pub semantic_id: SemanticId,
    pub module_digest: ModuleDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralEquivalenceBinding {
    pub semantic_id: SemanticId,
    pub definition: StructuralEquivalenceDef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticIndexBinding {
    semantic_revision: SemanticRevision,
    structural_definitions: Vec<StructuralEquivalenceBinding>,
    dependencies: Vec<SemanticModuleBinding>,
    key_encoding_revision: u64,
}

impl SemanticIndexBinding {
    #[must_use]
    pub fn new(context: &SemanticContext, dependencies: Vec<SemanticModuleBinding>) -> Self {
        Self::new_with_structural_definitions(context, dependencies, Vec::new())
    }

    #[must_use]
    pub fn new_with_structural_definitions(
        context: &SemanticContext,
        mut dependencies: Vec<SemanticModuleBinding>,
        mut structural_definitions: Vec<StructuralEquivalenceBinding>,
    ) -> Self {
        dependencies.sort_unstable();
        dependencies.dedup();
        structural_definitions.sort_by_key(|binding| binding.semantic_id);
        Self {
            semantic_revision: context.revision(),
            structural_definitions,
            dependencies,
            key_encoding_revision: KEY_ENCODING_REVISION,
        }
    }

    /// Reconstructs a binding descriptor read from persistent metadata.
    ///
    /// Callers must run [`Self::compatibility`] before reusing the associated
    /// derived cache. Historical key revisions are represented explicitly so
    /// they fail closed into rebuild rather than being silently reinterpreted.
    #[must_use]
    pub fn from_persisted(
        semantic_revision: SemanticRevision,
        mut dependencies: Vec<SemanticModuleBinding>,
        key_encoding_revision: u64,
    ) -> Self {
        dependencies.sort_unstable();
        dependencies.dedup();
        Self {
            semantic_revision,
            structural_definitions: Vec::new(),
            dependencies,
            key_encoding_revision,
        }
    }

    /// Reconstructs cache metadata that contains the exact structural law
    /// closure used by the persisted canonical keys.
    #[must_use]
    pub fn from_persisted_with_structural_definitions(
        semantic_revision: SemanticRevision,
        mut dependencies: Vec<SemanticModuleBinding>,
        mut structural_definitions: Vec<StructuralEquivalenceBinding>,
        key_encoding_revision: u64,
    ) -> Self {
        dependencies.sort_unstable();
        dependencies.dedup();
        structural_definitions.sort_by_key(|binding| binding.semantic_id);
        Self {
            semantic_revision,
            structural_definitions,
            dependencies,
            key_encoding_revision,
        }
    }

    #[must_use]
    pub const fn semantic_revision(&self) -> SemanticRevision {
        self.semantic_revision
    }

    #[must_use]
    pub fn dependencies(&self) -> &[SemanticModuleBinding] {
        &self.dependencies
    }

    #[must_use]
    pub fn structural_definitions(&self) -> &[StructuralEquivalenceBinding] {
        &self.structural_definitions
    }

    #[must_use]
    pub const fn key_encoding_revision(&self) -> u64 {
        self.key_encoding_revision
    }

    #[must_use]
    pub fn is_valid_for(
        &self,
        context: &SemanticContext,
        dependencies: &[SemanticModuleBinding],
    ) -> bool {
        self.compatibility(context, dependencies) == SemanticIndexCompatibility::Compatible
    }

    #[must_use]
    pub fn compatibility(
        &self,
        context: &SemanticContext,
        dependencies: &[SemanticModuleBinding],
    ) -> SemanticIndexCompatibility {
        if self.key_encoding_revision != KEY_ENCODING_REVISION {
            return SemanticIndexCompatibility::RebuildKeyEncoding;
        }
        if self.semantic_revision != context.revision() {
            return SemanticIndexCompatibility::RebuildSemanticRevision;
        }
        if self.structural_definitions.iter().any(|binding| {
            context.schema.structural_equivalence(binding.semantic_id) != Some(&binding.definition)
        }) {
            return SemanticIndexCompatibility::RebuildStructuralDefinitions;
        }
        let mut dependencies = dependencies.to_vec();
        dependencies.sort_unstable();
        dependencies.dedup();
        if self.dependencies != dependencies {
            return SemanticIndexCompatibility::RebuildDependencies;
        }
        SemanticIndexCompatibility::Compatible
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBucket<I: Clone> {
    entries: PersistentOrdMap<u64, I>,
}

impl<I: Clone> Default for SemanticBucket<I> {
    fn default() -> Self {
        Self {
            entries: PersistentOrdMap::default(),
        }
    }
}

impl<I: Clone> SemanticBucket<I> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &I> {
        self.entries.values()
    }
}

impl<'a, I: Clone> IntoIterator for &'a SemanticBucket<I> {
    type Item = &'a I;
    type IntoIter = std::iter::Map<PersistentOrdMapIter<'a, u64, I>, fn((&'a u64, &'a I)) -> &'a I>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(bucket_value::<I>)
    }
}

fn bucket_value<'a, I>((_, value): (&'a u64, &'a I)) -> &'a I {
    value
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticReverseEntry<K> {
    key: K,
    ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBucketIndex<K: Ord + Clone, I: Ord + Clone> {
    binding: SemanticIndexBinding,
    buckets: PersistentOrdMap<K, SemanticBucket<I>>,
    reverse: PersistentOrdMap<I, SemanticReverseEntry<K>>,
}

impl<K, I> SemanticBucketIndex<K, I>
where
    K: Ord + Clone,
    I: Ord + Clone,
{
    #[must_use]
    pub fn new(binding: SemanticIndexBinding) -> Self {
        Self {
            binding,
            buckets: PersistentOrdMap::default(),
            reverse: PersistentOrdMap::default(),
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &SemanticIndexBinding {
        &self.binding
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.reverse.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reverse.is_empty()
    }

    #[must_use]
    pub fn distinct_key_count(&self) -> usize {
        self.buckets.len()
    }

    #[must_use]
    pub fn bucket(&self, key: &K) -> Option<&SemanticBucket<I>> {
        self.buckets.get(key)
    }

    #[must_use]
    pub fn buckets(&self) -> impl DoubleEndedIterator<Item = (&K, &SemanticBucket<I>)> {
        self.buckets.iter()
    }

    #[must_use]
    pub fn key_for(&self, identity: &I) -> Option<&K> {
        self.reverse.get(identity).map(|entry| &entry.key)
    }

    /// Deterministic retained-size estimate for budget decisions.
    ///
    /// This accounts for owned key/identity payloads, including the reverse-map
    /// key clone and ordered occurrence payloads. Allocator/BTree node overhead is
    /// intentionally outside the contract and must not be confused with RSS.
    #[must_use]
    pub fn estimated_retained_bytes(&self, key_heap_bytes: impl Fn(&K) -> usize) -> usize {
        let mut bytes = std::mem::size_of::<Self>().saturating_add(
            self.binding
                .dependencies
                .capacity()
                .saturating_mul(std::mem::size_of::<SemanticModuleBinding>()),
        );
        bytes = bytes.saturating_add(
            self.binding
                .structural_definitions
                .capacity()
                .saturating_mul(std::mem::size_of::<StructuralEquivalenceBinding>()),
        );
        for binding in &self.binding.structural_definitions {
            let entries = match &binding.definition {
                StructuralEquivalenceDef::Product { fields } => fields.len(),
                StructuralEquivalenceDef::Sum { variants } => variants.len(),
                StructuralEquivalenceDef::Mu { .. }
                | StructuralEquivalenceDef::Var { .. }
                | StructuralEquivalenceDef::Option { .. }
                | StructuralEquivalenceDef::Set { .. }
                | StructuralEquivalenceDef::Bag { .. }
                | StructuralEquivalenceDef::Seq { .. }
                | StructuralEquivalenceDef::Map { .. } => 0,
            };
            bytes = bytes.saturating_add(
                entries.saturating_mul(std::mem::size_of::<(SemanticId, SemanticId)>()),
            );
        }
        for (key, bucket) in &self.buckets {
            bytes = bytes
                .saturating_add(std::mem::size_of::<K>())
                .saturating_add(key_heap_bytes(key))
                .saturating_add(std::mem::size_of::<SemanticBucket<I>>())
                .saturating_add(bucket.len().saturating_mul(std::mem::size_of::<(u64, I)>()));
        }
        for entry in self.reverse.values() {
            bytes = bytes
                .saturating_add(std::mem::size_of::<I>())
                .saturating_add(std::mem::size_of::<SemanticReverseEntry<K>>())
                .saturating_add(key_heap_bytes(&entry.key));
        }
        bytes
    }

    /// Inserts or rebinds one derivative identity, returning its previous key if present.
    pub fn insert(&mut self, identity: I, key: K) -> Option<K> {
        if self
            .reverse
            .get(&identity)
            .is_some_and(|current| current.key == key)
        {
            return Some(key);
        }

        let previous = self.reverse.remove(&identity);
        if let Some(previous_entry) = &previous {
            self.remove_from_bucket(&previous_entry.key, previous_entry.ordinal);
        }

        let ordinal = self.next_ordinal(&key);
        let mut bucket = self.buckets.get(&key).cloned().unwrap_or_default();
        bucket.entries.insert(ordinal, identity.clone());
        self.buckets.insert(key.clone(), bucket);
        self.reverse
            .insert(identity, SemanticReverseEntry { key, ordinal });
        previous.map(|entry| entry.key)
    }

    pub fn remove(&mut self, identity: &I) -> Option<K> {
        let entry = self.reverse.remove(identity)?;
        self.remove_from_bucket(&entry.key, entry.ordinal);
        Some(entry.key)
    }

    fn next_ordinal(&mut self, key: &K) -> u64 {
        let Some(bucket) = self.buckets.get(key) else {
            return 0;
        };
        let Some((&last, _)) = bucket.entries.iter().next_back() else {
            return 0;
        };
        if let Some(next) = last.checked_add(1) {
            return next;
        }

        let identities = bucket.entries.values().cloned().collect::<Vec<_>>();
        let mut bucket = SemanticBucket::default();
        for (ordinal, identity) in identities.into_iter().enumerate() {
            let ordinal = u64::try_from(ordinal)
                .expect("live bucket cardinality cannot exceed u64 address space");
            bucket.entries.insert(ordinal, identity.clone());
            if let Some(mut reverse) = self.reverse.get(&identity).cloned() {
                reverse.ordinal = ordinal;
                self.reverse.insert(identity, reverse);
            }
        }
        let next = u64::try_from(bucket.len())
            .expect("live bucket cardinality cannot exceed u64 address space");
        self.buckets.insert(key.clone(), bucket);
        next
    }

    fn remove_from_bucket(&mut self, key: &K, ordinal: u64) {
        let Some(mut bucket) = self.buckets.get(key).cloned() else {
            return;
        };
        bucket.entries.remove(&ordinal);
        if bucket.is_empty() {
            self.buckets.remove(key);
        } else {
            self.buckets.insert(key.clone(), bucket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_schema::{Schema, SemanticEnvironment};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    fn context() -> SemanticContext {
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(7)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(11)),
        }
    }

    #[test]
    fn duplicate_bucket_membership_and_reverse_map_are_exact() {
        let context = context();
        let dependency = SemanticModuleBinding {
            semantic_id: SemanticId::new(3),
            module_digest: ModuleDigest([9; 32]),
        };
        let mut index =
            SemanticBucketIndex::new(SemanticIndexBinding::new(&context, vec![dependency]));
        assert_eq!(index.insert(10_u64, "a"), None);
        assert_eq!(index.insert(11_u64, "a"), None);
        assert_eq!(index.bucket(&"a").map(SemanticBucket::len), Some(2));
        assert_eq!(index.key_for(&10), Some(&"a"));
        assert_eq!(index.remove(&10), Some("a"));
        assert_eq!(index.bucket(&"a").map(SemanticBucket::len), Some(1));
        assert_eq!(index.remove(&11), Some("a"));
        assert!(index.bucket(&"a").is_none());
    }

    #[test]
    fn rebinding_identity_removes_dead_bucket() {
        let context = context();
        let mut index = SemanticBucketIndex::new(SemanticIndexBinding::new(&context, Vec::new()));
        assert_eq!(index.insert(1_u64, 10_i64), None);
        assert_eq!(index.insert(1_u64, 20_i64), Some(10));
        assert!(index.bucket(&10).is_none());
        assert_eq!(index.bucket(&20).map(SemanticBucket::len), Some(1));
    }

    #[test]
    fn bucket_preserves_identity_insertion_order_and_same_key_reinsert_is_stable() {
        let context = context();
        let mut index = SemanticBucketIndex::new(SemanticIndexBinding::new(&context, Vec::new()));
        assert_eq!(index.insert(7_u64, "a"), None);
        assert_eq!(index.insert(2_u64, "a"), None);
        assert_eq!(index.insert(5_u64, "a"), None);
        assert_eq!(
            index
                .bucket(&"a")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![7_u64, 2, 5])
        );
        assert_eq!(index.insert(2_u64, "a"), Some("a"));
        assert_eq!(
            index
                .bucket(&"a")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![7_u64, 2, 5])
        );
    }

    #[test]
    fn ordered_bucket_delete_rebind_and_reinsert_preserve_insertion_order() {
        let context = context();
        let mut index = SemanticBucketIndex::new(SemanticIndexBinding::new(&context, Vec::new()));
        for identity in [7_u64, 2, 5, 11] {
            assert_eq!(index.insert(identity, "a"), None);
        }
        assert_eq!(index.remove(&2), Some("a"));
        assert_eq!(
            index
                .bucket(&"a")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![7, 5, 11])
        );
        assert_eq!(index.insert(2, "a"), None);
        assert_eq!(
            index
                .bucket(&"a")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![7, 5, 11, 2])
        );
        assert_eq!(index.insert(5, "b"), Some("a"));
        assert_eq!(
            index
                .bucket(&"a")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![7, 11, 2])
        );
        assert_eq!(
            index
                .bucket(&"b")
                .map(|bucket| bucket.iter().copied().collect::<Vec<_>>()),
            Some(vec![5])
        );
    }

    #[test]
    fn snapshot_mutation_path_copies_only_touched_semantic_bucket_state() {
        let context = context();
        let mut index = SemanticBucketIndex::new(SemanticIndexBinding::new(&context, Vec::new()));
        for key in 0_i64..16_384 {
            assert_eq!(index.insert(key.cast_unsigned(), key), None);
        }

        let snapshot = index.clone();
        assert!(index.buckets.shares_root_with(&snapshot.buckets));
        assert!(index.reverse.shares_root_with(&snapshot.reverse));

        assert_eq!(index.insert(20_000_u64, 20_000_i64), None);
        assert!(!index.buckets.shares_root_with(&snapshot.buckets));
        assert!(!index.reverse.shares_root_with(&snapshot.reverse));
        assert!(snapshot.bucket(&20_000).is_none());
        assert_eq!(index.bucket(&20_000).map(SemanticBucket::len), Some(1));

        let untouched_snapshot = snapshot.bucket(&8_192).unwrap();
        let untouched_next = index.bucket(&8_192).unwrap();
        assert!(
            untouched_snapshot
                .entries
                .shares_root_with(&untouched_next.entries),
            "novel-key insertion must not clone unrelated semantic bucket contents"
        );

        let before_existing = index.clone();
        assert_eq!(index.insert(30_000_u64, 8_192_i64), None);
        let old_bucket = before_existing.bucket(&8_192).unwrap();
        let new_bucket = index.bucket(&8_192).unwrap();
        assert!(!old_bucket.entries.shares_root_with(&new_bucket.entries));
        assert_eq!(old_bucket.len(), 1);
        assert_eq!(new_bucket.len(), 2);
        assert!(
            before_existing
                .bucket(&8_193)
                .unwrap()
                .entries
                .shares_root_with(&index.bucket(&8_193).unwrap().entries),
            "existing-key mutation must keep neighboring bucket contents shared"
        );
    }

    #[test]
    fn binding_rejects_revision_digest_and_encoding_mismatch() {
        let mut context = context();
        let dependency = SemanticModuleBinding {
            semantic_id: SemanticId::new(3),
            module_digest: ModuleDigest([9; 32]),
        };
        let binding = SemanticIndexBinding::new(&context, vec![dependency]);
        assert!(binding.is_valid_for(&context, &[dependency]));
        assert!(!binding.is_valid_for(
            &context,
            &[SemanticModuleBinding {
                semantic_id: dependency.semantic_id,
                module_digest: ModuleDigest([8; 32]),
            }]
        ));
        context.environment.revision = SemanticEnvId::new(12);
        assert!(!binding.is_valid_for(&context, &[dependency]));
    }

    #[test]
    fn retained_size_estimate_accounts_for_bucket_and_reverse_payloads() {
        let context = context();
        let mut index = SemanticBucketIndex::new(SemanticIndexBinding::new(&context, Vec::new()));
        let empty = index.estimated_retained_bytes(String::capacity);
        index.insert(1_u64, "alpha".to_owned());
        let one = index.estimated_retained_bytes(String::capacity);
        index.insert(2_u64, "alpha".to_owned());
        let two = index.estimated_retained_bytes(String::capacity);
        index.insert(3_u64, "a much larger retained semantic key".to_owned());
        let three = index.estimated_retained_bytes(String::capacity);
        assert!(empty < one && one < two && two < three);
    }

    #[test]
    fn compatibility_reports_exact_rebuild_reason() {
        let context = context();
        let dependency = SemanticModuleBinding {
            semantic_id: SemanticId::new(3),
            module_digest: ModuleDigest([9; 32]),
        };
        let binding = SemanticIndexBinding::new(&context, vec![dependency]);
        assert_eq!(
            binding.compatibility(&context, &[dependency]),
            SemanticIndexCompatibility::Compatible
        );

        let next_context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(8)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(11)),
        };
        assert_eq!(
            binding.compatibility(&next_context, &[dependency]),
            SemanticIndexCompatibility::RebuildSemanticRevision
        );
        assert_eq!(
            binding.compatibility(
                &context,
                &[SemanticModuleBinding {
                    semantic_id: dependency.semantic_id,
                    module_digest: ModuleDigest([8; 32]),
                }]
            ),
            SemanticIndexCompatibility::RebuildDependencies
        );
        assert_eq!(
            SemanticIndexBinding::from_persisted(
                context.revision(),
                vec![dependency],
                KEY_ENCODING_REVISION + 1,
            )
            .compatibility(&context, &[dependency]),
            SemanticIndexCompatibility::RebuildKeyEncoding
        );
        assert_eq!(
            SemanticIndexBinding::from_persisted(
                context.revision(),
                vec![dependency],
                KEY_ENCODING_REVISION - 1,
            )
            .compatibility(&context, &[dependency]),
            SemanticIndexCompatibility::RebuildKeyEncoding
        );
    }

    #[test]
    fn binding_rejects_same_nominal_revision_with_changed_structural_definition() {
        let root = SemanticId::new(100);
        let field_a = SemanticId::new(101);
        let field_b = SemanticId::new(102);
        let eq_a = SemanticId::new(103);
        let eq_b = SemanticId::new(104);
        let original = StructuralEquivalenceDef::Product {
            fields: [(field_a, eq_a), (field_b, eq_b)].into_iter().collect(),
        };
        let mut context = context();
        context
            .schema
            .define_structural_equivalence(root, original.clone())
            .unwrap();
        let binding = SemanticIndexBinding::new_with_structural_definitions(
            &context,
            Vec::new(),
            vec![StructuralEquivalenceBinding {
                semantic_id: root,
                definition: original,
            }],
        );
        assert_eq!(
            binding.compatibility(&context, &[]),
            SemanticIndexCompatibility::Compatible
        );

        let mut drifted = SemanticContext {
            schema: Schema::new(context.schema.revision),
            environment: context.environment.clone(),
        };
        drifted
            .schema
            .define_structural_equivalence(
                root,
                StructuralEquivalenceDef::Product {
                    fields: [(field_a, eq_b), (field_b, eq_a)].into_iter().collect(),
                },
            )
            .unwrap();
        assert_eq!(context.revision(), drifted.revision());
        assert_eq!(
            binding.compatibility(&drifted, &[]),
            SemanticIndexCompatibility::RebuildStructuralDefinitions
        );
    }
}
