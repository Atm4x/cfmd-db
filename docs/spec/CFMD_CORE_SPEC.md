# CFMD Core Specification — Current Architecture + Future

**Статус:** текущая нормативная спецификация CFMD после закрытия исторического kernel-backlog и сертификации первого поддерживаемого durability-профиля.

**Назначение:** этот документ описывает **текущий** логический, математический, runtime, durability, replication и trust-контракт CFMD. Он заменяет старую практику держать отдельный разрастающийся `CFMD_IDEAL_DB_SPEC` с append-only pass-addendum: реализованная архитектура находится в основной нормативной части, а ещё не реализованное идеальное направление — только в `Future / Non-Normative Roadmap`. История решений и pass-отчёты должны храниться отдельно.

**Версия архитектурного baseline:** post-Pass120 / repository-hardening baseline.

**Язык реализации:** Rust 2024, pinned Rust 1.98.1. Собственный production Rust workspace запрещает `unsafe` через workspace lint.

**Формальная система:** Lean 4.34.0 для механизированных proof surfaces, с source-refinement binders к production Rust.

---

## 0. Нормативные слова и область документа

В документе используются следующие статусы:

- **MUST / ОБЯЗАНО** — нарушение делает реализацию несовместимой с этой спецификацией.
- **MUST NOT / НЕ ДОЛЖНО** — запрещённое поведение.
- **SHOULD / СЛЕДУЕТ** — сильная инженерная рекомендация, от которой можно отступить только с явным обоснованием.
- **MAY / МОЖЕТ** — допустимое физическое/инженерное решение.
- **CURRENT** — существует в текущем kernel/runtime и является частью поддерживаемой архитектуры.
- **FUTURE** — намеренно не является обещанием текущего kernel; описано в разделе Future.

Эта спецификация определяет:

1. логическое состояние базы;
2. типовой универсум;
3. pinned semantic environment `Γ`;
4. exact query/change/rewrite semantics;
5. lifecycle, identity, constraints и revision semantics;
6. lowering в physical/runtime слой;
7. authority/publication rules;
8. durability/recovery/freshness;
9. replication/consensus/authentication;
10. formal proof boundary;
11. текущую implementation decomposition;
12. явно вынесенные future surfaces.

Эта спецификация **не** обещает:

- что одна физическая стратегия оптимальна для всех workloads;
- что любой filesystem/device автоматически поддерживается;
- что arbitrary host callback является частью exact semantics;
- что внутренние `kernel-*` crates являются стабильным пользовательским API;
- что arbitrary plugin/native code можно грузить в процесс БД;
- что approximation может молча заменять exact semantics.

---

# Part I. Mathematical core

## 1. Одна строка, определяющая CFMD

Логическая база данных в CFMD — неизменяемая ревизия

```text
R = (S, Γ, M)
```

где:

- `S` — schema/structural state;
- `Γ` — pinned semantic environment;
- `M` — конечная типизированная модель данных.

В production runtime эта логическая ревизия публикуется вместе с физическим состоянием:

```text
RuntimeRoot = (R, P, V)
```

где:

- `P` — authoritative physical store/layout bindings для этой же `R`;
- `V` — зарегистрированные maintained materializations/derived runtime state.

Ключевой инвариант:

```text
revision(R) = revision(P) = revision(V)
```

в смысле их schema/semantic/root identity bindings.

Нельзя публиковать смешанный root, в котором логическая модель относится к одной ревизии, physical store — к другой, а maintained state — к третьей.

Файлы, страницы, hash tables, B-trees, CSR, column vectors, semantic indexes, caches, WAL frames и materialized views **не являются второй логической БД**. Они являются физическими представлениями или производными артефактами одной логической ревизии.

---

## 2. Семантическая идентичность ревизии

Текущий kernel использует составную semantic revision:

```text
SemanticRevision = (SchemaRevisionId, SemanticEnvId)
```

Schema и semantic environment являются независимыми осями версии.

Это важно, потому что:

- структура может меняться при неизменной семантике;
- semantic implementation / collation / ordering / equality rules могут меняться при неизменной структурной форме;
- любой физический индекс или compiled artifact, зависящий от `Γ`, должен быть привязан к точной semantic revision.

Presentation names не определяют semantic identity. Rename символа не обязан менять его `SemanticId`.

---

## 3. Логический типовой универсум

### 3.1 Scalar layer

Текущий structural type kernel содержит scalar vocabulary:

```text
Unit
Bool
I64
F64
Text
LiveEntityRef(E)
HistoricalEntityId(E)
```

`F64` как representation не даёт права использовать случайное host-level `PartialEq`, `Ord` или hash как semantic equality/order там, где операция логически наблюдаема. Наблюдаемая equality/order определяется через `Γ`.

### 3.2 Structural constructors

Текущий `TypeExpr`:

```text
T ::=
    Scalar(s)
  | Product { field_i : T_i }
  | Sum     { variant_i : T_i }
  | Option(T)
  | Set<T, EqΓ>
  | Bag<T, EqΓ>
  | Seq<T>
  | Map<K, V, KeyEqΓ>
  | Var(X)
  | μX.T
```

Семантика collection constructors различна:

- `Set<T>` — extensional uniqueness по pinned semantic equivalence;
- `Bag<T>` — multiplicity по semantic equivalence classes;
- `Seq<T>` — порядок является частью logical value;
- `Map<K,V>` — функциональное отображение по semantic key classes.

CFMD не использует universal SQL-`NULL` как скрытый третий truth/value state. Optionality выражается `Option<T>`.

CFMD не должен silently преобразовывать:

```text
Set <-> Bag <-> Seq
```

если такое преобразование теряет или добавляет логический смысл. Любая смена collection semantics должна быть явной.

### 3.3 Guarded recursion

Recursive structural type задаётся `μX.T` и обязан быть guarded.

Текущий validator отклоняет:

- free type variables;
- unguarded recursion.

Нормативно:

```text
well_formed(μX.T)
```

требует, чтобы каждое рекурсивное использование `X` проходило через допустимый structural constructor.

### 3.4 Nominal identity

Structural equality и nominal identity — разные понятия.

Минимально различаются:

```text
EntityId / Atom identity
LiveEntityRef<E>
HistoricalEntityId<E>
SemanticId
Revision-local EqClassId
StableRowHandle
```

Две сущности с одинаковыми полями не обязаны иметь одну nominal identity.

Physical dense/local IDs не являются durable/public identity. Stable physical row identity использует generational handle:

```text
StableRowHandle = (slot, generation)
```

чтобы reuse слота не превращал старый handle в alias новой строки.

### 3.5 Surface models are views over one kernel

CFMD не имеет независимых «object DB», «document DB», «graph DB» и «relational DB» с разными законами. Ergonomic surface constructs elaborated в один типизированный kernel.

