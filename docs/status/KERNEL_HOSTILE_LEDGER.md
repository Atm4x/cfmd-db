# Kernel Hostile / Freeze Ledger

## Policy

This is the current repository-facing inventory after Pass280. It complements historical pass reports; it is not a promise that undiscovered bugs are impossible.

`FROZEN` means reopen only on evidence: correctness counterexample, proof/authority seam, measured complexity/performance regression, new R&D requirement, or public API/DX requirement.

| crate | closeout evidence | current status |
|---|---|---|
| `kernel-aggregate` | grouped small-kernel hostile sweep + final P280 global revalidation | FROZEN |
| `kernel-auth` | dedicated P277 security/resource hostile pass; P278 ownership split | FROZEN |
| `kernel-change` | P256–P267 closure; final hostile inventory; P280 revalidation | COMPLETE / FROZEN |
| `kernel-deployment` | dedicated P277 bounded/auth-first audit; P278 ownership split | FROZEN |
| `kernel-durability` | dedicated durability/store closeout line; P280 heavy-kernel revalidation | FROZEN |
| `kernel-exact` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-fixpoint` | P276 incidence/checker/ownership closure | COMPLETE / FROZEN |
| `kernel-grounded-closure` | P275 unified witness calculus; P279 structural closeout | COMPLETE / FROZEN |
| `kernel-identity` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-integration` | P268–P273 prepared writable/integration closure; P280 revalidation | COMPLETE / FROZEN |
| `kernel-lens` | P274 hostile correctness pass; P275 ownership split | FROZEN |
| `kernel-lifecycle` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-model` | P279 dedicated hostile audit; P280 ownership split | COMPLETE / FROZEN |
| `kernel-persistent` | P276/P279 copy-amplification and whole-crate sweep | COMPLETE / FROZEN |
| `kernel-plan` | historical crate-wide hostile closeout (P194 line) + P280 heavy-kernel revalidation | FROZEN |
| `kernel-proof` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-query` | dedicated query/TopK/ExecGraph hostile line + P280 heavy-kernel revalidation | FROZEN |
| `kernel-retention` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-revision` | P274 dedicated inspection/regression pass | FROZEN |
| `kernel-schema` | P278 subtype authority; P280 bidirectional closure + ownership split | COMPLETE / FROZEN |
| `kernel-semantic-index` | P276 dedicated audit, clean/no-change | FROZEN |
| `kernel-semantics` | earlier dedicated semantics work + P280 heavy-kernel revalidation | FROZEN |
| `kernel-transport` | P274 hostile correctness/performance pass; P275 ownership split | FROZEN |
| `kernel-types` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-validation` | P275/P277 correctness fixes; P278 ownership split | FROZEN |
| `kernel-violation` | grouped small-kernel hostile sweep | FROZEN |
| `storage-memory` | P274 LCA performance reopen; P279 revision-authority closeout | COMPLETE / FROZEN |

## Complexity inventory policy

The hostile campaign specifically searched for avoidable repeated full scans, nested traversals, repeated BFS/DFS, repeated typecheck/prepare/materialization, duplicated solver/authority state, copy amplification and error-driven generic fallbacks.

The closeout does **not** assert that every operation is sub-quadratic for every input. Some relations, closures, joins, products and result-producing operations have output-sensitive or mathematically inherent costs. Reopen when a measured/derived extra factor can be removed by an index, maintained authority, batching, structural sharing or stronger CFMD semantics.

## Marker policy

Production source is intentionally not spammed with `// CLEAR` markers, which become stale after edits. Use markers only for active, exceptional debt:

- `PAYER` — known hot-path/complexity debt requiring a ledger item;
- `RESIDUE` — structural/ownership debt requiring a ledger item;
- `BOUNDARY` — intentionally expensive semantic/I/O boundary when the cost is non-obvious;
- `INHERENT` — non-obvious asymptotic cost justified by result size/mathematical semantics;
- `SPECIALIZE` — specialized implementation of a shared semantic principle.

Any future `PAYER`/`RESIDUE` must be reflected in this ledger before a kernel can remain marked frozen.
