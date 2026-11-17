# Record Storage Evaluation

## Decision and boundary

The current engine keeps owned-vector dependency records in an optional per-node
record block. Complete empty records are implicit; nodes allocate a block only
when retaining nonempty or incomplete evidence. This reduces initial and warmed
heap without changing the per-read recording path. A shared buffer pool is not
selected. There is no propagation pruning or public-core change.

## Selected per-node record block

Measurement-cache slots contain only their query and metrics. An optional boxed
`DependencyRecords` holds one read set per cache slot and one for published layout.
Cache slot indexes identify their matching records. The presence of a valid
measurement or published geometry establishes whether a complete empty record
exists; an absent block does not by itself establish result validity.

Incomplete evidence is not empty evidence, even when its vector has no entries.
It retains a block and cannot be treated as an independent result. When every
read set becomes complete and empty, the block is released. Clearing measurement
records preserves any record still associated with published layout. The
allocation is node-local; there is no shared arena, free list, or pooled capacity.
During a callback, reads still append directly to the local `ReadSet`, and pending
layout records still publish atomically with geometry.

A focused test checks empty leaf evidence without a block, repeated transitions
between nonempty and empty measurement evidence, release of an unused block, and
preservation of published layout evidence during measurement invalidation. The
record-budget test verifies that incomplete evidence retains its block. Existing
failure, conditional replacement, cache eviction, and geometry checks also pass.

### Paired measurements

This comparison uses the same owned-vector baseline as the pooled investigation,
but is a separate measurement session. Both variants use three cache slots and
the same workload source. Release builds use Rust 1.100.0 on Linux x86-64, AMD
Ryzen AI 7 H 350. Three sequential pairs alternate baseline/candidate,
candidate/baseline, baseline/candidate. Geometry fingerprints and all work counts
agree. Timings are microseconds per update, median of three run medians with their
range; do not compare absolute times across separate sessions below.

| Nodes | Update | Inline records | Optional record block |
| --- | --- | --- | --- |
| 401 | Stable label | 6.63 [6.60, 6.92] | 6.65 [6.49, 6.91] |
| 401 | Ten percent of labels | 9.72 [9.58, 10.21] | 10.07 [9.97, 10.12] |
| 401 | Two alternating widths | 23.73 [23.25, 23.89] | 24.00 [23.03, 24.38] |
| 10,001 | One label | 167.91 [155.92, 180.29] | 164.24 [153.83, 164.45] |
| 10,001 | Stable label | 165.11 [156.02, 176.66] | 158.11 [156.68, 165.87] |
| 10,001 | Ten percent of labels | 285.70 [260.32, 296.02] | 277.95 [268.97, 279.40] |
| 10,001 | Two alternating widths | 598.43 [579.71, 654.91] | 593.72 [592.13, 642.51] |
| 10,001 | 24-width sweep | 944.39 [911.41, 966.08] | 912.61 [898.84, 940.06] |
| 10,001 | Allocated height | 109.62 [106.30, 120.09] | 113.12 [111.06, 123.19] |

The ranges overlap and some medians increase. This supports retaining a memory
reduction without claiming a stable CPU speedup. The shared-machine synthetic
workload is not an application frame-time guarantee.

A separate allocation probe, checked with the Rust 123-byte positive control,
reports these live allocator-usable bytes:

| Boundary | Inline records | Optional record block |
| --- | --- | --- |
| 401-node initial layout | 296,984 | 265,824 |
| 10,001-node initial layout | 8,093,496 | 7,313,728 |
| Stable-label preparation plus 100 updates | 8,093,464 | 7,313,728 |
| Two alternating widths, 100 updates | 8,790,128 | 8,010,264 |
| 24-width sweep, 100 updates | 9,486,776 | 8,706,944 |

The large fixture saves approximately 761 KiB, including after activity: about
9.6 percent for the stable-label workload and 8.2 percent for the width sweep.
Scratch release frees the same 1,430,736 bytes in both variants. After stable-label
warmup, retained heap after release is 6,662,728 versus 5,882,992 bytes. Release
plus destruction accounts for each measured live-byte increase. These figures
include allocator rounding and are not RSS/PSS or peak temporary memory.

Warmed update allocation counts and requested bytes are unchanged: the large
stable edit still uses 26 allocation/reallocation calls and 1,180,032 cumulative
requested bytes. Construction adds 2,501 record-block allocations for the root
column and rows: 30,081 versus 32,582 total calls, while cumulative requested bytes
fall from 11,585,272 to 10,705,280. Nodes that repeatedly acquire and lose all
evidence may allocate again. This tradeoff targets retained metadata, not allocation
elimination or startup CPU performance.

Keep the optional block as the current representation. Its benefit is measured
memory reduction; dependency vectors still allocate during real updates, and
ancestor traversal remains unchanged.

## Pooled candidate: not selected

The retained benchmark improvement measures live fixture storage after preparation
and 100 updates, in addition to initial construction. It then measures explicit
scratch release and destruction separately. These probes run after all CPU timing,
so they do not change the timed workloads' allocator history. Looking only at
initial construction misses retained buffers accumulated during updates.

