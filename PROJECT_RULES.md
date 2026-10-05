# CFMD PROJECT RULES — MANDATORY PASS ENTRY CONTRACT

This file is normative for every numbered implementation PASS and R&D-to-mainline integration pass.

## 1. Mandatory read-before-work rule

Before changing production code in **every new PASS**, the executor MUST read:

1. this `PROJECT_RULES.md` in full;
2. the immediately preceding PASS report;
3. the current tail of `docs/status/PRODUCTIZATION_LEDGER.md`;
4. `docs/status/POST_PASS400_MASTER_LEDGER.md` for unresolved long-horizon lines;
5. any R&D report explicitly named by the preceding PASS or by the user as authority for the active line.

Every PASS report MUST contain a `PROJECT RULES / CONTINUATION CONTRACT` section stating that this file was read before implementation and MUST carry this same read-before-work requirement forward to the next PASS. A successor PASS is not allowed to omit this instruction merely because the active subsystem changed.

## 2. Wall-clock execution discipline

- Official PASS start is the real UTC time recorded before implementation work.
- Useful wall clock: 20 minutes.
- Hard wall clock: 24 minutes.
- At hard stop, terminate work/processes and package the honest state, even if incomplete.
- Functional freeze should occur before the useful boundary; after freeze, only deterministic verification, reports, cleanup, integrity and packaging.
- Compile/test the workspace by crate/target in stages; do not assume a monolithic build fits a single tool-call timeout.
- Final repository artifact must not contain `target/`.

## 3. Hostile / R&D architecture law

Before adding fallback, routing, compatibility machinery, generic SQL-shaped algorithms, whole-state recomputation or O(data)/O(history-depth) workarounds, first test whether the CFMD semantic/mathematical kernels admit one exact universal law.

Preferred order:

1. find the semantic owner;
2. recover existing Γ/change/query/transport/lens/proof authority;
3. state the exact law/certificate;
4. implement one universal primitive;
5. specialize/lower for performance;
6. fail closed where proof is absent.

Do not preserve inferior pre-release architecture for compatibility. Legacy/Obsolete sources may be mined for information but are not to be modernized unless still active.

## 4. Schema-epoch transaction law — FORMATION WORLD SEALS AT FIRST BOUNDARY

For an in-flight transaction formed in schema epoch `A`:

- its typed reads, predicates, requirements and client meaning remain in formation world `A`;
- the runtime rebases/certifies the transaction **inside A only** up to the first crossed schema-migration source revision;
- source guards/requirements are evaluated or otherwise exactly certified against the rebased `Candidate<A>` at that boundary;
- successful source certification creates a **formation-world seal**: all A-specific observation semantics end at that logical serialization point;
- schema migration transports only the already-formed exact effect forward (`ΔA -> ΔB`);
- after the boundary, B/C/... epochs perform only their native exact effect conflict/rebase/residual laws;
- original A intent + guard digest remain durable retry identity, but no executable A guard is carried into B;
- final publication remains revision-bound and atomically sealed by existing runtime freshness authority.

Formation sealing fixes the old transaction's logical position before the crossed migration; it does **not** grant permission to retroactively invalidate later current-world transactions. When a transported sealed effect is physically published after B/C-native history, the rebase/braid proof MUST preserve causal observations/requirements of those already-committed later transactions. This proof is entirely native to the later schema epoch: it checks `old transported effect -> later B/C observation`, never `B/C event -> old A guard`.

If a later committed transaction observed a coordinate that the retroactively inserted effect would change, automatic reordering is forbidden unless an explicit observation-commutation/preservation certificate proves the observation remains valid. A mere write/write commutation certificate is insufficient.

For product `Transaction::require`, later-world preservation certificates are current-world causal authority, not transported formation guards. A unary requirement whose complete `SemanticRuleExpr` depends on exactly one field may be normalized to the same rule over `RuleValueExpr::Input` and persisted with that field observation. A retroactive write may cross that observation when the proposed value satisfies the persisted unary rule. Multi-field requirements MUST NOT be decomposed into independent per-field certificates. When the complete current-world requirement and its exact observed field vector are retained as one grouped causal certificate, retroactive writes may be checked only by atomically applying the entire proposed field vector and evaluating the complete deterministic predicate once. Absent such a grouped theorem they fall back to exact-input preservation / coordination-required behavior.

