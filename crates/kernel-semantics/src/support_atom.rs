use std::collections::{BTreeMap, BTreeSet};

use kernel_persistent::{PersistentOrdMap, PersistentOrdSet};
use kernel_types::{EqClassId, RevisionObservableId, SemanticRevision};

use crate::observable::{
    ObservableClassSignature, ObservableError, RevisionObservableCatalog,
    SemanticObservableDefinition,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportAtomError<RowId> {
    Observable(ObservableError),
    CatalogMismatch,
    ProductObservableMismatch,
    ArityMismatch {
        expected: usize,
        actual: usize,
    },
    ForeignEqClass {
        class: EqClassId,
        expected: RevisionObservableId,
        actual: RevisionObservableId,
    },
    InvalidProductClass(EqClassId),
    DuplicateRow(RowId),
    UnknownRow(RowId),
}

impl<RowId> From<ObservableError> for SupportAtomError<RowId> {
    fn from(value: ObservableError) -> Self {
        Self::Observable(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportAtom<RowId: Ord + Clone> {
    signature: Vec<EqClassId>,
    rows: PersistentOrdSet<RowId>,
}

/// Finite partition of physical support by one revision-local product observable.
///
/// `EqClassId` of the product observable is the atom identity. No atom identifier
/// is durable semantic authority: the whole structure is reconstructible from the
/// pinned revision, observable recipe, and authoritative relation rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportAtomFabric<RowId: Ord + Clone> {
    revision: SemanticRevision,
    catalog_instance: u64,
    product: RevisionObservableId,
    coordinates: Vec<RevisionObservableId>,
    signature_to_atom: PersistentOrdMap<Vec<EqClassId>, EqClassId>,
    atoms: PersistentOrdMap<EqClassId, SupportAtom<RowId>>,
    row_to_atom: PersistentOrdMap<RowId, EqClassId>,
    inverse: Vec<PersistentOrdMap<EqClassId, PersistentOrdSet<EqClassId>>>,
}

impl<RowId: Ord + Clone> SupportAtomFabric<RowId> {
    pub fn new(
        catalog: &RevisionObservableCatalog,
        product: RevisionObservableId,
        coordinates: Vec<RevisionObservableId>,
    ) -> Result<Self, SupportAtomError<RowId>> {
        for coordinate in &coordinates {
            catalog.definition(*coordinate)?;
        }
        match catalog.definition(product)? {
            SemanticObservableDefinition::Product(components) if components == &coordinates => {}
            _ => return Err(SupportAtomError::ProductObservableMismatch),
        }
        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance(),
            product,
            inverse: vec![PersistentOrdMap::default(); coordinates.len()],
            coordinates,
            signature_to_atom: PersistentOrdMap::default(),
            atoms: PersistentOrdMap::default(),
            row_to_atom: PersistentOrdMap::default(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub const fn product(&self) -> RevisionObservableId {
        self.product
    }

    #[must_use]
    pub fn coordinates(&self) -> &[RevisionObservableId] {
        &self.coordinates
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_to_atom.len()
    }

    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.atoms.len()
    }

    pub fn atom_masses(&self) -> impl Iterator<Item = (EqClassId, usize)> + '_ {
        self.atoms
            .iter()
            .map(|(&atom, support)| (atom, support.rows.len()))
    }

    #[must_use]
    pub fn projection_atom_reference_count(&self) -> usize {
        self.inverse
            .iter()
            .map(|classes| classes.values().map(PersistentOrdSet::len).sum::<usize>())
            .sum()
    }

    #[must_use]
    pub fn projected_class_count(&self) -> usize {
        self.inverse.iter().map(PersistentOrdMap::len).sum()
    }

    pub fn insert(
        &mut self,
        catalog: &RevisionObservableCatalog,
        row: RowId,
        atom_class: EqClassId,
        signature: &[EqClassId],
    ) -> Result<(), SupportAtomError<RowId>> {
        self.ensure_catalog(catalog)?;
        self.validate_signature(catalog, signature)?;
        self.validate_product_class(catalog, atom_class, signature)?;
        if self.row_to_atom.contains_key(&row) {
            return Err(SupportAtomError::DuplicateRow(row));
        }

        if let Some(mut atom) = self.atoms.get(&atom_class).cloned() {
            if atom.signature != signature {
                return Err(SupportAtomError::InvalidProductClass(atom_class));
            }
            atom.rows.insert(row.clone());
            self.atoms.insert(atom_class, atom);
        } else {
            if let Some(existing) = self
                .signature_to_atom
                .insert(signature.to_owned(), atom_class)
                && existing != atom_class
            {
                return Err(SupportAtomError::InvalidProductClass(atom_class));
            }
            for (slot, class) in signature.iter().copied().enumerate() {
                let mut atom_classes = self.inverse[slot].get(&class).cloned().unwrap_or_default();
                atom_classes.insert(atom_class);
                self.inverse[slot].insert(class, atom_classes);
            }
            self.atoms.insert(
                atom_class,
                SupportAtom {
                    signature: signature.to_owned(),
                    rows: [row.clone()].into_iter().collect(),
                },
            );
        }
        self.row_to_atom.insert(row, atom_class);
        Ok(())
    }

    pub fn remove(&mut self, row: &RowId) -> Result<Vec<EqClassId>, SupportAtomError<RowId>> {
        let atom_class = self
            .row_to_atom
            .remove(row)
            .ok_or_else(|| SupportAtomError::UnknownRow(row.clone()))?;
        let mut atom = self
            .atoms
            .get(&atom_class)
            .cloned()
            .expect("row-to-atom map references an existing atom");
        atom.rows.remove(row);
        let signature = atom.signature.clone();
        if atom.rows.is_empty() {
            self.atoms.remove(&atom_class);
            self.signature_to_atom.remove(&signature);
            for (slot, class) in signature.iter().copied().enumerate() {
                let mut atom_classes = self.inverse[slot]
                    .get(&class)
                    .cloned()
                    .expect("inverse projection contains every live atom");
                atom_classes.remove(&atom_class);
                if atom_classes.is_empty() {
                    self.inverse[slot].remove(&class);
                } else {
                    self.inverse[slot].insert(class, atom_classes);
                }
            }
        } else {
            self.atoms.insert(atom_class, atom);
        }
        Ok(signature)
    }

    #[must_use]
    pub fn row_signature(&self, row: &RowId) -> Option<&[EqClassId]> {
        let atom_class = self.row_to_atom.get(row)?;
        self.atoms
            .get(atom_class)
            .map(|atom| atom.signature.as_slice())
    }

    #[must_use]
    pub fn joint_fiber(&self, signature: &[EqClassId]) -> Option<&PersistentOrdSet<RowId>> {
        let atom_class = self.signature_to_atom.get(&signature.to_vec())?;
        self.atoms.get(atom_class).map(|atom| &atom.rows)
    }

    pub fn projected_fiber(
        &self,
        slot: usize,
        class: EqClassId,
    ) -> Result<BTreeSet<RowId>, SupportAtomError<RowId>> {
        if slot >= self.coordinates.len() {
            return Err(SupportAtomError::ArityMismatch {
                expected: self.coordinates.len(),
                actual: slot.saturating_add(1),
            });
        }
        let mut rows = BTreeSet::new();
        if let Some(atom_classes) = self.inverse[slot].get(&class) {
            for atom_class in atom_classes {
                rows.extend(
                    self.atoms
                        .get(atom_class)
                        .expect("inverse projection references an existing atom")
                        .rows
                        .iter()
                        .cloned(),
                );
            }
        }
        Ok(rows)
    }

    pub fn projected_count(
        &self,
        slot: usize,
        class: EqClassId,
    ) -> Result<usize, SupportAtomError<RowId>> {
        if slot >= self.coordinates.len() {
            return Err(SupportAtomError::ArityMismatch {
                expected: self.coordinates.len(),
                actual: slot.saturating_add(1),
            });
        }
        Ok(self.inverse[slot]
            .get(&class)
            .into_iter()
            .flatten()
            .map(|atom_class| {
                self.atoms
                    .get(atom_class)
                    .expect("inverse projection references an existing atom")
                    .rows
                    .len()
            })
            .sum())
    }

    pub fn distinct_classes(
        &self,
        slot: usize,
    ) -> Result<BTreeSet<EqClassId>, SupportAtomError<RowId>> {
        if slot >= self.coordinates.len() {
            return Err(SupportAtomError::ArityMismatch {
                expected: self.coordinates.len(),
                actual: slot.saturating_add(1),
            });
        }
        Ok(self.inverse[slot].keys().copied().collect())
    }

    fn ensure_catalog(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<(), SupportAtomError<RowId>> {
        if self.revision != catalog.revision()
            || self.catalog_instance != catalog.catalog_instance()
        {
            return Err(SupportAtomError::CatalogMismatch);
        }
        Ok(())
    }

    fn validate_signature(
        &self,
        catalog: &RevisionObservableCatalog,
        signature: &[EqClassId],
    ) -> Result<(), SupportAtomError<RowId>> {
        if signature.len() != self.coordinates.len() {
            return Err(SupportAtomError::ArityMismatch {
                expected: self.coordinates.len(),
                actual: signature.len(),
            });
        }
        for (&expected, &class) in self.coordinates.iter().zip(signature) {
            let actual = catalog.class_record(class)?.observable;
            if expected != actual {
                return Err(SupportAtomError::ForeignEqClass {
                    class,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    fn validate_product_class(
        &self,
        catalog: &RevisionObservableCatalog,
        atom_class: EqClassId,
        signature: &[EqClassId],
    ) -> Result<(), SupportAtomError<RowId>> {
        let record = catalog.class_record(atom_class)?;
        if record.observable != self.product
            || record.signature != ObservableClassSignature::Product(signature.to_owned())
        {
            return Err(SupportAtomError::InvalidProductClass(atom_class));
        }
        Ok(())
    }
}

/// Reconstructible per-atom annotations layered over one support fabric.
///
/// The annotation payload is physical/derived state. Atom identity remains revision-local and
/// cannot be persisted as semantic authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportAtomAnnotationOverlay<A> {
    revision: SemanticRevision,
    catalog_instance: u64,
    product: RevisionObservableId,
    annotations: BTreeMap<EqClassId, A>,
}

impl<A> SupportAtomAnnotationOverlay<A> {
    #[must_use]
    pub fn new<RowId: Ord + Clone>(fabric: &SupportAtomFabric<RowId>) -> Self {
        Self {
            revision: fabric.revision,
            catalog_instance: fabric.catalog_instance,
            product: fabric.product,
            annotations: BTreeMap::new(),
        }
    }

    pub fn insert<RowId: Ord + Clone>(
        &mut self,
        fabric: &SupportAtomFabric<RowId>,
        atom: EqClassId,
        annotation: A,
    ) -> Result<Option<A>, SupportAtomOverlayError<RowId>> {
        self.ensure_fabric(fabric)?;
        if !fabric.atoms.contains_key(&atom) {
            return Err(SupportAtomOverlayError::UnknownAtom(atom));
        }
        Ok(self.annotations.insert(atom, annotation))
    }

    #[must_use]
    pub fn get(&self, atom: EqClassId) -> Option<&A> {
        self.annotations.get(&atom)
    }

    pub fn retain_live<RowId: Ord + Clone>(
        &mut self,
        fabric: &SupportAtomFabric<RowId>,
    ) -> Result<(), SupportAtomOverlayError<RowId>> {
        self.ensure_fabric(fabric)?;
        self.annotations
            .retain(|atom, _| fabric.atoms.contains_key(atom));
        Ok(())
    }

    fn ensure_fabric<RowId: Ord + Clone>(
        &self,
        fabric: &SupportAtomFabric<RowId>,
    ) -> Result<(), SupportAtomOverlayError<RowId>> {
        if self.revision != fabric.revision
            || self.catalog_instance != fabric.catalog_instance
            || self.product != fabric.product
        {
            return Err(SupportAtomOverlayError::FabricMismatch);
        }
        Ok(())
    }
}

/// Ordered overlay over SAMF atoms.
///
/// Construction consumes one order-class key per live row and proves that the key is constant on
/// each equality atom. That is exactly the physical congruence condition needed to use equality
/// atoms as WITH-TIES order classes; any disagreement is rejected rather than tie-broken away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportAtomOrderedOverlay<K> {
    revision: SemanticRevision,
    catalog_instance: u64,
    product: RevisionObservableId,
    atom_key: BTreeMap<EqClassId, K>,
    ordered_atoms: BTreeMap<K, BTreeSet<EqClassId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportAtomOverlayError<RowId> {
    FabricMismatch,
    UnknownAtom(EqClassId),
    UnknownRow(RowId),
    DuplicateRow(RowId),
    MissingRows { expected: usize, actual: usize },
    IncongruentOrderKey(EqClassId),
}

impl<K: Ord + Clone> SupportAtomOrderedOverlay<K> {
    pub fn build<RowId: Ord + Clone>(
        fabric: &SupportAtomFabric<RowId>,
        row_keys: impl IntoIterator<Item = (RowId, K)>,
    ) -> Result<Self, SupportAtomOverlayError<RowId>> {
        let mut atom_key = BTreeMap::<EqClassId, K>::new();
        let mut seen = BTreeSet::new();
        for (row, key) in row_keys {
            if !seen.insert(row.clone()) {
                return Err(SupportAtomOverlayError::DuplicateRow(row));
            }
            let atom = fabric
                .row_to_atom
                .get(&row)
                .copied()
                .ok_or_else(|| SupportAtomOverlayError::UnknownRow(row.clone()))?;
            if let Some(existing) = atom_key.get(&atom) {
                if existing != &key {
                    return Err(SupportAtomOverlayError::IncongruentOrderKey(atom));
                }
            } else {
                atom_key.insert(atom, key);
            }
        }
        if seen.len() != fabric.row_count() {
            return Err(SupportAtomOverlayError::MissingRows {
                expected: fabric.row_count(),
                actual: seen.len(),
            });
        }
        let mut ordered_atoms = BTreeMap::<K, BTreeSet<EqClassId>>::new();
        for (&atom, key) in &atom_key {
            ordered_atoms.entry(key.clone()).or_default().insert(atom);
        }
        Ok(Self {
            revision: fabric.revision,
            catalog_instance: fabric.catalog_instance,
            product: fabric.product,
            atom_key,
            ordered_atoms,
        })
    }

    #[must_use]
    pub fn order_key(&self, atom: EqClassId) -> Option<&K> {
        self.atom_key.get(&atom)
    }

    #[must_use]
    pub fn classes_in_order(&self) -> impl DoubleEndedIterator<Item = (&K, &BTreeSet<EqClassId>)> {
        self.ordered_atoms.iter()
    }

    #[must_use]
    pub fn compatible_with<RowId: Ord + Clone>(&self, fabric: &SupportAtomFabric<RowId>) -> bool {
        self.revision == fabric.revision
            && self.catalog_instance == fabric.catalog_instance
            && self.product == fabric.product
    }
}

#[cfg(test)]
mod tests {
    use kernel_model::Value;
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;
    use crate::{EquivalenceModule, SemanticRegistry};

    fn fixture() -> (SemanticContext, SemanticRegistry, SemanticId, SemanticId) {
        let left = SemanticId::new(10);
        let right = SemanticId::new(11);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9));
        environment.pin_module(left, digest);
        environment.pin_module(right, digest);
        (
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(7)),
                environment,
            },
            registry,
            left,
            right,
        )
    }

    #[test]
    fn support_atoms_are_product_classes_and_factor_projected_fibers() {
        let (context, registry, left_eq, right_eq) = fixture();
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let right = catalog
            .register_equivalence(&registry, &context, right_eq)
            .unwrap();
        let product = catalog.register_product(vec![left, right]).unwrap();
        let mut fabric = SupportAtomFabric::new(&catalog, product, vec![left, right]).unwrap();

        let l1 = catalog
            .observe_value(&registry, &context, left, &Value::I64(1))
            .unwrap();
        let l2 = catalog
            .observe_value(&registry, &context, left, &Value::I64(2))
            .unwrap();
        let r7 = catalog
            .observe_value(&registry, &context, right, &Value::I64(7))
            .unwrap();
        let a17 = catalog.intern_product_class(product, vec![l1, r7]).unwrap();
        let a27 = catalog.intern_product_class(product, vec![l2, r7]).unwrap();
        fabric.insert(&catalog, 1_u64, a17, &[l1, r7]).unwrap();
        fabric.insert(&catalog, 2_u64, a17, &[l1, r7]).unwrap();
        fabric.insert(&catalog, 3_u64, a27, &[l2, r7]).unwrap();

        assert_eq!(fabric.row_count(), 3);
        assert_eq!(fabric.atom_count(), 2);
        assert_eq!(fabric.joint_fiber(&[l1, r7]).unwrap().len(), 2);
        assert_eq!(fabric.projected_count(0, l1).unwrap(), 2);
        assert_eq!(fabric.projected_count(1, r7).unwrap(), 3);
        assert_eq!(fabric.projected_fiber(0, l2).unwrap(), BTreeSet::from([3]));
    }

    #[test]
    fn deletion_removes_empty_atoms_but_preserves_other_projection_members() {
        let (context, registry, left_eq, right_eq) = fixture();
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let right = catalog
            .register_equivalence(&registry, &context, right_eq)
            .unwrap();
        let product = catalog.register_product(vec![left, right]).unwrap();
        let mut fabric = SupportAtomFabric::new(&catalog, product, vec![left, right]).unwrap();
        let l1 = catalog
            .observe_value(&registry, &context, left, &Value::I64(1))
            .unwrap();
        let l2 = catalog
            .observe_value(&registry, &context, left, &Value::I64(2))
            .unwrap();
        let r7 = catalog
            .observe_value(&registry, &context, right, &Value::I64(7))
            .unwrap();
        let a17 = catalog.intern_product_class(product, vec![l1, r7]).unwrap();
        let a27 = catalog.intern_product_class(product, vec![l2, r7]).unwrap();
        fabric.insert(&catalog, 1_u64, a17, &[l1, r7]).unwrap();
        fabric.insert(&catalog, 2_u64, a27, &[l2, r7]).unwrap();

        fabric.remove(&1).unwrap();
        assert_eq!(fabric.atom_count(), 1);
        assert!(fabric.joint_fiber(&[l1, r7]).is_none());
        assert_eq!(fabric.projected_count(1, r7).unwrap(), 1);
    }

    #[test]
    fn support_atom_snapshot_path_copies_only_touched_atom_and_row_directories() {
        let (context, registry, left_eq, right_eq) = fixture();
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let right = catalog
            .register_equivalence(&registry, &context, right_eq)
            .unwrap();
        let product = catalog.register_product(vec![left, right]).unwrap();
        let mut fabric = SupportAtomFabric::new(&catalog, product, vec![left, right]).unwrap();

        let l1 = catalog
            .observe_value(&registry, &context, left, &Value::I64(1))
            .unwrap();
        let l2 = catalog
            .observe_value(&registry, &context, left, &Value::I64(2))
            .unwrap();
        let r7 = catalog
            .observe_value(&registry, &context, right, &Value::I64(7))
            .unwrap();
        let a17 = catalog.intern_product_class(product, vec![l1, r7]).unwrap();
        let a27 = catalog.intern_product_class(product, vec![l2, r7]).unwrap();

        for row in 0..2048_u64 {
            fabric.insert(&catalog, row, a17, &[l1, r7]).unwrap();
            fabric
                .insert(&catalog, 10_000 + row, a27, &[l2, r7])
                .unwrap();
        }
        let snapshot = fabric.clone();
        fabric.remove(&1024).unwrap();

        let old_untouched = snapshot.atoms.get(&a27).unwrap();
        let new_untouched = fabric.atoms.get(&a27).unwrap();
        assert!(old_untouched.rows.shares_root_with(&new_untouched.rows));
        assert!(
            snapshot
                .signature_to_atom
                .shares_root_with(&fabric.signature_to_atom)
        );
        assert!(snapshot.inverse[0].shares_root_with(&fabric.inverse[0]));
        assert!(snapshot.row_signature(&1024).is_some());
        assert!(fabric.row_signature(&1024).is_none());
    }

    #[test]
    fn foreign_catalog_product_class_is_rejected_even_for_same_revision() {
        let (context, registry, left_eq, _) = fixture();
        let mut first = RevisionObservableCatalog::new(&context).unwrap();
        let mut second = RevisionObservableCatalog::new(&context).unwrap();
        let first_observable = first
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let first_product = first.register_product(vec![first_observable]).unwrap();
        let second_observable = second
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let second_product = second.register_product(vec![second_observable]).unwrap();
        let foreign_component = second
            .observe_value(&registry, &context, second_observable, &Value::I64(1))
            .unwrap();
        let foreign_atom = second
            .intern_product_class(second_product, vec![foreign_component])
            .unwrap();
        let mut fabric =
            SupportAtomFabric::new(&first, first_product, vec![first_observable]).unwrap();

        assert!(matches!(
            fabric.insert(&first, 1_u64, foreign_atom, &[foreign_component]),
            Err(SupportAtomError::Observable(
                ObservableError::UnknownEqClass(_)
            ))
        ));
    }

    #[test]
    fn ordered_overlay_rejects_order_keys_that_split_one_equality_atom() {
        let (context, registry, left_eq, _) = fixture();
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let product = catalog.register_product(vec![left]).unwrap();
        let mut fabric = SupportAtomFabric::new(&catalog, product, vec![left]).unwrap();
        let class = catalog
            .observe_value(&registry, &context, left, &Value::I64(7))
            .unwrap();
        let atom = catalog.intern_product_class(product, vec![class]).unwrap();
        fabric.insert(&catalog, 10_u64, atom, &[class]).unwrap();
        fabric.insert(&catalog, 11_u64, atom, &[class]).unwrap();

        assert!(matches!(
            SupportAtomOrderedOverlay::build(&fabric, [(10_u64, 1_i64), (11_u64, 2_i64)]),
            Err(SupportAtomOverlayError::IncongruentOrderKey(candidate)) if candidate == atom
        ));
        let overlay =
            SupportAtomOrderedOverlay::build(&fabric, [(10_u64, 1_i64), (11_u64, 1_i64)]).unwrap();
        assert_eq!(overlay.order_key(atom), Some(&1));
        assert!(overlay.compatible_with(&fabric));
    }

    #[test]
    fn annotation_overlay_is_atom_scoped_and_drops_dead_atoms() {
        let (context, registry, left_eq, _) = fixture();
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let left = catalog
            .register_equivalence(&registry, &context, left_eq)
            .unwrap();
        let product = catalog.register_product(vec![left]).unwrap();
        let mut fabric = SupportAtomFabric::new(&catalog, product, vec![left]).unwrap();
        let class = catalog
            .observe_value(&registry, &context, left, &Value::I64(7))
            .unwrap();
        let atom = catalog.intern_product_class(product, vec![class]).unwrap();
        fabric.insert(&catalog, 10_u64, atom, &[class]).unwrap();
        let mut overlay = SupportAtomAnnotationOverlay::new(&fabric);
        assert_eq!(overlay.insert(&fabric, atom, 99_u64).unwrap(), None);
        assert_eq!(overlay.get(atom), Some(&99));
        fabric.remove(&10_u64).unwrap();
        overlay.retain_live(&fabric).unwrap();
        assert_eq!(overlay.get(atom), None);
    }
}
