# CFMD — Embedded / Developer DX Feature Ideas

Basis: current CFMD product/kernel state around Pass533.

This document intentionally focuses on **new embedded/DX directions** rather than features that already exist in the product line.

Already-existing foundations such as `Context<M>`, `Ref<T>`, `Many<T>`, `OwnedMany<T>`, deep traversal, exact Watch, history/undo/redo, single-file storage, encryption, migration calculus and advisor-managed physical artifacts are not repeated here as new feature ideas.

---

# 1. True In-Memory Database Backend

Add a real product-level:

```rust
Storage::Memory
```

with exactly the same logical/runtime semantics as durable CFMD backends.

Example:

```rust
let db = Database::memory::<App>()?;
let ctx = db.context::<App>()?;
```

The following remain identical:

```text
Schema / Γ
Revision
Context<M>
Query
Change
Rules
Lifecycle
History
Undo/Redo
Watch
Authorization
Physical realization
```

Only the durability/storage contract changes.

## Required guarantee

`Storage::Memory` should mean:

- no WAL on the filesystem;
- no checkpoints;
- no temporary spill files created by CFMD;
- no durable physical artifacts;
- process exit destroys the database unless explicitly persisted/exported;
- same query/change semantics as durable backends.

This should not be a second simplified database engine.

---

# 2. Memory -> `.cfmd` Promotion

Allow a live in-memory database to become durable without exporting/importing logical data.

Example:

```rust
let db = Database::memory::<App>()?;

// normal work...

let db = db.persist("project.cfmd")?;
```

Desired semantic model:

```text
Revision R in volatile physical realization
        ↓
durable publication
        ↓
same logical Revision/world under durable realization
```

Not:

```text
memory rows
→ serialization
→ import
→ reconstructed unrelated database
```

Useful operations:

```rust
db.save_as("copy.cfmd")?;
db.persist("project.cfmd")?;
```

Possible distinction:

- `save_as(...)` creates a durable copy while the current DB remains memory-only;
- `persist(...)` transitions the live DB instance to durable storage.

This is especially valuable for document/editor applications:

```text
New document
→ RAM only
→ user works
→ Save
→ same DB becomes durable
```

---

# 3. Durable -> Memory Detach / Memory Fork

The inverse workflow is also useful:

```rust
let db = Database::open("project.cfmd")?;
let scratch = db.detach_to_memory()?;
```

or:

```rust
let scratch = db.fork_memory()?;
```

Use cases:

- experiments;
- temporary editing;
- import staging;
- simulation;
- tests;
- “Open as Copy” workflows.

The original `.cfmd` remains untouched until the developer explicitly persists or exports the memory world.

---

# 4. Secure Memory Profile

Ordinary at-rest encryption does not protect an in-memory database because there are no durable bytes.

A separate optional memory-security profile can cover:

```text
locked pages / mlock-like behavior
zeroize-on-release
bounded sensitive regions
external key provider for eventual persistence
```

Possible API direction:

```rust
Database::memory::<App>()?
    .memory_protection(MemoryProtection::LockedAndZeroized)
```

Do not market ordinary “encrypted RAM” without a precise threat model.

When persisting a memory database:

```rust
db.persist_encrypted(
    "private.cfmd",
    key_provider,
)?;
```

the file should be created encrypted from the first durable byte, with no temporary plaintext database.

---

# 5. `CfmdValue` — Native Structural Application Values

CFMD's logical kernel already supports structural algebraic values.

Expose them ergonomically as normal Rust values.

Example:

```rust
#[derive(CfmdValue)]
struct WindowState {
    size: Size,
    position: Option<Point>,
}

#[derive(CfmdValue)]
struct Settings {
    window: WindowState,
    recent: Vec<PathBuf>,
    shortcuts: BTreeMap<Action, KeyChord>,
    tags: BTreeSet<String>,
}
```

These should not be opaque JSON blobs.

They should remain real CFMD structural values with:

- typed schema;
- exact equality;
- structural change semantics;
- schema evolution;
- queryability where supported;
- native physical lowering.

This lets CFMD compete not only with SQLite, but also with:

```text
JSON
RON
bincode
custom save files
```

while remaining a real database.

---

# 6. Persistent Value / Slot Facade

Provide a very lightweight embedded surface over the same CFMD model:

```rust
let settings = db.slot::<Settings>("settings")?;
```

or:

```rust
let state = db.state::<AppState>()?;
```

This remains a real durable or in-memory CFMD database.

The point is only DX:

> Small application state should not require the developer to think in “tables/entities/database schema” immediately.

Example:

```rust
state.modify(|s| {
    s.settings.volume = 0.8;
    s.recent_projects.push(path);
})?;
```

Internally this should lower to normal CFMD typed changes rather than serializing the entire root value.

---

# 7. Structural `modify()` / Automatic Fine Diff

Allow developers to mutate ordinary Rust values while CFMD derives the minimal semantic structural change.

Example:

```rust
ctx.todos.modify(id, |todo| {
    todo.title = "New title".into();
    todo.settings.theme = Theme::Dark;
    todo.tags.insert("important".into());
    todo.steps.remove(3);
})?;

ctx.commit()?;
```

Conceptually:

```text
Product
 ├─ title: Replace
 ├─ settings.theme: Sum change
 ├─ tags: Set insert
 └─ steps: Seq splice
```

instead of:

```text
replace whole object/blob
```

This is a major embedded DX improvement because it makes CFMD feel almost like normal mutable application state while preserving precise database semantics.

---

# 8. Writable Derived Views

Expose the existing lens/rewrite mathematics as a product capability.

A view should be editable when CFMD can prove a lawful backward rewrite.

Example:

```rust
#[derive(CfmdView)]
struct TodoCard {
    id: Id<Todo>,
    title: String,
    done: bool,
}

let mut card = ctx
    .todos
    .where_(|t| t.id.eq(id))
    .project::<TodoCard>()
    .one()?;

card.title = "Renamed".into();

ctx.save(card)?;
```

CFMD should preserve hidden source fields automatically.

The same idea should extend to:

- projection;
- filtered writable views;
- selected determinant/join cases;
- view-model structs;
- safe write-through derived interfaces.

If write-back is ambiguous, the view is read-only and returns a structured explanation.

---

# 9. View-Model Structs With Safe Write-Back

A particularly useful form of writable views for embedded UI applications.

Full model:

```rust
struct User {
    id: Id<User>,
    name: String,
    password_hash: String,
    internal_flags: Flags,
    avatar: Ref<Image>,
}
```

UI-facing model:

```rust
#[derive(CfmdView)]
struct ProfileForm {
    name: String,
    avatar: Ref<Image>,
}
```

Usage:

```rust
let mut form = ctx.user(id).view::<ProfileForm>()?;
form.name = input;
ctx.save(form)?;
```

This removes a lot of conventional application boilerplate:

```text
load full object
map object -> form DTO
edit DTO
construct patch
avoid overwriting hidden fields
write source object
```

---

# 10. Computed / Derived Properties

Allow exact queries to appear as ordinary domain properties.

Example:

```rust
#[cfmd::derived]
fn full_name(user: User) -> TextExpr {
    user.first_name.concat(" ").concat(user.last_name)
}

#[cfmd::derived]
fn open_tasks(project: Project) -> Query<Task> {
    project.tasks.where_(|t| !t.closed)
}
```

They should be usable in:

```text
filter
projection
ordering
watch
rules
other derived expressions
```

Example:

```rust
ctx.users
    .where_(|u| u.full_name().starts_with("Alex"))
```

Physical realization remains a database concern:

```text
compute on demand
materialize
index
incrementally maintain
```

The developer should not have to choose manually.

Possible advanced extension:

```rust
temperature.fahrenheit().set(72.0)
```

for derived values that have a lawful writable lens.

