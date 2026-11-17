# Selective Dependency Checks

## Status and boundary

The private experiment can select dependency reads reached by recent input edits.
It avoids scanning a large parent's entire record for a local edit. It preserves
the [linear audit](dependency-validation.md) as a reference and leaves normal
ancestor invalidation and geometry publication unchanged.

The mechanism is retained as an explicit experiment, not selected for automatic
runtime validation. It reduces audit work for stable local updates. The
full-recapture reference measured below rebuilds indexes on changed results and
resizes. The current [checkpoint refresh](checkpoint-refresh.md) reuses indexes
when the ordered child reads match and measures that cost separately.
No public-core API, propagation pruning, or new layout algorithm is introduced.

## Ownership and validity

`capture_selective_dependencies` captures the same valid result as the linear
API, plus a child-to-read index and a scene edit cursor. The index uses one hash
entry per observed direct child and a flat linked-index array over the original
reads. Different queries and repeated reads of the same child remain distinct.
There is no per-child vector allocation or global reverse dependency graph.

The first selective capture enables a scene-local journal. It retains at most
256 edited node handles and a monotonic edit revision. Layout edits are recorded
before the mutation callback; a panic cannot hide a partial input change. Paint
edits and speculative audit invalidation do not enter the journal. Revision
exhaustion rejects mutation. Scenes without selective captures do not allocate
this history. Once enabled, it remains bounded for the scene's lifetime.

Checking follows these steps:

1. Apply the existing owner-input, exact-request, handle, and complete-record gates.
2. Require every edit since this checkpoint's cursor to remain in the journal.
   Missing history returns `Unverifiable` without executing children.
3. Walk each edited node's parent chain to identify the checkpoint owner's direct
   child, ignoring edits outside that subtree. Deduplicate these direct children.
4. Look up only their recorded reads. Sort selected indices to preserve original
   execution order, including cache eviction and failure order.
5. Replay and compare complete metrics or layout outputs using the existing audit.

Only `Unchanged` advances the checkpoint's edit cursor. Its original outputs
remain the comparison values: equality establishes that they still hold at the
new cursor. `Changed`, `Unverifiable`, errors, and panics retain the old cursor.
Normal updates and scratch reclamation do not consume history on behalf of other
checkpoints. A dormant checkpoint can expire while an actively checked one stays
valid. This bounded-history policy is an experiment limit, not a UI frame limit.

After normal layout, `refresh_selective_dependencies` can install current
evidence into an existing checkpoint. Unavailable evidence leaves it intact;
changed child order rebuilds the index. Refreshing is separate from advancing an
audit cursor after equality.

Selection costs scale with edits times tree depth, affected recorded reads, and
sorting those reads. History is capped at 256 edits and each record at 16,384
reads. Complete equality still depends on the existing contract: layout-affecting
changes use the edit boundary, topology is locked, and callbacks compute from
declared inputs and child results.

## Publication and correctness

The audit discards speculative geometry and leaves pending layout work intact.
Even a subsequent check that selects zero reads cannot clear an internal child
movement. Normal layout still publishes that movement atomically. This preserves
changes but does not reuse speculative geometry: some layout bodies execute twice.

Six focused tests cover deep routing, duplicate edits in one branch, unread
branches and external subtrees, exact queries after cache eviction, history
expiry and cursor advancement, independent checkpoints, incomplete evidence,
owner mutation, foreign handles, and ordered error/panic recovery. A 384-step
mixed workload compares both audit verdicts and complete geometry with linear
validation and cache-disabled recomputation, using both cache capacities.

## Full-recapture reference measurements

The full-recapture reference at `e111a54` uses the shared
[workload](../crates/solor-core/examples/layout_cost/selection.rs) and
compares three paths: normal layout alone, linear audit followed by normal layout,
and selective audit followed by normal layout. Every timed update includes edits,
history maintenance when enabled, both root audits, reconciliation, publication,
and checkpoint replacement when either result is changed or unverifiable.
Initial scene/checkpoint construction is outside timing and measured separately
for storage. Snapshots and count aggregation are outside timing.

