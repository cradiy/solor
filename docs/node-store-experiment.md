# Node Store Access Experiment

## Scope and decision

The `node_store` example tests one borrowing mechanism: a node holds mutable
access to its behavior while invoking a child through a restricted context.
It wraps the existing `Tree` and its `SlotMap`; it does not implement a new map
or change the public core API.

The mechanism works in safe Rust with heterogeneous boxed payloads. It remains
an experiment because traversal has measurable overhead and the UI execution
contract needs separate validation. No layout, paint, invalidation scheduler,
deferred edit queue, or component API is implemented here.

Implementation: [store](../crates/solor-core/examples/node_store/store.rs),
[tests](../crates/solor-core/examples/node_store/tests.rs),
[scalar measurements](../crates/solor-core/examples/node_store/measure.rs), and
[heterogeneous workloads](../crates/solor-core/examples/node_store/workload.rs).

## Access contract

`NodeStore<T>` owns a `Tree<Slot<T>>`. Each slot contains an `Option<Box<T>>`
and an interruption flag. `T` may be a trait object. Identity and topology stay
in the tree throughout execution.

An execution guard moves the box out of the slot, then lends `&mut T` and a
restricted context to the callback. Moving the box does not move its allocation.
The parent retains its behavior while a child runs using another guard. No
payload cloning, runtime borrow cell, or unsafe aliasing is needed.

The context exposes current identity, parent identity, child enumeration, node
status, and execution of direct children. It does not expose the backing store
or another payload reference. Self/ancestor reentry and calls to siblings or
grandchildren that bypass their parent are rejected before running the callback.
Sequential calls to the same child are allowed.

Insertion, removal, and direct payload mutation are available outside execution.
During execution, the store is exclusively borrowed and the context has no
structural mutation methods. This experiment does not provide deferred edits.

## Completion and failure

| Status | Meaning |
| --- | --- |
| `Ready` | Payload is present; its last invocation returned `Ok`, or it has not run |
| `Running` | Payload belongs to an active execution guard |
| `Interrupted` | Payload is restored; its last invocation returned `Err` or unwound |

The guard restores the payload on success, error, and panic unwinding. Restoration
does not allocate or invoke payload code. Structural mutation restrictions ensure
that the destination slot still exists. An invalid or foreign handle is rejected
without starting execution.

Restoration does not undo payload mutations. `Interrupted` is local execution
status, not a cache-validity bit or pending-work queue. A successful retry clears
it; direct mutation does not. A parent that catches a child failure and returns
`Ok` becomes `Ready` while that child remains `Interrupted`. A future scheduler
must track dependencies and incomplete work separately.

Callers supply a nonzero maximum execution depth. Entry starts at depth one;
exceeding the limit returns an error before borrowing the next payload. This
bounds nested store invocation, not arbitrary recursion or stack allocation
inside user callbacks. It does not guarantee recovery from stack overflow,
process abort, or a second panic during unwinding.

## Correctness evidence

Seven focused tests cover:

- Heterogeneous parent/child dispatch and stable payload addresses.
- Reentry and forbidden traversal paths, rejected before callback invocation.
- Stale/foreign handles, slot reuse, and root-removal rejection.
- Nested panic restoration, retained mutations, retry, and exact destruction.
- A parent catching a child's panic and continuing with a sibling.
- Depth-limit failure and restoration along a deep tree.
- Returned errors and status changes after successful retry.

Separate compile-fail checks verify that callback context insertion/removal is
unavailable (`E0599`) and capturing the mutably borrowed outer store for removal
is rejected (`E0499`). These were local compiler checks, not committed test
fixtures. The existing tree and state tests remain separate from this example.

```sh
cargo test --workspace --offline
cargo test -p solor-core --example node_store --offline
cargo run --release -p solor-core --example node_store --offline
```

## Measurement boundaries

The harness pairs direct `Tree<Box<T>>` operations with scoped loans, alternating
their order over seven samples after warmup. Both perform the same payload edits
on equivalent trees. It prints median and minimum/maximum batch time. Construction
and destruction are outside execution timing. The wide-tree batch visits the
root and all 10,000 children; the chain batch visits all 32 nodes.

Direct traversal releases the parent's payload borrow before descending. It is
a lower-bound cost reference, not an alternative implementing the same parent
behavior and context contract. Ratios measure the added access mechanism in
minimal callbacks, not an application slowdown or a comparison with another GUI.

Three local release runs produced these ranges of per-run medians:

| Batch | Direct tree | Scoped loan |
| --- | --- | --- |
| Parent and one child | 6.3–6.7 ns | 14.8–15.7 ns |
| Root and 10,000 children | 35.5–36.3 us | 115.7–120.9 us |
| Chain of 32 nodes | 154.6–161.1 ns | 263.1–280.1 ns |

