# CFMD — Migratable Watch DX / Semantic Subscription Model

## 1. Что такое `watch`

`watch` — долгоживущее semantic-наблюдение за результатом чтения БД.

Он не должен быть scoped-child `Context<M>`, потому что:

- `Context<M>` — bounded/scoped working world;
- `Watch` может жить секунды, часы или весь процесс;
- lifetime watch не должен зависеть от lifetime Context.

Поэтому ownership-направление:

```text
Database
    -> Watch / MigratableWatch

Context<M>
    -> scoped read/write execution
```

Query при этом может быть самостоятельным immutable typed-описанием, например:

```rust
let q = User::query()
    .filter(|u| u.active.eq(true));
```

Один и тот же query может использоваться и для scoped read, и для долгоживущей подписки:

```rust
ctx.read(&q)?;
db.watch(q)?;
```

## 2. Обычный `Watch<T>`

Обычный watch имеет заранее известный public output type:

```rust
Watch<String>
Watch<UserV1>
Watch<Vec<UserV1>>
```

Он привязан к schema/type contract, в котором был создан.

Если происходит schema migration и этот binding перестаёт быть authoritative, обычный `Watch<T>` не должен скрытно менять тип или semantic meaning.

Базовый закон:

```text
Watch<T>
    schema-bound
    output type fixed eagerly

schema boundary
    -> terminate / SchemaChanged
```

То есть:

```rust
let watch: Watch<UserV1> = ...;
```

никогда не должен внезапно начать выдавать `UserV2`.

## 3. Почему `Watch<UserV1>` недостаточен для zero-downtime migration

Если:

```text
Schema A:
    UserV1

migration A -> B

Schema B:
    UserV2
```

то долгоживущий:

```rust
Watch<UserV1>
```

уже зафиксировал старый materialization contract.

Даже если БД способна перенести semantic observation в новую схему, public type старого stream уже определён.

Поэтому migration-aware watch не должен быть типизирован схемой результата заранее.

## 4. `MigratableWatch`

`MigratableWatch` — долгоживущая semantic subscription, которая подписывает не materialized объект, а semantic observation contract.

Примерно:

```rust
let mut watch = db.migratable_watch(User::query(...))?;
```

На старте БД компилирует query в собственный semantic descriptor:

```text
query
    -> D_A
    -> maintained subscription
```

При migration:

```text
A -> B
```

сама migration-модель задаёт перенос semantic descriptor:

```text
D_A
    -- migration transform -->
D_B
```

Подписка остаётся той же логической подпиской, но её внутренняя реализация/descriptor теперь относится к B-world.

Важно:

```text
MigratableWatch
    НЕ является Watch<UserV1>
    НЕ является Watch<UserV2>
    НЕ выбирает "current type"
```

Тип появляется только при явной материализации конкретного события.

## 5. Поздняя материализация

`MigratableWatch::next()` должен возвращать schema-neutral change/event.

Условный DX:

```rust
let mut watch = db.migratable_watch(User::query(...))?;

while let Some(change) = watch.next().await? {
    change.materialize_schema! {
        A::ID => |ctx: Context<A>, event: Change<A>| {
            // обработка A
        },

        B::ID => |ctx: Context<B>, event: Change<B>| {
            // обработка B
        },
    }
}
```

Точный синтаксис не выбран.

Смысл:

```text
MigratableWatch::next()
    -> schema-neutral committed change

materialize_schema(...)
    -> explicit schema match
    -> typed materialization в конкретный Context<M>
```

Никакого понятия "текущий тип" у watch нет.

Пользователь сам явно перечисляет schema branches, которые умеет обрабатывать.

## 6. Payload лучше как change/delta, а не просто объект

Для entity-наблюдения удобнее возвращать изменение:

```rust
Change<T> {
    before: Option<T>,
    after: Option<T>,
}
```

По смыслу аналогично event-модели:

```text
OnEventChanged(event) {
    event.before
    event.after
}
```

Примеры:

```text
create:
    before = None
    after  = Some(new)

update:
    before = Some(old)
    after  = Some(new)

delete:
    before = Some(old)
    after  = None
```

Для collection/query-result watch может понадобиться отдельный exact delta:

```rust
CollectionChange<T> {
    inserted: ...,
    removed: ...,
    updated: ...,
}
```

Но принцип тот же: watch сообщает изменение semantic result, а не просто повторно отдаёт весь current object.

## 7. Migration semantics

Пример:

```text
A.UserV1.name
    ↓ migration
B.UserV2.display_name
```

Обычный:

```rust
Watch<String>
```

может считаться schema-bound и завершиться на migration boundary.

А `MigratableWatch`:

```text
D_A(UserV1.name)
    ↓ MigrationModel
D_B(UserV2.display_name)
```

продолжает subscription.

Следующее событие приходит schema-neutral и уже материализуется через B branch.

Таким образом migration переносит:

```text
semantic observation descriptor
```

а не:

```text
public generic type
```

## 8. Если semantic observation не переносим

Migration не обязана сохранять любой watch.

Если descriptor старого observation невозможно корректно представить в новой схеме:

```text
D_A
    -> no valid D_B
```

то `MigratableWatch` должен завершиться fail-closed с явной причиной:

```text
WatchContractNotMigratable
```

или эквивалентной typed ошибкой.

Никаких default values, guessed mappings или hidden recompute semantics.

## 9. Итоговая модель

```text
Database
    long-lived runtime authority

Query
    typed/entity-oriented immutable semantic read description
    schema binding может происходить позднее

Context<M>
    scoped typed working world
    reads/writes/Candidate/commit

Watch<T>
    обычная schema-bound subscription
    public type фиксирован сразу
    migration boundary => terminate

MigratableWatch
    long-lived schema-neutral semantic subscription
    хранит/несёт migratable observation contract
    migration переносит descriptor
    next() возвращает schema-neutral change
    typed materialization происходит только через explicit schema match
```

Главный закон:

```text
Обычный Watch фиксирует materialization type заранее.

MigratableWatch фиксирует semantic subscription,
а materialization откладывается до обработки конкретного события.
```

## PASS533 — implemented foundation

PASS533 closes the authorization-frontend gate and implements the first production slice of this model.

- `Database::migratable_watch(&Query)` creates a schema-neutral exact subscription directly from the database authority.
- `MigratableQueryWatch` does not expose a typed/object result contract. Its events carry exact raw inserted/removed rows plus the target authoritative `schema_revision`.
- Ordinary `QueryWatch` remains schema-bound and still fails closed at every semantic schema boundary.
- A migratable watch may currently cross only a **definitionally equivalent** schema revision. `kernel-query::MaterializedRelPlanState` rebinds its already-maintained Γ-DTC state to the new semantic context without rebuilding rows, replaying the query, or materializing a second subscription state.
- Structural migration is deliberately rejected as `WatchUnavailable` with a contract-not-migratable diagnostic. There is no guessed mapping and no rebuild/recompute fallback.

This is intentionally narrower than the final model. The next theorem must consume the retained authoritative `SchemaMigrationProgram` and transport the semantic observation descriptor itself. Typed event materialization remains explicit future work; PASS533 only establishes the schema-neutral event boundary on which that API can be built.
