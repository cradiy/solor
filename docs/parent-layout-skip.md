# Retaining a Validated Parent Layout

## Scope and decision

The private headless experiment provides `Scene::run_pruned(query, assigned,
measurement, placement)`. It can retain the current root layout while publishing
changed internal geometry from audited children. Normal invalidation still marks
ancestors dirty; root measurement still executes when dirty. This is one explicit
layout boundary, not recursive pruning, automatic scheduling, or a public UI API.

The acceptance boundary is correctness against forced recomputation, complete
update costs across stable and changed results, and bounded, reclaimable storage.
The measured selective benefit is sufficient to conclude this headless milestone.
Keep normal layout as the default: failed equality checks still cost time, and
checkpoint storage remains significant. Further local optimization is not required
before investigating a different foundation concern.

## Conditions and publication

The method uses the [shared attempt](shared-layout-attempt.md): select reads,
audit affected children, then reconcile and publish in one exclusive scene update.
Both checkpoints must belong to the root. Layout reuse requires all of:

- The exact allocated size and unchanged root layout inputs.
- Complete recorded reads and enough edit history to select affected reads.
- Equal consumed measurement metrics and final layout outputs, including baseline
  and content extent, not just allocated size.
- Evidence anchored to the currently published scene layout.

The scene increments a checked publication counter only after successful layout
publication. Captured or refreshed evidence records that counter. A historical
checkpoint may become equal again after an intervening publication; equality
alone does not validate the intervening parent positions. A mismatched anchor
therefore executes normal root layout. Successful combined reconciliation anchors
unchanged evidence to the newly published result.

A standalone selective audit can advance its edit cursor while discarding staged
geometry. When it consumes selected reads successfully, it clears its publication
anchor. Otherwise a subsequent edit to another child could allow pruning to miss
the earlier child's unpublished movement. Combined reconciliation restores the
anchor only after all pending work is published.

When root layout is skipped, each audited direct child's root retains its previous
published origin. Descendants retain their newly computed internal origins.
Adopted ranges publish geometry and layout records together; the root keeps its
existing geometry and layout read set. Root measurement remains conservative and
can fail before publication. Errors or panics retain published geometry, layout
records, checkpoint cursors, and pending work for retry. Staged ranges never
survive into another update.

Changed results, owner edits, incomplete evidence, missing edit history, changed
allocations, and invalid publication anchors use normal reconciliation. A call
with no selected reads uses the normal path, including the existing clean-frame
fast path. Callbacks must compute from declared inputs and consumed child results;
layout cannot depend on untracked measurement side effects.

## Correctness

Six focused tests in
[parent_skip_tests.rs](../crates/solor-core/examples/layout_contract/parent_skip_tests.rs)
cover nonzero direct-child placement with new internal movement, repeated updates
and scratch release, historical equality against a different publication,
standalone auditing followed by another child's edit, changed final baseline or
content, changed owner inputs, incomplete layout evidence, and error/panic retry
after equal audits.

The existing 384-step mixed oracle includes the pruned path. Verdicts match linear
auditing, and every resulting geometry snapshot matches forced recomputation.
Both one-slot and three-slot release examples run this oracle. There are 82 passing
workspace tests, including 54 layout-contract tests. These checks establish the
private execution contract; they do not validate real fonts, pixels, windows, or
application frame times.

## Measurement boundaries

The [selection workload](../crates/solor-core/examples/layout_cost/selection.rs)
compares `Plain`, `Linear`, `Selective`, `Shared`, and `Pruned` in the same binary.
Plain still records dependencies but performs no audits. Shared audits and adopts
child layouts while always executing root layout. Pruned additionally retains an
equal root layout. Timing includes edits, selection, audits, measurement, layout,
publication, and checkpoint refresh when results or requests differ. Construction,
snapshots, and counter aggregation are outside timing.

Release three-slot measurements use three sequential process pairs against
`28c5073`; the second pair reverses binary order. Within the candidate, five paths
rotate sample order after 30 warmup updates. Seven samples use 300 iterations for
401 nodes and 30 for 10,001 nodes. Timing excludes allocator instrumentation.
The fixtures use synthetic wrapping labels and explicit root allocation. Stable
edits toggle content units between 80 and 81 while preserving consumed outputs;
changing edits toggle 10 and 100. Batch edits affect one tenth of rows. Resizing
alternates widths 96 and 120.

