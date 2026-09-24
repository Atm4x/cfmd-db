# PASS115 frontier — finish 16B gate, then 16C

1. First action: rerun `cargo test --workspace --all-targets --offline` on frozen Pass114 source. No source edits before this gate unless it fails.
2. If green: mark 16B COMPLETE.
3. 16C: authenticated transport envelopes and peer session binding.
4. Anti-entropy: exchange durable journal/decision-lock frontiers; missing/conflicting locks must reconcile before authority recovery.
5. Failure detector is liveness-only and must never create authority.
6. Distributed multi-process hostile matrix: partition, delayed/reordered/duplicated messages, stale trust epoch, competing leaders, quorum loss/rejoin, membership transition, crash/restart.
7. #16 closes only after the actual multi-process runtime evidence is green; do not substitute unit tests for transport/fault execution.
