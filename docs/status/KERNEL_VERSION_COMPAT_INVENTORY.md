# Kernel Version / Compatibility Inventory

**Authority:** repository-facing hostile inventory.  
**Last sweep:** PASS589, 2026-10-07 (compatibility grammar rechecked; no production codec changed in PASS589).  
**Pre-release law:** CFMD has not shipped a compatibility release. `FORMAT_VERSION = 1` is the current release-candidate discriminator, not a frozen promise to read PASS snapshots. Superseded active encodings are removed, and the sole current pre-release encoding may be renumbered/reset when doing so removes pass-era debris.

## Status vocabulary

- **CURRENT PRE-RELEASE / FAIL-CLOSED** — one current encoding/protocol; older/newer values fail closed. No historical decoder promise.
- **DOMAIN / IDENTITY TAG** — cryptographic/hash semantic domain separation, not compatibility routing. Keep unless deliberately changing the identity law.
- **TEST-ONLY ORACLE** — reference adapter exists only under `cfg(test)` and cannot route production execution.
- **ACTIVE ARCHITECTURE DEBT** — current production capability whose owner/model is suspected to be superseded; remove only after an equivalent owner is proven.
- **CURRENT CAPABILITY** — active semantic/physical capability; misleading `legacy` terminology must be renamed, not mechanically deleted.
- **CLOSED / REMOVED** — obsolete production family no longer exists.

## 1. Current storage / persisted encoding discriminators

PASS568 resets historical PASS-era numeric residues because no released compatibility boundary exists.

| owner | current tag | status | PASS568 result |
|---|---:|---|---|
| `kernel-durability::FORMAT_VERSION` | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | Release candidate only; old development snapshots may fail closed. |
| checkpoint semantic revision/context codec | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | Was historical `7`; codecs 1–6 had already been deleted in PASS557, so retaining `7` served no compatibility law. Reset to canonical current `1`. |
| durable physical realization codec | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | Was historical `4`; v1–v3 readers were already absent. Reset to canonical current `1`. |
| `CanonicalEqKey` / semantic-index key encoding | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | Was historical `2`; one current grammar only. `kernel-semantic-index::KEY_ENCODING_REVISION` is kept equal to this discriminator. |
| WAL frame / prepared capsule / storage AE / single-file section families | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | One current grammar each; no decoder ladder. |
| replication authority segment/index/object | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | One current grammar each. |
| external freshness TCP / replication transport / deployment package ABI | `1` | CURRENT PRE-RELEASE / FAIL-CLOSED | Protocol/package discriminators independent from disk FORMAT_VERSION; current-only. |

### Release rule

At the actual compatibility release, current persisted/wire byte languages must be inventoried and frozen deliberately. Until then, **do not preserve a pass-era number merely because a previous PASS wrote it**. Preserve only semantic identity/domain versions that are themselves part of a deliberate cryptographic or mathematical identity law.

## 2. Domain / identity `v1` strings

The following are not decoder stacks and are intentionally not mass-renumbered:

- `kernel-auth` signature/freshness/key domain strings;
- `kernel-deployment` policy/profile identity domains;
- storage AE/HKDF domain-separation labels;
- replication evidence/locator/segment domain-separation labels.

Changing these changes cryptographic or semantic identity, not merely file admission. They require an explicit identity-law R&D decision.

## 3. Query/change/runtime compatibility sweep

### CLOSED / TEST-ONLY

- `kernel-query::{exact_delta_from_legacy, exact_delta_to_legacy_checked}` remain `cfg(test)` oracle conversions only. Production maintained execution uses exact Γ-measure deltas.
- Join/group test variables named `legacy` compare current exact execution against reference results only; no production route selects them.
- `kernel-query::RelationDeltaView` is a **current zero-copy public/input boundary**, not a compatibility route. PASS568 renamed the misleading comment.
- `kernel-change::SetChange::into_fine` and `SeqSplice::into_fine` are used only by core tests. PASS568 gates them with `cfg(test)` rather than carrying them as production compatibility convenience.
- `RevisionEffect::certify_compiled_residual_frontier` is test-only convenience; production callers retain the prepared coordination graph. PASS568 gates it with `cfg(test)`.
- `DurableRuntime::commit_schema_aware_field_intent` was a one-shot compatibility wrapper used only by kernel-plan regressions. PASS568 gates it with `cfg(test)`; production uses the prepared schema-aware publication path.
- `kernel-query::execgraph` “retired V3” is documentation only; no V3 scheduler route exists.
- PASS472 `LegacyIndex` / `MaterializedSemanticIndexState` remains removed end-to-end.

### CLOSED in PASS568 — arbitrary-target durable relation commit