Текущая нормативная таблица, связанная с Lean surface theorem:

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
computed property        -> Query/View
business constraint      -> violation query required empty
transaction method       -> typed Rewrite
```

Следовательно, relational, object, document, graph и key/value workloads являются surface projections одного logical model, а не peer databases внутри одного процесса.

Surface elaboration не имеет права терять type structure или добавлять скрытые logical nodes. Эта граница механизирована в `SurfaceKernel.lean` для текущего vocabulary.

---

## 4. Schema `S`

Schema определяет как минимум:

- semantic symbols;
- type definitions;
- relation declarations;
- capabilities/subtyping constraints;
- semantic equivalence/order dependencies;
- допустимые structural dependencies.

Symbol identity задаётся `SemanticId`, а presentation name — только человекочитаемое представление.

Типовая/схемная валидность включает:

- отсутствие duplicate semantic IDs;
- отсутствие duplicate type/relation/capability definitions;
- корректную arity semantic equivalences;
- отсутствие subtype cycles;
- well-formed recursive types;
- наличие всех semantic module dependencies в `Γ`.

---

## 5. SemanticEnvironment `Γ`

### 5.1 Основной принцип

Смысл — часть revision state.

CFMD не должен считать следующие вещи невидимым ambient environment, если они влияют на observable semantics:

- text equality/collation;
- ordering;
- normalization/casefold;
- exact numerical comparison rules;
- tokenizer/model version;
- timezone tables;
- certified logical functions;
- semantic canonicalization.

Текущий semantic environment хранит versioned module binding:

```text
Γ : SemanticId -> ModuleDigest
```

и имеет собственный `SemanticEnvId`.

Schema обязана ссылаться только на semantic modules, присутствующие в pinned `Γ`.

### 5.2 Semantic equivalence

Для observable `o` и pinned revision `ρ` определим:

```text
x ≡[ρ,o] y
```

как certified semantic equivalence, а не Rust `==`.

Для коллекций semantic classes являются revision-local realization:

```text
[x]_(ρ,o) -> EqClassId
```

`EqClassId` нельзя переносить между независимыми observable catalogs или semantic revisions без certified transport.

### 5.3 Semantic ordering

Ordering также является semantic module.

Для ordering `ord`:

```text
compare[ρ,ord](x,y) ∈ {Less, Equal, Greater}
```

должно быть определено pinned implementation/specification, а не physical iteration order.

### 5.4 Canonical semantic keys

Physical semantic indexes могут использовать canonical keys, если они:

1. versioned;
2. reconstructible/checkable;
3. привязаны к semantic module/revision;
4. не создают semantic authority отдельно от `Γ`.

Для unordered structural collections canonicalization использует finite counting-measure law: физически разные representatives, схлопнувшиеся в одну semantic class, агрегируются, а не остаются duplicate canonical atoms.

### 5.5 Semantic observable catalog

Revision-local observable catalog разделяет:

```text
semantic definition
        vs
revision-local compact realization
```

`RevisionObservableId` и `EqClassId` — номинальные координаты одного конкретного catalog instance.

Cross-catalog aliasing запрещено.

---

## 6. Model `M`

`M` — конечный типизированный экземпляр `S` под `Γ`.

Он включает logical carriers/relations/values и lifecycle state, но не включает physical cache/index authority.

Для relational части relation row должна быть well-typed относительно schema и текущих live extents.

Current kernel поддерживает dense revision-local projections для performance, но они reconstructible из authoritative logical state и не заменяют его.

---

## 7. Lifecycle как least fixed point

Ownership/liveness не задаётся процедурными каскадами.

Пусть:

```text
Root(M)      — явно живые roots
KeepsAlive(M) ⊆ Entity × Entity
```

Тогда live set определяется least fixed point:

```text
Live(M) = μX. Root(M) ∪ KeepsAlive(M)[X]
```

После candidate mutation модель нормализуется ограничением lifecycle-managed carriers на `Live(M)`.

Из этого следуют свойства:

- цикл сам по себе не является root;
- SCC живёт, пока достижим из root;
- strong reference обязан указывать на live target;
- reference сам по себе не обязан быть lifetime edge;
- weak/history references не обязаны продлевать lifetime;
- «cascade delete» — следствие reachability normalization, а не независимый delete engine.

Lifecycle normalization должна быть idempotent:

```text
N(N(M)) = N(M)
```

---

## 8. Constraints и валидность

Валидные модели образуют подмножество:

```text
Valid(S,Γ) ⊆ Model(S,Γ)
```

CFMD разделяет:

1. structural/type constraints;
2. global declarative constraints.

Global invariant может быть представлен finite total violation query:

```text
Viol_P(M)
```

и

```text
M satisfies P  <=>  Viol_P(M) = ∅
```

Incremental maintenance violation state — optimization. Authoritative semantics остаётся equivalent to exact from-scratch check.

---

# Part II. Query semantics

## 9. Exact query

Exact query имеет форму:

```text
q : (S, Γ, M, explicit_inputs) -> O
```

и обязана быть:

- deterministic относительно pinned revision и explicit inputs;
- pure относительно logical semantics;
- total в объявленной domain semantics;
- extensional;
- representable closed IR или certified module, а не arbitrary host closure.

Ambient randomness, wall clock, locale или hidden process state не могут silently влиять на exact result.

---

## 10. Текущий relational IR

Текущий `RelExpr` vocabulary:

```text
Scan(relation)

FilterEqConst {
    input,
    column,
    value,
    equivalence
}

FilterEqColumns {
    input,
    left_column,
    right_column,
    equivalence
}

Project {
    input,
    columns
}

JoinEq {
    left,
    right,
    left_column,
    right_column,
    equivalence
}

Difference {
    left,
    right
}

AntiJoin {
    left,
    right,
    left_column,
    right_column,
    equivalence
}

Distinct {
    input,
    column_equivalences
}

Group {
    input,
    group_columns,
    group_equivalences,
    aggregate
}

TopKWithTies {
    input,
    column,
    ordering,
    direction,
    k
}

PromoteToBag(input)
```

Это exact logical vocabulary. Physical join family, index family, maintained state layout и scheduler не добавляют новые logical operators.

---

## 11. Exact lowering

Пусть `L(q)` — physical/logical execution plan, созданный lowering pipeline.

Корректность требует:

```text
⟦L(q)⟧_(S,Γ,M) = ⟦q⟧_(S,Γ,M)
```

для всех admitted states текущего contract domain.

Production lowering checker не должен принимать план только потому, что он «похож» на query. Он проверяет exact binding и certificate/witness boundary.

Formal surface proof дополнительно фиксирует:

- round-trip lowering;
- отсутствие hidden logical node expansion;
- type preservation;
- checked-plan semantic preservation.

---

## 12. Ordered views и pagination

Порядок, используемый для pagination, обязан быть pinned semantic ordering, а cursor — связан с exact ordered-view specification/revision.

Нельзя использовать incidental physical order как stable logical pagination contract.

Иными словами:

```text
Page = page(OrderedViewSpec, Revision, Cursor)
```

а не:

```text
Page = "следующие N строк из того, как сейчас обошёлся B-tree/hash table"
```

---

## 13. Recursion и fixed points

Recursive logical computation допускается через admitted finite/monotone fixed-point semantics и certified solvers.

Нормативная форма least fixed point:

```text
lfp(F) = μX.F(X)
```

где domain/termination obligations должны быть известны checker/runtime.

Specialized solver не должен становиться semantic authority. Он либо:

- выдаёт checkable result/certificate;
- либо является certified refinement конкретной declarative fixed-point specification.

---

# Part III. Change and differential calculus

## 14. Universal change

Для любого logical type `T` CFMD имеет total change envelope:

```text
Change<T> =
    NoChange
  | Replace(T)
  | Fine(FineChange<T>)