The mainline transaction mechanism MUST NOT require:

- synthesis of a current-schema query `q_B` from old `q_A`;
- an inverse migration `M^-1`;
- execution of an A query against B objects/current B state;
- current-HEAD reinterpretation of A semantics;
- old-schema query routing/fallback;
- transport of an A guard/query/derivative state into B in order to keep that A observation "live" after its formation seal;
- interpreting B-native events as evidence about old A observations;
- full-state migrate/recompute/diff as a production transport algorithm.

A mathematical query factorization, observation transport theorem, or derivative-event theorem may exist as an independent kernel capability, but none is the schema-epoch transaction authority unless a future explicit architecture decision supersedes this rule.

If product semantics require a condition to remain true at the **current publication head** (for example revocation/current-authority freshness), model it as a distinct current-world publication precondition/authority. Do not disguise it as an old formation-world transaction guard.

## 5. One semantic/current-world authority

At any current revision the database has one authoritative current semantic schema. Historical schema epochs may remain retained proof/read authorities for already-bound historical/transaction snapshots, but they do not become alternate current query authorities. Physical realization may lag semantic cutover without exposing a second current semantic world.

### Causal observation group / relational OFC law

Grouped causal-observation payloads are interned once per `(effect_id, group_id)`. Reconstructible coordinate/relation indexes may hold only compact routing references; they MUST NOT copy a full predicate/vector/capsule once per member coordinate.

For general relational/OFC causal observations, `query + observed OFC key + source relation envelope` is not exact authority. Hidden Γ-DTC state can distinguish equal output fibers (for example, an empty join result with or without a hidden matching left fiber). Any future general relational causal certificate therefore MUST retain the exact hidden differential state required by the compiled query. It MUST NOT degrade to a static relation read-set, query replay at each braid, or a per-transaction O(data) copy of `MaterializedRelPlanState`. The selected continuation direction is a shared/interned persistent maintained-state capsule lineage, reconstructible from existing durable causal authority.

PASS485 fixes the carrier law: relational causal state is a shared revision-bound `RelCausalCapsule` over the existing persistent/COW `MaterializedRelPlanState`. Exact braid impact is computed by Γ-DTC delta propagation against that state, not query/model replay. Reopen reconstruction may rewind the existing exact durable semantic relation effects to recover predecessor capsule states; physical row handles and serialized maintained-plan snapshots are not durable causal authority. Observation/history indexes may retain only compact references to shared capsule states.

## 6. Ledger carry rule

Every PASS report and successor handoff MUST carry unresolved lines in these buckets:

- `CLOSED THIS PASS`
- `OPEN — IMMEDIATE`
- `OPEN — DEFERRED / RETURN AFTER CURRENT LINE`
- `SUPERSEDED / DO NOT EXTEND`
- `PERFORMANCE BASELINES TO PRESERVE`
- `NEXT RECOMMENDED PASS`

Context/DX, authorization, Semantic Rules, history/retention, bindings, durability and productization lines do not disappear just because another subsystem is active. A line may disappear only as explicitly `CLOSED`, `SUPERSEDED`, or `REJECTED` with its replacement/reason.

## 7. Required final artifacts

For a numbered implementation PASS, return only:

1. full repository ZIP;
2. `PASSxxx_REPORT.md`;
3. concise overview, verification/performance results, and next target.

The PASS report itself must repeat the mandatory rule for the next PASS: **read `PROJECT_RULES.md` before any implementation work and carry this instruction forward again.**

### PASS486 relational causal identity / reopen law

Relational causal authority is durably identified by the committed effect plus an observation-local id and reconstructible semantic payload `(observed_state_frontier, RelExpr)`. A runtime `capsule_ref` is derived compact routing metadata only and MUST NOT become a WAL/on-disk semantic identifier.

On reopen, group observations by canonical structural query identity, build the maintained query state once at the epoch boundary, rewind that lineage once through existing exact durable relation effects, and intern the resulting persistent capsule states. Hot-relation indexes route compact observation refs to shared capsules; they must not copy query/capsule payload per source occurrence. Retroactive relational effects are certified by exact Γ-DTC capsule impact, deduplicated per observation even when several touched relations route to the same observation.

### PASS487 product relational observation / braid law

