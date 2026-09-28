use super::{RelQueryError, RelType, Row, value_shape_matches_type};

/// One finite carrier atom of a grounded positive recursive relational query.
/// `seed_multiplicity` is the exact non-recursive Bag contribution at the
/// fixpoint base. Runtime row handles are deliberately absent: this is a
/// semantic/query object, not physical identity authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveRecursiveRowAtom {
    pub row: Row,
    pub seed_multiplicity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositiveRecursiveRowRule {
    pub body: Vec<usize>,
    pub head: usize,
    pub coefficient: u64,
}

/// Query-level positive recursion leaf after APNF/SAMF grounding.
///
/// The finite carrier is explicit. Rule bodies may repeat an atom because Bag
/// proof-tree multiplicity distinguishes duplicate recursive occurrences.
/// Evaluation returns compact `N∞` weights and never expands a large or
/// infinite multiplicity into repeated rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixpointCall {
    pub result_type: RelType,
    pub atoms: Vec<PositiveRecursiveRowAtom>,
    pub rules: Vec<PositiveRecursiveRowRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRecursiveBag {
    entries: Vec<(Row, kernel_fixpoint::NaturalInfinity)>,
}

impl CompactRecursiveBag {
    #[must_use]
    pub fn entries(&self) -> &[(Row, kernel_fixpoint::NaturalInfinity)] {
        &self.entries
    }

    #[must_use]
    pub fn has_infinite_multiplicity(&self) -> bool {
        self.entries.iter().any(|(_, weight)| weight.is_infinite())
    }

    pub fn require_finite(
        &self,
    ) -> Result<&[(Row, kernel_fixpoint::NaturalInfinity)], RelQueryError> {
        if self.has_infinite_multiplicity() {
            return Err(RelQueryError::NonFiniteRecursiveMultiplicity);
        }
        Ok(&self.entries)
    }
}

impl FixpointCall {
    pub fn typecheck(
        &self,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        if !matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Bag { .. }
        ) {
            return Err(RelQueryError::TypeMismatch);
        }
        for atom in &self.atoms {
            if atom.row.len() != self.result_type.columns.len()
                || atom
                    .row
                    .iter()
                    .zip(&self.result_type.columns)
                    .any(|(value, ty)| !value_shape_matches_type(value, ty))
            {
                return Err(RelQueryError::TypeMismatch);
            }
        }
        for rule in &self.rules {
            if rule.head >= self.atoms.len()
                || rule.body.iter().any(|&atom| atom >= self.atoms.len())
            {
                return Err(RelQueryError::RecursiveAtomOutsideCarrier);
            }
        }
        Ok(self.result_type.clone())
    }

    pub fn evaluate_compact(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<CompactRecursiveBag, RelQueryError> {
        self.typecheck(context, registry)?;
        let seed_multiplicity = self
            .atoms
            .iter()
            .map(|atom| atom.seed_multiplicity)
            .collect::<Vec<_>>();
        let rules = self
            .rules
            .iter()
            .map(|rule| {
                kernel_fixpoint::PositiveBagRule::new(
                    rule.body
                        .iter()
                        .copied()
                        .map(kernel_fixpoint::GroundedAtomId::new),
                    kernel_fixpoint::GroundedAtomId::new(rule.head),
                    rule.coefficient,
                )
            })
            .collect();
        let program =
            kernel_fixpoint::PositiveBagProgram::new(self.atoms.len(), seed_multiplicity, rules)?;
        let certificate = kernel_fixpoint::solve_positive_bag(&program)?;
        kernel_fixpoint::check_positive_bag(&program, &certificate)?;
        let entries = self
            .atoms
            .iter()
            .enumerate()
            .filter_map(|(index, atom)| {
                let weight = certificate
                    .multiplicity(kernel_fixpoint::GroundedAtomId::new(index))?
                    .clone();
                (!matches!(weight, kernel_fixpoint::NaturalInfinity::Finite(ref n) if n.is_zero()))
                    .then(|| (atom.row.clone(), weight))
            })
            .collect();
        Ok(CompactRecursiveBag { entries })
    }
}