```

Базовые laws:

```text
apply(x, NoChange)   = x
apply(x, Replace(y)) = y
apply(x, Fine(d))    = endpoint(d)
```

`FineChange` — refinement change semantics, а не необходимое условие completeness.

Следовательно, exact derivative существует хотя бы через replacement/recompute.

---

## 15. Exact derivative law

Для query

```text
q : A -> B
```

корректный derivative

```text
Dq : A × Change<A> -> Change<B>
```

обязан удовлетворять:

```text
apply(q(a), Dq(a, da)) = q(apply(a, da))
```

Это основной закон incremental correctness.

Performance derivative не может менять этот закон. Если специализированный delta path неприменим, корректная реализация может деградировать до recomputation/Replace.

---

## 16. Fine changes и semantic classes

Текущий kernel различает fine-change kinds для scalar/product/sum/option/set/bag/seq/map/relation/recursive и extensible semantic kinds.

Для `Set`, `Bag` и `Map` structural patches могут адресовать semantic classes pinned observable catalog, а не Rust hash/equality.

### 16.1 Set

Для observable `o`:

```text
ΔSet = (inserted : EqClass -> representative,
        removed  : Set<EqClass>)
```

remove и insert одного semantic class в конфликтующей форме не должны получать order-dependent semantics.

### 16.2 Bag

Multiplicity меняется по semantic class:

```text
m'(c) = m(c) - removed(c) + inserted(c)
```

при обязательных условиях:

```text
removed(c) <= m(c)
```

и отсутствии overflow.

### 16.3 Map

Map update адресует semantic key class. Upsert задаёт replacement value для class; ambiguous remove+upsert intent должен быть rejected или иметь явно заданный закон.

### 16.4 Sequence

Для user/rewrite semantics snapshot index недостаточно стабилен при concurrency.

Current write calculus использует stable occurrence/gap anchors для sequence intent; concrete indices — execution detail после разрешения anchor относительно snapshot.

---

## 17. Differential operator classes

Relational operators классифицируются по exact differential structure:

```text
Source
Linear
ZeroCrossing
BilinearPullback
Annotation
OrderedBoundary
BlockerZeroCrossing
```

Смысл классов:

- **Source** — authoritative leaf delta;
- **Linear** — output delta является линейным transport input delta;
- **ZeroCrossing** — требуется знать переход support `0 <-> nonzero`;
- **BilinearPullback** — join-like interaction двух меняющихся inputs;
- **Annotation** — maintained grouped/annotated state;
- **OrderedBoundary** — поддержка boundary/cut для TopK-like operators;
- **BlockerZeroCrossing** — anti-join/difference-style blocker support.

Эта классификация семантическая. Она не фиксирует concrete data structure.

---

## 18. Compositional differential law

Для композиции:

```text
h = g ∘ f
```

корректные derivatives композиционно дают:

```text
Dh(a, da) = Dg(f(a), Df(a, da))
```

Это основание для maintained plan graph.

Current runtime компилирует transition в graph patch, затем guarded commit. Planned patch не становится authoritative до publication.

---

## 19. Γ-quotient coordinates

Для semantic observable `o` значения рассматриваются через quotient:

```text
A / ≡[Γ,o]
```

Внутри одной revision-local catalog realization quotient classes имеют compact nominal coordinates.

Физический kernel может оперировать этими coordinates, если:

1. catalog/revision identity совпадает;
2. observable identity совпадает;
3. coordinates reconstructible/checkable;
4. durable/public semantics не зависит от случайного numeric ID.

---

## 20. Determinant closure

Для набора certified morphisms `D` closure semantic coordinates определяется least fixed point:

```text
Cl_D(A) = μX. (A ∪ consequences_D(X))
```

Execution может вычислять closure incidence/worklist алгоритмом, но semantic object — именно least closure, а не конкретный порядок обхода rules.

Минимальный/оптимальный FD cover не является authority requirement. Достаточен deterministic inclusion-minimal generator, если он сохраняет тот же closure.

---

## 21. Anchor-Pullback Normal Form

Для finite weighted factor `F` по координатам `C` выбирается anchor subset `A ⊆ C`, если projection

```text
π_A : support(F) -> Values(A)
```

injective на distinct support classes.

Тогда удалённые coordinates восстанавливаются certified deterministic morphisms:

```text
reconstruct : A -> C \ A
```

а фактор может быть представлен как weighted anchor measure плюс reconstruction law.

Обобщённо:

```text
F  ≅  (μ_A, reconstruct_A)
```

где `μ_A` — конечная counting measure над anchor tuples.

Этот representation не меняет semantics и позволяет runtime выбирать algebraic/native physical form.

---

## 22. Quotient Constraint Network

Γ-QCN представляет semantic quotient constraints/factors как reconstructible physical coordination layer.

Нормативные правила:

- quotient/factor state привязан к exact semantic revision;
- cached quotient IDs не являются logical authority;
- updates должны сохранять closure/factor invariants;
- deletion/insertion derivatives должны быть exact относительно source-bound change;
- durable state хранит reconstructible recipe/binding, а не доверяет stale process-local IDs.

QCN используется как ускоряющий semantic/physical substrate, а не как отдельная модель данных.

---

# Part IV. Rewrite semantics and transactions

## 23. Rewrite как first-class intent

Write в CFMD — semantic rewrite, а не набор page writes.

Упрощённо:

```text
τ : R -> R'
```

или для отдельной typed domain:

```text
τ : T -> T
```

Current kernel имеет first-class rewrite identity/spec/law sets и prepared rewrite boundary.

Physical mutation — следствие accepted rewrite, а не источник его logical meaning.

---

## 24. Transaction pipeline

Логический commit обязан проходить последовательность, эквивалентную:

```text
source revision
  -> prepare explicit rewrite/change
  -> construct candidate
  -> lifecycle normalization
  -> type / relation validation
  -> invariant / violation validation
  -> semantic/freshness checks
  -> durability preparation
  -> seal
  -> atomic authority publication
```

До seal/publication recoverable failure не должен менять reader-visible authoritative root.

После authoritative publication runtime не может сообщить «обычную recoverable error», если фактически authority уже сменилась; uncertainty должна иметь отдельную fail-stop/recovery semantics.

---

## 25. Rewrite transport и concurrency

Concurrency reasoning использует semantic observations и rewrite laws, а не только page-level read/write sets.

Для двух rewrites `a`, `b` автоматическое commuting допустимо только при certified law, например квадрат:

```text
      x --a--> x_a
      |         |
      b         b/a
      |         |
      v         v
     x_b --a/b-> x_ab
