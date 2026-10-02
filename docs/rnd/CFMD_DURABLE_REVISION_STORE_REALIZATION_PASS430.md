# PASS430 R&D — DurableRevisionStore checkpoint realization authority

## Law

A durable physical realization belongs to one exact checkpoint cut, not implicitly to the recovered WAL head:

```text
checkpoint Revision Rcut
+ optional CFPR(Rcut)
+ exact WAL suffix Rcut -> Rhead
= recovered durable authority
```

`CFPR.revision != checkpoint.id` is corruption. Carried WAL does not mutate or relabel the checkpoint physical root.

## Accepted implementation

Single-file `DurableRevisionStore` owns an optional validated `DurableFactorizedRealization`. Synchronous checkpoint publication and streaming checkpoint finalization include that image in the same authenticated generation. Streaming captures CFPR when the cut is taken, then permits later WAL commits. Reopen restores both authorities separately.

## Hostile results

- synchronous physical checkpoint -> reopen: PASS;
- active-generation compaction -> reopen: PASS;
- ordinary same-revision rotation preserves physical root: PASS;
- streaming physical cut at R0 + commits to R1/R2 + finalize/reopen: checkpoint physical root remains R0 while durable head is R2: PASS;
- full `kernel-durability`: 237 passed / 2 ignored.

## Rejected

- relabeling a cut-bound physical root to durable WAL head;
- silently omitting explicit physical authority on directory backend;
- persisting runtime Scan/witness/cache state;
- creating a second semantic or history store.

## Immediate continuation

1. directory generation-owned physical carrier with atomic/checksummed publication;
2. streaming/bounded CFPR encoding instead of first-boundary full `Vec`;
3. encrypted physical-root-specific crash/compaction matrix;
4. historical root -> realization root -> shared physical atoms reachability.
