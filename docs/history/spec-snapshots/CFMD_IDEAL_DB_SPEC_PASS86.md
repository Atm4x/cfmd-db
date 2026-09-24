# CFMD — актуальная спецификация идеальной математической БД

**Статус документа:** нормативный ориентир проекта, snapshot после исследовательского журнала `db_model_research_2026-09-18.md` и verified implementation through **Pass80** (2026-09-21).

**Назначение:** этот файл можно отдавать другому агенту как самостоятельное описание того, **что именно строится**, какие свойства являются частью идеальной семантики, какие уже подтверждены кодом/тестами, какие физические решения временные, и какие проблемы остаются открытыми.

> В этом документе «идеальная БД» означает не фантазию «быстрее всех всегда», а минимальное логическое ядро, которое не вынуждает худшее физическое представление и позволяет специализированной реализации стирать высокий уровень абстракции до обычных row/column/index/graph структур.

---

## 0. Легенда статусов

- **[NORM]** — нормативная конечная теория: менять только после нового контрпримера/доказательства необходимости.
- **[VERIFIED]** — реализовано и проверено текущим Rust workspace.
- **[PARTIAL]** — архитектура реализована, но покрытие/производительность/доказательство неполные.
- **[OPEN]** — часть идеальной системы, ещё не реализована или не механизирована.
- **[REJECTED]** — сознательно исключённая архитектурная идея.

---

# 1. Одна строка, определяющая БД

**[NORM]** Логическое состояние базы — конечная модель одной неизменяемой ревизии:

```text
Revision R = (Schema S, SemanticEnvironment Γ, finite Model M)
```

Где:

- `S` задаёт типы, номинальные carriers, relations, constraints, допустимые rewrites и зависимости от семантических модулей;
- `Γ` фиксирует все внешние детерминированные правила, от которых зависит смысл: equality/collation, ordering, timezone rules, numeric semantics, certified functions, tokenizer/model versions и т.п.;
- `M` — конечный типизированный экземпляр схемы в этой точке истории.

**Физические страницы, row/column layout, B-tree, CSR, inverted index, ANN, materialized view, cache, WAL representation и т.д. не являются вторым логическим состоянием.** Это сертификаты/кодировки/производные артефакты одной и той же ревизии.

---

# 2. Логический типовой универсум

## 2.1 Structural values

**[NORM]** Базовый язык значений намеренно маленький:

```text
Scalar
Product / Record
Sum / Variant
Option
Set<T>
Bag<T>
Seq<T>
Map<K,V>
μX.F(X)       // guarded positive recursive structural value
```

Ключевой принцип: тип коллекции задаёт семантику, а не синтаксис хранения.

- `Set<T>` — extensional uniqueness;
- `Bag<T>` — multiplicity;
- `Seq<T>` — семантический порядок;
- `Map<K,V>` — функциональное отображение.

Нет:

- универсального SQL `NULL`;
- неявного bag-default;
- случайного порядка Set/Bag из B-tree/hash iteration;
- автоматических lossy coercions между Set/Bag/Seq.

Partial operations возвращают `Option`/`Result` или требуют refinement-domain; host exceptions не являются query semantics.

## 2.2 Nominal values

**[NORM]** Структурное равенство и nominal identity принципиально различаются.

```text
Entity<E>
AtomId<E>
LiveRef<E>
HistoricalId<E>
Inclusion / capability membership
```

`AtomId` не равен «совпадающим данным». Реальная coreference/deduplication — отдельная versioned relation/resolution view.

Коррекция privacy: `AtomId` стабилен и не переиспользуется **внутри retention domain**, но privileged semantic/physical erasure может уничтожить даже токен идентичности.

## 2.3 Structure

**[NORM]** Базовые структурные связи:

```text
TotalMap
PartialMap
n-ary Relation
```

Relation — обычная типизированная структура, а не фундаментально отдельная «табличная модель». Graph, object links и n-ary facts кодируются той же relation semantics.

---

# 3. Все привычные модели — surface views, не peer databases

**[NORM]** Один kernel должен принимать ergonomic surface syntax, но не поддерживать независимые «object/document/graph modes» с разными законами.

Типичное elaboration:

```text
entity/class             -> nominal Entity carrier + typed maps/relations
record/value object      -> Product
closed enum              -> Sum
optional                 -> Option
list/array               -> Seq
set                      -> Set
multiset                 -> Bag
map/dictionary           -> Map
recursive document       -> guarded μ
reference                -> typed Ref/Id
interface/capability     -> subcarrier/inclusion + required symbols
inheritance              -> coherent inclusion
owned/shared lifecycle   -> RetentionRoot/KeepsAlive + constraints
computed property        -> Query/View
business constraint      -> violation query required empty
transaction method       -> typed Rewrite
```

Следовательно:

- relational workload — Relation / Set<Record> / Bag<Record>;
- document workload — nested structural values;
- graph — entity carriers + relation/adjacency projection;
- object repository — nominal entities + maps/relations;
- key/value — `Map<K,V>`;
- stream — subscription to revision changes;
- current state/history — два представления одного revision/change system.

**[OPEN]** Полный surface→kernel preservation theorem ещё не механизирован.

---

# 4. SemanticEnvironment Γ — смысл является версионированными данными

**[NORM]** Нельзя считать locale/FPU/timezone/collation/library callback невидимым окружением.

В `Γ` входят content-addressed semantic modules, например:

- text equality / collation;
- ordering;
- exact numeric/equality rules;
- timezone database version;
- normalization/casefold;
- certified functions;
- tokenizer/model version для логически наблюдаемых операторов.

`CertifiedFn` допустима только как deterministic total logical function, привязанная к semantic specification/module и refinement certificate/trusted implementation.

Если implementation меняется, но доказанно реализует ту же specification — semantic module id может остаться. Если меняется смысл — меняется module id и обычная revision/dependency machinery инвалидирует зависимые результаты.

**[VERIFIED]** Equality и ordering modules уже versioned; congruence/refinement admission проверяется через certificates. Logical Set/Bag/Relation equality не наследуется от Rust `Eq/Ord`.

---

# 5. Query calculus

## 5.1 Exact queries

**[NORM]** Exact query:

```text
q : Revision × explicit_inputs -> T
```

должна быть:

- total в своей логической domain semantics;
- pure;
- extensional;
- deterministic относительно pinned `(S, Γ, M)` и explicit inputs;
- closed/declarative, чтобы optimizer видел структуру.

Arbitrary host `Fn` не является ExactQuery.

**[VERIFIED]** В коде запросы представлены closed typed IR, а не Rust closures.

## 5.2 Recursion

**[NORM]** Recursive query — explicit least fixed point над admitted monotone finite-height domain либо solver с termination/correctness certificate.

Specialized solver не должен расширять trusted core. Два пути:

1. result-certified: `(result, certificate)` проверяется маленьким verifier;
2. module-certified: implementation module один раз доказывает refinement относительно declarative lfp specification.

**[VERIFIED/PARTIAL]** Guarded structural `μ/Var` equivalence реализована; unguarded/free recursion отвергается. Общая ecosystem fixed-point physical solvers ещё открыта.

## 5.3 Approximation

**[NORM]** Approximate/heuristic computation нельзя silently подставить вместо exact query.

```text
Exact Query<T>
ApproxQuery<T, Contract P>
HeuristicSearch<AlgorithmId, BuildId, Seed/Budget>
```

ANN/AQP допускается только если application явно просит weaker semantics. Ambient RNG запрещён; seed — explicit input либо отдельный probabilistic effect.

---

# 6. Change semantics — универсальный слой изменений

## 6.1 Universal change exists for every logical type

**[NORM]** Минимум:

```text
Change<T> =
    NoChange
  | Replace(T)
  | Fine(FineChange<T>)
```

```text
apply(x, NoChange)   = x
apply(x, Replace(y)) = y
```

Значит exact derivative существует для любого total query хотя бы через recomputation:

```text
Dq_replace(x, dx) = Replace(q(apply(x, dx)))
```

Разделение фундаментальное:

```text
logical completeness       always
incremental correctness    always
incremental efficiency     optional optimization
```

Fine changes — только refinement той же semantics: set insert/delete, bag weight delta, map patch, sequence splice, relation delta и т.п.

## 6.2 Composition

Если `Df` и `Dg` корректны, то:

```text
D(g∘f)(a, da) = Dg(f(a), Df(a, da))
```

Эта chain law является базой incremental view maintenance, transaction repair и maintained physical artifacts.

**[VERIFIED/PARTIAL]** Текущий RelExpr имеет exact recompute oracle и compositional delta paths. `Project(Set)`/`Distinct` имеют long-lived support state. Join, Group и TopK имеют long-lived state. Pass26 добавил `MaterializedRelPlanState`: все текущие `RelExpr` variants (`Scan`, `Filter`, `Project`, `Join`, `Distinct`, `Group`, `TopKWithTies`, `PromoteToBag`) строятся bottom-up из child snapshots и владеют recursive child state; exact child `RelationDelta` проталкивается вверх без model replay. Pass27 подключил authoritative storage row identity к Scan, устранив повторный semantic membership scan. Pass28 ввёл correctness-first prepared storage+plan transition; Pass29 обобщил его до multi-relation one-root `prepare -> seal -> publish`; Pass30 сделал root authoritative для полного validated `Revision=(S,Γ,M)` и registry maintained materializations. Pass31 добавил конкретную reader-visible публикацию через `RuntimeRevisionCell = RwLock<Arc<RuntimeRevisionBundle>>`, nominal root-lineage/version freshness и capability sealing. Старый `StorageCertifiedRelationDelta` удалён как authority concept: storage→query boundary теперь использует `StorageResolvedRelationDelta` как проверяемое non-authoritative row-identity evidence. Pass32 интегрировал logical-authoritative WAL tail: versioned relation-data codec, PREPARE/COMMIT durability barriers, validated committed-prefix scan, exact-base replay и reconstruction нового runtime root. Pass33 закрыл hostile defects в точной storage→Scan identity/order привязке и сделал uncertain durable COMMIT fail-stop состоянием `RecoveryRequired`. Pass34 добавил exact durable checkpoint полного `Revision=(S,Γ,M)`, immutable checkpoint/WAL generations, manifest publication с directory-fsync ordering, rotation/compaction и `DurableRuntime` как единый restart owner. Physical handles/indexes/materializations не являются durable authority. Open теперь — empirical process/filesystem crash falsification, durable materialization/config authority, COW candidate state и Schema/Γ-changing durable migration transactions.

---

# 7. Transactions — typed rewrites, а не набор page writes

**[NORM]** Mutation — typed rewrite/transition:

```text
τ : Revision-domain -> Revision-domain
```

или, в indexed/change notation, `δ : s -> s'`.

Commit semantics:

1. применить explicit rewrite к candidate model;
2. нормализовать lifecycle;
3. ограничить carriers/facts результирующей live model;
4. проверить strong refs;
5. проверить invariants;
6. atomically publish новую revision.

Никакое промежуточное dangling state не наблюдается.

## 7.1 Observation-based transaction repair

**[NORM]** Concurrency должна отслеживать semantic observations, а не только physical read sets.

Если транзакция читала `q_i(R)`, validity после concurrent delta определяется `Dq_i` / `NoImpact`, что естественно покрывает phantoms и semantic dependencies.

Repair obligations:

- observation preservation/repair;
- rewrite transport/commutation;
- postcondition/invariant preservation.

Если доказано commuting square — операция может быть rebased. Если нет — typed conflict/serialization, без «магического CRDT для всего».

---

# 8. Invariants

**[NORM]** Валидные состояния образуют подпространство `Valid ⊆ Model`.

Два уровня:

1. structural/static constraints, выражаемые типами/schema;
2. global declarative invariants как finite total violation queries:

```text
M valid wrt P  <=>  Viol_P(M) = ∅
```

Incremental maintenance — optimization; correctness всегда определена from-scratch semantics.

Host callbacks не могут silently участвовать в integrity semantics.

---

# 9. Lifecycle / ownership

**[NORM]** Ownership не выводится из «foreign key shape» и не задаётся процедурными CASCADE chains.

Для candidate model `M*`:

```text
Roots(M*)
K(M*) = KeepsAlive relation
Live(M*) = μX. Roots(M*) ∪ K(M*)[X]
```

Затем model normal form = restriction lifecycle-managed carriers to `Live(M*)`, далее invariants.

Следствия:

- independently persistent entity — root;
- exclusive ownership — дополнительная cardinality law;
- shared ownership — несколько incoming KeepsAlive paths;
- cycles не self-root: SCC жив пока достижим от root;
- strong ref требует live target, но сам по себе не обязан удерживать lifetime;
- weak/history refs не создают KeepsAlive;
- cascade deletion = reachability normalization, а не отдельный delete engine.

**[VERIFIED]** Lifecycle normalization, idempotence и hostile reachability cases реализованы и тестируются; модель ограничивается целиком, а не только отдельным lifecycle graph.

---

# 10. Identity

**[NORM]** Нужно различать:

1. primitive nominal identity (`AtomId`);
2. domain-level coreference / alias / resolver;
3. canonical presentation/view identity.

Identity transport автоматически допустим только как bijection/isomorphism. Split/merge equivalence classes не являются «сохранением той же identity»: создаётся новая derived identity/resolution + lineage.

Изменение key equality/collation также может split/merge quotient classes, поэтому identity transport зависит не только от schema, но и от `Γ`.

**[VERIFIED]** Identity transports образуют проверяемую groupoid-like структуру; non-bijective split/merge rejected. Live refs и historical IDs разделены.

---

# 11. Schema evolution

**[NORM]** Schema versions immutable. По умолчанию нет destructive reinterpretation старых байтов.

Evolution — checked interpretation/lens:

```text
L : Schema_i <-> Schema_j
```

Она отображает state и, где нужно, changes; information loss удерживается в explicit complement/residual. Irreversible loss — explicit `Forget`, а не побочный эффект миграции.

Historical query имеет две оси:

```text
as_of(revision r, schema = schema_at_r)
as_of(revision r, through target_schema S_k)
```

Если lens chain отсутствует — typed inability, а не silently reinterpreted data.

Concurrent data rewrite через schema change разрешён только если существует transported rewrite с commuting law:

```text
F ∘ τ_old  ==  τ_new ∘ F
```

**[PARTIAL]** Versioned schema/environment transports, conservative extensions и identity-coordinate conjugation реализованы фрагментарно. Полный lens/complement engine и merge-cube theorem остаются open.

---

# 12. Influence = provenance + sensitivity

**[NORM]** Их нельзя сливать.

```text
Influence {
    provenance,    // что кодирует/объясняет текущий результат
    sensitivity    // какие допустимые изменения способны его изменить
}
```

- provenance: explanation/audit/direct information influence;
- sensitivity: invalidation, transaction repair, deletion effects, cache maintenance.

Anti-join counterexample показывает, почему deletion нельзя определять только положительной provenance: удаление tuple, которого нет в output lineage, может создать output.

Sound summaries могут coarsen/widen (token → region → top). False positives = лишняя repair/rebuild; false negatives запрещены.

**[PARTIAL]** `Impact/Unaffected/Changed/Unknown` и semantic relational deltas существуют; полный unified Influence calculus не механизирован.

---

# 13. Deletion / privacy guarantees

**[NORM]** Один глагол `DELETE` не должен обещать несовместимые свойства.

```text
D0 Logical deletion
D1 System-derived consistency deletion
D2 Physical/cryptographic erasure inside controlled domain
D3 Identity/existence erasure
D4 Inference-aware privacy deletion
```

D1 требует sensitivity-aware repair всех database-maintained derived artifacts.

D2 имеет явный storage/backup/replica threat boundary.

D3 допускает уничтожение AtomId/tombstone trace.

D4 не может быть гарантирован автоматически для arbitrary correlated retained data без privacy/utility tradeoff.

Privacy/security — orthogonal policy/effect layer:

```text
access/release labels
retention/erasure labels + pc-flow
explicit authority/declassification
ErasureIndependent certificate for label removal
```

**[PARTIAL]** Closed IFC-style retention primitives существуют; полный transition-level retention theorem и D1–D4 engine не завершены.

---

# 14. History and distribution

## 14.1 History

**[NORM]** Revision immutable, representation mutable.

Эквивалентные physical forms:

- snapshot;
- snapshot + deltas;
- compressed delta segments;
- deduplicated immutable fragments;
- hot materialized current state.

Compaction не должна менять semantics. Historical availability — explicit retention capability.

## 14.2 Revision DAG

**[VERIFIED]** История уже моделируется DAG, а не «единственной линейной parent chain». Multiple LCAs возможны; engine не выдумывает unique merge base.

Lifecycle intents сохраняются относительно parent, потому что нормализованный snapshot сам по себе теряет merge intent.

## 14.3 Distribution

**[NORM]** Distribution добавляет coordination contracts, не вторую data model:

- strong linearizable/serializable head;
- mergeable branches только с доказанными commutation/I-confluence/coherence laws;
- erasure barrier для реплик/key holders.

Automatic merge существует только если согласованы schema, identity, data rewrites и target invariants. Иначе explicit typed conflict.

**[OPEN]** Полная distributed implementation отсутствует.

---

# 15. Proof-carrying trust boundary

**[NORM]** Большой optimizer/solver не входит целиком в TCB.

```text
untrusted search / optimizer / solver
                |
                v
       executable plan/module + proof
                |
                v
         small trusted checker
```

Минимальные judgments:

```text
WellFormed(...)
Equivalent(q1,q2)
Refines(plan,spec)
HasLaw(op,law)
PreservesInvariant(rewrite,invariant)
Terminates(solver,witness)
```

Каждый certificate привязан к exact law/spec/module/version hashes.

**[VERIFIED/PARTIAL]** Единый `CheckedCertificate` boundary используется для semantic modules, ordering laws и PlanIR lowering. Это executable архитектура, но не формальная proof-assistant mechanization всего kernel.

---

# 16. PlanIR и physical model

## 16.1 Нормативная граница

**[NORM]** Trusted execution vocabulary должен быть маленьким:

```text
Source / Decode
Map / Filter / Project
Union / Difference / Distinct
Lookup / Join
Group / Aggregate
Order / TopK
StructuralFold
FixpointCall(certified solver)
Encode / Sink
```

Hash join, merge join, WCOJ, SIMD, CSR traversal, nested decoder и т.п. — implementations/refinements, не новые semantics.

`Source/Decode` не может быть escape hatch «вызови arbitrary backend и поверь».

## 16.2 Physical lowerability target

**[NORM]** Отвергнута невозможная цель «универсальная БД всегда не хуже любой специализированной».

Нормативная цель:

> Semantic model не должен **вынуждать** physical representation асимптотически хуже стандартной специализированной структуры для того же declared workload class.

Примеры erasing lowering:

```text
Scalar/Product       -> native scalar / rows / columns
Option                -> validity bitmap + payload
Sum                   -> tag + payloads
Set/Bag               -> hash/sorted/column vectors
Seq                   -> offsets/tree/rope/order index
Map                   -> hash/sorted map / child arrays
μ                     -> tagged nested/chunked + streaming decoder
Entity AtomId         -> stable external ID, dense LocalId internal
capability/subtype    -> bitmap/tag/extent index
TotalMap dense IDs    -> direct column/array
Relation              -> flat columns / index / CSR / WCOJ input
```

Generalization tax должен уходить в compiler/metadata/adaptation, а не быть обязательным wrapper на каждой записи.

---

# 17. Текущая executable physical reality — Pass17

Эта секция **не определяет идеальную семантику**, а фиксирует, насколько реализация к ней приблизилась.

## 17.1 Workspace

**[VERIFIED]** Pass17 baseline:

- Rust 1.98.1;
- 19 workspace crates;
- 194 declared tests after Pass17 stable-row-handle/index coverage;
- debug + release tests PASS;
- strict Clippy PASS;
- fmt PASS;
- release build PASS;
- strict rustdoc PASS;
- overflow-check release tests для plan/integration PASS;
- no external Cargo registry/git sources;
- 0 `unsafe`;
- 0 TODO/FIXME/panic-shaped macros.

## 17.2 Typed physical columns

**[VERIFIED]** `NativeRelation::TypedColumnar` / `NativeColumn` покрывает все текущие scalar carriers:

- Unit;
- Bool;
- I64;
- exact F64 bits;
- Text;
- LiveEntityRef IDs с entity type;
- HistoricalEntityId IDs с entity type.

Physical storage валидируется против pinned schema. Hot filter/project kernel dispatches по physical type один раз **до row loop**, а не по `Value` variant на каждой строке.

Первый naïve unified batch дал 1.566× hand-written I64 baseline — вариант признан плохим и заменён hoisted dispatch. Финальные процессы были около baseline; это evidence against mandatory wrapper-tax на этом фрагменте, но не universal benchmark claim.

