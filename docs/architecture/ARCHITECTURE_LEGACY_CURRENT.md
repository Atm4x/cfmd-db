# Workspace architecture

The workspace is deliberately layered so semantic definitions do not depend on storage or optimizer implementations.

- `kernel-types`: stable IDs only.
- `kernel-schema`: constructive type/schema calculus and explicit semantic environment dependencies.
- `kernel-semantics`: pinned exact equality/ordering/tokenizer modules and checked semantic-law resolution.
- `kernel-lifecycle`: deterministic liveness normalization and semantic lifecycle intents.
- `kernel-identity`: only bijective identity transport; split/merge are not transports.
- `kernel-model`: finite model values and revision-state normalization.
- `kernel-change`: universal and fine-grained change representations.
- `kernel-query`: closed exact-query IR plus reconstructible maintained-plan state; no arbitrary host callbacks.
- `kernel-durability`: versioned logical WAL framing/codec plus exact `Revision=(S,Γ,M)` checkpoint generations, immutable manifest publication, rotation/compaction, committed-prefix scanning, and logical replay. Durable authority is the validated checkpoint + committed logical WAL tail selected by the highest published manifest; row handles, layouts, indexes, maintained state, and runtime root identity are reconstructible and are not serialized as semantic authority.
- `kernel-plan`: checked physical lowering/execution plus the authoritative runtime publication boundary. `RuntimeRevisionCell` publishes one immutable `Arc<RuntimeRevisionBundle>` containing the validated `Revision=(S,Γ,M)`, selected authoritative physical layouts, and all registered maintained materializations. `DurableRuntime` binds that cell to one matching `DurableRevisionStore`, so commit/checkpoint/reopen share one durable-head contract. The cell is internally either `Serving(root)` or `RecoveryRequired`; an uncertain durable COMMIT fail-stops publication/read service until validated reopen. Durable mainline publication is `prepare -> durable PREPARE -> seal -> durable COMMIT -> infallible publish`; restart selects a published generation, replays its WAL tail and rebuilds a fresh runtime root.
- `kernel-retention`: explicit + implicit information-flow retention labels.
- `kernel-proof`: tiny PlanIR fragment and proof-certificate checker.
- `storage-memory`: revision DAG and intent-preserving in-memory semantic store.
- `kernel-integration`: cross-layer falsification tests.

Current invariants:

1. every optimization may fail or fall back without changing logical semantics;
2. semantic revisions pin both schema and `Γ`;
3. physical indexes/materializations and storage-resolved row-handle evidence are reconstructible state, not semantic authority;
4. a revision-bound runtime publishes logical revision, physical state, and maintained materializations as one reader-visible root;
5. process-local runtime root lineage/version is freshness identity only and must not become durable semantic identity.
6. only a validated committed logical WAL prefix may advance durable semantic authority; uncommitted PREPARE and reconstructible physical artifacts cannot do so.
7. a returned durable-COMMIT I/O error is an uncertain outcome until WAL recovery, never proof that the semantic commit aborted.

8. maintained Scan row identity is bootstrapped only from exact ordered `(StableRowHandle,row)` bindings; semantic Bag equality is not an identity proof.
9. dense physical/leaf compaction must not define semantic Scan output order; logical order is maintained independently.
10. only a fully published immutable generation manifest may select a checkpoint/WAL pair as restart authority; pending/orphan generation files are non-authoritative.
11. checkpoint/WAL prerequisite directory entries must be durably named before manifest publication, and manifest publication itself ends with a directory durability barrier.
12. restart must validate checkpoint framing/content and the committed WAL prefix, rebuild reconstructible state, and only then return to `Serving`; a missing/corrupt manifest-referenced prerequisite is corruption, not an implicit empty tail.

Pass35 durability falsification note:

- the checkpoint/WAL/manifest protocol is now exercised through externally killed subprocesses at PREPARE, COMMIT, checkpoint, manifest and compaction boundaries on the current Linux/container filesystem;
- a durable COMMIT is recovery authority even if the process dies before runtime root publication/ACK;
- these tests establish process-crash behavior only and are not a claim about sudden machine power loss or other filesystem implementations;
- client transaction/idempotency identity remains required to resolve ACK-loss retries at the API boundary.

Pass36 durable control-plane note:

- every external semantic commit may carry a nominal `ClientTransactionId`; the checksummed WAL PREPARE binds that ID to one source/target logical descriptor, and committed outcomes are retained across checkpoint generations for idempotent ACK-loss retry;
- published generation authority includes `metadata-N.cfdm`, containing durable maintained-materialization specifications and committed transaction outcomes. The final manifest binds its checksum alongside the checkpoint checksum;
- `DurableRuntime::open` reconstructs materializations from durable specs rather than trusting caller configuration;
- `DurableRuntimeSupervisor` is the normal process-level owner for `RecoveryRequired` / uncertain-COMMIT reopen and same-ID outcome resolution/retry.

Pass37 durable semantic-change note:

- WAL mutation codec v3 has two explicit revision-change classes: compact `RelationData` for unchanged semantic context, and canonical `FullRevision` for schema/Γ/lifecycle/field changes; no physical handle/layout/index state enters either durable descriptor;
- `FullRevision` carries an exact canonical target `Revision=(S,Γ,M)` image and recovery revalidates it through the normal `Revision::build` boundary before rebuilding physical/materialized runtime state;
- materialization configuration can now be changed online through an atomic checkpoint-generation rotation. Runtime publication occurs only after the new generation manifest is durable, and reopen obtains the changed configuration from durable metadata;
- same `ClientTransactionId` + same current `RevisionId` no longer suffices for an idempotent success when the supplied target Revision content differs; the runtime reports an explicit target-content mismatch;
- mutation codec v2 relation-data PREPARE payloads remain readable under v3. This is backward payload compatibility, not a complete store-format migration system;
- executable semantic-module implementations remain externally deployed and validated against durable pinned digests; durable packaging of implementation artifacts is still open.

Pass38 durable exact-intent/deployment note:

