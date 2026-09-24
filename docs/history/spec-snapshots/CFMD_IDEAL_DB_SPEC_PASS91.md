# CFMD IDEAL DB SPEC — PASS91 ADDENDUM

## Status
Pass91 advances historical #10 and #8 without changing the 13/22 production-closed count. Historical #21/#22 are explicitly deferred to their external R&D owner.

## #10 — semantic implementation package / authentication / deployment boundary — PROD PARTIAL

Γ remains semantic authority. Executable implementation deployment is split into:
- semantic contract identity;
- implementation artifact identity;
- runtime profile identity;
- semantic refinement certificate;
- independent artifact authentication evidence;
- runtime execution policy and explicit authorization.

For a defined contract, execution requires a valid refinement certificate to that contract. For an opaque contract, execution identity is the exact artifact/runtime pair. Authentication never substitutes for refinement, and refinement never substitutes for authentication. Revocation affects executability rather than semantic contract identity.

The production durable builtin reopen/replay path now passes through this boundary. Capability checks are scoped to the contracts required by the current historical operation and do not fail because an unrelated package is unavailable.

### Deliberate non-claims
Pass91 does not provide a general external executable plugin stack. Still open:
- external package/CAS storage and download;
- cryptographic executable-byte digesting as a deployment service;
- signatures, trust roots and key rotation;
- native/WASM sandbox and ABI;
- external refinement-proof format/checker integration;
- cross-platform artifact distribution and operational revocation service.

`ImplementationArtifactDigest` for builtins is therefore a stable descriptor identity, not a claim that arbitrary executable bytes were cryptographically authenticated.

## #8 — durable causal effect ledger / REIC DAG lifecycle — PROD PARTIAL / ADVANCED

Production already has independent durable `RevisionEffectId`, causal prerequisites, self-contained exact intent payloads, recovered REIC ideals/frontiers and multi-parent resolution with exact prerequisite cuts. Pass91 adds explicit `DurableEffectKind` and conservative `OpaqueNonConfluent` coordination classification.

No durable effect is coordination-free merely because it is a Rewrite or because peers share Γ. Until a durable confluence/coherence certificate exists, independent effects require an explicit ordering/resolution authority.

Remaining gap: the store still owns one mutable publication head and has no durable ingestion/retention lifecycle for independently advancing branch heads. This is now the direct boundary with historical #16 replication/consensus.

## #21/#22 ownership
No Group/TopK production source was modified. The user-supplied HIST21/22 R&D package was reviewed only to avoid overlapping work; integration is deferred until that R&D branch converges.