Transaction-bound relational reads that can participate in causal normalization MUST capture their exact relational causal certificate from the same maintained Γ-DTC execution that produces the application-visible result. A second query replay for certificate capture is forbidden. Repeated identical `(revision, RelExpr)` reads in one transaction may share one captured maintained state/observation authority.

Canonical stale-effect certification MUST preserve later committed relational observations through the interned capsule/route index before permitting reordering. The same law applies inside retained native schema epochs; formation-world guards still end at `FormationWorldSeal` and are not transported as relational capsules.

Runtime history `effect_id` is the durable causal-ledger `RevisionEffectId`, not the client transaction id. Durable PREPARE assigns that identity before the runtime history root is sealed, and live indexing MUST bind the prepared transition with that exact identity so restart does not alter conflict witnesses or causal prerequisites.

Transaction observation authority belongs to the application-visible read surface, not to mutation lowering. A transaction may retain one observation-enabled formation `ReadContext`, but internal `set`/relationship/query-update/delete lowering MUST use a passive clone of that same exact snapshot with relational capture disabled. Explicit reads through the transaction-bound read surface retain the shared capture authority. This is a semantic distinction, not an operation whitelist: lowering computes the exact effect; only values exposed to application logic become causal observations.

### PASS487/PASS488 relational observation / reopen locality law

Application-visible transaction reads are the only ordinary product surface that creates relational causal observations. Internal mutation/effect lowering uses a passive clone of the same formation snapshot and MUST NOT manufacture anti-dependencies merely by reading data needed to construct an exact effect. Live history binds causal identity only from the durable PREPARE-assigned `RevisionEffectId`; client transaction/idempotency identity is separate.

Relational capsule reopen reconstruction uses one canonical backward history sweep. Unique query lineages are built once at the boundary and indexed by exact compiled source relations. An exact historical effect may rewind only lineages whose source set intersects that effect; unrelated history may not pay per-query delta construction or per-lineage rewind work. The target reconstruction complexity is `O(H + Q + I + O)`, where `H` is scanned exact history, `Q` unique query lineages, `I` actual semantic effect-lineage influences and `O` observation routes. If every one of Q distinct maintained states truly depends on every one of H hot effects, `I = H*Q` is genuine semantic work; do not hide it behind a generic fallback or unsound static shortcut. Any future improvement below that worst case requires an explicit shared maintained-DAG theorem/implementation, not skipped exact Γ-DTC transitions.

### PASS489 shared maintained relational DAG law

Cross-observation maintained-state sharing is legal only for the same canonical semantic `RelExpr` subtree inside one semantic-context/revision world. Source-relation envelopes, equal current outputs, or local ExecGraph node numbers are not semantic state identities. `RelExpr`/`Value` structural equality + hashing are now available as an in-memory derived compiler key; this is not a durable identifier and MUST NOT replace the durable query codec.

The selected continuation architecture separates **maintained state-cell identity** from **operator occurrence/root identity**. A canonical-identical subtree may be transitioned once and its exact Γ-DTC delta fanned out to multiple parents/observations. Observation ids and source-occurrence multiplicity remain distinct. The multi-root forest/DAG is reconstructible runtime authority only: no durable DAG ids, serialized maintained-node snapshots, relation-envelope cache, or recovery-only fallback.

Structural sharing reduces work to unique influenced semantic nodes, but it does not prove every fully-hot query family sub-`H*Q`: non-equivalent stateful quotients remain independently distinguishable. Stronger batching such as equality-value dispatch or ordered-cut routing is allowed only as an exact lowering of the same multi-root forest semantics with equivalence against independent Γ-DTC execution.

### PASS490 canonical relational observation forest law

Canonical-identical `RelExpr` subtrees in one semantic world may share one maintained Γ-DTC state cell across observation roots. State-cell identity and operator/root occurrence identity are separate: repeated occurrences, including the two sides of a self-join, retain distinct fanout edges even when they reference one shared state cell. `RelObservationForest` is the selected multi-root execution owner and must reuse the existing node-local Γ-DTC transition/patch kernels rather than introduce a second query executor.

A multi-root forest has one reconstruction/sweep world anchor but **per-root dependency-frontier revisions**. A root frontier changes only when one of that root's exact source relations changes; unrelated churn must not manufacture a new causal capsule identity. Forest/node ids remain derived runtime coordinates and MUST NOT become durable identifiers.