- new-format committed client identity is `DurableTransactionIntent::Exact`, not merely `ClientTransactionId -> RevisionId`. Exact intent retains canonical target Revision bytes, applicable materialization registry and builtin semantic implementation descriptors, and survives WAL -> checkpoint -> compaction -> restart;
- `FullRevisionAndMaterializations` is one durable transaction class, so a semantic migration and its required maintained registry share one prepare/seal/COMMIT/whole-root publication boundary;
- published generation metadata and exact historical intents carry `BuiltinSemanticModuleSpec` descriptors for equality/tokenizer/ordering implementation revisions. Normal reopen reconstructs the builtin registry before validating durable Revision bytes;
- legacy target-only transaction records remain explicitly weaker and are never silently promoted to exact request identity;
- this deployment model is intentionally limited to builtin semantic implementation families. Arbitrary external executable/module artifact packaging, authentication and lifecycle remain outside current durable authority.

Pass39 semantic-index note:

- `kernel-semantics` is the sole authority for canonical primitive equality/order keys. A planner/operator may use a key only after resolving the exact pinned Γ module; ordinary host `Eq`/`Hash`/`Ord` is not semantic authority.
- `kernel-semantic-index` is reconstructible derivative infrastructure. `SemanticIndexBinding` pins semantic revision, exact module digests and canonical-key encoding revision; `SemanticBucketIndex<Key, Identity>` is generic over derivative identity and is not tied to `StableRowHandle`.
- maintained Join uses the existing I64 specialization first, exact semantic buckets for other current builtin primitive equivalences, and an exact semantic-scan fallback for structural/custom equivalences;
- maintained Group uses composite canonical equality keys when every grouping equivalence is currently primitive/canonicalizable; structural/custom grouping remains scan fallback;
- maintained TopK uses the I64 specialization or an ordered canonical semantic-key/tie-bucket index for current builtin non-I64 orderings;
- these indexes are in-memory reconstructible maintained-query state. Pass39 does not turn them into persisted `PhysicalStore` indexes or durable authority.
- benchmark evidence shows the indexed Join fixes asymptotic scaling but can lose on tiny relations, so future planner work must include an explicit cost/threshold policy rather than assuming every index is always faster.

Pass40 persisted semantic-index note:

- `PhysicalStore` now has reconstructible `MaterializedSemanticIndexState` for every current builtin primitive equality contract, keyed by `CanonicalEqKey` and storing only stable `PhysicalRowId` handles. The existing specialized I64 index remains a separate hot path.
- semantic physical indexes are maintained in the same prepared physical transition as their relation; relation reinstall invalidates them, and a Γ contract mismatch requires rebuild rather than reinterpretation.
- direct physical Filter/Join planning may consume a compatible persisted primitive semantic index through `IndexedPrimitiveIfAvailable`; candidate rows are still verified by the resolved Γ equivalence before output. Missing/incompatible indexes fall back exactly.
- installed-index selection is currently structural, not cost-based. Automatic creation/retention/eviction and small-relation crossover policy remain planner work.
- `Chain<T>` / `Lineage<T>` was hostile-reviewed and was deliberately **not** added to the logical universe. Rooted single-parent acyclic lineage remains a refinement over `PartialMap<Node,Node>` + constraints + derived `Seq` ancestry; adjacency/jump-table/rope structures are reconstructible physical lowerings.

Pass40 final index-layer cleanup: the same identity-generic `kernel-semantic-index::SemanticBucketIndex<Key, Identity>` now backs maintained semantic indexes and persisted primitive physical semantic indexes. Equal-key buckets preserve identity insertion order; exact reverse mapping enforces one live key per identity. `PhysicalRowId` remains only a physical identity parameter, not semantic authority. The specialized I64 storage index remains a deliberate optimized representation.

Pass41 typed-stateful execution note:

- current builtin primitive `Group` and `TopKWithTies` are compositional physical typed-batch producers, not just typed consumers. Their internal `OwnedTypedBatch` may flow through downstream Group/TopK/Filter/Project and is materialized to logical `Row` values only at an external/fallback boundary;
- primitive Group equality is encoded only through pinned-Γ canonical equality keys; structural/custom semantics retain exact fallback;
- exact scalar-I64 maintained TopK uses count-only ordered state, while multi-column I64 and non-I64 domains retain their richer exact representations;
- descending I64 TopK threshold selection is direction-correct (`k-1` ascending, `len-k` descending);
- typed producer state remains reconstructible physical execution state and introduces no new logical value/relation type or durable authority.

## Pass42 — composite semantic access paths

Persisted semantic indexes are reconstructible physical state keyed by an ordered `Vec<SemanticIndexKeyPart { column, equivalence }>` and materialized as exact pinned-Γ vectors of `CanonicalEqKey`. The binding is not semantic authority; every component is resolved against the current `SemanticContext` and incompatible Γ requires rebuild/fallback.

`FilterEqColumns` is the general exact relational predicate for equality between two input columns. Physical planning may recognize a direct two-way `JoinEq` followed by cross-side `FilterEqColumns` predicates and satisfy the complete equality key through one compatible composite persisted index. Returned physical candidates are still revalidated under Γ.

`SemanticAccessCostModel` decides only between full scan and an **already installed** persisted semantic index for current direct Filter/Join paths. Index creation, retention, sharing, eviction and rebuild are deliberately outside this local execution rule and remain optimizer/lifecycle responsibilities.

## Pass43 — semantic-index advisor and lifecycle ownership

The current builtin primitive persisted semantic-index family has a first production lifecycle policy boundary. `SemanticIndexWorkloadSample` supplies an explicit physical workload horizon; the advisor aggregates demand for each exact Γ-bound `SemanticIndexBinding`, charges a missing/stale build once, and chooses profitable managed candidates under a deterministic key-cell budget. This is physical planning policy only; it neither changes query semantics nor enters durable `Revision=(S,Γ,M)` authority.

Advisor ownership is distinct from index existence. A compatible manually/externally installed semantic index may satisfy workload demand without becoming advisor-owned, and the advisor may evict only indexes it created. Missing/stale selected indexes are fully built and validated before reconciliation. A changed reconciliation advances one physical transition and `RuntimeRevisionCell` publishes one new immutable root; a no-op does not create a new root version.

`max_managed_key_cells` is a deterministic proxy for current advisor-managed index footprint, not byte-accurate memory accounting. Workload telemetry/decay/background scheduling, exact resident-memory budgeting, structural/custom canonical indexing, multiway join ordering, and comparison across the specialized I64 index family / generic semantic indexes / future layouts remain OPEN.

## Pass44 — physical Join reassociation boundary

The physical planner may reassociate the current connected contiguous equality-Join fragment while preserving leaf order. Equality edges include `JoinEq` and admitted `FilterEqColumns`; all physical index candidates remain derivative and every indexed candidate row is Γ-revalidated. Current direct/fused I64 paths and transient primitive semantic-index builds are cost-gated rather than unconditional. Arbitrary relation permutation/general bushy planning remains outside the verified fragment.