## Evaluated representation

The pooled candidate uses compact internal handles for measurement and published
layout records. Complete empty and incomplete records need no arena slot. Buffers
are reused through capacity-class free lists; slots retain only nonempty records
and reusable storage. A callback owns its active vector directly and returns it
to the pool after execution, avoiding an arena lookup on every observed read.

Cache invalidation and eviction return buffers to the pool. Successful layout
records stay in the unpublished journal until the whole attempt commits. Failure
and panic cleanup return pending buffers without replacing published evidence.
Explicit scratch release frees unused buffer capacity while retaining live records
and the pool's slot metadata. The per-record 16,384-read bound still applies.

The existing recording tests and geometry oracles pass for the candidate. A
candidate-specific stress check also covers repeated failures, retries, and
reclamation without accumulating record slots. Passing those checks does not
establish an acceptable performance tradeoff.

## Complete-path timings

Both variants use the same workload source, three measurement-cache slots, and
reusable geometry scratch. Release builds use Rust 1.100.0 on Linux x86-64, AMD
Ryzen AI 7 H 350. Three sequential pairs alternate baseline/candidate,
candidate/baseline, baseline/candidate. All six runs agree on geometry fingerprints,
query/hit counts, measurement/layout body counts, and publication counts.

Microseconds per update, median of three run medians with their range:

| Nodes | Update | Owned vectors | Pooled candidate |
| --- | --- | --- | --- |
| 401 | Stable label | 7.39 [7.10, 7.45] | 7.72 [6.79, 7.86] |
| 401 | Two alternating widths | 26.86 [26.76, 26.93] | 23.72 [21.80, 23.98] |
| 10,001 | One label | 182.93 [173.76, 184.77] | 210.63 [190.00, 217.92] |
| 10,001 | Stable label | 180.60 [178.62, 185.26] | 208.96 [188.01, 215.70] |
| 10,001 | Ten percent of labels | 297.45 [291.44, 303.18] | 311.58 [294.35, 322.59] |
| 10,001 | Two alternating widths | 670.07 [657.75, 704.24] | 647.77 [605.84, 670.31] |
| 10,001 | 24-width sweep | 1,039.85 [999.16, 1,087.05] | 1,065.82 [1,061.57, 1,102.10] |
| 10,001 | Allocated height | 120.74 [117.56, 127.60] | 129.12 [121.16, 136.55] |

The stable large edit is about 16 percent slower. The small alternating-width
workload improves, but this does not offset regression in the targeted local-edit
path. These are shared-machine synthetic results, not confidence intervals or
application frame timings. No particular CPU cause is established by these timings
alone; allocation count is not a substitute for measuring the complete path.

## Allocation and warmed heap

A separate glibc interceptor, checked with the Rust 123-byte positive control,
observes zero allocation/reallocation calls and requested bytes in every warmed
candidate workload. The owned-vector large stable edit uses 26 calls and
1,180,032 cumulative requested bytes per update. Instrumented timings are excluded
from the CPU table.

The live allocator-usable heap for the 10,001-node fixtures is:

| Probe boundary | Owned vectors, bytes | Pooled candidate, bytes |
| --- | --- | --- |
| Initial construction and layout | 8,093,496 | 7,133,520 |
| Stable-label preparation plus 100 updates | 8,093,464 | 8,506,696 |
| Two alternating widths, 100 updates | 8,790,128 | 9,508,736 |
| 24-width sweep, 100 updates | 9,486,776 | 10,477,400 |

Initial heap is about 12 percent smaller, but warmed storage is roughly 5 to 10
percent larger. Pooling retains capacity for old and pending layout records and
for multiple capacity classes. This retention is a cost of avoiding future
allocations; it must be measured after activity rather than inferred from startup.

After the stable-label warmup, explicit release frees 1,430,736 bytes with owned
vectors and 2,410,744 bytes with the pooled candidate. Destruction frees the
remaining 6,662,728 and 6,095,952 bytes respectively. The corresponding release
and destruction totals also match the warm live-byte increases for both width
workloads. This checks ownership and reclamation at those boundaries, not a global
leak proof. Allocator rounding explains small differences between probe instances.
These measurements are neither peak live storage nor RSS/PSS.

## Next boundary

Keep the recording lifecycle and warmed-storage probes as part of validation.
The optional record block addresses empty-node metadata; read-vector allocation
churn remains. The [result-validation experiment](dependency-validation.md) checks
recorded child results without using evidence to stop propagation. The storage
measurements here isolate the record representation and exclude that experiment's
owner-input revision and explicit checkpoint copies. Any
later storage change must be compared on local edits as well as resizing, and
account for active and post-reclamation storage. Zero allocations alone is not
an acceptance criterion.

```sh
cargo run --release -p solor-core --example layout_cost_three --offline
```

The executable emits `warmed-storage`, `warmed-release`, and `warmed-destruction`
markers for external allocation probes after the CPU and initial-storage runs.