Values below are medians of three process medians, with their ranges in brackets,
in microseconds per complete update in the same candidate build.

| 10,001-node update | Plain | Shared | Pruned |
| --- | --- | --- | --- |
| One stable label | 183.90 [182.89, 217.71] | 192.19 [186.34, 219.54] | 48.14 [47.42, 50.23] |
| 250 stable labels | 287.48 [272.90, 309.17] | 358.59 [352.54, 393.32] | 219.16 [218.31, 222.64] |
| One changing label | 187.87 [187.38, 214.62] | 204.65 [193.02, 233.36] | 209.82 [199.57, 229.80] |
| Alternating width | 758.64 [753.16, 772.11] | 772.39 [741.85, 780.13] | 770.01 [762.09, 777.76] |

Against plain layout, stable single and batch updates reduce the median by about
74 and 24 percent. They skip one root layout, adopt one or 250 subtree roots,
and execute two or 500 child layout callbacks. Measurement counts stay at four
or 751. Removing one root callback saves its whole sibling traversal and read
recording, not just the callback invocation itself.

Changing-result checks cannot skip and add about 22 us (12 percent) over plain
layout at these medians. Resize adds about 11 us (1.5 percent). Ranges overlap and
the earlier validation pass observed a larger changing-result penalty; treat
these as workload observations, not a fixed overhead guarantee. For 401 nodes,
plain/pruned medians are 7.78/2.70 us for stable single edits, 10.89/8.62 us for
stable batches, 7.78/8.44 us for changing results, and 30.70/31.19 us for resizing.

Reference normal-path medians for the large fixture are 183.77, 274.06, 184.72,
and 749.84 us in the same workload order. Their ranges overlap the candidate's
normal-path ranges; this does not establish zero overhead from the new publication
counter or distinguish code-layout effects from run variability. The stable-edit
benefit is clear in the within-build comparison. Failed checks remain a reason
to keep the mechanism opt-in.

## Allocation and retained storage

A separate allocator probe passes a Rust positive control: one 123-byte request
occupies 136 usable bytes. Instrumented timings are excluded from CPU comparisons.
One warmed complete update of the 10,001-node fixture produces:

| Update | Plain calls / requested bytes | Shared calls / requested bytes | Pruned calls / requested bytes |
| --- | --- | --- | --- |
| One stable label | 26 / 1,180,032 | 30 / 1,180,264 | 18 / 394,024 |
| 250 stable labels | 773 / 1,371,264 | 804 / 1,418,424 | 792 / 632,184 |
| One changing label | 26 / 1,180,032 | 30 / 1,180,264 | 30 / 1,180,264 |
| Alternating width | 5,008 / 2,225,088 | 5,008 / 2,225,088 | 5,008 / 2,225,088 |

All four pruned updates have zero net live-heap change across their markers.
After construction and 100 stable batch updates, retained usable heap is
7,313,872 bytes plain, 7,980,696 shared, and 7,977,688 pruned. The pruned fixture
retains about 648 KiB more than plain, primarily the existing checkpoint and shared
attempt machinery; it does not add another persistent node graph. The publication
anchor is one scene counter and one optional counter per checkpoint.

Scratch release reclaims 1,430,736 bytes plain, 1,468,656 shared, and 1,465,584
pruned. Destruction then releases the remaining measured fixture storage:
5,883,136, 6,512,040, and 6,512,104 bytes respectively. These boundaries confirm
ownership and explicit reclamation for this workload, not global leak freedom.
Small usable-size differences include allocator rounding. The remaining root
measurement still allocates and visits siblings; the experiment does not make
stable updates allocation-free or proportional only to changed nodes.

## Reproduction

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-core --example layout_contract --offline
cargo run --release -p solor-core --example layout_cost_three --offline
cargo run --release -p solor-core --example layout_cost_one --offline
```

Allocation markers after timing delimit construction, warmed updates, retained
fixtures, scratch release, and destruction. Requested bytes measure allocation
traffic, not peak memory. Live usable heap measures allocator-owned storage, not
RSS or PSS. Scratch and evidence scale with bounded reads, visited nodes, and the
256-entry edit history; they do not accumulate one record per update.

This experiment establishes that complete consumed results can form a useful
layout boundary without losing child changes. It does not select a general UI
execution model or establish the framework's total resource budget. The retained
measurement traversal, recording costs, and failed-check overhead remain explicit
tradeoffs rather than reasons to extend this milestone indefinitely.