## Pass45 — unified Join access candidate boundary

Current primitive equality Join physical planning uses one deterministic `JoinAccessDecision` across exact scan, persisted specialized I64, persisted generic semantic, transient specialized I64 and transient generic semantic access. The same boundary is consumed by direct, intermediate/multiway and current fused Join execution. Physical candidates never become semantic authority; pinned Γ equality remains authoritative and indexed candidates are revalidated.

Transient access is speculative physical state. A prospective candidate may be built, but execution must re-cost it from actual distinct-key statistics before relying on it. If the built candidate is unprofitable, the executor returns to exact scan rather than treating the speculative estimate as planner truth. Duplicate-heavy fused execution must not rebuild a rejected transient index through fallback.

The verified multiway planner still preserves leaf order. Arbitrary relation permutation/general bushy search, retained histograms/correlation statistics and lifecycle ownership across all physical index/layout families remain separate OPEN architecture.


## Pass46 — retained semantic statistics

- `PhysicalStore` may retain reconstructible `MaterializedSemanticStatisticsState` keyed by the same `SemanticIndexBinding` used by semantic indexes. It stores canonical Γ-bound key multiplicities and exposes only row/distinct-key summaries to planner costing.
- Index and statistics construction share one binding resolver, so primitive single/composite key statistics cannot silently use a different equality contract from the corresponding persisted semantic index.
- Relation deltas validate and update statistics in the same prepared physical transition as relation/index state. Relation reinstall invalidates statistics; Γ drift requires rebuild rather than reinterpretation.
- `RuntimeRevisionCell` publishes statistics through an immutable root swap under the same semantic revision. Old readers retain their old root and already-prepared transitions become stale.
- Join costing may consume compatible retained distinctness for transient candidates. Statistics remain advisory: inconsistent/stale state is ignored, and exact execution/post-build rechecks remain authoritative.
- Arbitrary leaf permutation is still not admitted: current Join execution has a deterministic row-production order, so general permutation requires an explicit order-restoration/provenance mechanism first.

## Pass47 — physical Join provenance and exact order restoration

The physical optimizer no longer has to equate "same equality tuples" with "same observable Bag sequence" when considering relation permutation. The first order-restored permutation path carries each base leaf's authoritative logical scan ordinal and row fragment through reordered intermediate joins, continues to evaluate all equality predicates under pinned Γ, and restores both original leaf column order and lexicographic original scan-ordinal row order before returning the physical result.

The currently verified fragment is deliberately narrow: three base leaves, primitive equality edges with retained compatible statistics, no relevant persisted index, and a non-contiguous first pair only when estimated first/third join work plus restoration-sort work beats adjacent alternatives. Existing persisted-index execution remains on the Pass44–46 path.

This provenance is reconstructible physical execution state, not a logical value or durable authority. The current implementation is correctness-first and carries logical row fragments; the architectural frontier is N-way subset/bushy enumeration plus stable-handle/typed-native provenance and indexed access inside permuted plans.


## Pass48 — order-preserving semijoin masks supersede final order restoration

Pass47 established the correctness requirement for leaf permutation, but its physical realization (carry tuple provenance, then globally sort completed rows back to original scan-ordinal order) is no longer the current architecture. Hostile review in Pass48 showed that this repairs enumeration order too late and creates avoidable tuple/provenance materialization plus `O(output log output)` restoration work.

The current primitive-equality permutation path instead separates **pruning order** from **result enumeration order**. Pinned-Γ canonical equality keys build execution-local compatibility buckets/bit masks and semijoin support masks. These structures may exploit non-contiguous/selective predicate edges, but final tuples are enumerated directly in original leaf order and original logical scan-ordinal order. Exact Γ predicates are revalidated before output. There is no final order-restoration sort and no `ProvenanceJoinRow` production carrier.

A bounded 3–8-leaf subset DP supplies selectivity/cost evidence for admission; it does not force tuple materialization in the chosen bushy order. Existing persisted singleton-side index access remains preferred when the unified `JoinAccessDecision` says it is cheaper. Compatibility masks are reconstructible execution-local physical state, never semantic or durable authority.

The next physical frontier is adaptive compatibility/access representation: dense bitsets vs sparse ordinal lists vs persisted indexes / future WCOJ-style kernels, richer correlation statistics, memory accounting and a general subset-node access interface before relaxing the current eight-leaf search cap.

## Pass49 — Γ-Quotient Constraint Network supersedes pairwise compatibility masks

Pass48's order-preserving enumeration remains the correctness boundary, but its pairwise compatibility-mask representation is no longer current architecture. Pass49 moves the physical optimizer to a semantic quotient network derived from pinned Γ.

For every equivalence law appearing in the multiway predicate graph, checked `equivalence_refines` determines which finer edges imply that quotient law. Connected components become quotient coordinates. Primitive `CanonicalEqKey` maps rows into these coordinates; redundant equality cliques/cycles factor to one component, finer laws can eliminate same-component coarser constraints, and derived coarser coordinates can connect endpoints that had no direct predicate.

Each quotient coordinate intersects key domains across all participating leaves. Because one physical row may couple several quotient coordinates, row support is propagated monotonically to a finite fixed point before original-order enumeration. Completed tuples are still revalidated through every original pinned-Γ predicate, so the quotient network remains reconstructible physical evidence rather than semantic authority.

This is the preferred direction for CFMD-specific optimization: exploit explicit semantic laws already present in `Γ` rather than adding opaque host-language shortcuts or specialist algorithms by name. It does not claim that generic quotienting/constraint-propagation ideas are new to computer science.



## Pass50 — prepared Γ-QCN and maintained canonical quotient factors

Pass49 established Γ-QCN as the current semantic physical representation for the bounded primitive multiway fragment. Pass50 separates the parts of that representation by their correct reuse/authority boundary.

The quotient **basis** is a function of the prepared physical query shape and pinned Γ refinement/equality laws. `PreparedPlan` therefore compiles it once into private `PreparedSemanticQuotientProgram` metadata. Dynamic, non-prepared execution retains exact on-demand compilation as a fallback. Prepared quotient metadata is neither data state nor semantic authority.