---

# 11. Stable Semantic Sequences

Expose a first-class ordered collection whose element occurrence identity is stable across insertions/moves.

Example:

```rust
struct Playlist {
    tracks: StableSeq<TrackRef>,
}
```

API:

```rust
playlist.tracks.insert_after(track_a, track_b)?;
playlist.tracks.move_before(track_d, track_a)?;
playlist.tracks.remove(track_c)?;
```

Avoid representing list identity through:

```text
position INTEGER
```

Stable sequence occurrences are useful even in single-threaded embedded applications:

- UI selection survives inserts before it;
- saved anchors/bookmarks remain valid;
- undo/redo targets semantic occurrences;
- persistent references into ordered collections are stable;
- editors do not constantly renumber positions.

---

# 12. Semantic Graph Duplicate

Use CFMD lifecycle/ownership semantics to clone an application-owned object graph automatically.

Example:

```rust
let new_project = ctx.duplicate(project_id)?;
```

Expected behavior:

- compute owned / `KeepsAlive` closure;
- allocate fresh identities;
- rewrite internal references to the cloned identities;
- preserve external references as external;
- preserve lifecycle/relationship constraints.

This is common application functionality that is usually implemented manually.

Useful for:

- projects;
- scenes;
- documents;
- templates;
- configuration trees;
- editor objects.

---

# 13. Semantic Clipboard / Graph Export & Import

Extend graph duplication into a portable package:

```rust
let package = ctx.export_owned(project_id)?;
other_ctx.import(package)?;
```

A package can preserve:

- schema/semantic identity;
- owned graph structure;
- internal identity mapping;
- external references where allowed;
- migration/transport metadata;
- lifecycle semantics.

This creates a generic typed clipboard/file-package primitive rather than requiring every application to invent its own export graph format.

---

# 14. Schema-Aware Copy/Paste Across App Versions

A portable object package created under an older schema can be imported into a newer schema through the same certified schema transport machinery.

Conceptually:

```text
ClipboardPackage<Schema17>
        ↓
17 -> 18 -> 20 -> 21
        ↓
Schema21 object graph
```

Possible result:

```text
Imported exactly
```

or a precise failure:

```text
Cannot transport:
Project.settings.legacy_mode
```

This is particularly useful for editors, project tools and document-oriented embedded applications.

---

# 15. Explicit Live vs Historical References

Expose the distinction between “target must still exist” and “identity may outlive the object” directly in the product type system.

Example:

```rust
owner: Ref<User>
created_by: HistoricalRef<User>
```

Semantics:

```text
Ref<T>
    target must remain live

HistoricalRef<T>
    preserves historical identity after deletion
```

This avoids application patterns such as:

```text
nullable foreign key
raw unvalidated id
ON DELETE SET NULL
special audit tables
```

---

# 16. Native Rust Enum / Sum DX

Rust already has native algebraic `enum`.

CFMD already has the corresponding logical kernel form:

```text
TypeExpr::Sum
Value::Variant
```

Expose this directly through `CfmdValue`.

Example:

```rust
#[derive(CfmdValue)]
#[cfmd(key = "app.task-status")]
enum TaskStatus {
    #[cfmd(key = "draft")]
    Draft,

    #[cfmd(key = "running")]
    Running {
        started_at: Timestamp,
    },

    #[cfmd(key = "failed")]
    Failed {
        message: String,
        retryable: bool,
    },

    #[cfmd(key = "done")]
    Done,
}
```

Then:

```rust
#[derive(CfmdEntity)]
struct Task {
    #[cfmd(id)]
    id: Id<Task>,

    status: TaskStatus,
}
```

Variant identity must be stable semantic identity, not the Rust declaration ordinal.

---

# 17. First-Class Enum Query / Pattern Surface

Allow expressive query construction over enum variants and payloads.

Possible API directions:

```rust
ctx.tasks.where_(|t| t.status.is::<TaskStatus::Running>())
```

or:

```rust
ctx.tasks.where_(|t| {
    t.status
        .as_running()
        .map(|r| r.started_at.lt(deadline))
})
```

Goals:

- variant tests;
- variant set membership;
- payload projection;
- exhaustive matching where possible;
- query planner visibility;
- no opaque Rust callbacks.

---

# 18. Packed Fieldless Enum Physical Pages

For payload-free enums, specialize the existing logical `Sum` into an extremely compact physical representation.

Example:

```rust
enum State {
    Idle,
    Running,
    Paused,
    Done,
}
```

Physical realization:

```text
semantic variants
    ↓
layout-local compact tags
    ↓
packed tag stream
```

Possible packing:

```text
2 variants     -> 1 bit / row
3–4 variants   -> 2 bits / row
5–16 variants  -> 4 bits / row
17–256 variants -> 8 bits / row
```

Important rule:

```text
Rust enum ordinal != semantic identity
```

Instead:

```text
stable semantic variant ID
        ↓
revision/layout-local tag
```

The local tag may be rebuilt during reopen/compaction without changing logical meaning.

---

# 19. Specialized Enum Predicate Microkernels

A packed enum representation enables very cheap operations:

```text
EnumEq
EnumNotEq
EnumIn
EnumNotIn
```

Example:

```rust
ctx.tiles.where_(|t| {
    t.kind.in_([
        TileType::Grass,
        TileType::Dirt,
    ])
})
```

For small fieldless enums this can lower to packed-mask operations rather than generic structural comparison.

This should remain a physical optimization of logical `Sum`, not a second enum data model.

---

# 20. Payload Enum Column Specialization

For enums with payloads:

```rust
enum Shape {
    Circle { radius: f64 },
    Rect { width: f64, height: f64 },
    Label { text: String },
}
```

use the existing algebraic physical shape:

```text
Tag column
+
Payload ordinal
+
dense payload columns per variant
```

Conceptually:

```text
rows:
Circle
Rect
Circle
Label

Circle payload:
radius
radius

Rect payload:
width | height

Label payload:
text
```

This avoids nullable “one giant record with every possible field” storage.

Future physical specialization can optimize common variant distributions without changing logical `Sum` semantics.

---

# 21. Embedded DX Design Principle

The strongest embedded direction is:

> Let developers start with ordinary application values and domain types while retaining a full database underneath.

A lightweight application should be able to begin with:

```rust
let db = Database::memory::<App>()?;
let state = db.slot::<AppState>("app")?;
```

and later grow into:

```text
entities
relationships
derived properties
writable views
history
watches
durable storage
encryption
migration
```

without changing persistence architecture.

CFMD should therefore compete on total application-state DX, not merely on SQL/database features.

---

# 22. Suggested Priority

## Tier A — Highest Embedded Value

1. `Storage::Memory`
2. Memory -> `.cfmd` promotion / Save As
3. `CfmdValue` for structs/enums/collections
4. Persistent `slot<T>` / state facade
5. Structural `modify()` / fine diff

## Tier B — Strong Everyday DX

6. Writable views / view-model write-back
7. Computed properties
8. Stable semantic sequences
9. Live vs Historical references
10. Native enum query surface

## Tier C — High-Leverage Project/Editor Features

11. Graph duplicate
12. Semantic clipboard/export/import
13. Cross-schema copy/paste
14. Packed enum pages + enum predicate microkernels
15. Optional secure-memory profile

---

# 23. Product Positioning

The goal is not:

> “CFMD is a more complicated SQLite.”

A stronger embedded proposition is:

> **CFMD is a typed application-state database that can begin as simply as an in-memory value store, grow into a durable single-file database, and keep the same exact domain model throughout.**

The desired adoption path is:

```text
RAM-only state
    ↓
typed structural values
    ↓
single-file persistence
    ↓
relationships / derived state / watches
    ↓
history / migrations / advanced semantics
```

without forcing the project to replace its persistence layer as complexity grows.
