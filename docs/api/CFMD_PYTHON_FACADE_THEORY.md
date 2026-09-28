# CFMD Python Facade Theory
## Python-first application facade: deep queries, exact watch, speculative state, history and local tooling

**Status:** standalone design theory  
**Target:** Python-first facade for embedded/local CFMD applications  
**Repository changes:** none

---

# 1. Thesis

CFMD should **not** try to be unique by replacing familiar query words with unusual syntax.

The read side can deliberately be familiar:

```python
users = db.users.where(
    lambda u:
        u.active
        & (u.passport.country.code == "RU")
)

names = users.select(lambda u: u.name).all()
```

The important change is that relationships are navigated **in depth** through the domain schema:

```python
u.passport.country.code
```

rather than forcing the user to think in:

```text
JOIN Passport
JOIN Country
ON ...
```

`JOIN` remains part of the mathematical/planner vocabulary, not the normal application vocabulary.

The CFMD-specific value then appears around the query:

```python
query.all()

async for delta in query.watch():
    ...

old = db.at(revision)
old.run(query)

plan = ...
future = db.preview(plan)

future.run(query)
future.delta(query)
future.why_changed(query)

future.commit()
```

History is also active:

```python
entry = db.history.get(173)
undo = entry.inverse()

future = db.preview(undo)

if future.can_commit:
    future.commit()
else:
    show_conflicts(future.conflicts)
```

The product model is therefore:

> **Query current state. Watch exact changes. Preview future state. Reuse history as validated state transitions.**

---

# 2. Non-negotiable DX laws

## 2.1 Types are types, not databases

Do not make this primary API:

```python
User[17]
```

It suggests that `User` is a container or somehow knows which database contains id 17.

Instead:

```python
db.users
```

means:

```text
EntitySet[User] bound to this concrete database context
```

If generated names are unavailable:

```python
db.entities(User)
```

can be the lower-level equivalent.

---

## 2.2 No hidden I/O on materialized Python objects

This must not silently execute database requests:

```python
user = db.users.get(17)
print(user.passport.country.name)
```

A materialized `User` is ordinary Python data.

A relationship can be represented as:

```python
user.passport
# Ref[Passport]
```

and loading it remains explicit:

```python
passport = db.get(user.passport)
```

or:

```python
passport = user.passport.load(db)
```

The key rule is simple:

> ordinary Python attribute access never unexpectedly performs storage I/O.

This prevents the classic ORM N+1 trap.

---

## 2.3 Deep traversal is allowed inside query construction

This is different:

```python
db.users.where(
    lambda u: u.passport.country.code == "RU"
)
```

Here `u` is a symbolic query proxy.

`u.passport.country.code` is a schema path. CFMD sees the whole expression and compiles it as one logical plan.

So the facade can remove explicit joins **without** hiding I/O.

---

## 2.4 Many-valued paths must not silently flatten

Suppose:

```text
User.projects : Many[Project]
Project.tasks : Many[Task]
```

This is ambiguous and should be rejected:

```python
u.projects.tasks.done == False
```

Does it mean any? all? flatten? duplicate-sensitive existential?

Require the semantic choice:

```python
u.projects.any(
    lambda p:
        p.tasks.any(lambda t: ~t.done)
)
```

or an aggregate:

```python
u.projects.match(id=project_id).attachments.count()
```

Conciseness is good only while the meaning stays explicit.

---

## 2.5 Current, historical and speculative state are explicit worlds

```python
db
```

means current committed world.

```python
db.at(revision)
```

means a historical world.

```python
db.preview(plan)
```

means a proposed future world.

The receiving context tells the programmer which world they are operating on.

---

# 3. Core public concepts

The facade should teach a small vocabulary:

```text
Database / ReadContext
EntitySet[T]
Query[T]
ScalarQuery[T]
Snapshot[T]
Ref[T]
Plan / Rewrite
Candidate
Revision
HistoryEntry
Watch
```

Mental model:

```text
db.users
    ↓
EntitySet[User]
    ↓ where / match / select
Query[T]
    ↓ all / one / value
Python value

Query[T]
    ↓ watch
exact async result deltas

Plan
    ↓ preview
Candidate
    ↓ run/delta/why_changed
speculative future database

HistoryEntry
    ↓ inverse
Plan
```

---

# 4. Entity sets

Canonical root:

```python
db.users
db.messages
db.projects
db.attachments
```

Conceptually:

```text
EntitySet[User]
EntitySet[Message]
EntitySet[Project]
EntitySet[Attachment]
```

The database context is explicit before any query begins.

That is more important than inventing a novel query operator.

---

# 5. Identity lookup, equality matching and arbitrary predicates

These are different operations and should remain different.

## 5.1 Identity

```python
user = db.users.get(user_id)
```

Suggested semantics:

```python
db.users.get(id)       # User | None
db.users.require(id)   # User, raises EntityNotFound
```

`get()` is only for declared entity identity.

---

## 5.2 Equality matching

Common filters should be easy:

```python
db.users.match(
    username="Artem",
    active=True,
)
```

Meaning:

```text
username == "Artem"
AND active == True
```

using declared CFMD semantics for each field.

Relationships can participate:

```python
db.users.match(passport=passport_ref)
```

and, if a materialized persisted entity retains identity:

```python
db.users.match(passport=passport)
```

An unpersisted `Passport(...)` should not silently be interpreted as identity.

---

## 5.3 Arbitrary predicate

```python
db.users.where(
    lambda u:
        (u.age >= 18)
        & u.active
        & (u.passport.country.code == "RU")
)
```

This builds query IR.

### Python boolean caveat

A proxy object cannot reliably overload Python `and/or/not`.

Therefore the robust core syntax should use overloadable operators:

```python
&
|
~
```

Example:

```python
lambda u: (u.age >= 18) & (u.status == Status.ACTIVE)
```

CFMD may later add optional AST-capture sugar for natural `and/or/not`, but the public contract should not depend on source inspection working in every REPL/package/generated function.

This is a real Python DX constraint and should be handled explicitly rather than hidden.

---

# 6. Deep navigation instead of joins

Schema:

```text
User.passport -> Passport
Passport.country -> Country
Country.code -> str
```

Application:

```python
russian_users = db.users.where(
    lambda u: u.passport.country.code == "RU"
)
```

The user expresses the domain relationship.

Planner chooses physical implementation:

```text
index lookup
direct relation map
hash join
maintained relation
semantic index
...
```

Changing the physical plan should not change application code.

This is the correct sense in which CFMD should “remove joins”.

It removes **manual relational plumbing**, not the mathematical relation operation.

---

# 7. Many relationships

Useful symbolic collection operations:

```text
any
all
none
match
where
contains
count
exists
sum
min
max
```

Examples:

```python
db.users.where(
    lambda u:
        u.projects.any(
            lambda p:
                p.tasks.any(lambda t: ~t.done)
        )
)
```

Specific nested entity:

```python
attachment_count = (
    db.users
      .match(id=user_id)
      .select(
          lambda u:
              u.projects
               .match(id=project_id)
               .attachments
               .count()
      )
)
```

Important: matching a declared unique identity can narrow cardinality; matching a non-unique field cannot.

Typing should reflect this where practical.

---

# 8. Projection and shaping

Simple projection:

```python
names = (
    db.users
      .where(lambda u: u.active)
      .select(lambda u: u.name)
)
```

Deep projection:

```python
countries = (
    db.users
      .select(lambda u: u.passport.country.name)
)
```

Structured result:

```python
@dataclass(frozen=True)
class UserRow:
    id: UserId
    name: str
    country: str
    project_count: int

rows = db.users.select(
    lambda u: UserRow(
        id=u.id,
        name=u.name,
        country=u.passport.country.name,
        project_count=u.projects.count(),
    )
)
```

If constructor capture proves brittle, `shape(...)` can be added, but the objective is a statically understandable Python return type.

---

# 9. Laziness rule

A useful DX law:

> operations that change the **shape of the query** stay lazy; operations that explicitly request a **value** execute.

Lazy:

```text
match
where
select
order_by
take
count
exists
sum
```

Execution:

```text
all
one
one_or_none
first
first_or_none
value
```

This is especially important for reactive aggregates.

Prefer:

```python
unread_count = db.messages.where(...).count()
# ScalarQuery[int]

current = unread_count.value()
```

Then naturally:

```python
async for change in unread_count.watch():
    ...
```

If `.count()` immediately returned `int`, the reactive scalar use case would become awkward.

---

# 10. Materialization semantics

Recommended terminals:

```python
q.all()             # list[T]
q.first()           # T, error if empty
q.first_or_none()   # T | None
q.one()             # T, error unless exactly one
q.one_or_none()     # T | None, error if >1
```

Avoid silent arbitrary row selection.

Once materialized, values are ordinary Python values with no hidden DB behavior.

---

# 11. Query as an application object

A query can be named and reused:

```python
open_tasks = db.tasks.where(lambda t: ~t.done)
```

Current state:

```python
open_tasks.all()
```

Historical:

```python
old = db.at(old_revision)
old.run(open_tasks)
```

Future:

```python
future = db.preview(plan)

future.run(open_tasks)
future.delta(open_tasks)
```

Diagnostics:

```python
open_tasks.explain()
open_tasks.dependencies()
open_tasks.capabilities()
```

Possible capabilities:

```text
executable
historical
exact_watch
candidate_delta
writable
ordered_delta
```

The system should report unsupported capabilities explicitly instead of silently replacing them with weaker behavior.

---

# 12. Watch: observe an expression result

This is the central reactive concept.

```python
q = db.tasks.where(lambda t: ~t.done)
```

Subscription:

```python
async with q.watch() as changes:
    async for delta in changes:
        apply(delta)
```

A shorter:

```python
async for delta in q.watch():
    ...
```

can exist if cleanup remains deterministic.

The user is **not** subscribing to “Task table changed”.

The user is subscribing to:

```text
the result of q
```

---

# 13. Exact delta semantics

For entity collections:

```python
QueryDelta(
    revision=...,
    transaction=...,
    added=[...],
    removed=[...],
    updated=[...],
    moved=[...],
)
```

For scalar queries:

```python
ValueChanged(
    old=5,
    new=6,
    revision=...,
    transaction=...,
)
```

One committed transaction should normally yield one atomic watch batch for a query.

The subscriber must not observe half a transaction.

---

# 14. Attachment-count example

This should work naturally:

```python
attachment_count = (
    db.users
      .match(id=user_id)
      .select(
          lambda u:
              u.projects
               .match(id=project_id)
               .attachments
               .count()
      )
)
```

Read once:

```python
count = attachment_count.value()
```

Watch exact changes:

```python
async for change in attachment_count.watch():
    print(change.old, "->", change.new)
```

If the current count is 5:

```text
rename Attachment #8
    count remains 5
    -> no event

add Attachment #9
    5 -> 6
    -> ValueChanged(5, 6)

remove Attachment #3
    6 -> 5
    -> ValueChanged(6, 5)
```

This is stronger than a table-level observer.

---

# 15. Deep dependency tracking

Query:

```python
q = db.users.where(
    lambda u: u.passport.country.code == "RU"
)
```

Changes that may affect it include:

```text
User.passport changed
Passport.country changed
Country.code changed
matching User inserted/deleted
```

CFMD derives this dependency graph from the query.

Application code does not manually subscribe to Users, Passports and Countries.

---

# 16. Scalar value stream

For GUI code the full metadata may be unnecessary.

Offer:

```python
async for value in attachment_count.values():
    label.setText(str(value))
```

`values()` is convenience over `watch()`.

Full provenance remains:

```python
async for change in attachment_count.watch():
    print(change.transaction)
    print(change.revision)
```

---

# 17. Resume and revision continuity

External tools and GUIs can disconnect.

Support:

```python
q.watch(since=last_revision)
```

If retained history permits exact replay, CFMD catches the subscriber up.

If not:

```text
ResetRequired
```

should be explicit.

Never fake continuity.

---

# 18. Backpressure

A GUI may be slower than database changes.

Watch needs explicit policies, for example:

```python
q.watch(buffer=256)
q.watch(coalesce="latest")
q.watch(coalesce="transaction")
```

Correctness-oriented defaults should not silently drop semantically relevant transitions.

UI adapters may choose safe display-oriented coalescing.

---

# 19. No implicit polling

If a query has no exact incremental derivative, an exact watch should fail with a useful capability reason.

Example:

```text
WatchUnsupported:
operator X is executable but not exact-watchable
```

Optional escape hatch:

```python
q.watch(mode="recompute")
```

The user then explicitly chooses full recomputation.

CFMD should never market polling as exact reactive maintenance.

---

# 20. PyQt / PySide

Core remains framework-neutral:

```python
async for delta in query.watch():
    ...
```

A Qt adapter can handle event-loop and model details:

```python
model = cfmd.qt.list_model(query)
view.setModel(model)
```

Mapping:

```text
Added   -> beginInsertRows
Removed -> beginRemoveRows
Updated -> dataChanged
Moved   -> beginMoveRows
```

Scalar:

```python
binding = cfmd.qt.bind_text(
    unread_label,
    unread_count,
)
```

The adapter is convenience; the underlying watch semantics remain language-neutral.

---

# 21. WPF / .NET continuation

The same protocol maps naturally to:

```csharp
await foreach (var delta in query.WatchAsync())
{
    ...
}
```

and adapters for:

```csharp
ObservableCollection<T>
```

Therefore the important kernel/runtime decision is not “support Qt”.

It is:

> make exact query subscription a first-class, language-neutral protocol over the revision stream.

---

# 22. Writes should be plans before they are commits

For advanced workflows, do not make assignment to live objects the primary write model.

Use an explicit plan:

```python
plan = db.plan(EditMessage(message_id))

plan.update(
    db.messages.match(id=message_id),
    text="new text",
)
```

Nothing has committed.

Additional operations:

```python
plan.insert(db.attachments, attachment)
plan.delete(db.messages.match(id=old_message_id))
plan.update(...)
```

A Plan is an inspectable proposed transition.

---

# 23. Typed operation metadata

Domain operation:

```python
@cfmd.operation
@dataclass(frozen=True)
class EditMessage:
    message_id: MessageId
```

Use:

```python
plan = db.plan(EditMessage(message_id))
plan.update(
    db.messages.match(id=message_id),
    text=new_text,
)
```

History can later expose:

```python
db.history.of(EditMessage)
```

This gives useful event-sourcing-like metadata without requiring the entire application to be event-sourced.

---

# 24. Preview creates a Candidate world

This is a core CFMD idea:

```python
future = db.preview(plan)
```

Do **not** conceptualize `future` as merely a report.

It represents:

```text
current/base revision
+
proposed rewrite
=
candidate future state
```

and is queryable.

```python
future.messages.match(id=message_id).one()
future.run(current_chat_messages)
future.run(unread_count)
```

The programmer asks:

> if I commit this, what would the database say?

---

# 25. Exact future impact

Candidate can compare query results:

```python
future.delta(current_chat_messages)
future.delta(unread_count)
future.delta(search_results)
```

Example:

```text
UnreadCount
    5 -> 4

CurrentChatMessages
    Updated Message #173

SearchResults
    Message #173 removed from 2 active results
```

This is more useful than a generic row-diff preview because the application can ask about exactly the derived state it cares about.

---

# 26. Explain future changes

```python
future.why_changed(unread_count)
```

Possible result:

```text
UnreadCount changed 5 -> 4

because:
    Message #173.read changed False -> True

query path:
    message belongs to current chat
    AND message.read == False no longer holds

source operation:
    MarkMessageRead(message_id=173)
```

This is where CFMD mathematical dependency/change information becomes practical developer tooling.

---

# 27. Candidate freshness and TOCTOU

A Candidate is based on a revision.

If:

```text
candidate base = revision 100
```

and another client commits revision 101 before commit, CFMD must not blindly publish stale assumptions.

Default:

```text
head unchanged
    -> atomic commit

head changed
    -> StaleCandidate / RebaseRequired
```

Then:

```python
future = future.rebase()
```

may attempt to re-evaluate on the new head.

Automatic rebase should occur only if proven safe, never as invisible guesswork.

---

# 28. History and semantic undo

History entry:

```python
entry = db.history.get(173)
```

Construct inverse:

```python
undo = entry.inverse()
```

This produces a Plan/Rewrite, not an immediate mutation.

Then:

```python
future = db.preview(undo)
```

The same preview machinery applies to undo as to ordinary edits.

---

# 29. Undo conflict reasoning should be finer than “same row”

Transaction 173:

```text
Project.name:
"A" -> "B"
```

Later 180:

```text
Project.icon:
a.png -> b.png
```

Undo 173 should normally be safe even though both touch the same entity.

CFMD can reason about semantic independence/commutation.

But if 180 is:

```text
Project.name:
"B" -> "C"
```

then inverse #173 may conflict.

Useful explanation:

```text
CONFLICT

inverse #173 expects the semantic value produced by #173
current Project.name was subsequently changed by #180

cannot preserve #180 while applying the original inverse unchanged
```