Primitive canonical quotient **endpoint factors** are revision-derived physical state. They live in a dedicated `PhysicalStore` family separate from ordinary semantic access indexes. The implementation deliberately reuses the existing stable-handle canonical semantic-index representation and atomic delta law, but factor existence does not alter `JoinAccessDecision`, advisor ownership or index lifecycle. Prepared plans may explicitly materialize missing factors; compatible factors are then reused as `PhysicalRowId -> CanonicalEqKey` evidence and maintained by subsequent relation deltas.

The complete QCN state is not yet persistent/incremental: N-way common domains, row-support masks and the cross-coordinate support fixed point are still rebuilt per execution. That boundary remains intentional until it can be derived cleanly from CFMD's `Change/Dq` calculus without creating a parallel invalidation/authority system. Final exact Γ predicate revalidation remains mandatory.


## Pass51 Γ-QCN maintained support boundary

Pass51 adds one more reconstructible physical layer under `kernel-plan` without changing the authority graph:

```text
Revision=(S,Γ,M)                         semantic authority
        |
        v
stable physical relation + row handles   reconstructible selected layout
        |
        v
Γ-QCN canonical factors                  fine relation-delta maintained
        |
        v
Γ-QCN support fixed point                exact Change/Replace maintained
        |
        v
original-order enumeration               exact Γ predicates rechecked
```

The support artifact is keyed by the prepared quotient program and participating relation/layout leaves and carries the exact pinned `SemanticContext` plus the current stable-handle vectors. Prepared execution consumes it only under exact coherence. Relation mutation derives a replacement support fixed point inside the unpublished physical candidate after quotient-factor maintenance; no separate invalidation authority is introduced.

The current derivative is correctness-first `Replace`, not a claim of fine local propagation. Support counters/queues, multi-relation derivative coalescing, automatic lifecycle and memory budgeting remain physical optimization work.

## Pass52 — stable-handle local Γ-QCN deletion Dq

Pass51's maintained Γ-QCN support fixed point remains the universal correctness boundary. Pass52 refines that derivative only where the change law is monotone: a pure deletion cannot create support.

Affected leaves are transported by stable `PhysicalRowId`, not by dense ordinal identity. When the current logical handle vector is an ordered subsequence of the old vector, base masks and quotient endpoint keys are remapped to the new ordinals without payload recanonicalization. A constraints-by-leaf dependency queue then propagates only support loss through Γ-QCN quotient coordinates until convergence.

This local derivative is intentionally asymmetric. Insertions/resurrection may activate a mutually supporting fixed-point component and therefore continue to use the exact Pass51 `Replace` derivative until a separately proved activation closure exists. The local path is an optimization refinement of `Change/Dq`, never a second invalidation or authority graph.

The next architecture question is whether insertion activation and complete-revision coalescing admit equally clean derivatives. If they require ad-hoc cache invalidation semantics, the Pass51 Replace boundary remains the accepted correctness implementation.



## Pass53 — semantic quotient lowering outside Join

Pass53 generalizes the same architectural rule used by Γ-QCN: when Γ or the finite relation model already supplies an exact quotient/edge law, physical execution should compile that law rather than repeatedly invoking a generic scan. Structural equality now has compositional in-memory `CanonicalEqKey`; set-support state indexes those keys; reachability and schema inclusion relations compile to deterministic adjacency.

Authority boundaries remain explicit. Primitive/structural Γ laws define equality; canonical keys and adjacency are reconstructible derivatives. Schema durability stores direct inclusions, not adjacency. Structural canonical keys are not yet a durable semantic-index encoding. Rejected high-churn bucket representations remain outside production until the multi-family advisor can choose among read- and mutation-oriented families.

## Pass54 — structural Γ canonical Group production

Pass53's compositional structural `CanonicalEqKey` is now consumed by maintained Group, not merely exposed by the semantic layer. Group physical admission has three explicit layers: specialized I64, pre-resolved all-primitive canonical keys, and general structural/mixed Γ canonical keys. Unsupported future/custom laws retain exact semantic fallback.

The same canonical contract is used for group lookup, lookup repair after `swap_remove`, and maintained-delta key equality. Canonical keys are reconstructible physical evidence under the state-pinned `SemanticContext`; the logical group representative and aggregate remain semantic payload.

No recursive structural key is persisted under the current durable encoding revision. Structural Join/index planning remains a separate frontier.


## Pass55 — Γ quotient relation multiset lowering

Pass55 extends the Pass53 structural canonical law from support/group lookup into relation-level equality and delta comparison. When every row coordinate admits an exact canonical Γ key, a Bag is compiled to `CanonicalRowKey -> multiplicity`; equality is map equality and diff is multiplicity subtraction while source representatives are enumerated in original order.

This is a physical quotient representation, not semantic authority. The old pairwise `equivalent(...)` matcher remains the exact fallback for future/custom laws without canonical keys. Recursive structural keys remain in-memory only; durable encoding and structural Join access stay separate boundaries.


Pass56 maintained structural-Join note:

- maintained Join has no current `GenericScan` physical family; I64, primitive canonical and structural Γ-canonical bucket families cover the admitted equality universe;
- structural buckets are reconstructible state tied to exact semantic context and are maintained atomically with query deltas;
- persisted recursive structural key encoding remains a separate future physical/durability contract rather than being smuggled into current semantic-index encoding.


## Pass57 — COW physical/runtime roots

Heavy reconstructible state no longer relies on whole-payload cloning to create unpublished candidates. `MaterializedRelPlanState` shares its recursive maintained plan through `Arc` and uses `Arc::make_mut` on mutation paths. `PhysicalStore` shares installed relations, indexes, quotient factors/support and statistics through per-artifact `Arc` roots; mutation performs COW only for touched artifacts.

Prepared physical freshness is checked by a non-durable in-process root identity witness plus epoch/revision guards. This identity is physical concurrency metadata, not semantic authority and not restart identity. Logical transition validation compares invariant state directly and reconstructs only affected relations rather than cloning all of `DatabaseState`.

The map layer itself remains `BTreeMap<K, Arc<State>>`; therefore artifact-map metadata is not yet persistent and historical persistent-root work is only partially closed.


Pass58 dense identity / Γ-QCN derivative note:

- finite revision identity may be compiled to deterministic `LocalEntityId` ordinals for reconstructible dense physical structures, while `EntityId` remains logical/durable authority;
- lifecycle roots/edges may be lowered to local-ID adjacency under the same mapping;
- materialized Γ-QCN support uses separate monotone derivatives: stable-handle loss propagation for pure deletion and connected-component greatest-fixed-point refresh for pure insertion/resurrection;
- mixed delete+insert remains exact rebuild fallback; no second support authority is introduced.


