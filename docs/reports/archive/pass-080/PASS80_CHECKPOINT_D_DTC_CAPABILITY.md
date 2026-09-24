# Pass80 checkpoint D — DTC maintained-state contract + capability obligations

This is an intermediate safety checkpoint, not the final Pass80 VERIFIED release.

## Production changes

1. `MaterializedRelPlanState` now owns the pinned `RelDifferentialProgram` for its exact query and semantic context.
2. Build rejects drift between DTC state requirements and concrete maintained state owned by the recursive tree.
3. `CapabilityDef.required_fields` is enforced against actual capability implementations and entity instances.
4. Capability metadata remains an interface obligation; the capability symbol itself is not treated as a nominal field-owner subtype.

## Verification

- debug workspace: 504 passed / 0 failed / 8 ignored
- fmt: PASS
- check all targets: PASS
- clippy `-D warnings`: PASS

Release/overflow/rustdoc are intentionally deferred until final Pass80 source freeze.
