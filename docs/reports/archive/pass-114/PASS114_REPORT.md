# PASS114 — Historical #16B authenticated peer evidence + quorum-loss/recovery

Status: **CLOSURE CANDIDATE / FULL-WORKSPACE TEST GATE INCOMPLETE**.

Source was frozen at hard wall. No production changes are permitted after this point in Pass114.

## Implemented

- Rebased `kernel-auth` verifier substrate and vendored offline crypto dependencies from `CFMD_RND_HIST10_DEPLOYMENT_AUTH_RND_CLOSED_PASS109_2026-09-23.zip`.
- Added durable `ReplicationPeerAuthPolicy` binding cluster identity, trust epoch and `ReplicaId -> KeyId`.
- Added domain-separated canonical Ed25519 peer evidence for term promises, leader votes, effect votes, membership votes, decision votes, joint-membership acknowledgements, and recovery acknowledgements.
- Authentication proof is durably journaled before semantic vote admission; replay requires the proof once auth is active.
- Certificates re-check current-epoch authenticated evidence; structural acknowledgement sets alone are no longer authority after auth activation.
- Added durable quorum-loss fence. While lost, new leader/decision/quorum/membership authority and publication are fenced, while local ingest remains possible.
- Added authenticated recovery acknowledgements carrying lock frontiers and a recovery certificate that requires current membership quorum, a strictly advancing term, exact local lock reconciliation, and fails closed when anti-entropy is required.
- Added authenticated joint-membership successor quorum, including disjoint old/new membership support.
- Added monotone quorum-loss observed term and authoritative availability term tracking.
- No private keys are stored in the consensus journal; signing remains external, verification is in `kernel-auth`.

## Hostile coverage

- forged/tampered evidence rejected;
- signer/voter, cluster and trust-epoch binding;
- stale evidence rejected after trust rotation;
- restart replay preserves auth policy/proofs;
- quorum-loss fences authority;
- authenticated lock-frontier recovery survives restart;
- joint membership requires authenticated successor quorum;
- quorum-loss observed term cannot regress.

## Gates before hard wall

- `cargo fmt --all -- --check`: PASS
- full workspace `cargo check --workspace --all-targets --offline`: PASS
- full workspace strict Clippy: PASS
- `kernel-auth`: 12/12 PASS
- `kernel-durability`: 86/86 PASS
- full workspace tests: **INCOMPLETE** because the tool invocation timed out during later workspace suites; no failure had been observed. This is not recorded as PASS.

Therefore historical #16 remains `ADVANCED / PROD PARTIAL`, and 16B is not promoted to COMPLETE until the remaining full-workspace test gate is rerun successfully in Pass115.