## Pass59 — Γ-QCN mixed Dq and revision-batch coalescing

The Γ-QCN support layer now has one exact local derivative for every ordinary relation-delta shape. Stable row-handle transport distinguishes survivors, deletions and insertions (including slot reuse through generation); inserted keys come from already-maintained quotient factors; only the connected quotient-constraint component is restarted from full support and pruned to its greatest fixed point. Pure deletion keeps the cheaper loss-only propagation path.

Revision preparation defers only Γ-QCN support maintenance while constructing the unpublished candidate. All base relations/factors are first moved to the target revision, then each affected support binding is maintained once from the complete physical change set. Public single-relation mutation remains immediate. This is batching of reconstructible derivatives, not a relaxation of the publication or authority boundary.

## Pass60 — Program3 dense-runtime closeout

Pass60 completes the production integration boundary for revision-local dense identity without changing logical identity. `Revision` owns one shared `Arc<DenseEntityIds>`; `DenseTypeExtents`, dense lifecycle state and reverse LiveRef sensitivity compile against that same coordinate system. `MaintainedDenseLifecycle` supplies an exact local least-fixed-point derivative for lifecycle fact changes. `DatabaseState::normalize` uses `LiveRefSensitivityIndex` rather than repeated whole-model live-reference scans. `NativeColumn::DenseLiveEntityIds` adds an explicit LocalId-backed physical family while retaining external `EntityId` materialization and the legacy external-ID column family.

`LocalEntityId` remains reconstructible revision-local physical identity and is never persisted as logical data. Program4 algebraic-native layout remains outside the authoritative tree.

## Pass61 — structural Γ-QCN quotient factors

Γ-QCN quotient factors are now a dedicated physical derivative rather than an alias of the generic semantic-index state. A factor binds one relation/layout/column/equivalence coordinate, stores exact Γ `CanonicalEqKey` buckets plus stable-handle reverse keys, and is rebuildable from the relation under the pinned `SemanticContext`.

The admission law is `SemanticRegistry::canonical_equivalence_key`, so both primitive and schema-declared structural equivalences can be materialized exactly. Future/custom equality without a canonical representation remains outside the quotient-factor family. Ordinary relation deltas validate and maintain the factor inside the existing candidate/COW transition boundary; semantic-context changes require rebuild.

This closes the in-memory structural Γ-QCN factor gap without defining a durable recursive-key encoding. Persisted canonical-key versioning/migration remains a separate authority boundary.


## Pass62 — algebraic native structural physical family

Pass62 integrates the independently researched Program4 structural layout as a normal reconstructible physical family under `kernel-plan`.

`AlgebraicNativeColumn` recursively erases the logical structural wrappers into constructor-shaped storage: Product child columns, Sum tags/payload ordinals, Option payload ordinals, flattened offset-based Seq/Set, Bag payload+multiplicity, Map key/value payloads and guarded Mu/Var recursion. Scalar leaves continue to reuse the existing specialist `NativeColumn` representations. The logical `TypeExpr` is retained as a reconstruction/type certificate; the physical representation is not another model of record.

Structural equality is consumed through the ordinary typed-batch compiler. `TypedBatchPredicateKind::Algebraic` binds a constant to the exact pinned-Γ `CanonicalEqKey` once and compares candidate keys directly from the algebraic column. This makes the representation compositional with downstream typed stateful operators instead of adding a new pattern-specific execution island. Primitive I64 remains on its existing specialist microkernel.

The authority boundary is explicit:

```text
validated Revision=(S,Γ,M)
        |
        +-> typed rows / TypeExpr / exact Γ laws
                |
                +-> AlgebraicNativeColumn     reconstructible
                +-> DenseLiveEntityIds        reconstructible
                +-> semantic indexes/QCN      reconstructible
```

Low-level algebraic mutation/canonical-key helpers are crate-internal. Physical relation mutation still goes through the candidate/stable-handle `PhysicalStore` transition. Public algebraic construction validates guarded/free-variable type laws before recursive descent, preventing invalid `μX.X` from turning a malformed physical request into uncontrolled recursion.

Program3 dense identity and Program4 algebraic storage intentionally remain sibling physical families. A relation can mix an algebraic predicate column with `DenseLiveEntityIds`; nested LiveRef leaves inside an algebraic value currently retain external IDs. Choosing dense-local nested leaves, chunked payloads or another representation belongs to the future multi-family advisor/lifecycle layer and must not be smuggled into logical semantics.

Durable recursive structural-key encoding, persisted structural index representation, structural ordering, plugin canonical-law packaging and the remaining physical layout families remain OPEN.


# Pass63 correction — multi-family physical inventory and memory budget

`PhysicalStore` now exposes one typed physical-artifact inventory across relation layouts, shared dense identity backing, specialized I64 indexes, semantic indexes, Γ-QCN factors/support and retained semantic statistics. Retained-memory policy separates deterministic estimated bytes from workload benefit; semantic-index lifecycle now has managed and global byte ceilings and treats non-owned families as fixed cost. Shared `Arc<DenseEntityIds>` backing is accounted once. The estimate is planning-grade and overflow-saturating; it is not allocator/RSS truth. Autonomous family-specific lifecycle remains OPEN outside semantic indexes.

Post-freeze Program7 review did not change production architecture: the proposed delta-native durability intent weakens exact retry identity under the current independent-target request API and was not integrated.

## Pass64 — workload lifecycle for Γ-QCN endpoint factors

Pass63's multi-family inventory now has a second production-managed family. A prepared Γ-QCN plan exposes its exact quotient endpoint bindings; for a path already selected by the current physical cost model, repeated-read workload evidence may justify materializing `PhysicalRowId -> CanonicalEqKey` factors once and retaining them under the shared global byte budget. Compatible factors reduce the planner's quotient canonicalization work to zero and are consumed directly by Γ-QCN execution.

Ownership remains explicit. Advisor-created factors may be evicted by later advice; an explicit `materialize_semantic_quotient_factors` call transfers them to manual ownership without requiring a rebuild. Manual compatible factors are reusable but never silently adopted. Stale manual artifact replacement receives exact replacement-byte credit rather than old+new double charging.

