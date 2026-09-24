use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

use kernel_model::Value;
use kernel_schema::SemanticContext;
use kernel_types::{EqClassId, RevisionObservableId, SemanticId, SemanticRevision};

use crate::{CanonicalEqKey, SemanticError, SemanticRegistry};

static NEXT_OBSERVABLE_CATALOG_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticObservableDefinition {
    PinnedEquivalence(SemanticId),
    /// Query-/derivative-local coordinate over a pinned equivalence law.
    ///
    /// Two coordinates may intentionally use the same semantic equivalence
    /// while remaining distinct variables until a certified morphism or
    /// pullback constraint relates them. The `coordinate` discriminator is
    /// reconstructible planner state, never semantic authority.
    PinnedEquivalenceCoordinate {
        equivalence: SemanticId,
        coordinate: u64,
    },
    Product(Vec<RevisionObservableId>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObservableClassSignature {
    Canonical(CanonicalEqKey),
    Product(Vec<EqClassId>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservableClassRecord {
    pub observable: RevisionObservableId,
    pub signature: ObservableClassSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservableError {
    Semantic(SemanticError),
    CatalogIdExhausted,
    ObservableIdExhausted,
    EqClassIdExhausted,
    UnknownObservable(RevisionObservableId),
    UnknownEqClass(EqClassId),
    ObservableKindMismatch(RevisionObservableId),
    ArityMismatch {
        expected: usize,
        actual: usize,
    },
    ForeignEqClass {
        class: EqClassId,
        expected: RevisionObservableId,
        actual: RevisionObservableId,
    },
    RevisionMismatch {
        expected: SemanticRevision,
        actual: SemanticRevision,
    },
    MorphismBoundaryMismatch,
    ConflictingMorphismImage,
    InvalidProjectionIndex(usize),
    ObservableCatalogMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionObservableCatalog {
    revision: SemanticRevision,
    catalog_instance: u64,
    next_observable: u64,
    next_class: u64,
    definitions: BTreeMap<RevisionObservableId, SemanticObservableDefinition>,
    definition_ids: BTreeMap<SemanticObservableDefinition, RevisionObservableId>,
    classes: BTreeMap<(RevisionObservableId, ObservableClassSignature), EqClassId>,
    class_records: BTreeMap<EqClassId, ObservableClassRecord>,
}

impl RevisionObservableCatalog {
    pub fn new(context: &SemanticContext) -> Result<Self, ObservableError> {
        let catalog_instance = NEXT_OBSERVABLE_CATALOG_INSTANCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| ObservableError::CatalogIdExhausted)?;
        Ok(Self {
            revision: context.revision(),
            catalog_instance,
            next_observable: 1,
            next_class: 1,
            definitions: BTreeMap::new(),
            definition_ids: BTreeMap::new(),
            classes: BTreeMap::new(),
            class_records: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub(crate) const fn catalog_instance(&self) -> u64 {
        self.catalog_instance
    }

    pub fn register_equivalence(
        &mut self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<RevisionObservableId, ObservableError> {
        self.ensure_revision(context.revision())?;
        registry
            .equivalence_domain(context, equivalence)
            .map_err(ObservableError::Semantic)?;
        self.intern_definition(SemanticObservableDefinition::PinnedEquivalence(equivalence))
    }

    pub fn register_equivalence_coordinate(
        &mut self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
        coordinate: u64,
    ) -> Result<RevisionObservableId, ObservableError> {
        self.ensure_revision(context.revision())?;
        registry
            .equivalence_domain(context, equivalence)
            .map_err(ObservableError::Semantic)?;
        self.intern_definition(SemanticObservableDefinition::PinnedEquivalenceCoordinate {
            equivalence,
            coordinate,
        })
    }

    pub fn register_product(
        &mut self,
        components: Vec<RevisionObservableId>,
    ) -> Result<RevisionObservableId, ObservableError> {
        for component in &components {
            self.definition(*component)?;
        }
        self.intern_definition(SemanticObservableDefinition::Product(components))
    }

    fn intern_definition(
        &mut self,
        definition: SemanticObservableDefinition,
    ) -> Result<RevisionObservableId, ObservableError> {
        if let Some(id) = self.definition_ids.get(&definition) {
            return Ok(*id);
        }
        let id = RevisionObservableId::new(compose_local_id(
            self.catalog_instance,
            self.next_observable,
        ));
        self.next_observable = self
            .next_observable
            .checked_add(1)
            .ok_or(ObservableError::ObservableIdExhausted)?;
        self.definition_ids.insert(definition.clone(), id);
        self.definitions.insert(id, definition);
        Ok(id)
    }

    pub fn definition(
        &self,
        observable: RevisionObservableId,
    ) -> Result<&SemanticObservableDefinition, ObservableError> {
        self.definitions
            .get(&observable)
            .ok_or(ObservableError::UnknownObservable(observable))
    }

    fn intern_equivalence_key(
        &mut self,
        observable: RevisionObservableId,
        key: CanonicalEqKey,
    ) -> Result<EqClassId, ObservableError> {
        if !matches!(
            self.definition(observable)?,
            SemanticObservableDefinition::PinnedEquivalence(_)
                | SemanticObservableDefinition::PinnedEquivalenceCoordinate { .. }
        ) {
            return Err(ObservableError::ObservableKindMismatch(observable));
        }
        self.intern_class(observable, ObservableClassSignature::Canonical(key))
    }

    /// Rehydrates a previously canonicalized equality key into this revision-local catalog.
    /// The caller must already have validated the durable key encoding and pinned observable.
    pub fn intern_canonical_equivalence_key(
        &mut self,
        observable: RevisionObservableId,
        key: CanonicalEqKey,
    ) -> Result<EqClassId, ObservableError> {
        self.intern_equivalence_key(observable, key)
    }

    pub fn observe_value(
        &mut self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        observable: RevisionObservableId,
        value: &Value,
    ) -> Result<EqClassId, ObservableError> {
        self.ensure_revision(context.revision())?;
        let equivalence = match self.definition(observable)? {
            SemanticObservableDefinition::PinnedEquivalence(equivalence)
            | SemanticObservableDefinition::PinnedEquivalenceCoordinate { equivalence, .. } => {
                *equivalence
            }
            SemanticObservableDefinition::Product(_) => {
                return Err(ObservableError::ObservableKindMismatch(observable));
            }
        };
        let key = registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(ObservableError::Semantic)?;
        self.intern_equivalence_key(observable, key)
    }

    pub fn lookup_value_class(
        &self,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        observable: RevisionObservableId,
        value: &Value,
    ) -> Result<Option<EqClassId>, ObservableError> {
        self.ensure_revision(context.revision())?;
        let equivalence = match self.definition(observable)? {
            SemanticObservableDefinition::PinnedEquivalence(equivalence)
            | SemanticObservableDefinition::PinnedEquivalenceCoordinate { equivalence, .. } => {
                *equivalence
            }
            SemanticObservableDefinition::Product(_) => {
                return Err(ObservableError::ObservableKindMismatch(observable));
            }
        };
        let key = registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(ObservableError::Semantic)?;
        Ok(self
            .classes
            .get(&(observable, ObservableClassSignature::Canonical(key)))
            .copied())
    }

    pub fn intern_product_class(
        &mut self,
        observable: RevisionObservableId,
        components: Vec<EqClassId>,
    ) -> Result<EqClassId, ObservableError> {
        let expected = match self.definition(observable)? {
            SemanticObservableDefinition::Product(expected) => expected.clone(),
            SemanticObservableDefinition::PinnedEquivalence(_)
            | SemanticObservableDefinition::PinnedEquivalenceCoordinate { .. } => {
                return Err(ObservableError::ObservableKindMismatch(observable));
            }
        };
        self.validate_class_tuple(&expected, &components)?;
        self.intern_class(observable, ObservableClassSignature::Product(components))
    }

    fn intern_class(
        &mut self,
        observable: RevisionObservableId,
        signature: ObservableClassSignature,
    ) -> Result<EqClassId, ObservableError> {
        let lookup = (observable, signature.clone());
        if let Some(id) = self.classes.get(&lookup) {
            return Ok(*id);
        }
        let id = EqClassId::new(compose_local_id(self.catalog_instance, self.next_class));
        self.next_class = self
            .next_class
            .checked_add(1)
            .ok_or(ObservableError::EqClassIdExhausted)?;
        self.classes.insert(lookup, id);
        self.class_records.insert(
            id,
            ObservableClassRecord {
                observable,
                signature,
            },
        );
        Ok(id)
    }

    pub fn class_record(
        &self,
        class: EqClassId,
    ) -> Result<&ObservableClassRecord, ObservableError> {
        self.class_records
            .get(&class)
            .ok_or(ObservableError::UnknownEqClass(class))
    }

    fn validate_class_tuple(
        &self,
        observables: &[RevisionObservableId],
        classes: &[EqClassId],
    ) -> Result<(), ObservableError> {
        if observables.len() != classes.len() {
            return Err(ObservableError::ArityMismatch {
                expected: observables.len(),
                actual: classes.len(),
            });
        }
        for (&expected, &class) in observables.iter().zip(classes) {
            let actual = self.class_record(class)?.observable;
            if expected != actual {
                return Err(ObservableError::ForeignEqClass {
                    class,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    fn ensure_revision(&self, actual: SemanticRevision) -> Result<(), ObservableError> {
        if self.revision == actual {
            Ok(())
        } else {
            Err(ObservableError::RevisionMismatch {
                expected: self.revision,
                actual,
            })
        }
    }

    fn product_classes(
        &self,
        observable: RevisionObservableId,
    ) -> impl Iterator<Item = (EqClassId, &[EqClassId])> {
        self.class_records.iter().filter_map(move |(id, record)| {
            if record.observable != observable {
                return None;
            }
            let ObservableClassSignature::Product(components) = &record.signature else {
                return None;
            };
            Some((*id, components.as_slice()))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticMorphismCertificate {
    RevisionFiniteSupport,
    AnchorReconstruction,
    ProductProjection {
        product: RevisionObservableId,
        component_indices: Vec<usize>,
    },
    Composition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSemanticMorphism {
    revision: SemanticRevision,
    catalog_instance: u64,
    source: Vec<RevisionObservableId>,
    target: Vec<RevisionObservableId>,
    certificate: SemanticMorphismCertificate,
    mapping: BTreeMap<Vec<EqClassId>, Vec<EqClassId>>,
}

impl CertifiedSemanticMorphism {
    pub(crate) fn from_revision_observations(
        catalog: &RevisionObservableCatalog,
        source: Vec<RevisionObservableId>,
        target: Vec<RevisionObservableId>,
        observations: impl IntoIterator<Item = (Vec<EqClassId>, Vec<EqClassId>)>,
    ) -> Result<Self, ObservableError> {
        Self::from_certified_mapping(
            catalog,
            source,
            target,
            SemanticMorphismCertificate::RevisionFiniteSupport,
            observations,
        )
    }

    pub(crate) fn from_anchor_reconstruction(
        catalog: &RevisionObservableCatalog,
        source: Vec<RevisionObservableId>,
        target: Vec<RevisionObservableId>,
        observations: impl IntoIterator<Item = (Vec<EqClassId>, Vec<EqClassId>)>,
    ) -> Result<Self, ObservableError> {
        Self::from_certified_mapping(
            catalog,
            source,
            target,
            SemanticMorphismCertificate::AnchorReconstruction,
            observations,
        )
    }

    fn from_certified_mapping(
        catalog: &RevisionObservableCatalog,
        source: Vec<RevisionObservableId>,
        target: Vec<RevisionObservableId>,
        certificate: SemanticMorphismCertificate,
        observations: impl IntoIterator<Item = (Vec<EqClassId>, Vec<EqClassId>)>,
    ) -> Result<Self, ObservableError> {
        validate_observables(catalog, &source)?;
        validate_observables(catalog, &target)?;
        let mut mapping = BTreeMap::new();
        for (from, to) in observations {
            catalog.validate_class_tuple(&source, &from)?;
            catalog.validate_class_tuple(&target, &to)?;
            if let Some(existing) = mapping.insert(from, to.clone())
                && existing != to
            {
                return Err(ObservableError::ConflictingMorphismImage);
            }
        }
        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance,
            source,
            target,
            certificate,
            mapping,
        })
    }

    pub fn product_projection(
        catalog: &RevisionObservableCatalog,
        product: RevisionObservableId,
        component_indices: Vec<usize>,
    ) -> Result<Self, ObservableError> {
        let components = match catalog.definition(product)? {
            SemanticObservableDefinition::Product(components) => components,
            SemanticObservableDefinition::PinnedEquivalence(_)
            | SemanticObservableDefinition::PinnedEquivalenceCoordinate { .. } => {
                return Err(ObservableError::ObservableKindMismatch(product));
            }
        };
        let mut target = Vec::with_capacity(component_indices.len());
        for &index in &component_indices {
            let component = components
                .get(index)
                .copied()
                .ok_or(ObservableError::InvalidProjectionIndex(index))?;
            target.push(component);
        }
        let mut mapping = BTreeMap::new();
        for (class, classes) in catalog.product_classes(product) {
            let projected = component_indices
                .iter()
                .map(|&index| classes[index])
                .collect::<Vec<_>>();
            mapping.insert(vec![class], projected);
        }
        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance,
            source: vec![product],
            target,
            certificate: SemanticMorphismCertificate::ProductProjection {
                product,
                component_indices,
            },
            mapping,
        })
    }

    pub fn compose(&self, next: &Self) -> Result<Self, ObservableError> {
        if self.revision != next.revision {
            return Err(ObservableError::RevisionMismatch {
                expected: self.revision,
                actual: next.revision,
            });
        }
        if self.catalog_instance != next.catalog_instance {
            return Err(ObservableError::ObservableCatalogMismatch);
        }
        if self.target != next.source {
            return Err(ObservableError::MorphismBoundaryMismatch);
        }
        let mapping = self
            .mapping
            .iter()
            .filter_map(|(source, middle)| {
                next.mapping
                    .get(middle)
                    .map(|target| (source.clone(), target.clone()))
            })
            .collect();
        Ok(Self {
            revision: self.revision,
            catalog_instance: self.catalog_instance,
            source: self.source.clone(),
            target: next.target.clone(),
            certificate: SemanticMorphismCertificate::Composition,
            mapping,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub(crate) const fn catalog_instance(&self) -> u64 {
        self.catalog_instance
    }

    #[must_use]
    pub fn source(&self) -> &[RevisionObservableId] {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &[RevisionObservableId] {
        &self.target
    }

    #[must_use]
    pub const fn certificate(&self) -> &SemanticMorphismCertificate {
        &self.certificate
    }

    #[must_use]
    pub fn image(&self, source: &[EqClassId]) -> Option<&[EqClassId]> {
        self.mapping.get(source).map(Vec::as_slice)
    }

    #[must_use]
    pub fn materialized_mapping(&self) -> &BTreeMap<Vec<EqClassId>, Vec<EqClassId>> {
        &self.mapping
    }
}

fn compose_local_id(catalog_instance: u64, local: u64) -> u128 {
    (u128::from(catalog_instance) << 64) | u128::from(local)
}

fn validate_observables(
    catalog: &RevisionObservableCatalog,
    observables: &[RevisionObservableId],
) -> Result<(), ObservableError> {
    for observable in observables {
        catalog.definition(*observable)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use kernel_schema::{Schema, SemanticEnvironment};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    use super::*;

    fn fixture(ids: &[SemanticId]) -> (SemanticContext, SemanticRegistry) {
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(crate::EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9));
        for id in ids {
            environment.pin_module(*id, digest);
        }
        (
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(7)),
                environment,
            },
            registry,
        )
    }

    #[test]
    fn product_observable_is_revision_local_exact_meet_coordinate() {
        let left_eq = SemanticId::new(10);
        let right_eq = SemanticId::new(11);
        let (context, registry) = fixture(&[left_eq, right_eq]);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let right = catalog
            .register_equivalence(&registry, &context, right_eq)
            .unwrap();
        let product = catalog.register_product(vec![left, right]).unwrap();

        let left_a = catalog
            .observe_value(&registry, &context, left, &Value::I64(1))
            .unwrap();
        let left_b = catalog
            .observe_value(&registry, &context, left, &Value::I64(2))
            .unwrap();
        let right_a = catalog
            .observe_value(&registry, &context, right, &Value::I64(7))
            .unwrap();

        let joint_a = catalog
            .intern_product_class(product, vec![left_a, right_a])
            .unwrap();
        let joint_a_again = catalog
            .intern_product_class(product, vec![left_a, right_a])
            .unwrap();
        let joint_b = catalog
            .intern_product_class(product, vec![left_b, right_a])
            .unwrap();
        assert_eq!(joint_a, joint_a_again);
        assert_ne!(joint_a, joint_b);

        let projection =
            CertifiedSemanticMorphism::product_projection(&catalog, product, vec![0, 1]).unwrap();
        assert_eq!(
            projection.image(&[joint_a]),
            Some([left_a, right_a].as_slice())
        );
    }

    #[test]
    fn observable_registration_is_pinned_to_one_semantic_revision() {
        let eq = SemanticId::new(12);
        let (context, registry) = fixture(&[eq]);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let observable = catalog
            .register_equivalence(&registry, &context, eq)
            .unwrap();

        let mut changed = context.clone();
        changed.environment = SemanticEnvironment::new(SemanticEnvId::new(10));
        assert!(matches!(
            catalog.observe_value(&registry, &changed, observable, &Value::I64(1)),
            Err(ObservableError::RevisionMismatch { .. })
        ));
    }

    #[test]
    fn morphism_ir_is_nary_to_mary_and_rejects_non_functions() {
        let ids = [
            SemanticId::new(1),
            SemanticId::new(2),
            SemanticId::new(3),
            SemanticId::new(4),
        ];
        let (context, registry) = fixture(&ids);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let mut observables = Vec::new();
        for id in ids {
            observables.push(
                catalog
                    .register_equivalence(&registry, &context, id)
                    .unwrap(),
            );
        }
        let [a, b, c, d] = observables.as_slice() else {
            unreachable!();
        };
        let a = *a;
        let b = *b;
        let c = *c;
        let d = *d;
        let a1 = catalog
            .observe_value(&registry, &context, a, &Value::I64(1))
            .unwrap();
        let b1 = catalog
            .observe_value(&registry, &context, b, &Value::I64(2))
            .unwrap();
        let c1 = catalog
            .observe_value(&registry, &context, c, &Value::I64(3))
            .unwrap();
        let d1 = catalog
            .observe_value(&registry, &context, d, &Value::I64(4))
            .unwrap();
        let d2 = catalog
            .observe_value(&registry, &context, d, &Value::I64(5))
            .unwrap();

        let morphism = CertifiedSemanticMorphism::from_revision_observations(
            &catalog,
            vec![a, b],
            vec![c, d],
            [(vec![a1, b1], vec![c1, d1])],
        )
        .unwrap();
        assert_eq!(morphism.image(&[a1, b1]), Some([c1, d1].as_slice()));

        assert_eq!(
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![a, b],
                vec![c, d],
                [(vec![a1, b1], vec![c1, d1]), (vec![a1, b1], vec![c1, d2]),],
            ),
            Err(ObservableError::ConflictingMorphismImage)
        );
    }

    #[test]
    fn materialized_morphisms_compose_without_changing_semantic_revision() {
        let ids = [
            SemanticId::new(21),
            SemanticId::new(22),
            SemanticId::new(23),
        ];
        let (context, registry) = fixture(&ids);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let a = catalog
            .register_equivalence(&registry, &context, ids[0])
            .unwrap();
        let b = catalog
            .register_equivalence(&registry, &context, ids[1])
            .unwrap();
        let c = catalog
            .register_equivalence(&registry, &context, ids[2])
            .unwrap();
        let a1 = catalog
            .observe_value(&registry, &context, a, &Value::I64(1))
            .unwrap();
        let b1 = catalog
            .observe_value(&registry, &context, b, &Value::I64(2))
            .unwrap();
        let c1 = catalog
            .observe_value(&registry, &context, c, &Value::I64(3))
            .unwrap();
        let ab = CertifiedSemanticMorphism::from_revision_observations(
            &catalog,
            vec![a],
            vec![b],
            [(vec![a1], vec![b1])],
        )
        .unwrap();
        let bc = CertifiedSemanticMorphism::from_revision_observations(
            &catalog,
            vec![b],
            vec![c],
            [(vec![b1], vec![c1])],
        )
        .unwrap();
        let ac = ab.compose(&bc).unwrap();
        assert_eq!(ac.revision(), context.revision());
        assert_eq!(ac.image(&[a1]), Some([c1].as_slice()));
        assert_eq!(ac.certificate(), &SemanticMorphismCertificate::Composition);
    }

    #[test]
    fn independent_catalogs_cannot_alias_revision_local_observable_or_class_ids() {
        let ids = [SemanticId::new(31), SemanticId::new(32)];
        let (context, registry) = fixture(&ids);
        let mut left_catalog = RevisionObservableCatalog::new(&context).unwrap();
        let mut right_catalog = RevisionObservableCatalog::new(&context).unwrap();

        let left_a = left_catalog
            .register_equivalence(&registry, &context, ids[0])
            .unwrap();
        let left_b = left_catalog
            .register_equivalence(&registry, &context, ids[1])
            .unwrap();
        let right_a = right_catalog
            .register_equivalence(&registry, &context, ids[0])
            .unwrap();
        let right_b = right_catalog
            .register_equivalence(&registry, &context, ids[1])
            .unwrap();
        assert_ne!(left_a, right_a);
        assert_ne!(left_b, right_b);

        let left_source_class = left_catalog
            .observe_value(&registry, &context, left_a, &Value::I64(1))
            .unwrap();
        let left_target_class = left_catalog
            .observe_value(&registry, &context, left_b, &Value::I64(2))
            .unwrap();
        let right_source_class = right_catalog
            .observe_value(&registry, &context, right_a, &Value::I64(1))
            .unwrap();
        let right_target_class = right_catalog
            .observe_value(&registry, &context, right_b, &Value::I64(2))
            .unwrap();
        assert_ne!(left_source_class, right_source_class);

        let left = CertifiedSemanticMorphism::from_revision_observations(
            &left_catalog,
            vec![left_a],
            vec![left_b],
            [(vec![left_source_class], vec![left_target_class])],
        )
        .unwrap();
        let right = CertifiedSemanticMorphism::from_revision_observations(
            &right_catalog,
            vec![right_a],
            vec![right_b],
            [(vec![right_source_class], vec![right_target_class])],
        )
        .unwrap();
        assert_eq!(
            left.compose(&right),
            Err(ObservableError::ObservableCatalogMismatch)
        );
    }

    #[test]
    fn query_coordinates_keep_same_equivalence_law_nominally_distinct() {
        let equivalence = SemanticId::new(41);
        let (context, registry) = fixture(&[equivalence]);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence_coordinate(&registry, &context, equivalence, 10)
            .unwrap();
        let right = catalog
            .register_equivalence_coordinate(&registry, &context, equivalence, 11)
            .unwrap();
        assert_ne!(left, right);

        let left_class = catalog
            .observe_value(&registry, &context, left, &Value::I64(7))
            .unwrap();
        let right_class = catalog
            .observe_value(&registry, &context, right, &Value::I64(7))
            .unwrap();
        assert_ne!(left_class, right_class);
        assert_eq!(catalog.class_record(left_class).unwrap().observable, left);
        assert_eq!(catalog.class_record(right_class).unwrap().observable, right);
    }
}