The production chain

`DurableRuntimeSupervisor::commit_revision -> DurableRuntime::commit_revision -> RuntimeRevisionCell::commit_revision_durable_full_exact`

was an actual compatibility path. It accepted an independently constructed target `Revision` and therefore persisted a complete target witness for retry identity.

PASS568 removes that production path and removes `commit_revision_durable_full_exact`. Current relation-data durability is `DerivedRelationTransitionRequest`: source revision + target id + exact relation deltas construct the endpoint inside the runtime. Old kernel regressions use a `cfg(test)` adapter to this derived law; they no longer exercise a distinct full-target WAL descriptor.

`RevisionTransitionRequest` may remain as an in-memory/preparation test primitive; it is no longer a production durable publication API.

## 4. Current alternate capability debt

### OPEN A — advisor statistics / quotient-factor duplication

`UnifiedArtifactId::SemanticStatistics` and `SemanticQuotientFactor` remain consumed by the cyclic optimizer/admission logic and may be manually pinned. They are **CURRENT CAPABILITY**, not dead compatibility code.

PASS568 removes misleading “legacy duplicate/adapter” wording but does not delete the families. Required theorem: cyclic optimization/admission must consume the unified ObservableAtom/SAMF authority directly with equivalent cost/admission semantics. Only then may these artifact families be removed end-to-end.

### OPEN B — public/kernel surface audit after durable commit retirement

`RevisionTransitionRequest` is still exported by `kernel-plan` because non-durable preparation/test machinery uses it. Determine whether any legitimate external kernel consumer needs that arbitrary-target relation request. If not, make it crate-private/test-only in a later API cleanup; do not resurrect durable publication for it.

## 5. Non-legacy words that must not trigger deletion

- replication branch `retired` is domain state, not obsolete implementation;
- `SemanticIndexBinding::compatibility(...)` checks semantic binding compatibility, not version compatibility;
- persistent-vector “compatibility projection” describes an API projection, not a historical decoder;
- “obsolete generation removal” is live GC terminology.

## 6. Sweep rule

1. Every active-kernel discovery of `vN`, `VERSION`, `legacy`, `compat`, deprecated/retired implementation, or multi-version decoding is classified here in the discovering PASS.
2. Before first release, old snapshot compatibility is never preserved by default.
3. One current fail-closed discriminator is allowed as corruption/type framing; pass-number ladders are not.
4. Test-only reference algorithms are allowed only behind `cfg(test)` and may not route production execution.
5. A current alternate capability is removed only after its production consumers have a mathematically equivalent owner.
6. At actual release, create a separate explicit compatibility freeze review; do not infer release obligations from PASS557 wording.

## PASS569 update — semantic artifact family classification

- **CLOSED semantic duplication:** cyclic optimizer/admission and join costing no longer require the concrete `SemanticStatistics` family. They request exact semantic cardinality through one capability owner.
- **CURRENT IMPLEMENTATION PROFILES / OPEN RETIREMENT:** `SemanticStatistics`, `SemanticQuotientFactor`, and `ObservableAtom` still exist as physical retained representations with different memory/maintenance profiles. They are no longer allowed to define separate semantic laws.
- **CAPABILITY LATTICE:** `ExactCardinality <= QuotientFiber <= ObservableFiber`. SAMF answers all three; quotient projection answers the first two; cardinality-only projection answers the first.
- **NEXT CLEANUP:** unify artifact identity/advisor/durable recipe vocabulary around one semantic-fiber profile before deleting compact representations. Do not replace all compact profiles with full SAMF without a measured resource proof.

### PASS569 verification debt discovered during full kernel-plan sweep

- **OPEN / inherited test-only debt from PASS568:** the full `kernel-plan` suite reaches two failures in tests that still exercise the PASS568 `cfg(test)` arbitrary-target `commit_revision` adapter: `committed_transaction_id_rejects_same_revision_id_with_different_revision_content` and `exact_transaction_intent_survives_later_heads_checkpoint_compaction_and_reopen`.
- PASS569 does not modify that adapter or durable transaction publication. The two new semantic-capability regressions pass and the library Clippy/fmt gates pass. Do not count the full kernel-plan suite as green until the PASS568 adapter tests are rewritten against the derived durable law or the adapter is given an exact test-only identity law.

## PASS570 update — one semantic-fiber artifact identity