## 17.3 Physical operator coverage

**[VERIFIED]** Все текущие `RelExpr` имеют physical correctness path, включая:

- Scan;
- FilterEqConst;
- Project;
- Distinct;
- JoinEq;
- PromoteToBag;
- Group Count / ExactF64Sum;
- TopKWithTies.

Group/TopK пока correctness-first replay/sort, не maintained state.

## 17.4 Adaptive indexed I64 Join

**[VERIFIED/PARTIAL]** Direct columnar Scan×Scan join может использовать `IndexedI64IfAvailable`:

- runtime подтверждает `I64Exact` semantic equality;
- берёт raw I64 key slices;
- строит right-side `BTreeMap<i64, Vec<row_index>>`;
- сохраняет Bag multiplicity и reference nested-loop order;
- при неподходящем type/contract делает semantic fallback.

На 20k unique keys O(n²) baseline устранён; final measured overhead относительно hand-written BTreeMap порядка нескольких процентов/десятка процентов с заметным process noise.

**Pass16 correction:** right-side I64 index стал `MaterializedI64IndexState`, keyed by first-class `I64IndexBinding`, сохраняемым в `PhysicalStore`; relation+index delta update публикуется атомарно.

**Pass17 correction:** persisted index **больше не хранит logical `Vec<Value>` payload rows**. `PhysicalStore` владеет `InstalledRelation` с stable `PhysicalRowId`; index buckets хранят только row handles. Payload materialize из текущего typed relation только на semantic/output boundary. Удаление физической строки может сдвинуть dense position, но handle не меняется; relation поддерживает `RowId -> current position`, а hostile multi-delete test проверяет, что index не начинает ссылаться на соседний payload после compaction. Standalone `maintain_i64_index_delta()` удалён как архитектурно опасный split-brain API: единственный корректный maintained path — атомарный `apply_relation_delta`, который сначала строит physical row-handle edits, затем применяет их ко всем связанным indexes и публикует relation+indexes вместе.

**Performance status:** на 20k unique-key self-join gated run дал ~3.09 ms persisted против ~2.71 ms hand-written prebuilt BTreeMap (**1.141×**) и ~6.07 ms ephemeral rebuild (persisted ≈ **0.509×** времени ephemeral). Пять process-level runs были примерно 1.10–1.26× baseline. Значит defect `Vec<Value>` payload устранён; остаётся более узкий runtime gap от generic plan/materialization/row-position machinery и update-cost dense compaction.

## 17.5 Incremental state

**[VERIFIED/PARTIAL]** Есть `MaterializedSetSupportState` / `MaterializedRelDeltaState` для long-lived `Project(Set)` и `Distinct`; sequential transitions сравниваются с full recompute oracle.

Join теперь **частично подключён** к долгоживущему physical state: persisted right-side I64 index поддерживается `RelationDelta` атомарно с relation. Pass22 добавил `MaterializedGroupDeltaState`, Pass23 — `MaterializedTopKDeltaState`, Pass24–25 — compositional Join→Group→TopK ownership/build. Pass26 обобщил это до `MaterializedRelPlanState`: recursive ownership/build/maintenance покрывает все текущие `RelExpr` nodes. Pass27 закрыл leaf lookup duplication: authoritative PhysicalStore возвращает exact stable-handle receipt, а recursive Scan consuming certified delta больше не повторяет semantic membership scan. Следующая граница — joint prepare/commit across storage + maintained-plan state.

## 17.6 Изменение спецификации по результатам Pass17

**Логическая теория не изменилась.** `Revision=(S,Γ,M)`, query/change/lifecycle/identity/lens laws остаются прежними. Изменилась физическая спецификация:

1. persisted index — reconstructible revision-derived artifact с first-class binding, а не второй source of truth;
2. index payload в ideal representation — stable physical handle/typed slot, **не logical row copy**;
3. standalone index maintenance запрещён: relation delta и все зависимые index deltas образуют одну атомарную physical state transition;
4. row handle должен переживать physical compaction/position shift; positional index сам по себе недостаточен;
5. logical `Value` materialization допускается только на semantic/output boundary, а не как mandatory representation внутри index;
6. correctness maintenance продолжает выражаться тем же `RelationDelta`, что и IVM.

Это пример общего правила проекта: implementation falsifier может уточнить physical contract, не создавая новый logical primitive.

---


## 17.7 Pass18 correction — stable-slot updates and clone-free atomic commit

**[VERIFIED/PARTIAL]** Pass17 stable handles still used dense `Vec::remove` and cloned the complete relation plus every bound index before applying a delta. Pass18 separates stable logical row identity from dense physical position more sharply:

- physical deletion uses `swap_remove`, so only the removed handle and at most one moved row position are repaired;
- logical scan/insertion order is reconstructed through the stable handle→position layer, so physical compaction cannot leak into Bag/Set query output order;
- `apply_relation_delta` is now two-phase: first plan and validate the complete relation/index mutation, then mutate in place; invalid user deltas are rejected before the first mutation;
- compatible persisted I64 indexes are also used as **candidate locators for removal**, followed by full semantic-row equality, so an index accelerates update planning without becoming the source of truth.

A hostile atomicity test proves missing removals and wrong-typed inserts leave the complete `PhysicalStore` unchanged. A structural test proves deleting a dense slot repairs only the swapped row position.

The clone-tax falsifier changed sharply. Before two-phase commit, removing the first row scaled roughly 0.12 ms (10k) → 1.54 ms (100k) → 4.41 ms (300k) without an index, and up to ~37.5 ms at 300k with a persisted index because full state was cloned. After Pass18, first-row deletion is only a few microseconds and no longer scales with relation size. Removing the last row still costs O(n) without an index because semantic row resolution scans; with the persisted I64 index, the 300k case falls from ~17 ms to ~0.012 ms.

**Pass18 caveat retired by Pass19:** monotone handle-space growth is no longer part of the target implementation. Pass19 replaces monotone IDs with generational reusable slots while preserving stale-handle safety and logical scan order.

## 17.8 Pass19 correction — bounded generational row slots + first Join→Project fusion

**[VERIFIED/PARTIAL]** Physical row identity is now a generational slot handle `(slot,generation)` rather than a monotonically growing tombstone index. Deleted slots enter a free-list; reuse increments generation, so an old handle can never alias the new row occupying the same slot. Logical insertion order is maintained independently from dense physical `swap_remove` order through O(1) previous/next links.

A 10,000-cycle hostile churn test on one live row keeps `slots.len()==1` for the entire run. The original generation-0 handle remains invalid while the current generation reaches 10,000. Metadata therefore scales with peak live/allocated slot capacity rather than lifetime mutation count. Generation exhaustion is rejected during prevalidation before any physical mutation. Fresh persisted-index rebuilds also iterate logical scan order, not current dense physical order.

Planning no longer clones the complete free-list before small inserts: LIFO slot reuse is previewed directly from existing free slots plus slots removed by the same atomic transition.

The first downstream batch-composition slice is also verified: `Project(IndexedI64Join(Scan,Scan))` now probes the typed/indexed join and materializes only projected columns. It does not first create the complete joined `Vec<Row>`. This is not yet a general batch DAG, but it establishes the intended operator-boundary direction.

**Remaining physical caveat:** slot-table capacity is bounded by historical peak simultaneous allocation, not necessarily current live cardinality. Shrinking a relation far below its historical peak still leaves reusable free-slot metadata. Whether explicit epoch/chunk reclamation is worthwhile is now a memory-footprint policy question rather than an unbounded per-update tombstone leak.


## 17.9 Pass20 correction — compositional typed batch programs beyond one fused pattern

**[VERIFIED/PARTIAL]** The typed physical layer now has an explicit compositional batch-program path rather than only isolated pattern-specific fused operators. Arbitrary unary chains over `TypedColumnar` built from `Scan`, `FilterEqConst`, `Project` and `PromoteToBag` compile into a single `TypedBatchProgram`: projections become column remaps, predicates are bound once, and logical `Value` objects are created only at the final semantic boundary.

Direct indexed-I64 joins gained a matching downstream batch path. `Project/Filter/PromoteToBag` above `IndexedI64IfAvailable` can operate over raw left/right typed columns and join-position pairs; the verified `Join -> Filter -> Project` slice therefore avoids materializing the full joined `Vec<Row>` before applying downstream operators. The old `Join -> Project` specialization remains semantically compatible, but the normative target is now the compositional batch-program mechanism.

A performance falsifier initially measured the generic unary batch interpreter at about **5.9x** a hand-written I64 loop on a 100k-row two-filter/project workload, despite already being materially faster than the row-materialized runtime. Replacing intermediate selected-position vectors with a compiled single-scan program, hoisting physical type dispatch, and selecting a raw-I64 microkernel for common predicate counts reduced five final process-level ratios versus hand-written code to **1.503x, 1.641x, 1.576x, 1.359x and 1.393x** (median process ratio **1.503x**). Against the old row-materialized executor the same runs were **0.085-0.113x** its runtime (median **0.095x**, about **10.5x faster**). The residual gap and inter-process spread are explicit open performance debt, not a universal no-tax result.

**Remaining batch caveat:** the compositional batch DAG still covers only unary operators plus the current direct indexed-I64 join boundary. `Distinct`, `Group`, `TopK`, nested/multiway joins and mixed-type indexed joins still cross a materialized logical-row boundary. These are now the main batch-DAG frontier rather than basic `Filter/Project` composition.


## 17.12 Pass23 correction — TopKWithTies becomes maintained ordered state

**[VERIFIED/PARTIAL]** `TopKWithTies` больше не обязан replay/sort полного input при каждом child delta. `MaterializedTopKDeltaState` строится один раз, принимает готовый child `RelationDelta` напрямую и возвращает exact output `RelationDelta`.

- Для текущего exact-I64 ordering persistent state использует `BTreeMap<i64, tie-bucket>`. Изменение строки затрагивает только соответствующий key bucket; TopK output читает ordered buckets только до достижения `k`, причём threshold bucket включается целиком, поэтому exact ties сохраняются.
- Ascending/descending threshold jumps, group-like tie births/deaths и empty/non-empty transitions sequentially сравниваются с `rel_delta_by_recompute`.
- Generic ordering path хранит semantic-sorted rows и сохраняет declared Γ ordering/equality. Text ASCII-CI hostile test подтверждает, что host `String` equality/order не подменяют semantic contract. Generic fallback корректен, но insertion/removal пока O(n).
- Direct input delta проверяет pinned context, exact input `RelType` и row shapes до первой мутации. Missing removal rejected atomically.
- Для exact one-column I64 result semantic output-diff не вызывает registry equality в threshold hot path; logical `RelationDelta` формируется из raw key multiplicity differences.

Performance evidence: на 50k input rows, `k=10`, alternating threshold-changing one-row delete/insert, пять frozen-code process runs дают maintained ~2.37–2.55 µs. Tiny hand-written ordered-map baseline даёт ~0.11–0.12 µs, поэтому constant-factor gap остаётся большим (~20–23x). Однако full replay/remove+sort того же 50k input занимает ~0.31–0.36 ms; maintained path использует примерно 0.7% replay time (около 100–150x speedup). Это доказывает устранение full-input replay tax, но **не** закрывает microkernel no-tax target.

**Remaining TopK boundary:** generic Text/F64 orderings нуждаются в indexed/order-statistics representation; maintained state ещё не является typed-batch producer и не включён в единый owned state tree.

# 18. Матрица «идеал ↔ реализация»


## 17.10 Pass21 correction — stateful/set operators consume typed batch selections

**[VERIFIED/PARTIAL]** `Distinct`, `Group` and `TopKWithTies` no longer require their typed-columnar input subtree to be materialized as a complete logical `Vec<Row>` first. A shared `TypedBatchSelection` carries the compiled unary batch program plus selected physical positions into the stateful operator.

- `Distinct` consumes selected positions directly; exact single-column I64 equality has a raw `BTreeSet<i64>` path, while the semantic fallback materializes only candidate rows needed for equality checking. A verified Text ASCII-CI fallback test confirms non-I64 paths still obey pinned Γ equality (`"A" ≡ "a"`) rather than host equality.
- `Group Count` can group exact-I64 keys directly from native columns and uses `ExactCount`; `Group ExactF64Sum` reads only group-key and aggregate columns and retains the exact/reproducible accumulator.
- `TopKWithTies` sorts/selects physical positions and materializes only final output rows. Primitive I64/F64 ordering keeps semantic dispatch outside the hot comparator where possible.
- `typed_stateful_batch_hits` makes admission observable, and dedicated differential tests compare every path with the logical evaluator.

This closes the Pass20 caveat that these operators necessarily materialize their **input** rows. It does **not** yet make them arbitrary typed-batch producers: their result currently returns to the logical-row boundary. Therefore a chain such as `Group -> Filter -> Project` cannot yet stay entirely native/batched through the Group output. The normative target remains one compositional typed batch DAG where stateful nodes can emit typed batches or maintained artifacts for downstream operators.

**ExactCount implementation correction.** Ordinary counts now use a `u64` fast representation and promote losslessly to arbitrary precision only when `u64` overflows. A hostile boundary test proves `Small(u64::MAX) + 1` and merge-based overflow produce the same Big representation. This is an implementation refinement only; logical exact-count semantics are unchanged.

Performance evidence on the current 100k-row diagnostic is intentionally narrow. Five final process runs on the frozen refactored code gave Group Count ratios 1.028x, 1.075x, 1.104x, 1.093x and 1.214x versus the current hand-written BTreeMap baseline (median **1.093x**). TopK ratios were 0.453x, 0.480x, 0.460x, 0.633x and 0.540x (median **0.480x**), demonstrating the benefit of late materialization on this workload, not superiority to an optimal specialist TopK implementation. A 20-process frozen-code follow-up measured Group at 1.030-1.288x with median **1.128x**, and TopK at 0.426-0.727x with median **0.497x**. The Group constant-factor gap remains explicit performance debt.


## 17.11 Pass22 correction — Group becomes maintained delta state

**[VERIFIED/PARTIAL]** `Group` больше не только execution-time aggregate/replay. `MaterializedGroupDeltaState` строится один раз и затем принимает child `RelationDelta` напрямую. Это первый compositional state boundary для stateful relational operator: parent Group не обязан снова выводить изменение из полного `(old Model, Change<Model>)`.

- `Count` хранит exact count и поддерживает checked decrement; small `u64` representation losslessly promotes/demotes across the Big boundary.
- `ExactF64Sum` хранит exact reproducible accumulator и поддерживает deletion как exact inverse update.
- birth/death group semantics, global empty-group identity row и ASCII-CI semantic keys сравниваются sequentially с full recompute oracle.
- update planning валидирует весь input delta до первой мутации и планирует только affected buckets; full clone всех groups отсутствует.
- exact-I64 single-key Group получает persistent `BTreeMap<i64,bucket-index>` и compiled Count maintenance kernel. `swap_remove` bucket deletion re-resolves key immediately before commit, поэтому multi-group death не оставляет stale indices.

Performance evidence remains deliberately diagnostic. На 50k simultaneously maintained I64 groups frozen-code process runs дают примерно 0.685–0.767 µs на one-row maintained transition. Fair hand-written `BTreeMap` baseline, который также строит logical `RelationDelta`, даёт ~0.153–0.160 µs; median process ratio около **4.56x**. Это означает: asymptotic replay/lookup tax закрыт, но constant-factor semantic/validation/output packaging overhead maintained Group ещё не закрыт.

**Remaining maintained-state boundary:** Group и TopK пока отдельные state objects, а не узлы единого owned state tree. Их downstream output всё ещё materialized logical rows. Exact-I64 TopK уже maintained/order-statistics-like; generic Text/F64 ordering fallback корректен, но пока не performance-grade.

| Компонент идеальной БД | Статус после Pass22 | Что ещё нужно |
|---|---|---|
| `Revision=(S,Γ,M)` | **VERIFIED core shape** | durable persistent engine |
| Algebraic structural types | **VERIFIED broad** | fuller surface elaboration |
| Nominal identity / refs | **VERIFIED** | retention-tier identity erasure |
| Explicit Set/Bag/Seq/Map semantics | **VERIFIED core** | broader physical kernels |
| Guarded structural μ | **VERIFIED** | efficient nested/fold lowering |
| Versioned equality/order Γ | **VERIFIED** | more module kinds + proofs |
| Exact closed query IR | **VERIFIED** | richer algebra/operators |
| Universal exact Change/Dq fallback | **VERIFIED for current kernel / theorem accepted** | mechanized general proof |
| Efficient fine IVM | **PARTIAL/VERIFIED Group+TopK** | maintained Group Count/ExactF64Sum + persisted Join + maintained TopK verified; general state tree open |
| Lifecycle LFP normalization | **VERIFIED** | formal proof + durable integration |
| Schema lenses/complements | **PARTIAL** | full writable lens engine |
| Identity groupoid | **VERIFIED fragment** | full schema/Γ merge cube |
| Observation transaction repair | **PARTIAL/theory** | complete runtime scheduler |
| Influence provenance+sensitivity | **PARTIAL** | full operator transfer calculus |
| D0–D4 deletion ladder | **NORM / mostly OPEN** | physical erase + privacy machinery |
| Checked certificates | **VERIFIED architecture** | proof-assistant mechanization |
| Small PlanIR | **VERIFIED prototype** | richer batch/fixpoint/source contracts |
| Native typed columnar | **VERIFIED/PARTIAL batch DAG** | unary chains + downstream indexed-I64 Join + stateful-input batching for Distinct/Group/TopK verified; make stateful outputs composable typed batches and extend nested/mixed joins |
| Indexed Join | **PARTIAL/VERIFIED architecture** | persisted atomic delta maintenance + generational handles + Join→Filter→Project batch path verified; add other key types/costing, nested/multiway batch joins and close noisy prebuilt gap |
| Physical Group/TopK | **VERIFIED/PARTIAL** | Group Count/ExactF64Sum and exact-I64 TopKWithTies now maintained; generic TopK remains fallback; both still need composable batch outputs |
| OrderedView/pagination | **OPEN** | explicit semantic/runtime type |
| WAL/recovery/compaction | **PARTIAL/VERIFIED WAL tail + replay** | durable checkpoints/segments/real crash + compaction |
| Distributed revisions | **NORM / OPEN runtime** | replication/consensus/merge engine |

---

# 19. Плашка: архитектурные преимущества перед обычными классами БД

> ## Почему эта архитектура вообще нужна
> **Это не claim «CFMD уже быстрее PostgreSQL/MongoDB/TypeDB/graph DB».** Преимущество — в том, какие компромиссы *не зашиты* в логическую модель и потому могут быть выбраны компилятором/physical layer под workload.
>
> **Перед классическим relational/SQL:** algebraic nested values, nominal identity, explicit Set/Bag/Seq, lifecycle и schema lenses являются первичными typed concepts, а не ORM/DDL conventions; при этом relational algebra/query transparency не теряются.
>
> **Перед document DB:** nesting не диктует ownership или locality. Shared identity, n-ary relations и independently indexed entities не требуют denormalize/reference компромисса как части data model.
>
> **Перед graph DB:** graph traversal — одна projection/physical representation Relation, а не онтология всей базы. Те же данные могут быть row/column/CSR без logical duplication.
>
> **Перед object DB:** developer-facing entities/interfaces сохраняются, но arbitrary methods/virtual dispatch не проникают в declarative query semantics; optimizer остаётся способен видеть операции.
>
> **Перед event sourcing:** история и current state эквивалентны по semantics; приложение не обязано проектировать доменные event classes для каждой мутации и replay-from-genesis не является логическим требованием.
>
> **Перед «multi-model» DB:** нет нескольких loosely connected engines/models. Object/document/graph/relational формы должны elaboratе в один kernel и иметь одну correctness/transaction/history semantics.
>
> **Перед обычной schema migration:** old information не silently reinterpret/delete; lenses/transports дают формальный ответ, что можно переиспользовать, а где нужен explicit conflict/forget.
>
> **Перед традиционным IVM/cache subsystem:** change semantics является общим основанием для query repair, materialization, transaction repair, sensitivity и maintained indexes, а не набором отдельных invalidation механизмов.
>
> **Перед opaque optimizer stack:** fast plan/solver может быть untrusted; correctness переносится на маленький checker/certificate boundary.
>
> **Перед «универсальной БД с неизбежным abstraction tax»:** physical lowerability специально требует, чтобы высокоуровневые constructs могли стираться до тех же typed columns/maps/CSR/index structures, которые использует specialist. Pass14–15 уже дали первый executable falsification этого принципа на columnar filter/project и indexed I64 join.

---

# 20. Что нельзя обещать

**[NORM]** Проект сознательно не обещает:

- быть быстрее specialist на любом workload;
- автоматически понять business meaning произвольной lossy schema migration;
- сделать все concurrent writes coordination-free;
- превратить arbitrary operation в CRDT;
- точно удалить информацию из uncontrolled external replicas;
- автоматически гарантировать D4 inference privacy для arbitrary correlated retained data;
- считать heuristic ANN exact;
- получить эффективный fine derivative каждого query без special algorithm;
- скрыть hardware/space/write/read tradeoffs.

Именно эти ограничения делают итоговую спецификацию проверяемой, а не «dream DB» без falsifiable claims.

---

# 21. Текущий главный implementation frontier после Pass75

Приоритеты, если новый hostile falsifier не меняет архитектуру:

1. **Multi-family physical lifecycle/advisor.** Pass63 unified inventory/global retained-byte discipline, Pass64 added Γ-QCN endpoint-factor lifecycle, Pass66 added conservative persisted exact-I64 Join-index lifecycle, Pass67 added conservative semantic-statistics lifecycle for direct primitive Join cases, and Pass75 gives advisor-owned durable reconstruction the same explicit ownership/budget boundary across reopen. Γ-QCN support, algebraic/future layouts, multiway counterfactual path-shaping, write-maintenance costing and one benefit-ranked cross-family scheduler remain fragmented. The target is still one reconstructible cross-family benefit/cost contract rather than isolated heuristics.
2. **Exact physical memory accounting.** Pass63 deterministic retained-byte/shared-backing estimates now constrain both semantic-index and Pass64 Γ-QCN-factor advice, but they are planning estimates rather than allocator/RSS truth. External pressure, allocator overhead and rebuild scheduling remain OPEN.
3. **General nested/multiway/bushy Join planning.** Γ-QCN has prepared quotient metadata, maintained factors/support, local delete/insert/mixed Dq and revision coalescing. Search/execution remains bounded to the verified 3–8-leaf family; adaptive/unbounded search, richer predicate graphs and fully general indexed/typed subset nodes remain OPEN.
4. **Structural/custom semantic persistence.** Pass69 closes the canonical-key encoding/version migration law and Pass70 compiles structural Γ execution for quotient factors, algebraic filters and structural semantic indexes. The remaining structural gap is durable physical structural-index payload persistence/rebuild, arbitrary/plugin semantic executable packaging/deployment and structural ordering.
5. **Statistics / workload telemetry.** Exact retained key cardinality now has a conservative direct-Join create/retain/evict law, but multiway counterfactual scoring, histograms, correlation, online observations, decay/hysteresis and autonomous scheduling remain OPEN.
6. **Other physical layouts + OrderedView/pagination.** Pass62 closes the recursive algebraic native-family gap, but KeyValue, AdjacencyList/CSR, DenseArray, Inverted, Custom/chunked structural layouts and explicit pinned-revision ordered cursors remain OPEN.
7. **Recovery rebuild economics.** Pass75 bounds synchronous advisor-owned artifact reconstruction by deterministic key-evaluation and estimated-byte admission while preserving manual durable pins and reporting skipped/stale derivatives. Remaining work is benefit-ranked/background scheduling, faster reconstruction paths, structural-key-size-aware costing, pressure-aware admission and arbitrary future layout/index families.
8. **Residual I64 performance.** Maintained I64 Group and TopK constant-factor gaps were historically kept outside the numbered 22-item ledger despite remaining explicitly OPEN. Pass62 restores both to the authoritative active ledger; they are performance debt, not correctness gaps.
9. **Durable revision DAG.** Current durable control plane remains a linear committed-head protocol; branch/merge parents, ancestry and merge replay remain OPEN.
10. **Historical durable-format migration.** Selected payload compatibility exists, not a general checkpoint/manifest/metadata migration framework.
11. **Semantic artifact deployment.** Builtin implementation descriptors are durable; arbitrary/plugin executable packaging, signing, authentication and deployment remain OPEN.
12. **Durability scale/assurance.** Transaction outcome retention+GC, streaming/chunked checkpoints/metadata, real power-loss testing, Windows/network-FS/FUSE semantics, general lock-poison policy and authenticated durable storage remain OPEN.
13. **Throughput/distribution.** Group commit, async durability, replication, consensus and broader distribution architecture remain OPEN.
14. **Transaction repair and formal closure.** Observation-based repair runtime, surface elaboration theorem, Dq theorem, merge cube, retention IFC, formal power-loss proof and proof-checker mechanization remain OPEN.

### Active historical ledger after Pass70

The late Pass43–61 reports numbered **22 architectural OPEN**. They also continued to state that maintained I64 Group and TopK constant-factor gaps were OPEN, but those two performance items were not included in the numbered count. Pass62 corrected the accounting to **24 historical active OPEN** without treating the correction as new project debt. Pass65 temporarily added one genuinely new logical-snapshot clone OPEN (25 total); Pass66 closed that new item. Pass67 closed historical item #9 (persistent outer physical artifact catalogs), reducing the historical ledger to 23. Pass69 closes historical canonical-key/cache encoding-version migration and compatibility, reducing it again to **22 active OPEN**. Pass70 integrates compiled Γ and hardens exact structural binding but closes no additional whole historical item.

Historical performance/durability debt remains tracked even when the active pass works elsewhere. A feasibility prototype, local fast path or one layout family never silently closes production lifecycle, persistence, assurance or formal obligations.

# 22. Неприкосновенные правила для следующих implementation passes

Следующий агент не должен «оптимизировать» систему ценой нарушения этих правил:

1. Physical representation **никогда** не определяет logical equality/order.
2. Exact query нельзя тихо заменить approximate/heuristic implementation.
3. Fast path обязан иметь reference semantics/falsifier или checked refinement boundary.
4. Set/Bag не получают порядок из physical iteration.
5. Entity ID не становится обычным integer key без nominal type check.
6. Lifecycle ownership не выводится автоматически из reference/foreign-key shape.
7. Schema/Γ change нельзя пересечь без transport proof/check.
8. Fine delta — optimization exact `Replace/recompute` semantics, а не отдельный смысл.
9. Cached/materialized/indexed state — reconstructible certificate/artifact, не независимый source of truth.
10. Если specialized path неприменим, допустим semantic fallback; недопустима silently изменённая semantics.
11. Benchmark regression нельзя скрывать. Сначала локализовать mandatory tax; если он остаётся — записать как open gap.
12. Новый convenient surface feature допустим только если elaborates в существующий kernel или явно требует архитектурного изменения.

---

# 23. Правило обновления этого документа

Обновлять `CFMD_IDEAL_DB_SPEC.md` не на каждый cosmetic pass, а когда происходит одно из событий:

1. новый hostile counterexample меняет нормативную теорию;
2. теоретический OPEN закрывается executable design;
3. временный implementation choice становится признанной архитектурой;
4. benchmark обнаруживает/устраняет обязательный abstraction tax;
5. появляется новый major engine subsystem (durability, concurrency, distribution, privacy);
6. текущий код расходится с идеальной specification и нужно явно выбрать, что меняется — код или теория.

Каждая версия должна сохранять секции:

```text
NORMATIVE IDEAL
CURRENT VERIFIED REALITY
DEVIATIONS / TEMPORARY CHOICES
OPEN FRONTIER
ADVANTAGES / NON-CLAIMS
```

---

# 24. Source provenance этого snapshot

Основные поздние нормативные основания взяты из исследовательского журнала:

- §48.2–48.9 — lifecycle, schema/data transport, erasure summaries, certified solvers, physical lowerability;
- §49.1–49.9 — provenance vs sensitivity, atomic lifecycle normalization, distribution contracts, surface elaboration, explicit partiality/order;
- §50.1–50.7 — lifecycle normal form, deletion-sensitivity, proof-carrying optimizer, merge coherence, identity/deletion corrections;
- §51.17 — consolidated kernel `Revision=(S,Γ,M)`;
- §52.1–52.12 — universal change existence, Impact, approximation boundary, PlanIR direction;
- ранние §17–18 использованы только там, где идеи сохранились после поздних falsifier’ов (immutable schema versions, lenses, typed change, identity separation, physical independence).

Implementation corrections/status наложены из verified Pass09–Pass70; текущий executable checkpoint — **Pass70 / 432 declared Rust tests**. Pass28 первоначально был source-audited без toolchain, затем открыт заново под Rust 1.98.1, исправлен и прошёл полный gate до форка Pass29. Примечание: preserved Pass22 source audit показывает 216 declared tests; Pass22 report недосчитал 2.


---

## Pass24 implementation correction — maintained ownership tree

Status update from implementation/falsification:

- **VERIFIED/PARTIAL:** Join now has a long-lived maintained state with direct left/right `RelationDelta` input. Exact-I64 equality uses persistent per-key buckets; generic semantics retain a correctness-first fallback.
- **VERIFIED/PARTIAL:** an owned `Join → Group Count → TopKWithTies` state tree exists for the exact-I64 fast fragment. Deltas propagate between child states directly; no model replay or full-tree staging clone is required on the update path.
- **UNCHANGED IDEAL:** the normative architecture remains a compositional maintained-plan tree where every materialized operator owns its state and consumes/produces typed deltas.
- **NEW IMPLEMENTATION GAP:** initial state construction is not yet compositional. Parent `build` paths still re-evaluate nested logical expressions rather than consuming already materialized child snapshots; a large Join subtree can therefore re-enter O(n²) generic logical evaluation during construction.
- **OPEN GENERALIZATION:** the owned tree is currently one verified exact-I64 `Join→Group Count→TopK` fragment, not yet a recursive arbitrary operator-state ownership framework.
- **SEMANTIC NON-BUG:** exact `WITH TIES` may produce O(n) output when the threshold tie class itself has O(n) members. Physical design must not silently cap or drop ties to manufacture bounded runtime.

Practical next target: introduce child-snapshot build interfaces / maintained-node construction so initial materialization follows the same physical/indexed structure as subsequent delta maintenance, then generalize the owner tree beyond the fixed exact-I64 fragment.


## Pass25 implementation correction — compositional construction + local generic Join maintenance

Status after Pass25:

- **VERIFIED:** exact-I64 owned `Join → Group Count → TopKWithTies` initial construction no longer re-evaluates nested child queries. Join is materialized once; Group is built from the Join snapshot; TopK is built from the Group snapshot. On the unique-key diagnostic, 50k-row construction completes in ~75 ms instead of failing to complete in the Pass24 diagnostic window.
- **VERIFIED:** snapshot-built child states are regression-checked against ordinary standalone state construction.
- **VERIFIED/PARTIAL:** generic maintained Join no longer clones both full input relations and recomputes full before/after joins for every small delta. It prevalidates both leaf deltas, derives output as `ΔL × oldR + nextL × ΔR`, then commits inputs atomically. This removes full replay/full-state clone from the generic update path. Without a semantic index, generic Join remains `O(|Δ|·n)` and therefore is not yet performance-grade at high cardinality.
- **OPEN:** general recursive maintained-plan ownership beyond the fixed exact-I64 `Join→Group Count→TopK` fragment.
- **OPEN:** semantic indexes/canonical keying for generic Text/F64/etc. joins and groups.

Practical next target: generalize snapshot construction/ownership into a recursive maintained-plan node API rather than fixed fragment-specific constructors, while preserving exact Γ semantics and direct child-delta propagation.


## Pass26 implementation correction — general recursive maintained-plan ownership

Status after Pass26:

- **VERIFIED:** `MaterializedRelPlanState` is a recursive maintained-plan owner for every current `RelExpr` variant: `Scan`, `FilterEqConst`, `Project` (Bag/Set), `JoinEq`, `Distinct`, `Group`, `TopKWithTies`, and `PromoteToBag`.
- **VERIFIED:** construction is bottom-up from child snapshots. Join gained a snapshot constructor, matching the already-compositional Group/TopK construction. A parent does not re-evaluate its logical child expression during state construction.
- **VERIFIED:** maintenance accepts a map of exact base-relation `RelationDelta`s, prevalidates all referenced leaves before mutation, then propagates child deltas recursively through stateless unary operators and maintained Join/Group/TopK state. Two non-isomorphic hostile trees match the full recompute oracle: `Filter→Join(Project)→Project→Group→TopK` and `Filter→Project→Distinct→PromoteToBag`.
- **VERIFIED:** invalid leaf removal is rejected before any tree mutation. The recursive plan also rejects deltas for relations not present in the tree.
- **PERFORMANCE FALSIFIER / NEW OPEN:** the recursive tree currently keeps a logical `Scan` snapshot for output/consistency checking. On removal, leaf validation/commit performs semantic row lookup. In the 1k/10k/50k diagnostic this makes recursive update ~47 µs / 0.42 ms / 2.32 ms, while the older fixed exact-I64 stateful pipeline remains ~5–10 µs. This is not Join/Group/TopK replay; it is duplicate base-table responsibility at the Scan leaf.
- **NORMATIVE CONSEQUENCE:** derived maintained-plan state should not become a second authoritative table store. The physical/storage layer should validate the base mutation once and hand the plan a stable row-handle or already-certified exact `RelationDelta`. Scan nodes may retain reconstructible snapshot/cache state, but they must not impose a mandatory O(n) semantic membership re-check on every validated removal.

Practical next target: define the storage→maintained-plan leaf contract (stable row handle / certified relation delta), eliminate the duplicate Scan membership lookup, then resume the still-open generic semantic indexes, typed-batch stateful outputs, and durability work.


## Pass27 implementation correction — storage-certified Scan leaves

Status after Pass27:

- **VERIFIED:** generational row identity is now a shared kernel type (`StableRowHandle`), with `kernel-plan::PhysicalRowId` re-exporting that type rather than defining an isolated identity shape.
- **VERIFIED:** `PhysicalStore::apply_relation_delta_certified` reuses the already-validated physical transition and returns a `StorageCertifiedRelationDelta` carrying the exact removed/inserted stable handles selected by authoritative storage. Existing `apply_relation_delta` remains compatible and simply discards the receipt.
- **VERIFIED:** recursive Scan leaves can attach initial logical-order handles and consume storage-certified deltas by handle→position lookup. The old semantic membership path remains as a fallback when no certificate is available.
- **VERIFIED:** certified leaf deltas propagate through the same general recursive maintained-plan tree. The non-trivial `Filter→Join(Project)→Project→Group Count→TopKWithTies` falsifier matches full recompute under generation-changing slot reuse. Stale receipt replay is rejected before plan mutation.
- **PERFORMANCE:** median 50k removal moved from ~2.142 ms on the old semantic Scan path to ~7.12 µs for certified Scan maintenance, or ~15.35 µs including authoritative PhysicalStore mutation with persisted I64 lookup. This closes the Pass26 O(n) duplicate leaf-membership tax.
- **NEW OPEN:** storage and maintained-plan publication are not yet one cross-layer atomic transaction. PhysicalStore currently commits before the receipt is consumed by the plan. The next design should prepare both physical and maintained transitions before one commit boundary.
- **TRUST NOTE:** `StorageCertifiedRelationDelta` is currently a typed contract between trusted kernel crates, not a capability-sealed security token against hostile in-process callers.

Practical next target: introduce a joint prepared storage+plan transition so physical relation/index state and recursive maintained state publish atomically; then resume generic semantic indexes, typed-batch stateful outputs, and durability.

## Pass28 implementation correction — prepared storage + maintained-plan transition

Final status after reopening under Rust 1.98.1: **VERIFIED after correction**.

- **VERIFIED:** authoritative `PhysicalStore` mutation and recursive maintained-plan mutation can be prepared on candidates without changing live state, with explicit source/target revision binding and exact stale-source rejection.
- **VERIFIED:** bound legacy semantic mutation entrypoints are blocked; reconstructible physical-index mutation invalidates prepared work through freshness.
- **CORRECTION FROM ORIGINAL SOURCE-AUDIT:** real toolchain verification exposed formatting/lint issues, one bag-order test assumption, and stale-error boundary normalization. They were fixed before Pass29 and the complete debug/release/clippy/rustdoc/overflow gate passed.
- **LIMITATION LEFT TO PASS29:** public shape still handled one relation and two live owners; final freshness was not yet a capability boundary suitable for WAL-after-validation/before-publication.

Practical next target at the end of corrected Pass28 was multi-relation one-root runtime publication with a sealed pre-durable boundary.


## Pass29 implementation correction — sealed multi-relation runtime revision publication

Status after Pass29: **VERIFIED**.

- **VERIFIED:** `RuntimeRevisionBundle` owns `RevisionId + PhysicalStore + MaterializedRelPlanState` as one runtime publication root.
- **VERIFIED:** one `RevisionTransitionRequest` can carry several normalized base-relation mutations; failure of a later relation never exposes earlier candidate mutations.
- **VERIFIED:** preparation is invisible; `seal()` performs the last exact freshness check and retains exclusive live access; `publish()` is infallible and replaces the whole runtime bundle. This creates the required future seam `prepare -> seal -> WAL COMMIT/fsync -> publish`.
- **VERIFIED:** competing prepares, stale same-revision different-state substitution, reconstructible index mutation after prepare, sequential handle-generation behavior, duplicate relation mutations, two-sided Join batch publication and abort-by-drop are hostile-tested.
- **OPEN:** runtime owner still contains only logical `RevisionId`, not authoritative `kernel_revision::Revision=(S,Γ,M)`; Schema/Γ-changing commits are therefore not yet one complete semantic revision transaction.
- **OPEN:** multiple maintained materializations need a registry owner; storage certificate minting and cross-crate prepared capability sealing remain incomplete.
- **OPEN:** correctness-first full-state clone/source snapshot identity is too expensive for production and should become immutable/COW/version-root identity after authority semantics are fixed.
- **OPEN:** WAL/recovery is not integrated despite the verified Agent-1 protocol research; Agent-2 semantic indexes remain research evidence pending mainline transaction/durability completion.

Practical next target: make the publication root authoritative for the actual validated `Revision=(S,Γ,M)` and every maintained consumer, then hand only a complete sealed revision commit to the durability layer.

## Pass30 implementation correction — authoritative revision + materialization-registry publication

Status after Pass30: **VERIFIED**.

- **VERIFIED:** `RuntimeRevisionBundle` now owns the actual validated `kernel_revision::Revision=(S,Γ,M)`, not only a numeric `RevisionId`.
- **VERIFIED:** bootstrap no longer trusts caller-supplied maintained snapshots. `RuntimeRevisionBundle::build` validates the selected physical relation snapshot against the logical revision, binds storage to that revision, constructs every registered `MaterializedRelPlanState` from the authoritative model, attaches storage handles, and binds all materializations to the same revision.
- **VERIFIED:** one runtime root now owns a `MaterializationId -> MaterializedRelPlanState` registry. Every materialization, including one unaffected by a particular data delta, advances atomically to the target revision under one candidate publication.
- **VERIFIED:** transition input names a complete prevalidated target `Revision`; Pass30 recomputes the expected target logical state by applying the declared relation deltas to the source revision and rejects any target-state mismatch.
- **VERIFIED:** physical layout choice is no longer supplied per mutation by the caller; the authoritative bundle selects its pinned relation-layout binding.
- **VERIFIED:** `RevisionCommitDescriptor` crosses the prepared/sealed boundary with source/target revision ids, pinned semantic revision, and exact logical relation deltas only. Physical handles/layout mutations remain reconstructible evidence, not durable authority.
- **VERIFIED:** hostile tests reject logical/physical bootstrap divergence, duplicate materialization ids, target-revision/descriptor disagreement, and verify all registered materializations publish one revision atomically.
- **EXPLICIT LIMIT:** data-plane Pass30 accepts only unchanged `SemanticContext`; Schema/Γ-changing revisions return `SemanticContextTransitionRequiresRebuild` and remain an OPEN migration/rebuild transaction problem.
- **OPEN:** correctness-first full source/candidate cloning remains; reader root/COW/MVCC publication and capability sealing remain before production durability.
- **OPEN:** WAL/recovery is still not integrated; Agent-1 protocol remains research evidence ready for integration after the remaining authority/sealing cleanup. Agent-2 indexes stay deferred.

Practical next target: seal the remaining certificate/prepared capabilities and replace full-snapshot freshness with an immutable/versioned publication root. Then wire Agent-1 WAL strictly into `prepare -> seal -> durable COMMIT -> infallible publish`.

## Pass31 implementation correction — versioned immutable runtime root + capability sealing

Status after Pass31: **VERIFIED**.

