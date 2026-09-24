# Workspace Crate Map

Workspace crates: **25**.

This is an internal architecture map. These crates are not individually promised as the future stable public API.

| crate | internal workspace dependencies |
|---|---|
| `kernel-aggregate` | — |
| `kernel-auth` | `kernel-semantics` |
| `kernel-change` | `kernel-types` |
| `kernel-deployment` | `kernel-auth`, `kernel-semantics` |
| `kernel-durability` | `kernel-auth`, `kernel-change`, `kernel-lens`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-types` |
| `kernel-fixpoint` | `kernel-grounded-closure`, `kernel-proof`, `kernel-types` |
| `kernel-grounded-closure` | — |
| `kernel-identity` | `kernel-types` |
| `kernel-integration` | `kernel-change`, `kernel-identity`, `kernel-lens`, `kernel-lifecycle`, `kernel-model`, `kernel-plan`, `kernel-proof`, `kernel-query`, `kernel-retention`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `storage-memory` |
| `kernel-lens` | `kernel-change`, `kernel-model`, `kernel-query`, `kernel-schema`, `kernel-semantics`, `kernel-types` |
| `kernel-lifecycle` | `kernel-grounded-closure`, `kernel-identity`, `kernel-types` |
| `kernel-model` | `kernel-identity`, `kernel-lifecycle`, `kernel-schema`, `kernel-types` |
| `kernel-plan` | `kernel-aggregate`, `kernel-change`, `kernel-durability`, `kernel-fixpoint`, `kernel-grounded-closure`, `kernel-identity`, `kernel-lens`, `kernel-model`, `kernel-proof`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantic-index`, `kernel-semantics`, `kernel-transport`, `kernel-types`, `kernel-validation`, `kernel-violation` |
| `kernel-proof` | — |
| `kernel-query` | `kernel-aggregate`, `kernel-change`, `kernel-fixpoint`, `kernel-model`, `kernel-schema`, `kernel-semantic-index`, `kernel-semantics`, `kernel-types` |
| `kernel-retention` | `kernel-types` |
| `kernel-revision` | `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-validation` |
| `kernel-schema` | `kernel-types` |
| `kernel-semantic-index` | `kernel-schema`, `kernel-types` |
| `kernel-semantics` | `kernel-grounded-closure`, `kernel-model`, `kernel-proof`, `kernel-schema`, `kernel-types` |
| `kernel-transport` | `kernel-change`, `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-validation` |
| `kernel-types` | — |
| `kernel-validation` | `kernel-identity`, `kernel-model`, `kernel-schema`, `kernel-semantics`, `kernel-types`, `kernel-violation` |
| `kernel-violation` | — |
| `storage-memory` | `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-query`, `kernel-revision`, `kernel-schema`, `kernel-semantics`, `kernel-transport`, `kernel-types`, `kernel-validation` |

## Dependency edges

```text
kernel-auth -> kernel-semantics
kernel-change -> kernel-types
kernel-deployment -> kernel-auth, kernel-semantics
kernel-durability -> kernel-auth, kernel-change, kernel-lens, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-types
kernel-fixpoint -> kernel-grounded-closure, kernel-proof, kernel-types
kernel-identity -> kernel-types
kernel-integration -> kernel-change, kernel-identity, kernel-lens, kernel-lifecycle, kernel-model, kernel-plan, kernel-proof, kernel-query, kernel-retention, kernel-schema, kernel-semantics, kernel-types, storage-memory
kernel-lens -> kernel-change, kernel-model, kernel-query, kernel-schema, kernel-semantics, kernel-types
kernel-lifecycle -> kernel-grounded-closure, kernel-identity, kernel-types
kernel-model -> kernel-identity, kernel-lifecycle, kernel-schema, kernel-types
kernel-plan -> kernel-aggregate, kernel-change, kernel-durability, kernel-fixpoint, kernel-grounded-closure, kernel-identity, kernel-lens, kernel-model, kernel-proof, kernel-query, kernel-revision, kernel-schema, kernel-semantic-index, kernel-semantics, kernel-transport, kernel-types, kernel-validation, kernel-violation
kernel-query -> kernel-aggregate, kernel-change, kernel-fixpoint, kernel-model, kernel-schema, kernel-semantic-index, kernel-semantics, kernel-types
kernel-retention -> kernel-types
kernel-revision -> kernel-identity, kernel-lifecycle, kernel-model, kernel-schema, kernel-semantics, kernel-types, kernel-validation
kernel-schema -> kernel-types
kernel-semantic-index -> kernel-schema, kernel-types
kernel-semantics -> kernel-grounded-closure, kernel-model, kernel-proof, kernel-schema, kernel-types
kernel-transport -> kernel-change, kernel-identity, kernel-lifecycle, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-types, kernel-validation
kernel-validation -> kernel-identity, kernel-model, kernel-schema, kernel-semantics, kernel-types, kernel-violation
storage-memory -> kernel-identity, kernel-lifecycle, kernel-model, kernel-query, kernel-revision, kernel-schema, kernel-semantics, kernel-transport, kernel-types, kernel-validation
```
