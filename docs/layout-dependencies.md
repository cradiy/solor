# Layout Dependency Boundary

## Status and scope

This milestone establishes counterexamples and a stable-geometry workload before
changing invalidation. The engine still invalidates ancestors conservatively.
The separate [recording experiment](dependency-recording.md) implements per-result
read evidence while keeping that traversal. The tests and measurements here define
what ancestor pruning must preserve; they are not evidence of an implemented speedup.

The boundary is the private headless layout example, with locked topology and
explicit input edits. Public widget APIs, signals, rendering, topology mutation,
automatic external-resource tracking, and a general dependency engine are outside
this investigation.

## Executable counterexamples

The [dependency tests](../crates/solor-core/examples/layout_contract/dependency_tests.rs)
exercise three distinct reasons that equal width and height are insufficient:

| Case | Unchanged observation | Required work |
| --- | --- | --- |
| Wrapping label changes from 20 to 21 cells | Complete `Exact(18)` metrics, including baseline | Natural width changes; the naturally sized parent row grows from 72 to 74 |
| Leaf reports a different final baseline or content extent | Complete measured metrics and allocated size | Parent baseline or content extent must change |
| Fixed-size container moves its child from x=0 to x=7 | The container's entire published geometry and its parent's geometry | The child's new relative origin must still be published |

The natural-width case also fills the three-slot measurement cache with other
queries before the edit. Eviction does not remove the parent's dependence on a
previous Natural result. A cache of recent measurements therefore cannot serve
as a complete dependency record.

The final-output case independently changes the baseline from 4 to 8 and content
from 12-by-10 to 120-by-20. The parent's allocated size stays 100-by-10. Checking
only the measurement, or only the allocated size, misses these changes.

All cases compare the resulting geometry with cache-disabled recomputation.
Existing height-changing, baseline-only, sibling-placement, failed-attempt, and
panic-retry tests remain required for any future propagation change.

## Stable-geometry workload

`StableLabel` uses the existing 401-node and 10,001-node icon/label/button fixtures.
Before timing, all labels are set to 80 cells and the scene is laid out. Each
update toggles one selected label between 80 and 81 cells, reading its current
value so repeated visits still change the input. Labels occupy the same number
of lines at the assigned widths. Natural label width changes, while the row's
constrained result and all published geometry remain identical.

Preparation and fingerprints are outside timing. Edits, invalidation, measurement,
layout, and publication checks are included. Each timed workload asserts zero
changed-publication entries and an unchanged final geometry fingerprint. The
small reference run additionally checks geometry after every step against full
recomputation and the initial snapshot.

Three sequential release runs use Rust 1.100.0 on Linux x86-64, AMD Ryzen AI 7 H
350, three measurement-cache slots, and reusable scratch. The sampling procedure
is the same as [layout-cost.md](layout-cost.md). Values are microseconds per
update: median of three run medians, with their range.
This baseline has scratch reuse and no dependency recording; recording overhead
is measured separately in the linked experiment.

| Nodes | General one-label edit | Stable-geometry label edit | Queries per update in both |
| --- | --- | --- | --- |
| 401 | 5.67 [5.43, 5.92] | 5.68 [5.60, 5.76] | 209 |
| 10,001 | 142.61 [138.86, 145.68] | 141.61 [141.36, 146.57] | 5,009 |

Both workloads execute four measurement bodies and three layout bodies per
update. The general edit changes an average of 43.6 or 890.8 published entries;
the stable edit changes zero. This comparison isolates unnecessary work in the
current engine; it is not a comparison with an optimized implementation. The
shared machine and cheap synthetic metrics limit conclusions about wall time.

A separate allocation probe, checked with the 123-byte Rust positive control,
observes zero warmed allocation calls, requested bytes, and net live-byte change
for the stable workload at both sizes. Removing scratch allocations has not
removed the root column's full child-list traversal.

## Proposed validation contract

The candidate is to validate the results consumed by a parent before propagating
a child edit further. It needs to distinguish a node's own changed inputs from
pending work below it. Dirty-branch routing must identify affected children
without scanning every sibling merely to discover which child changed.

For each retained parent result, the relevant observations are:

- Child identity and the complete sizing query plus returned `Metrics` for each
  child measurement read. Query variant matters, not just its numeric width.
- Child identity, assigned size, and returned `LayoutOutput` for each child
  layout read. Baseline and content extent are part of the output.
- The parent's own inputs, topology, and external request under which it used
  these observations.

Measurement-cache results for different parent queries may depend on different
reads. Final layout may use another set. Evidence must belong to the retained
parent result it justifies; recording only the last callback's reads is not
sufficient. If evidence is absent or deliberately dropped to bound storage, that
result must be recomputed rather than assumed valid.

With unchanged parent inputs and request, all consumed results must remain equal
before parent reuse is permitted. Full returned values are the initial comparison
unit; field-specific dependencies would require an additional contract. Conditional
reads require replacing the relevant dependency record after a successful new
execution, including removing reads no longer used. This relies on the existing
cache assumption that callbacks are determined by explicit inputs and context
results, not untracked mutable global state.

Successful child validation may still produce an internal geometry draft. A
stable outward result can stop upward propagation, but cannot discard that draft.
Dependency records, output validity, and geometry publication must agree on the
same successful attempt. Errors and panics preserve pending work and the previous
published state; partial validation cannot mark an ancestor clean.

## Next implementation gate

The [recording experiment](dependency-recording.md) validates record replacement,
cache eviction, and failed attempts under conservative execution. Its current
storage overhead requires a representation experiment before a separate change
uses the evidence to stop propagation.

The first pruning comparison should cover stable edits, height changes that move
later siblings, baseline-only and content-only changes, batched edits, and width
changes. Require matching geometry after every edit, meaningful reductions in
visited work, repeated complete-path timings, and measured dependency-storage
cost. The stable workload must not retain an O(number of siblings) discovery
scan hidden behind fewer measurement bodies.
