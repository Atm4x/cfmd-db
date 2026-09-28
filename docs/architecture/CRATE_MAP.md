# Workspace Crate Map

Workspace crates: **27**.

This is an internal architecture map. These crates are not individually promised as the stable public application API. The product facade is intentionally designed above this graph.

| crate | internal workspace dependencies |
|---|---|
| `kernel-aggregate` | `kernel-exact` |
| `kernel-auth` | `kernel-semantics` |
| `kernel-change` | `kernel-types` |
| `kernel-deployment` | `kernel-auth`, `kernel-semantics` |
| `kernel-durability` | `kernel-auth`, `kernel-change`, `kernel-lens`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-types` |
| `kernel-exact` | — |
| `kernel-fixpoint` | `kernel-exact`, `kernel-grounded-closure`, `kernel-proof`, `kernel-types` |
| `kernel-grounded-closure` | `kernel-persistent` |
| `kernel-identity` | `kernel-types` |
| `kernel-integration` | `kernel-change`, `kernel-identity`, `kernel-lens`, `kernel-lifecycle`, `kernel-model`, `kernel-plan`, `kernel-proof`, `kernel-query`, `kernel-retention`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `storage-memory` |
| `kernel-lens` | `kernel-change`, `kernel-model`, `kernel-query`, `kernel-schema`, `kernel-semantics`, `kernel-types` |
| `kernel-lifecycle` | `kernel-grounded-closure`, `kernel-identity`, `kernel-types` |
| `kernel-model` | `kernel-identity`, `kernel-lifecycle`, `kernel-schema`, `kernel-types` |
| `kernel-persistent` | — |
| `kernel-plan` | `kernel-aggregate`, `kernel-change`, `kernel-durability`, `kernel-fixpoint`, `kernel-grounded-closure`, `kernel-identity`, `kernel-lens`, `kernel-model`, `kernel-persistent`, `kernel-proof`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantic-index`, `kernel-semantics`, `kernel-transport`, `kernel-types`, `kernel-validation`, `kernel-violation` |
| `kernel-proof` | — |
| `kernel-query` | `kernel-aggregate`, `kernel-change`, `kernel-exact`, `kernel-fixpoint`, `kernel-model`, `kernel-persistent`, `kernel-schema`, `kernel-semantic-index`, `kernel-semantics`, `kernel-types` |
| `kernel-retention` | `kernel-types` |
| `kernel-revision` | `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-validation` |
| `kernel-schema` | `kernel-types` |
| `kernel-semantic-index` | `kernel-persistent`, `kernel-schema`, `kernel-types` |
| `kernel-semantics` | `kernel-grounded-closure`, `kernel-model`, `kernel-persistent`, `kernel-proof`, `kernel-schema`, `kernel-types` |
| `kernel-transport` | `kernel-change`, `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-validation` |
| `kernel-types` | — |
| `kernel-validation` | `kernel-identity`, `kernel-model`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-violation` |
| `kernel-violation` | — |
| `storage-memory` | `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-transport`, `kernel-types`, `kernel-validation` |

## Dependency edges

```text
kernel-aggregate -> kernel-exact
kernel-auth -> kernel-semantics
kernel-change -> kernel-types
kernel-deployment -> kernel-auth, kernel-semantics
kernel-durability -> kernel-auth, kernel-change, kernel-lens, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-types
kernel-fixpoint -> kernel-exact, kernel-grounded-closure, kernel-proof, kernel-types
kernel-grounded-closure -> kernel-persistent
kernel-identity -> kernel-types
kernel-integration -> kernel-change, kernel-identity, kernel-lens, kernel-lifecycle, kernel-model, kernel-plan, kernel-proof, kernel-query, kernel-retention, kernel-schema, kernel-semantics, kernel-types, storage-memory
kernel-lens -> kernel-change, kernel-model, kernel-query, kernel-schema, kernel-semantics, kernel-types
kernel-lifecycle -> kernel-grounded-closure, kernel-identity, kernel-types
kernel-model -> kernel-identity, kernel-lifecycle, kernel-schema, kernel-types
kernel-plan -> kernel-aggregate, kernel-change, kernel-durability, kernel-fixpoint, kernel-grounded-closure, kernel-identity, kernel-lens, kernel-model, kernel-persistent, kernel-proof, kernel-query, kernel-revision, kernel-schema, kernel-semantic-index, kernel-semantics, kernel-transport, kernel-types, kernel-validation, kernel-violation
kernel-query -> kernel-aggregate, kernel-change, kernel-exact, kernel-fixpoint, kernel-model, kernel-persistent, kernel-schema, kernel-semantic-index, kernel-semantics, kernel-types
kernel-retention -> kernel-types
kernel-revision -> kernel-identity, kernel-lifecycle, kernel-model, kernel-schema, kernel-semantics, kernel-types, kernel-validation
kernel-schema -> kernel-types
kernel-semantic-index -> kernel-persistent, kernel-schema, kernel-types
kernel-semantics -> kernel-grounded-closure, kernel-model, kernel-persistent, kernel-proof, kernel-schema, kernel-types
kernel-transport -> kernel-change, kernel-identity, kernel-lifecycle, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-types, kernel-validation
kernel-validation -> kernel-identity, kernel-model, kernel-schema, kernel-semantics, kernel-types, kernel-violation
storage-memory -> kernel-identity, kernel-lifecycle, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-transport, kernel-types, kernel-validation
```

## Status

The Pass280 global kernel hostile/refactor campaign is frozen. This map describes implementation dependencies only; audit status is maintained separately in [`../status/KERNEL_HOSTILE_LEDGER.md`](../status/KERNEL_HOSTILE_LEDGER.md).
