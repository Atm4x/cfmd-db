# CFMD Pass280 — final model/schema ownership and global kernel-refactor closure

Date: 2026-09-28
Start: 00:20:19 UTC
Useful boundary: 00:40:19 UTC
Hard boundary: 00:44:19 UTC

## Goal

Treat P280 as the final ownership/closure candidate:

1. split the already-audited `kernel-model` into real semantic owners without changing semantics;
2. give `kernel-schema` a whole-crate hostile closure pass and split it by semantic responsibility;
3. revalidate the historically closed heavy kernels against the current workspace rather than assuming their old status;
4. close the global kernel refactor only if no new evidence-backed cluster remains.

## P280.MODEL-OWNERS — semantic physical split — CLOSED

The P279 dedicated hostile audit over COW relation storage, recursive values, live-reference sensitivity, lifecycle restriction and normalization found no new correctness/fallback/asymptotic seam. P280 therefore changed ownership only.

`kernel-model` is now divided into:

- `storage.rs` — `CowValue`, `CowMap`, persistent relation-row append patches and `RelationStore`;
- `value.rs` — recursive `Value` algebra and live-reference traversal;
- `live_refs.rs` — `LiveRefSensitivityIndex` and consumer coordinates;
- `state.rs` — `FiniteModel`, lifecycle restriction, `DatabaseState` normalization and normalized-state authority;
- root `lib.rs` — public re-exports and white-box regressions.

The sharing internals required by existing white-box tests are crate-internal only (`pub(super)`); no new public capability was introduced.

Gate: `kernel-model` **10/10**.

Status: `kernel-model` **COMPLETE / FROZEN** for the current global-refactor objective.

## P280.SCHEMA-BIDIRECTIONAL-CLOSURE — no full-map scan on inclusion — CLOSED

P278 made subtype reads consume a maintained `SubtypeClosure`, but its mutation path still found every descendant of a newly extended subtype by scanning the complete `ancestors` map.

P280 replaces that residual graph reconstruction with one bidirectional transitive authority:

- `ancestors[subtype]` stores already-proven strict supertypes;
- `descendants[supertype]` stores already-proven strict subtypes;
- adding `S ⊑ T` obtains `Desc(S) ∪ {S}` and `Anc(T) ∪ {T}` directly from the maintained closure;
- every lower/upper implication is inserted into both maps.

Thus `Schema::is_subtype`, `SubtypeClosure::ancestors`, validation extent compilation and future schema consumers share one relation. There is no threshold routing, fresh BFS, whole-map descendant scan, or fallback implementation.

A hostile late-bridge regression constructs `a ⊑ b` and `c ⊑ d`, adds `b ⊑ c` afterwards, and proves `a ⊑ d` plus the complete ancestor set `{a,b,c,d}` immediately.

Gate: `kernel-schema` **11/11**; downstream `kernel-validation` **16 passed / 0 failed / 1 ignored**.

## P280.SCHEMA-OWNERS — semantic physical split — CLOSED

The schema root is now split into:

- `types.rs` — type expressions and guarded recursion validation;
- `definitions.rs` — capability, field, relation, structural equivalence and ordering definitions;
- `subtype.rs` — maintained transitive subtype authority;
- `schema.rs` — schema mutation, transport/definitional equivalence and semantic dependency laws;
- `context.rs` — semantic environment/context authority;
- root `lib.rs` — public façade and regressions.

No compatibility wrappers or duplicate algorithms were introduced.

Status: `kernel-schema` **COMPLETE / FROZEN**.

## P280.GLOBAL-HOSTILE-REVALIDATION — historical closures checked, not assumed

P280 explicitly revisited the historical high-risk kernels after the later cross-kernel work.

### Heavy kernels

- `kernel-plan`: the Pass194 crate-wide closeout was re-read and its active generic/fallback markers were re-inspected in current source. They remain Γ-aware linear/canonical correctness implementations behind typed/persisted specializations, not SQL nested-loop substitutes, error-driven routing, or an unbounded row-count payer. P280 found no new counterexample. Current tests: **289 passed / 0 failed / 5 ignored**.
- `kernel-query`: current hostile marker/legacy inventory and full tests re-run; compatibility delta adapters remain explicit ABI/test representations while the maintained/exact execution authority is current. **126 passed / 0 failed / 1 ignored**.
- `kernel-semantics`: current full lib tests re-run after schema/model ownership changes; no new error-erasure/fallback/authority seam established. **65/65**.
- `kernel-durability`: full crash/recovery/store suite re-run after the P278 static capability law and P280 schema split. **153/153**.
- `kernel-change`: current suite **57/57**; no new evidence to reopen the P267 closure.
- `kernel-integration`: current suite **21/21**; prepared write authority from P273 remains intact.

The important distinction is that P280 does not claim “the source contains no word fallback”. It revalidates whether each remaining generic implementation is an accidental alternate authority. In `kernel-plan`, the known generic row implementation is an intentional member of one physical execution principle and was already the subject of P194's whole-crate asymptotic audit. Removing specialization routing without a new measured defect would be cleanup-by-inertia, not a mathematical improvement.

### Remaining kernels

The later campaign passes already supplied dedicated or grouped closure evidence for:

- `kernel-lens`, `kernel-transport`, `kernel-revision` — P274/P275;
- `kernel-grounded-closure`, `kernel-persistent`, `storage-memory` — P275/P279;
- `kernel-fixpoint`, `kernel-semantic-index` — P276;
- `kernel-auth`, `kernel-deployment` — P277/P278;
- `kernel-validation` — P275/P277/P278/P280;
- compact `kernel-aggregate`, `kernel-exact`, `kernel-identity`, `kernel-lifecycle`, `kernel-proof`, `kernel-retention`, `kernel-types`, `kernel-violation` — grouped P277 audit plus current workspace-wide regression.

No remaining kernel has an evidence-backed cleanup/refactor item in the current ledger.

A final owner-size/cohesion scan also rechecked the largest surviving production files rather than using a mechanical LOC cutoff. The largest are concentrated in already-closed mathematical/state-machine owners (`kernel-semantics::equivalence`, `kernel-semantics::ordering`, `kernel-query::maintained_plan`). Inspection showed coherent single calculi/authorities rather than the mixed unrelated responsibilities that motivated the earlier plan/query/durability/change god-owner splits. They therefore remain intact; reopening them solely to push every file below an arbitrary line threshold would contradict the evidence-driven rule.

The append-only core spec intentionally still contains historical `ACTIVE/OPEN` statements inside P269–P272 sections. Later P273/P279/P280 status entries supersede those historical snapshots; they are not current ledger entries and were not rewritten.

The final read-only production marker scan found no `TODO`, `FIXME`, `unimplemented!`, `todo!`, open hostile bug/payer marker, or active quadratic payer in the kernel set. The only `PAYER` hit is the marker legend in `kernel-plan`. Production `expect`/`unreachable!` sites were sampled as sealed/internal invariant assertions rather than error-erasing alternate execution routes; test-only `expect` sites in roots remain test code.

## Global status after P280

**GLOBAL KERNEL HOSTILE/REFACTOR CAMPAIGN: COMPLETE / FROZEN.**

This status means:

- do not manufacture more passes solely because a file could be split further or a generic implementation exists;
- reopen a frozen kernel only for a concrete correctness counterexample, proof/authority defect, measured performance/asymptotic regression, new R&D objective, or API/DX requirement;
- historical/compatibility decoders may remain where they are required to read durable old formats; obsolete/test-only reference implementations remain information/oracle sources and are not production alternatives.

This is not a proof that no undiscovered bug can exist. It is the end of the current evidence-driven global refactor objective.

## Verification before final freeze

- `kernel-model`: **10/10**;
- `kernel-schema`: **11/11**;
- `kernel-validation`: **16 passed / 0 failed / 1 ignored**;
- `kernel-plan`: **289 passed / 0 failed / 5 ignored**;
- `kernel-query`: **126 passed / 0 failed / 1 ignored**;
- `kernel-semantics`: **65/65**;
- `kernel-durability`: **153/153**;
- `kernel-change`: **57/57**;
- `kernel-integration`: **21/21**;
- complete `cargo test --workspace --lib --offline`: **PASS**;
- `cargo check --workspace --all-targets --offline`: **PASS**;
- strict workspace Clippy (`-D warnings`): **PASS**.

The first surface-refinement run correctly failed after the schema split because `check_surface_refinement.py` still read only `kernel-schema/src/lib.rs`. P280 updated that verifier to consume the full production schema source tree, matching its existing query/plan/change behavior; the repeated checker then passed. This keeps the mechanized source-binding gate fail-closed under semantic module splits instead of weakening or deleting it.

Post-fix gates before final freeze:

- `cargo fmt --all -- --check`: **PASS**;
- strict rustdoc for `kernel-model` + `kernel-schema`: **PASS**;
- `formal/lean/check_refinement.py`: **PASS, 10 fault points**;
- `formal/lean/check_surface_refinement.py`: **PASS**;
- `scripts/verify-repository.sh`: **PASS**.

Final functional freeze: **00:34:57 UTC**. Manifest/package metadata is recorded below.

## Principal production files changed

- `crates/kernel-model/src/{lib,storage,value,live_refs,state}.rs`;
- `crates/kernel-schema/src/{lib,types,definitions,subtype,schema,context}.rs`;
- `docs/spec/CFMD_CORE_SPEC.md`;
- `formal/lean/check_surface_refinement.py` (module-split-aware source binding).

## Next

There is no automatic P281 kernel-cleanup pass.

The next work should be selected from a new product/R&D objective (facade/DX, new mathematical capability, packaging/embedding, a benchmark regression, or a concrete hostile counterexample). If future evidence reopens a kernel, continue the append-only ledger from P280 rather than restarting a broad cleanup wave.


## Freeze / packaging

- final functional freeze: **00:34:57 UTC**;
- useful boundary: **00:40:19 UTC**;
- hard boundary: **00:44:19 UTC**;
- final repository manifest: **2935 entries / 2935 verified**;
- repository verifier after cleanup: **PASS**;
- `target/`: absent before packaging;
- ZIP-store packaging complete: **00:36:26 UTC**;
- ZIP entries: **2936** (2935 manifested files + `REPOSITORY_MANIFEST.sha256`);
- ZIP integrity: **PASS** (`testzip() -> None`);
- manifest verified again from bytes inside ZIP: **2935/2935**;
- ZIP SHA-256: `e71875ab5c6e4f7ee2c2f2aa3fbd11c7fba777a8432fbc0d48ff12901d3b4605`;
- ZIP size: **43,394,001 bytes**.