- **VERIFIED:** reader-visible runtime publication теперь имеет конкретный owner `RuntimeRevisionCell = RwLock<Arc<RuntimeRevisionBundle>>`. Reader snapshot клонирует `Arc`; уже начавшийся reader остаётся на полном старом root, а следующий reader после publish получает полный новый root.
- **VERIFIED:** runtime freshness больше не удерживает полный source bundle. Каждый built runtime root получает process-local nominal lineage и `RuntimeRootVersion`; prepared transition сохраняет только source identity и detached candidate. Independently built identical roots не являются взаимозаменяемыми.
- **VERIFIED:** `seal()` acquires sole writer guard и выполняет последнюю fallible freshness check. Пока sealed capability существует, live root не может advance. `publish()` после seal infallible и заменяет один whole `Arc` root. Это конкретизирует seam `prepare -> seal -> durable COMMIT/fsync -> publish`.
- **VERIFIED:** reconstructible I64-index installation тоже публикуется whole-root swap и увеличивает root version при неизменном semantic Revision; ранее prepared semantic transition после такого physical change становится stale.
- **VERIFIED:** query-layer prepared publication capability sealed: `PreparedMaterializedRelPlanTransition` private; runtime получает detached maintained candidate через `candidate_from_storage_resolved_deltas_for_revision`.
- **VERIFIED/NORMATIVE CORRECTION:** `StorageCertifiedRelationDelta` удалён как misleading authority concept. Новый `StorageResolvedRelationDelta` является constructible, validated row-identity evidence; он не может публиковать revision-bound runtime state. Авторитет принадлежит `Revision` + `RuntimeRevisionCell`, а не receipt possession.
- **DURABILITY RULE:** process-local runtime `root_id/version` — только freshness coordinates. Они **не** являются durable identity и не должны сериализоваться в WAL как semantic authority. Recovery создаёт новый runtime lineage из recovered logical revision/state.
- **OPEN:** candidate construction still deep-clones substantial state; перейти к COW/persistent roots после durability semantics.
- **OPEN:** production WAL/recovery/checkpoint/segment integration, stable `RevisionCommitDescriptor` codec/checksum и real crash falsification.
- **OPEN:** Schema/Γ-changing revision migration remains typed rebuild/migration problem.
- **OPEN PERFORMANCE:** `RwLock<Arc<_>>` closes correctness, not throughput optimality; lock-free/epoch publication requires benchmark justification later.

Practical next target: integrate Agent-1 logical WAL at the now-sealed boundary, preserving the law that all fallible runtime freshness work happens before durable COMMIT and runtime root publication after durable COMMIT is infallible.



## Pass32 implementation correction — logical WAL tail + exact-base recovery

Status after Pass32: **VERIFIED**.

- **VERIFIED:** new std-only `kernel-durability` implements versioned logical relation-data mutation encoding, CRC-32C framed PREPARE/COMMIT records, monotone LSN checking, exact duplicate/conflict rules, safe final-tail classification, and synchronous durability barriers.
- **VERIFIED:** `RuntimeRevisionCell::commit_revision_durable` executes `runtime prepare -> durable PREPARE -> seal -> durable COMMIT -> infallible whole-root publish`. Stale-after-PREPARE is safe because recovery ignores uncommitted PREPARE. No fallible semantic/freshness step remains after durable COMMIT.
- **VERIFIED:** a COMMIT durability I/O failure is reported as `CommitDurabilityUncertain`; live runtime is not published and caller must reopen/scan WAL rather than infer abort.
- **VERIFIED:** logical replay starts from one exact trusted base `Revision`, validates the pinned semantic revision, rebuilds every target through `Revision::build`, then reconstructs physical RowStore state and maintained materializations. Stable row handles, indexes, layouts and process-local root ids are not durable authority.
- **VERIFIED:** `FileRevisionWal` takes an exclusive filesystem lock and `create()` is create-new only, preventing accidental existing-log truncation. Safe torn tails may be truncated only after scanner validation.
- **HOSTILE FIX:** initial recovery incorrectly bound `NativeRelation::RowStore` to logical pseudo-layout `LogicalModelRows`; Pass32 introduced explicit reconstructible `RECOVERY_ROW_STORE`.
- **OPEN:** exact durable base checkpoint storage/installation, parent-directory fsync/rename and segment manifest/rotation/truncation.
- **OPEN:** real filesystem/power-loss tests, revision-DAG durable parent encoding, client idempotency/ACK recovery, and typed durable schema/Γ/lifecycle migrations.
- **OPEN PERFORMANCE:** candidate runtime state remains clone-heavy and recovered physical state is correctness-first generic RowStore until reconstructible indexes/layouts are rebuilt.

Practical next target: complete durable checkpoint + segment lifecycle and package reopen/scan/rebuild as one restart owner; then run real crash/fault falsification before widening the durable mutation class or integrating Agent-2 semantic indexes.

## Pass33 implementation correction — exact leaf binding/order + uncertain-COMMIT fail-stop

Status after Pass33: **VERIFIED**.

- **HOSTILE CORRECTION:** delayed independent review falsified the assumption that semantic Bag equality was sufficient to attach a positional storage-handle vector to a maintained Scan. Runtime bootstrap now consumes exact ordered `(StableRowHandle, Row)` bindings and rejects selected physical layouts whose concrete row payload/order differs from the authoritative logical revision.
- **VERIFIED:** maintained Scan dense storage and semantic logical order are separate. Dense payload/handle arrays may `swap_remove`, while stable-handle prev/next links preserve insertion/output order without O(n) positional repair on delete.
- **VERIFIED:** resolved removal validation now proves `handle -> exact current row payload`, not only handle membership. Forged payload/handle combinations fail before child or parent mutation.
- **VERIFIED:** storage-resolved deltas propagate the concrete rows actually selected by physical resolution, so semantic-equivalent request representatives cannot silently become downstream removal payloads.
- **DURABILITY CORRECTION:** `CommitDurabilityUncertain` is now a fail-stop state. `RuntimeRevisionCell` transitions from `Serving(root)` to `RecoveryRequired` under the sealed writer guard when durable COMMIT returns an uncertain error. The process may not continue serving the old root as if abort were known.
- **VERIFIED:** hostile regressions cover reordered Bag bootstrap, exact Scan order after deletion, forged handle/payload removal, and unreadable runtime after uncertain COMMIT.
- **OPEN:** automatic restart/recovery owner, exact durable base checkpoint, manifest/segment lifecycle and real crash testing remain next durability work.
- **OPEN PERFORMANCE:** exact selected-layout bootstrap and linked logical-order traversal are correctness baselines; certified fast bootstrap and compact/locality-optimized order nodes remain future optimizations.

Practical next target: durable exact checkpoint + atomic manifest/segment installation + restart/recovery owner, followed by real crash/fault falsification.

## Pass34 implementation correction — exact durable checkpoints + immutable generations + restart owner

Status after Pass34: **VERIFIED**.

- **VERIFIED:** `kernel-durability` now persists an exact versioned checkpoint of the complete validated `Revision=(S,Γ,M)`, including Schema, pinned SemanticEnvironment digests and normalized DatabaseState. Decode reconstructs domain values and must pass through ordinary `Revision::build` validation against the supplied semantic registry.
- **VERIFIED:** durable state is organized as immutable generations `checkpoint-N + wal-N + manifest-N`. Only a final published manifest is authority; pending manifests and orphan checkpoint/WAL files are ignored during reopen.
- **VERIFIED:** checkpoint publication orders durability as checkpoint sync → WAL sync → directory fsync for prerequisite names → pending-manifest sync → atomic rename to immutable final manifest → publication directory fsync. A manifest-referenced missing WAL or corrupt checkpoint is surfaced as corruption, never silently treated as an empty/fallback generation.
- **VERIFIED:** checkpoint rotation starts a fresh WAL at the exact current durable head instead of truncating/reusing the active segment. Obsolete generations can be compacted only after a newer manifest is authoritative, followed by directory fsync.
- **VERIFIED:** new `DurableRuntime` binds one `RuntimeRevisionCell` to exactly one `DurableRevisionStore`. Mainline durable commit, checkpoint, compaction and restart share the same durable-head contract; lower-level arbitrary runtime/backend pairing is no longer the normal public path.
- **VERIFIED:** `DurableRuntime::open` selects the highest published manifest, decodes and validates the checkpoint, scans/replays the WAL tail, rebuilds PhysicalStore and maintained materializations, and creates a fresh process-local runtime lineage whose semantic revision equals the recovered durable head.
- **OPEN / NEXT FALSIFIER:** the checkpoint/manifest ordering is now an explicit software contract but has not yet been validated by real process kill/power-loss/filesystem fault injection across target filesystems. This is the next durability blocker.
- **OPEN:** materialization configuration/specs are still supplied at reopen; durable operational configuration authority, revision-DAG parent encoding, client idempotency/ACK recovery, schema/Γ/lifecycle transition records, checkpoint streaming/migration and production COW candidates remain future work.

Practical next target: build a real killpoint/process-restart crash matrix around WAL commit, checkpoint generation publication, manifest rename/directory fsync and compaction; add a restart supervisor that returns to serving only after validated reopen. Only after that widen durable mutation classes or integrate Agent-2 semantic indexes.


## Pass35 implementation correction — real subprocess-kill durability falsification

Status after Pass35: **VERIFIED** on the current Linux/container filesystem.

- **VERIFIED PROCESS-CRASH EVIDENCE:** durability boundaries are now exercised by a real parent/child process harness. A child signals that it reached a private named durability point, blocks, and is terminated externally through `Child::kill()`; it does not run normal shutdown/destructors after the selected point.
- **VERIFIED:** kill after durable PREPARE reopens at the previous committed revision. An uncommitted PREPARE remains non-authoritative.
- **VERIFIED:** kill after durable COMMIT but before ACK reopens at the target revision. COMMIT is therefore recovery authority even when the dead process never acknowledged the caller.
- **VERIFIED:** checkpoint/WAL artifacts remain non-authoritative through checkpoint sync, new-WAL sync, prerequisite-directory sync and pending-manifest sync. After final manifest rename the new generation is process-visible on the tested filesystem; after publication directory sync it is the intended durable generation.
- **VERIFIED:** process kill before/after an obsolete-generation deletion cannot invalidate the active published generation; fresh open still selects and validates the active generation.
- **VERIFIED END-TO-END:** a process killed inside runtime commit immediately after durable COMMIT and before in-memory whole-root publication recovers the target logical Revision and rebuilds physical + maintained state from checkpoint/WAL authority.
- **PRIVATE TESTING RULE:** crash/fault hooks are internal implementation seams only. They are not semantic authority, persisted state or caller capabilities.
- **NON-CLAIM:** process kill is not machine power loss. In particular, observing a renamed manifest after killing a process before the final directory fsync does not prove that the rename survives sudden power loss or storage-controller cache loss.
- **OPEN:** durable client transaction/idempotency identity remains required for `COMMIT durable -> ACK lost -> client retry`; recovery knows the committed head, but the client cannot yet name/query its original transaction outcome.
- **OPEN:** cross-platform/filesystem durability semantics, durable materialization configuration, durable schema/Γ/lifecycle transaction records, revision-DAG parent encoding, checkpoint streaming/migration, COW candidates, and production supervisor/group-commit/replication work remain outside this pass.

Practical next target: add durable client transaction identity and an explicit transaction-outcome/retry protocol, then decide whether to generalize durable schema/Γ mutation or integrate the already-verified Agent-2 semantic-index design.

## Pass36 implementation correction — durable client identity, generation metadata, recovery supervisor

Status after Pass36: **VERIFIED**.

- **VERIFIED:** external retry identity is now explicit `ClientTransactionId`, encoded in the checksummed logical WAL PREPARE. Recovery records exact committed transaction outcomes; exact same-ID/same-target retry is idempotent, while reuse for another target is rejected.
- **VERIFIED:** client transaction outcomes survive checkpoint rotation and obsolete-generation compaction because the committed outcome ledger is carried in durable generation metadata, not only in the active WAL tail.
- **VERIFIED:** maintained materialization configuration is durable generation authority. `metadata-N.cfdm` stores exact `MaterializationId + RelExpr` specifications and is checksummed by manifest v2. Missing/corrupt published metadata is corruption.
- **VERIFIED:** `DurableRuntime::open` no longer accepts materialization specs from its caller. It selects checkpoint, WAL and metadata from one published generation, replays logical authority, then rebuilds physical and maintained state.
- **VERIFIED:** `DurableRuntimeSupervisor` owns the normal fail-stop recovery path. An uncertain durable COMMIT or `RuntimeRecoveryRequired` causes reopen from published authority, transaction-outcome lookup and either an idempotent `AlreadyCommitted`, explicit conflict or same-ID retry.
- **UNCHANGED AUTHORITY RULE:** stable row handles, indexes, layouts, materialized results and process-local root identity remain reconstructible/non-durable authority. Generation metadata persists materialization *specification*, not materialized result state.
- **OPEN:** semantic-module implementation deployment remains external; the durable `SemanticContext` pins module digests/contracts but does not package executable implementations.
- **OPEN:** current WAL tail still represents relation-data revisions under unchanged pinned Γ. Schema/Γ/lifecycle/field transitions need a general durable revision-change representation consistent with `kernel-transport`.
- **OPEN:** durable format migration, transaction-outcome retention/GC, online materialization-config mutation, checkpoint/metadata streaming, power-loss/cross-filesystem testing, revision-DAG ancestry, group commit/replication and COW runtime candidates.

Practical next target if durability remains the priority: design and falsify a general durable revision-change protocol for schema/Γ/lifecycle transitions rather than extending the relation-data descriptor with ad-hoc flags.


## Pass37 implementation correction — general durable revision changes + online materialization configuration

Status after Pass37: **VERIFIED**.

- **VERIFIED:** the active WAL is no longer restricted to relation-data revisions under unchanged Γ. Mutation codec v3 has two typed change classes: `RelationData` and `FullRevision`.
- **VERIFIED:** `FullRevision` durably records a canonical target `Revision=(S,Γ,M)` image. Recovery decodes and revalidates that image through the ordinary revision validation boundary before it may become semantic authority. This covers schema revision changes, semantic-environment/Γ changes, lifecycle/carrier changes, field changes and relation-state changes in one durable semantic transition.
- **VERIFIED:** full-revision runtime publication uses the same `prepare -> durable PREPARE -> seal -> durable COMMIT -> infallible publish` ordering as relation-data commits. Physical layouts, row handles, indexes and materialized results remain reconstructible and are rebuilt for the new semantic revision.
- **VERIFIED:** materialization configuration is now mutable durable authority, not merely restart bootstrap metadata. `DurableRuntime::reconfigure_materializations` validates/rebuilds the desired registry first, publishes it through an atomic checkpoint-generation rotation, then atomically publishes the matching runtime root. Reopen uses the new durable configuration with no caller-supplied specs.
- **VERIFIED:** invalid materialization reconfiguration fails before durable publication and leaves both live and reopened configuration unchanged. Reapplying the already-published configuration is idempotent.
- **VERIFIED:** the transaction retry boundary rejects a supplied target Revision whose content differs from the current committed Revision even if the client transaction ID and nominal target RevisionId coincide.
- **COMPATIBILITY:** mutation codec v2 relation-data PREPARE payloads remain readable under codec v3. This does not claim automatic migration for every historical checkpoint/manifest/metadata format.
- **OPEN:** exact long-lived client-request identity after the durable head has advanced beyond an old transaction is still represented primarily by the retained transaction outcome plus nominal target revision; a future retained exact intent/content identity or global RevisionId uniqueness contract should make that historical retry proof independent of the current head.
- **OPEN:** revision-DAG/branch+merge ancestry is not yet first-class durable authority; the current durable runtime is a linear committed-head protocol.
- **OPEN:** semantic Revision replacement and materialization-configuration replacement are each atomic, but they are not yet one combined transaction class. A schema migration that simultaneously requires a different maintained-query registry must currently use an explicit compatible staging sequence.
- **OPEN:** executable semantic-module implementation artifacts/registry deployment remain external to the store even though durable revisions pin their digests/contracts.
- **OPEN:** machine power-loss and non-current-filesystem durability validation, streaming checkpoints, outcome-ledger GC, COW candidates, group commit/replication and authenticated durable storage remain future work.

Practical next direction: durability is now functionally complete for the current single-head semantic model (relation data, full semantic revision replacement, materialization configuration, checkpoint/restart and ACK-loss recovery). Remaining durability work is primarily deployment/history/scalability/assurance. This is a reasonable boundary at which to return to the historical data-plane backlog, starting with Agent-2 semantic indexes unless an independent durability hostile review finds a new correctness defect.

## Pass38 implementation correction — exact durable intent + combined semantic/config migration + builtin deployment manifest

Status after Pass38: **VERIFIED**.

- **VERIFIED:** new-format client retry identity is exact. `DurableTransactionIntent::Exact` retains canonical target `Revision=(S,Γ,M)` bytes, optional exact materialization registry and the builtin semantic implementation descriptors required by the target. The committed intent survives WAL tail compaction into generation metadata and therefore remains matchable after later heads, checkpoint, compaction and restart.
- **VERIFIED:** same `ClientTransactionId` + same nominal `RevisionId` is no longer sufficient when the retained/supplied exact target differs. Conflicting Revision contents or combined materialization configuration are rejected rather than returning false idempotent success.
- **VERIFIED:** semantic Revision replacement and materialization-registry replacement now have a combined `FullRevisionAndMaterializations` transaction class. The complete target root is built before durable publication, PREPARE binds both pieces, COMMIT linearizes them, and recovery rebuilds one coherent runtime root.
- **VERIFIED:** current builtin equality/tokenizer/ordering implementation revisions have durable `BuiltinSemanticModuleSpec` descriptors. Published generation metadata and exact historical intents retain those descriptors; normal reopen reconstructs the builtin `SemanticRegistry` before revalidating checkpoint/WAL Revision images.
- **AUTHORITY RULE:** builtin deployment descriptors identify deterministic implementation families/revisions compiled into CFMD. Durable storage still does not carry arbitrary executable/plugin code, and a digest alone is never treated as executable authority.
- **COMPATIBILITY:** mutation codec v4 retains explicit v2/v3 decoding; metadata codec v2 retains v1 decoding. Legacy transaction records become `LegacyTargetOnly` and are never silently upgraded to exact intent. Legacy metadata without semantic deployment descriptors requires an explicit compatibility registry path.
- **OPEN:** durable revision DAG/merge ancestry, universal format migration, arbitrary semantic executable artifact packaging/signing, intent-ledger GC, streaming codecs, machine-power-loss/cross-filesystem assurance, COW runtime roots, distributed durability and authenticated durable storage.

Practical next direction: the single-head durable control plane is now functionally complete enough that the highest-value mainline returns to the historical data-plane backlog, beginning with production integration of the already-verified Agent-2 semantic canonical-key/index design unless hostile review produces a new correctness counterexample.

## Pass39 implementation correction — production semantic canonical indexes

Status after Pass39: **VERIFIED**.

- **VERIFIED:** every currently supported builtin primitive equality module has an exact production canonical equality key satisfying key equality iff pinned Γ equivalence.
- **VERIFIED:** every currently supported builtin ordering module has an exact production canonical order key whose key comparison matches pinned Γ comparison, including hostile F64 Total values.
- **VERIFIED:** new `kernel-semantic-index` provides Γ-bound, key-encoding-versioned derivative bucket indexes generic over row identity. Internal query indexes use local derivative identity and do not reuse storage handles as semantic authority.
- **VERIFIED:** maintained non-I64 primitive Join probes semantic buckets instead of scanning the complete opposite relation for every changed row. Structural/custom equivalences deliberately retain the exact scan fallback.
- **VERIFIED:** Group maintains composite canonical lookup for keys composed of current builtin primitive equivalences; structural/custom equivalence groups remain fallback.
- **VERIFIED:** generic Text/F64/current non-I64 builtin TopK uses ordered canonical-key tie buckets with exact WITH-TIES behavior.
- **HOSTILE CORRECTION:** release benchmarking exposed mutating index operations accidentally placed inside `debug_assert!`; release builds therefore dropped those side effects. Pass39 moved all mutations outside assertions, added release regression coverage, and completed the full release/overflow gate.
- **PERFORMANCE:** TextAsciiCI Join benchmark from 1k to 50k right rows leaves indexed update approximately flat (~1.36-1.40 us) while a deliberately minimal full scan grows ~50.8x. At 1k the minimal scan is still faster, so cost-based index selection remains required.
- **OPEN:** persisted Text/F64/Bool/entity storage indexes and planner integration, structural/custom canonical keys, shared index reuse, memory budgeting, key-encoding migration, typed-batch Group/TopK, I64 Group/TopK constant-factor gaps, and multiway/mixed-key join planning.

Practical next direction: extend the now-proven semantic-index abstraction into persisted `PhysicalStore`/planner paths or use it as the base for typed-batch Group/TopK optimization. No change to semantic authority is required for either route.

## Pass40 implementation correction — persisted primitive semantic indexes and lineage refinement decision

Status after Pass40: **VERIFIED**.