```

с требованием semantic equality endpoint:

```text
(b/a)(a(x))  ≡  (a/b)(b(x))
```

Для трёх и более interacting rewrites используются higher coherence witnesses/cubes там, где они необходимы.

Если required residual/diamond/cube law отсутствует, runtime обязан конфликтовать/fail closed, а не угадывать merge.

---

## 26. Revision effects и causal identity

Durable retry identity и causal effect identity различаются.

Нельзя считать:

```text
same retry token == same causal semantic effect
```

если протокол этого явно не доказывает.

Current kernel содержит revision effect identity, residual families, concurrency witnesses и durable idempotency/retry history boundaries.

---

## 27. Writable views / lenses

Writable derived view требует reconstruction law.

В общей форме lens связывает source `S` и view `V`:

```text
get : S -> V
put : S × V' -> S'
```

с explicit complement/residual там, где view lossless не является.

Current kernel имеет dependent lens/complement substrate и writable relational coordinates для поддерживаемых cases.

Lossy `Project` нельзя write-through без reconstruction witness, fixed-hidden constructor или другого certified complement authority.

Group остаётся read-derived semantics там, где aggregate write-back policy не определён отдельным contract.

---

## 28. Observation repair

Если transaction наблюдала `q(R)`, concurrent change `dR` может быть classified через exact differential effect:

```text
Dq(R, dR)
```

Если result `NoChange`/certified unaffected, observation может сохранять validity без full restart.

Если semantic impact не доказан, runtime не должен считать observation unaffected только потому, что physical pages не пересеклись.

---

# Part V. Physical execution

## 29. Logical vs physical authority

Logical semantics задаются `R=(S,Γ,M)`.

Physical execution может использовать:

- row/column storage;
- dense IDs;
- stable row handles;
- semantic indexes;
- hash/sort/merge/native join families;
- maintained operator state;
- APNF/QCN factors;
- workload statistics;
- materializations;
- caches;
- compiled plans.

Ни один из этих объектов не может становиться независимой semantic authority.

---

## 30. Physical lowerability principle

CFMD не обещает «быть быстрее любой специализированной БД всегда».

Нормативная цель:

> Логическая модель CFMD не должна вынуждать representation asymptotically хуже стандартной специализированной структуры для того же declared workload class.

Примеры допустимого erasing lowering:

```text
Product       -> row/column/native tuple
Option        -> validity bitmap + payload
Sum           -> tag + payloads
Set/Bag       -> hash/sorted/column representation
Seq           -> offsets/tree/rope/order structure
Map           -> hash/sorted map
μ             -> tagged recursive/nested encoding
Entity        -> stable external id + dense local id
Relation      -> flat columns/index/CSR/factor/native join input
```

Абстракция должна стираться compiler/planner-ом, а не оставаться обязательным heap wrapper на каждом value.

---

## 31. Maintained execution graph

Current runtime использует NodeId-addressed maintained execution graph/state arena.

Source change проходит:

```text
authoritative source delta
        -> plan transition
        -> GraphPatchSet
        -> validate guards/version bindings
        -> commit patch
        -> materialize root RelationDelta
```

Scratch/queues/inboxes являются reconstructible runtime optimization и не входят в revision identity.

Failure во время planning не должен partially мутировать authoritative maintained state.

---

## 32. Semantic indexes

Semantic index обязан быть связан как минимум с:

```text
(schema revision,
 semantic environment,
 semantic module binding,
 indexed expression/layout identity)
```

Stale semantic index должен быть rejected, rebuilt или excluded from planning.

Нельзя переиспользовать index только потому, что raw Rust key type тот же.

---

## 33. Statistics и advisor

Statistics/advisor могут выбирать physical strategy, но не менять semantics.

Telemetry может влиять на:

- build/drop semantic indexes;
- join family;
- materialization policy;
- recovery/rebuild economics;
- quotient factor retention.

Но advisor output является policy input, а не logical truth.

Deterministic replay/recovery не обязан восстанавливать exact ephemeral advisor state, если durable recipe/policy semantics позволяют reconstruct decision state.

---

## 34. Memory/resource accounting

Shared resource pressure должен быть first-class bounded concern.

Runtime обязан различать как минимум:

- authoritative logical/physical state;
- maintained materializations;
- reconstructible indexes;
- caches/scratch;
- recovery work.

Eviction reconstructible state допустима; потеря authority — нет.

---

# Part VI. Runtime revision publication

## 35. RuntimeRevisionBundle

Reader-visible authoritative runtime owner объединяет:

```text
RuntimeRevisionBundle =
    root identity
  + logical Revision
  + violation state
  + PhysicalStore
  + relation layout bindings
  + materialization specs
  + maintained materializations
```

Этот bundle публикуется как единое целое.

---

## 36. Immutable reader snapshots

Readers получают immutable shared snapshot root.

Publication меняет root atomically; reader, удерживающий старый snapshot, продолжает видеть coherent old revision.

Нельзя mutably «доправить» несколько частей root после того, как новый root стал reader-visible.

---

## 37. Prepared transition

Prepared transition:

```text
PreparedRuntimeRevisionTransition =
    source identity
  + commit descriptor
  + complete candidate RuntimeRevisionBundle
  + output deltas
