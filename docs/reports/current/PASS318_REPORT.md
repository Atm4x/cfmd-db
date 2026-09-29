# PASS318 REPORT — Acknowledged provider key-authority handoff

## Wall clock
- Start: 2026-09-29 01:07:45 UTC
- Functional freeze: 2026-09-29 01:27:45 UTC
- Useful boundary: 2026-09-29 01:27:45 UTC
- Hard boundary: 2026-09-29 01:31:45 UTC

## Goal
Close the authority gap left after P317 between a locally durable wrapped-DMK successor slot and an independently durable external provider/KMS anti-rollback decision. The handoff must survive a crash at every boundary without making a locally newer but externally unacknowledged key slot unconditional authority, without rewriting database ciphertext, and without requiring pass-to-pass format versioning.

## Delivered

### 1. Explicit external acknowledgement contract
`EncryptionKeyProvider` now exposes `acknowledge_database_key_epoch(...)`. The acknowledgement is typed as `EncryptionKeyAcknowledgement` and binds:

```text
provider key ID
provider key epoch
database key epoch
```

`Database::rewrap_encryption(...)` now performs the authority transition in this order:

```text
publish successor wrapped-DMK slot
        -> fsync local header
        -> provider/KMS durable acknowledgement
        -> retire predecessor wrapped-key slot
        -> fsync retirement
```

A provider that does not implement durable acknowledgement cannot silently claim a completed rotation. The failure is surfaced explicitly after local successor publication, with the operation documented as retryable/idempotent.

### 2. A physically newer slot is not automatically external authority
P317 selected the highest locally valid wrapped-key slot before checking provider identity. That was correct for local crash publication but wrong for a two-durability-domain handoff: a crash after local successor fsync but before provider acknowledgement could make the old acknowledged provider unable to reopen the database.

P318 changes wrapped-key admission to select the newest slot that is admissible under the supplied provider snapshot:

```text
valid local slot
AND provider key ID matches
AND provider key epoch matches
AND database-key epoch >= external floor
```

Therefore, while a successor is only locally pending:
- the old acknowledged provider can still reopen the old slot;
- the successor provider can open/adopt the new slot;
- neither path uses error fallback or guesses authority.

After the provider advances its minimum database-key epoch, the old slot is no longer externally admissible even if an attacker restores old valid bytes.

### 3. Crash-retry adopts the already-durable pending successor
A restarted process may reopen under the old acknowledged provider and retry the same rewrap using the successor provider. P318 detects an already-present matching pending slot instead of writing another wrap.

Pending adoption is accepted only when both coordinates are exact successors:

```text
pending.publication_sequence == active.publication_sequence + 1
pending.database_key_epoch   == active.database_key_epoch + 1
```

Any matching but non-consecutive jump is corruption.

A low-level regression verifies that retrying after restart leaves the complete header bytes unchanged before acknowledgement/retirement: no new nonce, no new wrapped DMK and no spurious epoch increment are published.

### 4. Predecessor retirement only after acknowledgement
After external acknowledgement succeeds, CFMD zeroes the inactive predecessor `CFKW` slot and syncs the file.

Crash behavior is therefore explicit:

```text
before successor fsync
    -> old authority only

after successor fsync, before provider ack
    -> old authority remains externally admissible
    -> successor is recoverable pending handoff

after provider ack, before local retirement
    -> external floor fences rollback to old slot
    -> both physical slots may temporarily remain

after predecessor retirement fsync
    -> only acknowledged successor slot remains locally
```

Retirement validates the acknowledged database-key epoch against the active successor and is idempotent for that epoch.

### 5. Product E2E acknowledgement-failure recovery
A new `cfmd-runtime` regression intentionally uses a successor provider that cannot acknowledge the epoch:
1. old provider creates the encrypted database;
2. successor wrapped slot becomes locally durable;
3. acknowledgement fails and the API returns an error;
4. database is closed/restarted;
5. old acknowledged provider successfully reopens it;
6. retry with the same successor provider adopts the existing pending slot;
7. external floor advances to epoch 2;
8. predecessor slot is retired;
9. old provider can no longer open, successor provider can.

This closes the main P317 hand-wave around independent local/KMS durability domains.

### 6. Hostile incidental fix: hosted watch cancellation race
During full verification, `cfmd-host::authorization_refresh_revokes_watch_without_closing_session` exposed a real pre-existing race: `SubscriptionRegistry::cancel_all()` removed subscription entries before cancellation reached a concurrently starting `NextWatch`. Depending on scheduling the request returned `NotFound` instead of the semantic `WatchClosed`.

P318 fixes the underlying protocol behavior rather than weakening the test: bulk cancellation now retains the bounded subscription entries and cancels their watch capabilities, matching single-subscription cancellation semantics. Explicit close/session close still removes entries. The race regression was repeated 20 times and the complete `cfmd-host` suite was repeated 10 times after the fix.

### 7. No format-version churn
P318 does not increment `FORMAT_VERSION`. CFMD is still pre-release and this pass changes authority semantics around the existing dual-slot layout, not a released compatibility boundary.

## Verification
- `kernel-durability`: **182/182 PASS** plus multiprocess suites PASS
- `cfmd-runtime`: **34/34 PASS**
- `cfmd-host`: **8/8 PASS**
- `cfmd-protocol`: **4/4 PASS**
- pending-handoff restart/adoption regression: PASS
- acknowledgement-failure recovery E2E: PASS
- provider-floor complete-header rollback regression: PASS
- hosted authorization/watch race regression: **20/20 PASS** repeated
- complete `cfmd-host` suite after race fix: **10/10 PASS** repeated
- strict `cargo clippy --workspace --all-targets --offline -- -D warnings`: PASS
- strict rustdoc `-D warnings` for touched/public production boundaries: PASS
- `cargo fmt --all -- --check`: PASS
- Rust: supplied **1.98.1** toolchain

Lean 4.34.0 was not required. P318 does not change root/generation/WAL publication authority, publication crash points, or the existing Lean single-file publication model; it closes the independent wrapped-key/provider authority lifecycle above that physical law.

## Architectural conclusion
The wrapped-key transition is now an explicit two-domain protocol rather than "fsync locally, then hopefully update KMS". Local durability answers which key envelopes physically survived. External provider authority answers which database-key epochs are admitted. Pending successor publication is recoverable without silently promoting it, and successful external acknowledgement is followed by deterministic predecessor retirement.

## Next target — P319
The next encryption payer is the remaining unbounded generation write path. `prepare_stored_sections()` currently materializes every encrypted section as a `Vec<u8>` before generation layout/publication, even though P315 already made encrypted read/copy bounded.

The clean pre-release redesign identified during P318 is a streaming generation layout rather than another buffer/fallback:

```text
fixed header with deterministically computable stored lengths
    -> aligned section ciphertext streamed directly to file
       while section digests + generation digest are accumulated
    -> descriptor table/footer written after section data
    -> sync generation
    -> publish root
```

Encrypted stored lengths are computable from plaintext lengths/chunk counts before encryption, so the header can know the final layout without staging ciphertext. Putting the descriptor table after section data lets section digests be produced during the one streaming write instead of requiring whole-section materialization or a second in-memory representation. Because CFMD has no released format yet, P319 should prefer this singular clean layout over compatibility routing.

After streaming generation publication, close directory-encryption parity through the same AE v1 codec/key hierarchy. Password-to-KEK/Argon2id remains a separate product adapter after the durability/encryption core is closed.