PASS490 executable equivalence covers Scan/stateless operators, Set support, blocker, Join, AntiJoin, Group, TopK and Union, including self-join occurrence multiplicity. PASS491 may replace PASS488 per-lineage reopen only by using this same forest transition law; recovery-specific caches/fallback executors remain forbidden.

### PASS491 forest-backed reopen / shared-root causal planning law

Relational causal reopen reconstructs all canonical query roots for one semantic epoch as one `RelObservationForest` world, not as independent maintained-plan lineages. A relevant durable relation effect rewinds that forest once; root observation capsules are runtime-only views over a shared forest `Arc` at the requested reconstructed frontier. Causal braid checking MUST batch roots that share the same forest snapshot and plan Γ-DTC impact once for that snapshot, then project the resulting impacts to observation roots. Durable query/observation identity remains unchanged and no forest/node id enters WAL.

The forest transition path is output-sensitive in unique maintained cells and exact fanout. Do not regress to per-root rewind or per-root causal replanning. PASS491 hostile audit also records that the current forest constructor still obtains each local state cell by transiently building an independent subtree plan; this is exact but can duplicate boundary source-data work. The next optimization must replace that construction with bottom-up local-cell initialization from already-built child cells, not add a recovery cache or serialized maintained-state snapshot.

### PASS492 bottom-up canonical forest construction law

`RelObservationForest` construction MUST initialize each canonical semantic state cell bottom-up from already-interned child outputs/state. A parent cell may not obtain its local Γ-DTC state by transiently constructing a complete independent `MaterializedRelPlanState` subtree and discarding the descendants. Canonical source Scan cells are materialized once per semantic source expression in the forest; repeated root/occurrence references reuse that initialized cell while preserving distinct fanout occurrence edges.

Constructor work must be accounted separately from transition/reopen work. The baseline is proportional to unique canonical cells plus each cell's exact local initialization over already-built child outputs; repeated roots/shared prefixes/self-joins must not multiply source materialization or descendant initialization. Recovery-only caches, serialized maintained-state snapshots and durable forest/node ids remain forbidden.

### PASS493 exact parameter-family fusion law

`RelObservationForest` MAY fuse sibling parameterized stateless filters only when they share the exact same canonical parent state cell and semantic operator law. Equality families are keyed by `(parent, column, equivalence)` and dispatch each changed row through the compiled Γ-equivalence canonical key once; all semantically equivalent constants receive that signed row effect. Ordered families are keyed by `(parent, column, ordering)` and compute one canonical order key per changed row, then project that row through exact sorted `<`, `<=`, `>`, `>=` cut sets. Raw host equality/order, relation-envelope heuristics and recovery-only routing are forbidden.

Family fusion is an execution lowering of the existing canonical forest, not a new query representation: every filter root keeps its own `RelExpr`, observation id, dependency frontier and downstream fanout occurrence. Work is output-sensitive: equality dispatch is `O(|Δ| + emitted memberships)` modulo canonical-key/map cost; ordered cuts are `O(|Δ| log Q + emitted memberships)`. Emitted memberships are genuine semantic output work and MUST NOT be hidden by a fallback or approximate batch result.

### PASS494 predicate-family boundary law

PASS493 scalar-family fusion is exact only because one changed row is classified in one pinned semantic quotient/order coordinate. Conjunctions of constant order predicates over the same `(parent, column, ordering)` admit an exact canonical interval normal form (intersection of lower/upper cuts). Once predicates depend on two or more independent semantic coordinates, the exact normal form is a product region; no one existing scalar family key is sufficient. Do not accumulate conjunction-specific routers or flatten such regions into host tuple/hash shortcuts. Any future multidimensional acceleration must be a first-class Γ-aware index algebra with its own exactness law and performance justification. The current parameter-family specialization line is closed; continue product breadth through the existing `RelExpr` / `RelObservationForest` authority.

### PASS495 scoped Context / product read-surface law

The selected product DX authority is `Database` as long-lived schema-neutral runtime authority and one bounded `Context<M>` as the public typed working world / unit-of-work owner. A write-capable scoped Context owns one formation world, Candidate, semantic observations and internal exact intent journal; application-visible reads through that Context are semantic observations by default, while internal mutation/effect lowering reads remain passive. `Transaction` is transitional public plumbing and must ultimately collapse behind the Context-owned intent journal rather than become a second observation model.