- **VERIFIED:** Pass39 canonical primitive equality keys now back a long-lived reconstructible physical index inside `PhysicalStore`, not only maintained-query derivative state. `MaterializedSemanticIndexState` stores `CanonicalEqKey -> stable PhysicalRowId bucket`; copied logical rows never become index authority.
- **VERIFIED:** current builtin primitive equality domains supported by the persisted semantic index are UnitExact, BoolExact, I64Exact, F64Bitwise, TextExact, TextAsciiCaseInsensitive, LiveEntityIdExact and HistoricalEntityIdExact. The existing specialized I64 physical index remains available as its own hot path.
- **VERIFIED:** relation mutation and every bound primitive semantic index are validated/planned together and published atomically. Reinstalling the relation invalidates its bound semantic indexes. An index whose resolved equivalence no longer matches pinned Γ requires rebuild and is not silently reinterpreted under the new law.
- **VERIFIED:** direct physical `FilterEqConst(Scan)` and equality `Join(Scan,Scan)` can consume a compatible persisted primitive semantic index through `IndexedPrimitiveIfAvailable`; index candidates are rechecked with the exact resolved Γ equivalence before output. The historical Pass26 persisted Text/F64/Bool/entity-index + direct physical Filter/Join consumption gap is therefore closed. Cost/selectivity-driven automatic index choice remains separate planner debt.

- **VERIFIED Pass40 final cleanup:** persisted primitive physical semantic indexes and maintained semantic indexes share the identity-generic `kernel-semantic-index::SemanticBucketIndex<Key, Identity>` implementation. Bucket membership preserves identity insertion order while an exact reverse map prevents one identity from belonging to multiple keys. This avoids duplicated bucket machinery without making `PhysicalRowId` semantic authority.
- **PARTIAL:** the maintained I64 Group count hot path no longer re-admits an already pinned semantic environment for every tiny delta and has a direct one-remove/one-insert replacement path with singleton slot reuse. A 50k-group release benchmark improved from the historical ~4.8x hand baseline to ~2.3–2.4x. The remaining constant-factor gap stays OPEN.
- **DESIGN DECISION:** first-class universal `Chain<T>` / `Lineage<T>` is rejected for the current model. A rooted single-parent acyclic lineage elaborates to `PartialMap<Node,Node>` + rooted/acyclic/reachability constraints; its root→node path is a derived `Seq<Node>`, and append/branch/ancestor/LCA are rewrites/queries. If ergonomic support is wanted, it should be a surface refinement/standard-library schema helper. Physical acceleration may use `AdjacencyList`, depth/jump tables, binary lifting, persistent chunks/ropes or cached shortcut edges without changing authoritative parent semantics.
- **OPEN:** structural/custom canonical keys, cost-based index creation/selection, shared-index reuse, memory budgeting/eviction, canonical-key encoding migration, multi-column/mixed-key physical semantic indexes, typed-batch Group/TopK, residual I64 Group/TopK constant factors and multiway join planning.

Practical next direction: the primitive persisted-index hole is closed. The highest-value data-plane work is now typed-batch Group/TopK plus remaining I64 constant-factor work, or broader multi-column/mixed-key/multiway planning with cost-aware index selection.


## Pass41 implementation correction — compositional typed Group/TopK producers

Status after Pass41: **VERIFIED**.

- **VERIFIED:** typed columnar execution no longer has to materialize `Vec<Row>` merely because a current builtin primitive `Group` or `TopKWithTies` occurs in the middle of the physical DAG. The internal `OwnedTypedBatch` is derivative execution state only and may feed downstream Group/TopK/Filter/Project before final row materialization.
- **VERIFIED:** primitive Group production derives equality keys only from the exact pinned Γ modules through canonical-key law. Text ASCII-CI and other current builtin primitive equivalences are eligible; structural/custom equivalences remain exact fallback and are not assigned an invented key representation.
- **VERIFIED:** TopK production supports current builtin orderings, including F64 total ordering, and can consume the output of another stateful typed producer. Raw typed TopK compacts retained positions instead of cloning the complete physical source columns.
- **VERIFIED CORRECTION:** descending I64 positional TopK now chooses the `len-k` order statistic rather than the k-th minimum. This fixes a pre-existing case that could retain too many rows in descending mode.
- **PARTIAL PERFORMANCE:** exact one-column I64 maintained TopK now stores only `key -> multiplicity` and has a specialized tiny-delta path. Its measured hand-baseline gap fell from historical ~20–23x to ~6.3–7.3x, but remains OPEN.
- **PARTIAL PERFORMANCE:** the maintained exact-I64 Group singleton replacement path reuses the existing group slot and avoids redundant count/lookup work. Its measured gap fell from Pass40 ~2.3–2.4x to ~1.94–2.00x, but remains OPEN.
- **OPEN:** structural/custom canonical indexes, composite/mixed-key persisted indexes, multi-key/multiway Join planning, explicit cost/selectivity/index-lifecycle policy, and the remaining I64 constant-factor gaps.

No new logical type or semantic authority is introduced by `OwnedTypedBatch`; it is an erasable physical execution representation.

## Pass42 refinement — composite semantic indexes and costed access

The current physical layer may maintain a reconstructible persisted semantic index over an ordered non-empty list of primitive key parts `(column, equivalence)`. Each part MUST be canonicalized under the exact pinned Γ module; the physical index MUST NOT be reused when any resolved component contract changes.

A logical query may express exact equality between two columns using `FilterEqColumns`. This is ordinary relational semantics, not a storage/index primitive. A physical optimizer MAY combine a direct two-way equality Join and cross-side column-equality filters into a composite persisted semantic-index access only when every predicate is represented by the index binding and physical candidates are checked against the same pinned Γ laws before publication as query output.

The executor MUST be allowed to reject use of an installed index when its estimated work is not lower than the corresponding scan path. Pass42 supplies a correctness-first direct Filter/Join work model. This does not define the policy for creating, retaining, sharing, rebuilding or evicting physical indexes; those remain optimizer/lifecycle concerns.

## Pass43 implementation correction — workload-driven semantic-index lifecycle

Status after Pass43: **VERIFIED**.

- **VERIFIED:** current builtin primitive persisted semantic indexes have a production advisor boundary driven by an explicit workload horizon. The advisor may create, rebuild, retain, reuse or evict its own `MaterializedSemanticIndexState` instances without changing semantic `Revision=(S,Γ,M)`.
- **VERIFIED:** demand is aggregated by exact Γ-bound `SemanticIndexBinding`; a prospective build is charged once against expected repeated Filter/Join savings, so multiple plans/operators can justify and share one physical index instead of independently creating copies.
- **VERIFIED:** advisor ownership is separate from index existence. An externally/manual installed compatible index may be reused but does not become advisor-owned and cannot be evicted by this policy. Explicit installation relinquishes advisor ownership for that binding.
- **VERIFIED:** selected missing/stale indexes are fully rebuilt/validated before the physical reconciliation begins. Changed advice advances one physical transition and publishes one immutable runtime root; identical no-op advice does not republish. Γ contract drift behind the same `SemanticId` forces rebuild rather than reinterpretation.
- **VERIFIED:** advisor discovery covers current direct primitive Filter/composite-Filter and direct two-way equality/composite-Join opportunities even when nested below ordinary unary plan nodes. Structural/custom equivalence still has exact fallback and receives no fabricated canonical key.
- **BOUNDARY:** `max_managed_key_cells` is a deterministic footprint proxy (`rows × key parts`) for advisor-owned generic semantic indexes. It is not allocator-byte/RSS accounting and external indexes are intentionally outside this managed-owner budget.
- **BOUNDARY:** explicit workload samples are not autonomous workload telemetry. Online observation/decay/hysteresis/background scheduling remains OPEN.
- **BOUNDARY:** the specialized persisted I64 index remains a separate physical family. Pass43 does not claim an optimal choice between specialized I64, generic semantic indexes and future layouts; multi-family costing belongs with the next general planner layer.
- **PERFORMANCE DEBT:** post-Pass43 diagnostics show maintained I64 Group ~2.007x and I64 TopK ~5.175x current hand-written baselines. Neither gap is CLOSED. Existing semantic Join diagnostics again show a small-relation scan/index crossover, supporting costed rather than unconditional index policy.

No new logical primitive, equality/order law or durable authority is introduced by the advisor. It is a reconstructible physical policy over already-certified semantic contracts.

## Pass44 implementation correction — costed Join families + contiguous multiway reassociation

Status after Pass44: **VERIFIED**.

- **VERIFIED:** scan-vs-index costing now applies across current ordinary and specialized I64 Join fast paths, including fused `Join -> Project` and typed-batch paths. A rejected persisted I64 path cannot silently become an uncosted ephemeral I64 build.
- **VERIFIED:** a profitable direct primitive non-I64 equality Join may construct an execution-local canonical semantic index. Its keys come only from the admitted pinned-Γ canonical equality law; candidates are rechecked under the exact equivalence before output. This derivative index is neither durable nor installed physical authority.
- **VERIFIED/PARTIAL:** connected contiguous/in-order equality Join trees can be reassociated by an interval planner using row/distinct-index statistics. `FilterEqColumns` participates as equality edges, and a persisted index on a later base relation remains usable even when the left side is already an intermediate result.
- **BOUNDARY:** Pass44 does not prove or implement arbitrary relation permutation/general bushy planning. Current search preserves leaf order and assumes the supported equality-tree fragment.
- **BOUNDARY:** current statistics are deliberately simple. Correlated selectivity, histograms and arbitrary structural-key statistics remain optimizer work.
- **BOUNDARY:** specialized I64 and generic semantic indexes are both visible to current Join costing, but lifecycle ownership/advice is not yet represented by one universal multi-family candidate model.

No new logical query primitive or semantic authority is introduced. Reassociation is a physical refinement checked against the same logical query semantics and pinned Γ contracts.


## Pass45 implementation correction — first-class multi-family Join access decisions

Status after Pass45: **VERIFIED**.

- **VERIFIED:** current primitive equality Join planning/execution has one deterministic `JoinAccessDecision` boundary over `FullScan`, persisted specialized I64, persisted generic semantic, transient specialized I64 and transient generic semantic access. Direct, multiway/intermediate and current fused Join paths consume the same candidate law rather than relying on sequential family-specific fallthrough.
- **VERIFIED:** access work is explicit in `SemanticAccessCostModel`: scan work, persisted probe work and transient build+probe work are compared under one correctness-first physical model. Equal-work behavior is deterministic and does not silently depend on helper-call order.
- **VERIFIED:** intermediate-left/direct-right Join execution implements the same current primitive families that the planner may select. Transient candidates are re-costed after construction with the **actual** distinct-key count; if duplicate-heavy data invalidates the optimistic prospective estimate, execution discards that candidate and returns to exact scan semantics.
- **VERIFIED:** the multiway interval planner deliberately does not treat unknown transient distinctness as measured statistics. It credits persisted statistics or scan and leaves richer retained statistics/histograms/correlation estimates OPEN.
- **VERIFIED:** duplicate-heavy fused I64 Join execution performs at most one observable transient build attempt. If actual selectivity rejects the transient index, the already-prepared batch path performs an exact scan directly rather than escaping into a fallback that rebuilds the same derivative index.
- **BOUNDARY:** this closes the current primitive Join candidate-selection inconsistency, not general arbitrary-permutation/bushy Join planning. Leaf-order-preserving interval search remains the verified multiway fragment.
- **BOUNDARY:** Pass43 lifecycle ownership still manages the generic semantic-index family. Unified execution costing does not yet make specialized I64/future layouts first-class lifecycle-owned advisor families.
- **BOUNDARY:** byte-accurate memory budgeting, autonomous telemetry/decay, retained multi-column statistics and future physical-layout candidates remain OPEN.

No new semantic primitive or source of truth is introduced. Every indexed/transient structure remains reconstructible physical state and every result is governed by the same pinned-Γ exact equality semantics.


## Pass46 implementation correction — retained Γ-bound semantic key statistics

Status after Pass46: **VERIFIED**.

- **VERIFIED:** `PhysicalStore` can retain reconstructible exact key multiplicities for current builtin primitive `SemanticIndexBinding` values. Public planner summary exposes exact `row_count` and `distinct_key_count`; the retained artifact is not semantic authority.
- **VERIFIED:** persisted semantic indexes and retained statistics share the same binding→resolved-module/canonical-key resolver. Single-column and composite/mixed primitive keys therefore use one pinned-Γ canonicalization contract. Structural/custom equivalence remains exact fallback.
- **VERIFIED:** statistics participate in the atomic physical relation transition. Sequential duplicate birth/death updates maintain exact multiplicities, relation reinstall invalidates dependent statistics, and Γ drift behind the same `SemanticId` requires rebuild rather than reinterpretation.
- **VERIFIED:** `RuntimeRevisionCell` publishes statistics as reconstructible physical root state under the same semantic revision. Old readers retain prior immutable roots; a physical statistics publication stales earlier prepared transitions.
- **VERIFIED:** `JoinAccessDecision` consumes compatible retained distinctness for transient I64/generic semantic costing. Duplicate-heavy distributions can reject a build before construction; multiway costing can credit transient access only when measured retained statistics exist. Pass45 post-build actual-distinct recheck remains in force.
- **AUTHORITY GUARD:** stale/incompatible or row-count-incoherent statistics are ignored by the planner. They can change only physical plan choice, never exact query meaning or result validation.
- **BOUNDARY:** Pass46 does not implement arbitrary leaf permutation/general bushy planning. The current deterministic physical row-production order makes naïve permutation observably different; an explicit order-restoration/provenance mechanism is required first.
- **BOUNDARY:** retained statistics are exact key cardinality, not histograms/correlation/telemetry. Their autonomous lifecycle, decay and byte-level budget remain OPEN.

No new logical primitive or durable authority is introduced. Statistics are reconstructible physical evidence derived from validated `Revision=(S,Γ,M)` under pinned Γ.


## Pass47 implementation correction — physical Join provenance and exact order restoration

Status after Pass47: **VERIFIED**.

- **VERIFIED:** physical multiway execution has an explicit provenance boundary for relation permutation. Each base leaf contributes its authoritative logical scan ordinal and row fragment; reordered intermediate joins carry these per-leaf coordinates without changing semantic authority.
- **VERIFIED:** restoration sorts completed rows lexicographically by the original leaf scan-ordinal vector and flattens fragments in original logical leaf order. For the current flattened equality fragment this reproduces the logical evaluator's left-major/right-minor Bag row-production contract.
- **VERIFIED:** the first non-contiguous three-way primitive equality path can join leaves 0 and 2 first when retained Γ-bound statistics make its complete estimated work lower than adjacent alternatives. The estimate includes final restoration-sort work.
- **VERIFIED:** all equality filtering remains pinned-Γ exact. Provenance/order metadata is reconstructible physical execution state and does not enter `Revision=(S,Γ,M)` or durable authority.
- **HOSTILE BOUNDARY:** the initial permutation path declines when relevant persisted semantic/I64 indexes already exist and when compatible retained statistics are unavailable; existing exact indexed/fallback paths remain authoritative for those cases.
- **OPEN:** this is not arbitrary general bushy closure. N-way subset enumeration, indexed/typed permuted execution, richer predicate graphs and a compact stable-handle/native provenance representation remain OPEN. The current correctness-first implementation clones logical `Row` fragments and performs an explicit final sort.


## Pass48 implementation correction — order-preserving semijoin masks replace final restoration

Status after Pass48: **VERIFIED**.

- **HOSTILE CORRECTION:** Pass47's provenance + final global sort was semantically sound but is no longer the current physical architecture. Pass48 review identified it as post-hoc compensation: reordered execution destroyed reference enumeration order and then paid tuple provenance plus `O(output log output)` restoration to reconstruct it.
- **VERIFIED:** current primitive equality permutation execution builds pinned-Γ canonical compatibility buckets/bit masks and semijoin support masks. These are reconstructible execution-local physical state only.
- **VERIFIED:** final result enumeration proceeds directly in original leaf order and authoritative logical scan-ordinal order. Candidate masks are intersected as earlier leaves are assigned, with forward viability checks for future leaves. There is no final order-restoration sort and production source contains no `ProvenanceJoinRow`/`ProvenanceJoinFragment` carrier.
- **VERIFIED:** completed tuples are revalidated against every exact Γ equality predicate before output. Canonical masks therefore accelerate/prune execution but never become semantic authority.
- **VERIFIED/PARTIAL:** retained statistics drive bounded subset selectivity search for 3–8 leaves. The subset DP is an admission/cost device; execution does not need to materialize the selected bushy tree in its tuple order. A four-way hostile fixture verifies exact reference Bag multiplicity, columns and order.
- **VERIFIED:** existing persisted singleton-side I64/semantic access remains preferred when the unified `JoinAccessDecision` says it is cheaper. Execution-local masks are treated as their own transient pruning structure rather than blindly losing to another ephemeral-build estimate.
- **PERFORMANCE EVIDENCE:** the existing adversarial three-way diagnostic after the rewrite measured about 5.396 ms baseline versus 0.036 ms optimized (~148.7x). This is fixture-specific evidence only, not a universal speed claim.
- **OPEN:** search remains capped at eight leaves; compatibility masks are rebuilt per admitted execution and currently use builtin primitive canonical equality. Adaptive dense/sparse/indexed mask representation, WCOJ-style physical kernels, general indexed/typed subset nodes, structural/custom canonical laws, richer correlation statistics and byte-level memory policy remain OPEN.

No logical query primitive, semantic equality/order rule, transaction law or durable authority changed in Pass48. The change is a physical correction: preserve required enumeration order by construction instead of destroying and restoring it.

## Pass49 implementation correction — Γ-Quotient Constraint Network

Status after Pass49: **VERIFIED**.

- **HOSTILE/ARCHITECTURAL CORRECTION:** Pass48's pairwise semantic compatibility masks were correct but still mirrored the syntactic equality-edge graph. They are no longer the current physical representation.
- **VERIFIED:** multiway primitive equality pruning now constructs a **Γ-Quotient Constraint Network (Γ-QCN)**. For a target equivalence `E`, every predicate edge whose law refines `E` participates in `G_E`; each connected component is one sound `E`-quotient coordinate because pinned Γ proves refinement and equivalence transitivity.
- **VERIFIED:** a finer/coarser law chain can derive a useful quotient coordinate that was not written as a direct predicate. Example: `A TextExact B` plus `B TextAsciiCI C` yields `{A,B}/Exact` and `{A,B,C}/ASCII-CI` physical coordinates.
- **VERIFIED:** redundant equality cycles/cliques are factorized. A four-coordinate exact-equality clique with six syntactic edges becomes one quotient coordinate. If two constraints cover the same component, a checked finer law eliminates the redundant coarser physical constraint.
- **VERIFIED:** physical canonicalization is cached by `(leaf,column,target equivalence)`. Multiple same-row columns in one quotient coordinate must map to the same canonical class; disagreement eliminates the row before tuple enumeration.
- **VERIFIED:** each quotient coordinate computes the N-way intersection of canonical-key domains across participating leaves. Support masks are then monotonically reduced across all quotient coordinates to a finite fixed point, so loss of support in one semantic coordinate can invalidate support in another before enumeration.
- **VERIFIED:** result enumeration still follows original leaf and authoritative scan-ordinal order. Completed tuples are revalidated against every original exact Γ predicate. Quotient components/domains/masks are reconstructible optimizer evidence only and never semantic authority.
- **VERIFIED:** the quotient basis is Γ-sensitive: hostile tests show that repinning the same semantic IDs to different certified module contracts changes the physical quotient basis. Host `Eq/Hash/Ord` cannot substitute for this boundary.
- **VERIFIED:** costing now counts actual unique derived quotient coordinates rather than only syntactic predicate endpoints, so refinement-derived canonicalization work is not hidden.
- **PERFORMANCE EVIDENCE:** the established adversarial 3-way diagnostic remains about 4.649 ms baseline versus 0.0315 ms optimized (~147.4x). This is fixture-specific evidence only.
- **NON-CLAIM:** this is not a claim that quotienting, transitive equality reasoning, semijoin reduction or constraint propagation are academically novel in isolation. The project-specific architectural result is that CFMD can combine them as an exact optimizer primitive over versioned semantic quotient laws supplied by pinned Γ.
- **OPEN:** Γ-QCN is rebuilt per admitted execution; checked prepared-plan compilation, shared/persisted quotient factors, `Change/Dq` maintenance, adaptive dense/sparse/native representation, structural/custom canonical targets, unbounded/adaptive search and full multi-family lifecycle remain OPEN.

No logical query primitive, semantic equality/order rule, transaction law or durable authority changed in Pass49. The physical optimizer now reasons over semantic quotient coordinates rather than treating the written pairwise Join graph as the fundamental object.



## Pass50 implementation correction — prepared Γ-QCN and maintained canonical quotient factors