All fixtures start with 80-unit labels and a fixed allocated height of 100,000.
Stable edits toggle 80/81 units without changing geometry. Changing edits toggle
a selected label between 10/100 units after its first edit. Resize alternates
widths 96/120 and requires fresh checkpoints for the new requests. Each sample
rotates the order of the three paths. There are 30 warmup updates, seven samples,
300 iterations per sample for 401 nodes and 30 for 10,001 nodes.

The following release three-slot results are medians of three process medians,
with the range of those process medians in brackets, in microseconds per complete
update. Runs are sequential; allocation instrumentation is disabled for timing.

| 10,001-node update | Normal | Linear audit + normal | Selective audit + normal |
| --- | --- | --- | --- |
| One stable label | 180.22 [177.79, 186.06] | 266.20 [265.32, 268.10] | 181.43 [180.31, 183.62] |
| 250 stable labels | 267.20 [267.18, 274.77] | 418.72 [406.23, 423.55] | 400.75 [386.69, 417.69] |
| One changing label | 180.36 [179.96, 187.86] | 230.01 [223.85, 230.92] | 388.14 [383.75, 391.77] |
| Alternating width | 752.90 [736.73, 756.99] | 745.42 [738.10, 757.03] | 954.46 [953.97, 968.45] |

For 401 nodes, the stable single-label medians are 7.31, 10.66, and 7.85 us;
the changing-label medians are 7.27, 9.31, and 16.22 us respectively. These are
headless synthetic workloads, not real text, paint, frame, or application results.

For the 10,001-node stable single-label update, selection checks 3 recorded reads
instead of 7,500 and stages 4 entries instead of 2,503. Both audit paths still
execute five layout bodies across audit and reconciliation, versus three for
normal layout alone. With 250 stable edits, selection checks 750 records instead
of 7,500, but the complete CPU ranges overlap the linear audit's range.

Changed results need new copied evidence and newly built indexes. Resize requests
are rejected before child replay, yet still pay for those replacements after
normal layout. This is why reduced read counts do not imply reduced update cost.
The stable single-label path approaches normal layout cost; no improvement over
normal layout is established.

## Full-recapture reference storage

An external allocator probe measures release builds separately from CPU timing.
The positive Rust control observes one 123-byte request and 136 usable bytes.
The figures below cover one warmed complete update, including any checkpoint
replacement. Requested bytes are cumulative allocation traffic, not peak memory.

| 10,001-node update | Normal calls / requested bytes | Linear calls / requested bytes | Selective calls / requested bytes |
| --- | --- | --- | --- |
| One stable label | 26 / 1,180,032 | 28 / 1,180,608 | 32 / 1,180,840 |
| 250 stable labels | 773 / 1,371,264 | 1,273 / 1,515,264 | 1,304 / 1,562,424 |
| One changing label | 26 / 1,180,032 | 28 / 1,540,032 | 56 / 2,010,016 |
| Alternating width | 5,008 / 2,225,088 | 5,010 / 2,585,088 | 5,034 / 3,054,840 |

After construction and 100 stable batch updates, retained live usable heap is
7,313,872 bytes for normal layout, 7,673,808 for linear auditing, and 7,942,744
for selective auditing. The latter includes the full bounded journal, checkpoint
copies, and indexes. `release_scratch` reclaims 1,430,736 bytes in each case;
remaining evidence and history stay valid. Subsequent destruction releases the
remaining measured heap in all three cases. This checks these ownership boundaries,
not global leak freedom. The figures are neither RSS/PSS nor peak storage, and
small differences can reflect allocator rounding.

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-core --example layout_cost_three --offline
cargo run --release -p solor-core --example layout_cost_one --offline
```

`selection-storage`, `selection-update`, `selection-warmed`, `selection-release`,
and destruction markers delimit allocation probes after timing. The one-slot
target runs the same correctness and complete-update workloads.

The measured decision is to keep normal layout as the default. Same-order
[checkpoint refresh](checkpoint-refresh.md) addresses redundant index rebuilding;
speculative layout work still executes again during reconciliation. Any candidate must
retain failure atomicity and be measured across stable edits, changed results,
batch updates, and resizing rather than only the favorable stable-label case.