This policy is intentionally conservative: it does not yet speculate that factor construction would itself change the preferred join path, and it does not price future write/delta maintenance. Those require the broader workload/counterfactual advisor rather than being hidden inside a local Γ-QCN heuristic. The semantic authority remains `Revision=(S,Γ,M)`; all factors remain reconstructible physical derivatives.


## Pass65 revision compiler boundary

Relation-only revision compilation has a constructive source-bound fast path. `Revision::relation_update_candidate()` returns `RelationUpdateCandidate<'a>`, which borrows the exact source Revision, owns an immutable-cloned logical candidate state, and exposes relation-row replacement only. Candidate build revalidates the pinned semantic context, locally applies the same dangling-LiveRef row normalization as full revision construction, validates only touched relations, recompiles only touched reverse-LiveRef partitions, and reuses source dense identity, dense lifecycle and dense type extents.

This boundary is intentionally stronger than the original Program5 R&D API: callers cannot provide a different source at build time. Recovery therefore derives every relation-data candidate from the current authoritative Revision and cannot mix untouched state from one revision with physical/validation derivatives from another.

The logical candidate state is still obtained by `DatabaseState::clone()`. That clone is not treated as a compiler requirement or semantic authority; replacing it with persistent/path-copy logical state is an explicit OPEN.


## Pass66 — persistent logical roots and persisted-I64 lifecycle

Pass66 removes the full logical snapshot clone that remained after Pass65. `DatabaseState` keeps ordinary extensional value semantics while its large roots are COW-backed: carriers/fields/lifecycle share `Arc` roots and relation payloads are independently shared beneath a COW relation directory. A relation candidate therefore does not deep-copy unrelated logical payloads. The boundary is intentionally narrower than a general persistent-map or typed derivative claim: first write can still copy BTreeMap metadata and current relation deltas can reconstruct the touched relation as a whole.

The Pass63/64 physical lifecycle contract now manages a third family: persisted exact-I64 indexes. Advice is admitted only for Join paths that actually consume the persisted I64 family, applies read-amortized build cost plus global/managed retained-byte budgets, rejects unprofitable work before building a candidate, and may evict only advisor-owned indexes. Manual installation transfers ownership out of the advisor. Semantic indexes + Γ-QCN endpoint factors + I64 indexes are managed; statistics, Γ-QCN support and layout families remain outside the complete lifecycle law.


## Pass67 architecture correction — advisor statistics + COW physical directories

Pass67 extends the reconstructible physical-artifact lifecycle with a conservative semantic-statistics family for direct primitive Join workloads. The statistic is admitted only when the existing cost model and executor consume exact distinct cardinality and the retained state amortizes its build; exact persisted access artifacts suppress redundant statistics. Manual installation transfers ownership out of the advisor, and immutable runtime publication occurs only on physical change.

The outer `PhysicalStore` directories are now COW `Arc` roots rather than value-owned `BTreeMap<K, Arc<State>>` metadata. Candidate clones share relation/index/statistics/QCN/ownership directories, and mutation detaches only artifact families with affected bindings. This closes the Pass57 outer-physical-catalog clone boundary while preserving the invariant that all such roots are reconstructible physical state, never semantic authority.

## Pass68 — exact delta-authoritative durability surface

Pass68 integrates the corrected Program7 by splitting two logically different request authorities instead of weakening exact retry identity.

`DurableRuntime::commit_revision` remains a compatibility/full-authority surface: the client supplies a complete target `Revision`, so durable idempotency must retain that complete canonical target witness because `RevisionId` is nominal rather than content-addressed.

`DurableRuntime::commit_derived_relation_data` is a separate delta-authoritative surface. The client supplies only source/target revision ids and typed relation deltas. The runtime derives the target from the live authoritative source under pinned Γ, then passes it through the existing prepared/sealed publication boundary. Compact `RelationDataExact` intent is exact on this surface because there is no independent target content that can be omitted.

```text
legacy full-target request
    caller target Revision -----------------------> full exact durable witness

compact relation-data request
    source id + target id + typed delta
                    |
                    v
        live authoritative source Revision
                    |
        source-bound derived target
                    |
       prepare -> durable PREPARE
                    |
              freshness seal
                    |
             durable COMMIT
                    |
          infallible publication
```

The seal still checks process-local root identity and source RevisionId, so a target derived from a stale snapshot cannot publish over a moved runtime root. Recovery replays relation-data deltas from the durable checkpoint/WAL source chain; reconstructible physical state remains outside durable authority.

This change fixes per-transaction snapshot duplication for relation-data intents. It does not define transaction-outcome retention/GC, streaming checkpointing, general durable-format migration, group commit, replication or authenticated storage.

## Pass69 — versioned Γ canonical-key binding

Long-lived canonical-key artifacts now share one reconstructible compatibility boundary: pinned semantic revision, exact primitive/module dependency closure, and a stable canonical-key format revision. `CanonicalEqKey` has an explicit recursive v1 byte grammar with fail-closed bounded decode and golden-byte regression. Semantic indexes, Γ-QCN endpoint factors/support and semantic statistics rebuild on semantic/dependency/key-format mismatch rather than interpreting stale cache state.

Generic semantic indexes may now bind schema-declared structural equivalence and consume the same authoritative Γ canonical keys as structural maintained query paths. This does not make index bytes semantic authority, and it does not imply that physical index payloads are currently persisted by the durability layer. Durable structural-index payload storage/recovery, plugin deployment and structural ordering remain separate architecture frontiers.

## Pass70 — compiled Γ executable cache

Pinned Γ now has a reusable compiled equality program for the structural equality universe. `CompiledEquivalence` resolves primitive certified modules once and lowers Product/Option/Sum/Seq/Set/Bag/Map/guarded Mu/Var structure to direct node indices. It is a reconstructible executable derivative, not semantic authority.

Pass69's versioned key binding remains the reuse/migration certificate. Pass70 additionally binds that certificate to the exact structural-definition closure, because nominal schema/environment revision IDs alone cannot exclude two different structural graphs reusing the same identifiers. `RebuildStructuralDefinitions` therefore fails closed before a stale key/compiled program is reused.

Compiled programs are consumed by Γ-QCN quotient factors, algebraic structural filters and structural semantic-index key parts. Primitive semantic-index leaves retain direct resolved-module specialization. Durable serialization of compiled programs is neither needed nor claimed; they can always be reconstructed from the pinned Revision.

# Pass71 architecture correction — durable derivative recipes