```

Prepared state не authoritative.

Seal/publication обязаны проверять source freshness/root lineage и не допускать stale prepared candidate.

---

## 38. Versioned/COW ownership

Candidate state может использовать persistent/COW sharing с authoritative state, если:

- candidate mutation не меняет bytes/objects, наблюдаемые старым root;
- ownership/version guards предотвращают stale alias commit;
- seal оставляет один coherent new root.

COW — physical optimization, не изменение transaction semantics.

---

# Part VII. Durability and recovery

## 39. Durable authority

Durable authority не определяется «последним файлом, который существует».

Она определяется versioned immutable-generation + WAL protocol и accepted publication record.

В общих обозначениях:

```text
DurableState = (Generation G, WAL tail W, AuthorityRecord A)
```

где `A` ссылается только на prerequisites, которые должны быть durable до authority publication.

---

## 40. Publication ordering

Нормативная publication sequence использует file/directory durability barriers.

Упрощённо:

```text
1. write candidate immutable generation/components
2. fsync required candidate files
3. fsync prerequisite directories where required
4. write pending/new authority record
5. fsync authority file
6. atomic rename/publication step
7. fsync containing directory
8. only after publication may obsolete authority/components be GC'd
9. GC removal followed by required directory sync
```

Конкретная реализация может оптимизировать protocol, но обязана refinement-equivalent этому safety model для supported profile.

---

## 41. Crash safety obligations

Mechanized publication model фиксирует по крайней мере:

1. **Unique authority** — recovery не выбирает две conflicting authoritative generations.
2. **No premature authority** — candidate не authoritative до publication point.
3. **Old-authority safety** — до publication старый valid authority остаётся recoverable.
4. **Rename uncertainty bounded** — crash around atomic rename допускает только формально разрешённые states.
5. **Publication closure** — новый authority не ссылается на недолговечные prerequisites.
6. **Recovery closure** — любой reachable crash state восстанавливается в permitted authority state либо fail-closed.
7. **GC non-interference** — GC не удаляет data, необходимую current authority.
8. **GC crash safety** — crash во время GC не уничтожает published authority.
9. **Generation monotonicity** — generation authority не регрессирует внутри protocol.
10. **Production refinement** — Rust fault points связаны с formal transition vocabulary.

---

## 42. WAL

WAL является частью exact durable protocol, а не произвольным append log.

Durable WAL records должны быть связаны с exact base/head identity и transaction/effect identity.

Torn/uncertain commits не должны silently интерпретироваться как success или clean rollback без protocol evidence.

Streaming checkpoint/cut semantics обязана учитывать transactions crossing checkpoint cut; shadow tail и chunk-root publication не могут терять authoritative logical effects.

---

## 43. Recovery

Recovery owner восстанавливает logical authority из durable generation/WAL и затем реконструирует physical/reconstructible runtime state.

Physical index/materialization/cache не должен быть принят только потому, что он лежит на диске; его binding/version/recipe должны быть compatible с recovered semantic revision.

При authority uncertainty runtime обязан fail-stop/RecoveryRequired, а не продолжать serving mixed state.

---

## 44. External freshness / anti-rollback

Local durable store не может сам доказать, что его собственная директория не была откатана к старому, но криптографически valid snapshot.

Для externally anchored store вводится freshness cut:

```text
F = (generation,
     WAL/head identity,
     trust/deployment identity,
     authenticated digest)
```

и внешний monotonic CAS authority:

```text
compare_and_advance(expected, next)
```

External authority находится в отдельном rollback domain/process и владеет signing key/state отдельно от database process.

Store open/recovery с external freshness MUST:

1. получить/проверить external cut;
2. сравнить local generation/WAL с ним;
3. отклонить rollback/fork/truncation;
4. только затем выполнять ordinary recovery/publication.

Unavailable/ambiguous external authority не разрешает local fallback.

Response loss после успешного external CAS должен сходиться через re-read/compare protocol, а не превращать один durable effect в второй.

---

## 45. Supported-platform certification

Формальный proof publication protocol использует filesystem axioms. Реальный support claim требует empirical certification exact platform profile.

Текущий сертифицированный профиль:

```text
QEMU 8.2.2
TCG software acceleration
Alpine Linux 3.24.2
Linux 6.18.52-0-virt x86_64
512 MiB dedicated raw virtio data device
ext4 data=ordered
QEMU cache=none,aio=threads
```

Certification campaign включает 7 destructive cuts:

```text
after-candidate-file-sync
after-prerequisite-directory-sync
after-pending-manifest-sync
after-manifest-rename
after-manifest-directory-sync
after-obsolete-remove
after-obsolete-directory-sync
```

Для каждого case:

```text
arm exact cut
-> hard power cut whole VM
-> fresh boot
-> verify allowed recovered state
```

Текущий profile прошёл `7/7`, signed campaign verification и certified-store create/reopen.

Это **не** сертифицирует автоматически:

- bare-metal NVMe;
- SATA;
- другой filesystem;
- XFS;
- другой kernel;
- другой QEMU cache mode;
- NFS/network filesystem;
- Windows storage stack.

Каждый новый support profile требует отдельной evidence campaign.

---

# Part VIII. Replication and consensus

## 46. Replication authority

Replication не создаёт вторую logical semantics. Она координирует authority над revision effects.

Текущий consensus state включает:

- membership epoch;
- term promises;
- leader votes/certificate;
- decision votes;
- decision locks;
- joint membership certificates;
- quorum-loss/recovery state;
- authenticated peer evidence;
- durable replay.

---

## 47. Leader authority

Leader authority существует только при valid certificate для exact:

```text
(membership_epoch, term, leader)
```

Наблюдение heartbeat/transport message само по себе не создаёт leader authority.

Term promise/vote state durable и предотвращает contradictory authority после restart.

---

## 48. Decision lock

Authority-bearing revision effect publication требует matching decision lock, связанный как минимум с:

```text
membership_epoch
term
leader
position
effect
```

и quorum evidence текущей policy.

Anti-entropy или heartbeat не может создать decision lock.

---

## 49. Authenticated peer evidence

Peer evidence проверяется до authority journal.

Signed evidence domain-separates как минимум:

```text
protocol/domain version
cluster identity
evidence kind
membership epoch
term
voter identity
key epoch
exact payload digest
```

Тем самым запрещаются:

- cross-cluster replay;
- cross-term substitution;
- cross-membership substitution;
- voter substitution;
- evidence-kind confusion;
- payload/effect/candidate substitution.

Authority journal должен принимать verified evidence/receipt, а не caller assertion «этот voter подтвердил».

---

## 50. Quorum loss

Quorum availability — explicit authority state.

При quorum loss запрещено создавать новые authority transitions, требующие consensus, включая new leader/decision/membership publication.

Локальная durability во время quorum loss не превращается автоматически в distributed authority.

Recovery/rejoin требует authenticated recovery evidence/certificate.

---

## 51. Membership change

Membership transition должен быть quorum-safe относительно old/new configurations.

Joint membership certificate связывает:

```text
previous epoch
term
leader
next membership
acknowledgements by previous
acknowledgements by next
```

Нельзя менять membership во время quorum-loss через административный shortcut, обходящий consensus authority.

---

## 52. Transport

Transport frame имеет canonical bounded encoding и authentication/replay fencing.

Transport session sequence — ephemeral anti-replay/dedup state. Он не заменяет durable term/vote/lock authority.

Transport MAY быть перенесён на TCP/QUIC/shared memory/иной I/O layer, если stable wire/security/authority contracts сохраняются.

---

## 53. Anti-entropy и failure detection

Anti-entropy:

- сравнивает durable decision-lock frontier;
- запрашивает bounded missing chunks;
- не создаёт vote/lock authority самостоятельно.

Failure detector advisory:

- может инициировать quorum-loss fencing/coordination;
- не может самостоятельно назначить leader или выполнить recovery.

---

# Part IX. Authentication, deployment and trust

## 54. Cryptographic primitives

Current authenticated boundary использует SHA-256 и Ed25519 через vendored Rust dependencies.

Cryptographic authentication отвечает на вопрос:

```text
"это exact bytes/evidence, подписанные разрешённым key?"
```

Она **не** отвечает автоматически на вопрос:

```text
"эта implementation семантически корректна?"
```

Semantic refinement и cryptographic authority являются разными obligations.

---

## 55. Trust roots и key lifecycle

Trust policy включает root/key epochs, rotation/revocation и fail-closed stale-key handling.

Signed records обязаны быть domain-separated и содержать достаточно context, чтобы signature нельзя было перенести между protocol objects.

Weak/malformed key material rejected до authority use.

---

## 56. Semantic package deployment

External semantic implementation поставляется как canonical bounded package envelope.

Admission pipeline:

```text
untrusted repository bytes
-> exact expected Γ descriptor match
-> content digest verification
-> signature/trust policy
-> deployment/revocation policy
-> refinement checker authorization
-> runtime-profile authorization
-> bounded ABI invocation
```

Filesystem repository/CAS являются byte sources, а не trust authority.

---

## 57. Sandboxed execution

Нативная arbitrary plugin `.so/.dll` загрузка в DB process не является текущей architecture.

Current Linux backend запускает authenticated semantic artifact out-of-process под constrained namespace/process profile с bounded request/response и wall-time/fuel contract.

Semantic runtime profile является частью authenticated/deployment binding.

Runtime substitution между «проверенным package» и фактическим execution environment запрещена.

---

# Part X. Formal assurance

## 58. Что именно формализовано

CFMD не заявляет full machine-code verification всего Rust workspace.

Формальный boundary точечный и executable.

### 58.1 Publication proof

`formal/lean/CFMD/Publication.lean` механизирует immutable-generation publication/crash/GC safety model.

Mechanized obligations включают:

```text
P18.1 unique authority
P18.2 no premature authority
P18.3 old-authority safety
P18.4 rename uncertainty
P18.5 publication closure
P18.6 recovery closure
P18.7 GC non-interference
P18.8 GC crash safety
P18.9 generation monotonicity
P18.10 production refinement mapping
```

### 58.2 Surface-to-kernel proof

`formal/lean/CFMD/SurfaceKernel.lean` механизирует:

- complete surface vocabulary binding;
- structural type preservation;
- guarded-recursion/free-variable preservation;
- lowering round-trip;
- logical node-count/no-hidden-expansion property;
- checked-plan semantic preservation;
- production checker contract soundness;
- end-to-end surface-to-kernel preservation.

---

## 59. Source-refinement binders

Lean theorem сам по себе не доказывает, что production Rust остался тем же protocol.

Поэтому repository имеет fail-closed source binders, которые проверяют exact production vocabulary/landmarks/hashes/structure, относящиеся к theorem surface.

Если proof-relevant Rust vocabulary меняется, CI должен требовать formal revalidation, а не молча считать старый theorem применимым.

---

## 60. Proof-carrying boundary

Общий architecture principle:

```text
large/untrusted search, optimizer, solver, deployment source
                    |
                    v
           artifact + witness/certificate
                    |
                    v
             small checker boundary
