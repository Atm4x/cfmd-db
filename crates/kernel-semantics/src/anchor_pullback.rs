use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kernel_grounded_closure::{
    GroundedAtomId, GroundedClosureError, GroundedIncidenceIndex, GroundedProgram, GroundedRule,
    solve_indexed as solve_grounded_closure,
};
use kernel_types::{EqClassId, RevisionObservableId, SemanticRevision};

use crate::observable::{CertifiedSemanticMorphism, ObservableError, RevisionObservableCatalog};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorPullbackError {
    Observable(ObservableError),
    DuplicateCoordinate(RevisionObservableId),
    UnknownCoordinate(RevisionObservableId),
    TupleArityMismatch {
        expected: usize,
        actual: usize,
    },
    WeightOverflow,
    MorphismRevisionMismatch,
    MorphismCatalogMismatch,
    UndefinedMorphismSource {
        morphism_index: usize,
        source: Vec<EqClassId>,
    },
    ConflictingAssignment {
        observable: RevisionObservableId,
        existing: EqClassId,
        proposed: EqClassId,
    },
    GroundedClosure(GroundedClosureError),
}

impl From<ObservableError> for AnchorPullbackError {
    fn from(value: ObservableError) -> Self {
        Self::Observable(value)
    }
}

impl From<GroundedClosureError> for AnchorPullbackError {
    fn from(value: GroundedClosureError) -> Self {
        Self::GroundedClosure(value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeterminantClosureStats {
    pub incidence_updates: usize,
    pub target_attempts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterminantTheory {
    revision: SemanticRevision,
    catalog_instance: u64,
    coordinate_order: Vec<RevisionObservableId>,
    coordinates: BTreeSet<RevisionObservableId>,
    morphisms: Vec<CertifiedSemanticMorphism>,
    atom_by_coordinate: BTreeMap<RevisionObservableId, GroundedAtomId>,
    coordinate_by_atom: Vec<RevisionObservableId>,
    grounded_rules: Vec<GroundedRule>,
    grounded_index: GroundedIncidenceIndex,
}

impl DeterminantTheory {
    pub fn new(
        catalog: &RevisionObservableCatalog,
        coordinates: impl IntoIterator<Item = RevisionObservableId>,
        morphisms: Vec<CertifiedSemanticMorphism>,
    ) -> Result<Self, AnchorPullbackError> {
        let mut coordinate_order = Vec::new();
        let mut coordinate_set = BTreeSet::new();
        for coordinate in coordinates {
            if coordinate_set.insert(coordinate) {
                coordinate_order.push(coordinate);
            }
        }
        for coordinate in &coordinate_set {
            catalog.definition(*coordinate)?;
        }

        let coordinate_by_atom = coordinate_order.clone();
        let atom_by_coordinate = coordinate_by_atom
            .iter()
            .enumerate()
            .map(|(index, &coordinate)| (coordinate, GroundedAtomId::new(index)))
            .collect::<BTreeMap<_, _>>();

        let mut normalized_rules = Vec::<(
            BTreeSet<RevisionObservableId>,
            BTreeSet<RevisionObservableId>,
        )>::new();
        for morphism in &morphisms {
            if morphism.revision() != catalog.revision() {
                return Err(AnchorPullbackError::MorphismRevisionMismatch);
            }
            if morphism.catalog_instance() != catalog.catalog_instance() {
                return Err(AnchorPullbackError::MorphismCatalogMismatch);
            }
            let source = morphism.source().iter().copied().collect::<BTreeSet<_>>();
            let target = morphism.target().iter().copied().collect::<BTreeSet<_>>();
            for coordinate in source.iter().chain(&target) {
                if !coordinate_set.contains(coordinate) {
                    return Err(AnchorPullbackError::UnknownCoordinate(*coordinate));
                }
            }
            if !target.is_subset(&source) {
                normalized_rules.push((source, target));
            }
        }
        normalized_rules.sort();
        normalized_rules.dedup();

        let mut grounded_rules = Vec::new();
        for (source, targets) in &normalized_rules {
            let body = source
                .iter()
                .map(|coordinate| atom_by_coordinate[coordinate])
                .collect::<Vec<_>>();
            for target in targets {
                grounded_rules.push(GroundedRule::new(
                    body.iter().copied(),
                    atom_by_coordinate[target],
                ));
            }
        }
        let empty_program =
            GroundedProgram::new(coordinate_by_atom.len(), [], grounded_rules.clone())?;
        let grounded_index = GroundedIncidenceIndex::compile(&empty_program);

        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance(),
            coordinate_order,
            coordinates: coordinate_set,
            morphisms,
            atom_by_coordinate,
            coordinate_by_atom,
            grounded_rules,
            grounded_index,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub fn coordinates(&self) -> &BTreeSet<RevisionObservableId> {
        &self.coordinates
    }

    #[must_use]
    pub fn coordinate_order(&self) -> &[RevisionObservableId] {
        &self.coordinate_order
    }

    #[must_use]
    pub fn morphisms(&self) -> &[CertifiedSemanticMorphism] {
        &self.morphisms
    }

    pub fn closure(
        &self,
        seed: &BTreeSet<RevisionObservableId>,
    ) -> Result<BTreeSet<RevisionObservableId>, AnchorPullbackError> {
        self.closure_with_stats(seed).map(|(closure, _)| closure)
    }

    pub fn closure_with_stats(
        &self,
        seed: &BTreeSet<RevisionObservableId>,
    ) -> Result<(BTreeSet<RevisionObservableId>, DeterminantClosureStats), AnchorPullbackError>
    {
        self.validate_coordinate_set(seed)?;
        let grounded_seed = seed
            .iter()
            .map(|coordinate| self.atom_by_coordinate[coordinate])
            .collect::<BTreeSet<_>>();
        let seed_incidence_count = self
            .grounded_rules
            .iter()
            .map(|rule| {
                rule.body()
                    .iter()
                    .filter(|atom| grounded_seed.contains(atom))
                    .count()
            })
            .sum::<usize>();
        let program = GroundedProgram::new(
            self.coordinate_by_atom.len(),
            grounded_seed,
            self.grounded_rules.clone(),
        )?;
        let (certificate, work) = solve_grounded_closure(&program, &self.grounded_index);
        let closure = certificate
            .live_atoms()
            .map(|atom| self.coordinate_by_atom[atom.index()])
            .collect::<BTreeSet<_>>();
        Ok((
            closure,
            DeterminantClosureStats {
                incidence_updates: work.incidence_updates.saturating_sub(seed_incidence_count),
                target_attempts: work.rule_fires,
            },
        ))
    }

    pub fn stable_inclusion_minimal_generator(
        &self,
    ) -> Result<BTreeSet<RevisionObservableId>, AnchorPullbackError> {
        let mut basis = self.coordinates.clone();
        for coordinate in self.coordinate_order.iter().rev() {
            let mut trial = basis.clone();
            trial.remove(coordinate);
            if self.closure(&trial)?.is_superset(&self.coordinates) {
                basis = trial;
            }
        }
        Ok(basis)
    }

    pub fn saturate_assignment(
        &self,
        catalog: &RevisionObservableCatalog,
        seed: &BTreeMap<RevisionObservableId, EqClassId>,
    ) -> Result<BTreeMap<RevisionObservableId, EqClassId>, AnchorPullbackError> {
        self.ensure_catalog(catalog)?;
        let mut assignment = seed.clone();
        for (&observable, &class) in &assignment {
            if !self.coordinates.contains(&observable) {
                return Err(AnchorPullbackError::UnknownCoordinate(observable));
            }
            let record = catalog.class_record(class)?;
            if record.observable != observable {
                return Err(ObservableError::ForeignEqClass {
                    class,
                    expected: observable,
                    actual: record.observable,
                }
                .into());
            }
        }

        let mut remaining = Vec::with_capacity(self.morphisms.len());
        let mut dependents = BTreeMap::<RevisionObservableId, Vec<usize>>::new();
        let mut fired = vec![false; self.morphisms.len()];
        let mut queue = VecDeque::new();

        for (index, morphism) in self.morphisms.iter().enumerate() {
            let unique_source = morphism.source().iter().copied().collect::<BTreeSet<_>>();
            let missing = unique_source
                .iter()
                .filter(|coord| !assignment.contains_key(coord))
                .count();
            remaining.push(missing);
            for coordinate in unique_source {
                dependents.entry(coordinate).or_default().push(index);
            }
        }

        for index in 0..self.morphisms.len() {
            if remaining[index] == 0 {
                fired[index] = true;
                fire_morphism(index, &self.morphisms[index], &mut assignment, &mut queue)?;
            }
        }

        while let Some(coordinate) = queue.pop_front() {
            let Some(rule_ids) = dependents.get(&coordinate) else {
                continue;
            };
            for &index in rule_ids {
                if fired[index] || remaining[index] == 0 {
                    continue;
                }
                remaining[index] -= 1;
                if remaining[index] == 0 {
                    fired[index] = true;
                    fire_morphism(index, &self.morphisms[index], &mut assignment, &mut queue)?;
                }
            }
        }

        Ok(assignment)
    }

    fn validate_coordinate_set(
        &self,
        coordinates: &BTreeSet<RevisionObservableId>,
    ) -> Result<(), AnchorPullbackError> {
        if let Some(unknown) = coordinates
            .iter()
            .find(|coordinate| !self.coordinates.contains(coordinate))
        {
            return Err(AnchorPullbackError::UnknownCoordinate(*unknown));
        }
        Ok(())
    }

    fn ensure_catalog(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<(), AnchorPullbackError> {
        if self.revision != catalog.revision() {
            return Err(AnchorPullbackError::MorphismRevisionMismatch);
        }
        if self.catalog_instance != catalog.catalog_instance() {
            return Err(AnchorPullbackError::MorphismCatalogMismatch);
        }
        Ok(())
    }
}

fn fire_morphism(
    morphism_index: usize,
    morphism: &CertifiedSemanticMorphism,
    assignment: &mut BTreeMap<RevisionObservableId, EqClassId>,
    queue: &mut VecDeque<RevisionObservableId>,
) -> Result<(), AnchorPullbackError> {
    let source = morphism
        .source()
        .iter()
        .map(|observable| assignment[observable])
        .collect::<Vec<_>>();
    let Some(target) = morphism.image(&source) else {
        return Err(AnchorPullbackError::UndefinedMorphismSource {
            morphism_index,
            source,
        });
    };
    for (&observable, &class) in morphism.target().iter().zip(target) {
        match assignment.get(&observable).copied() {
            Some(existing) if existing != class => {
                return Err(AnchorPullbackError::ConflictingAssignment {
                    observable,
                    existing,
                    proposed: class,
                });
            }
            Some(_) => {}
            None => {
                assignment.insert(observable, class);
                queue.push_back(observable);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionFiniteMeasure {
    revision: SemanticRevision,
    catalog_instance: u64,
    coordinates: Vec<RevisionObservableId>,
    rows: BTreeMap<Vec<EqClassId>, u64>,
}

impl RevisionFiniteMeasure {
    pub fn new(
        catalog: &RevisionObservableCatalog,
        coordinates: Vec<RevisionObservableId>,
        rows: impl IntoIterator<Item = (Vec<EqClassId>, u64)>,
    ) -> Result<Self, AnchorPullbackError> {
        let mut seen = BTreeSet::new();
        for coordinate in &coordinates {
            catalog.definition(*coordinate)?;
            if !seen.insert(*coordinate) {
                return Err(AnchorPullbackError::DuplicateCoordinate(*coordinate));
            }
        }

        let mut merged = BTreeMap::<Vec<EqClassId>, u64>::new();
        for (tuple, weight) in rows {
            if tuple.len() != coordinates.len() {
                return Err(AnchorPullbackError::TupleArityMismatch {
                    expected: coordinates.len(),
                    actual: tuple.len(),
                });
            }
            for (&coordinate, &class) in coordinates.iter().zip(&tuple) {
                let record = catalog.class_record(class)?;
                if record.observable != coordinate {
                    return Err(ObservableError::ForeignEqClass {
                        class,
                        expected: coordinate,
                        actual: record.observable,
                    }
                    .into());
                }
            }
            if weight == 0 {
                continue;
            }
            let entry = merged.entry(tuple).or_default();
            *entry = entry
                .checked_add(weight)
                .ok_or(AnchorPullbackError::WeightOverflow)?;
        }

        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance(),
            coordinates,
            rows: merged,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub fn coordinates(&self) -> &[RevisionObservableId] {
        &self.coordinates
    }

    #[must_use]
    pub fn rows(&self) -> &BTreeMap<Vec<EqClassId>, u64> {
        &self.rows
    }

    pub fn stable_anchor_basis(
        &self,
    ) -> Result<BTreeSet<RevisionObservableId>, AnchorPullbackError> {
        let all = self.coordinates.iter().copied().collect::<BTreeSet<_>>();
        let mut basis = all.clone();
        for coordinate in self.coordinates.iter().rev() {
            let mut trial = basis.clone();
            trial.remove(coordinate);
            if self.projection_is_injective(&trial)? {
                basis = trial;
            }
        }
        Ok(basis)
    }

    pub fn determinant_morphism(
        &self,
        catalog: &RevisionObservableCatalog,
        source: Vec<RevisionObservableId>,
        target: Vec<RevisionObservableId>,
    ) -> Result<Option<CertifiedSemanticMorphism>, AnchorPullbackError> {
        self.ensure_catalog(catalog)?;
        let source_positions = self.ordered_positions(&source)?;
        let target_positions = self.ordered_positions(&target)?;
        let mut mapping = BTreeMap::<Vec<EqClassId>, Vec<EqClassId>>::new();
        for tuple in self.rows.keys() {
            let source_key = source_positions
                .iter()
                .map(|&position| tuple[position])
                .collect::<Vec<_>>();
            let target_value = target_positions
                .iter()
                .map(|&position| tuple[position])
                .collect::<Vec<_>>();
            if let Some(previous) = mapping.insert(source_key, target_value.clone())
                && previous != target_value
            {
                return Ok(None);
            }
        }
        CertifiedSemanticMorphism::from_revision_observations(catalog, source, target, mapping)
            .map(Some)
            .map_err(Into::into)
    }

    pub fn anchor_factorization(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<AnchorMeasureState, AnchorPullbackError> {
        self.ensure_catalog(catalog)?;
        let basis_set = self.stable_anchor_basis()?;
        let basis = self
            .coordinates
            .iter()
            .copied()
            .filter(|coordinate| basis_set.contains(coordinate))
            .collect::<Vec<_>>();
        let target = self
            .coordinates
            .iter()
            .copied()
            .filter(|coordinate| !basis_set.contains(coordinate))
            .collect::<Vec<_>>();
        let basis_positions = self.positions(&basis_set)?;
        let target_set = target.iter().copied().collect::<BTreeSet<_>>();
        let target_positions = self.positions(&target_set)?;

        let mut measure = BTreeMap::new();
        for (tuple, weight) in &self.rows {
            let anchor = basis_positions
                .iter()
                .map(|&position| tuple[position])
                .collect::<Vec<_>>();
            let reconstruction = target_positions
                .iter()
                .map(|&position| tuple[position])
                .collect::<Vec<_>>();
            let previous = measure.insert(
                anchor,
                AnchorMeasureEntry {
                    reconstruction,
                    weight: *weight,
                },
            );
            debug_assert!(previous.is_none());
        }

        Ok(AnchorMeasureState {
            revision: self.revision,
            catalog_instance: self.catalog_instance,
            coordinates: self.coordinates.clone(),
            basis,
            target,
            measure,
        })
    }

    fn projection_is_injective(
        &self,
        subset: &BTreeSet<RevisionObservableId>,
    ) -> Result<bool, AnchorPullbackError> {
        let positions = self.positions(subset)?;
        let mut projections = BTreeSet::new();
        for tuple in self.rows.keys() {
            let projection = positions
                .iter()
                .map(|&position| tuple[position])
                .collect::<Vec<_>>();
            if !projections.insert(projection) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn positions(
        &self,
        subset: &BTreeSet<RevisionObservableId>,
    ) -> Result<Vec<usize>, AnchorPullbackError> {
        if let Some(unknown) = subset
            .iter()
            .find(|coordinate| !self.coordinates.contains(coordinate))
        {
            return Err(AnchorPullbackError::UnknownCoordinate(*unknown));
        }
        Ok(self
            .coordinates
            .iter()
            .enumerate()
            .filter_map(|(position, coordinate)| subset.contains(coordinate).then_some(position))
            .collect())
    }

    fn ordered_positions(
        &self,
        observables: &[RevisionObservableId],
    ) -> Result<Vec<usize>, AnchorPullbackError> {
        let mut seen = BTreeSet::new();
        observables
            .iter()
            .map(|observable| {
                if !seen.insert(*observable) {
                    return Err(AnchorPullbackError::DuplicateCoordinate(*observable));
                }
                self.coordinates
                    .iter()
                    .position(|candidate| candidate == observable)
                    .ok_or(AnchorPullbackError::UnknownCoordinate(*observable))
            })
            .collect()
    }

    fn ensure_catalog(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<(), AnchorPullbackError> {
        if self.revision != catalog.revision() {
            return Err(AnchorPullbackError::MorphismRevisionMismatch);
        }
        if self.catalog_instance != catalog.catalog_instance() {
            return Err(AnchorPullbackError::MorphismCatalogMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorMeasureEntry {
    pub reconstruction: Vec<EqClassId>,
    pub weight: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorMeasureState {
    revision: SemanticRevision,
    catalog_instance: u64,
    coordinates: Vec<RevisionObservableId>,
    basis: Vec<RevisionObservableId>,
    target: Vec<RevisionObservableId>,
    measure: BTreeMap<Vec<EqClassId>, AnchorMeasureEntry>,
}

impl AnchorMeasureState {
    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub fn coordinates(&self) -> &[RevisionObservableId] {
        &self.coordinates
    }

    #[must_use]
    pub fn basis(&self) -> &[RevisionObservableId] {
        &self.basis
    }

    #[must_use]
    pub fn target(&self) -> &[RevisionObservableId] {
        &self.target
    }

    #[must_use]
    pub fn measure(&self) -> &BTreeMap<Vec<EqClassId>, AnchorMeasureEntry> {
        &self.measure
    }

    pub fn reconstruction_morphism(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<CertifiedSemanticMorphism, AnchorPullbackError> {
        self.ensure_catalog(catalog)?;
        CertifiedSemanticMorphism::from_anchor_reconstruction(
            catalog,
            self.basis.clone(),
            self.target.clone(),
            self.measure
                .iter()
                .map(|(basis, entry)| (basis.clone(), entry.reconstruction.clone())),
        )
        .map_err(Into::into)
    }

    #[must_use]
    pub fn reconstructed_measure(&self) -> BTreeMap<Vec<EqClassId>, u64> {
        let basis_positions = self
            .coordinates
            .iter()
            .enumerate()
            .filter_map(|(position, coordinate)| {
                self.basis.contains(coordinate).then_some(position)
            })
            .collect::<Vec<_>>();
        let target_positions = self
            .coordinates
            .iter()
            .enumerate()
            .filter_map(|(position, coordinate)| {
                self.target.contains(coordinate).then_some(position)
            })
            .collect::<Vec<_>>();
        let mut rows = BTreeMap::new();
        for (basis, entry) in &self.measure {
            let mut row = vec![EqClassId::default(); self.coordinates.len()];
            for (&position, &class) in basis_positions.iter().zip(basis) {
                row[position] = class;
            }
            for (&position, &class) in target_positions.iter().zip(&entry.reconstruction) {
                row[position] = class;
            }
            rows.insert(row, entry.weight);
        }
        rows
    }

    fn ensure_catalog(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<(), AnchorPullbackError> {
        if self.revision != catalog.revision() {
            return Err(AnchorPullbackError::MorphismRevisionMismatch);
        }
        if self.catalog_instance != catalog.catalog_instance() {
            return Err(AnchorPullbackError::MorphismCatalogMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterminantBranchFreeCertificate {
    anchor_index: usize,
    seed: BTreeSet<RevisionObservableId>,
    closure: BTreeSet<RevisionObservableId>,
}

impl DeterminantBranchFreeCertificate {
    #[must_use]
    pub const fn anchor_index(&self) -> usize {
        self.anchor_index
    }

    #[must_use]
    pub fn seed(&self) -> &BTreeSet<RevisionObservableId> {
        &self.seed
    }

    #[must_use]
    pub fn closure(&self) -> &BTreeSet<RevisionObservableId> {
        &self.closure
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorPullbackNormalForm {
    revision: SemanticRevision,
    catalog_instance: u64,
    anchors: Vec<AnchorMeasureState>,
    determinant_theory: DeterminantTheory,
    output_observables: Vec<RevisionObservableId>,
}

impl AnchorPullbackNormalForm {
    pub fn new(
        catalog: &RevisionObservableCatalog,
        factors: Vec<RevisionFiniteMeasure>,
        certified_morphisms: Vec<CertifiedSemanticMorphism>,
        output_observables: Vec<RevisionObservableId>,
    ) -> Result<Self, AnchorPullbackError> {
        for output in &output_observables {
            catalog.definition(*output)?;
        }

        let mut anchors = Vec::with_capacity(factors.len());
        let mut coordinate_order = Vec::new();
        let mut seen_coordinates = BTreeSet::new();
        for output in &output_observables {
            if seen_coordinates.insert(*output) {
                coordinate_order.push(*output);
            }
        }
        let mut morphisms = certified_morphisms;

        for factor in factors {
            factor.ensure_catalog(catalog)?;
            for coordinate in factor.coordinates() {
                if seen_coordinates.insert(*coordinate) {
                    coordinate_order.push(*coordinate);
                }
            }
            let anchor = factor.anchor_factorization(catalog)?;
            morphisms.push(anchor.reconstruction_morphism(catalog)?);
            anchors.push(anchor);
        }
        for morphism in &morphisms {
            for coordinate in morphism.source().iter().chain(morphism.target()) {
                if seen_coordinates.insert(*coordinate) {
                    coordinate_order.push(*coordinate);
                }
            }
        }

        let determinant_theory = DeterminantTheory::new(catalog, coordinate_order, morphisms)?;
        Ok(Self {
            revision: catalog.revision(),
            catalog_instance: catalog.catalog_instance(),
            anchors,
            determinant_theory,
            output_observables,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> SemanticRevision {
        self.revision
    }

    #[must_use]
    pub fn anchors(&self) -> &[AnchorMeasureState] {
        &self.anchors
    }

    #[must_use]
    pub const fn determinant_theory(&self) -> &DeterminantTheory {
        &self.determinant_theory
    }

    #[must_use]
    pub fn output_observables(&self) -> &[RevisionObservableId] {
        &self.output_observables
    }

    pub fn branch_free_from_anchor(
        &self,
        anchor_index: usize,
    ) -> Result<Option<DeterminantBranchFreeCertificate>, AnchorPullbackError> {
        let Some(anchor) = self.anchors.get(anchor_index) else {
            return Ok(None);
        };
        let seed = anchor.basis.iter().copied().collect::<BTreeSet<_>>();
        let closure = self.determinant_theory.closure(&seed)?;
        if closure.is_superset(self.determinant_theory.coordinates()) {
            Ok(Some(DeterminantBranchFreeCertificate {
                anchor_index,
                seed,
                closure,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn ensure_catalog(
        &self,
        catalog: &RevisionObservableCatalog,
    ) -> Result<(), AnchorPullbackError> {
        if self.revision != catalog.revision() {
            return Err(AnchorPullbackError::MorphismRevisionMismatch);
        }
        if self.catalog_instance != catalog.catalog_instance() {
            return Err(AnchorPullbackError::MorphismCatalogMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use kernel_model::Value;
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;
    use crate::observable::SemanticMorphismCertificate;
    use crate::{EquivalenceModule, SemanticRegistry};

    fn fixture(count: usize) -> (SemanticContext, SemanticRegistry, Vec<SemanticId>) {
        let ids = (0..count)
            .map(|index| SemanticId::new(90_000 + index as u128))
            .collect::<Vec<_>>();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(90_000));
        for id in &ids {
            environment.pin_module(*id, digest);
        }
        (
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(90_000)),
                environment,
            },
            registry,
            ids,
        )
    }

    fn observables(
        count: usize,
    ) -> (
        SemanticContext,
        SemanticRegistry,
        RevisionObservableCatalog,
        Vec<RevisionObservableId>,
    ) {
        let (context, registry, ids) = fixture(count);
        let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
        let observables = ids
            .into_iter()
            .map(|id| {
                catalog
                    .register_equivalence(&registry, &context, id)
                    .unwrap()
            })
            .collect();
        (context, registry, catalog, observables)
    }

    fn class(
        catalog: &mut RevisionObservableCatalog,
        registry: &SemanticRegistry,
        context: &SemanticContext,
        observable: RevisionObservableId,
        value: i64,
    ) -> EqClassId {
        catalog
            .observe_value(registry, context, observable, &Value::I64(value))
            .unwrap()
    }

    #[test]
    fn determinant_closure_handles_true_hypercycle_without_unary_scc_assumption() {
        let (context, registry, mut catalog, obs) = observables(3);
        let [a, b, c] = obs.as_slice() else {
            unreachable!()
        };
        let a1 = class(&mut catalog, &registry, &context, *a, 1);
        let b1 = class(&mut catalog, &registry, &context, *b, 2);
        let c1 = class(&mut catalog, &registry, &context, *c, 3);

        let rules = vec![
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*a, *b],
                vec![*c],
                [(vec![a1, b1], vec![c1])],
            )
            .unwrap(),
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*a, *c],
                vec![*b],
                [(vec![a1, c1], vec![b1])],
            )
            .unwrap(),
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*b, *c],
                vec![*a],
                [(vec![b1, c1], vec![a1])],
            )
            .unwrap(),
        ];
        let theory = DeterminantTheory::new(&catalog, obs.clone(), rules).unwrap();
        let basis = theory.stable_inclusion_minimal_generator().unwrap();
        assert_eq!(basis.len(), 2);
        assert_eq!(theory.closure(&basis).unwrap(), obs.into_iter().collect());
        for coordinate in &basis {
            let mut strict = basis.clone();
            strict.remove(coordinate);
            assert_ne!(theory.closure(&strict).unwrap(), *theory.coordinates());
        }
    }

    #[test]
    fn closure_worklist_scales_with_incidence_not_rule_scan_rounds() {
        let (context, registry, mut catalog, obs) = observables(128);
        let mut classes = Vec::new();
        for (index, observable) in obs.iter().enumerate() {
            classes.push(class(
                &mut catalog,
                &registry,
                &context,
                *observable,
                i64::try_from(index).unwrap(),
            ));
        }
        let rules = (0..127)
            .rev()
            .map(|index| {
                CertifiedSemanticMorphism::from_revision_observations(
                    &catalog,
                    vec![obs[index]],
                    vec![obs[index + 1]],
                    [(vec![classes[index]], vec![classes[index + 1]])],
                )
                .unwrap()
            })
            .collect();
        let theory = DeterminantTheory::new(&catalog, obs.clone(), rules).unwrap();
        let seed = BTreeSet::from([obs[0]]);
        let (closure, stats) = theory.closure_with_stats(&seed).unwrap();
        assert_eq!(closure.len(), 128);
        assert_eq!(stats.incidence_updates, 126);
        assert_eq!(stats.target_attempts, 127);
    }

    #[test]
    fn weighted_anchor_factorization_is_lossless_and_inclusion_minimal() {
        let (context, registry, mut catalog, obs) = observables(3);
        let [a, b, c] = obs.as_slice() else {
            unreachable!()
        };
        let rows = [
            (
                vec![
                    class(&mut catalog, &registry, &context, *a, 1),
                    class(&mut catalog, &registry, &context, *b, 10),
                    class(&mut catalog, &registry, &context, *c, 100),
                ],
                2,
            ),
            (
                vec![
                    class(&mut catalog, &registry, &context, *a, 2),
                    class(&mut catalog, &registry, &context, *b, 10),
                    class(&mut catalog, &registry, &context, *c, 200),
                ],
                5,
            ),
        ];
        let factor = RevisionFiniteMeasure::new(&catalog, obs.clone(), rows.clone()).unwrap();
        let anchor = factor.anchor_factorization(&catalog).unwrap();
        assert_eq!(anchor.reconstructed_measure(), BTreeMap::from(rows));
        let basis = anchor.basis().iter().copied().collect::<BTreeSet<_>>();
        for coordinate in &basis {
            let mut strict = basis.clone();
            strict.remove(coordinate);
            assert!(!factor.projection_is_injective(&strict).unwrap());
        }
        assert!(matches!(
            anchor
                .reconstruction_morphism(&catalog)
                .unwrap()
                .certificate(),
            SemanticMorphismCertificate::AnchorReconstruction
        ));
    }

    #[test]
    fn finite_factor_derives_ordered_hyper_determinant_morphism_without_unary_special_case() {
        let (context, registry, mut catalog, obs) = observables(4);
        let [a, b, c, d] = obs.as_slice() else {
            unreachable!()
        };
        let rows = [
            vec![
                class(&mut catalog, &registry, &context, *a, 1),
                class(&mut catalog, &registry, &context, *b, 2),
                class(&mut catalog, &registry, &context, *c, 3),
                class(&mut catalog, &registry, &context, *d, 4),
            ],
            vec![
                class(&mut catalog, &registry, &context, *a, 5),
                class(&mut catalog, &registry, &context, *b, 6),
                class(&mut catalog, &registry, &context, *c, 7),
                class(&mut catalog, &registry, &context, *d, 8),
            ],
        ];
        let factor = RevisionFiniteMeasure::new(
            &catalog,
            obs.clone(),
            rows.iter().cloned().map(|row| (row, 1)),
        )
        .unwrap();
        let morphism = factor
            .determinant_morphism(&catalog, vec![*b, *a], vec![*d, *c])
            .unwrap()
            .unwrap();
        assert_eq!(
            morphism.image(&[rows[0][1], rows[0][0]]),
            Some([rows[0][3], rows[0][2]].as_slice())
        );
        assert_eq!(morphism.source(), &[*b, *a]);
        assert_eq!(morphism.target(), &[*d, *c]);
    }

    #[test]
    fn finite_factor_rejects_non_functional_projection_and_preserves_duplicate_weight() {
        let (context, registry, mut catalog, obs) = observables(2);
        let [a, b] = obs.as_slice() else {
            unreachable!()
        };
        let a1 = class(&mut catalog, &registry, &context, *a, 1);
        let b1 = class(&mut catalog, &registry, &context, *b, 10);
        let b2 = class(&mut catalog, &registry, &context, *b, 20);
        let factor = RevisionFiniteMeasure::new(
            &catalog,
            obs.clone(),
            [(vec![a1, b1], 2), (vec![a1, b1], 3), (vec![a1, b2], 7)],
        )
        .unwrap();
        assert_eq!(factor.rows().get(&vec![a1, b1]), Some(&5));
        assert!(
            factor
                .determinant_morphism(&catalog, vec![*a], vec![*b])
                .unwrap()
                .is_none()
        );
        let anchor = factor.anchor_factorization(&catalog).unwrap();
        assert_eq!(anchor.reconstructed_measure(), factor.rows().clone());
    }

    #[test]
    fn apnf_is_bound_to_one_observable_catalog_realization() {
        let (context, registry, mut catalog, obs) = observables(1);
        let [a] = obs.as_slice() else { unreachable!() };
        let a1 = class(&mut catalog, &registry, &context, *a, 1);
        let factor = RevisionFiniteMeasure::new(&catalog, vec![*a], [(vec![a1], 1)]).unwrap();
        let apnf =
            AnchorPullbackNormalForm::new(&catalog, vec![factor], Vec::new(), vec![*a]).unwrap();

        let foreign = RevisionObservableCatalog::new(&context).unwrap();
        assert_eq!(
            apnf.ensure_catalog(&foreign),
            Err(AnchorPullbackError::MorphismCatalogMismatch)
        );
    }

    #[test]
    fn value_saturation_is_confluent_and_rejects_conflicting_exact_images() {
        let (context, registry, mut catalog, obs) = observables(3);
        let [a, b, c] = obs.as_slice() else {
            unreachable!()
        };
        let a1 = class(&mut catalog, &registry, &context, *a, 1);
        let b1 = class(&mut catalog, &registry, &context, *b, 2);
        let c1 = class(&mut catalog, &registry, &context, *c, 3);
        let c2 = class(&mut catalog, &registry, &context, *c, 4);
        let ab = CertifiedSemanticMorphism::from_revision_observations(
            &catalog,
            vec![*a],
            vec![*b],
            [(vec![a1], vec![b1])],
        )
        .unwrap();
        let bc = CertifiedSemanticMorphism::from_revision_observations(
            &catalog,
            vec![*b],
            vec![*c],
            [(vec![b1], vec![c1])],
        )
        .unwrap();
        let theory = DeterminantTheory::new(&catalog, obs.clone(), vec![bc, ab]).unwrap();
        let saturated = theory
            .saturate_assignment(&catalog, &BTreeMap::from([(*a, a1)]))
            .unwrap();
        assert_eq!(saturated.get(c), Some(&c1));
        assert_eq!(
            theory.saturate_assignment(&catalog, &BTreeMap::from([(*a, a1), (*c, c2)])),
            Err(AnchorPullbackError::ConflictingAssignment {
                observable: *c,
                existing: c2,
                proposed: c1,
            })
        );
    }

    #[test]
    fn apnf_keeps_redundant_direct_morphism_and_can_prove_branch_free_anchor() {
        let (context, registry, mut catalog, obs) = observables(3);
        let [a, b, c] = obs.as_slice() else {
            unreachable!()
        };
        let a1 = class(&mut catalog, &registry, &context, *a, 1);
        let b1 = class(&mut catalog, &registry, &context, *b, 2);
        let c1 = class(&mut catalog, &registry, &context, *c, 3);
        let factor = RevisionFiniteMeasure::new(&catalog, vec![*a], [(vec![a1], 1)]).unwrap();
        let morphisms = vec![
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*a],
                vec![*b],
                [(vec![a1], vec![b1])],
            )
            .unwrap(),
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*b],
                vec![*c],
                [(vec![b1], vec![c1])],
            )
            .unwrap(),
            CertifiedSemanticMorphism::from_revision_observations(
                &catalog,
                vec![*a],
                vec![*c],
                [(vec![a1], vec![c1])],
            )
            .unwrap(),
        ];
        let apnf =
            AnchorPullbackNormalForm::new(&catalog, vec![factor], morphisms, vec![*c]).unwrap();
        assert_eq!(apnf.determinant_theory().morphisms().len(), 4);
        let certificate = apnf.branch_free_from_anchor(0).unwrap().unwrap();
        assert!(certificate.seed().is_empty());
        assert!(
            certificate
                .closure()
                .is_superset(&BTreeSet::from([*a, *b, *c]))
        );
    }
}