Durability may persist a recipe for a reconstructible physical artifact, but not layout-local `PhysicalRowId` payloads as logical authority. On reopen, semantic indexes, Γ-QCN factors and semantic statistics are rebuilt from the recovered authoritative Revision and pinned Γ under the fresh recovery layout. Valid but incompatible recipes are discarded as optional derived state; malformed/unknown recipe formats fail closed. Layout-independent recipe collapse preserves manual pinning by making manual ownership dominate advisor ownership.

Specialized I64 index recovery remains contingent on a durable typed-layout lowering recipe and is intentionally not reconstructed on the generic recovery RowStore.

# Pass72 architecture correction — certified quotient-hypergraph execution

The Γ-QCN physical planner may use a prepared quotient-hypergraph search order. For 3..=8 leaves, the existing subset-DP cost gate remains authoritative. For >8 leaves, QCN is admitted only when semantic quotient edges cover every leaf and GYO reduction succeeds; otherwise the prior fallback remains authoritative.

Maintained quotient support uses duplicate-safe per-leaf live row counts plus static/dynamic leaf-support counters. QCN enumeration carries physical row ordinals/handles and materializes full logical rows only after a complete assignment survives; complete assignments are sorted back into logical leaf order before materialization.

These structures are reconstructible physical derivatives. Pass69/70 key-binding/compiled-Γ authority rules and Pass71 durable recipe boundaries remain unchanged.

# Pass73 — bounded cyclic Γ-QCN search certificate

The Γ-QCN engine no longer treats GYO reducibility as a correctness prerequisite. Fully-covered non-GYO quotient hypergraphs may use a deterministic min-fill search order when a conservative pre-enumeration work certificate proves the current ordinal-scan DFS fits the bounded cyclic budget. Every original Γ predicate is still verified before materialization, so the certificate controls performance admission, not semantics.

The work certificate validates the search permutation first, uses saturating arithmetic, charges complete physical ordinal scans at each recursion depth, and charges final materialization. Over-budget cyclic programs abort before DFS and transparently use the existing exact fallback. The GYO branch remains separately certified and unbounded by this cyclic threshold. Advisor quotient-factor creation performs the same preflight.

# Pass73 implementation correction — bounded cyclic/non-GYO Γ-QCN

**[VERIFIED]** GYO-reducibility is no longer the only >8-leaf Γ-QCN admission certificate. A fully-covered non-GYO/cyclic quotient hypergraph may derive a deterministic min-fill search order and execute under `BoundedCyclic` when a conservative upper bound for the current ordinal-scan DFS fits the configured work budget.

**[VERIFIED]** The cyclic work bound validates the search permutation before arithmetic, uses saturating prefix × physical-row-count scan costs plus terminal materialization cost, and rejects over-budget programs before DFS. A genuine ten-leaf non-GYO cycle matches the logical Γ evaluator exactly; duplicate-heavy cases reject pre-enumeration.

**[VERIFIED]** maintained quotient support remains exact after relation delta on the cyclic branch. Quotient-factor advice performs the same admission preflight, so no endpoint factor is created for an over-budget cyclic path. `multiway_join_cyclic_budget_rejections` distinguishes this fallback from generic QCN inapplicability.

**[OPEN]** This is bounded cyclic exact execution, not a general worst-case-optimal join or unrestricted hypertree-width planner. The historical multiway/bushy item therefore remains OPEN.

**[LEDGER]** Pass72 had 22 active historical OPEN. Pass73 closes no complete historical ledger item and introduces no new one; the authoritative count remains **22**.


## Pass74 — cyclic prefix indexing and durable typed-layout recovery

Bounded cyclic Γ-QCN execution may derive reconstructible prefix candidate indexes from canonical quotient keys. These indexes are execution derivatives only: the original pinned-Γ constraints remain authoritative and are rechecked on candidate assignments. Their build/lookup work participates in the cyclic work certificate.

Durable physical recovery persists lowering **recipes**, never layout-local identity. Recipe v2 may request row-store, value-columnar, I64-columnar or typed-columnar reconstruction and exact I64 index rebuild. Recovery derives fresh physical state from the authoritative recovered Revision. Live entity references are rebound through that Revision's fresh dense-ID table. Stale or contradictory physical advice falls back to recovery row-store; corrupt/unsupported format remains fail-closed.

## Pass75 — bounded advisor-owned physical reconstruction

Recovery now has an explicit physical-reconstruction policy boundary instead of rebuilding every durable optional artifact recipe eagerly and without resource admission. `PhysicalRecoveryPolicy` bounds advisor-owned reconstruction by deterministic row/key-part evaluation count, advisor-retained estimated bytes and global estimated retained bytes. Relation layouts and manual durable pins are fixed recovery intent: they rebuild before advisor admission and are never silently evicted by the recovery policy.

`PhysicalRecoveryReport` records rebuilt recipes, key-evaluation-budget skips, byte-budget skips and incompatible/stale drops. Logical recovery remains authoritative through recovered `Revision=(S,Γ,M)` regardless of optional derivative admission. The default `DurableRuntime::open` remains behavior-compatible through an unlimited policy, while `open_with_recovery_policy` exposes bounded reconstruction explicitly.

The process-level `DurableRuntimeSupervisor` stores the selected recovery policy and reuses it for explicit and automatic reopen after fail-stop. A bounded initial open therefore cannot silently revert to unlimited artifact reconstruction during recovery retry.

The key-evaluation budget is deliberately a deterministic planning proxy, not a claim about CPU time: one structural `CanonicalEqKey` may have recursively large payload. Estimated retained bytes remain the Pass63 planning-grade accounting model rather than allocator/RSS truth. Benefit-ranked/background rebuild scheduling, structural-key-size-aware costing, exact allocator pressure and arbitrary future artifact-family reconstruction remain OPEN.

# Pass76 — continuable work-aware recovery scheduling

Pass76 keeps the Pass75 authority split but removes two accidental recovery-policy artifacts. Recovery preflights compatible optional recipes before reconstruction, fixed/manual recipes remain first, and advisor-owned recipes are ordered by deterministic rebuild key-evaluation work rather than by semantic identifier/recipe order. This ordering is a resource scheduler only; it does not claim workload benefit.

A bounded reopen may now be followed by `resume_deferred_physical_recovery`. Only advisor-owned recipes that were skipped by resource budgets are eligible. They are rebuilt against the current authoritative Revision, pinned Γ and current relation layouts. Successful continuation publishes a fresh immutable runtime-root version under the same logical Revision; a no-op/incompatible continuation publishes nothing. `DurableRuntimeSupervisor` exposes the same continuation boundary without forcing another reopen.