Status after Pass50: **VERIFIED**.

- **VERIFIED:** eligible prepared multiway plans compile the Pass49 semantic quotient basis once under pinned `(query, Γ)` into private `PreparedSemanticQuotientProgram` metadata. Prepared execution reuses that basis; dynamic Plan execution retains exact on-demand compilation.
- **VERIFIED:** primitive quotient endpoint keys can be materialized as dedicated `semantic_quotient_factors` in `PhysicalStore`. The representation reuses the stable-handle `MaterializedSemanticIndexState` canonical-key machinery but the family is intentionally separate from ordinary semantic access indexes.
- **AUTHORITY BOUNDARY:** quotient factors do not participate in generic Join access-path selection and are not silently adopted by the semantic-index advisor. Factor presence therefore cannot change logical semantics or ordinary index ownership merely because a prepared Γ-QCN wanted reusable canonical evidence.
- **VERIFIED:** `PreparedPlan::materialize_semantic_quotient_factors` fully builds missing/stale factors before one physical publication epoch; compatible repeated materialization is a no-op.
- **VERIFIED:** relation reinstall invalidates factor state. Exact relation deltas prevalidate and atomically update specialized I64 indexes, generic semantic indexes, quotient factors and retained semantic statistics together with the relation transition. Γ incompatibility requires rebuild rather than reinterpretation.
- **VERIFIED:** Γ-QCN key-cache construction first consumes maintained `PhysicalRowId -> CanonicalEqKey` factor evidence. If no compatible factor exists, exact row canonicalization remains the fallback. A hostile 140-row test observes all 140 quotient endpoint keys served from factors both before and after a relation delta while output remains equal to fresh logical reference evaluation.
- **PARTIAL / OPEN:** N-way common quotient-key domains, support masks and the cross-coordinate support fixed point are still reconstructed per execution. Pass50 therefore does not claim full incremental Γ-QCN maintenance through `Change/Dq`.
- **OPEN:** factor create/retain/share/evict/rebuild policy, exact byte/RSS budgeting, structural/custom canonical factor representations, adaptive dense/sparse/native support state and bounded-search expansion remain part of existing multi-family/multiway frontiers.

No logical query primitive, semantic equality/order law, transaction law or durable authority changed in Pass50. The architecture now places each reusable artifact at the narrowest valid boundary: quotient-law compilation in prepared `(query,Γ)` metadata, canonical endpoint factors in reconstructible revision-derived physical state, and exact Γ predicates at the semantic checker boundary.


## Pass51 implementation correction — maintained Γ-QCN support fixed point through exact Change/Replace

Status after Pass51: **VERIFIED**.

- **VERIFIED:** the Pass49/50 Γ-QCN common-domain/support fixed point can now be materialized as dedicated reconstructible physical state in `PhysicalStore`; it is not semantic authority and is never serialized as logical revision truth.
- **VERIFIED:** the support artifact is bound to the prepared quotient specification, participating relation/layout leaves, exact pinned `SemanticContext`, and exact current stable-handle vectors. A mismatch causes execution to use fresh exact Γ-QCN construction rather than consuming stale support.
- **VERIFIED:** prepared execution can consume the already-converged support state directly. On a maintained-support hit, read execution does not rebuild quotient endpoint keys or rerun the support fixed point.
- **VERIFIED:** relation reinstall invalidates every support program touching that physical relation/layout.
- **VERIFIED:** an exact relation delta first updates the authoritative physical relation and the Pass50 canonical quotient factors inside the unpublished candidate, then derives a fresh support fixed point and replaces the affected support artifact before one physical transition epoch is published. Invalid deltas leave relation/factors/support unchanged.
- **CHANGE-CALCULUS INTERPRETATION:** this is the universal exact derivative boundary already permitted by the normative change theory: `Fine input delta -> Replace(fresh dependent support state)`. Logical completeness and incremental correctness therefore do not depend on a bespoke invalidation system.
- **PARTIAL / OPEN:** the support derivative is not yet fine-grained. An affected program currently recomputes its support fixed point on the write path; queue/support-count propagation and revision-batch coalescing remain optimization work. A multi-relation candidate may rebuild the same support program more than once before unpublished candidate publication.
- **EXTERNAL R&D / NOT INTEGRATED:** reported structural canonical-key, structural maintained-support and fixpoint-adjacency prototypes remain independent R&D evidence until their patch/ZIP/raw measurements are received and hostile-reviewed against the authoritative branch.

No logical query primitive, equality/order law, transaction law or durable authority changed in Pass51. Pass51 moves the Γ-QCN support fixed point from execution-local recomputation to an exact maintained derivative boundary while preserving `Revision=(S,Γ,M)` as semantic authority.

## Pass52 implementation correction — stable-handle local Γ-QCN deletion derivative

Status after Pass52: **VERIFIED**.

- **VERIFIED:** materialized Γ-QCN support state no longer requires the Pass51 full `Replace` derivative for every fine relation delta. Pure deletions use an exact local derivative when current stable-handle vectors prove the new leaf coordinates are ordered subsequences of the old ones.
- **VERIFIED:** `PhysicalRowId` transports the derivative across dense-position changes. Base masks and quotient-leaf canonical-key arrays are remapped to current logical ordinals without recanonicalizing surviving payload rows; changed quotient buckets are rebuilt only for affected leaves.
- **VERIFIED:** support loss is propagated through a quotient-constraint dependency queue. The derivative is monotone finite descent from the previously converged support fixed point and can cascade across several quotient coordinates without recomputing the complete fixed point.
- **HOSTILE CORRECTION:** an initial suffix-only deletion route was rejected before freeze because it made local maintainability depend on accidental physical row position. The production derivative handles arbitrary pure deletions, including interior rows and duplicate buckets spanning multiple mask words.
- **EXACT FALLBACK:** insertions/resurrection and mixed deltas still use Pass51 exact `Replace`. Support growth can activate mutually supporting fixed-point components, so Pass52 does not introduce an unproved symmetric bit-activation rule.
- **AUTHORITY:** local support state remains reconstructible physical evidence. `Revision=(S,Γ,M)` remains semantic authority and completed tuples still pass original exact Γ predicates.
- **OPEN:** sound local activation for insertions/resurrection, one derivative per complete multi-relation revision, per-key support-counter refinements, Γ-QCN lifecycle/memory policy and structural/custom canonical factors remain open.

No logical query primitive, equality/order law, transaction law or durable authority changed in Pass52. The change is a physical refinement of the universal exact Change law: where deletion monotonicity and stable-handle transport prove a smaller derivative, the implementation uses it; otherwise it falls back to exact `Replace`.



## Pass53 implementation correction — structural Γ canonicalization and relation→adjacency lowering

Status after Pass53: **VERIFIED**.

- **VERIFIED:** exact in-memory canonical equality keys now compose through admitted structural Product/Option/Sum/Seq/Set/Bag/Map and guarded Mu/Var definitions. Primitive pinned Γ modules remain the base authority; unordered constructors normalize by child canonical keys.
- **VERIFIED:** hostile tests compare structural canonical-key equality directly against the independent `equivalent(...)` oracle on representative-sensitive, order-permuted and multiplicity-sensitive samples. Existing recursive tests cover guarded recursive equality.
- **VERIFIED:** `MaterializedSetSupportState` now lowers semantic support classes to `CanonicalRowKey -> slot`; delta application prevalidates grouped underflow and no longer clones the whole support state for atomicity.
- **VERIFIED:** reachability and schema subtype traversal erase finite relations to deterministic adjacency instead of rescanning all edges/inclusions at every frontier step. Exact certificate witness behavior and all-pairs subtype closure are hostile-checked against relation-scan references.
- **AUTHORITY:** schema `inclusions` remains the direct-relation authority/durable representation; `inclusion_parents` is private derived adjacency reconstructed by `include`. Structural canonical keys are reconstructible in-memory evidence and are **not** admitted into persisted `KEY_ENCODING_REVISION=1`.
- **BOUNDARY:** structural maintained Join/index/Group families, structural Γ-QCN factors, custom/plugin canonical laws and durable recursive-key encoding remain OPEN. The R&D SemanticBucketIndex replacement experiment remains rejected because mutation wins caused material read/build regressions.

No logical query primitive, semantic equality/order law, transaction law or durable authority changed in Pass53. The pass removes generalization tax by compiling already-admitted semantic/relational mathematics into exact reconstructible physical forms.


## Pass54 implementation correction — maintained structural Group canonicalization

Status after Pass54: **VERIFIED**.

- **VERIFIED:** maintained Group may use exact compositional Γ canonical keys for structural and mixed structural+primitive group coordinates instead of falling back to recursive semantic group scans.
- **VERIFIED:** all-primitive composite Group retains pre-resolved primitive encoders, single exact I64 retains the specialized lookup, and unsupported future/custom laws retain the exact semantic fallback.
- **VERIFIED:** canonical-admitted Group replay uses the same canonical equality contract for delta-side key comparison, group lookup, bucket insertion/removal and moved-slot repair.
- **HOSTILE:** `Set<TextAsciiCaseInsensitive> × I64Exact` fixture proves physical Set permutation/case changes collapse only in the structural coordinate while the I64 coordinate remains discriminating; maintained delta equals full recompute.
- **AUTHORITY BOUNDARY:** canonical keys are in-memory reconstructible physical evidence under the pinned `SemanticContext`; no logical Group law or representative semantics changed.
- **DURABILITY BOUNDARY:** recursive structural keys are still not persisted under the current semantic-index key encoding revision. Structural Join/index and Γ-QCN structural factors remain OPEN.

No new logical primitive, semantic law or durable authority is introduced in Pass54.


## Pass55 implementation correction — Γ quotient relation multiset equality/diff

Status after Pass55: **VERIFIED**.

- **VERIFIED:** exact relation Bag equality uses `CanonicalRowKey -> multiplicity` when all participating pinned Γ equivalences admit certified canonical keys; the previous pairwise matcher remains the exact fallback when canonicalization is unavailable.
- **VERIFIED:** semantic relation diff subtracts target multiplicities by canonical class while scanning source rows in original order, preserving Bag multiplicity, source representative identity and output ordering of the prior first-match algorithm.
- **HOSTILE:** a mixed `Set<TextAsciiCaseInsensitive> × I64Exact` fixture compares the canonical path directly against the old semantic matching oracle for equality and diff, including duplicate semantic classes and physically permuted structural representatives.
- **DIAGNOSTIC:** the rebased 4000-row reverse-order release fixture measured `205,238,214 ns -> 1,764,479 ns` (~116.316x); this is adversarial evidence against the quadratic baseline, not a universal speed claim.
- **R&D REVIEW:** the same bundle's relation-only Arc/COW, dense lifecycle and algebraic native-layout programs remain prototypes rather than merged production because their root ownership/lifecycle/layout contracts are incomplete.
- **AUTHORITY/DURABILITY:** canonical relation multisets are reconstructible physical evidence. `Revision=(S,Γ,M)` remains authority; recursive structural key persistence, custom/plugin canonical laws and structural Join/Γ-QCN physical families remain OPEN.

No logical relation law, semantic equality law or durable encoding changed in Pass55.


## Pass56 implementation correction — maintained structural Join canonical indexing

Status after Pass56: **VERIFIED**.

- **VERIFIED:** maintained Join no longer has a `GenericScan` storage family for the current admitted semantic universe. Exact I64 retains its specialized buckets; other primitive equalities retain resolved semantic indexes; structural equalities use Γ-canonical structural buckets.
- **VERIFIED:** structural maintained output traverses left identities and matching right canonical buckets in deterministic insertion order, preserving Bag multiplicity and the existing maintained Join enumeration contract without post-sort restoration.
- **VERIFIED:** structural Join delta maintenance canonicalizes only changed join keys, probes the opposite canonical class, prevalidates both side mutation plans and commits only after validation succeeds. Full row semantic equality is retained inside a narrowed canonical bucket where row identity/set-validity, rather than join-key equality alone, must be established.
- **HOSTILE:** a structural Product over `TextAsciiCaseInsensitive` joins differently represented values through one canonical class and maintained left/right insertions produce a delta semantically identical to full recomputation.
- **REMOVAL:** workspace search contains zero `GenericScan` symbols in `crates/` after Pass56. The old Join-specific generic scan/delta functions are deleted rather than retained as a second reachable path.
- **AUTHORITY/DURABILITY:** structural Join buckets are reconstructible in-memory evidence under the exact `SemanticContext`; recursive structural keys are still not persisted under the current key encoding revision.
- **OPEN:** persisted structural indexes, structural Γ-QCN factors, custom/plugin canonical laws and durable recursive-key encoding remain outside this closure.

No logical Join primitive, semantic equality law or durable authority changed in Pass56.


## Pass57 implementation correction — persistent/COW runtime payload roots

Status after Pass57: **VERIFIED**.

- **VERIFIED:** maintained relational plans share recursive physical state through `Arc<MaintainedRelPlanNode>` and detach only mutation paths with `Arc::make_mut`; clone no longer scales with maintained subtree payload size.
- **VERIFIED:** heavy `PhysicalStore` artifacts (relations, I64/semantic indexes, Γ-QCN factors/support and semantic statistics) are shared per artifact and COW-mutated. Exact physical mutation semantics and atomic candidate publication remain unchanged.
- **VERIFIED:** a prepared physical transition records an in-process root identity witness rather than a deep source-store snapshot. Identity, epoch and source revision jointly reject stale publication. The witness is reconstructible runtime metadata and is never durable semantic authority.
- **VERIFIED:** logical target validation compares untouched state directly and reconstructs only relations named by the transition, avoiding a whole-`DatabaseState` clone.
- **HOSTILE:** explicit tests prove that cloned maintained-plan/physical-store candidates initially share roots, detach affected roots on mutation, preserve the original snapshot, and maintain affected indexes consistently.
- **BOUNDARY:** the outer artifact catalogs remain ordinary `BTreeMap<K, Arc<State>>`; cloning still copies map metadata in proportion to the number of managed artifacts. A persistent map/root representation remains OPEN for large catalogs.
- **BOUNDARY:** transient `EqClassId` from the reviewed R&D branch is not part of production. Pass55 canonical row-multiset lowering remains authoritative, including exact fallback for future/custom equality without canonical keys.

No logical primitive, Γ law, durable revision identity or transaction semantics changed in Pass57. This pass removes payload-size-dependent clone tax from unpublished runtime candidates while preserving immutable-reader/root publication semantics.


## Pass58 implementation correction — dense revision-local identity and Γ-QCN insertion Dq

Status after Pass58: **VERIFIED**.

- **VERIFIED:** deterministic `DenseEntityIds` compiles one finite revision entity set into opaque `LocalEntityId(u32)` ordinals and supports exact external↔local reconstruction; `DenseEntitySet` is a compact exact local extent. Logical/durable identity remains `EntityId`.
- **VERIFIED:** `DenseLifecycleProjection` lowers roots and `KeepsAlive` to local-ID adjacency and computes liveness without repeated external-ID B-tree traversal. Exhaustive three-node hostile checking covers every root mask and directed-edge mask against `LifecycleGraph::live_entities()`.
- **VERIFIED:** materialized Γ-QCN support now has local derivatives for both monotone directions handled separately. Pure deletion uses Pass52 stable-handle loss propagation. Pure insertion transports old keys, reads only inserted canonical endpoint keys from maintained quotient factors, resets the connected constraint component to full local support and monotonically prunes to its greatest fixed point.
- **HOSTILE:** deleting the only supporting value drives the maintained Γ-QCN fixed point to empty; reinserting it locally resurrects the dependent rows and produces a support state exactly equal to a fresh full rebuild.
- **FALLBACK:** mixed delete+insert deltas still use exact Replace. Missing/incompatible quotient factors, context mismatch or non-monotone handle transport also fall back to exact rebuild.
- **R&D REVIEW:** Program2 heavy-payload COW is already represented by Pass57. Its residual outer `BTreeMap` metadata clone remains benchmark-before-redesign rather than justification for a blind persistent-map rewrite. Program3 dense subtype/capability extents, LocalId-backed reference columns and incremental lifecycle/SCC maintenance remain OPEN.
- **AUTHORITY:** dense IDs, lifecycle projection, quotient factors and support masks are reconstructible physical evidence. `Revision=(S,Γ,M)` remains the sole semantic authority.

No logical identity, lifecycle, query, equality/order, transaction or durable-format law changed in Pass58.


## Pass59 — complete local Γ-QCN change-shape derivative and revision coalescing

Pass59 removes the remaining mixed-delta rebuild as the normal Γ-QCN support path. Stable row-handle transport now represents survivor, deletion and insertion identities in one mapping; changed quotient leaves reuse survivor keys, obtain only genuinely inserted keys from maintained factors, and recompute the greatest fixed point only for the causally connected constraint component. Pure deletion retains the cheaper monotone loss-only derivative. Exact full rebuild remains a fallback when the physical derivative preconditions cannot be established.

Runtime revision preparation also coalesces support maintenance across all relation mutations in one unpublished candidate. Base relations and ordinary derived physical families are first advanced to the complete target candidate; each affected Γ-QCN support binding is then maintained once against the complete set of physical changes. No incoherent intermediate support root is published and no new semantic authority is introduced.

Current derivative summary: pure delete, pure insert/resurrection and mixed delete+insert all have local exact Dq; multi-relation revisions coalesce those changes to one support transition per affected binding. Remaining work is finer per-key support accounting, structural/custom quotient factors, lifecycle/budgeting and the broader historical physical/durability frontier.


## Pass60 correction / dense-runtime closeout

**[VERIFIED]** Program3 is now integrated as one revision-local physical identity layer rather than isolated dense-ID benchmarks. `Revision` owns one shared `Arc<DenseEntityIds>`; validation compiles subtype-aware `DenseTypeExtents` against that map; lifecycle has an exact maintained dense least-fixed-point derivative; normalization uses reverse `LiveRefSensitivityIndex`; and typed physical execution admits `DenseLiveEntityIds` columns carrying the exact map needed to reconstruct external IDs.

**[NORM]** `EntityId` remains logical/durable identity. `LocalEntityId` is revision-local reconstructible physical identity and MUST NOT be serialized as stable semantic identity. Program4 algebraic-native layout remains R&D and is not part of this checkpoint.


# Pass61 implementation correction — structural Γ-QCN factors

**[VERIFIED]** Γ-QCN quotient factors are no longer restricted to primitive equivalence modules. The factor family is a dedicated reconstructible derivative keyed by exact `CanonicalEqKey`, with stable `PhysicalRowId` reverse mapping and pinned `SemanticContext`. Primitive and schema-declared structural equivalences are admitted through `SemanticRegistry::canonical_equivalence_key`; future/custom equality without an exact canonical representation is not silently approximated.

**[VERIFIED]** Ordinary relation deltas maintain these structural quotient factors atomically inside the existing candidate/COW transition boundary. Γ-QCN execution can reuse maintained structural factor keys before/after mixed deltas.

**[OPEN]** This does not define a durable encoding for recursive structural canonical keys. Long-lived persisted structural indexes still require an explicit key-format revision, semantic dependency closure, rebuild/migration policy and compatibility discipline.

# Pass62 implementation correction — algebraic native structural layout

**[VERIFIED]** The current structural value algebra now has an integrated reconstructible native physical family. Product, Sum, Option, Seq, Set, Bag, Map and guarded Mu/Var lower recursively into constructor-shaped columns while scalar leaves reuse existing native scalar families. `NativeRelation::typed_from_rows` may therefore mix scalar-specialized and algebraic columns without making the physical form semantic authority.

**[VERIFIED]** Structural `FilterEqConst` is no longer merely a pattern-specific fused specialization. The ordinary typed-batch compiler admits `Algebraic` predicates bound to exact pinned-Γ `CanonicalEqKey` values, so a structural filter can feed downstream typed stateful operators without mandatory full logical-row materialization. A composed Option/Seq/Set/Bag/Map/Sum hostile test requires native canonical keys to equal `SemanticRegistry::canonical_equivalence_key` exactly.

**[HOSTILE CORRECTION]** Public algebraic construction now validates `TypeExpr` before recursive descent. A non-empty unguarded `μX.X` is rejected rather than recursively expanding. Low-level algebraic mutation/canonical helpers are crate-internal so relation mutation continues through the authoritative PhysicalStore candidate/stable-handle boundary.

**[VERIFIED COEXISTENCE]** Program3 `DenseLiveEntityIds` remains a sibling physical family. Algebraic structural filtering and dense local-ID projection compose across an authoritative mixed remove+insert transition while preserving logical scan order and exact external-ID reconstruction. Nested LiveRef leaves inside an algebraic value currently use external IDs; automatic dense-local nested lowering remains a multi-family advisor choice.