```

Типовые judgments:

```text
WellFormed(x)
Equivalent(a,b)
Refines(impl,spec)
PreservesInvariant(r,P)
HasLaw(op,law)
Terminates(solver,witness)
```

Certificate должен быть связан с exact specification/module/version identity.

---

# Part XI. Current implementation architecture

## 61. Workspace decomposition

Current repository содержит 25 internal crates.

### 61.1 Logical/type core

- `kernel-types` — stable IDs, revision-local coordinates, physical handle primitives.
- `kernel-schema` — `TypeExpr`, schema, semantic environment/context.
- `kernel-model` — logical database state/value model.
- `kernel-identity` — identity projections/transports.
- `kernel-lifecycle` — liveness/ownership normalization.
- `kernel-validation` — typed model/relation validation.
- `kernel-violation` — violation/invariant representation.

### 61.2 Query/change/write

- `kernel-change` — universal `Change`, FineChange, rewrite/coherence/effect calculus.
- `kernel-query` — `RelExpr`, exact execution/maintained differential state.
- `kernel-aggregate` — aggregate semantics.
- `kernel-fixpoint` — admitted fixed-point machinery.
- `kernel-grounded-closure` — finite grounded closure substrate.
- `kernel-lens` — dependent lens/complement/writable-view substrate.
- `kernel-retention` — retention policy primitives.

### 61.3 Semantics / planning / physical

- `kernel-semantics` — semantic registry, observables, canonical keys, APNF/morphisms.
- `kernel-semantic-index` — Γ-bound semantic indexes.
- `kernel-plan` — physical planning, maintained runtime bundle, advisors, QCN/native execution.
- `storage-memory` — in-memory physical store implementation.
- `kernel-integration` — cross-layer integration boundary.

### 61.4 Revision / distribution / durability

- `kernel-revision` — coherent logical revision construction/validation.
- `kernel-transport` — semantic law/transition transport and distributed integration substrate.
- `kernel-durability` — store, WAL, immutable generations, replication authority, platform assurance, external freshness.

### 61.5 Trust / proof / deployment

- `kernel-auth` — cryptographic identities, signed records, freshness primitives.
- `kernel-deployment` — semantic package verification/deployment/sandbox profile.
- `kernel-proof` — checked certificate/plan proof vocabulary.

---

## 62. Internal API stability

Все `kernel-*` crates сейчас считаются internal implementation modules.

Их public Rust symbols существуют для workspace composition/tests, но не являются обещанным end-user semver surface.

Будущий пользовательский Rust API обязан скрывать большую часть:

- NodeId/state arena;
- RuntimeRevisionBundle internals;
- raw semantic catalog IDs;
- durability protocol records;
- consensus journal internals;
- planner/advisor implementation types.

---

## 63. Verification baseline

Projectized baseline после закрытия historic kernel backlog имеет:

```text
25 workspace crates
766 passing tests
0 failed tests
8 ignored benchmark-style tests
774 declared tests
```

Repository gates:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --all-targets --offline
formal refinement checks
Lean proof CI on proof-relevant changes
```

Historical kernel problem ledger: `22 / 22 PROD CLOSED` для заявленного scope.

---

# Part XII. Global invariants

## 64. Invariant G1 — semantic pinning

Любая observable equality/order/canonicalization operation обязана использовать semantic module pinned exact revision.

```text
observable(x) @ R
```

не может использовать module от `R' != R` без explicit certified transport.

---

## 65. Invariant G2 — coherent runtime root

Reader-visible runtime root содержит одну coherent logical/physical/materialized revision.

Mixed-root publication запрещена.

---

## 66. Invariant G3 — prepared is not authoritative

Planning/preparation/candidate construction не создаёт authority.

Authority меняется только на declared seal/publication boundary.

---

## 67. Invariant G4 — exact delta correctness

Каждый specialized differential path обязан быть observationally equal exact recomputation semantics.

При невозможности доказать specialized path runtime должен использовать safe fallback или reject transition.

---

## 68. Invariant G5 — reconstructible physical state

Любой cache/index/materialization, не являющийся declared durable authority, обязан быть:

- reconstructible;
- version/binding checked;
- safely discardable.

---

## 69. Invariant G6 — fail closed on authority ambiguity

Неопределённость относительно:

- durable commit;
- external freshness;
- consensus authority;
- semantic package identity;
- platform certification;

не может превращаться в optimistic success.

---

## 70. Invariant G7 — authentication is not semantics