The continuation set is reconstructible evidence, not authority. Manual recipes are filtered even from fabricated reports, stale recipes fail open, and normal checkpointing persists only the physical artifacts actually present after continuation. Benefit-ranked/autonomous scheduling, exact structural-key work, allocator/RSS pressure and future artifact families remain outside this boundary.


## Pass77 semantic observable / Γ-APNF foundation

Pass77 adds a new semantic layer inside `kernel-semantics` without changing the authority hierarchy.

- `CanonicalEqKey` remains the exact Γ-derived representative law; unordered structural constructors now normalize to finite counting measures, so coarse child-key collisions have one canonical representation. Key codec revision v2 is explicit and old physical key caches rebuild rather than reinterpret.
- `RevisionObservableCatalog` is a reconstructible realization of exact semantic coordinates for one pinned `SemanticRevision`. `RevisionObservableId` and `EqClassId` are nominal catalog-local IDs, not durable identities or schema IDs.
- product observables are the generic conjunction/meet mechanism. There is no Set/Bag/Map-specific observable exception.
- `CertifiedSemanticMorphism` has product-capable n-ary source and m-ary target. A raw observation table is not a public certification boundary; production factor-derived determinants are certified from a complete `RevisionFiniteMeasure` object.
- `DeterminantTheory` treats the least closure operator induced by certified morphisms as semantic normal form. Any direct/composed morphism may remain physically materialized even when closure-redundant.
- `RevisionFiniteMeasure -> AnchorMeasureState` is lossless on distinct support and preserves weights. Anchor selection is a deterministic inclusion-minimal physical choice, never a claim of minimum-cardinality key optimality.
- `AnchorPullbackNormalForm` is query-local and reconstructible. It is not stored as semantic authority and Pass77 does not route general `RelExpr` execution through it.

The intended future boundary is: semantic correctness is expressed by pinned Γ observables, finite measures, certified determinant closure and anchor-pullback compatibility; GYO/QCN/WCOJ-like machinery may survive as physical residual strategies during migration.


## Pass78 resource/materialization boundary — semantic work + Γ-SAMF

Pass78 introduces a representation-independent `SemanticWorkEstimate` for recovery admission and the first common observable-support materialization substrate. Semantic work counts logical nodes and stable text payload bytes and must not be interpreted as CPU/RSS truth. Manual durable intent remains non-budget-evictable; only advisor derivatives consume the semantic-work ceiling.

`SupportAtomFabric<RowId>` represents the finite partition induced by a revision-local product observable. The product `EqClassId` is the atom identity; no separate durable atom namespace exists. `MaterializedObservableAtomState` instantiates this over `PhysicalRowId`, owns only reconstructible catalog-local observable/class IDs, and keeps exact joint fibers, projected fibers and a certified product projection. Relation deltas maintain the derivative under the same pinned Γ; layout replacement invalidates it; publication remains COW under the same logical Revision.

This is a staged migration boundary. Legacy semantic index/statistics/quotient/Group/TopK states remain production oracles. Pass78 does not yet define the unified `ObservableDemand` advisor, durable SAMF recipes, exact/pruned Pareto capability selection, generated delta compiler or family retirement. The later Γ-GCC R&D is also not merged: grounded finite hyperrule closure is a candidate shared semantic substrate for determinant/fixpoint/lifecycle recursion in a later pass.

# Pass79 architecture update — Γ Grounded Closure Calculus

Finite grounded least closure is now a shared mathematical kernel in `kernel-grounded-closure`. The crate is intentionally below Γ/schema/physical authority: it knows only dense finite atom IDs, finite hyperrules, seeds, incidence indexes and grounded proof witnesses.

Semantic consumers compile into it. `DeterminantTheory` lowers certified observable morphisms to hyperrules; `kernel-fixpoint` lowers graph edges to unary rules. `MaintainedDenseLifecycle` remains a specialized physical lowering, with exhaustive parity against the same grounded-closure law.

The generic certificate requires both closure and a finite strict-rank proof from seeds/rules, so an ungrounded cycle cannot certify itself. Dynamic deletion follows selected witness dependencies and locally re-fixpoints only the invalidated witness cone; recursive liveness is therefore not reduced to nonrecursive support counts.

This architecture deliberately separates one semantic calculus from many physical implementations. Future positive recursion may use Γ-GCC only after rule instances are compiled through the Γ observable/APNF/SAMF layer and the recursive carrier is certified finite-height/idempotent. Unrestricted recursive Bag/Natural multiplicity is not implied.


# Pass80 architecture update — convergence runtime

Pass80 moves the observable/APNF/SAMF/GCC research chain into actual query/runtime execution while preserving the authority boundary.

- General prepared finite multiway execution owns query-local observable coordinates, compiles leaf finite measures and APNF anchors, executes residual compatibility pullback and expands exact physical rows. QCN/GYO remain specialized physical paths/oracles.
- SAMF is an independent reconstructible physical capability: exact fibers can satisfy semantic access directly, exact statistics derive from atom masses, QCN may consume row signatures, semantic-key implementation binding is pinned, and `ObservableAtom` has a durable recovery recipe.
- `RelDifferentialProgram` is the semantic incremental-maintenance contract carried by `MaterializedRelPlanState`; concrete maintained state must satisfy its required capabilities. Manual delta kernels are implementations/oracles, not a second semantic calculus.
- Γ-BFC uses `kernel-grounded-closure` structural reconciliation for QCN insertion, deletion and resurrection. Stable row-handle generation participates in BFC atom identity so slot reuse cannot revive stale proof state.
- `RuntimeViolationState` makes Γ-VMF a checked candidate-publication boundary. Bootstrap/prepare/seal require exact zero violation under the target Revision. Relation-only mutations may maintain only relation-local witness families because schema/carriers/fields are unchanged by construction.
- `RuntimeObservationGuard` makes Γ-OFC a root/revision-bound runtime contract. Relation sensitivity is a routing accelerator; exact impact is pinned Γ-DTC with full-recompute oracle.

Still intentionally outside the Pass80 architecture closure: positive recursive/PWRC execution, SAMF annotation/ordered overlays and unified advisor, fully generated DTC state lowering, generic invariant compilation and bounded repair execution. Later write-side and distributed/security R&D closeouts are future production layers, not implicit current capability.
