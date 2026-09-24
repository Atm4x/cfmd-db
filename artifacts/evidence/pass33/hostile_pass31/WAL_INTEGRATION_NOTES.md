# WAL integration notes from hostile review

Agent-1's logical-authoritative WAL design is compatible with the Pass31 root-owner direction, but integration should not start on the current leaf state.

Required order:

```text
repair maintained leaf row/handle/order invariants
    -> hostile regression gate
    -> stable durable descriptor/envelope
    -> prepare runtime candidate
    -> durable PREPARE as required by protocol
    -> seal/final freshness guard
    -> durable COMMIT fsync
    -> immediate infallible root publication
    -> ACK
```

After COMMIT fsync, no ordinary abort/drop path may leave the process serving the old root. A crash is safe because recovery replays the commit; a normal in-process early return is not equivalent unless the runtime switches to an explicit recovery-required/fail-stop state.

`RwLockWriteGuard` held through fsync is correctness-first but stalls new readers. Do not weaken the guard merely for latency until there is a replacement protocol with an equally strong final-freshness/commit fence.
