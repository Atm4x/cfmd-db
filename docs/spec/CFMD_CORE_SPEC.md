# CFMD — актуальная спецификация идеальной математической БД

**Статус документа:** нормативный ориентир проекта с append-only implementation journal; verified workspace baseline through **Pass280** (2026-09-28), включая global kernel hostile/refactor closeout.

**Назначение:** этот файл можно отдавать другому агенту как самостоятельное описание того, **что именно строится**, какие свойства являются частью идеальной семантики, какие уже подтверждены кодом/тестами, какие физические решения временные, и какие проблемы остаются открытыми.

**Как читать текущий статус:** pass-секции ниже являются append-only историей и сохраняют формулировки `[OPEN]/[NEXT]`, актуальные на момент соответствующего pass. Они не должны интерпретироваться как текущий backlog без сверки с более поздними addendum. Текущий kernel status задаётся Pass280 global closeout и `docs/status/KERNEL_HOSTILE_LEDGER.md`; текущий product backlog задаётся `docs/api/PRODUCT_ROADMAP.md`.

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

**[VERIFIED]** Полный surface→kernel preservation theorem механизирован в `formal/lean/CFMD/SurfaceKernel.lean`; `check_surface_refinement.py` fail-closed привязывает normative surface table, `TypeExpr`, полный текущий `RelExpr`/`Plan` vocabulary и production `LoweringChecker` к theorem artifact.

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

**[VERIFIED, Pass256]** Multi-intent resolution may prepare one `PreparedStableSeqSnapshot` that validates exact occurrence-ID uniqueness once and reuses a deterministic occurrence-to-index map for all subsequent intent resolutions against that snapshot. Stable-sequence preparation fails closed when the selected `RewriteSpec` footprint does not conservatively cover the canonical footprint derived from the concrete intent; compiled coordination may therefore never obtain authority from an underdeclared stable-sequence footprint.

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


# Pass87 addendum — restart authority classification and group durability

Pass87 closes historical problems #14 and #15 on top of the Pass86 durability model.

## Authority-uncertainty restart law

Checkpoint failure is classified by whether authoritative publication may have changed. Failure while checkpoint/shadow/materialization work is still provably unpublished leaves the previous durable root authoritative and serving may continue. Once manifest rename is attempted, publication outcome may be authoritative; any error from that boundary onward requires reopen before further mutation. Active WAL durability uncertainty remains fail-stop.

A poisoned process-local runtime lock is not authority. `DurableRuntimeSupervisor` reconstructs the runtime from durable authority, clears the process poison, and resumes from the recovered root. No stale in-memory runtime may survive that recovery. Derived physical state remains disposable/rebuildable.

## Barrier-safe group commit law

A valid group is an ordered contiguous revision chain. Group commit performs: append every PREPARE without an acknowledgement barrier; one prepare durability barrier; append every COMMIT; one commit durability barrier; only then publish receipts and advance in-memory committed authority. Noncontiguous chains and duplicate transaction identities are rejected before WAL publication.

Every descriptor continues to use Pass86 epoch-qualified retry identity and independent WAL-persisted causal event identity. Grouping changes durability amortization only; it does not merge transaction identities, revision effects, or Γ-REIC events.

`DurableCommitBatcher` is reconstructible scheduling state, never semantic or durable authority. Enqueue returns only `Queued` or `FlushRequired`, never a commit receipt. A failed flush retains the exact pending descriptors for retry/recovery handling. Capacity, deadline, or explicit scheduling may all invoke the same `flush` boundary; no wall-clock scheduler state is authoritative.

## Closure invariants

1. Errors before manifest publication keep the previous root usable; errors at/after publication uncertainty require recovery.
2. Runtime mutex poison is repaired only by reopening durable authority.
3. No group receipt exists before the final commit durability barrier.
4. Reopen reconstructs the same committed group outcomes.
5. Group commit rejects invalid revision chains before durable append.
6. Async batching cannot acknowledge queued work and retains pending descriptors after a failed flush.

Historical #14 and #15 are **PROD CLOSED**. The next production target is #19 bounded repair with finite candidate frontier, VMF/OFC verification, and verified cross-context observation transport.

---

# Pass88 normative addendum — bounded repair and canonical durable-state migration

## Repair is verification, not authority

A repair search is a bounded search over an untrusted finite `RepairCandidateProvider`. Candidate generation is policy. Acceptance remains semantic authority:

1. prepare the candidate through the ordinary revision-transition path;
2. require VMF validity (`Viol = 0`);
3. require preservation of the guarded observation fiber through OFC;
4. when source and target semantic contexts differ, require a verified `kernel-transport` witness and compare the transported source observation/query in the target context;
5. never interpret transport as permission to change the guarded observation.

The result surface is total over the bounded search: `NoRepair`, one unique `Prepared` transition, `Ambiguous`, or `BudgetExceeded`. A cross-context candidate without a verified witness is rejected. Rewrite/WritableLens may supply candidates, but repair does not become a second rewrite authority.

## Canonical durable-state boundary

Durable format evolution is defined through one canonical recovered authority, not chained byte-to-byte migrations. Historical component codecs remain component-local, but every supported durable generation must decode and reconcile into `CanonicalDurableState` before runtime publication.

The migration law is:

`decode historical generation -> validate/reconcile canonical durable state -> encode current formats -> publish a fresh immutable generation`.

Migration must not mutate the old generation in place. Missing historical fields may receive defaults only where the historical format had one unique semantic interpretation.

Unsupported format at the highest published generation is a typed failure. Recovery must never fall back to an older generation merely because the newest published generation uses an unsupported format; such fallback would be semantic rollback.

The current migration boundary covers checkpoint/metadata/manifest authority and preserves the recovered revision, semantic registry, materialization/physical specifications, durable artifact cores, migration complements, retry epochs/history, causal coverage root and revision-effect state. WAL retains its own versioned codec; this is intentional physical-codec separation, not a second durable authority.

Real filesystem/controller power-loss assurance is not implied by this addendum and remains historical #13.

---

# Pass89 authoritative addendum — streaming checkpoint cut/tail protocol

## Historical #12 production closure

Historical problem #12 is **PROD CLOSED** at the durability-correctness/protocol boundary.

A streaming checkpoint is no longer a monolithic stop-the-world generation switch. Production now exposes an unpublished `StreamingCheckpointJob` over one immutable cut revision **H**. The job writes a chunked checkpoint root while ordinary durable commits may continue. The old published generation and active WAL remain the sole authority until the new manifest is published.

### Cut and cross-cut transaction rule

At `begin_streaming_checkpoint`, the store barriers the current authoritative WAL, pins the current durable revision as cut **H**, and snapshots every unresolved exact PREPARE into a `PreparedCutCapsule`. Each capsule entry preserves its prepare LSN, payload CRC and exact durable prepare descriptor. Therefore a transaction prepared before H and committed after H remains reconstructible from `checkpoint(H) + capsule + shadow tail`; it is not silently lost at the cut.

### Exact shadow-tail rule

The unpublished shadow WAL starts at the same next LSN as the authoritative WAL at the cut. Every subsequent PREPARE/COMMIT frame appended through `DurableRevisionStore` is mirrored into the shadow as the **same encoded frame bytes and same LSN**. The job tracks both `mirrored_lsn` and `durable_shadow_lsn`; shadow failure aborts only the unpublished job and never changes the success semantics of the authoritative active WAL.

### Chunk-root rule

Streaming checkpoints use checkpoint format v2: an ordered root identifies the cut revision, canonical logical stream length, ordered chunk ordinals/lengths/CRC32C values and descriptor checksum. Chunk payloads are written and synced independently. Reopen accepts a v2 root only when all descriptors and chunks validate and the reconstructed logical checkpoint decodes to the pinned cut revision. Legacy monolithic checkpoint v1 remains readable.

### Publication certificate

Manifest format v3 binds:

- immutable generation id;
- cut revision **H**;
- exact publication endpoint **E**;
- first shadow-tail LSN;
- certified published tail LSN;
- checkpoint-root checksum;
- metadata checksum;
- prepared-capsule checksum.

Before publication, finalization barriers the authoritative WAL and the shadow WAL, requires `mirrored_lsn == active.last_lsn` and `durable_shadow_lsn == mirrored_lsn`, replays the shadow from the cut capsule, and requires exact recovery to the current durable head **E**. It then revalidates checkpoint root, metadata and capsule and publishes the new immutable manifest through the existing authority-uncertainty boundary. Only after successful manifest publication does the shadow become the active WAL in memory.

Reopen treats `(H, E, first_tail_lsn, published_tail_lsn)` as a publication-prefix certificate: the certified prefix must recover exactly E, while later WAL records written after publication are legal and may recover a newer head.

### Failure classification

Chunk, capsule or shadow failures before manifest publication are failures of reconstructible unpublished work and do **not** poison the published store. Manifest publication uncertainty retains the Pass87 fail-stop/reopen rule. Generation allocation observes orphan chunk/capsule artifacts so an aborted or crashed job cannot reuse the same generation identity.

## Verification boundary

Pass89 hostile coverage includes:

1. PREPARE before cut and COMMIT after cut, recovered through `PreparedCutCapsule`;
2. commits between chunk writes and another commit after publication;
3. corrupted checkpoint chunk preventing publication while old authority remains usable;
4. unpublished shadow failure aborting the job without poisoning the published store;
5. compatibility with Pass88 historical-format migration and Pass87 authority-uncertainty semantics.

The final frozen workspace passes fmt, workspace check, strict Clippy and the full test suite with **664 declared tests, 0 failed, 8 ignored**.

## Explicit non-claim

Pass89 closes the **cut/tail durability protocol**, not every performance optimization of checkpoint encoding. `checkpoint::encode_revision` still materializes the canonical logical checkpoint byte stream before it is divided into chunks. Incremental encoder/decoder replacement, asynchronous executor scheduling, throttling, chunk-size tuning and lower peak-memory checkpoint construction remain engineering follow-ups. They do not weaken the H/capsule/shadow/E publication invariant.

## Historical frontier after Pass89

Closed: #1, #2, #3, #4, #5, #7, #9, #11, #12, #14, #15, #19 (**12 / 22**).

Still open: #6, #8, #10, #13, #16, #17, #18, #20, #21, #22.

The next production pass should start with a whole-row audit of **#6 OrderedView/pagination + remaining layout parity**. #17 still requires a real freshness/anti-rollback anchor to close honestly; #13 requires destructive supported-platform evidence; #18 requires mechanization of the publication state machine rather than additional ad-hoc runtime code.

# Pass90 addendum — pinned OrderedView pagination

Historical problem #6 is production-closed on the authoritative hostile closure boundary.

## OrderedView contract

A prepared ordered view is compiled only from an already pinned `PreparedPlan` and an `OrderedViewSpec { column, ordering, direction }`. Preparation requires the selected ordering to be congruent with the column's pinned semantic equivalence. Pagination executes the same pinned logical plan on the native `PhysicalStore`; the cursor never becomes semantic authority.

`OrderedViewCursor` is fail-closed bound to:

- exact physical `RevisionId`;
- exact `SemanticRevision`;
- the complete pinned `SemanticContext`, not revision numbers alone;
- the exact logical `RelExpr`;
- ordered column, ordering id and direction;
- semantic order-class key;
- canonical semantic row key;
- occurrence ordinal for indistinguishable Bag duplicates.

The total cursor order is semantic order class, then canonical semantic row key, then duplicate occurrence ordinal. The latter two are stable physical tie data only; they do not refine semantic WITH-TIES membership. No `PhysicalRowId` is persisted into or exposed by the cursor, so cursor continuation is independent of runtime row handles and physical backend reconstruction.

## Layout parity boundary

Pass90 proves the same paginated logical result, including semantic ties and duplicate Bag occurrences, across every currently implemented native payload backend: RowStore, generic ValueColumnar, I64Columnar and TypedColumnar. A cursor emitted while reading one backend can continue on another backend for the same logical revision and Γ.

This closure does **not** claim that taxonomy labels such as KeyValue, Adjacency/CSR, DenseArray, Inverted or Custom were turned into new specialized storage engines in Pass90. The authoritative historical hostile gate for #6 is stable pagination under ties plus parity across the implemented row/value/typed payload backends; structural total ordering and the Ordered SAMF overlay were already production-closed in Pass82.

## Hostile verification

Pass90 verifies:

1. stable continuation across RowStore, ValueColumnar, I64Columnar and TypedColumnar;
2. semantic ties spanning a page boundary;
3. repeated identical Bag rows without omission or duplication;
4. physical revision mismatch rejection;
5. ordering/direction mismatch rejection;
6. exact Γ drift rejection even when schema/environment revision IDs are unchanged but the pinned module set differs;
7. rejection of unbound physical stores and zero-sized pages.

Frozen workspace gate: fmt/check/strict Clippy/full tests PASS; **666 declared tests, 0 failed, 8 ignored**.

## Historical frontier after Pass90

Closed: #1, #2, #3, #4, #5, #6, #7, #9, #11, #12, #14, #15, #19 (**13 / 22**).

Still open: #8, #10, #13, #16, #17, #18, #20, #21, #22.

The next self-contained production wave should attack #21/#22 constant-factor lowering/benchmark debt before the externally dependent #13/#17/#18 and the coupled #8/#16 distribution wave.

## Pass91 addendum — semantic implementation deployment boundary and REIC coordination classification

### Semantic implementation package boundary
Semantic authority remains the pinned semantic contract in Γ. An executable implementation is a separate deployment object and MUST NOT become semantic authority merely because it is present or authenticated.

Production separates four concerns:
1. semantic contract identity;
2. implementation artifact identity and runtime profile;
3. semantic refinement evidence connecting an implementation to a defined contract;
4. artifact authentication plus runtime execution policy producing explicit `ExecutionAuthorization`.

Authentication does not prove semantic refinement. Refinement does not authenticate executable bytes. Revocation removes execution authorization without changing the semantic contract identity; a separately certified compatible implementation may replace a revoked implementation.

The durable builtin reopen/replay path MUST pass through this package/refinement/authentication/authorization boundary before installing a semantic implementation. Historical operation capability is computed from the contracts actually required by that operation, not from unrelated packages in the registry.

Current production scope is deliberately narrower than a general external plugin platform. Builtin `ImplementationArtifactDigest` is a stable implementation-descriptor identity, not a claim of cryptographic digest over arbitrary executable bytes. External CAS/package files, signature/trust-root/key-rotation verification, sandbox/native/WASM ABI, external proof formats and artifact distribution remain open systems/security work. Opaque external packages may be identified and policy-checked but cannot execute without an available executable backend.

### Durable effect coordination classification
Every durable transaction intent has a stable `DurableEffectKind`. Current durable effects are classified conservatively as `OpaqueNonConfluent` unless a future durable confluence/coherence certificate proves otherwise. Therefore receipt, authentication or shared Γ alone MUST NOT authorize coordination-free union of independently produced effects.

