#!/usr/bin/env python3
"""Bind CFMD SurfaceKernel.lean to the production Rust vocabulary.

Fail closed if the production surface/kernel constructors or the executable
round-trip certificate boundary drift away from the mechanized model.
"""
from pathlib import Path
import re, sys
ROOT = Path(__file__).resolve().parents[2]

def read(rel):
    return (ROOT / rel).read_text(encoding='utf-8')

def enum_variants(text, enum_name):
    m = re.search(rf'pub enum {re.escape(enum_name)}\s*\{{', text)
    if not m:
        raise AssertionError(f'missing enum {enum_name}')
    i = m.end(); depth = 1; j = i
    while j < len(text) and depth:
        if text[j] == '{': depth += 1
        elif text[j] == '}': depth -= 1
        j += 1
    body = text[i:j-1]
    return set(re.findall(r'^    ([A-Z][A-Za-z0-9_]*)\b', body, re.M))

def require(text, needle, where):
    if needle not in text:
        raise AssertionError(f'{where}: missing {needle!r}')

schema = read('crates/kernel-schema/src/lib.rs')
query = read('crates/kernel-query/src/lib.rs')
plan = read('crates/kernel-plan/src/lib.rs')
change = read('crates/kernel-change/src/lib.rs')
violation = read('crates/kernel-violation/src/lib.rs')
retention = read('crates/kernel-retention/src/lib.rs')
formal = read('formal/lean/CFMD/SurfaceKernel.lean')
specdoc = read('docs/spec/CFMD_CORE_SPEC.md')

expected_types = {'Scalar','Product','Sum','Option','Set','Bag','Seq','Map','Var','Mu'}
actual_types = enum_variants(schema, 'TypeExpr')
assert actual_types == expected_types, (actual_types, expected_types)
for scalar in ['Unit','Bool','I64','F64','Text','LiveEntityRef','HistoricalEntityId']:
    assert scalar in enum_variants(schema, 'ScalarType'), scalar

expected_rel = {'Scan','FilterEqConst','FilterEqColumns','Project','JoinEq','Difference','AntiJoin','Distinct','Group','TopKWithTies','PromoteToBag'}
actual_rel = enum_variants(query, 'RelExpr')
actual_plan = enum_variants(plan, 'Plan')
assert actual_rel == expected_rel, (actual_rel, expected_rel)
assert actual_plan == expected_rel, (actual_plan, expected_rel)

# Normative surface table in the spec must not drift outside the mechanized vocabulary.
for row in [
    'entity/class             -> nominal Entity carrier + typed maps/relations',
    'record/value object      -> Product',
    'closed enum              -> Sum',
    'optional                 -> Option',
    'list/array               -> Seq',
    'set                      -> Set',
    'multiset                 -> Bag',
    'map/dictionary           -> Map',
    'recursive document       -> guarded μ',
    'reference                -> typed Ref/Id',
    'interface/capability     -> subcarrier/inclusion + required symbols',
    'inheritance              -> coherent inclusion',
    'computed property        -> Query/View',
    'business constraint      -> violation query required empty',
    'transaction method       -> typed Rewrite',
]: require(specdoc, row, 'docs/spec/CFMD_CORE_SPEC.md')

# Production checked-lowering boundary must still be exact AST round-trip + no hidden nodes.
for needle in [
    'pub struct LoweringChecker;',
    'LoweringCertificate::ExactLogicalRoundTrip',
    'spec.physical.to_logical_expr() != spec.logical',
    'spec.physical.shape().nodes != logical_node_count(&spec.logical)',
    'pub fn to_logical_expr(&self) -> RelExpr',
    'pub fn lower_with_catalog(expr: &RelExpr, catalog: &PhysicalCatalog) -> Self',
    'let prepared = logical.prepare(context, registry)?;',
    'let lowering = certify_baseline_lowering(logical)?;',
]: require(plan, needle, 'kernel-plan')

# Non-relational normative surface rows must have concrete kernel witnesses.
for needle in ['pub struct CapabilityDef', 'inclusions: BTreeSet<(SemanticId, SemanticId)>',
               'pub fn include(', 'LiveEntityRef', 'HistoricalEntityId']:
    require(schema, needle, 'kernel-schema')
require(change, 'pub struct PreparedRewrite<T, I = ()>', 'kernel-change')
require(violation, 'pub struct ViolationMeasure<W>', 'kernel-violation')
require(retention, 'pub struct RetentionLabel', 'kernel-retention')

# Guarded μ validation must remain explicit in the production type checker.
for needle in ['TypeError::FreeVariable', 'TypeError::UnguardedRecursion',
               'Self::Mu { binder, body }', 'validate_under_constructor']:
    require(schema, needle, 'kernel-schema')

# The proof artifact itself must expose all closure theorems.
for needle in [
    'theorem surface_feature_bijective',
    'theorem surface_type_weight_preserved',
    'theorem surface_type_wellformed_preserved',
    'theorem lower_roundtrip',
    'theorem lower_node_count',
    'theorem checked_plan_certificate_sound',
    'theorem surface_query_semantics_preserved',
    'theorem surface_to_kernel_preservation',
]: require(formal, needle, 'SurfaceKernel.lean')

print('surface-to-kernel source refinement: PASS')
print('TypeExpr variants:', ','.join(sorted(actual_types)))
print('RelExpr/Plan variants:', ','.join(sorted(actual_rel)))
