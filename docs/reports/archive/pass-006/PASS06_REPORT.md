# CFMD Pass 06 — Identity-coordinate conjugation, generic semantic modules, unified proof boundary

Date: 2026-09-19
Wall-clock target: 15 minutes
Baseline: pass05-final (110 tests)
Final strict test count: 120 tests

## 1. Identity transport now has a retention-domain, not merely a live-set domain

**Problem.** Pass05 required a bijective identity transport to cover exactly the entities still live in the snapshot where transport occurred. That is insufficient for branch merge: an atom can be locally GC-pruned in one branch while another branch later supplies a new live path from the LCA. Historical lifecycle intents still mention the atom even though the local snapshot no longer does.

**Hypothesis.** Identity-coordinate transport used by revision history must be defined over an identity retention-domain that may strictly contain the current live set. State transport must map only the live subset and must never resurrect the rest merely because they occur in the proof-domain.

**Implementation.** `BijectiveIdentityRevisionTransport` now accepts a source identity-domain that is a superset of the current live entities. `transport_database_state` maps only identities actually present in canonical state. `HistoricalEntityId` no longer silently falls back to the old ID when a mapping is absent: every AtomId that occurs in canonical state must be covered by the transport-domain.

**Falsification.** A two-element transport-domain is applied to a state containing only one live element. The second target ID is not resurrected. A historical ID absent from the transport-domain is rejected with `IdentitySourceCoverageMismatch`.

**Result.** The executable model now matches the retention-scoped identity thesis rather than conflating “known to history” with “currently live”.

## 2. Lifecycle intents are conjugated through identity bijections

**Problem.** Pass05 correctly blocked lifecycle merge across a bijective AtomId transport because history facts were expressed in old coordinates. Merely declaring the states isomorphic is insufficient: `Root(1)` must become `Root(11)`, and the LCA graph must be expressed in the same coordinate system before merge.

**Implementation.** The verified identity transport now transports `LifecycleIntent` and `LifecycleGraph`. `storage-memory` stores the actual verified identity transport on the revision edge, not only an edge tag. `lifecycle_trace_since` walks a linear branch history forward from the LCA and carries:

- the LCA lifecycle graph in descendant coordinates;
- the composed lifecycle intent in descendant coordinates;
- the composed bijection from LCA identity coordinates to descendant coordinates.

Mathematically, earlier intents are conjugated by the accumulated coordinate transformation. When two traces are merged, each is aligned to a chosen target coordinate system via

`f_branch^{-1} ; f_target`.

Only then are the two intents merged and lifecycle normalization applied.

**Result.** A bijective identity edge is no longer inherently opaque to lifecycle merge.

## 3. Hostile GC → identity transport → cross-branch rescue now restores canonical data

**Problem.** A lifecycle-only proof could still hide a serious bug: merge might restore the entity ID but lose carrier membership/field data that local GC removed.

**Falsification scenario.** LCA has atoms 1, 2, 3, root 1 and root 3, edge `1 -> 2`, plus a carrier membership for atom 2 and field value `42` owned by atom 2.

- left branch removes `1 -> 2`; local normalization removes atom 2, its carrier membership, and field value;
- left branch then transports the retention-domain `{1,2,3}` to `{11,12,13}` even though 2 is no longer locally live;
- right branch, still in old coordinates, adds `3 -> 2`;
- merge targets the new coordinate system.

**Result.** Atom 12 is live after merge, `13 -> 12` exists, carrier membership for 12 is restored, and field value `42` is restored under owner 12. The data comes from the transported LCA state; merge does not “revive an empty identity”.

## 4. Identity-coordinate choice is explicit when semantics cannot determine it

**Problem.** Identity renumbering may occur while `(S, Γ)` remains unchanged. In that case both branches have the same semantic context but different ID coordinate systems. Picking left or right silently would be an arbitrary storage heuristic.

**Implementation.** The existing `merge_lifecycle_branches` returns `AmbiguousIdentityCoordinate` when context cannot determine the coordinate system. Added `merge_lifecycle_branches_in_revision_space(..., coordinate_revision)`, where the caller explicitly selects the left or right revision as the coordinate target.

**Falsification.** Same `(S, Γ)`, left branch remains at ID 1, right branch renumbers 1 -> 11. Automatic merge reports ambiguity. Explicit target `right` succeeds and returns root 11.

