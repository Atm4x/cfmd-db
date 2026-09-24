# IMPLEMENTATION REPORT — Pass86

## Scope

Production integration of historical problem #11 on top of Pass85. No portable workspace was transplanted wholesale; the implementation was rebased around the current Γ-REIC and durability model.

## `kernel-durability/src/lib.rs`

Introduced `IdempotencyEpoch` and `DurableTransactionKey`. Recovery and committed transaction maps now use the composite key. `DurableRevisionDescriptor` carries the bound epoch and optional independently allocated revision-effect id. `DurableRevisionEffectRecord` stores transaction epoch/id plus canonical durable intent. Added `RetryHistoryExpired` and epoch-qualified recovery accessors.

WAL prepare codec v9 serializes the epoch and independent effect id for bound prepares. Legacy descriptors remain decodable and unbound legacy-compatible descriptors retain the previous encoding where required by compatibility tests.

## `kernel-durability/src/metadata.rs`

Metadata codec v12 persists current/minimum retry epochs and epoch-qualified retry entries. Revision-effect metadata is self-contained and includes canonical durable intent. Legacy metadata paths rehydrate zero-epoch records and infer legacy effect intent from retained transaction data when available.

## `kernel-durability/src/store.rs`

The store now owns the retry horizon and a monotone independent effect-id allocator. Prepare binds the current epoch and allocates the causal identity before WAL publication. Reopen merges checkpoint and WAL retry keys, derives a newer current epoch from committed WAL when a crash happened before checkpoint, and advances the effect allocator beyond every recovered event.

Added epoch-qualified retry queries, epoch advance, and watermark/GC operations. Γ-REIC validation/ideal construction consumes the causal record's own intent rather than the retry ledger.

Two hostile tests cover (1) raw-id reuse in a new epoch followed by crash before checkpoint and (2) retry GC/watermark persistence while causal history remains valid.

## `kernel-plan/src/lib.rs`

Exposed epoch-qualified transaction outcome, retry-horizon, epoch-advance, and retry-expiry operations through the durable runtime surface. Existing causal tests were updated to discover causal ids from revision frontiers rather than assuming `RevisionEffectId == ClientTransactionId`.

## Verification result

Final frozen tree: fmt PASS, workspace check PASS, workspace strict Clippy PASS, full workspace tests **636/0/8** (passed/failed/ignored).
