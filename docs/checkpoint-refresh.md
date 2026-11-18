# Refreshing Selective Checkpoints

## Scope and contract

The private selective audit supports refreshing a checkpoint from a currently
valid measurement or published layout. The purpose is to avoid rebuilding the
child/read index when only queries, allocations, or consumed outputs change.
Normal invalidation, audit selection, and atomic geometry publication are unchanged.
Standalone audits discard speculative layout and may execute it again during
reconciliation. The separate [shared-attempt path](shared-layout-attempt.md)
adopts matching staged subtrees within one complete update.

`Scene::refresh_selective_dependencies(checkpoint, target)` keeps the existing
owner and applies the same result-validity gates as capture. It returns `false`
for unavailable or dirty evidence and an error for an invalid owner. Either
outcome leaves the checkpoint, index, and edit cursor unchanged. No callbacks run.

For available evidence, refresh compares the ordered child handles with the
checkpoint's reads. If the lengths and every handle match, the existing index is
valid regardless of query kinds or values, layout allocations, metrics, or content
outputs. Those complete read values are copied into the existing buffer. This
path scans and copies the record but allocates no new index or read storage.

If a handle, read count, or ordering differs, refresh creates new evidence and a
new index. This handles conditional reads and avoids keeping the previous large
capacity after a smaller record replaces it. Empty and incomplete records remain
distinct. Finally, refresh installs the new request, owner-input revision,
completeness flag, and current edit cursor. It does not mark pending layout work
complete or change published geometry.

The experiment runner refreshes both selective checkpoints after normal layout
when either audit is changed or unverifiable. Initial capture still constructs
an index. Linear auditing continues to capture independent evidence as the
comparison path. There is no dependency sharing between nodes or checkpoints.

## Correctness

Three focused tests cover updated comparison values and requests with identical
child order, conditional child replacement and read-count changes, complete empty
versus incomplete evidence, and rejected refresh after a layout error or panic.
Foreign-scene and unavailable-request failures retain the original evidence.
The existing 384-step mixed workload now exercises refresh and compares audit
verdicts and complete geometry with linear auditing and forced recomputation in
both cache-capacity variants.

## Complete-path measurements

The release three-slot executable is compared with the full-recapture reference
at `e111a54`. Both binaries run the same four update workloads, with identical
geometry and work-count checks. There are three sequential before/after pairs;
the second pair reverses binary order. Each executable retains the existing
seven-sample rotation among normal, linear-audit, and selective-audit paths.

Timing includes edits, audit selection and replay, normal reconciliation,
publication, and checkpoint replacement or refresh. Initial capture, snapshots,
and count aggregation remain outside timing. Values below are medians of three
process medians with their range in brackets, in microseconds per complete
selective-audit update.

| Nodes | Update | Full recapture | Refresh |
| --- | --- | --- | --- |
| 401 | One stable label | 8.29 [7.76, 8.33] | 7.70 [7.63, 8.00] |
| 401 | Ten stable labels | 15.31 [15.05, 15.45] | 14.80 [14.72, 14.84] |
| 401 | One changing label | 16.38 [16.07, 17.37] | 7.97 [7.87, 8.18] |
| 401 | Alternating width | 37.38 [36.57, 37.43] | 29.19 [28.81, 29.56] |
| 10,001 | One stable label | 196.47 [180.10, 196.68] | 179.23 [178.70, 184.81] |
| 10,001 | 250 stable labels | 390.48 [378.55, 398.47] | 376.56 [374.20, 377.52] |
| 10,001 | One changing label | 397.33 [377.27, 407.62] | 187.30 [182.67, 187.45] |
| 10,001 | Alternating width | 944.27 [918.79, 948.18] | 728.68 [720.10, 734.67] |

Stable-label updates do not refresh their checkpoints. Their variation, and
variation in the normal-layout controls, should not be attributed to index reuse.
The directly affected changing-label and resize cases show a consistent reduction.

In the refresh executable, normal layout alone takes median 171.81 us for the
large changing-label case and 719.03 us for resizing. Selective validation still
adds work. Large stable batch updates take 376.56 us with validation versus
262.83 us without it: index refresh does not address repeated layout bodies or
all the costs of routing many edits. No application/frame performance claim or
advantage over normal layout is established.

## Allocation and retained storage

Separate allocator-instrumented runs use the same update markers. The Rust
positive control observes one 123-byte allocation request with 136 usable bytes.
Figures below are calls and cumulative requested bytes for one warmed complete
10,001-node update, not peak memory.

| Update | Full recapture calls / bytes | Refresh calls / bytes |
| --- | --- | --- |
| One stable label | 32 / 1,180,840 | 32 / 1,180,840 |
| 250 stable labels | 1,304 / 1,562,424 | 1,304 / 1,562,424 |
| One changing label | 56 / 2,010,016 | 30 / 1,180,264 |
| Alternating width | 5,034 / 3,054,840 | 5,008 / 2,225,088 |

Each refreshing update avoids 26 calls and 829,752 requested bytes in these
fixed-read-order fixtures. Normal layout and audit callbacks still allocate;
the complete update is not allocation-free. Frequently changing conditional read
structure still requires rebuilding and has no established performance gain.

After construction and 100 stable batch updates, retained usable heap measures
7,942,952 bytes for full recapture and 7,942,776 for refresh. Both release
1,430,736 bytes through `release_scratch`; destruction releases the remaining
measured heap. These small retained-heap differences reflect allocator rounding
and do not establish a storage reduction. The indexes and evidence still consume
memory while their checkpoints live. Measurements are not RSS/PSS or peak storage.

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-core --example layout_cost_three --features layout-audit --offline
cargo run --release -p solor-core --example layout_cost_one --features layout-audit --offline
```

Keep same-order refresh in the explicit experiment. The
[shared-attempt experiment](shared-layout-attempt.md) measures descendant layout
reuse while preserving atomic publication. The separate
[parent-layout skip](parent-layout-skip.md) evaluates root layout pruning with
publication-anchored evidence. Normal layout remains the default.