**[PERFORMANCE EVIDENCE]** Five frozen release process runs give a Product child-field representation diagnostic median 41.834x (25.634–50.328x) over the boxed logical Product fixture and a Sum-tag median 1.806x (1.508–2.016x) over boxed logical Variant. These are fixture-specific representation measurements, not universal database speed claims.

**[OPEN]** Durable recursive structural-key encoding/version migration, persisted structural indexes, arbitrary/plugin canonical laws, structural ordering, memory-aware multi-family lifecycle and other layout families are not closed by Pass62.



# Pass63 implementation correction — multi-family inventory and retained-memory discipline

**[VERIFIED/PARTIAL]** PhysicalStore now inventories current reconstructible families through one typed artifact boundary and exposes deterministic, saturating retained-byte estimates. Relation layouts, shared dense identity backing, I64/semantic indexes, Γ-QCN factors/support and retained semantic statistics are visible to one memory report. Shared dense backing is deduplicated by root identity.

**[VERIFIED/PARTIAL]** Semantic-index lifecycle now has separate managed-index and global-store estimated-byte ceilings in addition to the historical key-cell/work proxy. Non-owned physical families count as fixed cost and cannot be silently evicted by the semantic-index advisor. Family-specific measured create/retain/evict/rebuild laws for I64/statistics/QCN/layout families remain OPEN; allocator/RSS truth and external pressure integration remain OPEN.

**[R&D FALSIFIED / NOT INTEGRATED]** Program7 correctly identifies snapshot-sized relation-data idempotency intents as a durability-space problem, but its `(source RevisionId, target RevisionId, delta, Γ)` replacement is insufficient for the current exact retry contract because RevisionId is not content-addressed and relation-data requests still carry an independent target Revision. A post-compaction same-ID/same-delta/different-target-content counterexample remains distinguishable by current production full target bytes but not by the Program7 candidate. Delta-native durability therefore remains OPEN pending a stronger authority/content-identity design.


# Pass64 implementation correction — Γ-QCN factor lifecycle

**[VERIFIED/PARTIAL]** Γ-QCN endpoint factors now participate in a conservative workload-driven lifecycle policy. For prepared plans whose current cost model already selects Γ-QCN, repeated-read evidence can create/rebuild/retain/reuse/evict exact canonical factors under the Pass63 global retained-byte budget. Only advisor-owned factors are evictable; explicit manual materialization transfers compatible factors to manual ownership without rebuilding them.

**[VERIFIED]** Multiway costing now recognizes compatible maintained quotient factors and removes their endpoint canonicalization work. Γ-QCN execution observes the same maintained factors; hostile coverage records 140 maintained quotient-key hits after advice while preserving the logical result. Stale manual semantic-index/factor replacement receives old-artifact byte credit before the replacement is charged, preventing false exact-fit global-budget rejection.

**[OPEN]** This is not yet counterfactual or write-aware lifecycle planning. The advisor does not build factors merely to make a currently rejected Γ-QCN path become preferable, and expected executions do not price future delta-maintenance work. Specialized I64/statistics/QCN-support/layout policies, autonomous telemetry and exact allocator/RSS integration remain part of the existing multi-family frontier.


# Pass65 implementation correction — source-bound incremental revision compiler

**[VERIFIED]** Relation-only revision construction now has a source-bound certified fast path. `RelationUpdateCandidate<'a>` is created only from one authoritative source `Revision`, exposes relation-row replacement, and consumes itself to build the target. The source is carried by borrow rather than supplied by the caller, so a candidate cannot be rebound to a different Revision while reusing that Revision's dense/type/lifecycle derivatives.

**[VERIFIED]** The certified path revalidates the complete pinned Γ registry, performs the same touched-row dangling-`LiveEntityRef` normalization as full `Revision::build`, validates only touched relations against retained `DenseTypeExtents`, recompiles only touched LiveRef sensitivity partitions, and reuses dense identity/lifecycle/type extents exactly. Relation-data WAL recovery uses this path instead of generic full revision construction.

**[VERIFIED]** Full construction no longer recompiles derivatives already produced by normalization: `normalize_certified()` returns final dense IDs and reverse LiveRef sensitivity, while `Revision` retains dense type extents. Reverse LiveRef sensitivity has shared field roots and `Arc` per-relation partitions so untouched relation payloads remain shared.

**[HOSTILE CORRECTION]** The R&D Program5 patch was not merged verbatim. Production review found and fixed cross-source candidate rebinding, missing pinned-Γ registry validation, and a mismatch with full normalization for touched rows containing dangling nested LiveRefs.

**[PERFORMANCE EVIDENCE]** On 40 Bag<I64> relations × 5,000 rows with one touched relation, seven release runs give full compiler median 3.407 ms versus hardened certified compiler median 0.294 ms (11.604x compiler-only). `RelationUpdateCandidate` still pays a separate full `DatabaseState::clone()` median 4.331 ms. Therefore compiler work is incremental, but logical snapshot construction is not.

**[SUPERSEDED BY PASS66]** Pass65 formalized full logical `DatabaseState` cloning as a frontier. Pass66 removes that full-payload clone with COW roots/per-relation sharing. End-to-end O(|Δ|) is still not claimed because relation-directory metadata and touched relation reconstruction are not yet a general persistent-map / typed derivative calculus. Compiled schema/Γ validation remains separate future work.


# Pass66 implementation correction — persistent logical COW and persisted-I64 lifecycle

**[VERIFIED]** Logical revision candidates no longer deep-clone the full `DatabaseState`. Carriers, fields and lifecycle are COW roots; the relation directory is COW and every relation row vector has an independent shared root. A cloned candidate shares all logical payloads until mutation, and a hostile pointer-identity test verifies that mutating one relation does not detach unrelated relation rows or other logical roots. This closes the Pass65-new clone-tax OPEN, but not a future persistent-tree/typed `D(Change)` program: first writes may still copy BTreeMap metadata and current relation-data application can reconstruct the touched relation as a whole.

**[VERIFIED/PARTIAL]** Persisted exact-I64 Join indexes become the third advisor-managed physical family after generic semantic indexes and Γ-QCN endpoint factors. Repeated Join evidence can create/retain/reuse/evict them under Pass63 byte budgets; manual install pins them outside advisor ownership; one-shot unamortized work is rejected before candidate build. Hostile review rejected Filter-derived advice because that executor did not consume persisted I64 state. Statistics, Γ-QCN support and layout-family lifecycle remain OPEN.

**[LEDGER]** Pass65 had 25 active OPEN only because it added the full logical-snapshot clone as a new item on top of the corrected historical 24. Pass66 closes that new item and introduces no new OPEN, returning the authoritative active ledger to **24**.


# Pass67 implementation correction — semantic-statistics lifecycle + persistent physical catalog roots

**[VERIFIED] Semantic-statistics lifecycle (bounded scope).** `MaterializedSemanticStatisticsState` is no longer manual-only for the direct primitive Join case where exact distinct-key cardinality changes the existing cost decision from transient-index construction to scan. The advisor uses the existing physical-artifact ownership and managed/global retained-byte budgets, evicts only owned statistics, treats explicit install as a manual pin, skips bindings already covered by exact persisted access artifacts, and publishes a new runtime root only when physical state changes. Multiway counterfactual statistics, histograms/correlation and autonomous telemetry remain OPEN.

**[VERIFIED] Persistent outer `PhysicalStore` catalogs.** Installed relations, persisted I64 indexes, semantic indexes, Γ-QCN endpoint factors/support, semantic statistics and the advisor-ownership set are now shared COW roots. `PhysicalStore::clone()` shares all directory metadata; relation-delta maintenance detaches only directories that actually contain affected bindings. This supersedes the Pass57 boundary that outer physical catalogs were ordinary value-owned `BTreeMap`s and fully closes the old historical physical-catalog OPEN. It does not claim that logical `DatabaseState` BTreeMap metadata or touched-relation reconstruction is a complete persistent-map / typed `D(Change)` solution.

**[VERIFIED ledger correction].** Pass66 had 24 historical active OPEN. Pass67 fully closes one of them — persistent outer physical artifact catalogs — with no genuinely new OPEN, leaving **23 active OPEN**.

# Pass68 implementation correction — exact delta-authoritative durability

**[NORM] Request authority determines durable retry evidence.** If an API accepts an independently constructed target `Revision`, exact durable idempotency must retain evidence sufficient to distinguish arbitrary target contents; a nominal `RevisionId` plus delta is not sufficient. If an API instead makes the typed delta authoritative and derives the target internally from one pinned source Revision, no independent target content exists and the canonical delta may be the exact request witness.

**[VERIFIED]** Pass68 implements both surfaces explicitly. `DurableRuntime::commit_revision` remains the legacy full-target exact path and retains the complete canonical target witness. `DurableRuntime::commit_derived_relation_data` accepts only source/target ids and typed relation mutations; the runtime derives and validates the target from the live authoritative source, then enters the normal prepare/seal/durable-COMMIT/publication boundary. Compact `RelationDataExact` is used only on this second surface.

**[VERIFIED]** The Pass63 falsifier is preserved as a regression: legacy same-transaction/same nominal id/same delta but different supplied target content conflicts even after checkpoint/compaction/reopen. The compact surface separately proves exact retry, changed-delta conflict, authoritative target equivalence and stale-source rejection. Relation mutation codec v5 and metadata codec v4 keep local backward decoders for prior formats.

**[PARTIAL / OPEN]** This fixes snapshot-sized relation-data idempotency entries but does not bound idempotency-history length. Transaction intent/outcome retention+GC remains OPEN, as do a general historical format-migration framework, streaming/chunked checkpoint metadata, group commit, replication, authenticated durable storage and machine-power-loss proof obligations.

**[LEDGER]** Pass67 had 23 active historical OPEN. Pass68 closes no complete historical ledger item and adds none; the authoritative count remains **23**.

# Pass69 implementation correction — stable structural canonical-key versioning

**[VERIFIED]** Γ-canonical keys now have an explicit stable recursive v1 byte grammar with fixed tags, deterministic lengths/order, big-endian numeric encoding, bounded fail-closed decode and a golden-byte regression. Unknown format revisions are incompatibility/rebuild events; they are never silently reinterpreted.

**[VERIFIED]** long-lived canonical-key physical artifacts bind to semantic revision + exact primitive/module dependency closure + key-format revision. Semantic indexes, Γ-QCN factors/support and semantic statistics share this contract. Schema-declared structural equivalence is admitted to the maintained semantic-index family through the same authoritative canonical-key law.

**[NON-CLAIM]** this is the key-format/cache migration boundary, not durable persistence of physical index payloads. Disk persistence/recovery of structural indexes, arbitrary/plugin semantic executable deployment and structural ordering remain OPEN.

**[LEDGER]** Pass68 had 23 active OPEN. Historical canonical-key/cache encoding-version migration/compatibility is fully closed in Pass69, no new OPEN is introduced, leaving **22 active OPEN**.

# Pass70 implementation correction — compiled Γ execution and exact structural cache binding

**[VERIFIED]** structural equality laws may be compiled from pinned Γ into a reconstructible node program containing resolved primitive contracts and direct Product/Option/Sum/Seq/Set/Bag/Map/guarded-Mu/Var edges. Γ-QCN factors, algebraic structural filters and structural semantic-index key parts use this executable derivative instead of repeated structural registry interpretation. Primitive key paths remain specialized.

**[VERIFIED / HOSTILE]** canonical-key cache compatibility is bound not only to nominal semantic revision and primitive module digests but also to the exact structural-definition closure. Two contexts with identical nominal revision IDs and the same primitive dependency set but different assignment of those laws inside a structural graph require `RebuildStructuralDefinitions`.

**[AUTHORITY]** neither compiled programs nor cache bindings define equality. They are certificates/derivatives of the pinned `SemanticContext` and installed certified semantic modules and can always be rebuilt.

**[OPEN]** durable structural-index payload persistence, arbitrary/plugin semantic executable deployment, structural ordering, revision-wide compiled-program sharing, general multiway planning and the remaining lifecycle/durability/formal frontiers remain separate work.

**[LEDGER]** Pass69 left 22 active historical OPEN. Pass70 closes no whole historical item and introduces no new one; authoritative count remains **22**.

# Pass71 implementation correction — durable physical artifact recipes

**[VERIFIED]** Checkpoint metadata can persist versioned logical recipes for reconstructible semantic indexes, Γ-QCN quotient factors and semantic statistics. Recovery rebuilds fresh physical payloads from authoritative `Revision=(S,Γ,M)` and pinned Γ; raw layout-local `PhysicalRowId` buckets are not durable authority.

**[VERIFIED]** Valid-but-semantically-stale recipes are dropped without blocking logical recovery. Corrupt/unknown recipe-format metadata fails closed. Equivalent recipes across layouts collapse deterministically with manual pinning dominating advisor ownership.

**[NON-CLAIM]** Specialized I64 index recovery is not closed: it requires a durable typed-columnar layout recipe. Raw physical payload persistence is also not claimed.

**[LEDGER]** Pass70 had 22 active historical OPEN. Pass71 closes no complete historical item and introduces no new one; the count remains **22**.

# Pass72 implementation correction — Quotient Hypergraph Engine

**[VERIFIED]** Γ-QCN support now maintains duplicate-safe live key support instead of repeatedly rediscovering viability by scanning every leaf bucket.

**[VERIFIED]** Fully covered GYO-reducible quotient hypergraphs can provide the QCN search order beyond eight leaves. For 3..=8 leaves the existing subset-DP cost gate remains authoritative; for >8 leaves cyclic/incomplete quotient systems fail back to the existing planner.

**[VERIFIED]** QCN enumeration is late-materialized: endpoint values are read by row handle, assignments carry ordinals, and full rows are materialized only after complete surviving assignments. Logical bag order/multiplicity remains exact.

**[HOSTILE FIX]** Program8 support-counter maps are included in retained-byte accounting so Pass63+ global physical-memory budgets remain conservative.

**[NON-CLAIM]** This is not arbitrary cyclic/general bushy planning, hypertree-width planning, or a universal QCN speedup.

**[LEDGER]** Pass71 had 22 active historical OPEN. Pass72 closes no complete historical item and introduces no new one; the count remains **22**.

# Pass73 implementation correction — bounded cyclic/non-GYO Γ-QCN

**[VERIFIED]** Fully-covered Γ-QCN programs are no longer required to be GYO-reducible in order to cross the historical >8-leaf boundary. When GYO reduction fails, the runtime may derive a deterministic min-fill cyclic search order and admit it only under a conservative work certificate.

**[VERIFIED]** The cyclic certificate is tied to the actual executor: it upper-bounds complete physical ordinal scans at every recursive depth plus final materialization, validates the search permutation before arithmetic, and uses saturating operations. Over-budget programs abort before DFS and fall back to the existing exact executor. All original Γ predicates remain checked on surviving assignments.

**[VERIFIED]** Semantic-quotient-factor advice uses the same bounded-cyclic admission law, so physical factors are not built for a cyclic QCN path that runtime would reject. Maintained Program8 quotient support remains exact across relation deltas.

**[NON-CLAIM]** This does not close general multiway planning. The bounded cyclic branch is not a worst-case-optimal join implementation, hypertree-width planner, or unrestricted complexity guarantee. The historical active ledger therefore remains **22**.

# Pass74 implementation correction — cyclic prefix indexing + durable typed-layout recovery

**[VERIFIED]** bounded cyclic/non-GYO Γ-QCN execution may build deterministic prefix candidate indexes keyed by the joint exact `CanonicalEqKey` signature of already-bound quotient constraints. Prefix-index build/use participates in the bounded work certificate; original pinned-Γ predicates remain authoritative and surviving assignments are still validated. Sparse cyclic execution therefore avoids repeated broad ordinal scans without changing bag order or multiplicity.

**[VERIFIED]** physical artifact recipe format v2 can persist logical reconstruction requests for supported relation layouts (`RowStore`, value-columnar, I64-columnar, typed-columnar) and exact I64 indexes. Recovery rebuilds these from the authoritative recovered `Revision=(S,Γ,M)`; no `PhysicalRowId`, local dense ID, pointer or revision-local dense table is durable authority.

**[VERIFIED]** typed `LiveEntityRef` recovery binds columns to the recovered Revision's fresh dense identity table while preserving external entity identity. Stale or contradictory physical layout recipes are discarded as advice and deterministically fall back to `RECOVERY_ROW_STORE`. Recipe v1 remains decodable; unsupported future recipe format revisions fail closed.

**[NON-CLAIM]** this does not define universal recovery economics for arbitrary future physical families, a general historical durable-format migration calculus, streaming checkpoints, or a machine/filesystem power-loss proof. Cyclic prefix indexing is also not a worst-case-optimal/general hypertree-width join algorithm.

**[LEDGER]** Pass73 had 22 active historical OPEN. Pass74 closes no whole historical item and adds none; authoritative count remains **22**.

# Pass75 implementation correction — bounded recovery economics for advisor-owned derivatives

**[VERIFIED]** durable reopen has an explicit `PhysicalRecoveryPolicy`. Relation layouts and manually pinned durable recipes remain fixed recovery intent. Advisor-owned I64 indexes, semantic indexes, Γ-QCN quotient factors and semantic statistics are admitted under deterministic row/key-part evaluation and estimated retained-byte ceilings; skipping them cannot change recovered logical state.

**[HOSTILE FIX]** an earlier candidate policy allowed a global recovery budget to skip manual pins. That was rejected because a later checkpoint could then erase explicit durable physical intent. Production semantics instead match the existing lifecycle law: fixed/manual state is reconstructed first and only advisor-owned derivatives are budget-evictable.

**[VERIFIED]** `PhysicalRecoveryReport` makes stale/incompatible optional-artifact drops and budget skips observable. `DurableRuntimeSupervisor` retains the configured recovery policy and applies it on every explicit or fail-stop-triggered reopen, so automatic recovery cannot silently fall back to unlimited eager reconstruction.

**[NON-CLAIM]** row/key-part evaluation count is not a CPU bound for recursively large structural canonical keys, and estimated retained bytes are not allocator/RSS truth. Pass75 also does not provide benefit-ranked/background rebuild scheduling or a universal fast reconstruction format.

**[LEDGER]** Pass74 had 22 active historical OPEN. Pass75 closes concrete eager-rebuild policy/observability defects but not the complete historical recovery-economics or multi-family-lifecycle items; authoritative historical count remains **22**.

# Pass76 implementation correction — continuable work-aware recovery

**[VERIFIED]** bounded physical recovery no longer allocates advisor key-work budget according to durable recipe or `SemanticId` ordering. Compatible manual/fixed recipes remain first; compatible advisor recipes are scheduled by increasing deterministic rebuild key-evaluation work with stable identity only as a tie-breaker.

**[VERIFIED]** budget-skipped compatible advisor recipes can be retried after the runtime starts serving. Continuation rebuilds against the current authoritative `Revision=(S,Γ,M)` and pinned Γ, publishes a new immutable physical root only when reconstruction actually changes physical state, and never changes logical Revision authority. The supervisor exposes the same continuation boundary without requiring another reopen.

**[HOSTILE]** a fabricated continuation report cannot replay manual physical intent, and an incompatible advisor recipe remains a fail-open derived-state drop. A cheap advisor rebuild wins a constrained key-work budget even when an expensive recipe has the lexicographically earlier semantic identity.

**[NON-CLAIM]** this is not benefit-ranked autonomous/background scheduling. Recovery recipes do not yet persist workload-benefit evidence; structural canonical-key size is not priced into the current key-evaluation proxy, and retained-byte accounting is not allocator/RSS truth.

**[LEDGER]** Pass75 left 22 active historical OPEN. Pass76 closes two narrower recovery-policy defects but no whole historical item and introduces no new one; authoritative count remains **22**.


# Pass77 normative/implementation correction — semantic observables and Γ Anchor-Pullback normal form

**[NORM] Observable boundary.** A query-local semantic coordinate is an exact observable under pinned Γ. Its runtime realization may use `RevisionObservableId` / `EqClassId`, but those IDs are reconstructible nominal coordinates bound to one `SemanticRevision` and one catalog realization. They are never durable semantic identity.

**[NORM] Product observable.** Finite conjunction of observables is represented by the product observable. Hyper-determinants `(A,B,...) -> (C,D,...)` are therefore ordinary deterministic morphisms between finite products; the kernel must not introduce a unary-only determinant architecture.

**[NORM] Unordered structural finite measure.** `Set`, `Bag` and `Map` canonical equality under coarser Γ semantics use an ordered finite counting-measure normal form. Coarse canonical atoms are aggregated with explicit multiplicity. Bag stored multiplicity remains semantic payload of its atom while measure multiplicity records collisions between physical entries.

**[VERIFIED]** Production canonical-key codec v2 and semantic-index compatibility implement this law and fail closed across encoding revision drift.