This is much stronger than a naive undo stack.

---

# 30. Partial and dependency-aware undo

Transaction 173:

```text
created Task A
created Task B
```

Later 181:

```text
edited Task B
```

Naive inverse would delete both and destroy 181.

Potential CFMD result:

```text
Undo #173 is not independent of #181.

Options:
1. abort
2. undo independent part only
      delete Task A
3. cascade
      inverse #181
      inverse #173
```

The exact strength depends on the final rewrite/repair calculus, but the public facade should leave room for it.

---

# 31. Drafts for forms

UI code often needs local mutability.

Do not turn persisted objects into live ORM objects.

Instead:

```python
user = db.users.require(user_id)
draft = cfmd.draft(user)

draft.name = name_field.text()
draft.email = email_field.text()

plan = draft.plan(EditUser(user_id))
future = db.preview(plan)
```

Draft mutation is plain Python.

No assignment performs database I/O.

This is a clean fit for editors, settings panels and forms.

---

# 32. Candidate-backed form validation

The same form can validate against future DB state:

```python
future = db.preview(draft.plan(EditUser(user_id)))

conflicts = future.run(username_conflicts)
valid = future.run(account_validity)
```

The UI no longer needs a second, hand-maintained imitation of database constraints.

Committed and tentative states use the same logic.

---

# 33. Embedded chat scenario

Imagine local chat storage.

Query backing the open chat:

```python
current_messages = (
    db.messages
      .match(chat_id=current_chat_id)
      .order_by(lambda m: m.created_at)
)
```

UI:

```python
async for delta in current_messages.watch():
    ui.apply(delta)
```

Unread count:

```python
unread_count = (
    db.messages
      .where(
          lambda m:
              (m.chat_id == current_chat_id)
              & (~m.read)
      )
      .count()
)
```

UI:

```python
async for count in unread_count.values():
    badge.setText(str(count))
```

This requires no hand-written `MessageChanged`, `UnreadChanged`, `AttachmentChanged` event bus.

The queries themselves define what each subsystem observes.

---

# 34. External live editing

Suppose a CFMD Studio is connected while the chat application runs.

Studio edits:

```text
Message #173.text:
"hello" -> "edited externally"
```

Studio does not mutate file bytes.

It submits a Plan to the same authoritative runtime.

Runtime:

```text
Plan
 -> validate
 -> Candidate
 -> commit
 -> Revision 901
 -> exact query derivatives
 -> subscribers
```

The chat's `current_messages.watch()` receives an Updated delta and changes that UI row immediately.

If Studio sets:

```text
Message #173.read:
False -> True
```

then `unread_count.watch()` emits:

```text
5 -> 4
```

This is the intended live-tooling behavior.

---

# 35. One authoritative runtime

Wrong:

```text
chat.exe   -> opens and writes chat.cfmd
studio.exe -> independently opens and writes chat.cfmd
```

This risks:

```text
writer races
stale caches
bypassed invariants
stale maintained state
broken watch streams
revision ambiguity
corruption
```

Correct:

```text
             +----------------------+
             | authoritative CFMD   |
             | runtime              |
             +----------+-----------+
                        |
          +-------------+-------------+
          |                           |
      chat app                    CFMD Studio
      watch/query                 query/plan/preview
```

Modes:

```text
A. in-process embedded runtime
B. app-owned runtime + local tooling endpoint
C. standalone local CFMD service
```

For pet projects A is default.
For live development tooling B is especially attractive.

---

# 36. Local transport

Development endpoint can use:

```text
Windows:
    Named Pipe

Linux/macOS:
    Unix Domain Socket

Optional:
    localhost TCP
```

Example conceptual setup:

```python
db = cfmd.open(
    "chat.cfmd",
    tooling=True,
)
```

Then:

```bash
cfmd studio
```

discovers/attaches to the application-owned runtime.

The exact transport is implementation detail; semantic protocol matters more.

---

# 37. Tooling security

Localhost is not automatically trusted.

Tooling endpoint should be:

```text
opt-in
per-user where possible
capability-token protected
read-only or read-write explicitly
disabled in production by default
```

Potential API:

```python
db.expose_tools(
    mode="development",
    write=True,
)
```

No external editor should bypass the revision/validation pipeline.

---

# 38. Language-neutral tooling protocol

The runtime protocol should support:

```text
schema introspection
query execution
watch registration
watch resume/cancel
revision access
history access
plan submission
candidate creation
candidate query
candidate delta
candidate explanation
candidate commit
```

This allows the same foundation for:

```text
Python
PyQt
.NET/WPF
Rust tools
CFMD Studio
CLI
plugins
```

---

# 39. CFMD Studio

Studio should be more than a table editor.

Useful areas:

```text
Schema
Entities
Relations
Queries
Revisions
History
Plans/Candidates
Active Watches
Invariant Violations
```

Editing workflow:

```text
1. user edits Message #173.text
2. Studio builds Plan
3. Studio previews Candidate
4. Studio shows direct and derived impact
5. user commits
6. runtime publishes Revision
7. application watch streams update
```

Preview can show:

```text
Direct changes:
    Message #173.text

Derived query effects:
    CurrentChatMessages:
        Updated #173

    SearchResults:
        changed

    UnreadCount:
        unchanged

Active watch subscribers affected:
    2

Invariant status:
    valid
```

This turns the DB into a live state debugger.

---

# 40. Active-watch inspector

Studio can show:

```text
Watch #12

query:
    messages where chat_id == 51
    order by created_at

subscriber:
    chat.exe / CurrentChatModel

last revision:
    1832

result size:
    138
```

Scalar example:

```text
Watch #18

query:
    unread message count for chat 51

current value:
    5
```

This is far more diagnosable than invisible application event wiring.

---

# 41. Query explainability

```python
query.explain()
```

Logical level:

```text
Messages
filter chat_id == 51
traverse ...
order by created_at
project ...
```

Physical level:

```text
semantic index used
maintained ordered state
exact derivative available
```

The user sees domain paths, not forced SQL vocabulary.

---

# 42. Semantic types / Γ

CFMD can eventually make semantic equality/order/canonicalization consistent across the entire system.

Conceptual declaration:

```python
Username = cfmd.semantic(
    str,
    canonicalize=unicode_casefold,
)
```

Then the same semantics power:

```text
match
equality
uniqueness
distinct
grouping
relationship matching
semantic indexes
watch invalidation
```

Normal users should not need Γ notation.

Advanced tooling can expose:

```python
db.semantics.explain(User.username)
```

The value is consistency of meaning, not mathematical terminology in ordinary app code.

---

# 43. Querying semantic/schema futures

Long-term, Candidate should not assume plans only mutate row values.

Conceptually:

```python
plan = change_username_semantics(...)
future = db.preview(plan)

future.delta(canonical_users)
future.delta(username_conflicts)
```

This is not required for Python v1, but the facade should not hard-code “plan = row patch”.

CFMD revisions are richer than that.

---

# 44. Watch is a runtime protocol, not a Python callback trick

Do not implement Python `watch()` merely as:

```text
commit callback
 -> rerun Python query
 -> compare lists
```

The kernel/runtime contract should conceptually be:

```text
register logical query
obtain subscription id
record starting revision
on commit derive ΔQ
emit revision-tagged exact delta
resume/cancel by subscription state
```

That decision is what makes WPF, Studio and other processes possible later.

---

# 45. Candidate is also a runtime protocol

Preview should have first-class runtime operations:

```text
create candidate(base_revision, plan)
query candidate
compute candidate query delta
explain candidate impact
validate candidate
rebase candidate
commit candidate
discard candidate
```

Then Python and Studio see the same behavior.

---

# 46. History is user-facing, not only WAL

A history record should have stable logical identity:

```text
transaction id
base revision
result revision
typed operation metadata
logical change/rewrite
optional actor/client metadata
```

User-facing operations:

```text
filter history
fetch transaction
build inverse
preview inverse
explain conflict
```

WAL/durability internals can remain separate.

---

# 47. Simple CRUD must remain simple

Advanced capability must not make a tiny script unpleasant.

Convenience may exist:

```python
db.insert(User(...))
db.delete(user_ref)
```

and perform immediate one-operation commits.

But these should be documented as sugar over Plan + commit.

When the app needs preview/history metadata:

```python
plan = db.plan(CreateUser(...))
plan.insert(db.users, user)

future = db.preview(plan)
future.commit()
```

Beginner simplicity and advanced correctness do not need separate database models.

---

# 48. Error quality is part of the facade

Use domain errors:

```text
EntityNotFound
ExpectedOneButFoundMany
UnboundEntityReference
AmbiguousCollectionTraversal
WatchUnsupported
RevisionUnavailable
StaleCandidate
InvariantViolation
SemanticConflict
PermissionDenied
```

Example good error:

```text
AmbiguousCollectionTraversal

Expression:
    user.projects.tasks.done

`projects` and `tasks` are many-valued paths.

Choose an explicit meaning:
    user.projects.any(...)
    user.projects.all(...)
    user.projects.match(...).tasks.count()
```

This matters enormously for DX.

---

# 49. Typing and IDE requirements

Python-first should target:

```text
Pyright / Pylance
good autocomplete
generated schema stubs
mypy where practical
```

Expected type flow:

```python
db.users
# EntitySet[User]

db.users.match(active=True)
# Query[User]

db.users.select(lambda u: u.name)
# Query[str]

db.messages.where(...).count()
# ScalarQuery[int]

...value()
# int

...watch()
# Watch[ValueChanged[int]]
```

Relationship cardinality should be reflected in symbolic expression types where practical.

---

# 50. Optional relations

If:

```text
User.passport : Optional[Passport]
```

then a deep predicate such as:

```python
u.passport.country.code == "RU"
```

should have defined null propagation.

Recommended semantics:

```text
absent optional path makes a positive comparison false
```

Explicit tests:

```python
u.passport.is_none()
u.passport.is_some()
```

Projection through an optional path should become optional rather than raising surprise runtime errors.

---

# 51. Equality against persisted entities

This should be natural:

```python
passport = db.passports.require(passport_id)

db.users.where(lambda u: u.passport == passport)
```

or:

```python
db.users.match(passport=passport)
```

because `passport` carries database identity.

This should fail:

```python
passport = Passport(...)
db.users.match(passport=passport)
```

if that object has no bound identity.

CFMD should never guess whether the user meant entity identity or structural equality.

---

# 52. Performance transparency

Queries should be inspectable:

```python
query.explain()
query.dependencies()
query.capabilities()
```

Watches should be inspectable:

```python
watch.explain()
```

A facade that looks object-oriented must still let developers see costs.

No “property access therefore cheap” illusion.

---

# 53. Familiar query syntax is not a weakness

Yes, the read surface is LINQ-like:

```python
db.users.where(...).select(...)
```

That is acceptable.

The originality should not be:

```text
invent strange replacement for where()
```

The originality should be:

```text
familiar lazy query
+
deep domain traversal
+
no manual join plumbing
+
exact query watch
+
historical context
+
queryable future Candidate
+
future query delta
+
semantic inverse/undo
+
live multi-client Studio
```

That composition is the real facade.

---

# 54. Explicit relational algebra remains an escape hatch

Some advanced query may genuinely need an ad-hoc relation not declared by schema.

A lower-level API can support it.

For example under an explicit namespace:

```python
cfmd.relational(...)
```

or equivalent.

`join` may exist there.

It should not dominate ordinary application docs.

---

# 55. Pet-project readiness

Minimum credible Python pet-project milestone:

1. installable Python wheels;
2. open/create durable local DB;
3. typed schema;
4. ids and relationships;
5. `get/require`;
6. `match/where/select`;
7. ordering/limits;
8. basic aggregates;
9. deep relation traversal;
10. atomic commits;
11. useful domain errors;
12. basic exact watch subset;
13. Plan + preview;
14. revision/history access;
15. inspector/CLI;
16. complete local-app tutorial.

Without these, CFMD remains a powerful kernel rather than a practical application database.

---

# 56. Compelling pet-project milestone

To make CFMD actively attractive rather than merely usable:

1. exact watch for most normal app queries;
2. PyQt/PySide adapter;
3. typed operation metadata;
4. Candidate query/delta;
5. history inverse/undo preview;
6. local tooling endpoint;
7. live CFMD Studio;
8. active watch inspector;
9. explainability;
10. predictable schema migration workflow.

At that point the pitch becomes:

> build reactive local applications without implementing a second event, undo and preview architecture beside the database.

---

# 57. Production continuation

Production readiness later requires:

```text
crash recovery guarantees
backup/restore
corruption detection
schema migration contracts
stable compatibility policy
resource limits
watch memory bounds
observability
security
multi-process ownership rules
benchmarking
upgrade/downgrade story
```