Valid signature не означает valid semantic refinement.

Valid refinement witness без valid deployment authority не означает authorized executable package.

Обе проверки обязательны там, где обе применимы.

---

## 71. Invariant G8 — no ambient semantic callbacks

Arbitrary Rust/Python/C callback не может участвовать в exact query/invariant semantics без explicit certified semantic module contract.

---

## 72. Invariant G9 — support claims are profile-scoped

Наличие `fsync`, ext4 или Linux в имени окружения не является durability certificate.

Поддержка storage stack существует только для exact certified profile/fingerprint/evidence contract.

---

## 73. Invariant G10 — formal proofs are source-bound

Proof artifact считается применимым к production только пока source-refinement boundary проходит.

Изменение proof-relevant production vocabulary обязано invalidировать старую автоматическую уверенность.

---

# Part XIII. Explicit non-goals and boundaries

## 74. Не универсальный SQL clone

CFMD core не определяется SQL grammar.

SQL может быть future surface/lowering frontend, но SQL-specific quirks не должны становиться foundation logical semantics.

---

## 75. Не ORM поверх обычной БД

Object/document/graph/relational views должны elaborated в один typed kernel, а не жить как независимые peer data models.

---

## 76. Не «всё CRDT»

Automatic merge разрешён только там, где есть необходимые commutation/coherence/invariant proofs.

Отсутствие proof означает conflict/coordination, а не silent merge heuristic.

---

## 77. Не arbitrary plugin host

Dynamic native extension, имеющий произвольный доступ к process memory, не является поддерживаемой trust model.

---

## 78. Не magic durability

CFMD не может доказать power-loss behavior неизвестного controller/filesystem только из POSIX API documentation.

Formal protocol proof и empirical platform certification — отдельные уровни assurance.

---

## 79. Не скрытая approximation

Approximate search, ANN, AQP или probabilistic algorithm не может подменять exact operator с тем же API contract.

Если такие surfaces появятся, они должны иметь отдельный type/contract.

---

# Part XIV. Future / Non-Normative Roadmap

Этот раздел **не является текущим обязательным kernel contract**. Он фиксирует следующий product/engineering layer после завершения core R&D.

## F1. Stable user-facing Rust crate

Первый следующий product milestone — один facade crate, условно:

```text
cfmd
```

Пользователь не должен импортировать 25 внутренних `kernel-*` crates.

Целевая форма API:

```rust
use cfmd::{Database, Schema, Transaction, Query};

let db = Database::open("app.cfmd")?;

let users = db.schema().relation("users")?;

let result = db.read(|r| {
    r.query(/* typed/query-builder expression */)
})?;

let commit = db.transaction(|tx| {
    // typed writes / rewrites
    Ok(())
})?;
```

Конкретные имена ещё не нормативны.

### Требования к facade

- internal kernel types hidden;
- stable error taxonomy;
- explicit resource/lifecycle semantics;
- borrowed/owned result model без accidental lifetime traps;
- bulk operations без per-row API overhead;
- async не должен навязываться embedded synchronous use case;
- semantic configuration `Γ` должна быть expressible без exposure raw internal registries;
- safe defaults не должны скрывать semantic choices.

---

## F2. Public error model

Нужна стабильная taxonomy, отделяющая:

```text
UserInput / Schema
SemanticMismatch
ConstraintViolation
Conflict
Durability
RecoveryRequired
QuorumUnavailable
Authentication
Deployment
UnsupportedPlatform
InternalInvariant
```

Internal 25-crate error enums не должны протекать напрямую в public semver API.

---

## F3. Schema builder / typed schema surface

Нужен ergonomic Rust surface для определения:

- entities;
- structural values;
- relations;
- Set/Bag/Seq/Map semantics;
- semantic equality/order modules;
- constraints;
- indexes/materialization hints как non-authoritative policy.

Surface должен elaborated в текущие `Schema`/`SemanticContext` contracts и проходить тот же checker boundary.

---

## F4. Query builder / macro layer

Варианты future ergonomic layer:

```text
builder API
procedural macros
compile-time typed handles
dynamic runtime schema handles
```

Любой frontend обязан снижаться в существующий exact query IR или его совместимое расширение и не обходить `Γ` semantics.

---

## F5. Transaction/rewrite DX

Пользовательский write API должен выражать semantic intent без exposure внутренних residual/cube types для обычных случаев.

Advanced API MAY expose explicit conflict/coherence policy для distributed/concurrent applications.

---

## F6. Introspection and diagnostics

Нужны stable user surfaces для:

- explain logical query;
- explain physical plan;
- show semantic module bindings;
- show revision/root identity;
- show index/materialization health;
- durability/support profile;
- replication/quorum status;
- recovery-required reason;
- storage/rebuild economics.

Diagnostics не должны давать mutation backdoor во внутреннюю authority state.

---

## F7. Packaging

После facade:

- normal Cargo package/repository release;
- examples;
- rustdoc;
- changelog/semver discipline;
- reproducible offline/vendor path для controlled builds;
- release binary-size and performance regression gates.

---

## F8. Python binding

Python/PyO3 — downstream facade, не replacement Rust API.

Архитектура:

```text
Python API
   -> thin PyO3 layer
   -> stable cfmd Rust facade
   -> internal kernel
```

Python binding не должен связываться напрямую со множеством внутренних crates.

Для performance нужны bulk/batch FFI boundaries.

---

## F9. Other language bindings

Node/C#/Java и другие bindings имеют смысл только после стабилизации Rust facade и ownership/error model.

---

## F10. SQL compatibility frontend

SQL parser/compatibility — optional future frontend.

Он должен быть отдельным elaboration layer над CFMD semantics.

Не следует делать SQL semantics новым внутренним kernel authority.

---

## F11. Additional certified storage profiles

Новые профили должны добавляться как support-matrix entries с собственной campaign evidence:

- bare-metal Linux + NVMe/ext4;
- Linux + XFS;
- SATA profiles;
- Windows/NTFS или другой Windows durability protocol;
- cloud block devices;
- network filesystems — только если можно сформулировать и доказать другой корректный authority protocol.

Новый профиль расширяет support matrix и не меняет математическое ядро.

---

## F12. Performance hardening

После public API нужны reproducible end-to-end benchmarks:

- point lookup;
- inserts/updates;
- bulk load;
- Group/TopK/Join maintained chains;
- recursive queries;
- checkpoint/recovery;
- replication overhead;
- small embedded workloads;
- comparison с SQLite/PostgreSQL/embedded competitors только на честно сопоставимых semantics.

Microbenchmark gap сам по себе не должен приводить к semantic special-case, если end-to-end cost уже acceptable.

---

## F13. More formalization

Текущие Lean proofs покрывают selected critical boundaries, а не весь kernel.

Возможные future proof targets:

- more of rewrite/coherence calculus;
- lifecycle normalization theorem;
- exact differential composition theorem linked deeper to production;
- replication safety model;
- external freshness protocol;
- semantic index refinement;
- retention/erasure transition properties.