The existing Γ-REIC durable ledger remains the causal authority: independent `RevisionEffectId`, causal prerequisites, exact durable intent payload, recovered ideals/frontiers, and exact multi-parent resolution cuts. Production still has one mutable publication head and does not yet provide durable ingestion/retention of independently advancing branch heads; that remaining lifecycle boundary is coupled to replication/consensus (#16).

---

## Pass92 addendum — durable REIC branch authority

Pass92 closes the durable causal-DAG lifecycle beyond the single published head. Non-head replicated effects use the same `DurableRevisionEffectRecord` / `DurableTransactionIntent` ontology, live in a CRC-protected fsync-backed branch journal, and are admitted only at an exact known causal cut. Branch heads and retirement survive restart; replicated ideals are reconstructed over local + remote events. Local effect IDs remain in origin namespace zero; replicated IDs are namespaced by replica origin.

Current effects remain `OpaqueNonConfluent`. Remote admission therefore requires a unique durable sequencer slot with monotone epoch fencing. This is an ordered-admission boundary, not a claim that CFMD already implements membership/quorum/leader-election/network consensus.

## Pass93 addendum — replication membership, quorum durability and publication

Replication authority distinguishes transport arrival from local durability, quorum durability and reader publication. A replicated effect is never made reader-visible merely because it arrived or was fsynced locally.

A durable `ReplicationMembership` has a monotonically increasing nonzero epoch, a nonempty member set and a strict-majority threshold. Bootstrap is explicit. A later membership epoch is accepted only with the configured quorum of the immediately authoritative previous membership. Membership state is reconstructed from the durable replication journal before quorum/publication decisions resume.

A `ReplicationQuorumCertificate` binds one exact REIC effect identity to one current membership epoch and a unique authenticated-by-caller acknowledgement set. The durability layer validates membership and threshold but does not substitute replica numeric identity for cryptographic proof.

The authoritative lifecycle is:

`Received -> LocalDurable -> QuorumDurable -> Published`.

Only the last three states are durable. `Published` requires the effect and every replicated causal predecessor to be quorum durable, and publication advances contiguously within a branch. The branch's local-durable head and published head are intentionally distinct. None of these transitions changes the store's single linear `durable_head` by implication.

Consensus closure requires more than majority counting: voter identity authentication plus durable vote-once/election decision semantics must make conflicting certificates impossible across replicas. Until that protocol exists, replication/consensus remains partial even though membership/quorum/publication authority semantics are production-defined.
# CFMD IDEAL DB SPEC — PASS94 ADDENDUM

This addendum extends the authoritative Pass93 specification.

## Durable consensus observations are not quorum authority until voted

A replication quorum certificate MUST be backed by durable vote evidence. Transport arrival, authenticated observation, local fsync and quorum authority are distinct stages.

For replicated effects, each voter has at most one durable vote at a given `(membership_epoch, global_decision_position)`. Leader/sequencer identity does not partition the decision slot. Therefore leader replacement cannot authorize the same voter to support a conflicting effect at the same ordered position.

For membership replacement, each voter may durably bind to at most one successor configuration of a previous membership epoch. A non-bootstrap membership change MUST carry a strict-majority acknowledgement set whose members already have matching durable votes for the exact successor and one common term.

These rules are intentionally safety-biased. They do not constitute a complete leader-election/locking protocol and may sacrifice liveness after an abandoned membership vote. A future consensus layer may weaken the conservative rule only with a proved safe lock/term protocol.

Authenticated peer identity remains an external security obligation. `ReplicaId` is an authority coordinate, not cryptographic evidence.

## Universal internal Delta ABI — V5 Stage 1

Public/query compatibility continues to use `RelationDelta`. Internally, CFMD now defines a representation-independent finite signed-effect interface:

`DeltaView<Row>` / `DeltaSink<Row>`.

The first production carrier implementations are:

- `CompactDelta` for common tiny effects;
- fixed `InlineDelta` storage;
- `AdaptiveDelta` with reusable spill storage;
- zero-copy `RelationDeltaView` over the legacy/public delta representation.

This stage changes no maintained-query execution semantics. It exists so subsequent integration can migrate internal edges one at a time while differential tests prove representation equivalence.

## Required next step for #21/#22

Validation/preparation should next produce `ValidatedTransitionFrame` objects that retain both the already-computed source mutation plan and the certified signed effect. Publication authority and candidate-state semantics MUST remain unchanged. Operator fusion and specialized Group/TopK lowerings must not precede this proof-producing preparation boundary.

# CFMD IDEAL DB SPEC — PASS95 ADDENDUM

## V5 Delta Kernel Stage 2 — proof-producing leaf preparation

Maintained-query leaf validation MUST retain successful source-mutation proof work instead of discarding it and recomputing the same mutation during commit.

`ValidatedTransitionFrame<P,D>` binds one deterministic `CompiledDeltaEdgeIdentity` to:

- the already-computed prepared source patch `P`;
- the certified signed effect `D`.

For the current legacy `RelationDelta` compatibility path, validation computes `RelationMutationPlan` exactly once. Commit consumes the matching frame and MUST NOT repeat semantic membership search or reconstruct the source mutation plan. Multiple `Scan` leaves of the same relation are distinct compiled edges and therefore receive disjoint frames. Candidate-clone / atomic publication semantics remain unchanged.

The storage-resolved runtime path already carries authoritative physical row identities and does not have this duplicate semantic membership-plan problem; Pass95 does not regress it to the legacy path.

## V5 Delta Kernel Stage 3 — maximal linear islands

`RelDifferentialProgram` now owns a reconstructible `CompiledDeltaProgram` physical companion. It discovers maximal chains of existing Γ-DTC `Linear` operators and compiles them to `LinearIslandNormalForm`.

The admitted linear operators are exactly:

- `FilterEqConst`;
- `FilterEqColumns`;
- Bag `Project` / `ProjectBag`;
- `PromoteToBag`.

Predicates are rewritten into source-row coordinates and intermediate projections are collapsed into one final projection. `ProjectSet`, `Distinct`, `Group`, `TopK`, Join and blocker classes remain barriers and MUST NOT be fused into a linear island.

The normal-form executor consumes `DeltaView<Row>`, evaluates pinned Γ equivalence through the existing semantic registry, and emits one `AdaptiveDelta<Row>` without changing relational semantics.

Pass95 deliberately stops before Stage 4. Stateful barrier plan/commit kernels remain the next ownership boundary and no partial barrier migration is present in this checkpoint.

## Pass96 addendum — physical barrier kernel boundary and ZeroCrossing

The certified Delta ABI now extends through the first stateful DTC barrier. `CompiledDeltaProgram` enumerates non-linear maintained nodes by `BarrierKernelClass` and therefore separates semantic DTC classification from physical state implementation selection.

`ZeroCrossing` (`Distinct` and set projection) follows a two-phase state-kernel contract: planning consumes a `DeltaView<Row>`, validates Γ-bound rows and support underflow, and returns an immutable patch plus signed output effect without mutating maintained state. Commit only applies that checked patch. This is the production realization of the v5 plan/commit fail-atomicity theorem for support normalization.

No claim is made that Group/TopK/Join/Blocker have migrated merely because their barrier classes appear in the physical program. Annotation must next integrate the v3 Group lowering; Stage 5 cannot begin until all maintained internal barriers use the certified ABI.

## Pass97 addendum — V5 Stage 4.2 Annotation / Group kernel

The maintained `Group` barrier now uses the certified internal Delta ABI as a two-phase state kernel. Planning consumes a `DeltaView<Row>`, validates against the pinned SemanticContext, derives a checked physical patch without mutating maintained state, and emits an `AdaptiveDelta<Row, 4>`. Commit applies only the prepared patch. Public compatibility still materializes `RelationDelta` at the existing boundary; Stage 5 has not started.

Exact-I64 `Count` has a v3-style dense physical tier. Dense admission is a physical decision only: the admitted key span includes a bounded margin and is capped by a fixed slot budget. A changed key outside that window causes a pre-commit fallback to the sparse exact representation; it is never a query-semantic error. Dense cells store `ExactCount`, preserving arbitrary-precision intermediate multiplicity. For the singleton-source/vacant-target replacement microcase the physical count object is moved between dense cells rather than decremented and recreated.

Generic Γ-aware Group remains authoritative for non-I64/non-Count cases and is also split into read-only plan and commit. Semantic equality/order continues to come exclusively from pinned Γ; the dense tier is admitted only for exact-I64 semantics already certified at build time.

Stage status after Pass97: Stages 1–3 complete; Stage 4 framework complete; Stage 4.1 ZeroCrossing complete; Stage 4.2 Annotation complete. Stage 4.3 OrderedBoundary/TopK is not started. No production-closure claim is made for historical #21/#22 yet.

## Pass98 addendum — V5 Stage 4.3a scalar-I64 OrderedBoundary / TopK

The exact scalar-I64 `TopKWithTies` maintained barrier now uses a dedicated physical state behind the existing pinned semantic ordering contract. Physical representation is a replaceable implementation detail and MUST NOT become semantic authority.

The admitted scalar-I64 lowering lattice is:

- `DenseUnit`: bounded dense occupancy when every live multiplicity is exactly one;
- `DenseCounted`: bounded dense exact-count storage after duplicate multiplicity appears;
- `PagedRadix`: sparse exact ordered fallback when the dense span/window premise is not satisfied.

Dense admission is physical only (`margin=64`, `max_slots=4096`). A duplicate insertion into `DenseUnit` promotes to counted state; a key outside the dense window promotes/falls back to paged radix. Neither event is a query-semantic error. Planning happens on an unpublished candidate state, so failed validation or promotion cannot partially mutate authoritative TopK state.

The scalar barrier consumes the universal signed `DeltaView<Row>` through a read-only planner. For a unit replacement the state maintains the kth-with-ties threshold plus `better_rows` and repairs the threshold by at most one adjacent live bucket, matching the v3 unit-replacement theorem. General signed packets remain exact and recompute the physical boundary inside the candidate state. The produced output is an `AdaptiveDelta<Row, 4>` and is materialized back to `RelationDelta` only at the still-existing compatibility boundary.

This is Stage 4.3a, not completion of OrderedBoundary. Non-scalar I64-row TopK and generic Γ-ordered TopK still use the legacy mutation/output path and must be migrated to the same plan/commit kernel before Stage 4.3 is complete. Stage 4.4 Join and Stage 5 root-only compatibility materialization have not started.


## Pass99 addendum — V5 Stage 4.3 OrderedBoundary / TopK complete

Every maintained `TopKWithTies` physical backend MUST obey one two-phase OrderedBoundary contract. Planning consumes only the certified signed `DeltaView<Row>` plus pinned Γ state and returns a typed physical patch together with an `AdaptiveDelta<Row,4>` output effect. Planning MUST NOT mutate authoritative maintained state. Compatibility `RelationDelta` materialization occurs only after the full plan succeeds; commit consumes only the prepared patch.

The physical patch may vary by backend without changing this contract. Scalar exact-I64 keeps the Pass98 dense-unit/dense-counted/paged-radix state. Non-scalar exact-I64 uses checked affected-bucket replacements. Generic Γ-ordering uses indexed identity removals and insertions bound to the pinned ordering encoder. Carrier visitation order is not mutation order: signed bucket planning applies removals before insertions so a semantically identical packet cannot change behavior merely by choosing another `DeltaView` representation.

No generic backend may recover the old protocol of mutating first and then manufacturing the result by comparing whole pre/post `RelationValue` snapshots. Selected-prefix preview used internally by a correctness fallback is a physical implementation detail and remains subject to Stage 6 optimization; it is not semantic authority.

Stage status after Pass99: Stages 1–3 complete; Stage 4 framework complete; 4.1 ZeroCrossing complete; 4.2 Annotation complete; **4.3 OrderedBoundary complete**. Stage 4.4 BilinearPullback/Join and 4.5 BlockerZeroCrossing remain open. Stage 5 and Stage 6 remain open; therefore historical #21/#22 are not yet PROD CLOSED.

## Pass100 addendum — V5 Stage 4.4 BilinearPullback / Join complete

Every maintained equality Join physical backend MUST obey one binary two-phase state-kernel contract. Planning consumes two certified signed `DeltaView<Row>` carriers plus pinned Γ state and returns a typed `JoinDeltaPatch` together with an `AdaptiveDelta<Row,4>` output effect. Planning MUST NOT mutate either authoritative input-side maintained index. Compatibility `RelationDelta` construction occurs only after the complete binary plan succeeds; commit consumes only the prepared patch.

The output differential is defined directly by the bilinear identity `ΔL ⋈ R_old + L_new ⋈ ΔR`, where `L_new` is the left state described by the successful left patch. Therefore the simultaneous-change cross-term is included exactly once without a separate special case. This identity is representation-independent: carrier visitation order is not mutation order, signed multiplicities are normalized during side planning, and Bag weights greater than one remain exact.

Exact-I64 Join may use changed key-bucket replacement patches. Primitive Γ equivalence uses semantic-index identity patches. Structural Γ equivalence uses canonical structural-index identity patches. These are physical choices only; pinned Γ remains semantic authority.

Stage status after Pass100: Stages 1–3 complete; Stage 4 framework complete; 4.1 ZeroCrossing complete; 4.2 Annotation complete; 4.3 OrderedBoundary complete; **4.4 BilinearPullback complete**. Stage 4.5 BlockerZeroCrossing, Stage 5 internal compatibility removal and Stage 6 performance/allocation closure remain open. Historical #21/#22 are therefore not yet PROD CLOSED.

## Pass101 addendum — V5 Stage 4.5 BlockerZeroCrossing complete

Every maintained Difference/AntiJoin blocker MUST use a local Γ-keyed two-phase state kernel. Planning consumes the two certified signed `DeltaView<Row>` inputs, validates them against the pinned `SemanticContext`, computes only affected blocker classes, and returns `BlockerDeltaPatch + AdaptiveDelta<Row,4>` without mutating authoritative blocker state. Compatibility `RelationDelta` materialization occurs only after the complete binary plan succeeds; commit consumes only the prepared patch.

Bag Difference is maintained per full Γ-canonical row class as `(left fiber, right fiber)` with visible multiplicity `max(|L|-|R|,0)`. The emitted effect is only the change in this monus count. AntiJoin is maintained per Γ join-key as `(left fiber, right support count)`. Whole-fiber enumeration is permitted only when right support crosses `0 <-> positive`, because that is exactly when the semantic output changes by the whole fiber.

When AntiJoin remains unblocked before and after a transition, its output effect MUST preserve the actual signed left-fiber change, not merely fiber cardinality. A same-key replacement of one left row by a distinct left row therefore emits a replacement even when the fiber length is unchanged. This tightens the V2 prototype behavior while preserving its blocker-locality theorem.

Carrier visitation order is not mutation order. Blocker planning normalizes signed removals before insertions and accepts weighted entries without first constructing an owned compatibility delta. Underflow, malformed rows, context drift, or an invalid right-side removal fail before blocker commit.

Stage status after Pass101: Stages 1–3 complete; Stage 4 framework complete; 4.1 ZeroCrossing complete; 4.2 Annotation complete; 4.3 OrderedBoundary complete; 4.4 BilinearPullback complete; **4.5 BlockerZeroCrossing complete**. Stage 4 is therefore complete. Stage 5 internal compatibility-materialization removal and Stage 6 corrected performance/allocation closure remain open; historical #21/#22 are not yet PROD CLOSED.

## Pass102 addendum — V5 Stage 5 root-only compatibility materialization

Maintained recursive execution MUST propagate one representation-independent signed carrier between internal nodes. The production carrier is `AdaptiveDelta<Row,4>` behind the `DeltaView<Row>` ABI. `RelationDelta` is a compatibility/persistence envelope only and MUST NOT be constructed on maintained internal edges.

The ordinary leaf-update path may accept `BTreeMap<SemanticId, RelationDelta>` because that is an existing public ingress contract. Leaf validation binds each public delta to its compiled edge and prepares the exact mutation plan. After the leaf commit, the edge effect is converted once into the internal signed carrier. Storage-resolved ingress follows the same rule: `StorageResolvedRelationDelta` remains an authoritative storage/public boundary object, but recursive propagation clones only its signed row effect into the internal carrier.

Linear maintained nodes preserve weights directly. Filter predicates operate on the carrier without compatibility conversion. Bag projection preserves the previous Γ-cancellation semantics after projection before emitting its internal signed effect. Stateful barriers (`ZeroCrossing`, `Annotation`, `OrderedBoundary`, `BilinearPullback`, `BlockerZeroCrossing`) consume child `DeltaView`s directly, commit only their prepared physical patch, and forward the already-certified `planned.effect` carrier.

A successful top-level maintained transition materializes `RelationDelta` exactly once, after the recursive transition has succeeded, using the root result type. Invalid transitions materialize no root result. The same single-materialization invariant applies to storage-resolved propagation. This boundary is intentionally retained for existing public callers, revision transition payloads and persistence integration.

Stage status after Pass102: Stages 1–3 complete; Stage 4 complete; **Stage 5 complete**. Stage 6 corrected whole-chain performance/allocation/differential closure remains open. Historical #21/#22 remain integration-in-progress until Stage 6 demonstrates that the new carrier path preserves the intended performance/allocation properties and no hidden fallback dominates the chain.

## Pass110 addendum — publication proof model and storage-binding cleanup

Authoritative storage-row attachment MUST use validate-before-mutate semantics. A binding operation first verifies every matching Scan snapshot against the supplied ordered `(StableRowHandle, Row)` payload, then performs only the required flat-arena COW writes. Validation failure MUST leave the transition epoch, authoritative arena and published state unchanged. Detached revision candidates remain shallow-COW snapshots by design and are not an authority mutation until publication.

Immutable-generation durability reasoning is now governed by an explicit publication state machine: generation prerequisites are file-synced and followed by prerequisite directory sync before any final manifest rename; pending manifests are never authority; rename-before-directory-sync is an authority-uncertain interval; final manifest directory sync closes publication; obsolete-generation GC may persist removals in any subset before its final directory sync but MUST never target the selected authoritative generation. These are protocol axioms whose supported-platform validity remains a separate durability-assurance obligation.

Historical #18 remains OPEN until this state machine and its production refinement are checked in an external theorem/model-checker environment. Rust exhaustive model checking and subprocess kill tests are required evidence but are not treated as the final proof artifact.

## Pass257 addendum — structural Rewrite boundary and dynamic relation footprints

**[VERIFIED] Universal structural Rewrite preparation.** `PreparedStructuralRewrite<T,I,E>` and `StructuralRewriteEffect<T>` separate certified structural preparation from extensional endpoint materialization. A structural Rewrite retains the same `RewriteSpecId`, law-set identity and explicit semantic intent as an endpoint-backed `PreparedRewrite`; `materialize` is the single compatibility/certification boundary that derives `FineChange<T>`. This is a generic calculus rather than a sequence-only lazy fallback.

**[VERIFIED] Stable sequence endpoint deferral.** `SeqSplice<T>` implements the universal structural-effect contract. `RewriteSpec::prepare_stable_seq_structural_on` resolves stable occurrence/gap intent against a prepared authoritative snapshot and returns the certified splice without cloning the full sequence endpoint. Existing `prepare_stable_seq[_on]` remain endpoint-compatible adapters by materializing exactly once. Hostile clone-count coverage proves structural preparation is O(local payload) with respect to endpoint cloning; full O(N) endpoint construction occurs only when `materialize` is explicitly invoked.

**[VERIFIED] Dynamic relation Rewrite footprints are Γ-bound.** `RelationDelta::prepare_rewrite` / `prepare_relation_rewrite` now derive the required write footprint from the concrete delta and target relation before producing a `PreparedRewrite`. Relation classes are addressed by the collision-free canonical tuple bytes produced by the pinned Γ semantic layer, never by Rust equality/hash or a truncated digest. An underdeclared `RewriteSpec` fails closed with `RewriteFootprintMismatch`.

**[VERIFIED] Coarse relation authority is explicit, not fallback routing.** `SemanticWriteCoordinate::RelationWhole` is an optional conservative declaration which covers class-local coordinates only for the same relation and exact action law. Granular specs may instead declare `RelationClass` coordinates directly. Γ-equal row representatives therefore converge on the same generic coordination coordinate; hostile coverage verifies two such rewrites require coordination.

**[OPEN]** Endpoint-owning consumers in `kernel-plan`, `kernel-transport` and `kernel-lens` still traffic primarily in `PreparedRewrite`; P258 should move the nearest reusable preparation/transport boundary to `PreparedStructuralRewrite` so stable-sequence batches can retain O(B log N + local payload) preparation until a genuine endpoint certificate is required. The same audit pattern should continue for any remaining typed prepare helper whose dynamic semantic coordinates are not derivable at its current API boundary.

## Pass258 addendum — Γ-structural relation Rewrite propagation

**[VERIFIED] Structural Rewrite effects admit an explicit execution context.** `StructuralRewriteEffect<T,C>` is the universal structural contract. Context-free effects such as `SeqSplice<T>` retain the `C=()` compatibility surface, while semantic effects may consume a pinned runtime context without cloning that context into every prepared Rewrite. `PreparedStructuralRewrite<T,I,E,C>` preserves Rewrite family, law-set and explicit intent identity independent of endpoint representation.

**[VERIFIED] Relation Rewrite preparation is structural rather than endpoint-backed.** `RelationDelta::prepare_relation_rewrite` now produces a `PreparedStructuralRewrite<RelationValue,...>` whose `RelationStructuralEffect` owns the concrete delta, one Γ-canonical base-support witness, and the canonical removed/inserted class keys. Delta rows are canonicalized once at preparation; the same evidence derives the dynamic Rewrite footprint and later drives structural application. No stored extensional `RelationValue` endpoint is required.

**[VERIFIED] Base authority is Γ-semantic, collision-free, and fail-closed.** The prepared relation effect binds to the complete canonical support map of the source relation rather than a host hash or opaque fingerprint. A semantically equivalent source representative is therefore admissible, while a different Γ base fails with `StructuralRewriteBaseMismatch`. The compatibility `PreparedRelationRewrite::delta` mirror is checked against the effect-owned delta before use so a forged delta/effect pair cannot acquire transition authority.

**[VERIFIED] Coordination is representation-independent.** `PreparedRewriteIntent` abstracts the Rewrite spec/law-set identity required by the generic coordination graph. Both endpoint-backed and structural prepared rewrites compile through the same coordination registry; the graph no longer depends on `PreparedRewrite<T,I>` specifically.

**[VERIFIED] Real consumers no longer require relation endpoint materialization for certification.** `kernel-plan::prepare_rewrites_inner` validates structural consistency and the Γ base witness directly against authoritative revision rows before transition preparation. `kernel-lens` candidate classification applies the certified structural relation effect when it actually needs a candidate endpoint instead of independently replaying the delta and comparing two endpoint representations. This removes the old relation endpoint-as-proof seam without introducing a relation-specific fallback route.

**[OPEN]** The compatibility `PreparedRelationRewrite` wrapper still mirrors the effect-owned `RelationDelta` for existing plan/durability APIs, so relation intent payload storage remains duplicated by O(B) rather than O(N). P259 should decide whether that mirror can become a read-only accessor over the structural effect and then audit the next endpoint-owning transport/durable boundary. The general TopK whole selected-count replacement path remains a separate kernel-query performance target after the structural Rewrite line is closed.

## Pass259 addendum — endpoint-proof authority without extensional rematerialization

**[VERIFIED] Endpoint-backed Rewrite proof paths borrow the endpoint they already own.** `PreparedRewrite::endpoint()` exposes the exact extensional endpoint carried by its prepared effect without cloning `T`. Residual diamonds, sequential-composition certificates, cube faces and finite braid normalization compare or borrow these authoritative endpoints directly instead of calling `PreparedRewrite::apply()` merely to reconstruct the same value.

**[VERIFIED] Candidate generation and certificate checking have distinct state requirements.** `RewriteResidualPairResolver` and `RewriteSequentialPairResolver` remain base-sensitive because they may derive residual/composite candidates from the concrete reachable state. `RewriteResidualFamilyRegistry` and `RewriteSequentialFamilyRegistry`, however, certify family identity plus exact endpoint coherence and therefore no longer accept a fake `base` argument. The registry surface now reflects the proof it actually performs rather than implying state-sensitive validation that endpoint-backed `FineChange` never supplied.

**[VERIFIED] Residual certificates do not cache duplicate `T` endpoints.** `RewriteResidualDiamond`, `RewriteResidualCubeCertificate`, `RewriteConcurrentNormalizationCertificate`, two-layer certificates and revision residual chain/square/mixed-chain certificates derive `common_endpoint()` from the endpoint already owned by their final residual/composite certificate. `ResidualChainProgress` no longer stores cloned left/right/merged states that were consumed only by endpoint-only registry certification. A clone-count hostile regression verifies that direct residual and sequential registry certification performs zero `T::clone()` operations.

**[BOUNDARY]** This does not claim that arbitrary structural effects can be substituted directly into residual resolution. Residual candidate generation genuinely crosses derived bases, so a structural residual calculus must carry or derive valid post-base structural witnesses rather than reusing an effect certified only for its original base. Endpoint materialization is therefore removed from proof-only paths, not from state-sensitive resolver authority by assumption.

**[OPEN]** `PreparedRelationRewrite` still mirrors its effect-owned `RelationDelta` for compatibility, and several higher residual-chain compatibility entry points still retain a caller `base` parameter even though their endpoint-only certificate subpath no longer consumes it. P260 should close one of these representation/API remnants or, if the relation mirror is removed first, propagate the accessor form through plan/durability consumers before returning to the separate TopK whole-state bottleneck.


## Pass260 addendum — sealed structural Rewrite authority

**[VERIFIED] Prepared structural Rewrite identity is construction-bound.** `PreparedStructuralRewrite<T,I,E,C>` no longer exposes mutable `spec`, `law_set`, `explicit_inputs`, or structural `effect` fields. Read-only accessors expose the certified identity/evidence required by downstream kernels, while construction remains owned by `RewriteSpec::prepare_structural[_with_context]`. A consumer can no longer take a checked structural effect and rebind it in place to another Rewrite family after preparation.

**[VERIFIED] Relation Rewrite has one delta authority.** `PreparedRelationRewrite` no longer stores a compatibility `RelationDelta` beside the effect-owned delta. `delta()` is a zero-copy projection from the sealed `RelationStructuralEffect`, and `rewrite()` exposes only the immutable prepared structural authority. This removes the duplicated O(B) relation-intent payload and makes the historical `is_structurally_consistent()` runtime check unrepresentable by construction rather than repeatedly re-validating two mutable mirrors. Plan, durability, lens and integration consumers now read the same effect-owned delta.

**[VERIFIED] Γ-base binding remains the hostile runtime check.** Removing the duplicate delta/effect pair does not weaken dynamic authority. A relation structural effect still owns the complete canonical base-support witness from preparation, and plan/lens consumers reject a candidate prepared against a different Γ-base with `StructuralRewriteBaseMismatch` / `RewriteEffectMismatch`. A plan regression also asserts that `PreparedRelationRewrite::delta()` and `rewrite().effect().delta()` are the identical borrowed object.

**[AUDITED] Residual materialization boundary is currently semantic, not a compatibility fallback.** The P259 follow-up audited every remaining `base: &T` in maintained Rewrite residual code. These bases feed `RewriteResidualPairResolver` / `RewriteSequentialPairResolver` candidate generation or finite-prefix normalization; the next reachable state is the certified pivot endpoint. Diamond/cube coherence itself remains clone-free, but deleting endpoint materialization from the resolver layer without a new proof object would erase the state on which residual candidates are defined. No fake structural-residual shortcut was introduced.

**[VERIFIED/PARTIAL] TopK threshold work no longer materializes or scans the whole ordered state unnecessarily.** The hostile follow-on found that `I64TopKState::recompute_boundary()` converted every physical count backend into a full `Vec<(key,count)>` before walking only to the k-th threshold, while `selected_counts()` scanned all physical keys and filtered the selected prefix afterwards. `PhysicalCounts::visit_ordered_until` now expresses one ordered boundary traversal with DenseUnit, DenseCounted and PagedRadix lowerings. Threshold recomputation allocates no whole-state vector and stops at the threshold; selected-count projection also stops there instead of traversing the unselected tail. A hostile regression proves first-boundary termination for all three physical representations in both directions.

**[OPEN]** The structural Rewrite line is closed at the typed relation preparation/consumer boundary unless a future residual calculus supplies a first-class certified post-base witness. The remaining TopK target is deeper: the general I64 `plan_signed` path still clones candidate physical state and constructs full `selected_counts()` maps before/after. P261 should derive a sparse arbitrary-delta patch plus output effect from one universal ordered-count overlay / boundary repair calculus, retaining representation-specific lowerings but avoiding whole-state replacement and selected-map materialization without threshold/fallback routing.


## Pass261 correction — disconnected I64 TopK lowering removed from active kernel

**[HOSTILE CORRECTION] `topk_i64.rs` was not a production backend.** A workspace-wide consumer audit found no reference to `I64TopKState`, `I64TopKPlan`, or `I64TopKPatch` outside that module and its own tests. `MaterializedTopKDeltaState`, including the exact one-column I64 relational tests, is backed by `CountedOrderedRows` in `topk.rs`. Therefore the TopK optimization portion of the Pass260 addendum described work on a disconnected lowering and MUST NOT be interpreted as a production performance improvement.

**[VERIFIED] Disconnected code is no longer part of the active crate graph.** `kernel-query::lib` no longer declares `mod topk_i64`; the source file is retained unchanged from the accepted Pass260 snapshot as historical/reference material rather than being deleted or further refactored. This follows the hostile rule that unused legacy/obsolete code is evidence, not a target for polish.

**[VERIFIED] The real production bottleneck is localized.** `CountedOrderedRows::plan_exact_mutation` uses persistent-COW roots, so its top-level clone is not equivalent to an O(N) row clone. The proven avoidable materialization is `selected_effect`: it calls `selected_measure` on both old and candidate state, cloning complete selected row-class maps before computing their difference. Any production OrderedBoundary optimization must therefore operate on Γ-canonical order keys and row classes, not on the disconnected scalar-count lowering.

**[OPEN / NEXT]** P262 should introduce a production boundary certificate for `CountedOrderedRows` (canonical threshold order key plus exact better/threshold mass, with per-order-bucket exact mass) and derive output effects from changed row classes plus the crossed canonical-order frontier. The design must preserve WITH-TIES semantics and semantic representatives, use one generic Γ-ordered calculus for I64/Text/F64/current primitive orderings, and avoid resurrecting a scalar-I64 semantic route.

## Pass262 addendum — production Γ-TopK certified boundary repair

**[VERIFIED] Production OrderedBoundary authority is now cached on the real `CountedOrderedRows` backend.** `MaterializedTopKDeltaState` retains a `TopKBoundary` certificate consisting of the canonical threshold order key and the exact multiplicity strictly better than that key. Every canonical-order bucket also retains its exact total multiplicity. This is one Γ-ordered calculus for all current primitive ordering encoders; no scalar-I64 semantic route has been reintroduced.

**[VERIFIED] Boundary repair is local to the changed order support and crossed frontier.** `plan_exact_mutation` still produces a persistent-COW candidate state, but now also emits exact per-order-key mass deltas and the touched canonical row classes. The candidate boundary is repaired from the previous certificate: changed mass strictly better than the old threshold adjusts the cached prefix exactly, and ordered predecessor/successor steps move only until the kth-with-ties crossing is re-established. If the relation becomes empty the boundary disappears; if total multiplicity is at most `k`, the certified boundary is simply the physical worst live canonical bucket and all rows are selected.

**[VERIFIED] Incremental TopK output no longer materializes complete selected maps before and after each change.** `selected_effect` computes support from touched row classes plus only the canonical buckets whose selected/unselected status changes when the boundary crosses them. Exact multiplicity differences are then read directly from old/candidate row classes. `selected_measure` is no longer reachable from the delta-effect path; it remains only at `output_rows`, where the caller actually requests extensional output materialization.

**[VERIFIED] Redundant row-to-order metadata is removed by the Γ congruence law.** The production state no longer stores a second `row_orders: CanonicalRowKey -> CanonicalOrderKey` persistent directory. A prepared TopK already certifies that its ordering is congruent with the relation equality on the ordered column, so one canonical row class cannot legitimately occupy two distinct canonical order classes. Sparse effect support therefore carries `(CanonicalOrderKey, CanonicalRowKey)` coordinates directly from mutation/frontier evidence, eliminating one O(number-of-row-classes) mirror and one persistent-map mutation per touched class.

**[VERIFIED] WITH-TIES and semantic representatives remain exact.** Boundary membership is inclusive at the canonical threshold bucket. When the threshold moves, complete crossed buckets enter or leave the candidate effect, while touched classes on a stationary boundary are handled by their exact before/after multiplicities. A hostile deterministic regression runs 320 ascending/descending transitions across empty states, `total < k`, dense tie populations and large threshold jumps, checking both emitted deltas and committed materialized output against from-scratch Γ semantics. Existing Text case-insensitive ties, F64 total-order, large exact-multiplicity and maintained-plan tests continue to use the same backend.

**[PERFORMANCE BOUNDARY]** P262 removes the proven `selected_measure(before) + selected_measure(after)` O(selected-state) materialization term from ordinary TopK delta planning. Work is now proportional to canonicalization of the input delta, persistent mutation of touched classes, changed order-support accounting, and row classes in the actually crossed threshold frontier. Initial construction, explicit `output_rows`, and a transition from an empty state to a non-empty selected prefix necessarily enumerate the materialized prefix they create. No threshold/fallback routing is introduced.

**[AUDITED NON-TARGET]** The adjacent Group `dense_i64_count` outlier transition is a physical-cache retirement, not a semantic fallback into the generic Group algorithm: the exact I64-count calculus remains active and continues against the persistent I64 lookup when a dense window can no longer represent the keys. It should not be removed merely because historical tests describe the physical transition as “falls back”.

**[OPEN / NEXT]** The TopK-local temporary `BTreeMap<(CanonicalOrderKey, CanonicalRowKey), ...>` is bounded by delta support rather than database cardinality and is not yet proven to be a bottleneck. A stronger production seam was found in the maintained ExecGraph boundary: every transition allocates `Vec<Option<GraphNodePatch>>` with `graph.node_count()` slots and commit linearly scans all slots even when scheduling/propagation touched only a sparse subset. P263 should replace this dense patch transport with a deterministic sparse patch set whose commit cost is proportional to affected nodes while preserving node-order, failure atomicity and the unified execution calculus. This directly restores the intended sparse-cost property of ExecGraph; only after that should smaller delta-local aggregation costs be reconsidered.

## Pass263 addendum — sparse ExecGraph source/patch/scheduler boundary

**[VERIFIED] Transition source discovery no longer scans the complete flat arena.** `UnifiedTransitionProgram` now owns the single compiled source-occurrence directory: each relation maps to its deterministic global compiled source ordinal and `NodeId`. Semantic and storage-resolved leaf validation iterate only relations present in the incoming delta and only their actual source occurrences. Initial ExecGraph delivery consumes those validated frames through the same directory instead of reconstructing scan ordinals by walking every node on every transition. Repeated-source/self-join occurrences remain disjoint because the existing `CompiledDeltaEdgeIdentity { ordinal, relation }` is preserved exactly.

**[VERIFIED] Stateful patch transport is sparse and deterministically ordered.** `GraphPatchSet` no longer allocates `Vec<Option<GraphNodePatch>>` with one slot per graph node. Planning collects only actual stateful patches as `(NodeId, GraphNodePatch)`, seals them by deterministic `NodeId` sort, rejects duplicate coordinates before publication, and commit walks only that sparse sealed vector. This preserves the old postorder commit order and failure atomicity while removing the dense allocation and dense commit scan. A hostile regression uses a 65-node graph with a large unaffected branch and verifies that a transition on the opposite source carries exactly two stateful patches rather than 65 patch slots.

**[VERIFIED] Candidate scheduling no longer allocates capacity proportional to graph size.** Hostile review found that the old `UnifiedTransitionScratch::ensure_nodes` rebuilt a capacity-wide hierarchical activation bitmap/paged inbox directory whenever a COW `MaterializedRelPlanState` clone started with default scratch. Revision candidates therefore still paid O(V) on their first transition even after patch transport became sparse. The scheduler now keeps the same allocation-free continuation register for chains and spills only real branching work into one ordered sparse ready map. `ensure_nodes` records only the NodeId bound; reset cost is proportional to actually queued nodes. A regression schedules sparse NodeIds up to 999,999 without any capacity-sized ready structure and verifies deterministic order.

**[VERIFIED] Compiled source metadata has one authority.** The former `PreparedRelGraph::source_index` mirror and `SourceProgram::{feeds,is_root}` mirrors were removed. Source count/root/feed introspection is derived from the source-occurrence directory plus the already-authoritative `out_edges` and root NodeId. This avoids parallel compiled metadata that could drift from the transition authority.

**[PERFORMANCE BOUNDARY]** For an already compiled maintained plan, release transition overhead that is independent of actual operator kernels is now bounded by changed source occurrences, affected scheduling frontier, and stateful affected patches rather than total graph node count. The unified scheduler still performs ordered-map work for real branching; linear chains remain in the continuation register. Debug builds intentionally retain the recursive recompute oracle and may traverse more state for verification; that is not the production execution path.

**[VERIFIED] Storage-row identity attachment now uses the same compiled source authority.** `validate_storage_rows_binding` and `commit_storage_rows_binding` no longer walk the full flat arena to rediscover one relation's Scan nodes. They address only the relation's compiled source occurrences. The audit also exposed a correctness seam: attaching handles for a relation absent from the query graph previously succeeded as a no-op while still advancing the transition epoch. It now fails closed with `UnknownRelation` and leaves the epoch/state unchanged.

**[OPEN / NEXT]** P264 should resume hostile inventory across the remaining production `kernel-query` boundaries rather than optimize delta-local maps without evidence. Prefer seams where cost scales with database/query cardinality despite sparse Γ support, or where duplicated metadata/routing represents the same semantic authority twice. The P263 source/patch/scheduler line is closed unless downstream measurement shows a concrete regression in ordered-map branching cost.

## Pass264 addendum — persistent Γ relation-base authority

**[VERIFIED] Relation structural Rewrite preparation has a first-class persistent Γ base witness.** `RelationBaseWitness` binds one relation/revision to its exact canonical row-class multiplicities using the persistent ordered-map substrate. Runtime-owned witnesses carry an in-process authority token that cannot be reproduced by rebuilding equal host rows. A clone of the authoritative witness therefore proves the exact source base in O(1); detached compatibility witnesses remain collision-free and compare exact canonical supports rather than hashes.

**[VERIFIED] Runtime relation authority advances only along delta support.** `RuntimeRevisionBundle` owns one witness per schema relation. Bootstrap/full-revision rebuild constructs witnesses once from authoritative rows. Ordinary relation transitions clone the persistent witness directory and advance only mutated relation witnesses by canonicalizing inserted/removed delta rows and path-copying the touched support classes. Physical/index/materialization-only root publications preserve the same witnesses unchanged.

**[VERIFIED] Rewrite preparation can consume runtime authority directly.** `RelationDelta::prepare_relation_rewrite_on_base` validates removals/set insertions and derives dynamic `RelationClass` footprint coordinates from the supplied authoritative witness without scanning the old relation or cloning a complete support map. The resulting structural effect retains the witness root as its exact base certificate.

**[VERIFIED] Runtime Rewrite certification has one witness calculus and no raw-row replay.** `kernel-plan::prepare_rewrites_inner` no longer Γ-canonicalizes every authoritative source row to re-prove an already prepared effect. It compares the effect-owned base witness with the runtime witness: shared runtime authority is O(1), while detached compatibility preparation uses exact persistent-support equality. There is no fingerprint/hash shortcut and no second raw-row semantic implementation. Publication advances to a new witness authority, so an old runtime-issued Rewrite cannot be rebound to the new revision merely because it retains its old capability.

**[PERFORMANCE BOUNDARY]** The production runtime-issued path changes relation Rewrite preparation/certification from O(N + D log N + N) canonical-support work to O(D log N) touched-support work plus O(1) base-authority certification, where N is source relation cardinality/support and D is delta support. Initial runtime construction/full revision replacement necessarily establishes canonical support once. Detached standalone compatibility preparation still builds one exact witness from its supplied relation; it no longer causes a second raw-row canonicalization inside the runtime.

**[OPEN / NEXT]** The relation-base seam is closed. P265 should return to a kernel-change-focused hostile inventory: inspect residual/revision-effect certificate objects, compatibility adapters, and structural materialization boundaries for remaining duplicated authority or cardinality-scaled proof work. If no comparable production seam remains after that pass, freeze kernel-change as refactor-complete rather than continuing into unrelated kernel-query optimization work.

## Pass265 addendum — kernel-change semantic source split + clone-free cube certification

**[VERIFIED] `kernel-change` production ownership is not a monolithic crate root.** The apparent 2.5k-line `src/lib.rs` was almost entirely co-located unit tests rather than production implementation. Unit tests are now semantically owned under `src/tests/{core,revision_effect_ideal,rewrite_laws,rewrite_spec,semantic_collections}.rs`; `src/lib.rs` is a 14-line module/export root. Production `kernel-change` is approximately 4.5k source lines across the existing semantic owners (`change`, `stable_seq`, `semantic_collections`, `rewrite/*`, `revision_effect/*`); the largest production file is the 646-line residual transport owner. No generic `parts.rs`, include indirection, or mechanical pseudo-module was introduced.

**[VERIFIED] Cube coherence is proof authority, not a duplicate endpoint owner.** `RewriteCubeCoherence` now records the certified Rewrite family rather than cloning and retaining a second complete `PreparedRewrite<T,I>`. The enclosing cube certificate exposes the already-owned coherent upper-face residual when a concrete rewrite is required. Exact TP2 intent/effect equality is still checked before the marker is issued.

**[VERIFIED] Residual cube certification is ownership-linear.** `RewriteResidualFamilyRegistry::certify_cube` certifies upper faces while lower residual rewrites are still borrowable, then moves every endpoint-bearing prepared rewrite into exactly one owning face. Lower faces are certified only after those borrowed validations complete. This removes the previous `PreparedRewrite::clone()` fan-out from direct cube certification and drops the unnecessary `T: Clone` / `I: Clone` bounds at that boundary. Clone-probe regressions now cover diamond, sequential composition, and the full six-face cube and require zero `T::clone()` operations after candidate preparation. A separate non-`Clone` endpoint/intent regression proves the cube-coherence proof boundary itself no longer requires cloneability.

**[BOUNDARY]** The finite n-ary residual/revision-effect normalizer still duplicates endpoint-bearing prepared rewrites when one already-certified lower face must simultaneously remain in the pair-diamond certificate and participate in recursively residualized paths / higher braid certificates. Those clones are not required by direct cube semantics; they are an ownership representation seam in the normalization certificate graph. P266 should either introduce one immutable shared prepared-rewrite authority for residual certificates and normalization paths, or prove a move/borrow organization that removes the duplication without lifetimes leaking through the public certificate API. Do not replace this with shape routing or endpoint hash/fingerprint authority.

## Pass266 addendum — shared immutable Rewrite authority for finite residual normalization

**[VERIFIED] Prepared Rewrite fan-out is represented by immutable shared authority, not endpoint copies.** `SharedPreparedRewrite<T,I>` wraps one already-certified `PreparedRewrite<T,I>` in immutable reference-counted ownership. Cloning the authority is O(1) in endpoint size and cannot mutate or rebind Rewrite identity, explicit intent, effect, or law-set. Residual diamonds and sequential-composition certificates now retain shared authorities, so the same certified residual/composite can appear on several proof edges without duplicating `T`.

**[VERIFIED] Finite n-ary normalization is endpoint-Clone-free.** `RewriteResidualResolutionAuthority::certify_finite_concurrent` converts each caller-owned prepared rewrite into shared authority exactly once. Recursive residual tails, canonical paths, braid lower faces, upper cube witnesses, and level compositions propagate those handles rather than cloning endpoint-backed rewrites. The normalizer no longer requires `T: Clone` or `I: Clone`; a width-four hostile regression uses an endpoint type that deliberately does not implement `Clone` and certifies the complete braid normalization successfully.

**[VERIFIED] Revision-effect pair/triple certificate fan-out reuses the same shared residual/composite authority.** Registered pair/triple witnesses carry shared residual/composite candidates, cube faces retain shared residuals, and six triple final paths share the same certified pair composites/final composite rather than replicating endpoint payloads. Residual-chain progress likewise stores shared certified prefixes; prefix transport clones handles rather than `T`.

**[BOUNDARY]** `RevisionEffectIdeal<PreparedRewrite<T,I>>` still owns endpoint-backed payloads directly. A few compatibility/frontier paths must therefore clone an event payload once when they need a returned certificate to outlive the borrowed ideal (notably singleton branch certification and conversion of ideal frontier payloads into the finite normalizer). This is now the dominant remaining `kernel-change` ownership seam. P267 should decide whether revision-effect Rewrite ideals become first-class `SharedPreparedRewrite` ideals (or gain an equivalent shared authority projection at construction) and then run the final hostile inventory. If that removes the remaining endpoint-sized certificate clones without widening semantics, `kernel-change` can be closed as refactor-complete.

## Pass267 addendum — shared Rewrite revision-effect ideals; kernel-change refactor closure

**[VERIFIED] Revision-effect Rewrite history now owns shared prepared authority from construction onward.** Rewrite-specific ideals use `RevisionEffectIdeal<SharedPreparedRewrite<T,I>>`, so `common_ideal`, `union`, causal scheduling, frontier extraction, singleton/pair/triple normalization and residual-chain transport clone only immutable O(1) authority handles. The historical `RevisionEffectIdeal<PreparedRewrite<T,I>>` production path is removed.

**[VERIFIED] Shared prepared Rewrite is a first-class prepared intent.** `SharedPreparedRewrite` implements the same `PreparedRewriteIntent` identity/law-set contract as its underlying prepared Rewrite. Coordination preparation therefore consumes shared revision-effect payloads directly instead of materializing or duplicating endpoint-backed values.

**[VERIFIED] Finite normalization accepts owned or already-shared authority through one calculus.** `certify_finite_concurrent` accepts any input convertible to `SharedPreparedRewrite`; caller-owned prepared rewrites are shared once, while revision-effect frontiers pass their already-shared authorities without an endpoint clone. There is no separate shared/owned semantic implementation or width-based routing.

**[VERIFIED] Rewrite revision-effect proof paths are endpoint-Clone-free.** Residual frontier, concurrent pair/triple, square/mixed-chain transport and arbitrary-width finite normalization no longer require `T: Clone` or `I: Clone`. A hostile regression builds shared revision-effect ideals around endpoint and intent types that do not implement `Clone` and proves exact `common_ideal`, `union`, and causal-layer behavior. Existing width-four finite-normalization and direct cube non-Clone proofs remain green.

**[HOSTILE INVENTORY / CLOSED]** Production `kernel-change` contains no `RevisionEffectIdeal<PreparedRewrite<...>>` / `RevisionEffect<PreparedRewrite<...>>` Rewrite history, no endpoint-sized payload clone in the residual/revision-effect certificate graph, and no residual/rewrite `T: Clone` or `I: Clone` proof bound. Remaining `payload.clone()` sites clone `SharedPreparedRewrite` handles only. Active production markers `fallback`, `legacy`, `obsolete`, `PAYER`, `TODO`, and `FIXME` are absent. Source ownership remains semantically split (`lib.rs` is a 14-line export root; the largest production owner is approximately 645 lines).

**[STATUS] `kernel-change` hostile/global refactor is COMPLETE / FROZEN at Pass267.** Future edits to this crate require a new measured correctness, mathematical, DX, or performance seam rather than continued cleanup-by-inertia. The next global refactor target should be selected from the remaining kernels by a fresh hostile inventory; work discovered in neighboring `kernel-query`/`kernel-plan` during Pass257-P264 is not grounds to reopen `kernel-change` without new evidence.

## Pass268 addendum — cross-kernel hostile sweep; writable-view lift fail-closed routing

**[HOSTILE CROSS-KERNEL INVENTORY]** Pass268 did not reopen `kernel-semantics` by assumption. The active kernels were scanned for error erasure, fallback/legacy markers, repeated full materialization, nested scans and duplicated authority. The strongest confirmed correctness seam was in `kernel-integration`; the superficially similar `.ok()` replay path in `kernel-plan::recovery` is deliberately physical-only and falls back to exact rebuild from authoritative logical state, so it was audited and left unchanged.

**[FIXED] Join+project representative search no longer erases semantic/projection failures.** The lossy join/project reconstruction path previously used `find_map` with `.ok()?` around both staged projection and Γ-equivalence comparison. A real semantic failure could therefore be misclassified as “no existing preimage” and route into the hidden-column constructor. Representative discovery is now a `Result`-aware loop: projection/equivalence failures propagate unchanged and candidate rows remain unmodified. A hostile regression pins a projected equivalence with no available implementation and proves the semantic error is returned rather than constructing a replacement owner row.

**[VERIFIED] Durable writable-view commit no longer selects algorithms by catching `CandidateGenerationUnsupported`.** The previous publication boundary tried identity, bijective project, lossy project, join and filter synthesis in sequence, using errors from one implementation as control flow into the next. Pass268 replaces this with one structural `RelWritableLiftStrategy` classification from the already-compiled lift stages and obligations. Exactly one implementation is selected (`Identity`, `BijectiveProject`, `LossyProject`, `JoinOwnerSide`, or `Filter`); any error from that implementation is terminal. This removes error-driven fallback routing without introducing thresholds or duplicated semantics.

**[AUDITED / NEXT]** The next concentrated risk remains `kernel-integration`: plain lossy projection and join+projection maintain parallel preimage-reconstruction engines, each repeatedly projects/scans owner rows for every inserted/removed view row. P269 should derive one Γ-preimage catalog keyed by canonical projected row class, preserving Bag/Set multiplicity and determinant ambiguity semantics, then reuse it in both paths. This should remove the duplicated `O(delta_support * owner_rows)` search and provide a natural semantic split for the current integration god-file. Do not optimize the deliberately exact-rebuild physical recovery path merely because it uses fallback terminology.

### Pass269 hostile integration boundary — Γ-preimage catalog

**[FIXED] Writable lossy projection uses one exact Γ-preimage calculus.** Plain lossy project reconstruction and lossy join+project reconstruction no longer perform a fresh owner-row projection/equivalence scan for every removed or inserted view row. Both build one `GammaPreimageCatalog` from the authoritative owner section. The catalog is keyed by the tuple of `CanonicalEqKey` values induced by the projected relation's declared equivalences, stores exact source occurrence identities, and records the set of canonical owner-row classes inside each projected class.

**[SEMANTICS] Bag/Set ambiguity is derived from canonical classes, not host equality.** Set deletion consumes the full projected Γ-class. Bag deletion consumes one source occurrence only when the projected class has exactly one owner Γ-class; multiple owner classes are `ProjectionPreimageAmbiguous`. Existing-class insertion reuses an authoritative source representative from the same catalog. Canonicalization failures propagate fail-closed; there is no scan fallback or host hash/equality path.

**[COMPLEXITY] Preimage discovery is prepared once per reconstruction.** If `N` is the owner-section cardinality and `D` is requested view-delta support, owner projection/classification is `O(N)` semantic work plus ordered-map insertion, while each requested projected class is then resolved by canonical-key lookup rather than another `O(N)` scan. A regression proves owner projection occurs exactly once per source row and is not repeated across 128 subsequent class lookups.

**[STRUCTURE]** `GammaPreimageCatalog` now has a dedicated `kernel-integration::preimage` owner instead of extending the integration root further. Further integration splitting should follow semantic ownership boundaries, not arbitrary line counts.

**[FIXED] Direct join lookup uniqueness is no longer quadratic repeated query evaluation.** The writable direct-join guard previously evaluated the lookup query once, then rebuilt and executed `FilterEqConst(lookup_query, key)` for every lookup row. Pass269 validates uniqueness in one traversal of the already-materialized lookup value by inserting the declared join-equivalence `CanonicalEqKey` into an ordered set. This changes the guard from repeated full-query probes to one query evaluation plus canonical-key insertion, and it remains Γ-semantic: raw-distinct but equivalent keys (for example case-insensitive `"Alpha"` / `"alpha"`) are rejected as non-unique. A regression pins that behavior.

**[AUDITED / NEXT]** `kernel-integration` remains OPEN after P269. The next duplicated mechanism is projection composition itself: plain lossy-project and join-project retain separate stage-by-stage row projection and coordinate-composition helpers, allocating intermediate rows for deep project chains. P270 should introduce one prepared projection-path authority that composes Project stages once into owner coordinates and provides direct projection plus certified bijective inversion. This is a semantic unification target, not a threshold fast path. The apparent `FiniteModel::clone()` postcondition checks are not row-deep copies: the model/relation stores are COW; do not misclassify them as whole-database row cloning without new evidence.

### Pass270 hostile integration boundary — compiled projection authority

**[FIXED] Writable projection chains now have one compiled owner-coordinate authority.** `kernel-integration::projection::PreparedProjectionPath` compositionally reduces a chain of `Project` stages to the final owner-coordinate map once. Plain project writes, lossy preimage classification, join-owner project writes, bijective inversion, determinant visible/hidden coordinate derivation, and explicit hidden-column construction now consume that same authority.

**[REMOVED] Parallel staged projection machines are gone.** The production helpers `project_owner_row`, `project_row_through_stages`, `owner_projection_columns`, `projection_columns_from_stages`, and reverse stage-by-stage bijective inversion have been removed. Deep project chains no longer allocate an intermediate row for every stage when projecting each owner row; runtime projection directly gathers the final visible owner coordinates.

**[VERIFIED] Bijective and lossy projection are two laws over the same compiled map, not routed algorithms.** A path is bijective exactly when its composed owner-coordinate map is a permutation of the owner arity. Inversion places visible values directly back into those owner coordinates. Lossy construction uses the same map plus explicitly authorized hidden values; missing hidden values, visible-column override, out-of-range constructor coordinates, and non-bijective inversion all remain fail-closed.

**[HOSTILE REGRESSIONS]** Nested projection composition is pinned to its direct final owner coordinates; a composed nontrivial permutation is inverted without replaying the stage chain; and hidden-column inflation is proven to use the same compiled coordinate authority. Existing plain/join lossy, bijective, constructor, Γ-preimage, and complement-preservation tests remain green.

**[STATUS / NEXT]** `kernel-integration` remains ACTIVE. After Pass270 the next candidate should be chosen from a fresh integration hostile inventory rather than continued projection cleanup. A likely remaining duplication is postcondition certification: several lift strategies independently build the owner delta, apply it, install the candidate relation, evaluate the view, and compare the requested endpoint. If this is confirmed to be the same authority contract across strategies, P271 should centralize it as one certification boundary; otherwise the next target should come from the cross-kernel ledger.

## Pass271 addendum — prepared writable authority reaches commit

**[VERIFIED] Writable mutation no longer recompiles an already prepared relational query.** The public integration commit boundary now consumes `&mut PreparedWritableCompilation` rather than a detached `RelWritableViewPlan`. The prepared object owns both the compiled writable classification and its revision-pinned `PreparedRelWritableCoordinates`, whose observable catalog already enforces exact semantic-revision binding. Lossy projection and owner-side join therefore reuse that authority directly; production commit code contains no `prepare_baseline(plan.query.clone(), ...)` path.

**[VERIFIED] Owner candidate certification has one fail-closed implementation.** Filter, bijective projection, lossy projection and owner-side join strategies now stop after reconstructing a candidate owner relation. `certification::certify_owner_candidate` alone computes the exact owner delta, normalizes it, evaluates the writable view against the COW candidate model, rejects any non-empty difference from the requested endpoint, and only then prepares the authoritative relation Rewrite. Strategy-specific copies of this postcondition protocol are gone.

**[VERIFIED] No compatibility fallback was retained on the production mutation surface.** The old plan-only commit API was replaced rather than retained as a wrapper that silently re-prepared coordinates. Test-only direct synthesis helpers may construct coordinates locally to exercise internal strategies, but the durable production path requires prepared authority by type.

**[VERIFIED] Prepared writable authority is construction-bound.** `PreparedWritableCompilation` no longer exposes replaceable `coordinates` / `compilation` fields. Callers can inspect them through read-only accessors, but cannot splice a coordinate catalog from one prepared query into the writable classification of another. `compile_prepared_relational_writable_query` is the authority-producing constructor and durable commit consumes the sealed pair.

**[PERFORMANCE BOUNDARY]** For reusable writable plans, mutation no longer pays baseline plan preparation plus relation-column observable catalog construction on every lossy write. Runtime determinant revalidation remains intentionally data-dependent because it certifies the concrete before/after owner relation; only reconstructible compile-time authority was hoisted out of the mutation lifetime.

**[OPEN / NEXT]** `kernel-integration` remains ACTIVE pending one final hostile/structural pass. The largest remaining coherent subsystem is owner-side join lifting. P272 should determine whether direct-join analysis/reconstruction is a stable semantic owner worthy of `join.rs`, and audit whether its query fragments (`lookup_query`, owner section, unmatched complement) are repeatedly typechecked/evaluated in ways that can be safely prepared once without making model-dependent evidence stale. If no comparable correctness/performance seam remains after that pass, integration can move to COMPLETE and the campaign can return to grouped cross-kernel closure sweeps.

### Pass272 — prepared direct-join writable authority

**[VERIFIED] Owner-side join structure is compile-once authority.** `kernel-integration::PreparedWritableCompilation` now owns a prepared lift strategy. For owner-side joins that strategy contains a `PreparedDirectJoinLift` compiled exactly once from the pinned writable plan: owner/lookup placement, full-row owner section, lookup section, unmatched and rejected complement queries, join columns/equivalence, owner and visible-owner relation types, lookup arity, and the composed owner projection path. Runtime writable commit no longer re-enters `direct_join_section`, `owner_join_pipeline`, join-section typechecking, or projection-path compilation.

**[BOUNDARY] Runtime evidence remains runtime evidence.** The prepared join authority deliberately does not cache current lookup-key uniqueness, relation rows, Γ representatives, determinant before/after evidence, or requested-view postconditions. Those depend on the concrete runtime model and remain fail-closed on every mutation.

**[STRUCTURE] Writable lift strategy itself is prepared.** Lift classification is now construction-bound inside the sealed `PreparedWritableCompilation`; runtime commit dispatches the already prepared strategy rather than rescanning immutable lift stages. The join payload is boxed so non-join prepared capabilities are not inflated by the large compiled join structure.

**[HOSTILE FOLLOW-UP / NEXT]** `kernel-integration` is not yet marked COMPLETE after P272. The same reconstructible-authority class remains in the plain projection strategies: bijective and lossy projection still recompute owner/view types and rebuild `PreparedProjectionPath` from immutable lift stages per write; filter reconstruction also rebuilds its complement expression/type authority. P273 should make every lift strategy first-class prepared state (`PreparedBijectiveProjectLift`, `PreparedLossyProjectLift`, `PreparedFilterLift`) and then run the final integration hostile inventory. If no comparable seam remains, mark `kernel-integration` COMPLETE/FROZEN.

**[COMPATIBILITY]** Prepared compilation continues to represent `ReadOnly` lens classifications explicitly. P272 does not turn read-only analysis into a preparation error; durable mutation still rejects that prepared strategy at the writable boundary.

### Pass273 — origin-normalized prepared lifts and integration closure

**[VERIFIED] Every production writable lift now carries first-class prepared authority.** Identity, plain project, filter, and owner-side join strategies are no longer marker-only variants that reconstruct immutable query/type state during mutation. `PreparedLiftBase` owns prepared owner/view expressions and their pinned relation types; `PreparedProjectLift` adds the normalized projection authority; `PreparedFilterLift` adds a prepared rejected-complement expression; `PreparedDirectJoinLift` consumes the same prepared base.

**[R&D / NORMAL FORM] Plain projection authority is derived from final origins, not historical stages.** For owner-only projection lifts, `RelWritableViewPlan::output_origins` already records every final view column in original owner coordinates. `PreparedProjectionPath::from_output_origins` therefore treats that origin map as the canonical projection normal form, validates owner identity / bounds / uniqueness once, and directly constructs the visible-owner coordinate vector. Production plain-project writes no longer replay `Project` stages at preparation or mutation time. Stage composition remains only inside the structurally different owner-side join analysis, where the projected owner subsection must be recovered before the join boundary.

**[HOSTILE FIX] Hidden `RelExpr::evaluate()` recompilation was removed from the mutation path.** `RelExpr::evaluate()` prepares/typechecks its expression before execution, so merely caching explicit `typecheck()` results would still have left per-write compilation. Prepared Identity/Project/Filter payloads now evaluate `PreparedRelExpr`; P272's join payload was strengthened in the same pass so lookup, owner/view, unmatched, rejected-complement, and full-row-owner expressions are all prepared exactly once. The shared `certification::certify_owner_candidate` boundary now also consumes the prepared owner type and prepared view expression instead of re-typechecking them. Runtime retains only model-dependent lookup uniqueness, Γ-preimage/determinant evidence, current rows, requested endpoint, and the actual postcondition comparison.

**[STRUCTURE] Direct-join lifting now has a semantic owner.** The coherent owner-side join subsystem was moved from the integration root to `kernel-integration::join`. The root remains responsible for compilation/strategy dispatch and common project/filter preparation; join analysis, Γ-aware reconstruction, uniqueness validation, complement preservation, and join-specific test adapters are owned by `join.rs`. This is a semantic split rather than line-count sharding.

**[NO FALLBACK]** No threshold routing, SQL-style generic fallback, error-driven alternate algorithm, or compatibility mutation path was introduced. Strategy variants correspond to distinct proven lift semantics; common preparation and certification are shared authorities rather than fallback layers.

**[FINAL HOSTILE INVENTORY]** The production writable mutation path contains no raw immutable relation `typecheck()` or raw `RelExpr::evaluate()` calls. Remaining direct `typecheck()` in `join.rs` belongs exclusively to construction-time owner-pipeline analysis. Lift-stage / obligation inspection is construction-time only. The final inventory found no comparable repeated-compilation, duplicated-authority, error-erasure, or generic-fallback seam in `kernel-integration`.

**[STATUS]** `kernel-integration` is **COMPLETE / FROZEN** after P273. Reopen it only on new evidence. The next refactor target is the grouped cross-kernel closure sweep, beginning with the still only boundary-audited `kernel-lens` / `kernel-transport` / `kernel-revision` line and then the remaining medium/small kernels.

### Pass274 — owner-influence lens calculus, compiled transport, and DAG frontier closure

**[VERIFIED / R&D] Writable relational analysis is owner-influence aware.** Projection/filter preimage obligations and lift stages are now produced only for subexpressions whose dependency cone contains the writable owner. Owner-free lookup subtrees are observational dependencies rather than a second write owner, so lossy lookup projections no longer invent source-complement obligations and lookup-only nested joins are admissible when their direct observable origins remain defined. The determinant identity law `X ⊢ X` is discharged directly before consulting external determinant theory.

**[HOSTILE FIX] Determinant-theory failures are no longer erased.** Projection and join obligation construction previously converted `DeterminantTheory::closure` errors into `false` through `unwrap_or(false)`, silently reclassifying malformed/foreign Γ coordinates as unresolved writability obligations. The relational writable compiler now distinguishes structural writability failure from determinant-analysis failure and propagates `AnchorPullbackError` through its existing outer result.

**[VERIFIED] Typed relation transport is a compiled transport program.** `TypedRelationTransport::verify` now retains prepared relational transforms. Revision transport evaluates those prepared expressions directly and no longer re-runs `RelExpr::prepare` for every transported state. Runtime-dependent model data remains live; immutable query/type/order authority is construction-bound.

**[VERIFIED / PERFORMANCE] Typed field transport no longer scans the entire field store once per rewrite.** Verified field rewrites are grouped by source field. Transport performs one ordered pass over source field entries, copying passthrough authority and applying only transforms indexed for the encountered field. Work is proportional to source field entries plus actually applicable rewrite evaluations rather than `rewrite_count × field_entry_count`. No size threshold or generic fallback is introduced.

**[HOSTILE FIX] Explicit rewrites cannot be silently ignored by passthrough authority.** A rewrite targeting an unchanged passthrough field or relation is now rejected at verification instead of being accepted but omitted from execution. This removes dual/ambiguous transport authority.

**[R&D / PERFORMANCE] In-memory merge-base selection uses the common-ancestor frontier law.** Common ancestors form a downward-closed subset of the revision DAG. A common revision is therefore non-lowest iff it is an immediate parent of another common revision. `lowest_common_ancestors` now marks dominated parents in one pass over the induced common subgraph instead of issuing pairwise transitive ancestry traversals. `is_ancestor` likewise short-circuits on the requested ancestor instead of materializing a complete ancestor set first.

**[AUDIT STATUS]** `kernel-lens` and `kernel-transport` received dedicated whole-crate hostile passes in P274; `kernel-revision` was independently inspected and regression-tested with no comparable new correctness/performance seam found in its maintained production paths. `storage-memory` was reopened because the transport/revision sweep exposed the merge-base DAG bottleneck and the frontier rewrite above. The next grouped closure should structurally split the still-large transport/lens owners only where semantic ownership is coherent, then continue into the remaining medium/small kernels (`kernel-grounded-closure`, `kernel-semantic-index`, `kernel-validation`, `kernel-auth`, and adjacent small kernels).

### Pass275 — semantic kernel ownership, witness-indexed closure maintenance, and dense capability extents

**[STRUCTURE] `kernel-lens` and `kernel-transport` no longer concentrate maintained production semantics in crate-root god files.** `kernel-lens` is split into value/complement algebra, scalar writable compilation, relational writable calculus, and migration-complement retention behind a 9-line export root. `kernel-transport` is split into core value/query/model/lifecycle transport, semantic-environment transports, typed field/relation transport, and test ownership behind a 41-line export root. Public crate APIs remain root re-exports; this is semantic ownership rather than `include!` sharding.

**[R&D / UNIFIED CALCULUS] Grounded deletion/update repair now has one selected-witness principle.** The generic indexed deletion path previously rebuilt a second dense witness-child graph and maintained a duplicate recomputation engine even though bipolar maintenance already had `GroundedWitnessIndex` and sparse witness-cone repair. P275 removes that duplicate engine. One-shot indexed updates, structural reconciliation, generic maintained updates, and bipolar structural maintenance all use the same witness-indexed deletion/insertion calculus. `GroundedMaintenance` exposes this compiled incremental state for arbitrary grounded programs, retaining both incidence and selected-witness reverse indices across updates. A randomized mixed-update regression checks it against full recomputation.

**[R&D / RULE CORRESPONDENCE] Structural rule remapping no longer clones rule bodies into canonical keys.** `GroundedRule` itself is ordered and residual correspondence stores borrowed rule references. Stable positional equality remains only an allocation-free fast path of the same extensional occurrence-matching rule; reordered/changed rules are matched by borrowed value without a second fallback representation or `Vec<body>` allocation.

**[PERFORMANCE] Witness-index compilation scans live proof owners rather than every atom slot.** Dead atoms cannot own selected rule witnesses, so witness-index construction traverses `certificate.live_atoms()` and installs only actual proof edges. The dense child-reconstruction implementation and its separate `derive_from_rule` / insertion machinery were deleted.

**[HOSTILE FIX / VALIDATION] Capability membership is extensional dense-set authority, not repeated physical-carrier traversal.** `DenseTypeExtents::entities(type)` exposes the deduplicated extent already compiled for validation. Capability required-field checking computes matching physical carrier types once per capability for owner-contract validation, then validates each extensional capability member once per required field. Dynamic violation measurement uses the same dense extent.

**[CORRECTNESS] Overlapping physical carriers no longer inflate capability violation mass.** Before P275, one entity present in two concrete carriers that both subtype the same capability could add the identical `MissingCapabilityRequiredField` witness twice. Because violation mass is a finite measure over exact witnesses, that counted one semantic defect with mass 2. Dense capability extents deduplicate membership, and a regression now proves the witness mass is exactly 1.

**[AUDIT STATUS]** `kernel-lens` and `kernel-transport` have both semantic hostile closure and coherent physical ownership after P275. `kernel-grounded-closure` received a dedicated whole-crate hostile pass and its duplicate generic/maintained mutation engines were unified; future work there should be evidence-driven rather than cleanup-by-inertia. `kernel-validation` received a targeted hostile reopen that closed the capability/carrier correctness-performance cluster, but its remaining 1.6k-line root still merits a later structural split if no stronger kernel defect takes priority.

**[NEXT]** The next grouped target should begin with `kernel-fixpoint`: reconnaissance shows it already lowers ordinary reachability and support to grounded closure, but positive-bag analysis still owns an independent active dependency graph/SCC layer and deduplicates outgoing dependency edges through repeated `Vec::contains`. P276 should test whether that line can reuse a first-class grounded/condensation authority or otherwise obtain one universal sparse graph construction without quadratic per-adjacency dedup. Then continue through `kernel-semantic-index` and the remaining medium kernels (`kernel-auth`, `kernel-deployment`, `kernel-model`, `kernel-persistent`) by evidence.

## Pass276 addendum — fixpoint incidence authority and persistent page-wise mutation

**[VERIFIED] Positive-bag recursion has one active incidence authority.** `kernel-fixpoint` no longer reconstructs dependency information independently for SCC classification, finite indegree, and per-head multiplicity evaluation. `PositiveBagIncidence` compiles productive rules once into `rules_by_head`, a deterministic deduplicated premise→head graph, and its exact reverse graph. SCC/productive-cycle detection, infinity propagation, finite topological indegree, and finite proof-tree evaluation all consume those coordinates. The former per-head full `program.rules` scans and adjacency `Vec::contains` deduplication are removed. This is one universal representation; there is no size threshold or failure-driven fallback to the old solver.

**[VERIFIED] Positive-bag checking is no longer solver-as-oracle verification.** `check_positive_bag` independently validates the grounded least-support certificate, reconstructs the expected infinite frontier from the compiled incidence/SCC law, and checks finite multiplicity equations directly against the presented certificate. It does not call `solve_positive_bag` and compare outputs. Finite acyclic coordinates therefore have a direct equation check while productive recursive coordinates are certified by the same explicit infinity frontier law.

**[FIXED] Reachability checked certificates have an exact proof payload shape.** Public `rank` and `parent` maps may no longer carry unverified entries outside the reachable proof domain; seed parents are likewise rejected. This closes a proof-object integrity seam where the visible output set was valid but a `CheckedCertificate` could retain arbitrary unchecked auxiliary payload.

**[STRUCTURE] `kernel-fixpoint` is split by calculus, not line count.** The root owns ordinary reachability plus the generic certified-fixpoint boundary; positive-bag / N∞ proof-tree semantics live in `positive_bag.rs` and are re-exported through the existing crate API. No compatibility wrapper or alternate implementation was introduced.

**[FIXED] `PersistentVec` mutation obeys its page-copying contract for bulk operations.** Ordered removal no longer shifts a tail by calling `set` once per element. Each affected fixed-size page is copied once, shifted locally, and linked through one radix-path update per page while untouched prefix pages remain structurally shared. Bulk `resize`/`resize_with` now truncate or extend page-at-a-time instead of repeated `pop_last`/`push`; same-page `swap` and `swap_remove` use one page copy. Logical vector semantics and snapshot isolation are unchanged.

**[AUDITED / CLEAN] `kernel-semantic-index`.** The compact production owner was re-audited after the fixpoint work. Normal insert/remove uses persistent ordered buckets plus reverse identity authority; ordinal exhaustion compaction is an exceptional representation repair at the `u64` address boundary, not a normal routing/fallback path. No comparable semantic or complexity defect was established, so P276 leaves the crate unchanged.

**[AUDITED / NO CHANGE] `kernel-model`.** Normalization and live-reference sensitivity were inspected by evidence after the persistent-vector work. Recompilation when the dense identity domain changes is semantic-coordinate reconstruction rather than a duplicated fallback implementation. No comparable defect was established in P276, so the model code remains unchanged.

**[STATUS] `kernel-fixpoint` is COMPLETE / FROZEN after P276.** Its confirmed repeated-scan cluster, checker independence seam, proof-payload exactness, and physical ownership are closed. `kernel-persistent` received a targeted hostile performance reopen and remains a foundational crate to reopen only on new evidence; P276 does not claim a whole-crate mathematical closure merely from the vector work.

**[NEXT]** Grouped closure should move to the remaining unaudited security/authority kernels, beginning with `kernel-auth` and `kernel-deployment`, then `kernel-lifecycle`/`kernel-model` by evidence. `kernel-validation` still has a structural split opportunity, but line-count cleanup should remain behind correctness/authority/performance findings.

## Pass277 addendum — authenticated bounded distribution and static capability contracts

**[SECURITY / AUTHORITY] Artifact bytes are no longer touched before manifest metadata is authenticated.** `kernel-auth::authenticate_artifact_manifest` verifies the signer and signed artifact message first, then checks the semantic-descriptor commitment and yields a construction-sealed `AuthenticatedArtifactManifest`. `verify_artifact_from_cas` consumes that authority before invoking the CAS. A hostile regression forges a manifest with `artifact_len = u64::MAX` and proves the CAS is not called.

**[SECURITY / RESOURCE BOUND] CAS retrieval is exact-length authority, not whole-file retrieval followed by rejection.** `ArtifactCas` now exposes `load_exact(digest, authenticated_len)`. Filesystem CAS reads through `Read::take(authenticated_len + 1)` and rejects short/oversized objects as corruption; it never uses `fs::read` for artifact materialization. The artifact commitment is hashed exactly once after retrieval rather than once for CAS corruption and again for manifest verification.

**[SECURITY / PACKAGE BOUND] Package repositories enforce the size bound during retrieval.** `PackageRepository::load_package_bounded` makes bounded materialization part of the repository contract. The filesystem repository reads at most `MAX_PACKAGE_BYTES + 1` from one opened file handle, avoiding both unbounded allocation and `metadata()` TOCTOU authority. Memory repositories enforce the same contract. Package bytes remain non-authoritative until ordinary package identity, manifest authentication, policy, runtime-profile, and refinement checks complete.

**[SECURITY / VERIFY ORDER] Signed metadata is authenticated before expensive payload commitments where the signed digest already provides the authority cut.** Artifact descriptors, durable generation components, and WAL frames now verify the signer/message before hashing the supplied payload. Valid signatures still require exact digest/length commitments; invalid signatures can no longer force those payload hashes first.

**[CORRECTNESS / VALIDATION] Capability required-field ownership is a schema law, not a fact inferred from the current model population.** Validation now requires `CapabilityId ⊑ FieldOwner` directly. The former implementation scanned only currently present carrier types and could accept a malformed capability contract when the current concrete types happened to subtype both the capability and the field owner. A regression fixes that future-state unsoundness and removes the capability×present-carrier scan.

**[AUDIT STATUS]** `kernel-auth` and `kernel-deployment` received a dedicated whole-crate security/authority hostile pass. The compact `kernel-aggregate`, `kernel-identity`, `kernel-exact`, `kernel-lifecycle`, `kernel-retention`, `kernel-proof`, `kernel-types`, and `kernel-violation` owners were also grouped-audited and regression-tested; no comparable new defect was established, so they remain unchanged. Next evidence-driven closure should return to the still-partial medium owners: `kernel-validation` structural ownership, `kernel-persistent`, `kernel-grounded-closure`, `storage-memory`, and `kernel-model`.

## Pass278 addendum — schema subtype closure and authority-kernel ownership

**[R&D / SCHEMA AUTHORITY] Subtyping is a maintained transitive schema authority, not a repeated graph query.** `Schema` now owns `SubtypeClosure`, an incrementally maintained ancestor relation updated by every accepted `include(subtype, supertype)`. Adding an inclusion propagates the supertype's already-known ancestors to the subtype and every existing descendant of that subtype. Cycle rejection and every public `Schema::is_subtype` query therefore consume the same closure authority in logarithmic set lookup rather than starting a fresh inclusion-graph traversal. The existing direct-relation reference regression still checks the closure exhaustively over a generated acyclic inclusion graph, including duplicate direct insertions and transitive diamonds.

**[PERFORMANCE / VALIDATION] Dense extents propagate along prepared ancestors instead of testing every target independently.** `DenseTypeExtents::compile_with_ids` no longer performs `physical carrier × semantic target × fresh subtype traversal`. For each physical carrier type it walks that type's prepared ancestor set once and installs the carrier into only relevant initialized extents. This is one universal subtype representation shared with schema contract validation; there is no size threshold, memoization fallback, or validation-specific alternate graph algorithm.

**[STRUCTURE] `kernel-validation` is split by semantic responsibility.** The crate root is now an API/test façade. `error.rs` owns validation failure vocabulary and conversions; `violation.rs` owns dynamic/relation violation measures; `extents.rs` owns dense extensional type membership; `state.rs` owns state/relation/value typing. The public API is preserved through root re-exports, and recursive value validation remains one internal owner rather than a compatibility wrapper.

**[STRUCTURE] `kernel-auth` is split along cryptographic authority boundaries.** `core.rs` owns digest/key identities, trust roots, strict verification, and key rotation; `artifact.rs` owns authenticated artifact manifests and exact CAS verification; `durable.rs` owns signed generation/WAL authority; `freshness.rs` owns rollback/fork/freshness anchors. Cross-module verification primitives are crate-private; no raw key access or second verification path was introduced.

**[STRUCTURE] `kernel-deployment` is split along deployment authority boundaries.** Runtime/deployment policy, ABI framing, shared bounded cursor decoding, package repositories/CAS adapters, Linux sandbox runtime, external verification, package codec/error vocabulary, and resource limits now have separate semantic owners. ABI and package decoding deliberately share one internal bounded `Cursor` instead of duplicating framing logic. Existing package/authentication/runtime APIs remain root re-exports.

**[AUDIT STATUS]** `kernel-auth` and `kernel-deployment` are now both security-hostile-audited (P277) and structurally split (P278). `kernel-schema` has its repeated-subtype-traversal cluster closed by a first-class maintained closure authority. `kernel-validation` has both the P275/P277 correctness fixes and coherent physical ownership after P278. Remaining high-value work is concentrated in structural closure of `kernel-grounded-closure`, whole-crate closure of `kernel-persistent`, and evidence-driven final passes over `storage-memory` / `kernel-model`.

**[NO FALLBACK]** P278 introduces no threshold routing, SQL-shaped generic fallback, error-driven alternate algorithm, or duplicate subtype engine. Schema closure is the sole subtype reachability authority used by downstream validation preparation.

### Pass279 — grounded/persistent structural closure and revision-graph authority

**[STRUCTURE / CLOSED] Grounded closure now has explicit mathematical owners.** The unified witness-indexed calculus from P275 is preserved as the only mutation calculus, but the former monolithic production owner is physically separated into `index.rs` (incidence and witness authority), `solver.rs` (least grounded closure and independent certificate checker), `maintenance.rs` (incremental update/structural reconciliation), and `bipolar.rs` (greatest-support dualization/maintenance). The root owns program/certificate vocabulary plus tests. No second generic solver, threshold route, or compatibility fallback was introduced.

**[PERFORMANCE / CLOSED] Persistent ordered retain is one persistent tree traversal.** `PersistentOrdMap::retain` no longer collects every removed key and invokes path-copying AVL removal once per key. A recursive persistent retain evaluates the predicate in-order exactly once, returns the original `Arc` for untouched subtrees, and joins changed AVL subtrees with height-aware persistent `map_join`. `retain-all` preserves the exact root without cloning values; bulk filtering remains balanced and preserves snapshots. `PersistentOrdSet::retain` inherits the same authority.

**[STRUCTURE / CLOSED] Persistent collection ownership is explicit.** Ordered AVL map/set ownership lives in `ordered.rs`; page/radix vector ownership lives in `vector.rs`; the crate root is a small public façade plus white-box regressions. The P276 page-wise remove/resize/swap semantics are unchanged.

**[CORRECTNESS / CLOSED] Revision graph queries reject nonexistent coordinates.** `storage-memory` previously treated an unknown revision as its own synthetic ancestor because traversal seeded the requested id before establishing store membership. Consequently `unique_merge_base(unknown, unknown)` could return an id that was not a revision. `ancestors_including` now returns no ancestry for unknown ids, `is_ancestor` requires both endpoints to exist, and `unique_merge_base` fails explicitly with `UnknownRevision` before LCA analysis. The P274 linear LCA-frontier algorithm remains unchanged.

**[HOSTILE INVENTORY]** `kernel-model` received a dedicated whole-owner sweep over COW relation storage, nested live-reference sensitivity, lifecycle restriction, dense identity reuse, relation-only recompile, and normalization. No new correctness, duplicated-authority, fallback, or proven asymptotic seam was found. Its remaining debt is physical ownership only: the ~700 production LOC root still combines COW/relation storage, value algebra, live-reference index, and normalized database state.

**[STATUS]** `kernel-grounded-closure`, `kernel-persistent`, and `storage-memory` are COMPLETE / FROZEN after their previous semantic/performance work plus the P279 closure sweep. Reopen only on new evidence. `kernel-model` is DEEP-AUDITED and becomes the primary structural target for the next pass.

## Pass280 addendum — final schema/model ownership and global kernel-refactor closure

**[STRUCTURE / MODEL CLOSED] `kernel-model` now has explicit semantic owners.** The P279 whole-owner hostile audit found no new semantic defect, so P280 performs only the ownership split justified by the existing architecture: `storage.rs` owns COW containers and persistent relation-row append patches; `value.rs` owns the recursive value algebra and live-reference traversal; `live_refs.rs` owns compiled live-reference sensitivity; `state.rs` owns finite-model lifecycle restriction and normalized database-state authority. The root is an API/test façade. Internal sharing/sensitivity details remain crate-internal rather than becoming new public capabilities.

**[R&D / SUBTYPE AUTHORITY] Transitive subtyping is now maintained bidirectionally.** P278 removed repeated subtype BFS from query/validation reads, but `SubtypeClosure::include` still found existing descendants by scanning the entire ancestor map. P280 replaces that residual reconstructible graph work with one maintained pair of transitive relations: `ancestors` and `descendants`. For an accepted inclusion `S ⊑ T`, the closure computes the already-known lower cone `Desc(S) ∪ {S}` and upper cone `Anc(T) ∪ {T}`, then installs their Cartesian implication into both authorities. There is no threshold, graph-scan fallback, memoization side path, or validation-specific subtype engine. A hostile late-bridge regression proves that connecting two previously built chains immediately derives every lower-to-upper implication.

**[STRUCTURE / SCHEMA CLOSED] `kernel-schema` is split by semantic responsibility.** `types.rs` owns type expressions and guarded-recursion validation; `definitions.rs` owns capability/field/relation/structural semantic definitions; `subtype.rs` owns the maintained transitive subtype authority; `schema.rs` owns schema mutation/equivalence/dependency laws; `context.rs` owns semantic-environment/context authority. The root is a façade plus regressions. Public API behavior is preserved through re-exports.

**[GLOBAL HOSTILE REVALIDATION] Historical heavy kernels remain closed on current evidence.** P280 re-ran hostile inventory against current production sources and full regression gates for `kernel-query`, `kernel-plan`, `kernel-semantics`, `kernel-durability`, `kernel-change`, and `kernel-integration`. The old `kernel-plan` generic-row paths were specifically rechecked against the Pass194 closeout: they are Γ-aware universal correctness implementations selected when specialized typed/persisted physical implementations are unavailable, not SQL nested-loop substitutions, error-catching control flow, or hidden row-count-quadratic payers. Replacing implementation specialization merely to eliminate the word “fallback” would remove useful physical specialization without establishing a correctness or measured performance gain. No new falsifying case was found.

**[GLOBAL STATUS] The kernel hostile/global refactor campaign is COMPLETE / FROZEN after P280.** Every kernel has either a dedicated hostile closure pass or a grouped hostile audit proportional to its size, and the historically closed heavy kernels were revalidated against the current workspace after the later cross-kernel changes. Frozen means “do not continue cleanup by inertia”: any future reopen requires a concrete correctness counterexample, proof/authority seam, measured asymptotic/performance regression, new R&D requirement, or API/DX requirement. It is not a claim that undiscovered bugs are impossible.


## Productization handoff after Pass280 — universal external facade roadmap

**[CURRENT PRODUCT PHASE]** После global kernel freeze проект переходит от kernel-cleanup к productization. Authoritative product surface строится Rust-first в `cfmd-runtime`; Python/.NET/Studio являются последующими adapters над тем же protocol и не обращаются к kernel graph напрямую.

**[DX LAWS]** Материализованные Python objects не выполняют hidden I/O; deep relationship traversal разрешён внутри symbolic query construction; many-valued paths требуют явной `any/all/match/aggregate` семантики; current (`db`), historical (`db.at(revision)`) и speculative (`db.preview(plan)`) worlds различаются типом контекста.

**[QUERY/WATCH CONTRACT]** Familiar lazy query vocabulary (`match/where/select/order/aggregate`) компилируется в compact typed IR и существующий kernel. Exact `watch()` означает подписку на результат выражения и revision-tagged exact delta, а не table callback/polling. Если exact derivative отсутствует, capability обязана fail explicitly; recompute mode может существовать только как явный opt-in.

**[PLAN/CANDIDATE/HISTORY]** Advanced writes сначала существуют как inspectable Plan. `Candidate = base revision + proposed rewrite` является queryable future world и должен поддерживать validation, `delta(query)`, explanation, freshness/rebase и commit через тот же authoritative transition pipeline. History inverse/undo создаёт Plan, а не обходит validation/publication.

**[ONE RUNTIME]** Embedded application и external CFMD Studio/tooling не являются независимыми writers файла. App-owned/runtime service остаётся единственным authoritative writer; local tooling подключается через capability-protected language-neutral protocol (query/watch/revision/history/Plan/Candidate).

**[ROADMAP AUTHORITY]** Подробная facade theory хранится в `docs/api/CFMD_PYTHON_FACADE_THEORY.md`; active sequencing — в `docs/api/PRODUCT_ROADMAP.md`; supporting Rust runtime boundary — в `docs/api/RUST_API_ROADMAP.md`. Эти документы определяют следующий development phase, а не продолжают kernel cleanup без нового evidence.


## Pass281 product boundary — Rust-runtime-first facade

**[CURRENT PRODUCT AUTHORITY]** Product sequencing is now Rust-runtime first. The Python facade theory remains a UX target, but no Python/.NET/Studio binding may call `kernel-*` crates directly. The stable semantic/runtime boundary is the new `crates/cfmd-runtime` crate.

**[P281 IMPLEMENTED]** `cfmd-runtime` owns public IDs, recursive values, relation query IR, immutable read snapshots, prepared query authority, relation Plans, durable transaction identity and product error categories. Internal `SemanticId`, `RelExpr`, `RuntimeRevisionSnapshot`, `RelationDelta`, durability errors and crate topology stay private.

**[END-TO-END GATE]** A real durable runtime is created in the test fixture and closed; all subsequent open → snapshot → prepare → execute → Plan commit → new-revision query operations are performed only through `cfmd-runtime`.

**[NEXT]** P282 should add facade-owned schema/type/relation builders and `Database::create`, then Candidate/history/watch on the same Rust boundary before language bindings.


## Pass282 product boundary — create/schema authority

**[RUST PRODUCT AUTHORITY]** `cfmd-runtime` now owns database creation inputs as well as open/query/write operations. External Rust applications and future language bindings do not construct `kernel_schema::Schema`, `SemanticEnvironment`, `SemanticRegistry`, `PhysicalStore`, `RuntimeRevisionBundle`, or revision objects directly.

**[P282 IMPLEMENTED]** The facade provides recursive product `Type`, primitive equivalence/ordering contracts, `RelationSchema`, `SchemaBuilder`, `Database::create`, and `ReadContext::schema`. Creation compiles facade schema authority into the kernel graph once and persists the resulting semantic registry through the existing durable runtime.

**[EMPTY DATABASE LAW]** Empty relations are physically instantiated through typed empty native columns derived from declared relation types. Product creation does not use sentinel rows, delayed first-write typing, or type-specific fallback routing.

**[BOUNDARY LAW]** The universal low-level Rust facade remains data-oriented and binding-friendly. Future generated/typed Rust DX is layered above it and must lower to the same facade IR rather than bypassing it or duplicating semantics.

**[REPOSITORY RECONCILIATION]** GitHub CI exposed that `.gitignore` pattern `core.*` also ignored Rust files named `core.rs`. Repository policy now uses `core.[0-9]*` for numbered crash dumps, preserving source files while retaining crash-artifact filtering.


## Product boundary addendum — Pass283

The frozen kernel graph remains behind `cfmd-runtime`. Pass283 adds an idiomatic typed Rust adapter without creating a second query/write semantics: typed relation/field handles lower to the existing facade `Query`, `PreparedQuery`, `Plan`, `Value` and `Row` protocol. Equality authority is inherited from relation schema. `ValueCodec`/`RowCodec` are extension points for generated domain types. Deep relationship navigation is not authorized by a plain join alone; a future product-schema reference contract must state target identity and cardinality before object-like paths can be exposed.


## Pass284 object-first Rust product model

**[PRODUCT DX AUTHORITY]** Object-first is the primary native Rust application model. Relation-first remains the universal low-level/dynamic/tooling surface and the lowering target; there is still only one query/write semantics.

**[P284 IMPLEMENTED]** `cfmd_object!` maps a Rust domain object with one stable textual key and Rust fields to deterministic product semantic identifiers, field equivalences and one typed CFMD relation. `SchemaBuilder::object::<T>()` therefore requires no application-visible relation IDs or column indices. Snapshot-bound `ObjectSet<T>` exposes symbolic field accessors, typed filters/projections and whole-object materialization.

**[PLAN LAW]** Read construction returns Query values. Write operations over object sets/queries (`insert`, exact-query `update`, `delete`) return proposed `Plan` values. `Plan` is therefore an inspectable/composable transition value rather than the primary mutable command surface. The low-level mutable builder remains an escape hatch for dynamic tooling. Plans are bound to one open database instance and exact source snapshot; cross-database commit/composition fails closed.

**[IDENTITY ROADMAP]** P284 deliberately does not pretend that a plain relation column is an entity identity/reference. P285 must add first-class object identity, `Ref<T>` / optional/many cardinality authority and lifecycle-safe write semantics before exposing deep paths such as `u.passport().country().code()`.


### Pass285 product-layer entity contract

The external Rust product layer may declare identity-bearing entities independently of kernel lifecycle carriers. `Id<T>` and `Ref<T>` are encoded through the existing typed historical-entity identity domain; they do not create hidden object I/O or a second storage model. Entity plans carry a product-level contract: identity values are unique and every strong reference resolves in the final composed plan state. Validation is identity-indexed (`BTreeMap`/`BTreeSet`), not reference-by-target scanning. Deep reference predicates lower to existing equality joins, then project and semantically distinct the root shape, giving existential path semantics without multiplicity leakage. Kernel relation/query semantics remain unchanged.

## Pass286 product-layer addendum — explicit object cardinality

The Rust product facade now distinguishes required references, optional references and reverse-many relationships without introducing ORM-style hidden loading. `Option<Ref<T>>` is a real algebraic option value backed by the kernel structural-equivalence calculus. Reverse-many members are symbolic query relationships only and are not fields of materialized Rust objects.

Collection predicates lower to the existing relational algebra: `any` uses join/project/distinct, `none` uses anti-join, `all` uses anti-join against the difference between the target domain and the matching target subset, and `count().eq(n)` uses grouped exact count (`n = 0` is the anti-join law). No threshold routing or recompute fallback is introduced.

Open product/backend obligation: current strong-reference metadata is owned by `cfmd-runtime`; it is sufficient for mutations whose source object contract is present in the Plan, but it is not yet the durable global incoming-reference authority after reopen. The next lifecycle pass must unify object identity/reference with kernel carrier + `LiveEntityRef` semantics rather than add further facade-side scans.

## Pass287 lifecycle-backed object identity/reference authority

**[P287 IMPLEMENTED]** `cfmd-runtime` no longer treats strong-reference existence checking as a facade-only scan. Object relation rows remain the canonical product/query representation, but identity-bearing object writes derive a kernel semantic projection consisting of carrier membership, lifecycle roots/entities, and schema-declared reference fields.

**[IDENTITY SEPARATION]** Public `Id<T>` is type-local and remains represented in relation/query space as typed historical identity. Kernel lifecycle requires a global `EntityId`; the product runtime derives a stable internal surrogate from `(TypeId, external id)` and collision-checks it. This permits `Id<User>(7)` and `Id<Passport>(7)` without lifecycle aliasing.

**[REFERENCE LAW]** Required and optional object references are mirrored into kernel fields as `LiveEntityRef<T>` / `Option<LiveEntityRef<T>>`. Target Revision construction goes through `kernel_revision::Revision::build`; surviving references to deleted targets are rejected by kernel-model dangling-reference validation. This authority is durable and survives reopen.

**[PUBLICATION]** Because an entity mutation changes lifecycle/carriers/fields in addition to relation rows, P287 uses the existing correctness-first durable full-revision publication path for entity Plans. Dynamic relation-only Plans continue to use the compact derived-relation path. No error-driven fallback or threshold routing is introduced.

## Pass288 — mixed semantic revision, incremental physical publication

**[KERNEL-PLAN AUTHORITY]** `MixedRevisionTransitionRequest` is the general transition surface for a pinned semantic context when a revision changes both relation data and non-relation model state (lifecycle, carriers, fields). The target `Revision` remains the already-validated logical authority; supplied `RevisionRelationMutation`s must exactly explain all touched relation endpoints and untouched relations must remain identical.

**[PHYSICAL LAW]** Mixed publication clones and incrementally mutates the existing `PhysicalStore`, advances only touched relation witnesses, and maintains only affected materializations. It must not reconstruct the complete physical store merely because lifecycle/carrier/field state changed.

**[VALIDATION LAW]** Relation-delta endpoint equality is checked independently of non-relation state. Because lifecycle/carrier/field changes can affect global invariants, the mixed path rebuilds the runtime violation measure from the exact target Revision instead of transporting a relation-only violation certificate.

**[DURABILITY LAW]** P288 keeps the exact full target Revision in the durable transaction intent/change payload. Live publication is incremental; recovery can therefore reconstruct the exact target without a new WAL codec. Compact durable encoding of the non-relation delta remains an optimization payer and must be extensionally identical to the P288 target Revision before replacing this representation.

## Pass292 — product history as a durable causal projection

**[VERIFIED]** Product history no longer requires a prospective parallel journal. `kernel-durability` exposes the exact committed records already contained in the validated revision-effect ideal, `kernel-plan` normalizes those records into a runtime history bridge, and `cfmd-runtime::Database::history()` maps the bridge to product-owned identifiers/values. Causal prerequisites are retained explicitly; history listing is therefore a deterministic topological view of the same Γ-REIC authority rather than an invented linear log.

**[VERIFIED]** Exact relation-representable history inversion is an ordinary Plan. For the live head entry, inserted/removed relation rows are swapped against a fresh current snapshot, then the resulting Plan uses the existing `Plan -> Candidate -> durable commit` path. No undo publication primitive exists. A committed inverse is itself a normal effect, so repeating inversion over that head gives redo. Stale historical entries fail closed and require a future rebase calculus.

**[R&D PAYER / OPEN]** `DurableModelDelta` is forward-exact but not generally involutive: target-only field patches do not carry enough source information to reconstruct every prior model/lifecycle endpoint after checkpoint rotation. P292 therefore classifies mixed effects with non-empty model delta as `ComplementRequired`; it does not route them through relation-only reconstruction. The next history R&D block should define an exact compact mixed-transition complement/involution and preserve it in the same durable transaction authority.

## Pass295 — exact non-head history inverse rebase

**[PRODUCT DX]** `HistoryEntry::undo_plan()` is no longer restricted to the live-head transition. When the entry is older than HEAD, the runtime asks the kernel for an exact rebase certificate. If every intervening committed effect is semantically independent, the historical inverse is rebuilt as an ordinary Plan bound to the current snapshot and continues through the existing `Plan -> Candidate -> durable commit` pipeline. `HistoryEntry::undo_readiness()` exposes `Ready`, certified `Rebased`, structured `Conflict`, `RuntimeClosed`, `NonReversible`, or `Unavailable` state before Plan construction.

**[KERNEL R&D]** `kernel-plan` now owns `RuntimeHistoryFootprint` and the non-head inverse transport certificate. Relation coordinates are exact Γ-canonical relation classes produced by the pinned semantic relation-delta calculus; they are not Rust hashes, physical row positions, or relation-wide SQL locks. Mixed/model coordinates name carrier presence/membership, field owner, lifecycle entity/root, and keeps-alive presence/edges. This permits strong commutation proofs for independent writes even inside one relation or object family.

**[FAIL-CLOSED LAW]** A historical inverse is transported only when its exact write footprint is disjoint from every intervening exact effect. Overlapping semantic coordinates return `HistoryRebaseConflict`. Full/schema/legacy or otherwise opaque intervening effects are conflicts rather than optimistic replay. Candidate/Revision validation remains the final invariant authority after transport; no generic three-way merge, last-write-wins rule, snapshot diff fallback, or error-driven routing exists.

**[DURABILITY]** Rebase is derived entirely from the retained durable causal effect ideal plus P293 forward/reverse complements and P294 historical reconstruction. No extra rebase journal, undo stack, historical snapshot cache, or duplicated state authority is persisted. Regression coverage includes reopen followed by non-head undo, disjoint Γ-classes in the same relation, same-class conflict, and independent object/entity coordinates.

## Pass296 — exact product watch over maintained differential state

**[PRODUCT DX]** Live snapshot queries now expose exact subscriptions: raw `ReadContext::watch(&Query)`, `ObjectQuery::watch()`, and typed projected-object `.watch()`. A watch returns its initial result once and then revision-tagged delta events `(source_revision, target_revision, inserted, removed)`. `try_recv()` is non-blocking; `recv()` blocks without requiring Tokio. `db.at(revision)` remains immutable historical state and cannot open a live subscription.

**[KERNEL QUERY AUTHORITY]** Watch does not derive change by evaluating the query twice. Subscription creation builds the existing `kernel-query::MaterializedRelPlanState`; every relevant committed `RelationDelta` is then propagated through the already-certified differential/ExecGraph machinery. Unrelated committed relation changes advance the watch revision but produce an empty result delta when the maintained query is unaffected.

**[WAKE LAW]** `DurableRuntime` owns an in-process monotone publication generation plus `Condvar`. This is explicitly wake-only reconstructible state, not a second history/event authority. After wake the watch reads the durable causal effect chain and derives the next exact transition from the authoritative committed effect. A missed/spurious wake cannot invent data and backlog is consumed from causal history in revision order.

**[FAIL-CLOSED LAW]** Watch refuses historical contexts, unavailable exact causal paths, and opaque/full/schema/legacy transitions that cannot be transported by the current exact relational delta contract. There is no polling, whole-result recomputation fallback, filesystem timestamp heuristic, or error-driven routing.

**[OPEN TRANSPORT]** P296 wake-up is in-process for one `DurableRuntime`. Cross-process Studio/WPF/PyQt/external-tool reactivity remains an explicit transport payer: an OS/service notification layer must wake readers, after which the same durable effect + maintained-query protocol is used. Polling is not accepted as the exact-watch implementation.

## Pass297 — wake-provider and hosted-ingress boundary

**[VERIFIED]** Exact watch wake-up is no longer coupled to one concrete `Condvar`. `kernel-plan::RuntimeRevisionPublicationNotifier` is a wake-only provider interface and the standard in-process implementation retains the cheap monotone-generation + condition-variable behavior. `cfmd-runtime` owns the public `PublicationNotifier` vocabulary and bridges providers without leaking kernel types. P347 strengthens this boundary: executor `Waker` registration is owned by `PublicationNotifier` itself rather than by a bridge-local sidecar, so direct provider liveness/spurious signals wake blocking and async subscribers through one notification authority.

**[AUTHORITY LAW]** Notification is not mutation, history, or writer-resolution authority. A provider may duplicate/coalesce/spuriously emit wakes; a subscriber must recover the authoritative committed transition from Revision + durable causal history. The corresponding Lean model proves finite arbitrary wake repetition preserves authoritative Revision. Runtime E2E independently injects a spurious wake and observes no fabricated watch event.

**[HOSTING BOUNDARY]** Hosted/local-tool transports are adapters above the database runtime. Opening/creating a database does not bind IPC/TCP, trust localhost, or authenticate external principals. Optional first-party providers and third-party/application providers may implement wake/delivery integration, but semantic multi-writer commute/rebase/conflict certification remains kernel authority and cannot be asserted by a transport provider.

**[HOSTED WATCH PROTOCOL]** A hosted watch subscription is session-scoped protocol state over the ordinary exact runtime watch, not a second event authority. Open/next/status/cancel/close/session-close operations cannot manufacture database revisions; event source/target revisions and result deltas come only from committed durable causal effects. Hosted implementations must bound maintained subscriptions and must make session termination capable of cancelling blocked consumers. Concrete wire framing and transport remain outside the kernel/runtime authority.


## Pass302 — hosted wire/framing boundary

**[WIRE FRAMING LAW]** Hosted wire framing is transport metadata above `cfmd-protocol`, not database authority. Wire v1 has a fixed-size header containing magic, framing version, frame kind, reserved flags, request identity and bounded payload length. A conforming transport must validate the fixed header and payload bound before allocating/reading the announced payload. Request identity is correlation metadata only.

**[NEGOTIATION LAW]** A wire session must negotiate a hosted protocol version before ordinary requests. The selected version must lie in the client-offered range and be server-supported; capability negotiation is an intersection, never a permission grant. Current wire v1 selects hosted protocol v2 when compatible.

**[CANONICAL DECODE LAW]** Wire payloads are deterministic big-endian tagged encodings. Collection/string sizes, recursive depth and total decoded nodes are bounded. Unknown tags, invalid canonical discriminants, duplicate product fields, truncated payloads and trailing bytes fail closed. The protocol does not silently skip unknown mutation/security fields; evolution is explicit through version/capability negotiation.

**[AUTHORITY LAW]** `WireHostedSession` may dispatch only through an already-authorized `HostedSession`. Framing and negotiation cannot authenticate a peer, expand P299 grants, mutate Revision, certify multi-writer commutation, or create event authority. IPC/TCP/TLS/QUIC remain optional adapters above this boundary.

## Pass303 — hosted server composition authority

**[VERIFIED]** `cfmd-host` composes `Database -> SessionDatabase -> HostedSession -> WireHostedSession` without exposing unrestricted `Database` to transport adapters. Authentication and authorization are distinct provider boundaries: evidence establishes identity; grants come exactly from the configured authorizer.

**[SECURITY LAW]** Transport/locality metadata carries no implicit authority. Hosted connection identity is correlation/lifecycle metadata only. Writer conflict/rebase certificates remain kernel-owned and cannot be asserted by host/transport providers.

**[RESOURCE/LIFECYCLE LAW]** Active connections and per-connection in-flight requests are bounded. Connection/server close releases admission capacity once and closes the hosted session, thereby cancelling blocked exact-watch consumers without polling. Closing a connection cannot manufacture a database Revision.

## Pass304 — hosted security lifecycle authority

**[VERIFIED]** Hosted session authorization is a shared runtime authority, not copied endpoint metadata. All derived restricted product values retain the same session identity and consult its current grants at authority transitions. Grant refresh therefore reaches existing values; terminal revocation cannot be undone by later refresh.

**[SECURITY LAW]** Channel binding is explicit authentication evidence (`Bound` or `Unbound`), never an implicit grant. Authorizers may issue expiring grants. Host expiry scheduling is external/event-loop friendly (`next_expiration` + `expire_due`) and requires no polling or hidden server thread; expiration revokes the session and wakes blocking watch work.

**[LIFECYCLE LAW]** Graceful drain rejects new connections but does not revoke already-admitted sessions. Immediate close/revoke is distinct. Security lifecycle transitions preserve authoritative database Revision and do not certify writer commutation/conflicts.

## Pass314 — CFMD AE v1 / AES-256-GCM-SIV storage encryption

**[PRODUCT ENCRYPTION]** The default single-file product path may be created/opened with `DatabaseBuilder::encryption(Encryption::aes256_gcm_siv(key))`. Encryption is a storage concern, not hosted-user authentication. `Database::create/open` remain plaintext sugar unless an encryption policy is explicitly supplied through the builder.

**[PRIMITIVE]** CFMD AE v1 uses AES-256-GCM-SIV (RFC 8452) through the pure-Rust RustCrypto implementation. The format records an algorithm identifier and random per-database salt, never the master key. HKDF-SHA256 derives independent section and WAL keys. Key bytes are held behind zeroizing shared storage and are redacted from `Debug`.

**[PHYSICAL BINDING]** Immutable generation sections authenticate `(generation, section kind, ordinal)` as AEAD associated data. WAL payloads authenticate `(record kind, LSN, revision)` as associated data. Ciphertext envelopes carry a random 96-bit nonce and 128-bit authentication tag; payload CRC/digests remain corruption/local-integrity framing over stored ciphertext, while AEAD is the cryptographic authority over payload contents and physical identity.

**[CRASH LAW]** WAL frames remain independently appendable and recoverable. Torn-tail truncation, exact-frame shadow/carry-forward and single-file compaction preserve ciphertext bytes; recovery decrypts only complete CRC-valid frames and fails closed on AEAD authentication failure. AES-GCM-SIV misuse resistance is defense-in-depth for accidental nonce reuse, not permission to deliberately reuse nonces.

**[ROLLBACK SEPARATION]** AEAD does not make an old complete database snapshot invalid. Whole-file rollback remains the external-freshness obligation already owned by the durability layer. Cryptographic authentication failure is corruption and never authorizes fallback to an older root.

**[CRYPTO AGILITY]** `StorageAeadAlgorithm` is versioned format vocabulary rather than a routing heuristic. AE v1 implements one production primitive, AES-256-GCM-SIV. Future primitives may receive new algorithm identifiers while preserving the same storage-encryption contract; no error-driven primitive fallback is allowed.

**[CURRENT SCOPE]** P314 product encryption is implemented for the default `SingleFile` backend. Supplying encryption with explicit `Storage::Directory` fails closed until the directory backend is given the same codec contract; it never silently creates plaintext storage.

## Pass315 — CFMD AE v1 bounded sections / nonce namespace / key-provider boundary

**[BOUNDED AUTHENTICATED SECTIONS]** New encrypted generation sections are encoded as a versioned `CFSC` chunk stream with fixed 64 KiB plaintext chunks. Each chunk is an independent CFMD AE v1 envelope and its AAD binds `(generation, section kind, ordinal, chunk index, total plaintext length, chunk plaintext length)`. `copy_section_to` authenticates a chunk before releasing that chunk and uses bounded ciphertext/plaintext memory independent of total section length. Pre-release P314 whole-section section envelopes are not retained as a compatibility surface; the chunked layout is the sole current encrypted-section representation and unknown layouts fail closed.

**[NONCE NAMESPACE]** WAL and generation writers no longer call the OS CSPRNG for every AEAD message. A writer obtains an 80-bit random namespace and emits a 16-bit counter in the remaining nonce bits, then obtains a fresh namespace after 65,536 messages. Nonces are therefore exact and unique inside one namespace; cross-namespace collision remains probabilistic and AES-256-GCM-SIV misuse resistance is retained as defense in depth. This is not a replacement for future key-epoch/usage-limit policy.

**[KEY PROVIDER]** `cfmd-runtime` exposes `EncryptionKeyProvider` and `EncryptionKeyOperation::{Create,Open}`. `DatabaseBuilder` resolves provider key material only at the lifecycle boundary, then passes the same typed storage-encryption contract into the kernel. The raw 256-bit key constructor remains the minimal adapter. P315 does not yet claim wrapped random database-master-key/password-KDF lifecycle; that remains a separate key-management obligation.

**[FRESHNESS PARITY]** Low-level single-file create and external-freshness-aware open now accept the same `StorageEncryption` configuration. Freshness preflight opens/authenticates encrypted metadata with the supplied key before deriving external freshness material. There is no plaintext probe/fallback for an encrypted externally anchored store.


## Pass316 — wrapped database-master-key lifecycle / crash-safe rewrap

**[WRAPPED DMK]** Provider-backed AES-256-GCM-SIV databases no longer use provider material as the database encryption key. Creation generates a random 256-bit Database Master Key (DMK). The provider supplies a Key Encryption Key (KEK); CFMD derives a dedicated DMK-wrap key with HKDF-SHA256, wraps the DMK with AES-256-GCM-SIV, and persists only wrapped DMK material plus provider identity/epoch. Section/WAL keys continue to derive from the DMK and per-database salt, so KEK rotation does not change data ciphertext.

**[KEY IDENTITY]** `EncryptionKeyProvider` now resolves one atomic `EncryptionProviderKey` snapshot for `Create`, `Open`, or `Rewrap`: KEK bytes, a non-zero 128-bit `EncryptionKeyId`, and a non-zero provider key epoch. Wrapped-key AAD binds database salt, provider key ID, provider key epoch, database key epoch and wrapped-key publication sequence. Wrong KEK bytes or substituted provider/epoch/publication metadata therefore fail authentication.

**[DUAL-SLOT PUBLICATION]** The current pre-release single-file header reserves two independently checksummed wrapped-key slots. A rewrap writes the inactive slot with strictly incremented publication sequence and database key epoch, then durably syncs it. Recovery selects the highest valid sequence; a torn/incomplete new slot leaves the prior valid slot recoverable, while a fully valid newer slot is authoritative and is never error-fallback-routed to the older slot. The data generation/WAL region is not rewritten by rewrap.

**[ROTATION DX]** An opened product database may call `Database::rewrap_encryption(...)`. Provider-backed rotation resolves the new KEK at `EncryptionKeyOperation::Rewrap`, wraps the already-unlocked DMK, publishes the next wrapped-key slot, and returns the new database key epoch. Raw direct-key encryption remains a minimal adapter and is intentionally not advertised as rotatable wrapped-key management. Internal pre-release pass layouts are not retained as an on-disk compatibility promise.

**[ROLLBACK BOUNDARY]** Dual-slot publication closes local torn-write/crash recovery for key metadata. It does not by itself prove anti-rollback against an attacker restoring an older complete header/file image. Provider revocation/epoch policy and the existing external-freshness authority remain the mechanisms that can reject obsolete external key state or whole-file rollback.

## Pass317 — provider key-authority floor / pre-release layout cleanup

**[PRE-RELEASE FORMAT POLICY]** CFMD has not declared a released on-disk compatibility boundary. The P316 header-only `v4` split and its synthetic P314/P315 header compatibility branch were therefore removed. Header/root/generation records use the one current single-file format version. Version markers remain fail-closed format identifiers for future released evolution; they are not a reason to carry R&D-pass migration code today.

**[DATABASE-KEY EPOCH FLOOR]** `EncryptionProviderKey` may carry a non-zero minimum accepted database-key epoch. The provider is queried outside the database file and therefore acts as external key authority. Wrapped-key open rejects an otherwise valid authoritative slot when its database-key epoch is below that floor, before DMK unwrap. This closes complete-header rollback once the external provider has durably advanced its floor, even if the old KEK bytes remain available. The default constructor admits epoch 1 for simple providers; security-sensitive providers can raise the floor with `with_minimum_database_key_epoch(...)`.

**[ROTATION HANDOFF]** `Database::rewrap_encryption(...)` returns the newly published database-key epoch. A provider may durably raise its external floor to that returned epoch after successful rewrap. A requested floor above the epoch being created/rewrapped fails closed; create requires admission of epoch 1. This separates crash-safe in-file dual-slot publication from anti-rollback authority without trusting a minimum epoch stored in attacker-controlled database bytes.


## Pass318 — acknowledged provider handoff / recoverable key-authority transition

**[ACKNOWLEDGED HANDOFF]** Provider-backed rewrap is an explicit three-stage authority transition: publish and fsync the successor wrapped-DMK slot; durably acknowledge the resulting database-key epoch through `EncryptionKeyProvider::acknowledge_database_key_epoch(...)`; only then retire the predecessor slot. The acknowledgement binds provider key ID, provider key epoch and database-key epoch. Failure of external acknowledgement is surfaced after local publication and the operation is retryable rather than silently reported as fully committed.

**[PENDING AUTHORITY RECOVERY]** A physically newer wrapped-key slot is not by itself external authority. Open selects the newest admissible slot matching the provider snapshot and its minimum database-key epoch. Therefore a crash after local successor publication but before external acknowledgement can still reopen under the previously acknowledged provider, while the successor provider can adopt the already-durable pending slot. Rewrap retry detects that exact consecutive pending slot and reuses it without publishing a new wrap or incrementing the database-key epoch.

**[CONSECUTIVE TRANSITION LAW]** Recovery may adopt a pending successor only when both wrapped-key publication sequence and database-key epoch are exactly one greater than the currently admitted predecessor. A non-consecutive matching slot is corruption, not a routing hint. This keeps retry/recovery deterministic and prevents hidden jumps in key authority.

**[PREDECESSOR RETIREMENT]** After external acknowledgement succeeds, CFMD zeroes the obsolete wrapped-key slot and durably syncs the header. A crash before retirement is safe because the external minimum database-key epoch already rejects rollback to the predecessor; a crash after retirement leaves only the acknowledged successor. Retirement is idempotent for the acknowledged active epoch and cannot be applied to a different active epoch.

**[FAILURE ORDERING]** The supported ordering is `local successor durable -> external acknowledgement durable -> local predecessor retirement`. Advancing external authority before local successor durability is forbidden because it could make the only recoverable local key inadmissible. Retiring the predecessor before external acknowledgement is forbidden because a crash could strand the database between independent durability domains.

## Pass319 — bounded-memory encrypted generation publication / footer descriptor table

**[STREAMING PUBLICATION]** Generation publication MUST NOT materialize ciphertext for an entire section or generation before file publication. The writer reserves the exact generation extent, emits the fixed generation header, then streams each section directly to its final physical offset. Encrypted sections are sealed one 64 KiB AEAD chunk at a time; the only ciphertext allocation proportional to payload is one bounded chunk envelope. Unencrypted sections are written directly from the caller-provided slice. Section digests and the generation digest are updated while bytes are emitted.

**[FOOTER DESCRIPTORS]** Section descriptors are a footer after all section payloads rather than a prefix that would require ciphertext digests before payload emission. The fixed header records `data_start`, `section_table_offset`, `section_table_len` and `total_len`. Section offsets remain page-aligned and MUST end at or before `section_table_offset`; the descriptor table is emitted only after all section digests are known. The generation tail is padded to the next page before WAL begins.

**[ONE-PASS PHYSICAL DIGEST]** The published generation digest is computed in physical byte order during the same write that creates the generation: header, deterministic zero padding, section bytes, alignment padding, footer descriptor table and final padding. Publication does not reread the generation merely to calculate its digest. Root publication still occurs only after the complete generation has been written and durably synced, so a crash during streamed generation construction leaves only non-authoritative orphan bytes.

**[BOUNDED MEMORY LAW]** Generation publication memory is independent of section payload size. It is bounded by one AEAD chunk envelope plus metadata proportional only to the explicitly bounded section count (`MAX_SECTION_COUNT`). The removed `StoredSection`/whole-section staging path is not a fallback and is not retained as pre-release compatibility code.

## Pass320–Pass323 — end-to-end bounded canonical storage streams

**[CANONICAL STREAM LAW]** Checkpoint and metadata serialization have one canonical grammar shared by buffered, exact-length and streaming sinks. Generation publication consumes `SingleFileSectionSource` values with an exact plaintext length; a byte-slice is only an adapter to that maintained path. Large checkpoint/metadata payloads are not required to exist as one contiguous plaintext or ciphertext allocation.

**[RESUMABLE CHECKPOINT LAW]** Directory resumable checkpointing may use a generation-scoped canonical spool because publication spans multiple calls, but the spool is never recovery authority. It is produced once, consumed monotonically, integrity-bound to the original canonical stream, and deleted/ignored as orphan scratch after failure or recovery. Restarting canonical encoding from byte zero for every chunk is forbidden.

**[BOUNDED RECOVERY LAW]** SingleFile checkpoint/metadata recovery consumes a pull-based `BinarySource`. Plaintext sections use bounded read windows; encrypted sections authenticate each complete CFSC chunk before any bytes from that chunk are exposed to the canonical decoder. The same decoder grammar serves slice and streaming sources; trailing, truncated or unauthenticated bytes fail closed.

**[REPLICATION ARCHIVE COMPOSITION]** Replication-authority rotation is a composite stream, not a concatenated archive buffer. The new generation's replication section is the exact sequence `previous authoritative archive || frozen live-frame prefix`. The previous section is read through an independent bounded snapshot reader and live frames are emitted individually. Streaming-checkpoint cuts freeze the prefix by frame count; frames appended after the cut remain the live suffix and are not duplicated into the checkpoint generation. Whole-archive materialization is not a fallback path.

**[REPLICATION COPY-AMPLIFICATION OPEN OBLIGATION]** Bounded streaming removes archive-sized memory but does not certify asymptotically bounded rotation cost. P323 still rewrites the retained replication-authority prefix into each successor generation because that prefix is current recovery authority. A future compaction/snapshot calculus must prove which replication authority state is sufficient to replace historical frames before this repeated-history cost may be removed; silently dropping frames or error-routing to a generic fallback is forbidden.

## Pass327 — authenticated immutable replication-authority objects

**[IMMUTABLE OBJECT AE DOMAIN]** External immutable durability objects are storage payloads and MUST remain inside CFMD AE v1 whenever database encryption is enabled. AE v1 derives an HKDF-separated `immutable-object` key distinct from section and WAL keys. Error-driven fallback between those domains is forbidden.

**[CFAO AUTHORITY OBJECT]** A replication-authority segment stored outside a generation is wrapped by the versioned `CFAO` object grammar. Its public header binds object kind, canonical plaintext `ReplicationAuthoritySegmentId`, parent segment ID, canonical plaintext length, fixed chunk size and chunk count. Encrypted payload is emitted in bounded 64 KiB independent AE v1 chunks; each chunk AAD contains the complete `CFAO` header plus chunk index and chunk plaintext length. No unauthenticated replication frame bytes may appear outside the encrypted generation/WAL/object boundary of an encrypted database.

**[PLAINTEXT IDENTITY LAW]** `ReplicationAuthoritySegmentId` remains the P325 digest of canonical plaintext segment semantics. AEAD nonce, ciphertext bytes, physical file offset, locator offset and generation number MUST NOT participate in segment identity. Re-encrypting the same canonical segment or relocating its authenticated object therefore preserves logical identity.

**[RELOCATION LAW]** `CFAO` AAD intentionally excludes physical address. Compaction of an already-authenticated immutable authority object is `copy exact stored object bytes -> rebuild physical locator -> publish relocated root`; decrypt/re-encrypt solely for relocation is forbidden. Recovery must authenticate/decrypt the object and then run the existing P325 complete-segment verification before mutating replication authority state.

**[MODE LAW]** Plaintext and encrypted databases share the same object semantics but not an error fallback path. Object encryption mode must agree with the opened database encryption mode; mismatch is corruption/protocol failure. An encrypted database cannot downgrade an unreadable encrypted object to plaintext replay.

**[ACTIVATION BOUNDARY]** P327 defines and verifies the object layer but does not make external segments current SingleFile authority. P323's generation-contained replication archive remains the maintained product representation until linked locator/root publication, crash recovery and segment-aware compaction are activated and verified for both plaintext and encrypted stores.


## Pass342 — object-first relationship authority

**[PUBLIC MODEL LAW]** The application object schema is the public authority. `Ref<T>`, `Option<Ref<T>>`, and `Many<T>` are first-class object relationship values. Users are not required to declare foreign keys, joins, `include`, target-side backlinks, or internal relation IDs. `cfmd-runtime` compiles object relationships into relation/Γ structures as an internal lowering.

**[NO HIDDEN I/O LAW]** Materialized relationship values are bound to the exact snapshot that produced their owner, but ordinary Rust field access performs no I/O. Evaluation is explicit through relationship operations such as `Ref::load/query` and `Many::all/load/where_/query/count/one`. The lower-level query remains composable and can batch/optimize relationship traversal without introducing ORM-style lazy-loading/N+1 semantics.

**[GRAPH WRITE LAW]** Detached `Many::new(...)` values are object-graph input. Insertion recursively lowers new target objects plus relationship facts into the same source-bound Plan and commits them atomically. For update, a bound Many value from the source snapshot means preserve the existing relationship; a detached Many value means replace that relationship. Scalar-only rewrites therefore require no relationship boilerplate.

**[EDGE LIFECYCLE LAW]** Internal many-edge relations store historical object identities for query equality and kernel-live endpoint witnesses for lifecycle authority. Kernel revision normalization removes rows whose source or target endpoint is no longer live. Durable mutation descriptors are derived from the exact normalized object target, so implicit lifecycle edge removal and physical relation publication remain the same certified transition rather than a hidden cascade side channel.

## Pass345 — executor-neutral exact-watch readiness/drain boundary

**[ONE WATCH SEMANTICS]** Async integration is an adapter over the existing exact-watch engine. It MUST NOT introduce a second result queue, full-query recomputation path, polling fallback, or executor-owned revision cursor. Durable causal history plus the maintained query differential state remain the only event authority.

**[MULTIPLEXED READINESS LAW]** Every watch has a stable `WatchSubscriptionId`, while every watch created from one live runtime shares the same `WatchReadinessSourceId`. Readiness is wake-only and generation-based. Adapters may therefore deduplicate subscriptions by source and maintain one blocking/native readiness registration per runtime source rather than one blocked worker per watch. Cancellation of an adapter readiness handle is independent from cancellation of the underlying watch subscription.

**[BOUNDED DRAIN LAW]** `drain_ready(max_events)` performs no wait for future publication and advances at most the requested number of already-certified causal transitions. It returns the exact post-drain `WatchStatus`, allowing an adapter to apply an explicit fairness budget while preserving sequential durable catch-up. A zero event budget performs no transition work. Missing exact causal coverage continues to fail closed as unavailable rather than silently resetting or recomputing.

**[EXECUTOR BOUNDARY]** `cfmd-runtime` has no Tokio dependency. P346/P347 add race-free `WatchReadiness::poll_after` standard-library `Waker` registration and hostile validation. P348 makes the watch itself the executor-neutral async surface: `watch.next().await` requires no adapter crate or mode conversion, while blocking `recv`, `try_recv`, and bounded `drain_ready` remain on the same object. Relation dependency frontiers index pending wakers so unrelated relation publication does not wake the task; output-equivalent causal effects emit no empty public event. Tokio is dev-only compatibility coverage. Python asyncio and .NET bindings MUST preserve the same exact readiness/drain authority. Executor/OS-specific adapters are optional optimizations, not semantic layers.

## Productization delta — Pass351

Pass351 adds a snapshot-bound Rust `Transaction` product composer over the existing Plan/Candidate/commit authority. A transaction owns one exact live base view and accepts only mutation Plans from that same database snapshot and authority; stale publication and cross-snapshot composition fail closed. Publication is retryable with the same durable transaction identity and preserves the existing idempotent `AlreadyCommitted` outcome. No second mutation engine, implicit rebase, retry loop, lock model or SQL transaction semantics are introduced.

P351 hostile R&D also fixes the next query-surface direction: ordered predicates are to be expressed as a native Γ-ordering relational primitive, prepared and maintained in `kernel-query`, then surfaced through typed Fields. Generic callback/post-filter fallbacks are explicitly rejected for this path.

## Productization delta — Pass361

**[QUERY COMPOSITION LAW]** Product predicate conjunction is relational composition, not a callback/post-materialization evaluator. `ObjectPredicate::and` applies the left and right exact predicates to the same root query in sequence, so equality, Γ-order and deep relationship predicates continue to lower into the existing kernel-query calculus and maintained differential program.

**[ORDERED BOUNDARY LAW]** Object-first `top(k, field)` / `bottom(k, field)` are admitted only for fields carrying a declared canonical ordering. They lower directly to the existing `TopKWithTies { column, ordering, direction, k }` relation primitive. Boundary-equivalent rows are retained even when result cardinality exceeds `k`; no physical row ordering is promised. Materialize-then-sort, SQL-shaped ordering fallback, full-query recomputation and error-driven routing are forbidden for this surface.

**[WORLD PARITY]** Current object queries and Candidate object queries share the same typed ordered-boundary lowering. Exact watches consume the same maintained ordered-cut state; no watch-specific query interpretation is introduced.

## Productization delta — Pass362

**[MULTIPLICITY-PRESERVING PROJECTION LAW]** Ordinary object-first `select` preserves one projected occurrence per selected source row. The lowering MUST remain kernel-native: a set-shaped object query is promoted by `PromoteToBag` before `Project`, so projection does not collapse equal projected values merely because the source relation has set authority. Host-side `Vec` duplication, SQL emulation, provenance-table fallback, or post-materialization expansion are forbidden.

**[EXPLICIT Γ-DISTINCT LAW]** `select(...).distinct()` is the explicit semantic quotient. It lowers to the existing maintained kernel `Distinct` with the exact declared equivalences of the projected columns. Thus `select(...).count()` counts projected occurrences, while `select(...).distinct().count()` counts Γ-equivalence classes. No second deduplication engine is permitted.

**[EXACT COUNT LAW]** Product `count()` over object sets, object queries, typed projections and relationship selections lowers to the kernel `Group` aggregate with an empty group key and exact `Count`. Host-side `all().len()`, projected-row `len()`, identity-vector `len()`, or recompute fallback are forbidden for the ordinary count surface.

**[TYPED GROUP LAW]** `group_by(key).count()` lowers directly to kernel `Group` with the key field's declared Γ equivalence and `ExactCount`. `group_by(key).sum(f64_field)` lowers to the same `Group` operator with `ExactF64Sum`; host floating accumulation and separate aggregate routing are forbidden. Live and Candidate reads MUST share this lowering and result-shape semantics.

**[BOUNDARY NAME LAW]** `top(k, field)` and `bottom(k, field)` are the product names for CFMD's strongest admitted ordered-boundary semantics. They retain every row equivalent at the k-th Γ boundary class. The product layer MUST NOT add a weaker arbitrary exact-k variant merely to mimic conventional collection APIs; the kernel name `TopKWithTies` remains an implementation/dynamic detail.

**[PROJECTION WATCH LAW]** Maintained ordinary projection watches observe exact bag multiplicity deltas after `PromoteToBag -> Project`; inserting another source row with the same projected value emits that projected insertion. Maintained distinct projection watches observe semantic classes and therefore suppress source multiplicity changes that do not create or remove a Γ-equivalence class. Neither path may recompute or deduplicate in the product layer.

## Pass395 normative addendum — Physical Realization reference boundary

**[ARCHITECTURE]** Physical representation is not semantic/schema authority. The target storage law factors current logical state through a certified realization root `ρ : PhysicalAtoms -> finite Model`. A migration `M : A -> B` changes current semantic authority atomically and should compile the next realization compositionally as `ρ_B = normalize(M ∘ ρ_A)`. Retained atoms created under earlier physical layouts do not keep schema A alive as a current semantic world.

**[REFERENCE IMPLEMENTATION]** `kernel-realization` now provides the non-durable in-memory reference calculus: explicit atom identity/codec, direct/constant/exact-scalar field realizations, full `DatabaseState` evaluation as the oracle, extensional rewrite certification, semantic-coordinate dependency graphs and current+historical reachability. This crate is intentionally below `kernel-plan`; it must not become planner-specific routing state.

**[MATERIALIZATION LAW]** A representation-only rewrite may publish different physical atoms/root under the same semantic revision only after proving extensional equality of the realized logical state. The reference law is `evaluate(ρ_before) = evaluate(ρ_after)`. This is not a database revision and MUST NOT create a semantic/history event.

**[GC LAW]** Physical progress is graph reachability, not a migration-progress bitmap. An atom is reclaimable only when unreachable from every current or retained historical realization root (and from any future durable/recovery authority once those are integrated).

**[NON-CLAIM]** P395 does not change checkpoint/WAL authority. `Revision=(S,Γ,M)` remains the durable oracle, and current `PhysicalStore` artifacts remain reconstructible. Durable physical atoms/realization roots are deferred until migration composition and representation-rewrite laws are closed in-memory.

## Pass396 normative addendum — migration composition and realization performance law

**[COMPOSITION LAW]** A verified migration `M : A -> B` may compile the current physical realization compositionally as `ρ_B = normalize(M ∘ ρ_A)` without making semantic schema A a current runtime authority. Field merge/split/default/drop and row-local relation rewrites must reuse the existing deterministic query/transport algebra; unsupported global relational rewrites fail closed rather than falling back to an A-schema runtime branch.

**[PERFORMANCE LAW]** A derived realization is a correctness-preserving bridge, not automatically an acceptable steady-state hot representation. Measurements on the P396 reference evaluator show a generic scalar transform at roughly 12-13x the cost of a direct in-memory atom read, while materialized direct realization returns to native baseline. Production scheduling must therefore permit hot/on-access coordinates to converge to native physical realization without changing semantic revision.

**[FACTORIZATION LAW]** The per-value `PhysicalAtom` / `(FieldId,EntityId)->RealizationExpr` representation in P395/P396 is a reference oracle only. Production semantic cutover MUST NOT require O(number of stored values) realization metadata construction. Realization programs must be factorized over physical columns/segments/chunks or an equivalently bounded structural coordinate so migration cutover metadata is proportional to schema/layout structure rather than database cardinality.

**[NON-CLAIM]** P396 does not make realization durable authority and does not authorize the current per-cell reference representation for production storage. Full `Revision=(S,Γ,M)` remains the durable semantic oracle until factorized realization has equivalent correctness and measured performance.

## Pass397 normative addendum — factorized realization and compiled transform law

**[FACTORIZED FIELD LAW]** A production field realization is keyed by stable semantic field identity and applies to a physical column/segment containing many entity values. Semantic migration cutover MUST NOT allocate one realization expression per entity value when the same structural rule applies to the whole column/segment.

**[CUTOVER COMPLEXITY LAW]** For a migration whose field mapping is structurally uniform, `ρ_B = normalize(M ∘ ρ_A)` must construct metadata proportional to schema/layout structure, not database cardinality. P397 demonstrates 100,000 migrated values with three physical atoms and three target dependencies; row count is payload cardinality, not realization-program cardinality.

**[NORMALIZATION LAW]** Verified deterministic migration expressions should normalize into compiled physical realization rules when their semantics admit it. Direct alias/copy may reuse the existing column atom, constants may remain virtual, and direct `i64 -> f64` over one source column may execute as a specialized column rule without constructing generic product values on every read. Generic `ExactQuery` remains the semantic oracle/fallback representation only where no equivalent compiled realization rule exists; it must not be silently substituted for an unsupported factorized relation migration.

**[HOT-PATH PERFORMANCE LAW]** A factorized derived representation is acceptable on the hot read path only when its measured cost is near the corresponding native physical read or when policy guarantees bounded convergence to native materialization. P397 warm release measurements for normalized `i64 -> f64` are 0.85-0.98x the direct point-read time; the earlier generic 12-13x path is therefore not the accepted implementation for this primitive.

**[MATERIALIZATION LAW]** Materializing a factorized field column is a representation-only publication under the same semantic revision: one derived column rule may be replaced by one native physical column atom after extensional equivalence. Full-column materialization cost is physical maintenance work, not semantic migration work.

**[RELATION NON-CLAIM]** P397 factorizes entity fields only. Any `SchemaMigrationProgram` containing relation rewrites fails closed in the factorized compiler until stable relation-column/segment realization is implemented. The runtime must not route such a migration through the superseded mixed-current-world or per-row factorized fallback.

## Pass399 normative addendum — bounded chunk realization law

**[CHUNKING LAW]** Physical chunk subdivision is a representation detail under one semantic coordinate. Materializing a bounded range of a factorized column MUST NOT create a semantic revision, schema epoch, alternate current schema, or migration-progress record. The current semantic relation/column remains unchanged while its realization may contain a derived base plus sparse native chunks.

**[SPARSE PROGRESS LAW]** Chunk convergence is represented by physical reachability. The realization root may retain a base expression plus only the native chunks that have actually been materialized. A dense row/cardinality-sized progress bitmap is not part of semantic or durable migration authority.

**[GC LAW]** A base physical atom remains reachable while any current chunk still depends on it. Once every chunk covered by the realization is native, the current realization root MUST cease retaining the base atom. Historical realization roots may still pin it independently.

**[BOUNDED MATERIALIZATION LAW]** On-access or maintenance-driven materialization may rewrite one bounded chunk without rewriting the rest of the column. Reads from both native and still-derived chunks MUST realize the same current semantic values.

**[PERFORMANCE LAW]** Chunk routing must not reintroduce a large steady-state hot-read penalty. P399 measures 100k-row relation columns with 4096-row chunks; whole-column partial-materialization scans remain near the direct/native cost class, while chunk materialization itself is bounded to the selected range. Planner/executor scan lowering SHOULD resolve chunk routing at range granularity rather than perform generic migration dispatch per value.

**[NON-CLAIM]** P399 implements bounded subdivision for factorized relation columns only. Entity-field chunking requires an explicit physical carrier/segment coordinate and MUST NOT be approximated by arbitrary `EntityId` numeric ranges. Durable realization authority is still deferred.

## Pass401 normative addendum — exact prepared general relations and B-native writes

**[GENERAL RELATION PREPARATION LAW]** A verified general relational migration rewrite must not revive schema A as a current-world query authority. Let `D(q)` be the exact source-relation scan closure of the verified `RelExpr q`. CFMD may prepare `q` against `rho_A | D(q)` before cutover and publish native target-B relation-column atoms. The semantic cutover then installs direct B realization rules in metadata proportional to target schema/layout, while all data-cardinality work remains explicit preparation.

**[FAIL-CLOSED LAW]** `compose_schema_migration_factorized` without the required prepared relation remains fail-closed. No generic current-schema-A read route, SQL-shaped fallback, or per-row migration realization may be selected implicitly.

**[DEPENDENCY LAW]** General-relation preparation records exact semantic scan dependencies and the physical atoms reachable from those source relation rules. Unrelated relations are not preparation inputs. After cutover, the prepared target relation depends on native B atoms; source atoms remain live only through other current or retained historical roots.

**[B-WRITE LAW]** A write in the current schema B over a derived realization must never require inverse migration. Once semantic write authority is certified by the change layer, the physical layer may materialize the bounded B segment containing the coordinate and replace that value/cell by publishing a new immutable B-native overlay atom. Migration realization does not define independent conflict/rebase semantics.

**[GLOBAL WRITE NON-CLAIM]** An arbitrary general-query result is not assumed to have a stable writable row identity. `Union`, `Distinct`, `Group`, bag multiplicity and other global operators may destroy source-row identity. Local physical row ordinals therefore are not universal semantic write coordinates. A B-native relation-delta/endpoint overlay over existing kernel-change/query coordinates remains the required general law.

**[PERFORMANCE GATE]** On the PASS401 100k-row two-source bag-Union fixture, exact preparation measured 27.471–28.930 ms across three warm release runs while root cutover measured 39.769–43.053 us. The accepted complexity boundary is O(data touched by the general query) before cutover and O(schema/layout) at semantic cutover.

## Pass402 normative addendum — post-migration current-B relation write law

**[CURRENT-B AUTHORITY LAW]** A migration query used to construct a target relation does not remain a writable-view obligation after semantic cutover. Once schema B is current, the target relation is a B semantic coordinate. Exact writes are expressed as B-native relation rewrites under the existing Γ/`kernel-change`/`kernel-query` laws; they MUST NOT require locating or mutating a source-schema A row.

**[PREPARED ENDPOINT LAW]** A physical realization rewrite may consume an exact `PreparedRelationRewrite` bound to the current B relation support and materialize its extensional endpoint into native B atoms. The realization layer MUST reject a prepared rewrite whose certified base no longer matches the current realized B support. This operation introduces no migration-specific conflict engine.

**[QUERY-KIND INDEPENDENCE LAW]** `Union`, `Distinct`, `Group`, bag multiplicity, and other migration-query operators do not select different write semantics after cutover. Their provenance affects preparation of the initial B value, not the meaning of later B writes. No SQL-style writable-view router or inverse-migration fallback is permitted.

**[PERFORMANCE NON-CLAIM]** Full relation endpoint detachment is a correctness/reference lowering only. PASS402 measures roughly 92.9–98.9 ms to detach a one-row write on a 100k-row bag relation. Production current-B writes therefore require a bounded Γ-class/multiplicity delta-overlay lowering so write cost does not scale with whole relation cardinality; full endpoint detachment MUST NOT become the ordinary hot write path.

### Bounded current-relation delta realization (PASS403)

For a current semantic relation with an exact prepared relation rewrite, physical mutation MAY be represented by a persistent B-native delta overlay over immutable base realization. Set occurrence resolution MUST use pinned Γ canonical classes; Bag occurrence resolution MUST preserve exact multiplicity. The overlay MUST NOT infer source-schema provenance, invert a migration, or use semantic meaning from physical row ordinals. Snapshot/root cloning MUST be structurally shared. Missing bounded-write preparation MUST fail closed rather than route to O(data) endpoint reconstruction. Compaction is a same-revision extensional rewrite and MUST preserve the exact relation value while replacing base+overlay dependencies with native factorized columns.

### Runtime current-relation Scan evidence (P416)

A live runtime revision root MAY retain row-aligned Γ-canonical Scan evidence as part of the same immutable publication unit as its logical revision, physical store and relation base authority. Such evidence is not an independently mutable cache: it is derived at bootstrap from the exact relation authority and a target root is published only with evidence advanced by the same accepted relation transition. Exact query execution may consume structurally shared evidence only when the prepared relational algebra proves that seeded canonical evidence composes for that expression. Historical Watch replay MUST NOT treat current physical row handles as historical authority unless the causal-history transition itself carries storage-resolved identity evidence; semantic-history watches instead require semantic persistent evidence advanced by their retained deltas.

### Witness-owned logical Scan evidence (P417)

For a Set relation, the current semantic occurrence authority assigns stable occurrence slots monotonically: initial occurrences receive the initial logical order, deletion removes an occurrence without renumbering survivors, and insertion allocates a strictly later slot. Therefore ordering live Set occurrences by semantic stable handle is exactly the logical relation law `survivor-order + append`.

The authoritative `RelationBaseWitness` may maintain both `Γ class -> stable occurrences` and `stable occurrence -> canonical row key` as persistent projections of the same immutable transition. These are one authority: every delta advances both projections in one sealed support transition. Runtime exact Scan evidence is an O(1) view of the second projection; it is not rebuilt from all live rows and is not a separately mutable Γ index.

This law is Set-specific. Bag duplicate occurrences require a separate proof of logical occurrence order and must not reuse the Set stable-slot argument by assumption.

Watch bootstrap may consume semantic Scan evidence to construct maintained canonical lookup, but historical Watch evolution remains semantic delta replay. Current physical row handles are not historical semantic authority.

## Pass439 — semantic data-authorization coordinates

**[DATABASE-OWNED ENFORCEMENT]** Product authorization is enforced inside the shared runtime/query/change authority, not by `Context` shape and not independently by Rust, Python, host, transport or other frontends. An external host authorizer may establish principal grants, but executing a database observation/change checks those grants again at the database semantic boundary.

**[READ FOOTPRINT LAW]** A relational observation is authorized from a finite semantic footprint over stable relation and relation-column identities. The footprint includes both returned coordinates and coordinates whose values can influence the observation through filtering, ordering, grouping, equality/deduplication, joins, anti-joins or set/bag operators. It is derived structurally from `RelExpr`; it does not scan data and does not route on physical realization.

**[FIELD IDENTITY LAW]** Object relation columns use the same stable semantic field identity already carried by durable P438 object-field writes. Authorization therefore binds semantic coordinates that survive source-language naming and physical/ordinal layout changes; ordinal column positions are lowering coordinates only.

**[PUBLICATION RECHECK]** A plan does not freeze authority. Commit rechecks the current shared session authority against the exact write coordinates carried by the plan, so grant refresh/revocation reaches already-formed plans. Whole-relation write authority and exact field-write authority are distinct.

**[OPEN]** Object create/delete and relationship attach/detach/move still require an exact action classifier over existing lifecycle/relationship semantics before dedicated public grants are admitted. History undo/redo must authorize the coordinates of the effect being materialized. Field-only writers must eventually be able to form a patch without gaining read authority over unrelated hidden fields.

## Pass442 — role composition and explicit model/schema authority

**[ROLE LAW]** A product `Role` is an immutable named bundle of `Permission` values. Role composition MUST flatten into the ordinary `PermissionSet` before runtime enforcement. Role names, role hierarchy, and frontend/provider policy MUST NOT form a second authorization semantics layer.

**[MODEL DISCLOSURE]** Full authoritative schema inspection requires `ModelRead`. Ordinary data-read grants do not imply model disclosure. The narrow schema revision/epoch identifier remains observable independently so schema-compatible readers can select a local binding policy without receiving the full model.

**[SCHEMA PUBLICATION]** Restricted schema migration requires `SchemaMigrate`; generic data `Write` is insufficient. Expensive deterministic migration preparation need not hold session authority locks, but the durable publication boundary MUST execute under current migration authority so refresh/revocation linearizes with publication.

**[WATCH TERMINATION]** A blocked watch awakened by cancellation revalidates current session/query authority before returning ordinary `WatchClosed`. Session revocation maps to hosted `SessionClosed`; exact read-authority loss maps to `PermissionDenied`. Output-equivalent causal revisions remain quotiented from the public watch stream.