Single-node tests use trees of 100, 10,000, and 50,000 nodes. A separate
trait-object test includes dynamic dispatch. Their very short timings are
sensitive to optimization and execution noise; they do not justify a precise
cost or speedup claim. Real layout and recording work are absent from every case.

An external Linux allocation probe counted allocator calls between the printed
`begin`/`end` markers. A positive control observed one requested 123-byte
allocation. The execution cases observed zero allocations. This describes the
tested callbacks; arbitrary behavior code can allocate.

Construction of 10,000 scalar nodes, including the same handle vector and boxed
payloads, made 10,028 allocation calls in both variants. Live allocator-usable
bytes increased by approximately 128 KiB for the loan variant. This includes
arena capacity and allocator rounding, and is not RSS/PSS or a universal
per-node cost. Both variants already pay for individually boxed payloads; this
comparison does not evaluate alternatives to boxing. Probe code and diagnostic
logs are local artifacts, not project dependencies.

## Heterogeneous workloads

The workload fixture contains 10,101 nodes: a root, 100 branches, and 100 leaves
per branch. Branches, scalar leaves, and leaves containing a 16-word block have
different payload sizes and behavior implementations behind `dyn Body`. Each
invocation begins with node-specific computation, calls children, combines their
returned values in order, and finishes by updating the parent's output.

The direct reference splits parent execution around recursive calls, releasing
and then reacquiring its payload borrow. Both variants invoke the same `begin`
and `finish` methods and produce identical outputs and execution counts. This
reference requires a fixed, externally controlled traversal; it cannot replace
the loan contract for arbitrary behavior that holds its own state while deciding
when to call children. It is a cost reference, not a competing public API.

Two work levels use one or eight rounds of integer arithmetic per leaf. These
are synthetic execution loads, not text measurement, layout, or rendering.
The fresh fixture is compared with a reused fixture where half the leaves are
removed and recreated in reverse order within each branch. Both compared stores
receive the same edits. This tests one controlled reuse pattern, not arbitrary
heap fragmentation or prolonged application churn.

Each workload executes either the whole tree or explicit jobs targeting one or
ten branches. A job is a subtree root supplied by the benchmark; there is no
dirty discovery, deduplication, or dependency propagation. Unselected nodes,
including ancestors of selected roots, retain their previous outputs. A focused
test verifies result equivalence and untouched outputs for fresh and reused
fixtures. Runtime assertions also check every live node's output and total visit
count outside timing.

The harness uses 100 warmup batches, seven alternating samples, and approximately
two million node invocations per variant per sample. Three release runs gave the
following ranges of per-run medians, in microseconds per batch:

| Storage and work | Nodes executed | Split direct reference | Scoped loan |
| --- | --- | --- | --- |
| Fresh, light | 101 | 0.82–0.86 | 1.35–1.44 |
| Fresh, light | 1,010 | 8.78–9.09 | 14.20–14.71 |
| Fresh, light | 10,101 | 97.0–97.8 | 150.2–153.6 |
| Fresh, compute | 101 | 4.17–4.23 | 4.59–4.67 |
| Fresh, compute | 1,010 | 42.08–42.22 | 46.36–46.71 |
| Fresh, compute | 10,101 | 427.2–428.2 | 469.8–473.5 |
| Reused, light | 10,101 | 100.7–109.3 | 159.6–162.9 |
| Reused, compute | 10,101 | 442.3–444.9 | 485.9–488.4 |

For fresh whole-tree work, the added cost is about 55–57% with light computation
and 10–11% with heavier computation. The smaller percentage does not eliminate
the mechanism's cost. Repeated access to selected subtrees is cache-friendly;
these timings do not establish cold-cache latency. All measured execution
intervals had zero observed allocator calls using the same external probe and
positive control described above. Construction, edits, job selection, and
verification are outside those intervals.

These measurements do not isolate map lookup, validation, dispatch, and guard
restoration costs individually. They provide no evidence that changing the map
would improve the complete workload. The store implementation retains its
identity, reentry, depth, and failure checks.

## Next decision

The results support retaining this access layer as a candidate without replacing
`SlotMap`. They do not establish that its overhead is acceptable for the final
framework. Selected-subtree execution scales with supplied work in this fixture,
but does not establish correct incremental layout. The separate
[layout contract experiment](layout-contract.md) tests restricted contexts,
ancestor invalidation, and failure-safe publication in a narrow sizing domain.
The broader [node model](node-execution.md) still needs review before introducing
public behavior traits or integrating these examples into `Runtime`.
