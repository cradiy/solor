# Dependency Recording Experiment

## Scope and status

The private layout engine records child results consumed by each measurement and
layout result. Ancestor invalidation, traversal, and publication behavior remain
conservative. Records feed an explicit [result audit](dependency-validation.md);
they do not skip work in the normal update path. The representation has substantial measured
CPU and memory overhead and is not selected for a public runtime.
The current [storage representation](record-storage-evaluation.md) puts retained
read sets in an optional per-node block. The inline-record measurements below
remain the recording-cost baseline, not measurements of that smaller representation.

Acceptance covers separate query records, cache-hit reads, replacement of conditional
reads, eviction, successful versus failed callbacks, atomic layout publication,
bounded recording, and comparison with the same unrecorded workloads. There are
no public-core changes, reverse dependency indexes, automatic dependency validation,
field-level dependencies, or automatic propagation cuts.

## Ownership and validity

| Result | Record owner | Lifetime |
| --- | --- | --- |
| Measured metrics for a query | That exact measurement-cache slot | Installed after a successful callback and output validation; removed with invalidation or eviction |
| Published local layout | The node's published layout record | Replaced together with geometry after the complete scene attempt succeeds |
| Unpublished local layout | Its journal entry | Discarded if any later operation fails or panics |

`Read::Measure` contains the direct child's identity, full query, and returned
`Metrics`. `Read::Layout` contains the child's identity, assigned size, and returned
`LayoutOutput`. The records preserve call order and repeated reads. They include
calls served from a child's cache: returning a cached value is still a dependency
of the caller. They contain direct-child observations, not copies of transitive
subtree records.

Each actual measurement builds a fresh record. Parent-cache hits preserve the
record attached to that result. Different queries have independent records.
Replacing a result replaces its entire record, removing conditional reads no
longer used. Child-cache eviction does not erase the full value already observed
by a parent. Parent-cache eviction removes that parent result's own record.

Final layout records include both measurement reads and child layout outputs.
A child reused at the same allocation keeps its published record, while its
parent records the returned output. Child placement is performed by the parent;
it is not an additional value read from the child.

Measurement-cache validity and published-layout validity have different boundaries.
A successful measurement may remain cached even if a later layout fails. Its
record describes those measured metrics, not the old published geometry. Dirty
measurement entries are not exposed as valid evidence. Published layout records
remain attached to the previous geometry until the entire attempt succeeds,
including when an earlier subtree finishes and a later sibling fails. A dirty
node's old published record is historical evidence, not proof that a new layout
request is already valid.

The existing dirty flags and cache keys govern validity. Topology is locked after
initial layout, and input edits invalidate affected ancestors before user mutation.
Explicit audits additionally capture an owner-input revision and match the exact
request. Future reuse must preserve those gates and topology validity under the
[dependency-boundary contract](layout-dependencies.md).

## Bounds and failure behavior

A record holds at most 16,384 successful reads. Exceeding the experimental limit
drops its read buffer and marks the record incomplete; later reads are not stored.
Execution and geometry remain correct because recording does not control traversal.
An incomplete record is never equivalent to a complete empty record. The audit
returns `Unverifiable` rather than inferring independence from incomplete evidence.
A later shorter execution can replace it with a complete record.

Each node retains at most one record per measurement-cache slot and one published
layout record, plus temporary records during execution. Empty leaf records allocate
neither a read buffer nor a record block. An empty record is inferred only for a
valid cached measurement or published layout. Nonempty or incomplete records
require a block; the block is released when all its records are complete and empty.
This bounds per-record growth and
historical accumulation; it is not a global byte budget. Repeated callback reads
consume the limit even if they return equal values.

Errors, invalid outputs, unplaced children, and caught child failures cannot install
partial layout records. Panic unwinding drops pending records with the existing
journal guard. `release_scratch` reclaims temporary journal and child-state capacity
while preserving retained evidence with its corresponding results. Scene destruction
releases both evidence and temporary storage.

## Tests

Five focused recording tests supplement the dependency counterexamples:

- Different parent queries retain distinct evidence through cache hits and child
  eviction; parent eviction removes only its own result. Reused child layouts are
  still observed, and scratch release preserves retained records.
