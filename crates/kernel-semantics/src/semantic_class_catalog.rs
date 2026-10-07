use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use kernel_model::Value;
use kernel_persistent::PersistentOrdMap;
use kernel_schema::SemanticContext;
use kernel_types::{EqClassId, SemanticId, SemanticRevision};

use crate::{CanonicalEqKey, SemanticError, SemanticRegistry};

static NEXT_SEMANTIC_CLASS_CATALOG_INSTANCE: AtomicU64 = AtomicU64::new(1);
static NEXT_SEMANTIC_CLASS_SERIAL: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticClassCatalogError {
    Semantic(SemanticError),
    CatalogIdExhausted,
    ClassIdExhausted,
    RevisionMismatch {
        expected: SemanticRevision,
        actual: SemanticRevision,
    },
    UnknownClass(EqClassId),
    ReferenceCountOverflow(EqClassId),
    ReferenceCountUnderflow(EqClassId),
}

impl From<SemanticError> for SemanticClassCatalogError {
    fn from(value: SemanticError) -> Self {
        Self::Semantic(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticClassRecord {
    equivalence: SemanticId,
    key: Arc<CanonicalEqKey>,
    references: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionSemanticClassCatalog {
    revision: SemanticRevision,
    catalog_instance: u64,
    classes_by_equivalence:
        PersistentOrdMap<SemanticId, PersistentOrdMap<Arc<CanonicalEqKey>, EqClassId>>,
    records: PersistentOrdMap<EqClassId, SemanticClassRecord>,
}

impl RevisionSemanticClassCatalog {
    pub fn new(context: &SemanticContext) -> Result<Self, SemanticClassCatalogError> {
        let catalog_instance = NEXT_SEMANTIC_CLASS_CATALOG_INSTANCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SemanticClassCatalogError::CatalogIdExhausted)?;
        Ok(Self {
            revision: context.revision(),
            catalog_instance,
            classes_by_equivalence: PersistentOrdMap::default(),
            records: PersistentOrdMap::default(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub fn live_class_count(&self) -> usize {
        self.records.len()
    }

    pub fn retain_value(
        &mut self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
        value: &Value,
    ) -> Result<EqClassId, SemanticClassCatalogError> {
        self.ensure_revision(context.revision())?;
        let key = registry.canonical_equivalence_key(context, equivalence, value)?;
        self.retain_canonical_key(context, equivalence, key)
    }

    pub fn retain_canonical_key(
        &mut self,
        context: &SemanticContext,
        equivalence: SemanticId,
        key: CanonicalEqKey,
    ) -> Result<EqClassId, SemanticClassCatalogError> {
        self.ensure_revision(context.revision())?;
        if let Some(id) = self
            .classes_by_equivalence
            .get(&equivalence)
            .and_then(|classes| classes.get(&key))
            .copied()
        {
            let record = self
                .records
                .get_mut(&id)
                .ok_or(SemanticClassCatalogError::UnknownClass(id))?;
            record.references = record
                .references
                .checked_add(1)
                .ok_or(SemanticClassCatalogError::ReferenceCountOverflow(id))?;
            return Ok(id);
        }

        let serial = NEXT_SEMANTIC_CLASS_SERIAL
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SemanticClassCatalogError::ClassIdExhausted)?;
        let id = EqClassId::new(compose_local_id(self.catalog_instance, serial));
        let key = Arc::new(key);
        self.classes_by_equivalence
            .get_mut(&equivalence)
            .map(|classes| {
                classes.insert(Arc::clone(&key), id);
            })
            .unwrap_or_else(|| {
                let mut classes = PersistentOrdMap::default();
                classes.insert(Arc::clone(&key), id);
                self.classes_by_equivalence.insert(equivalence, classes);
            });
        self.records.insert(
            id,
            SemanticClassRecord {
                equivalence,
                key,
                references: 1,
            },
        );
        Ok(id)
    }

    pub fn lookup_value(
        &self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
        value: &Value,
    ) -> Result<Option<EqClassId>, SemanticClassCatalogError> {
        self.ensure_revision(context.revision())?;
        let key = registry.canonical_equivalence_key(context, equivalence, value)?;
        Ok(self
            .classes_by_equivalence
            .get(&equivalence)
            .and_then(|classes| classes.get(&key))
            .copied())
    }

    pub fn release(&mut self, class: EqClassId) -> Result<bool, SemanticClassCatalogError> {
        let record = self
            .records
            .get_mut(&class)
            .ok_or(SemanticClassCatalogError::UnknownClass(class))?;
        record.references = record
            .references
            .checked_sub(1)
            .ok_or(SemanticClassCatalogError::ReferenceCountUnderflow(class))?;
        if record.references != 0 {
            return Ok(false);
        }

        let record = self
            .records
            .remove(&class)
            .ok_or(SemanticClassCatalogError::UnknownClass(class))?;
        let remove_equivalence_bucket = {
            let classes = self
                .classes_by_equivalence
                .get_mut(&record.equivalence)
                .ok_or(SemanticClassCatalogError::UnknownClass(class))?;
            if classes.remove(&record.key) != Some(class) {
                return Err(SemanticClassCatalogError::UnknownClass(class));
            }
            classes.is_empty()
        };
        if remove_equivalence_bucket {
            self.classes_by_equivalence.remove(&record.equivalence);
        }
        Ok(true)
    }

    #[must_use]
    pub fn class_key(&self, class: EqClassId) -> Option<&CanonicalEqKey> {
        self.records.get(&class).map(|record| record.key.as_ref())
    }

    fn ensure_revision(&self, actual: SemanticRevision) -> Result<(), SemanticClassCatalogError> {
        if self.revision == actual {
            Ok(())
        } else {
            Err(SemanticClassCatalogError::RevisionMismatch {
                expected: self.revision,
                actual,
            })
        }
    }
}

fn compose_local_id(catalog_instance: u64, local: u64) -> u128 {
    (u128::from(catalog_instance) << 64) | u128::from(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    use crate::EquivalenceModule;

    fn context(equivalence: SemanticId, registry: &mut SemanticRegistry) -> SemanticContext {
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(700));
        environment.pin_module(equivalence, digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(700)),
            environment,
        }
    }

    #[test]
    fn semantic_class_catalog_shares_atomic_identity_and_retires_last_reference() {
        let equivalence = SemanticId::new(701);
        let mut registry = SemanticRegistry::default();
        let context = context(equivalence, &mut registry);
        let mut catalog = RevisionSemanticClassCatalog::new(&context).unwrap();

        let upper = catalog
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("Alpha".into()),
            )
            .unwrap();
        let lower = catalog
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("alpha".into()),
            )
            .unwrap();
        assert_eq!(upper, lower);
        assert_eq!(catalog.live_class_count(), 1);
        assert!(!catalog.release(upper).unwrap());
        assert_eq!(catalog.live_class_count(), 1);
        assert!(catalog.release(lower).unwrap());
        assert_eq!(catalog.live_class_count(), 0);
        assert_eq!(
            catalog
                .lookup_value(
                    &registry,
                    &context,
                    equivalence,
                    &Value::Text("ALPHA".into()),
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn retired_class_id_is_never_reused_and_snapshot_keeps_old_identity() {
        let equivalence = SemanticId::new(702);
        let mut registry = SemanticRegistry::default();
        let context = context(equivalence, &mut registry);
        let mut live = RevisionSemanticClassCatalog::new(&context).unwrap();
        let alpha = live
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("alpha".into()),
            )
            .unwrap();
        let snapshot = live.clone();
        assert!(live.release(alpha).unwrap());
        let beta = live
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("beta".into()),
            )
            .unwrap();
        assert_ne!(alpha, beta);
        assert_eq!(
            snapshot
                .lookup_value(
                    &registry,
                    &context,
                    equivalence,
                    &Value::Text("ALPHA".into()),
                )
                .unwrap(),
            Some(alpha)
        );
        assert_eq!(
            live.lookup_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("alpha".into()),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn sibling_snapshots_cannot_alias_different_new_classes() {
        let equivalence = SemanticId::new(703);
        let mut registry = SemanticRegistry::default();
        let context = context(equivalence, &mut registry);
        let root = RevisionSemanticClassCatalog::new(&context).unwrap();
        let mut left = root.clone();
        let mut right = root;
        let left_id = left
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("left".into()),
            )
            .unwrap();
        let right_id = right
            .retain_value(
                &registry,
                &context,
                equivalence,
                &Value::Text("right".into()),
            )
            .unwrap();
        assert_ne!(left_id, right_id);
    }
}
