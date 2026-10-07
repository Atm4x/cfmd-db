use std::collections::{BTreeMap, BTreeSet};

use crate::RelExpr;
use kernel_proof::{CertificateChecker, CheckedCertificate, verify_certificate};
use kernel_schema::{RelationSemantics, SemanticContext};
use kernel_semantics::{CanonicalEqKey, SemanticRegistry};
use kernel_types::SemanticId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Var {
    id: u16,
    equivalence: SemanticId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Term {
    Var(Var),
    Const {
        equivalence: SemanticId,
        key: CanonicalEqKey,
    },
}

impl Term {
    const fn equivalence(&self) -> SemanticId {
        match self {
            Self::Var(var) => var.equivalence,
            Self::Const { equivalence, .. } => *equivalence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Atom {
    relation: SemanticId,
    args: Vec<Term>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConjunctiveQuery {
    head: Vec<Term>,
    atoms: Vec<Atom>,
}

impl ConjunctiveQuery {
    fn variables(&self) -> BTreeSet<Var> {
        self.head
            .iter()
            .chain(self.atoms.iter().flat_map(|atom| atom.args.iter()))
            .filter_map(|term| match term {
                Term::Var(var) => Some(*var),
                Term::Const { .. } => None,
            })
            .collect()
    }

    fn is_safe(&self) -> bool {
        let body = self
            .atoms
            .iter()
            .flat_map(|atom| atom.args.iter())
            .filter_map(|term| match term {
                Term::Var(var) => Some(*var),
                Term::Const { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        self.head.iter().all(|term| match term {
            Term::Var(var) => body.contains(var),
            Term::Const { .. } => true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HomomorphismCertificate {
    variable_map: BTreeMap<Var, Var>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CqEquivalenceCertificate {
    left_to_right: HomomorphismCertificate,
    right_to_left: HomomorphismCertificate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CqEquivalenceSpec {
    left: ConjunctiveQuery,
    right: ConjunctiveQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CqProofError {
    UnsafeQuery,
    HeadArityMismatch,
    InvalidHomomorphism,
}

struct CqEquivalenceChecker;

impl CertificateChecker for CqEquivalenceChecker {
    type Spec = CqEquivalenceSpec;
    type Certificate = CqEquivalenceCertificate;
    type Error = CqProofError;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        if !spec.left.is_safe() || !spec.right.is_safe() {
            return Err(CqProofError::UnsafeQuery);
        }
        if spec.left.head.len() != spec.right.head.len() {
            return Err(CqProofError::HeadArityMismatch);
        }
        verify_homomorphism(&spec.left, &spec.right, &certificate.left_to_right)?;
        verify_homomorphism(&spec.right, &spec.left, &certificate.right_to_left)?;
        Ok(())
    }
}

fn mapped_term(term: &Term, mapping: &BTreeMap<Var, Var>) -> Option<Term> {
    match term {
        Term::Var(var) => mapping.get(var).copied().map(Term::Var),
        Term::Const { equivalence, key } => Some(Term::Const {
            equivalence: *equivalence,
            key: key.clone(),
        }),
    }
}

fn verify_homomorphism(
    source: &ConjunctiveQuery,
    target: &ConjunctiveQuery,
    certificate: &HomomorphismCertificate,
) -> Result<(), CqProofError> {
    let source_vars = source.variables();
    let target_vars = target.variables();
    if certificate.variable_map.len() != source_vars.len()
        || certificate
            .variable_map
            .keys()
            .any(|var| !source_vars.contains(var))
        || certificate
            .variable_map
            .values()
            .any(|var| !target_vars.contains(var))
        || certificate
            .variable_map
            .iter()
            .any(|(source, target)| source.equivalence != target.equivalence)
    {
        return Err(CqProofError::InvalidHomomorphism);
    }

    for (source_head, target_head) in source.head.iter().zip(&target.head) {
        if mapped_term(source_head, &certificate.variable_map).as_ref() != Some(target_head) {
            return Err(CqProofError::InvalidHomomorphism);
        }
    }

    for source_atom in &source.atoms {
        let mapped = Atom {
            relation: source_atom.relation,
            args: source_atom
                .args
                .iter()
                .map(|term| mapped_term(term, &certificate.variable_map))
                .collect::<Option<Vec<_>>>()
                .ok_or(CqProofError::InvalidHomomorphism)?,
        };
        if !target.atoms.contains(&mapped) {
            return Err(CqProofError::InvalidHomomorphism);
        }
    }
    Ok(())
}

fn partial_atoms_possible(
    source: &ConjunctiveQuery,
    target: &ConjunctiveQuery,
    mapping: &BTreeMap<Var, Var>,
) -> bool {
    source.atoms.iter().all(|source_atom| {
        target.atoms.iter().any(|target_atom| {
            source_atom.relation == target_atom.relation
                && source_atom.args.len() == target_atom.args.len()
                && source_atom.args.iter().zip(&target_atom.args).all(
                    |(source_term, target_term)| match (source_term, target_term) {
                        (Term::Const { .. }, _) => source_term == target_term,
                        (Term::Var(source_var), Term::Var(target_var)) => {
                            source_var.equivalence == target_var.equivalence
                                && mapping
                                    .get(source_var)
                                    .is_none_or(|mapped| mapped == target_var)
                        }
                        (Term::Var(_), Term::Const { .. }) => false,
                    },
                )
        })
    })
}

fn search_homomorphism(
    index: usize,
    source_vars: &[Var],
    target_vars: &[Var],
    source: &ConjunctiveQuery,
    target: &ConjunctiveQuery,
    mapping: &mut BTreeMap<Var, Var>,
) -> bool {
    if index == source_vars.len() {
        return verify_homomorphism(
            source,
            target,
            &HomomorphismCertificate {
                variable_map: mapping.clone(),
            },
        )
        .is_ok();
    }

    let source_var = source_vars[index];
    if mapping.contains_key(&source_var) {
        return search_homomorphism(index + 1, source_vars, target_vars, source, target, mapping);
    }

    for &target_var in target_vars
        .iter()
        .filter(|target| target.equivalence == source_var.equivalence)
    {
        mapping.insert(source_var, target_var);
        if partial_atoms_possible(source, target, mapping)
            && search_homomorphism(index + 1, source_vars, target_vars, source, target, mapping)
        {
            return true;
        }
        mapping.remove(&source_var);
    }
    false
}

fn find_homomorphism(
    source: &ConjunctiveQuery,
    target: &ConjunctiveQuery,
) -> Option<HomomorphismCertificate> {
    if !source.is_safe() || !target.is_safe() || source.head.len() != target.head.len() {
        return None;
    }

    let target_vars = target.variables().into_iter().collect::<Vec<_>>();
    let source_vars = source.variables().into_iter().collect::<Vec<_>>();
    let mut mapping = BTreeMap::<Var, Var>::new();

    for (source_head, target_head) in source.head.iter().zip(&target.head) {
        match (source_head, target_head) {
            (Term::Const { .. }, _) if source_head != target_head => return None,
            (Term::Const { .. }, Term::Const { .. }) => {}
            (Term::Var(source_var), Term::Var(target_var)) => {
                if source_var.equivalence != target_var.equivalence {
                    return None;
                }
                match mapping.insert(*source_var, *target_var) {
                    Some(existing) if existing != *target_var => return None,
                    _ => {}
                }
            }
            (Term::Var(_), Term::Const { .. }) | (Term::Const { .. }, Term::Var(_)) => {
                return None;
            }
        }
    }

    if !partial_atoms_possible(source, target, &mapping)
        || !search_homomorphism(0, &source_vars, &target_vars, source, target, &mut mapping)
    {
        return None;
    }

    Some(HomomorphismCertificate {
        variable_map: mapping,
    })
}

fn discover_equivalence_certificate(
    left: &ConjunctiveQuery,
    right: &ConjunctiveQuery,
) -> Option<CqEquivalenceCertificate> {
    Some(CqEquivalenceCertificate {
        left_to_right: find_homomorphism(left, right)?,
        right_to_left: find_homomorphism(right, left)?,
    })
}

fn certify_equivalent(
    left: ConjunctiveQuery,
    right: ConjunctiveQuery,
) -> Result<CheckedCertificate<CqEquivalenceChecker>, CqProofError> {
    let certificate =
        discover_equivalence_certificate(&left, &right).ok_or(CqProofError::InvalidHomomorphism)?;
    verify_certificate::<CqEquivalenceChecker>(&CqEquivalenceSpec { left, right }, certificate)
}

fn minimize_redundant_atoms(query: &ConjunctiveQuery) -> ConjunctiveQuery {
    let mut current = query.clone();
    'restart: loop {
        for index in 0..current.atoms.len() {
            let mut candidate = current.clone();
            candidate.atoms.remove(index);
            if candidate.is_safe() && certify_equivalent(current.clone(), candidate.clone()).is_ok()
            {
                current = candidate;
                continue 'restart;
            }
        }
        return current;
    }
}

fn encode_length(out: &mut Vec<u8>, length: usize) -> Option<()> {
    out.extend_from_slice(&u64::try_from(length).ok()?.to_be_bytes());
    Some(())
}

fn encode_term(term: &Term, labels: &BTreeMap<Var, u16>, out: &mut Vec<u8>) -> Option<()> {
    match term {
        Term::Var(var) => {
            out.push(0);
            out.extend_from_slice(&var.equivalence.raw().to_be_bytes());
            out.extend_from_slice(&labels.get(var)?.to_be_bytes());
        }
        Term::Const { equivalence, key } => {
            out.push(1);
            out.extend_from_slice(&equivalence.raw().to_be_bytes());
            let encoded = kernel_semantics::encode_canonical_eq_key(key);
            encode_length(out, encoded.len())?;
            out.extend_from_slice(&encoded);
        }
    }
    Some(())
}

fn encode_canonical_query(
    query: &ConjunctiveQuery,
    labels: &BTreeMap<Var, u16>,
) -> Option<Vec<u8>> {
    let mut atoms = Vec::with_capacity(query.atoms.len());
    for atom in &query.atoms {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&atom.relation.raw().to_be_bytes());
        encode_length(&mut bytes, atom.args.len())?;
        for arg in &atom.args {
            encode_term(arg, labels, &mut bytes)?;
        }
        atoms.push(bytes);
    }
    atoms.sort();

    let mut out = Vec::new();
    encode_length(&mut out, query.head.len())?;
    for head in &query.head {
        encode_term(head, labels, &mut out)?;
    }
    encode_length(&mut out, atoms.len())?;
    for atom in atoms {
        encode_length(&mut out, atom.len())?;
        out.extend_from_slice(&atom);
    }
    Some(out)
}

fn permute_group_labels(
    index: usize,
    vars: &[Var],
    labels: &mut [u16],
    assigned: &mut BTreeMap<Var, u16>,
    query: &ConjunctiveQuery,
    best: &mut Option<Vec<u8>>,
    remaining_groups: &[(Vec<Var>, Vec<u16>)],
) {
    if index == labels.len() {
        for (&var, &label) in vars.iter().zip(labels.iter()) {
            assigned.insert(var, label);
        }
        if let Some((next_vars, next_labels)) = remaining_groups.first() {
            let mut next_labels = next_labels.clone();
            permute_group_labels(
                0,
                next_vars,
                &mut next_labels,
                assigned,
                query,
                best,
                &remaining_groups[1..],
            );
        } else if let Some(encoded) = encode_canonical_query(query, assigned)
            && best.as_ref().is_none_or(|current| encoded < *current)
        {
            *best = Some(encoded);
        }
        for var in vars {
            assigned.remove(var);
        }
        return;
    }

    for swap in index..labels.len() {
        labels.swap(index, swap);
        permute_group_labels(
            index + 1,
            vars,
            labels,
            assigned,
            query,
            best,
            remaining_groups,
        );
        labels.swap(index, swap);
    }
}

fn canonical_core_identity(query: &ConjunctiveQuery) -> Option<Vec<u8>> {
    let query = minimize_redundant_atoms(query);
    if !query.is_safe() {
        return None;
    }

    let mut fixed = BTreeMap::<Var, u16>::new();
    let mut next = 0_u16;
    for term in &query.head {
        if let Term::Var(var) = term
            && let std::collections::btree_map::Entry::Vacant(entry) = fixed.entry(*var)
        {
            entry.insert(next);
            next = next.checked_add(1)?;
        }
    }

    let mut groups = BTreeMap::<SemanticId, Vec<Var>>::new();
    for var in query.variables() {
        if !fixed.contains_key(&var) {
            groups.entry(var.equivalence).or_default().push(var);
        }
    }
    if groups.values().map(Vec::len).sum::<usize>() > 9 {
        return None;
    }

    let mut group_specs = Vec::<(Vec<Var>, Vec<u16>)>::new();
    for (_, vars) in groups {
        let end = next.checked_add(u16::try_from(vars.len()).ok()?)?;
        let labels = (next..end).collect::<Vec<_>>();
        next = end;
        group_specs.push((vars, labels));
    }

    if group_specs.is_empty() {
        return encode_canonical_query(&query, &fixed);
    }

    let (first_vars, first_labels) = group_specs.remove(0);
    let mut labels = first_labels;
    let mut assigned = fixed;
    let mut best = None;
    permute_group_labels(
        0,
        &first_vars,
        &mut labels,
        &mut assigned,
        &query,
        &mut best,
        &group_specs,
    );
    best
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoweringError {
    TypecheckRejected,
    UnknownRelation,
    BagRelation,
    ColumnOutOfBounds,
    EqualityDoesNotMatchColumnSemantics,
    ContradictoryConstant,
    UnsupportedOperator,
    VariableSpaceExhausted,
    SemanticKeyRejected,
}

#[derive(Debug, Clone)]
struct LoweredRelation {
    atoms: Vec<Atom>,
    columns: Vec<Term>,
    column_equivalences: Vec<SemanticId>,
}

fn replace_term(relation: &mut LoweredRelation, from: &Term, to: &Term) {
    if from == to {
        return;
    }
    for atom in &mut relation.atoms {
        for arg in &mut atom.args {
            if arg == from {
                *arg = to.clone();
            }
        }
    }
    for column in &mut relation.columns {
        if column == from {
            *column = to.clone();
        }
    }
}

fn unify_terms(
    relation: &mut LoweredRelation,
    left: Term,
    right: Term,
) -> Result<(), LoweringError> {
    if left.equivalence() != right.equivalence() {
        return Err(LoweringError::EqualityDoesNotMatchColumnSemantics);
    }
    match (&left, &right) {
        (Term::Const { key: left, .. }, Term::Const { key: right, .. }) => {
            if left == right {
                Ok(())
            } else {
                Err(LoweringError::ContradictoryConstant)
            }
        }
        (Term::Var(left_var), Term::Var(right_var)) => {
            let (keep, replace) = if left_var <= right_var {
                (left, right)
            } else {
                (right, left)
            };
            replace_term(relation, &replace, &keep);
            Ok(())
        }
        (Term::Var(_), Term::Const { .. }) => {
            replace_term(relation, &left, &right);
            Ok(())
        }
        (Term::Const { .. }, Term::Var(_)) => {
            replace_term(relation, &right, &left);
            Ok(())
        }
    }
}

fn lower_scan(
    relation: SemanticId,
    context: &SemanticContext,
    next_var: &mut u16,
) -> Result<LoweredRelation, LoweringError> {
    let definition = context
        .schema
        .relation(relation)
        .ok_or(LoweringError::UnknownRelation)?;
    let RelationSemantics::Set {
        column_equivalences,
    } = &definition.semantics
    else {
        return Err(LoweringError::BagRelation);
    };
    if column_equivalences.len() != definition.columns.len() {
        return Err(LoweringError::TypecheckRejected);
    }

    let mut columns = Vec::with_capacity(definition.columns.len());
    for &equivalence in column_equivalences {
        let var = Var {
            id: *next_var,
            equivalence,
        };
        *next_var = next_var
            .checked_add(1)
            .ok_or(LoweringError::VariableSpaceExhausted)?;
        columns.push(Term::Var(var));
    }
    Ok(LoweredRelation {
        atoms: vec![Atom {
            relation,
            args: columns.clone(),
        }],
        columns,
        column_equivalences: column_equivalences.clone(),
    })
}

fn exact_column(
    relation: &LoweredRelation,
    column: usize,
    equality: SemanticId,
) -> Result<Term, LoweringError> {
    let term = relation
        .columns
        .get(column)
        .cloned()
        .ok_or(LoweringError::ColumnOutOfBounds)?;
    let column_equality = relation
        .column_equivalences
        .get(column)
        .copied()
        .ok_or(LoweringError::ColumnOutOfBounds)?;
    if equality != column_equality || term.equivalence() != equality {
        return Err(LoweringError::EqualityDoesNotMatchColumnSemantics);
    }
    Ok(term)
}

fn lower_rel_expr_fragment(
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    next_var: &mut u16,
) -> Result<LoweredRelation, LoweringError> {
    match expr {
        RelExpr::Scan(relation) => lower_scan(*relation, context, next_var),
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            let mut lowered = lower_rel_expr_fragment(input, context, registry, next_var)?;
            let column_term = exact_column(&lowered, *column, *equivalence)?;
            let key = registry
                .compile_equivalence(context, *equivalence)
                .and_then(|compiled| compiled.canonical_key(value))
                .map_err(|_| LoweringError::SemanticKeyRejected)?;
            let constant = Term::Const {
                equivalence: *equivalence,
                key,
            };
            unify_terms(&mut lowered, column_term, constant)?;
            Ok(lowered)
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let mut lowered = lower_rel_expr_fragment(input, context, registry, next_var)?;
            let left = exact_column(&lowered, *left_column, *equivalence)?;
            let right = exact_column(&lowered, *right_column, *equivalence)?;
            unify_terms(&mut lowered, left, right)?;
            Ok(lowered)
        }
        RelExpr::Project { input, columns } => {
            let lowered = lower_rel_expr_fragment(input, context, registry, next_var)?;
            let projected = columns
                .iter()
                .map(|column| {
                    lowered
                        .columns
                        .get(*column)
                        .cloned()
                        .ok_or(LoweringError::ColumnOutOfBounds)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let projected_equivalences = columns
                .iter()
                .map(|column| {
                    lowered
                        .column_equivalences
                        .get(*column)
                        .copied()
                        .ok_or(LoweringError::ColumnOutOfBounds)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(LoweredRelation {
                atoms: lowered.atoms,
                columns: projected,
                column_equivalences: projected_equivalences,
            })
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            let left = lower_rel_expr_fragment(left, context, registry, next_var)?;
            let right = lower_rel_expr_fragment(right, context, registry, next_var)?;
            let left_join = exact_column(&left, *left_column, *equivalence)?;
            let right_join = exact_column(&right, *right_column, *equivalence)?;
            let mut combined = LoweredRelation {
                atoms: left.atoms.into_iter().chain(right.atoms).collect(),
                columns: left.columns.into_iter().chain(right.columns).collect(),
                column_equivalences: left
                    .column_equivalences
                    .into_iter()
                    .chain(right.column_equivalences)
                    .collect(),
            };
            unify_terms(&mut combined, left_join, right_join)?;
            Ok(combined)
        }
        RelExpr::FilterOrderConst { .. }
        | RelExpr::Difference { .. }
        | RelExpr::Union { .. }
        | RelExpr::AntiJoin { .. }
        | RelExpr::Distinct { .. }
        | RelExpr::Group { .. }
        | RelExpr::TopKWithTies { .. }
        | RelExpr::PromoteToBag(_) => Err(LoweringError::UnsupportedOperator),
    }
}

fn lower_rel_expr_to_cq(
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Result<ConjunctiveQuery, LoweringError> {
    expr.typecheck(context, registry)
        .map_err(|_| LoweringError::TypecheckRejected)?;
    let mut next_var = 0_u16;
    let lowered = lower_rel_expr_fragment(expr, context, registry, &mut next_var)?;
    let query = ConjunctiveQuery {
        head: lowered.columns,
        atoms: lowered.atoms,
    };
    query
        .is_safe()
        .then_some(query)
        .ok_or(LoweringError::UnsupportedOperator)
}

fn is_q_hierarchical(query: &ConjunctiveQuery) -> bool {
    let variables = query.variables().into_iter().collect::<Vec<_>>();
    let free = query
        .head
        .iter()
        .filter_map(|term| match term {
            Term::Var(var) => Some(*var),
            Term::Const { .. } => None,
        })
        .collect::<BTreeSet<_>>();
    let atom_sets = variables
        .iter()
        .map(|var| {
            query
                .atoms
                .iter()
                .enumerate()
                .filter_map(|(index, atom)| {
                    atom.args
                        .iter()
                        .any(|term| matches!(term, Term::Var(found) if found == var))
                        .then_some(index)
                })
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();

    for left_index in 0..variables.len() {
        for right_index in (left_index + 1)..variables.len() {
            let left = &atom_sets[left_index];
            let right = &atom_sets[right_index];
            let nested_or_disjoint =
                left.is_subset(right) || right.is_subset(left) || left.is_disjoint(right);
            if !nested_or_disjoint {
                return false;
            }
            if left.is_subset(right)
                && left != right
                && free.contains(&variables[left_index])
                && !free.contains(&variables[right_index])
            {
                return false;
            }
            if right.is_subset(left)
                && left != right
                && free.contains(&variables[right_index])
                && !free.contains(&variables[left_index])
            {
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CqSemanticIdentity {
    canonical_core: Box<[u8]>,
    q_hierarchical: bool,
}

impl CqSemanticIdentity {
    pub(crate) const fn is_q_hierarchical(&self) -> bool {
        self.q_hierarchical
    }
}

pub(crate) fn semantic_cq_identity(
    expr: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Option<CqSemanticIdentity> {
    let query = lower_rel_expr_to_cq(expr, context, registry).ok()?;
    let core = minimize_redundant_atoms(&query);
    let canonical_core = canonical_core_identity(&core)?.into_boxed_slice();
    Some(CqSemanticIdentity {
        canonical_core,
        q_hierarchical: is_q_hierarchical(&core),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_model::{FiniteModel, Value};
    use kernel_schema::{RelationDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_semantics::EquivalenceModule;
    use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId};

    use crate::{RelObservationForest, RelationDelta};

    fn set_unary_fixture(
        relation_count: usize,
    ) -> (
        SemanticContext,
        SemanticRegistry,
        SemanticId,
        Vec<SemanticId>,
        FiniteModel,
    ) {
        let eq = SemanticId::new(190_000);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(190_000));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(190_000));
        let relations = (0..relation_count)
            .map(|index| SemanticId::new(191_000 + u128::try_from(index).unwrap()))
            .collect::<Vec<_>>();
        for &relation in &relations {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        for &relation in &relations {
            model.relations.insert(
                relation,
                (0_i64..8).map(|value| vec![Value::I64(value)]).collect(),
            );
        }
        (context, registry, eq, relations, model)
    }

    fn intersection(order: &[SemanticId], equality: SemanticId) -> RelExpr {
        let mut query = RelExpr::Scan(order[0]);
        for &relation in &order[1..] {
            query = RelExpr::JoinEq {
                left: Box::new(query),
                right: Box::new(RelExpr::Scan(relation)),
                left_column: 0,
                right_column: 0,
                equivalence: equality,
            };
        }
        RelExpr::Project {
            input: Box::new(query),
            columns: vec![0],
        }
    }

    #[test]
    fn semantic_identity_collapses_join_reorder() {
        let (context, registry, eq, relations, _) = set_unary_fixture(4);
        let left = intersection(&relations, eq);
        let mut reversed = relations.clone();
        reversed.reverse();
        let right = intersection(&reversed, eq);
        assert_ne!(left, right);
        assert_eq!(
            semantic_cq_identity(&left, &context, &registry),
            semantic_cq_identity(&right, &context, &registry)
        );
    }
    #[test]
    fn q_hierarchical_classifier_separates_star_from_triangle() {
        let eq = SemanticId::new(195_000);
        let v0 = Var {
            id: 0,
            equivalence: eq,
        };
        let v1 = Var {
            id: 1,
            equivalence: eq,
        };
        let v2 = Var {
            id: 2,
            equivalence: eq,
        };
        let rel_a = SemanticId::new(195_001);
        let rel_b = SemanticId::new(195_002);
        let rel_c = SemanticId::new(195_003);
        let star = ConjunctiveQuery {
            head: vec![Term::Var(v0), Term::Var(v1), Term::Var(v2)],
            atoms: vec![
                Atom {
                    relation: rel_a,
                    args: vec![Term::Var(v0), Term::Var(v1)],
                },
                Atom {
                    relation: rel_b,
                    args: vec![Term::Var(v0), Term::Var(v2)],
                },
            ],
        };
        let triangle = ConjunctiveQuery {
            head: vec![Term::Var(v0), Term::Var(v1), Term::Var(v2)],
            atoms: vec![
                Atom {
                    relation: rel_a,
                    args: vec![Term::Var(v0), Term::Var(v1)],
                },
                Atom {
                    relation: rel_b,
                    args: vec![Term::Var(v1), Term::Var(v2)],
                },
                Atom {
                    relation: rel_c,
                    args: vec![Term::Var(v2), Term::Var(v0)],
                },
            ],
        };
        assert!(is_q_hierarchical(&star));
        assert!(!is_q_hierarchical(&triangle));
    }

    fn permutations<T: Copy>(values: &[T]) -> Vec<Vec<T>> {
        fn recurse<T: Copy>(cursor: usize, values: &mut [T], out: &mut Vec<Vec<T>>) {
            if cursor == values.len() {
                out.push(values.to_vec());
                return;
            }
            for index in cursor..values.len() {
                values.swap(cursor, index);
                recurse(cursor + 1, values, out);
                values.swap(cursor, index);
            }
        }
        let mut values = values.to_vec();
        let mut out = Vec::new();
        recurse(0, &mut values, &mut out);
        out
    }

    #[test]
    fn production_forest_semantic_interning_preserves_all_roots_and_deltas() {
        let (context, registry, eq, relations, _) = set_unary_fixture(3);
        let roots = permutations(&relations)
            .iter()
            .map(|order| intersection(order, eq))
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 6);

        for left_mask in 0_u8..4 {
            for middle_mask in 0_u8..4 {
                for right_mask in 0_u8..4 {
                    let masks = [left_mask, middle_mask, right_mask];
                    let mut model = FiniteModel::default();
                    for (&relation, mask) in relations.iter().zip(masks) {
                        let rows = (0_i64..2)
                            .filter(|value| mask & (1_u8 << u32::try_from(*value).unwrap()) != 0)
                            .map(|value| vec![Value::I64(value)])
                            .collect::<Vec<_>>();
                        model.relations.insert(relation, rows);
                    }
                    let (mut forest, stats) =
                        RelObservationForest::build_with_stats(&roots, &model, &context, &registry)
                            .unwrap();
                    assert!(stats.semantic_reused_subtrees >= 5);
                    let expected = forest.root_output_value(0, &context, &registry).unwrap();
                    for route in 1..roots.len() {
                        assert_eq!(
                            forest
                                .root_output_value(route, &context, &registry)
                                .unwrap(),
                            expected
                        );
                    }
                    forest.bind_revision(RevisionId::new(1)).unwrap();
                    for (relation_index, &relation) in relations.iter().enumerate() {
                        for value in 0_i64..2 {
                            let present = masks[relation_index]
                                & (1_u8 << u32::try_from(value).unwrap())
                                != 0;
                            let row = vec![Value::I64(value)];
                            let delta = RelationDelta {
                                inserted: (!present).then_some(row.clone()).into_iter().collect(),
                                removed: present.then_some(row).into_iter().collect(),
                                result_type: RelExpr::Scan(relation)
                                    .typecheck(&context, &registry)
                                    .unwrap(),
                            };
                            let deltas = BTreeMap::from([(relation, delta)]);
                            let (_, effects, _) = forest
                                .candidate_from_relation_deltas_for_revision_with_stats(
                                    RevisionId::new(
                                        10 + u64::try_from(relation_index * 2).unwrap()
                                            + u64::try_from(value).unwrap(),
                                    ),
                                    &deltas,
                                    &context,
                                    &registry,
                                )
                                .unwrap();
                            assert_eq!(effects.len(), roots.len());
                            assert!(effects.iter().all(|effect| effect == &effects[0]));
                        }
                    }
                }
            }
        }
    }
}