**[NORM] Determinant normal form.** The semantic deterministic theory is the least closure operator `Cl_D` induced by certified finite-support morphisms. It is extensive, monotone, idempotent and independent of fair firing order. A minimum/canonical FD list is not semantic authority. Closure-redundant direct/composed morphisms remain valid reconstructible accelerators.

**[VERIFIED]** `DeterminantTheory` implements incidence/worklist closure and exact value propagation. Stable reverse-delete produces an inclusion-minimal generator relative to the query-local coordinate order; no minimum-cardinality claim is made.

**[NORM] Finite-factor anchor factorization.** Every finite weighted semantic factor may choose any coordinate subset whose projection is injective on distinct support. The pushforward onto that basis is an anchor measure and the omitted coordinates are reconstructed by one exact partial morphism. Full coordinates are always a valid basis, so the representation is total for finite factors. Bag multiplicity is carried by the anchor measure.

**[VERIFIED]** `RevisionFiniteMeasure` / `AnchorMeasureState` implement weighted lossless reconstruction and safe n-ary→m-ary determinant derivation from factor support.

**[NORM] Γ Anchor-Pullback Normal Form.** A positive finite multiway component is semantically representable as finite anchor measures + deterministic reconstruction/morphism closure + compatibility over shared exact observables + deterministic output pushforward. The residual object is the compatible anchor pullback, not a binary join tree. GYO/min-fill/QCN/WCOJ may be retained as physical residual strategies but are not the semantic definition of general multiway correctness.

**[VERIFIED/PARTIAL]** `AnchorPullbackNormalForm` exists as a query-local semantic substrate and can issue a determinant-backed branch-free certificate when one concrete anchor closes the whole coordinate set. General `RelExpr -> APNF` lowering, exact residual fiber-profile materialization, the residual pullback executor, incremental maintenance, durable recipes and crossover policy are still OPEN.

**[LEDGER]** Pass76 had 22 active historical OPEN. Pass77 closes several substrate/correctness defects but no complete historical item; authoritative count remains **22**.


# Pass78 normative/implementation correction — semantic work accounting + support-atom fabric

**[NORM] Semantic work metric.** Deterministic resource admission may count logical semantic nodes and stable payload bytes, but such a metric is not CPU time, allocator usage or RSS and cannot become semantic authority.

**[VERIFIED]** `SemanticWorkEstimate` is layout-independent across logical values/canonical keys and supported native/algebraic storage. Recovery separates key-evaluation, semantic-work and retained-byte budgets; manual durable intent is never advisor-budget-evicted.

**[NORM] Support-atom partition.** For one finite relation support and a finite active observable family `Q`, the product observable induces a partition into support atoms. The product class is the atom identity. For every coordinate observable, its exact row fiber is the disjoint union of atom fibers whose product classes project to that coordinate class.

**[VERIFIED]** `SupportAtomFabric<RowId>` realizes that partition using catalog-local `EqClassId`. `MaterializedObservableAtomState` instantiates it over `PhysicalRowId`, maintains joint/projected fibers and counts, and carries a certified product projection. These are reconstructible physical derivatives of pinned Γ and current authoritative relation rows; revision-local IDs are never durable semantic identity.

**[NORM] Migration law.** A common observable materialization substrate does not justify deleting specialized representations immediately. Legacy semantic indexes/statistics/quotient/operator state may remain as parity oracles and specialized lowerings until adapters, global capability selection, durability/recovery and crossover evidence are complete.

**[PARTIAL / OPEN]** unified `ObservableDemand` advising, exact/pruned Pareto capability selection, durable SAMF recipes, ordered/annotation/I64 overlays, generic differential maintenance compilation and retirement of legacy families remain OPEN inside historical physical-lifecycle work.

**[R&D ORIENTATION]** Γ-GCC proposes grounded least finite hyperrule closure as a common semantic primitive for determinant saturation, lifecycle reachability and future positive recursive support. It is not yet production authority. Initial recursion must remain restricted to certified finite-height/idempotent carriers.

**[LEDGER]** Pass77 had 22 active historical OPEN. Pass78 closes narrower resource/substrate defects but no complete historical item; authoritative count remains **22**.

# Pass79 implementation status — grounded finite closure

**[VERIFIED]** CFMD now has a common finite grounded-closure substrate. A compiled program consists of finite atoms, grounded seeds and finite n-ary hyperrules. Its meaning is the least closure, not an arbitrary closed set.

**[CERTIFICATE LAW]** Every live non-seed atom has one selected enabled rule witness whose premises have strictly smaller finite ranks. Together with seed inclusion and rule closure this characterizes the exact least grounded closure and rejects self-supporting groundless cycles.

**[DYNAMIC LAW]** Removal invalidates only descendants in the selected witness graph; the affected witness cone is locally re-solved against the preserved outside closure, allowing alternative proofs to be rediscovered. Non-selected redundant-rule deletion can be a zero-invalidation operation.

**[CURRENT CONSUMERS]** APNF determinant closure and `kernel-fixpoint` reachability compile to this substrate. Dense lifecycle remains a specialized implementation but is parity-pinned to the same unary closure law.

**[NON-CLAIM]** This does not authorize unrestricted recursive Bag/Natural semantics. Initial recursive-query use must have a certified finite/idempotent support carrier and compile semantic tuple classes/rule instances through the Γ observable/APNF/SAMF direction.

**[LEDGER]** Pass78 left 22 active historical OPEN. Pass79 removes duplicated grounded-closure semantics but closes no whole historical ledger item; authoritative count remains **22**.


# Pass80 normative/implementation correction — convergence executor and runtime calculus

**[VERIFIED] General finite multiway execution.** The current finite nonrecursive relational fragment lowers prepared multiway equality constraints to query-local Γ observable coordinates, per-leaf finite measures, APNF anchor/reconstruction state and a residual compatibility pullback executor. Query coordinates are nominally distinct from semantic law identity, so independent variables using the same equivalence law do not collapse. Physical expansion preserves exact Bag multiplicity and logical order. Specialized QCN/GYO/index paths remain optional physical lowerings.

**[VERIFIED] SAMF consumption and durability.** Exact support-atom fibers may directly serve semantic Filter/Join access, statistics and QCN quotient reads. The materialized state pins exact semantic-key implementation binding and fails closed on Γ drift. `ObservableAtom` is a durable physical recipe and rebuilds after reopen; legacy artifact families remain parity/specialized implementations until annotation/ordered overlays and unified advising are complete.

**[VERIFIED] Differential/fixed-point/validity/observation chain.** `RelDifferentialProgram` is a pinned maintenance contract owned by maintained plans; Γ-BFC/GCC structurally repairs QCN support after insertion/deletion/resurrection; runtime candidates carry an exact revision-bound Γ-VMF violation state and cannot seal unless `V=0`; runtime observations are root/revision-bound Γ-OFC guards whose exact impact is evaluated through pinned Γ-DTC. These derivatives do not replace Revision authority.

**[VERIFIED] Capability obligations.** `CapabilityDef.required_fields` is an enforced interface obligation over actual implementation types and members rather than inert metadata.

**[PARTIAL / OPEN]** positive recursive query/PWRC production, SAMF Annotation/Ordered overlays and unified advisor, generic generated DTC maintenance, generic VMF invariant compilation, bounded OFC repair and the deferred FineChange/Rewrite/Lens write path remain production OPEN.

**[R&D CLASSIFICATION]** Later R&D has closed the architecture/theorem level for FineChange/Rewrite, dependent Lens/complements, non-monotone query, structural ordering, CertifiedFn, generic folds, access/release policy, subscriptions/coreference reduction, durable format migration, group-commit contract, REIC-aware replication boundary, authenticated durability/anti-rollback, distributed erasure and semantic module deployment. These are not Pass80 production claims.

**[LEDGER]** Pass79 left 22 compound historical OPEN. Pass80 closes major subproblems but no entire compound row; authoritative historical count remains **22**.

# Pass81 normative/implementation correction — unified write calculus convergence

**[NORM] Rewrite identity is not endpoint identity.** A write is identified by its `RewriteSpec`, law-set identity and explicit semantic intent. Equal resulting values never justify collapsing distinct intents. Fine changes remain extensional effects; intent remains first-class authority.

**[NORM] Writable-view inversion is certified, not guessed.** Project/Filter/Join write-through may reconstruct source state only through explicit dependent complements, planner-owned Γ coordinates, APNF determinant evidence, or an explicit constructor authority. Physical row handles and host equality/hash/order cannot supply missing logical information.

**[VERIFIED]** Pass81 production integrates structural and relational writable plans, Γ-aware collection/relation changes, guarded durable publication, lossy Project reconstruction/constructors, owner-side Join reconstruction, migration complements/historical structural restore, and Γ-REIC residual/coherence/multi-parent resolution for the currently supported bounded concurrent frontier.

**[NORM] Stable sequence intent.** Snapshot-local `SeqSplice(start, delete_count, insert)` is an execution/derivative representation, not durable concurrent intent. Canonical sequence Rewrite intent addresses stable semantic occurrence identities and stable gap anchors bound to retained anchor history. Missing occurrences, expired anchor history and stale gaps fail closed. Same-gap concurrent insertions require an explicit ordering policy; conflicting rewrites of one occurrence are not resolved by final-value equality.

**[VERIFIED]** `SeqOccurrenceId`, `StableSeqGapAnchor`, `StableSeqRewriteIntent`, stable snapshot resolution, `SeqOccurrence`/`SeqAnchor` Rewrite coordinates, conservative sequence footprints and `RewriteSpec::prepare_stable_seq` implement that boundary. Stable anchored intent survives unrelated index drift and is retained independently of the derived extensional sequence endpoint.

**[PASS81 CLOSEOUT]** The post-AY canonical audit found stable sequence intent to be the final missing contract from the closed write-R&D integration set. Checkpoint AZ closes it. Pass81 therefore declares the write-R&D production integration branch converged. This does not close the separate historical 22-item architecture/systems backlog; durable branch-DAG ingestion, broader plugin execution, wider antichain generalization and other deferred systems work remain later-pass concerns unless a concrete production consumer promotes them.

---

# Pass82 addendum — historical physical/read convergence rebase

**Status:** verified production integration over final Pass81/AZ.

Pass82 starts the post-Pass81 historical ledger rebase. It does not reopen the converged write calculus.

## Historical #1 — structural/custom semantic physical persistence and ordering — [VERIFIED/CLOSED]

The existing Γ-owned `StructuralOrderingDef`/canonical semantic order-class machinery, durable checkpoint/reopen metadata, and structural physical parity are now joined by a SAMF `SupportAtomOrderedOverlay`. The overlay is revision/catalog/product-bound and rejects any order key that splits one Γ-equality atom. `SupportAtomAnnotationOverlay` is also present and removes annotation state for dead atoms. Physical tie order remains non-semantic; WITH TIES continues to depend only on semantic order classes.

## Historical #3 — unified physical lifecycle/materialization ontology — [VERIFIED/CLOSED]

Production optional-artifact ownership now uses one internal `UnifiedArtifactId` across I64 index, semantic index, ObservableAtom/SAMF, quotient factor/support, and semantic statistics, with a shared family-neutral `PhysicalCapability` vocabulary and shared admission kernel. `PhysicalStore::converge_observable_atom_candidate` validates/installs the replacement SAMF candidate before retiring advisor-owned legacy semantic index/statistics/quotient-factor duplicates. Manual pins are preserved. Legacy Group/TopK and other specialized states remain permitted physical lowerings rather than semantic authorities.

## Historical #5 — shared resource/memory pressure — [VERIFIED/PARTIAL]

`ResourceFootprint` is an exact weighted union over declared kernel-owned resource atoms: identical shared atoms are charged once and inconsistent weights fail closed. OS/process memory pressure is modeled separately by `PhysicalPressureSample`/`PhysicalPressurePolicy`; it is never presented as exact per-artifact RSS attribution. The selector can reject optional builds under external pressure while preserving manual state. Production runtime telemetry/plumbing of real pressure samples is intentionally deferred to historical #4's unified advisor controller, so #5 is not yet marked fully closed.

## Next historical frontier

Historical #4 is next: autonomous deterministic telemetry/correlation/decay/hysteresis scheduling. It must feed actual read-saved/write-maintenance/rebuild work plus external pressure into the existing unified selector without becoming semantic authority. Completing this consumer also completes #5's remaining runtime pressure-plumbing gap. Historical #7 (recovery economics / durable semantic-core rehydration) follows #4/#5.

# Pass83 addendum — autonomous physical policy and bounded recovery economics

**Status:** verified production integration over Pass82.

## Historical #4 — autonomous physical advisor — [VERIFIED/CLOSED]

Physical lifecycle policy now has explicit reconstructible telemetry rather than implicit counters or semantic state. `ArtifactTelemetry` records read work saved, write-maintenance work and rebuild work; `AdvisorTelemetry` aggregates deterministically; `TelemetryDecayPolicy` applies integer epoch decay. `UnifiedAdvisorController` consumes workload evidence and external pressure, feeds the existing common admission selector, and publishes physical changes only through the immutable runtime root. Telemetry is never part of `Revision=(S,Γ,M)` and may be lost without changing exact query results.

Install/retain hysteresis is first-class. Failed maintenance publication does not decay telemetry, so retries do not silently change controller history. Manual/fixed artifacts are not evicted by external pressure.

## Historical #5 — shared resources and memory pressure — [VERIFIED/CLOSED]

The Pass82 weighted shared-resource union remains the exact kernel-owned accounting boundary. Pass83 connects its separate `PhysicalPressureSample` envelope to the production controller, closing the prior consumer gap. Host/process RSS remains an external observation and is intentionally not attributed exactly to individual artifacts.

## Historical #7 — recovery economics — [VERIFIED/PARTIAL]

Recovery scheduling now uses the common capability/work vocabulary. Initial reopen does not depend on ephemeral telemetry; manual recipes retain priority. Once serving, deferred advisor-owned recipes can be resumed through the controller and ranked by observed net read benefit per deterministic structural rebuild work, while existing key-evaluation, semantic-work and byte budgets remain hard limits.

This is not yet whole-row closure. The verified R&D semantic-core path has not been rebased: durable ObservableAtom canonical-key cores by durable occurrence ordinal, checkpoint rehydration with fresh process handles, exact pinned-Γ WAL-tail core replay, and stale-core fallback remain Pass84 work.

## Historical ledger state

Whole rows closed in production after Pass83: **#1, #3, #4, #5**. Historical #7 is partial. The next whole-row target is #7; positive recursive Bag/PWRC #2 follows it.

# Pass84 normative/implementation correction — durable semantic-core recovery

**[NORM] Durable physical semantic core.** A reconstructible physical artifact may persist a compact semantic core beside its durable recipe, but that core is never a second logical state. It is pinned to the checkpoint `Revision`, addresses semantic structure through durable semantic coordinates, contains no process-local `PhysicalRowId`, and may be discarded without affecting logical recovery.

**[VERIFIED] ObservableAtom durable core.** `DurableArtifactCore::ObservableAtom` persists one canonical Γ-equivalence key tuple per durable relation occurrence ordinal, together with relation/key-part identity and source revision. Durable metadata codec v11 stores these cores; prior metadata versions remain readable with an empty-core interpretation.

**[VERIFIED] Fresh-handle rehydration.** On reopen, the authoritative checkpoint relation is reconstructed first. A compatible ObservableAtom core then rebuilds its `RevisionObservableCatalog` / support-atom fabric against the newly recovered `PhysicalRowId` handles. Core admission consumes retained-byte budget but not semantic key-rebuild work; a stale, malformed or incompatible core is dropped and the ordinary exact rebuild path remains available.

**[VERIFIED] Exact WAL-tail core replay.** Before rehydration, checkpoint cores are replayed through the committed WAL prefix. Relation removals use the same pinned-Γ row equivalence and first-match occurrence order as logical relation replay; insertions derive canonical keys through the pinned semantic-index binding. A schema/Γ/full-revision transition invalidates the core and forces rebuild rather than transporting it speculatively.

**[VERIFIED] Recovery equivalence boundary.** Hostile coverage checks zero-rebuild-work checkpoint rehydration, WAL-tail rehydration, coarse-Γ first-match removal parity, stale-core fallback and preservation of authoritative target Revision. Initial recovery remains independent of process-local advisor telemetry; Pass83 telemetry-aware deferred recovery remains an economics layer above this semantic-core substrate.

**[LEDGER]** Historical #7 recovery rebuild economics is now **PROD CLOSED**. Together with #1/#3/#4/#5, five compound historical rows are production-closed. The next whole-row integration target is #2 positive recursive Bag execution/PWRC. Pass84 deliberately does not partially copy that three-crate R&D branch after the source cutoff.



# Pass85 normative/implementation correction — positive recursive Bag / PWRC

**[NORM] Positive recursion is compact N∞ semantics.** After a certified finite grounding, positive recursive Bag evaluation is represented as a finite carrier plus positive rules. The least-support computation distinguishes ungrounded zero support from grounded productive recursion. Proof-tree multiplicity is exact in `N∞ = N ∪ {∞}`: finite values use arbitrary-precision naturals, a productive grounded strongly connected component denotes `∞`, and infinity propagates only through live positive dependencies.

**[NORM] Bag multiplicity is structural, not row expansion.** Repeated recursive premises are multiplicative and therefore semantically observable. Runtime execution returns compact `(row, N∞)` entries. A consumer that requires a materialized finite Bag must reject an infinite multiplicity explicitly with `NonFiniteRecursiveMultiplicity`; it must never loop, saturate, or silently truncate.

**[NORM] Recursive physical plans remain Γ-pinned.** `PreparedPositiveRecursivePlan` is compiled against one exact `SemanticContext` and rejects execution under Γ drift. APNF/SAMF or another certified grounding owns carrier/rule construction; `kernel-fixpoint` owns least-support and proof-tree multiplicity; the physical plan does not make its carrier a second logical authority.

**[HOSTILE] Required closure properties.** Finite programs must agree with independent DAG/oracle evaluation; duplicate premises must preserve Bag multiplicity; productive and ungrounded cycles must be distinguished; a dead conjunctive premise must prevent false productive-cycle classification; finite counts must not overflow into infinity; SCC traversal must not depend on recursive call-stack depth; atoms outside the certified carrier and Γ drift must fail closed.

**[HISTORICAL] Historical problem #2 is PROD CLOSED at Pass85.** Historical #11 remains OPEN. Its old portable epoch/horizon reference is not directly safe on the post-Pass81 architecture because Γ-REIC currently derives `RevisionEffectId` from raw transaction id and validates causal effects through retained exact transaction intents. A correct production rebase must separate retry identity `(IdempotencyEpoch, ClientTransactionId)` from causal event identity, make causal events self-contained across retry-payload GC, and preserve crash-before-checkpoint replay when a numeric transaction id is reused in a new epoch.

# Pass86 addendum — durable idempotency epochs and bounded exact retry history

Pass86 closes historical problem #11 without weakening the post-Pass81 Γ-REIC authority model.

## Retry identity

Exact retry identity is the pair `(IdempotencyEpoch, ClientTransactionId)`. A raw client transaction id is never globally unique across epochs. The durable store owns `current_idempotency_epoch` and `minimum_retry_epoch`.

A retry in an epoch below `minimum_retry_epoch` returns `RetryHistoryExpired`; it must never be treated as an unknown/new transaction. Advancing the current epoch is monotone. Expiring retry history removes exact retry-ledger payloads below the published minimum horizon.

## Causal identity is separate from retry identity

Γ-REIC causal effects use an independent `RevisionEffectId`. The store allocates this identity before durable prepare and persists it in the WAL prepare descriptor. Recovery therefore reconstructs exactly the same causal event identity even after a crash before the next checkpoint.

Causal records retain their own canonical `DurableTransactionIntent`; Γ-REIC validation and ideals do not dereference the GC-able retry ledger. Retry-history retention and causal-history retention are separate concerns.

## Durable metadata / WAL compatibility

Metadata codec v12 persists current/minimum idempotency epochs, epoch-qualified retry entries, and self-contained causal records. Mutation/WAL codec v9 persists epoch and optional causal effect identity for newly bound prepares while retaining legacy decoding paths. Old zero-epoch records remain readable.

## Required invariants

1. Same raw transaction id may be committed in different epochs without aliasing retry or causal identity.
2. Same `(epoch, transaction_id)` with a conflicting intent remains a conflict.
3. Crash after raw-id reuse in a new epoch but before checkpoint must recover both exact outcomes and distinct causal effects.
4. GC below the retry watermark removes exact retry payloads but not causal meaning.
5. A retry below the watermark is `RetryHistoryExpired`, never `Unknown` and never executable as a fresh request.
6. The retry watermark becomes durable through normal checkpoint publication.