These should not pollute beginner DX, but the facade must leave room for them.

---

# 58. Suggested implementation layering

Do not expose Rust internals one-to-one.

Use:

```text
Python public facade
    |
    | EntitySet / Query / Plan / Candidate / Watch
    v
Python binding bridge
    |
    | compact typed IR and runtime operations
    v
Rust facade/runtime service
    |
    | query compile / revision contexts / watch protocol / rewrites
    v
CFMD kernel
```

The Python API should be designed from user semantics, not crate boundaries.

---

# 59. Query IR boundary

Python should construct a compact typed IR such as:

```text
Field
RelationPath
Compare
And
Or
Not
Any
All
Aggregate
Projection
Order
Limit
```

Rust resolves and plans it under schema/Γ.

Benefits:

```text
stable explainability
dependency extraction
watch compilation
alternative language bindings
less Python-side logic
```

---

# 60. Recommended v1 facade

```python
# identity
user = db.users.get(user_id)

# equality filter
active = db.users.match(active=True)

# arbitrary deep filter
ru_users = db.users.where(
    lambda u:
        u.active
        & (u.passport.country.code == "RU")
)

# projection
names = ru_users.select(lambda u: u.name)

# deep reactive aggregate
attachment_count = (
    db.users
      .match(id=user_id)
      .select(
          lambda u:
              u.projects
               .match(id=project_id)
               .attachments
               .count()
      )
)

current = attachment_count.value()

async for delta in attachment_count.watch():
    ...

# plan
plan = db.plan(EditMessage(message_id))
plan.update(
    db.messages.match(id=message_id),
    text="new text",
)

# speculative future
future = db.preview(plan)

future.messages.match(id=message_id).one()
future.delta(current_messages)
future.why_changed(current_messages)

future.commit()

# history
undo = db.history.get(173).inverse()
future = db.preview(undo)
```

This is the current recommended theory.

---

# 61. Open design questions

These should be prototyped, not decided by aesthetics alone.

1. Should `.count()` always return `ScalarQuery[int]` and `.value()` execute?
2. Is `match` the best name, given Python structural pattern matching?
3. Should identity `match(id=...)` narrow cardinality statically?
4. What is the cleanest Python boolean-expression syntax without fragile AST capture?
5. How should optional relationship projections type-check?
6. Should runtime snapshots expose only `Ref[T]`, only ids, or both?
7. Which query subset gets a hard exact-watch guarantee in v1?
8. How are ordered deltas represented under unstable/tied ordering?
9. What backpressure policy is default for UI watches?
10. How long are revision deltas retained for `watch(since=...)`?
11. How strong can semantic commutation/conflict checking be in undo v1?
12. Can Candidate be safely materialized/persisted, or should it be ephemeral?
13. What does `rebase()` return when the proposed write must be transformed?
14. What is the minimum language-neutral watch protocol?
15. Should tooling be an app-owned endpoint, sidecar, or both?
16. How should local Studio authenticate?
17. Can `why_changed` reuse proof/dependency objects from exact maintenance?
18. Which Γ/schema changes can eventually be previewed?
19. What is the clean boundary between immediate CRUD sugar and explicit Plans?
20. Which contracts already exist in the current CFMD kernel and which require new facade/runtime primitives?

---

# 62. Final architectural statement

CFMD's Python facade should not be a SQL clone and should not be a mathematically clever DSL that hides what the programmer is operating on.

It should be:

```text
explicit database context
+
familiar lazy query operations
+
deep semantic relationship traversal
+
no hidden object I/O
+
exact async query-result watch
+
first-class write Plans
+
queryable Candidate future state
+
revision/history-aware inverse
+
one authoritative runtime
+
live local tooling over the same protocol
```

The normal read path remains understandable:

```python
query = (
    db.users
      .where(lambda u: u.active)
      .select(lambda u: u.passport.country.name)
)
```

The CFMD-specific experience starts when that same query becomes reusable state:

```python
async for delta in query.watch():
    ...
```

and a proposed write becomes a world you can inspect:

```python
future = db.preview(plan)

future.delta(query)
future.why_changed(query)

future.commit()
```

and external tooling uses exactly the same transition pipeline:

```text
CFMD Studio
    -> Plan
    -> Candidate
    -> Commit
    -> Revision
    -> exact Watch deltas
    -> live application UI
```

That is the facade theory to build around.