Reference traversal (`Ref::load/query`), relationship reads (`Many/OwnedMany` query/load/count/ids), deep typed reference predicates, projections and aggregates MUST lower through the same `RelExpr` execution/capture authority as ordinary object reads. No navigation-specific causal engine or hidden ORM read set is permitted. Merely accessing a materialized Ref/Many field performs no I/O and therefore creates no observation until an explicit read/query operation executes.

The current mainline `ContextSource::Current` is not yet the selected scoped Context: it obtains a fresh committed snapshot for each collection operation. This is transitional and MUST NOT be treated as the final DX owner. The next Context refactor must bind one admitted formation world/Candidate for the scope, preserve read-your-writes incrementally, and reuse the existing relational causal capture + internal intent machinery rather than inventing parallel transaction semantics.

### PASS496 scoped Context formation / Candidate law

`Database::context::<M>()` is a bounded admission operation, not a live façade over a moving HEAD. One `Context<M>` owns one formation `ReadContext`, one internal adaptive exact-intent journal, one observation capture authority and one speculative current world. Ordinary Context reads before writes observe the admitted formation world; staged exact effects advance only the Context's Candidate, never the database HEAD. Drop never commits. A successful Context commit seals that unit of work against further mutation.

The old public `Transaction` machinery may remain as internal implementation substrate during the frontend migration, but a scoped Context must not create a second transaction/rebase/observation engine. Basic Context-owned create/delete/field-patch staging delegates into the existing exact transaction/Plan kernels.

Read-your-writes over a speculative Candidate is semantically valid as a local execution world, but a relational observation taken after a staged effect is **not** reconstructible by the current durable `(committed revision, RelExpr)` observation format. Until ordered intent-prefix/segment authority is durable, such a Context may execute the read but commit MUST fail closed. Do not serialize a virtual Candidate revision as if it were committed, do not drop the observation, and do not replay host callbacks. The next exact payer is a segmented IntentJournal / prefix-qualified observation certificate that lets recovery reconstruct the same virtual observation world.

### PASS497 continuation update — prefix-qualified Candidate observations

Scoped Context observations made after staged writes are now durable exact authorities rather than a fail-closed gap. A candidate observation is reconstructed from `(formation committed revision, exact staged relation-effect prefix, RelExpr)`. The maintained query state itself is never serialized: reopen reconstructs the committed forest at the formation revision and advances it through the prefix with the same Γ-DTC transition algebra. Equal prefixes are interned in the durability codec and grouped during recovery rebuild.

Do not regress this into fake candidate revision ids, callback replay, full-state Candidate snapshots, or a second query engine. The remaining hostile debt is structural: cumulative prefix payloads are still materialized from the cumulative Plan; a future pass should replace cumulative prefix snapshots with a persistent/interned ordered intent-segment chain so `read/write/read/write` depth cannot become O(number_of_segments²) durable prefix payload.

### PASS498 persistent scoped intent-prefix chain law

Candidate observations MUST NOT retain cumulative staged-effect snapshots. Scoped Context owns one persistent ordered intent-prefix lineage: each staged step appends one exact relation segment `Candidate_i -> Candidate_{i+1}` and observations retain only an O(1)-clone reference to the current prefix tail. Equal prefixes therefore share their entire parent chain in memory.

Durable relational observation encoding stores the lineage as topologically ordered `(parent_ref, exact segment)` nodes and each observation stores only its tail reference. Reopen reconstructs shared prefix nodes once and advances the existing `RelObservationForest` through the exact segments in order; it may not flatten the chain back into cumulative O(S^2) payloads, invent candidate revisions, replay application callbacks, or serialize maintained query state.

Prefix-qualified relational observations remain valid retained-schema causal authority. At migration boundaries the observation world is reconstructed inside its native formation epoch from the retained committed formation revision plus its exact prefix chain; after `FormationWorldSeal`, only the already-formed effect transports forward. Prefix reconstruction does not revive old-schema query execution in later epochs.

## PASS499 scoped reference/relationship mutation law

