# IMPLEMENTATION REPORT — Pass113

## Problem

Pass94 had durable membership epochs, vote-once evidence and quorum publication, but `DurableSequencerOrder` was still an externally supplied ordering witness. There was no durable current/promised term, leader-election authority, term fencing, or consensus lock/carry-forward rule.

## Hypothesis

A consensus authority layer can be added to the existing append-only replication journal without replacing the REIC effect ontology or changing old frame encodings. Safety should become active once a membership epoch has a certified leader, while pre-consensus journals retain their existing behavior.

## Implementation

Changed production files:

- `crates/kernel-durability/src/replication.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-durability/src/lib.rs`

Added journal frame kinds 8–13 for term promises, leader votes, leader certificates, decision votes, decision locks and joint membership certificates.

Added public durable API/query types and store methods for the new authority state. Replay reconstructs every new state component from the same fsynced journal.

`validate_quorum_certificate` now requires a matching `DecisionLock` when first-class leader authority exists for the current membership epoch. Legacy behavior remains unchanged before consensus activation.

Membership replacement in consensus mode now requires a `ReplicationJointMembershipCertificate` backed by old and new membership quorums in the certified term. Later higher-term promises fence stale transition certificates.

## Falsification

Hostile tests attack:

- conflicting same-term leader votes;
- promise regression;
- stale leader use;
- conflicting decision values;
- omission of carry-forward evidence;
- stale joint membership install;
- missing successor quorum;
- restart reconstruction.

The full pre-existing workspace remains green, demonstrating compatibility with earlier durable replication behavior.

## Result

16A is COMPLETE. Historical #16 remains PROD PARTIAL pending 16B+.
