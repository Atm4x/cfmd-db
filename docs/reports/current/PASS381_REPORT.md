# PASS381 REPORT — first-class semantic migration history boundary

Start: 2026-10-01 01:21:09 UTC  
Functional freeze: 2026-10-01 01:35:37 UTC  
Useful boundary: 2026-10-01 01:41:09 UTC  
Hard boundary: 2026-10-01 01:45:09 UTC

## Input / continuation authority

PASS381 continues PASS380 together with `CFMD_POST_PASS380_CONTINUATION_LEDGER`. The decisive clarification is that migration-transform invertibility and historical revision materializability are different properties. A retained revision before a schema migration must remain materializable even when the forward transform is non-injective; time travel should enter the historical schema epoch rather than reconstruct it by inverting the current HEAD.

## Hostile finding

The durable layer already records `SchemaMigrationExact` in the same causal effect ledger as ordinary revision effects. However, `kernel-plan::revision_at()` builds its reconstruction graph only from `ExactPlanInverse` effects. Schema migration is correctly `NonPlanTransition` for ordinary Plan undo, so migration edges are omitted and a pre-migration revision is currently unavailable through that reconstruction path.

The bug is conceptual rather than merely missing syntax: the runtime used one `reversibility` classification for two different questions:

1. can this committed transition be inverted as an ordinary current-schema `Plan`?;
2. is the historical world on the other side of a semantic migration boundary still retained/materializable?

Those questions must not share one boolean/enum.

## Closed in PASS381

### 1. First-class `SemanticChangeEvent`

Added `kernel_durability::SemanticChangeEvent` as a projection of the existing committed `DurableRevisionEffectRecord` for `SchemaMigrationExact`.

It carries:

- causal effect identity;
- idempotency epoch and client transaction identity;
- source and target revision;
- source and target schema revision;
- migration lens/spec identity;
- semantic manifest pins;
- complement encoding version;
- historical boundary authority.

This is deliberately **not a second history engine**. An orphan/staged migration complement is not a semantic event; only the committed causal schema-migration effect projects one.

### 2. Historical boundary authority separated from Plan reversibility

Added `HistoricalBoundaryAuthority`:

- `LocalComplement`;
- `ExternalArchive(ArchiveProofId)`;
- `ExplicitlyForgotten`;
- `LocalPayloadReleased`.

This classification is derived from durable complement retention state and is explicitly orthogonal to mathematical invertibility of the forward migration transform.

`retains_history_authority()` is true only for local complement authority and an explicitly named external archive authority.

Important non-claim: local migration-complement authority alone is not asserted to be a complete old database snapshot. Whole-world historical materializability still needs a source-epoch anchor/coverage law.

### 3. Runtime history carries the semantic boundary

`kernel_plan::RuntimeHistoryEffect` now carries `semantic_change: Option<SemanticChangeEvent>`.

`SchemaMigration` remains `RuntimeHistoryReversibility::NonPlanTransition`. This is intentional: PASS381 does not smuggle schema migration into the ordinary Plan undo calculus.

### 4. Product history surface

`cfmd-runtime::HistoryEntry` now exposes `semantic_change()` with a public `HistorySemanticChange` projection and `HistoryBoundaryAuthority`.

A caller can therefore observe one logical semantic history event such as:

```text
R100 / Schema A
  -- SemanticChangeEvent A -> B -->
R101 / Schema B
```

without interpreting physical rewrite/checkpoint/backfill progress as additional user history commits.

### 5. Regression coverage

The existing P380 runtime migration regression now additionally proves:

- migration appears as one `HistoryEffectKind::SchemaMigration` entry;
- ordinary undo classification remains `NonPlanTransition`;
- semantic boundary is `schema 380 -> 381`;
- migration spec identity is preserved;
- current prototype `Forget` policy is visible as `ExplicitlyForgotten`, not silently confused with transform non-invertibility.

A kernel-durability regression independently proves the four boundary-authority states and their retained/non-retained classification.

## R&D conclusion

The clean next primitive is a **historical epoch anchor**, not a generic inverse callback and not a full-row migration fallback.

Conceptually:

```text
SemanticChangeEvent
    source revision/schema epoch A
    target revision/schema epoch B
          |
          +-- historical source-world anchor
                 |
                 +-- old checkpoint/generation pin
                 |   OR compact archival historical representation
                 |   OR another certified materialization authority
                 |
                 +-- retention / GC proof
```

Then:

```text
db.at(revision_in_A)
    -> select epoch A authority
    -> reconstruct inside A
```

rather than:

```text
current B
    -> inverse Migration
    -> guess/reconstruct A
```

This also gives the correct place to solve physical compaction: GC may remove an old physical generation only after another authority proves every retained historical revision covered by that generation remains materializable.

## Explicitly not done

- No arbitrary Rust migration callback was added.
- No generic row-map was added to maintained query IR.
- No second migration/history journal was created.
- `MigrationHistoryPolicy::Forget` was not renamed into a false retention guarantee.
- No claim is made that current `db.at(before_migration)` is solved yet.
- No physical background A->B rewrite protocol was faked before the historical epoch authority exists.

## Verification

Toolchain bootstrap/probe:

- supplied Rust distribution: `rustc 1.98.1 (48a229cea 2026-09-01)`;
- standalone compiled probe executed successfully;
- only required components were extracted for the pass; rematerialized tar copies were deleted after component extraction.

Affected checks/tests:

- `cargo check -p kernel-durability --offline` — PASS;
- `cargo check -p kernel-plan --offline` — PASS;
- `cargo check -p kernel-durability -p kernel-plan -p cfmd-runtime --offline` after formatting — PASS;
- `cargo test -p kernel-durability --lib --offline` — **219 passed / 2 ignored**;
- `cargo test -p cfmd-runtime --offline` — unit **1/1**, async **9/9**, end-to-end **35/35**;
- focused migration semantic-history regression — **1/1**;
- public Rust facade verifier — PASS, including public surface **44/44** and API contract **1/1**.

`scripts/verify-repository.sh` after report insertion and manifest regeneration — **PASS**.

## Next target — PASS382

Develop the historical epoch anchor / retention law:

1. identify the minimal durable source-world authority for a semantic migration boundary;
2. bind it to revision + schema epoch, not frontend names;
3. make generation/checkpoint compaction consult historical coverage instead of blindly deleting the last old-world representation;
4. make `revision_at()` cross `SemanticChangeEvent` by selecting the correct epoch anchor, never by pretending the migration is an ordinary Plan inverse;
5. hostile crash matrix: before semantic publication, after cutover before physical rewrite, mid mixed-representation rewrite, reopen, and post-compaction;
6. only after that replace prototype `Forget` with real retained-history policies and proceed to semantic cutover + background physical materialization.

After migration/history is closed, return to Context reference/relationship patches and field-granular change coordinates as recorded in the continuation ledger.