- A conditional branch replaces the old selected-child read; even a prewarmed
  child cache hit appears in the new record. Leaf evidence is complete and empty.
- Measurement errors and panics expose no partial record; retries install the new
  selected-child observation.
- A later sibling's error or panic preserves an earlier subtree's old published
  record and geometry until a successful retry.
- Budget exhaustion is explicit, releases its buffer, and recovers after a shorter
  execution.

The caught-child-failure test also checks that no first-frame layout record is
published. Existing geometry oracles, baseline and overflow assertions, and scratch
cleanup tests continue to pass.

## Complete-path cost

The unrecorded baseline is the scratch-reuse engine at `cdd5b86` plus the same
stable-label workload described in [layout-dependencies.md](layout-dependencies.md).
Both executables use three cache slots, the same fixtures and edits, and release
builds with Rust 1.100.0 on Linux x86-64, AMD Ryzen AI 7 H 350. Three sequential
pairs use baseline/recorded, recorded/baseline, baseline/recorded order. Dependency
statistics are collected after timing. All six runs agree on final geometry
fingerprints, queries, cache hits, measure/layout bodies, and publication counts.

Microseconds per update, median of three run medians with their range:

| Nodes | Update | Without recording | With recording |
| --- | --- | --- | --- |
| 401 | Stable label | 5.59 [5.45, 5.86] | 7.35 [7.27, 8.21] |
| 401 | Two alternating widths | 16.74 [16.50, 17.39] | 26.71 [26.62, 26.94] |
| 10,001 | One label | 140.88 [136.99, 143.03] | 190.05 [187.58, 197.35] |
| 10,001 | Stable label | 139.12 [136.35, 141.59] | 184.67 [180.05, 194.73] |
| 10,001 | Ten percent of labels | 224.27 [217.05, 228.97] | 305.36 [303.64, 317.51] |
| 10,001 | Two alternating widths | 453.95 [451.38, 465.95] | 681.96 [667.95, 691.72] |
| 10,001 | 24-width sweep | 706.13 [702.01, 755.55] | 1,049.88 [1,041.00, 1,071.61] |
| 10,001 | Allocated height | 99.58 [99.40, 100.40] | 123.74 [123.15, 139.62] |

Clean requests and ink-only edits still avoid layout and recording. The stable
large edit remains at 5,009 queries: recording adds work without reducing traversal.
These are synthetic observations on a shared machine, not application timings.

Separate allocation instrumentation, verified by the Rust positive control,
shows that warmed large single-label and stable-label updates now have 26
allocation/reallocation calls and 1,180,032 cumulative requested bytes per update,
versus zero without recording. Alternating widths has 4,564 calls and 2,097,216
requested bytes. Fresh per-result vectors, their growth, and replacing old records
reintroduce allocation traffic despite reusable geometry scratch.

| Nodes | Baseline live usable heap bytes | Recorded live usable heap bytes |
| --- | --- | --- |
| 401 | 151,640 | 296,984 |
| 10,001 | 4,219,288 | 8,093,496 |

These fixture construction probes include initial layout. The large fixture adds
about 3.69 MiB, roughly 92 percent of its baseline retained heap. Destruction
releases the measured construction increases. These are allocator-usable bytes,
not RSS/PSS or peak temporary memory.

After the large stable-label workload, records contain 35,000 reads and reserve
2,029,824 bytes of read-buffer capacity, with no incomplete records. The rest of
the heap difference also includes per-node cache/record metadata, larger journal
entries, and allocator rounding. Repeated-width and width-sweep workloads retain
47,500 and 60,000 reads respectively as additional query results remain cached.

## Decision

The lifecycle contract passes the current tests. This vector-per-result format
remains a semantic prototype; its measured cost is unacceptable as a presumed
performance improvement. The [storage evaluation](record-storage-evaluation.md)
finds that a shared buffer pool removes warmed allocations but worsens large local
updates and warmed retained memory, so that candidate is not selected. The optional
per-node block reduces retained metadata with overlapping CPU measurement ranges
and unchanged warmed allocation counts. It preserves independent query lifetimes
and atomic layout publication. No propagation speedup or final public dependency
API is established here.
