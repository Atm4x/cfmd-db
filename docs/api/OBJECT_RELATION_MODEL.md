# CFMD Object Relation Model

The application-facing schema is object-first. A Rust declaration such as:

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user")]
struct User {
    #[cfmd(id)]
    id: Id<User>,
    name: String,
    children: Many<Child>,
}
```

is the public model. `Child` does not need a synthetic `parent: Ref<User>` merely to make `User.children` exist. CFMD lowers `children` into an internal typed edge relation and may execute it through relation/Γ machinery, but that representation is not the user's mental model or normal API.

## Read semantics

`Ref<T>` and `Many<T>` materialized from a database are bound to the exact snapshot of their owner. Reading the Rust field is inert. Evaluation is explicit:

```rust
let user = snapshot.objects::<User>()?.require(user_id)?;
let count = user.children.count()?;
let adults = user.children.where_(|c| c.age().greater_than_or_equal(18))?.all()?;
let all = user.children.all()?;
```

`Ref<T>` follows the same law through `ref.query()` / `ref.load()`. There is no implicit `include`, no field-access I/O and no lazy N+1 loop hidden by Rust indexing syntax.

## Write semantics

Detached relation values describe an object graph:

```rust
let user = User::cfmd_new(
    user_id,
    "Alice".into(),
    Many::new([
        Child::cfmd_new(...),
        Child::cfmd_new(...),
    ]),
);
let plan = snapshot.objects::<User>()?.insert(user)?;
```

The graph lowers into one Plan containing object rows and internal relationship facts. A bound `Many<T>` returned unchanged from `update` preserves the relationship. Replacing it with `Many::new(...)` or `Many::empty()` replaces that relationship atomically.

## Internal lifecycle

The edge relation contains historical endpoint identities for query equality plus live endpoint witnesses for lifecycle authority. If either endpoint is deleted, kernel revision normalization removes the dangling current edge. Commit descriptors are computed from the exact normalized target revision, so the durable relation delta and the object lifecycle transition cannot disagree.


## Relationship-set mutation

A concrete `Many<T>` / `OwnedMany<T>` remains mutable as a relationship value. `where_(...)` produces a snapshot-bound relationship selection that can be read or mutated. `move_to` and `detach_all` evaluate only target identities and emit edge mutations; they do not materialize target objects. Explicit `delete_all` deletes objects and relies on lifecycle normalization to remove dangling edge facts. `OwnedMany<T>` preserves its exclusivity/orphan contract across filtered mutations.