**Result.** Coordinate choice is part of merge intent, not an undocumented left/right tie-breaker.

## 5. Impact/change semantics commute with identity transport

**Problem.** Transporting state but not query constants/change values would make transaction sensitivity dependent on accidental ID coordinates.

**Implementation.** `kernel-transport` now exposes transport for `Value`, `Change<Value>`, and `ExactQuery`, including entity-ref constants embedded in the query IR. Added an executable commuting-square check:

`Impact(q, x, dx) == Impact(Tq, Tx, Tdx)`

for bijective identity transport `T`.

**Result.** The basic exact-query impact semantics is coordinate-invariant under verified identity isomorphism.

## 6. SemanticRegistry generalized beyond equality

**Problem.** Pass05’s contract/implementation split was only exercised by equality modules. A registry architecture that works only for equality is not the claimed general semantic environment.

**Implementation.** Added a second executable module kind, `TokenizerModule`, with versioned implementations. Registry availability now spans module kinds, while operations that require equality explicitly reject tokenizer digests as `WrongModuleKind` instead of misreporting `ModuleUnavailable`.

Implemented tokenizer contracts:

- `AsciiWhitespace`;
- `AsciiWhitespaceLowercase`.

Tokenizer execution is resolved exclusively through the pinned Γ binding.

**Falsification.** A tokenizer digest cannot masquerade as an equality module. Two implementation revisions of `AsciiWhitespace` are semantically equivalent; `AsciiWhitespace -> AsciiWhitespaceLowercase` is not.

## 7. Existing Γ transport automatically generalized to tokenizer contracts

**Test.** `EquivalentSemanticEnvironmentTransport` accepts tokenizer implementation v1 -> v2 for the same contract without any tokenizer-specific code in the transport layer. A tokenizer contract change is rejected as `SemanticContractChanged` and is accepted only by `SemanticLawMigration`.

**Result.** `module kind + semantic contract + implementation revision` is genuinely reusable architecture, not equality-specific plumbing.

## 8. One common checked-certificate boundary

**Problem.** `kernel-fixpoint` and `kernel-proof` each had their own notion of a checked certificate, beginning to duplicate the trusted boundary.

**Implementation.** `kernel-proof` now owns generic:

- `CertificateChecker`;
- `CheckedCertificate<C>`;
- `verify_certificate`.

`kernel-fixpoint` adapts its certified solver through this boundary instead of defining a separate checked wrapper. Optimizer rewrite verification was also moved onto the same `CheckedCertificate` path via `RewriteChecker`/`verify_rewrite`.

**Result.** Fixed-point certificates and optimizer rewrite certificates now share one proof-admission mechanism. Domain-specific checkers remain separate; the checked-object boundary is common.

## 9. Verification gate

Final checks:

- `cargo fmt --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --release` — 120/120 PASS
- external crates — 0
- `unsafe` — 0
- production `HashMap` / `HashSet` — 0
- TODO/FIXME — 0

## 10. Remaining real frontier

1. Transport relational changes / query sensitivities, not only scalar `ExactQuery` impact, through identity/schema isomorphisms.
2. General semantic module kinds beyond tokenizer/equality: timezone, collation/order, certified functions, numeric environments.
3. External implementation certification: a caller must not be able to claim a contract ID for arbitrary binary behavior.
4. Nominal carrier/type transport; split/merge must remain lineage, not be disguised as bijective identity transport.
5. Move the current toy PlanIR toward the actual relational/nested execution IR while retaining the common certificate checker.
6. Lens complements / CompatibilityVault under retention and strict-erasure policy.
7. Persistent fragments/WAL/recovery, then physical lowering and benchmark falsification of the zero-abstraction-tax claim.

## 11. Final hostile/static audit

- production sections before `#[cfg(test)]`: 0 `unwrap` / `expect` / `panic!` / `unreachable!` call sites;
- `Cargo.lock`: 0 registry/git sources, 18 local workspace packages;
- pass05 -> pass06 changed implementation surface: `kernel-fixpoint`, `kernel-identity`, `kernel-proof`, `kernel-query`, `kernel-semantics`, `kernel-transport`, `storage-memory` plus their local Cargo metadata;
- Rust source size after pass06: 10,018 LOC.

The production panic scan deliberately excludes test modules; test code retains assertions/unwraps as test harness convenience.