Scoped `Context<M>` is the ordinary public mutation owner for reference fields and many-valued relationships. Reference/optional-reference field patches use the same `Context::set` exact field-coordinate path; `Many<T>` and `OwnedMany<T>` are semantic relationship handles consumed by `Context::{attach,detach,move_to,move_all_to,detach_all,detach_ids,move_ids_to}`. Relationship handles MUST be rebound by semantic owner/relation identity to the Context's current Candidate before mutation lowering, so a handle materialized at an earlier prefix cannot force stale-snapshot reads. Internal edge discovery remains passive (`execute_for_mutation`) and MUST NOT create application causal observations. Every staged relationship effect flows through the existing Context-owned exact intent journal and persistent `DurableIntentPrefix` lineage; no second relationship transaction model or durable relationship-handle namespace is allowed.

## PASS500 atomic scoped Context admission law

Migration-ready typed Context selection MUST start from one linearizable runtime admission token, not from a live schema check followed by a later Context bind. `Database::begin_context()` samples one coherent immutable runtime root; the returned `ContextAdmission` carries both the exact formation revision/root and the schema revision observed at that same linearization point. The token is single-use for typed binding, so migration may publish immediately afterward without changing the already-admitted scope; the next admission observes the new world.

Do not implement this with a long-held migration/router lock, `Context<A,B>`, `MigrationRouter`, `ReaderContext`, old-schema current-world routing, or a client compatibility graph. Existing atomic root publication is the synchronization authority. `Database::context::<M>()` is the direct single-contract convenience path and MUST itself bind through the same admission primitive.

## 8. PASS501 scoped unit-of-work ownership law

Ordinary Rust SDK application code has exactly one typed mutable unit-of-work owner: `Context<M>`. The product facade MUST NOT re-export `Transaction`, `TransactionId`, `TransactionReadiness`, `SchemaDatabase<S>`, or `SchemaDatabaseBuilder<S>` as normal application primitives. The legacy transaction object remains hidden runtime intent/proof plumbing until renamed/refactored internally.

Typed collection sources are now structurally limited to scoped Context or immutable Snapshot worlds. `ContextSource::Current` and the live `SchemaDatabase<S>` facade are removed and MUST NOT be reintroduced. A previously admitted Context never advances merely because HEAD changed.

Strict historical editing is represented by `Snapshot<M>::edit() -> Context<M>`: it reuses the same scoped Candidate/observation/intent machinery, but its internal journal is snapshot-bound and therefore refuses silent transport to a newer HEAD. This is a semantic basis distinction, not a second public transaction vocabulary.

Consumer Context shape is not authoritative schema identity. Opening/binding a subset `Context<M>` against a larger authoritative database is valid when its semantic field bindings are valid; do not restore the old `SchemaDatabase<S>::open` full-schema-equivalence rule.

### PASS502 runtime intent-journal / authoritative creation law

The normal mutable product owner is `Context<M>` only. The runtime's exact-effect carrier is named `IntentJournal` and is hidden implementation/binding plumbing; do not re-export or rebuild it as a second ordinary SDK unit-of-work abstraction. `TransactionId` remains a genuine durable retry/protocol identity and MUST NOT be renamed away merely because the mutable carrier changed names. Strict historical editing remains `Snapshot<M>::edit() -> Context<M>`.

Authoritative typed creation consumes `S: DatabaseDefinition` at creation time and returns schema-neutral `Database`. The selected builder surface is `Database::builder(path).create_authoritative::<S>()`; storage/encryption/notifier configuration remains on the same builder. Consumer `Context<M>` binding is separate. Do not reintroduce `Database<S>`, `SchemaDatabase<S>`, global entity discovery, or compatibility routing. Raw `Schema` construction remains a dynamic/generated-binding primitive rather than normal typed Rust DX.

PASS503 MUST read this file, PASS502 report, both active ledger tails and any authorization authority named by the ledger before implementation. Granular authorization must bind to stable semantic coordinates and current Context operations, not Context shape or source-language names.

## PASS503 scoped authorization law

Restricted typed application work MUST enter through `SessionDatabase::begin_context/context` and carry the same live `RuntimeAuthority` through formation `ReadContext`, speculative Candidate worlds, exact snapshots, historical selection and publication. `Context` MUST NOT expose an unrestricted raw `Database` escape hatch. Authorization is checked on persisted semantic relation/field/action coordinates, never on consumer Rust shape or local spelling.

`HistoricalRead`, `HistoryRead`, watch/read footprints and publication-time session freshness are distinct authorities. `Context::undo_latest` must obtain history through the Context's restricted formation authority. Preview/readiness/publication of a restricted intent must validate its exact bound plan authority, not merely the existence of some generic write grant.

