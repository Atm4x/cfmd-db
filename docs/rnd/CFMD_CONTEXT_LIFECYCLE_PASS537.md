# CFMD PASS537 — bounded Context lifecycle and contract representability

## Question

Can the selected scoped `Context<M>` model cross an online schema publication without mutating its typed contract, reviving the old schema as current authority, or hiding incompatible consumer binding behind generic schema/type errors?

## Existing theorem recovered from mainline

The implementation already has the required bounded-scope mechanics:

```text
Context<A> admission
    -> one immutable A formation root
    -> Candidate<A> + exact observations + IntentJournal

A -> B publication
    -> Context<A> does not switch branch
    -> reads remain on its admitted A Candidate
    -> commit seals A formation proof at the migration boundary
    -> exact effect transports forward
    -> publication is authorized in current B

next admission
    -> B formation root
```

This is not dual-schema current-world routing. The old semantic world exists only as the already-admitted scope / retained proof authority required to justify that scope's intent.

## PASS537 executable closure

A hostile product regression now proves the complete bounded lifecycle:

1. admit `Context<TodoSchema>` in schema revision A;
2. publish a definitionally-equivalent A -> B migration;
3. commit a B-native row after cutover;
4. the running A Context still reads its original formation world and does not observe the B-only row;
5. stage a new A intent in that same scope after cutover;
6. `Context::commit` transports/publishes the intent into current B through the existing schema-aware publication theorem;
7. the next newly admitted Context observes schema B and both current rows.

No callback replay, HEAD re-read, old-schema live query routing, or Context type mutation is used.

## Stable representability surface

Typed consumer binding previously surfaced low-level `InvalidSchema` / `TypeMismatch` errors directly. That made an expected zero-downtime compatibility decision indistinguishable from malformed authoritative-schema diagnostics.

PASS537 introduces:

```text
ErrorKind::ContractNotRepresentable
ProtocolErrorCode::ContractNotRepresentable
```

`Context` and typed `Snapshot` binding translate only schema/type incompatibility at the consumer-binding boundary into this stable product error. Permission, session, query, recovery and internal errors remain unchanged.

The protocol code is carried explicitly over the canonical wire codec; it does not collapse to `Internal`.

## Hostile finding: what this does NOT solve

There are two different situations:

```text
(1) Context<A> admitted before A -> B, finishes after B
(2) a new A-only client arrives after B is already authoritative
```

PASS537 closes (1).

For (2), binding an A contract directly against B is valid only when the current B schema already represents that contract directly (for example a compatible consumer projection / unchanged semantic identity). A genuine semantic migration such as type conversion, split/merge, or renamed semantic coordinate needs an explicit certified current-world bridge.

Using the retained historical A snapshot for new reads would be stale and is rejected. Keeping A as a second live schema would violate the one-current-world law. Name/existence fallback and per-query routers are also rejected.

The clean remaining theorem is therefore a `SchemaBridge<A,B>`-class compiler:

```text
A-language read q_A
    -> certified B-native read q_B

A-language exact intent u_A
    -> certified B-native exact effect u_B
```

with representability certificates and `ContractNotRepresentable` when the law cannot be proved. It must operate on current B authority, not on reconstructed A state.

## Performance law

PASS537 adds no Context hot-path routing. Normal scoped reads remain bound to the admitted immutable root; cross-schema commit continues through the existing exact schema-aware effect walker. Consumer binding adds only an error-boundary translation. Protocol error encoding is one enum tag.

## Decision

The bounded Context lifecycle is CLOSED. The broader zero-downtime old-client-after-cutover problem remains OPEN and is now isolated precisely as current-world contract bridging rather than lifecycle ambiguity.
