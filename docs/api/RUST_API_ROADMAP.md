# User-Facing Rust API Roadmap

The next major project phase is a stable Rust facade over the now-converged kernel.

## Design rule

Applications should depend on one public crate and should not need to import `kernel-*` crates directly.

Conceptual target:

```rust,ignore
use cfmd::{Database, Result};

fn main() -> Result<()> {
    let db = Database::open("app.cfmd")?;
    // schema/query/transaction API to be designed at the facade layer
    Ok(())
}
```

## Public surface goals

- stable `Database` / open/create/close lifecycle;
- explicit transaction/revision boundary without exposing internal publication machinery;
- typed schema/model construction;
- ergonomic query builder that compiles to the existing exact `RelExpr`/prepared-plan machinery;
- typed rewrite/write API;
- bulk insert/query/change boundaries;
- stable error taxonomy separating validation, semantic, conflict, durability, authentication and unsupported-platform failures;
- diagnostics/introspection that exposes supported facts without leaking mutable internal structures;
- explicit opt-in APIs for advanced Γ/deployment/replication functionality.

## Non-goals for the first facade

- exposing internal `NodeId`, maintained-plan nodes or physical COW ownership;
- making every internal crate semver-stable;
- implementing a complete SQL parser before the native API is usable;
- tying the first public API to Python/FFI constraints.

## Acceptance criteria for API v0.1

1. A small embedded application can create/open a DB, define data, transact and query using only the facade crate.
2. Ordinary users never need to construct internal revision/plan/publication types.
3. Bulk operations avoid per-row facade overhead.
4. Public errors are deterministic and documented.
5. Examples are executable tests.
6. Internal crate refactors can occur without breaking facade callers unless the public contract itself changes.
