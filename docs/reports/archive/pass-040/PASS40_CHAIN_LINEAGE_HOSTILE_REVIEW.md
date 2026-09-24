# PASS40 HOSTILE DESIGN REVIEW — `Chain<T>` / `Lineage<T>`

Status: **REJECTED as a new universal logical primitive; ACCEPTED as a possible surface refinement + physical optimization family.**

This review is intentionally separate from the Pass40 production changes. No `Chain`/`Lineage` primitive was added to the kernel.

## 1. Proposal under review

The proposed object is a persistent rooted single-parent acyclic structure with a unique ordered root→node path, supporting workloads such as append, branch, ancestor, LCA and full-path retrieval. Candidate physical representations include persistent chunks, binary lifting/jump tables and rope/path caches.

A terminology note matters: because branching is explicitly wanted, the mathematical object is not generally a chain. It is closer to a rooted arborescence / lineage tree. `Lineage` is therefore the less misleading surface name.

## 2. Hostile question: does this add logical expressive power?

For the current requirements, **no**.

The existing CFMD logical universe already has:

```text
PartialMap<Node, Node>
Seq<Node>
n-ary Relation
guarded recursion / fixed-point query machinery
revisioned persistent semantic state
```

A lineage can elaborate to:

```text
parent : PartialMap<Node, Node>
payload : PartialMap<Node, Payload>     // or ordinary relation(s)
```

plus explicit integrity constraints:

1. `parent` is functional — already guaranteed by `PartialMap`;
2. parent edges are acyclic;
3. there is exactly one root for a single connected lineage (or a root set for a forest variant);
4. every live node reaches that root;
5. optionally, parent mutation is append-only if the application wants immutable ancestry.

Single-parent + acyclic + rooted connectivity already implies a unique root→node path. The ordered ancestry path is therefore a **derived `Seq<Node>` query**, not a new kind of logical value.

Append and branch are typed rewrites over the parent map / node carrier. `ancestor`, `LCA` and full path are derived recursive queries. CFMD revisions already provide persistent historical versions, so “persistent” in the immutable-version sense adds no new semantic law.

If “persistent” instead means “once a node receives a parent that parent can never change”, that is an append-only refinement/transaction constraint. It still does not require a new universal structural primitive.

## 3. What a first-class primitive would cost

Adding `Lineage<T>` to the universal logical kernel would create obligations across almost every layer:

- type/schema representation and validation;
- equality/refinement semantics;
- change/rewrite algebra;
- exact query IR and recursion semantics;
- transport across schema/Γ revisions;
- durability codec and migration;
- merge/branch semantics;
- lifecycle/identity interactions;
- lens/change propagation;
- planner/operator support;
- physical lowering contracts;
- formal laws and hostile tests.

That is substantial abstraction tax for an object whose current laws are already expressible by `PartialMap + constraints + queries`. It would also create exactly the kind of workload-specific behavior in the universal layer that CFMD has so far tried to avoid.

## 4. Useful part of the idea: a surface refinement

A useful API can still exist above the kernel, for example conceptually:

```text
Lineage<Node, Payload>
    elaborates to
        parent : PartialMap<Node, Node>
        payload : ...
        constraints : Rooted + Acyclic + ReachableFromRoot
        queries : ancestors(node), path(node), ancestor(a,b), lca(a,b)
```

This gives ergonomics and one place to certify the invariant bundle without making `Lineage` a peer of `Map`, `Relation`, `Seq`, etc.

If the refinement becomes common enough, CFMD could provide a standard-library/schema helper that emits the parent relation, invariant queries and typed operations. That remains compatible with the current logical universe.

## 5. Physical lowerings are independently worthwhile

The physical optimizer can recognize the refinement/workload without changing semantics. CFMD already has `LayoutFamily::AdjacencyList`, so even the physical vocabulary does not require a new logical primitive.

Reasonable reconstructible lowerings:

| Workload | Correctness-first physical form | More aggressive derivative state |
|---|---|---|
| append/branch | parent map + child adjacency | persistent chunked adjacency / copy-on-write chunks |
| parent lookup | keyed parent map | dense parent array where IDs permit |
| k-th ancestor | parent walk `O(k)` | binary lifting / jump table `O(log k)` |
| ancestor predicate | upward walk | depth + jump table; optional interval labels for suitable immutable trees |
| LCA | parent/depth walk | binary lifting / Euler-tour RMQ depending update regime |
| full root→node path | walk then reverse; output is Ω(path length) | persistent path chunks / ropes / cached shared prefixes |
| descendants | child adjacency | Euler/HLD-style derivative indexes where workload justifies them |

All of these remain reconstructible physical artifacts pinned to the same semantic revision.

### Path compression warning

Union-find-style path compression should **not** rewrite the authoritative parent relation: that changes the lineage itself. A shortcut/jump edge may exist only as derivative physical state. Persistent chunk/rope sharing is usually a cleaner lowering for immutable lineage paths.

## 6. Abstraction-tax comparison with ordinary Relation/PartialMap

### Existing representation

Advantages:

- zero new fundamental semantic type;
- reuses revision, durability, transport, validation and query machinery;
- arbitrary extra edge/node facts remain ordinary relations;
- physical optimizer can specialize only when profitable;
- same data can participate in generic relational queries without conversion.

Costs:

- invariant bundle is not encoded by the bare `PartialMap` type itself;
- generic recursive ancestry/LCA may be slower without a recognized physical lowering;
- ergonomic APIs require a surface helper/refinement.

### First-class kernel primitive

Potential advantage:

- invariants and lineage-specific operations would be syntactically explicit and could expose optimization intent immediately.

Costs:

- duplicates generic map/relation infrastructure;
- significantly expands universal semantics and migration surface;
- risks hard-coding one workload family into every layer;
- still needs the same physical indexes for actual performance.

The optimization advantage therefore does not justify the semantic tax: the optimizer can recognize a refinement/constraint pattern or explicit physical hint without changing the logical universe.

## 7. Falsifier that would change this conclusion

Reconsider a first-class logical primitive only if a future requirement demonstrates a law that cannot cleanly be represented as:

```text
existing structure + explicit invariant/refinement + typed rewrites + derived query
```

For example, a genuinely new cross-API capability semantics whose correctness intrinsically depends on sealed lineage append authority might justify a nominal domain abstraction. The current append/branch/ancestor/LCA/path proposal does not demonstrate such a need.

## 8. Decision

1. **Do not add `Chain<T>` / `Lineage<T>` to the universal CFMD logical kernel.**
2. If ergonomics are needed, add a standard surface refinement/helper over `PartialMap` + invariant queries.
3. Treat ancestry/LCA/path acceleration as reconstructible physical lowering: adjacency, depth/jump tables, binary lifting, persistent chunks/ropes, etc.
4. Keep authoritative parent semantics unchanged; path-compression-like shortcuts may only be derivative state.

This preserves the universal layer while retaining essentially all performance opportunities proposed by the idea.