Это расширение assurance, а не prerequisite для использования уже закрытого current core scope.

---

## F14. Approximate query types

Если проект добавит ANN/AQP/heuristic search, они должны быть first-class weaker contracts:

```text
ApproxQuery<T, Guarantee>
HeuristicSearch<AlgorithmId, BuildId, Budget, Seed>
```

а не hidden physical implementation exact `Query<T>`.

---

## F15. Retention/privacy product layer

Current kernel содержит retention primitives, но end-user privacy/erasure product semantics требуют отдельного threat-model/API design.

Следует явно различать:

```text
logical deletion
system-derived deletion
physical/cryptographic erasure
identity erasure
inference-aware privacy transformations
```

Нельзя обещать inference erasure arbitrary correlated data одной операцией `DELETE`.

---

## F16. Schema evolution UX

Kernel substrate поддерживает versioned semantic/schema transitions и lens/rewrite machinery, но user-facing migration language должен быть отдельным product layer:

- additive changes;
- renames preserving identity;
- checked transforms;
- retained complements;
- irreversible explicit forget;
- migration dry-run/explain;
- compatibility window.

---

## F17. Operational tooling

Для production-like use понадобятся:

- inspect/verify command-line tool;
- backup/restore orchestration;
- certification/evidence tooling;
- replication bootstrap/rejoin commands;
- metrics export;
- structured logs;
- corruption/recovery diagnostics.

CLI не должен становиться вторым authority path, обходящим library contracts.

---

# Appendix A. Compact law table

## A.1 Revision coherence

```text
R = (S, Γ, M)
well_formed(R) :=
    well_formed(S)
  ∧ Γ satisfies semantic_dependencies(S)
  ∧ typed(M,S,Γ)
  ∧ lifecycle_normal(M)
  ∧ invariants_hold(M)
```

## A.2 Semantic revision

```text
semantic_revision(R) = (schema_id(S), environment_id(Γ))
```

## A.3 Lifecycle

```text
Live(M) = μX. Roots(M) ∪ KeepsAlive(M)[X]
N(N(M)) = N(M)
```

## A.4 Exact query

```text
q : Revision × Input -> Output
```

with deterministic/pure/extensional semantics relative to pinned revision.

## A.5 Universal change

```text
apply(x, NoChange)   = x
apply(x, Replace(y)) = y
apply(x, Fine(d))    = endpoint(d)
```

## A.6 Derivative correctness

```text
apply(q(x), Dq(x, dx)) = q(apply(x, dx))
```

## A.7 Composition

```text
D(g∘f)(x,dx) = Dg(f(x), Df(x,dx))
```

## A.8 Constraint

```text
Valid_P(M) <=> Viol_P(M) = ∅
```

## A.9 Lowering refinement

```text
⟦lower(q)⟧_R = ⟦q⟧_R
```

## A.10 Rewrite diamond

```text
res_b_after_a(a(x)) ≡ res_a_after_b(b(x))
```

when a certified commuting/residual law exists.

## A.11 Determinant closure

```text
Cl_D(A) = μX. A ∪ consequences_D(X)
```

## A.12 APNF factorization

```text
F ≅ (anchor measure μ_A, certified reconstruction A -> C\A)
```

when anchor projection is injective on distinct support.

## A.13 Runtime authority

```text
ServingRoot = coherent(Revision, PhysicalStore, Materializations)
```

and prepared candidate is never serving authority before seal/publication.

## A.14 Durable freshness

```text
local_head must be compatible with external monotone FreshnessCut
```

before externally anchored store may become serving.

## A.15 Consensus authority

```text
transport observation != vote
heartbeat != authority
anti-entropy != decision lock
```

Only durable authenticated quorum evidence creates consensus authority.

---

# Appendix B. Current exact relational operator table

| Operator | Semantic role | Differential class / required state |
|---|---|---|
| `Scan` | authoritative relation leaf | Source |
| `FilterEqConst` | semantic equality predicate | Linear |
| `FilterEqColumns` | intra-row semantic equality | Linear |
| `Project` | column projection | Linear or ZeroCrossing depending collection semantics |
| `JoinEq` | equality join | BilinearPullback / join fibers |
| `Difference` | left minus right support | BlockerZeroCrossing |
| `AntiJoin` | emit left when no right blocker | BlockerZeroCrossing |
| `Distinct` | quotient support `>0` | ZeroCrossing / support counts |
| `Group` | group annotations + aggregate | Annotation |
| `TopKWithTies` | ordered semantic boundary | OrderedBoundary |
| `PromoteToBag` | explicit collection-semantic promotion | linear structural transport |

Physical implementation MAY specialize these classes, but output semantics MUST remain exact.

---

# Appendix C. Current support statement

### Core

```text
Historical production kernel backlog: 22 / 22 closed
```

### Formal

```text
Publication/GC model: mechanized in Lean
Surface-to-kernel preservation: mechanized in Lean
Production source binding: fail-closed refinement scripts
```

### Durability

```text
One declared QEMU/TCG + Linux/ext4 profile: destructively certified 7/7
Other storage profiles: unsupported until separately certified
```

### Distribution

```text
Consensus/replication authority: implemented
Authenticated peer evidence: implemented
Quorum loss/recovery: implemented
Anti-entropy/transport/fault tests: implemented
```

### Trust/deployment

```text
SHA-256 + Ed25519 auth boundary: implemented
Trust-root/key lifecycle: implemented
Semantic package verification/deployment: implemented
Out-of-process Linux sandbox profile: implemented
External monotone freshness authority: implemented
```

### Product API

```text
Stable end-user Rust facade: FUTURE
Python binding: FUTURE
SQL compatibility frontend: FUTURE
```

---

# Appendix D. Change control for this specification

Этот документ должен изменяться только при одном из событий:

1. изменился normative logical contract;
2. добавлен новый current kernel feature, который меняет observable semantics;
3. новый proof изменил/уточнил обязательный law;
4. обнаружен counterexample текущей формулировке;
5. internal implementation boundary стала user-facing/stable;
6. future item стал current и прошёл required verification.

Обычный optimization/pass, который не меняет contract, **не должен** добавлять очередной append-only раздел `PassXYZ correction`.

История реализации должна жить в reports/changelog/git history.

Если новый R&D результат противоречит этой spec, сначала должен быть явно сформулирован counterexample и новая law/contract, затем обновлены tests/proofs/runtime, и только после этого — этот документ.

---

# End state

На текущем baseline CFMD следует рассматривать как завершённое исследовательское **core/kernel** с закрытым историческим архитектурным backlog для объявленного scope. Следующая главная работа — не расширять kernel без необходимости, а создать стабильную product surface вокруг уже существующего ядра:

```text
stable Rust facade
-> ergonomic schema/query/write API
-> diagnostics/tooling
-> packaging/docs/examples
-> end-to-end application validation
-> language bindings
```

Математическое и authority ядро остаётся основанием этих surfaces; пользовательский DX не должен обходить или дублировать его.
