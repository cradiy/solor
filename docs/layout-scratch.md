# Layout Scratch Reuse

## Boundary and ownership

This experiment reuses temporary storage in the private headless layout engine.
It leaves measurement queries, caches, ancestor invalidation, traversal, and
publication semantics unchanged. Acceptance requires equivalent geometry and work
counts, no stale scratch after failure, measured allocation reduction, and an
explicit account of retained capacity and reclamation. It does not establish a
public UI API or an application memory policy.

Each `Scene` owns two vectors:

- A journal of geometry awaiting publication. Entries use stable node IDs.
- A stack of child placement states for active layout callbacks. Each callback
  appends its direct-child range and truncates it when leaving, including during
  unwinding. Sibling callbacks reuse that space.

Contexts retain indexes rather than references across recursive calls, since
growth can relocate either vector. New child ranges are initialized before use.
The attempt guard clears both vectors on success, error, and panic. Only capacity
survives: failed drafts cannot be published by a later attempt. A parent catching
a child error or panic still cannot complete that attempt successfully.

The journal's live length is bounded by visited nodes; the child stack's live
length by the sum of child counts on the active path. Retained capacity follows
the largest demand and vector growth policy, not the most recent edit. The
prototype locks topology after successful layout, so repeated updates cannot
accumulate additional node identities or child ranges. Capacity is linear in
the fixture size, not a fixed byte budget.

`Scene::release_scratch()` drops both allocations without changing dirty flags,
cached measurements, pending work, or published geometry. Exclusive access to the
scene prevents release during a callback. Clean requests after release allocate
nothing; the next actual layout rebuilds scratch as needed. Release is explicit,
with no timer, idle detection, or claim that freeing heap blocks returns RSS to
the operating system. Dropping the scene also releases retained capacity.

## Correctness

The layout-contract tests include a wide branch that grows the shared child stack
while its parent range remains active, followed by a narrow sibling. Repeated
errors and panics after partial layout leave both scratch lengths zero and retain
the prior published snapshot. Release while work is pending preserves the retry;
release after success preserves the clean fast path. A subsequent real edit
rebuilds storage and agrees with cache-disabled recomputation.

The existing sizing, partial-publication, caught-failure, and incremental oracle
checks also run with reusable scratch. Both cache-capacity workload targets
compare 48 steps of each workload with cache-disabled recomputation before timing.

## CPU measurements

The baseline is `29569b5` with three cache slots and fresh scratch per attempt.
The candidate uses the same three-slot workload and timing boundary described in
[layout-cost.md](layout-cost.md). Release builds use Rust 1.100.0 on Linux x86-64,
AMD Ryzen AI 7 H 350. Sequential runs alternate baseline/candidate,
candidate/baseline, baseline/candidate. Reclamation probes execute after all timed
workloads, outside the CPU measurements. All six runs agree on final fingerprints,
queries, cache hits, measure/layout bodies, and changed-publication counts.

Microseconds per update, median of three run medians with their range:

| Nodes | Update | Fresh scratch | Reused scratch |
| --- | --- | --- | --- |
| 401 | One label | 5.82 [5.80, 6.00] | 5.79 [5.51, 5.98] |
| 401 | Ten percent of labels | 8.59 [8.37, 8.87] | 8.35 [8.27, 8.50] |
| 401 | Two alternating widths | 17.71 [17.53, 18.04] | 17.12 [16.80, 17.25] |
| 401 | 24-width sweep | 28.11 [27.51, 28.49] | 27.20 [26.55, 27.72] |
| 401 | Allocated height | 4.01 [4.00, 4.01] | 4.00 [3.68, 4.03] |
| 10,001 | One label | 147.92 [144.94, 149.89] | 144.08 [142.98, 144.89] |
| 10,001 | Ten percent of labels | 230.74 [228.79, 233.15] | 226.93 [225.28, 227.40] |
| 10,001 | Two alternating widths | 487.14 [478.48, 491.11] | 459.26 [458.34, 464.39] |
| 10,001 | 24-width sweep | 754.16 [753.53, 754.96] | 725.46 [722.44, 730.11] |
| 10,001 | Allocated height | 104.39 [103.95, 109.05] | 108.35 [108.02, 109.83] |

CPU improvements are modest, not a proportional consequence of eliminating
allocations. The large height-only workload is about four percent slower in this
sample, with overlapping run ranges. Small differences on this shared machine
are not a universal speedup claim. Clean and ink-only requests still do no layout;
their few-nanosecond timings do not support a useful ranking.

## Allocation and retention measurements

The separate glibc interceptor and Rust positive control described in the cost
experiment are used again. Instrumented timings are excluded. For both fixture
sizes, every measured warmed workload has zero allocation/reallocation calls,
zero requested bytes, and zero net live-heap change with reuse. In the large
fixture, the baseline single-label update has 13 calls and 453,096 cumulative
requested bytes; alternating widths has 2,290 calls and 1,796,544 bytes.
This zero-allocation observation applies to warmed synthetic updates, not scene
construction, arbitrary node behaviors, or the first update after release.

| Nodes | Baseline live usable bytes | Reuse live usable bytes | Retained difference |
| --- | --- | --- | --- |
| 401 | 122,216 | 151,688 | 29,472 |
| 10,001 | 3,312,904 | 4,219,352 | 906,448 |

Construction includes initial layout. The large fixture retains about 885 KiB
more, roughly 27 percent of its baseline heap. These are allocator-usable live
bytes, not peak temporary usage or RSS/PSS. Vector and allocator rounding matter.
Both destruction probes release their complete live-byte increases.

For the large fixture, explicit release frees 906,448 live usable bytes with no
allocation. The next clean request allocates nothing. A single-label edit after
release uses 13 allocation/reallocation calls, cumulatively requests 573,024 bytes,
and retains 316,624 bytes. A second release frees those 316,624 bytes; scene
destruction frees the remaining 3,312,904 bytes. Rebuilding after release therefore
has a real cost, and releasing after every frame defeats reuse.

## Decision

Keep reuse as a private experiment with explicit reclamation. The strongest
result is removing allocator traffic during warmed updates; total CPU gains are
small and retained memory is substantial. This does not select a default policy
for a future application runtime. Idle scheduling and a byte-budgeted retention
policy require workloads beyond this prototype.

The remaining local-edit cost includes visiting the root column's entire child
list: a single-label edit still issues 5,009 queries in the large fixture. The
[dependency-boundary investigation](layout-dependencies.md) covers container
invalidation and placement dependencies, including edits that preserve a child's
size versus edits that move later siblings. Scratch reuse alone cannot reduce
that work.

```sh
cargo test -p solor-core --example layout_contract --example layout_cost_one --example layout_cost_three --features layout-audit --offline
cargo run --release -p solor-core --example layout_cost_three --features layout-audit --offline
```