- **CLOSED PASS569 verification debt:** the two PASS568 retry tests now exercise `DerivedRelationTransitionRequest` directly. Full `kernel-plan` suite is green again (305 passed / 0 failed / 9 ignored).
- **CLOSED artifact identity duplication:** `UnifiedArtifactId::{SemanticStatistics, SemanticQuotientFactor, ObservableAtom}` is replaced by one `UnifiedArtifactId::SemanticFiber { binding, profile }`.
- **CURRENT physical profiles:** `SemanticFiberProfile::{Cardinality, Quotient, Observable}`. Capabilities are derived once from the profile; advisor and durable capability metadata no longer duplicate the lattice.
- **CLOSED telemetry/memory family duplication:** telemetry and `PhysicalArtifactFamily` now identify one semantic-fiber family plus profile rather than three semantic families.
- **MEASURED frontier:** for one 4096-row/257-class binding, deterministic retained estimates were Cardinality=39,456 B, Quotient=848,368 B, Observable=230,192 B. Capability strength is therefore not monotone in retained memory. Never route by a hard-coded “stronger means larger” assumption.
- **OPEN:** durable recipe/install state representations still have separate structs/spec variants. Converge them only after build/update cost is measured alongside retained bytes; identity is already unified and no semantic consumer may depend on those family names.


## PASS575 follow-up — arbitrary-target relation request visibility CLOSED

The PASS568 visibility audit is now closed. Active external crates have no legitimate consumer of `RevisionTransitionRequest`; production durability already publishes relation data through `DerivedRelationTransitionRequest` / mixed/schema-specific exact authorities. `RevisionTransitionRequest` is crate-private, with a `cfg(test)` crate-private root alias only for kernel-plan regressions. No compatibility/public API obligation remains for arbitrary-target relation requests.

## PASS576 update — FORMAT V1 failure diagnostics are representation-neutral

- `FORMAT_VERSION = 1` and all persisted/wire grammars are unchanged.
- Product recovery diagnostics expose `UnsupportedFormat` with the exact known component/version and corruption/protocol byte offsets without adding decoder routing or compatibility fallback.
- Diagnostic enums are presentation/API vocabulary, not persisted format tags and not permission to read older PASS snapshots.
- No salvage decoder, guessed downgrade path, or alternate recovery state machine was added.

## PASS588 — dead SingleFile replication archive discriminator

- `SingleFileSectionKind::ReplicationAuthority = 4` was an unreleased historical P309/P323 generation-section discriminator left in the active decoder after the product had already moved to root-bound immutable `CFAS/CFAO + CFLN` authority objects.
- Production construction had no writer for that section and recovery/compaction already used the immutable object closure, so retaining raw section kind `4` served no current semantic or compatibility capability.
- PASS588 removes the variant/decoder arm. Raw section kind `4` now fails closed as unsupported rather than restoring monolithic archive compatibility or routing. No `FORMAT_VERSION` change is introduced.


## PASS589 — external freshness pre-release authority note

- The current external-freshness wire grammar remains the cut-only current pre-release protocol used by PASS588; PASS589 changes no production bytes.
- Hostile R&D proves that cut-only authority cannot represent an externally anchored pure volatile interval without either leaving the old durable source rollback-admissible or fabricating a durable cut with no durable bytes.
- The selected PASS590 successor is one signed persistence-lineage state machine, `DurableCut -> VolatileFence -> DurableCut`. Because CFMD is still pre-release, implementation must replace the current protocol cleanly rather than add a compatibility decoder/router or preserve the cut-only grammar as fallback.
- `FORMAT_VERSION` is not bumped merely for the PASS589 theorem; any protocol discriminator change belongs to the actual PASS590 wire implementation.

## PASS590 — external freshness authority protocol v2 replaces cut-only pre-release grammar

- The active external-freshness record is `FreshnessAuthorityState::{DurableCut, VolatileFence}` under one signed persistence-lineage domain.
- Built-in TCP authority protocol is v2 and encodes that sum state directly.
- Retired cut-only mutation vocabulary is not decoded, routed or preserved as compatibility surface; CFMD remains pre-release.
- `VolatileFence -> DurableCut` is an ordinary exact CAS in the same protocol, not a fallback/rebind format.
- `FORMAT_VERSION` is unchanged because this is external authority protocol grammar, not durable database file-format compatibility.


## PASS591 — persistence-transition compatibility classification

- No old cut-only decoder/API was restored. `FreshnessCut` remains the payload of the live `DurableCut` variant and is not itself legacy vocabulary.
- `compare_and_rebind_signed` is a current atomic authority-transfer primitive across different lineage/store identities; it is not a compatibility path for `DurableCut -> VolatileFence -> DurableCut`.
- Removal of the internal `RuntimePersistenceAuthority` wrapper changes no persisted/wire grammar.
- Retained-history materialization before demotion uses the current portable historical closure codec and introduces no new version route.