PASS504 MUST read this file, PASS503 report and both active ledger tails before implementation. The next hostile line is authorization/grant transport and current-world publication authority across schema migration; do not solve it with old-schema routing, copied role tables, or Context-shape security.

## 8. Authorization across schema migration — transport requirements, never grants

Authorization is current-world authority and is not a formation-world transaction guard.

For schema migration `A -> B`:

- roles/session grants are **never** rewritten, copied, aliased, or routed through a compatibility ACL table;
- grants name stable current semantic coordinates (`RelationId`, `RelationColumnId`, semantic actions). A definitionally preserved semantic ID therefore remains the same grant identity without transport;
- what migration may transport is the **required publication-authority footprint of an exact effect** through the same verified migration provenance used to transport that effect;
- a row-local split/merge fans one source write requirement out to every target column whose verified transform depends on that source coordinate. The current session must authorize every resulting B coordinate;
- changed/new target semantic IDs do not inherit old grants merely because names/ordinals/types look similar;
- general/global relational rewrites remain fail-closed for local authorization-footprint transport until an exact effect-footprint theorem exists;
- semantic mutation actions (create/delete/relationship attach/detach/move) may cross a migration only under explicit identity-preservation/action law. Do not silently degrade them to generic relation write;
- current session grant generation / revocation freshness must be serialized with the actual durable publication seal. A permission check that releases authority before publication is not sufficient.

This law is independent of client language and applies equally to typed Context, dynamic protocol and hosted bindings.

## PASS505 schema-aware publication authority law

After a transaction crosses a semantic schema boundary, publication authorization MUST be evaluated only against the exact transported current-world effect footprint. Do not attempt source-world publication authorization first. The migration proof that produces the current authority footprint MUST also bind that proof to one exact current HEAD; any later HEAD/schema advance invalidates it for new publication. Already-committed idempotent retry recognition is not a new publication and may precede this freshness rejection.

Semantic object/relationship action authority is stronger than relation-write authority. `CreateObject`, `DeleteObject`, `AttachRelationship`, `DetachRelationship` and `MoveRelationship` may cross a migration only where action identity is explicitly certified. Exact relation passthrough is currently sufficient; row-local relation rewrite is not, even if the relation ID is numerically unchanged. Never widen an unproved action into generic `WriteRelation`.

PASS506 MUST read PASS505 and both active ledger tails before implementation. Its hostile target is to replace separate authority-footprint and effect migration walks with one prepared schema-aware publication certificate, then reuse that certificate for commit/readiness/preview and hosted/dynamic publication.

## PASS506 prepared schema-aware publication law

A stale exact effect that crosses a semantic schema boundary MUST be prepared by one kernel-owned artifact before any product diagnostic or publication decision. `PreparedSchemaAwarePublication` is the selected relation-effect primitive: one retained-epoch traversal derives the transported current effect, exact current publication-authority footprint, exact authorized HEAD, client formation semantic identity and rebase certificate. `commit`, `preview` and `intent_readiness` MUST consume that same prepared meaning; they may not independently re-walk migrations or authorize the source-world plan after a schema boundary.

Preparation transports requirements, never grants. Publication still acquires the live session-generation seal against the prepared current footprint, and the prepared artifact is invalid for new publication when HEAD changes. Durable idempotent retry recognition remains allowed before freshness rejection because it does not publish again.

Unsupported effect classes remain explicit fail-closed gaps. In particular, formation-world `require` predicates, relational causal observations, field/model/lifecycle effects and hosted dynamic commits must not be coerced through the relation-only prepared law. Hosted/dynamic publication must first carry an explicit/certified formation semantic revision; `base_revision` alone is not a semantic identity and historical lookup must not be used as an implicit compatibility router.

## PASS507 continuation law
Hosted/dynamic stale intents must carry an explicit semantic formation identity; never infer it from `base_revision`. Exact history may verify a supplied identity but must not become a permanent O(history) preparation payer: move formation-context verification into the kernel preparation witness. `PreparedSchemaAwarePublication` is the single target artifact for relation and exact field publication. Carrier/lifecycle or mixed model effects may join it only through proved delta-transport laws; never use whole-state reconstruction, old-schema routing, ACL migration, or generic write fallback as compatibility mechanisms.
