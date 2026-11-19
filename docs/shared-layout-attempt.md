# Sharing Audited Layout Work

## Scope

The private headless experiment can execute root dependency checks and normal
layout in one unpublished attempt. A completed subtree from the audit can supply
the normal layout result without executing its layout callbacks again. Parent
callbacks still execute and normal ancestor invalidation remains intact. This
is an explicit experiment path, not propagation pruning or a public UI API.

`Scene::run_validated(query, assigned, measurement, placement)` accepts two root
selective checkpoints and an explicit root allocation. Non-root or foreign
checkpoint owners are rejected before work. Query, allocation, input-revision,
completeness, and history gates retain their meanings. If neither audit selects
a read, the method uses normal layout, including its clean-frame fast path.
Unverifiable evidence never licenses reuse.

The method does not accept arbitrary nested audit sequences, input edits between
phases, or implicit root allocation. Its verdicts do not skip the root callback.

## Lifetime and publication

Selected reads are determined before either audit. The root is marked dirty,
and one scratch guard owns auditing and normal layout. Exclusive scene access
lasts throughout; behaviors retain the existing contract of computing from
declared inputs and child results.

During auditing, completed dirty subtrees are indexed by node identity and their
contiguous draft range. Audits do not reuse other staged results. Each direct
child can be laid out once per recorded parent callback, and the two root audit
phases cannot traverse overlapping layout roots. These restrictions keep ranges
valid without a persistent cross-frame cache or a graph of draft versions.

During normal layout, a staged result is adopted only for the same node and exact
allocated size. Descendant geometry and layout records stay together. The parent
still supplies the subtree root's placement. A different allocation executes
layout again; matching descendants can still be adopted independently. Input
changes cannot occur between staging and adoption through this API.

Every draft has a scratch-owned adoption flag. Audit drafts begin unselected;
adopting a subtree selects its range. Fresh normal-layout drafts are selected as
they complete. Only selected drafts publish after root success. Obsolete geometry
from rejected allocations is discarded, even when part of that old subtree is
adopted into a newly computed parent.

Errors and panics discard drafts, flags, and the candidate index. Published
geometry and layout records remain intact. Successful measurements may remain
cached as before, but pending layout work is not cleared. Neither checkpoint
cursor advances until the entire update succeeds; on success, only `Unchanged`
checks advance. Other checkpoints can be refreshed from the resulting state.

The guard clears reuse state on every exit. Staged results never survive into
another update. Capacity is retained; `release_scratch` also releases the index
and flags. Ordinary updates do not allocate these additional buffers unless a
shared attempt has used them. Storage scales with visited drafts and staged dirty
nodes, not update history. Normal layout still traverses unaffected siblings.

`ValidatedRun` reports changed published entries, the two audit reports, and
adopted subtree roots. A root can avoid several descendant callbacks, so
`reused_layouts` is not a count of all skipped callbacks.

## Correctness

Four combined-path tests cover internal movement behind unchanged outputs,
clean subsequent updates, scratch release, errors and panics during auditing or
after adoption, cursor preservation, missing history, new root requests, and
invalid checkpoint owners.

A lower-level attempt test changes allocation after staging. It recomputes the
parent and wrapping text, adopts a matching drawing at its new origin, and
compares publication with forced recomputation. This checks obsolete drafts and
buffer growth directly and runs in the contract and both cache-capacity targets.

The existing 384-step mixed oracle also includes shared attempts: verdicts match
linear auditing and geometry matches cache-disabled recomputation. Both one-slot
and three-slot release examples complete. The [layout contract](layout-contract.md)
describes the current correctness coverage.

## Complete update cost

The [selection workload](../crates/solor/examples/layout_cost/selection.rs)
adds `Shared` alongside `Plain`, `Linear`, and `Selective`. Both selective paths
retain checkpoint refresh. Timing includes edits, selection, audits, layout,
publication, and required refresh. Construction, snapshots, and counter
aggregation remain outside timing.

Release three-slot measurements use three sequential process pairs with the
`fd50e6b` reference; the second pair reverses binary order. Within the candidate,
sample order rotates among four paths. Each has 30 warmup updates and seven
samples: 300 iterations for 401 nodes and 30 for 10,001 nodes. CPU runs exclude
allocator instrumentation. Values below are medians of process medians, with
their ranges in brackets, in microseconds per update in the same candidate build.

| 10,001-node update | Plain | Separate selective audit + layout | Shared attempt |
| --- | --- | --- | --- |
| One stable label | 188.22 [180.03, 213.18] | 191.85 [190.08, 221.92] | 185.28 [184.65, 192.12] |
| 250 stable labels | 281.58 [274.01, 301.17] | 403.18 [389.51, 433.35] | 357.53 [354.91, 358.62] |
| One changing label | 189.82 [187.02, 221.69] | 214.15 [208.69, 235.30] | 201.30 [199.86, 207.09] |
| Alternating width | 774.14 [767.22, 785.63] | 785.84 [778.23, 789.37] | 793.90 [770.27, 798.95] |

Stable batch updates reduce total layout callbacks from 1,001 to 501, matching
plain layout, and adopt 250 subtree roots. Complete time is about 11 percent
lower than separate selective auditing, but still exceeds plain layout. One
stable-label update adopts a row and reduces callbacks from five to three; timing
ranges overlap, so a CPU gain is not established there.

Changing-label checks stop on measurement differences before staging layout;
resize requests fail the request gate. Neither case adopts subtrees. Their timing
differences cannot be attributed to subtree reuse. For 401 nodes, stable batch
medians are 11.26 us plain, 15.34 us separate, and 14.28 us shared.

Normal-path control medians in the reference are 182.40 us for a large stable
single-label update and 273.75 us for a stable batch, compared with 188.22 and
281.58 us in the candidate. Ranges overlap and timings vary across runs. This
does not establish zero overhead on the ordinary path or an advantage over plain
layout. Promotion beyond the experiment needs further complete-path evaluation.
No pixel, real-text, frame, or application performance claim is made.

## Allocation and retained capacity

Separate allocation probes use a Rust positive control that observes one
123-byte request and 136 usable bytes. Figures below cover one warmed complete
10,001-node three-slot update. Requested bytes are cumulative traffic, not peak
storage.

| Update | Separate calls / requested bytes | Shared calls / requested bytes |
| --- | --- | --- |
| One stable label | 32 / 1,180,840 | 30 / 1,180,264 |
| 250 stable labels | 1,304 / 1,562,424 | 804 / 1,418,424 |
| One changing label | 30 / 1,180,264 | 30 / 1,180,264 |
| Alternating width | 5,008 / 2,225,088 | 5,008 / 2,225,088 |

After construction and 100 stable batch updates, live usable heap measures
7,942,840 bytes separately and 7,980,744 bytes shared, about 37 KiB more. Scratch
release reclaims 1,430,736 and 1,468,656 bytes respectively; destruction releases
the remaining measured storage. These boundaries check ownership and explicit
reclamation, not global leak freedom. Small residual differences reflect allocator
rounding. The figures are neither RSS/PSS nor peak memory.

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor --example layout_contract --features layout-audit --offline
cargo run --release -p solor --example layout_cost_three --features layout-audit --offline
cargo run --release -p solor --example layout_cost_one --features layout-audit --offline
```

Keep shared attempts as an explicit experiment: repeated subtree layout is
eliminated without losing publication atomicity. The separate
[parent-layout skip](parent-layout-skip.md) evaluates retaining the current root
layout when its inputs, allocation, and consumed results are unchanged, while
publishing descendant movement and preserving prior placement decisions.
