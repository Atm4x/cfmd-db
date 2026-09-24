# PASS115 — Historical #16 consensus runtime closeout

Status: **COMPLETE**.

Historical #16 is **PROD CLOSED Pass115**.
Historical ledger: **18 / 22 PROD CLOSED**.

## What changed

1. Completed the missing Pass114 full-workspace gate on unchanged source, promoting 16B to COMPLETE.
2. Added canonical authenticated replication transport frames and wire codec.
3. Added current-policy Ed25519 transport verification and per-session replay fencing.
4. Added bounded anti-entropy summaries/requests/chunks over the durable decision-lock frontier.
5. Added advisory logical-clock failure detection; it can only install the existing durable quorum-loss fence.
6. Added store-level transport routing so authority-bearing peer evidence reaches the durable 16B journal while heartbeat/anti-entropy stays advisory.
7. Added real multi-process transport fault tests for valid delivery, replay, tamper and torn send.

## Safety boundary

- Transport is I/O-runtime agnostic; callers may carry the stable wire format over TCP, QUIC,
  shared memory, etc. No specific networking stack becomes consensus authority.
- Transport session sequence numbers are ephemeral replay/dedup state, not durable authority.
- Durable safety remains term/vote/lock/auth/recovery state from 16A/16B.
- Failure detection never performs recovery. Authenticated quorum recovery remains mandatory.
- Anti-entropy chunks never create locks or votes.

## Final gates

- fmt: PASS
- workspace check: PASS
- strict workspace Clippy: PASS
- full workspace: **738 declared / 730 passed / 0 failed / 8 ignored**
- `kernel-durability`: **91/91 unit PASS**
- multi-process transport matrix: **2/2 PASS**

## Historical state

Closed: 18/22.
Remaining: #10, #13, #17, #20.
