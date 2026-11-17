# Layout Cost Experiment

## Question and boundary

Measure the complete update-to-layout-publication path of the headless fixtures.
Compare one versus three inline measurement-cache slots without changing node
behavior, invalidation, layout reuse, or publication rules. No rendering, text
shaping, input dispatch, or frame scheduling is included.

Two example targets compile the same source with different inline cache sizes.
This measures the actual storage difference, rather than limiting active slots
while reserving three slots in both variants. The normal layout-contract example
continues to use three slots. This is an experiment control, not a public option.

## Workloads and acceptance

The fixtures contain a vertical container and either 100 or 2,500 icon/label/button
rows: 401 or 10,001 nodes. Workloads cover clean requests, ink edits, one label
edit, edits to ten percent of labels, alternating widths, a 24-width sweep, and
changes to allocated height. All timing includes edits, ancestor invalidation,
query lookup, measurement/layout, temporary allocation, and geometry publication.
Construction, warmup, counters, checksums, and correctness checks are outside timing.

Each target first compares every workload with cache-disabled recomputation.
Timed runs emit final geometry fingerprints and work counts. Fingerprints and
published-change counts must agree across cache capacities. A performance result
is usable only when correctness checks pass and repeated sequential runs support
it. Cache-disabled recomputation is a correctness reference, not the timing baseline.

Seven samples follow 100 warmup updates. Diagnostic `begin`/`end` markers delimit
timed work, construction plus initial layout, and destruction for an external
allocation probe. Allocation-instrumented timings must not be used as CPU results.
Construction probes retain the fixture across the end marker; destruction probes
check that its live heap storage is released. Heap measurements are not RSS/PSS.

```sh
cargo build --release -p solor-core --example layout_cost_one --example layout_cost_three --offline
cargo test -p solor-core --example layout_cost_one --example layout_cost_three --offline
cargo run --release -p solor-core --example layout_cost_one --offline
cargo run --release -p solor-core --example layout_cost_three --offline
```

Repeat the two executables in alternating order without running other benchmarks
concurrently. Stop at a supported cache tradeoff and identification of remaining
costs; any optimization needs its own paired comparison and preserved invariants.

## Measurements

This cache-capacity comparison uses the fresh-scratch baseline at `29569b5`.
The current engine's separate [scratch reuse measurements](layout-scratch.md)
cover retained temporary capacity and explicit reclamation.

Release measurements used Rust 1.100.0 on Linux x86-64 with an AMD Ryzen AI 7 H
350. Six uninstrumented runs used the order one/three, three/one, one/three.
Each table cell reports the median of three run medians, followed by their range.
Times are microseconds per complete update. These are short synthetic workloads
on a shared machine, not statistical confidence intervals or frame budgets.

| Nodes | Update | One slot, us | Three slots, us |
| --- | --- | --- | --- |
| 401 | One label | 5.51 [5.51, 5.86] | 5.80 [5.54, 6.06] |
| 401 | Ten percent of labels | 8.68 [8.59, 8.71] | 8.54 [8.35, 8.62] |
| 401 | Two alternating widths | 30.19 [29.93, 30.31] | 17.79 [17.22, 17.94] |
| 401 | 24-width sweep | 28.98 [28.07, 29.01] | 27.77 [27.38, 27.97] |
| 401 | Allocated height | 3.89 [3.85, 4.10] | 4.02 [3.89, 4.04] |
| 10,001 | One label | 135.82 [133.95, 141.66] | 140.28 [138.68, 141.87] |
| 10,001 | Ten percent of labels | 223.74 [222.57, 228.21] | 225.75 [224.97, 226.60] |
| 10,001 | Two alternating widths | 788.47 [784.72, 790.66] | 465.66 [463.35, 481.38] |
| 10,001 | 24-width sweep | 776.22 [768.20, 778.28] | 727.46 [725.65, 732.15] |
| 10,001 | Allocated height | 99.25 [98.45, 102.07] | 105.10 [104.73, 105.21] |

Clean requests and ink-only edits perform no measurement or layout and allocate
nothing. Their timings were a few nanoseconds per request, too small to support
a useful cache-capacity ranking. Ink edits here only mutate a drawing property;
they do not measure paint scheduling or rendering.

Every workload passes 48 incremental-versus-cache-disabled steps before timing.
The two example tests run those comparisons independently. Final geometry
fingerprints and average changed-publication counts agree across all six timed
runs. The reference shares the fixture algorithms; the separate layout-contract
tests provide known-value sizing and failure checks.

## Allocation and retained storage

A separate external glibc allocation interceptor counted allocation/reallocation
calls, cumulative requested bytes, and live allocator-usable bytes between the
printed markers. A Rust positive control requesting 123 bytes produced one call
and 136 usable bytes. Instrumented CPU times are excluded from the table above.
The probe is a local diagnostic, not a dependency or portable measurement API.

Both cache capacities produced the same temporary allocation counts and bytes.
For 10,001 nodes:

| Update | Allocation/reallocation calls per update | Requested bytes per update |
| --- | --- | --- |
| Clean or ink | 0 | 0 |
| One label | 13 | 453,096 |
| Ten percent of labels | 262 | 471,024 |
| Two alternating widths | 2,290 | 1,796,544 |
| 24-width sweep | 2,203.84 | 1,694,051 |
| Allocated height | 12 | 453,024 |

Fractional values average updates visiting different numbers of nodes. Requested
bytes include repeated buffer growth and are not peak live bytes. All measured
update batches ended with zero net live-heap change; this does not measure peak
temporary storage.

| Nodes | One slot: live usable bytes | Three slots: live usable bytes | Difference |
| --- | --- | --- | --- |
| 401 | 103,064 | 122,296 | 19,232 |
| 10,001 | 2,832,776 | 3,312,936 | 480,160 |

Live storage includes the tree, node and behavior boxes, topology, caches,
published geometry, and fixture handle vectors after initial layout. Each
destruction probe released exactly its construction probe's live-byte increase.
The extra storage is approximately 48 bytes per node on this build, about 469 KiB
or 17 percent of this entire large fixture's retained heap. Allocator size classes
and rounding affect usable bytes. These figures exclude allocator metadata,
reserved arenas, stacks, code, and other process memory; they are not RSS/PSS.

## Decision and remaining cost

Three slots remain a useful experimental candidate: alternating two widths is
about 41 percent faster at both sizes, and the large width sweep is about 6 percent
faster. There is no measured local-edit advantage. The large single-label case
is about 3 percent slower, with overlapping run ranges, and height-only updates
are about 6 percent slower. Fewer measurement bodies do not guarantee less total
CPU time: these synthetic leaves are cheap and larger entries also have costs.
This comparison does not establish an optimal capacity; two slots, selective
caches, real shaping costs, and other tree shapes remain unmeasured.

With three slots, a single label edit in 10,001 nodes executes only four measure
bodies and three layout bodies, yet still performs 5,009 measurement queries.
Conservative ancestor invalidation makes the root column revisit its 2,500 rows.
Some subsequent rows must move when an earlier row changes height, but the current
implementation visits every row regardless of whether its geometry changes.
An allocated-height update similarly issues 2,501 cached queries, executes only
the root layout, and publishes one changed entry.

This baseline creates a fresh publication journal per attempt and a child-state
vector per executing container. The [scratch reuse experiment](layout-scratch.md)
compares the same workloads, including failure atomicity, retained capacity,
and allocation reduction.
Reducing traversal requires a separate decision about container invalidation and
placement dependencies. Neither finding calls for changing the backing map or
publishing the prototype as a UI API.
