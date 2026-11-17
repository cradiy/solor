# Size Negotiation Experiment

## Scenario and boundary

Extend the private headless layout example with a row containing an icon, a
wrapping label, and a button. The purpose is to distinguish a sizing query from
an allocation and final layout output. This is not a general flex algorithm,
text engine, or public control API.

Width queries have three explicit meanings:

| Query | Meaning |
| --- | --- |
| Natural | Preferred unwrapped width in these fixtures; no finite width bound |
| AtMost(w) | The measured box may choose a width from zero through w |
| Exact(w) | The measured box must have width w |

Zero is a valid allocation. Natural is a separate variant, not a sentinel integer
or infinity in arithmetic. Heights remain content-driven during measurement.
The caller may allocate a different finite width or height for final layout.
General two-axis constraints, min-content sizing, fractional coordinates, and
negative origins remain outside this experiment.

## Row negotiation

1. Query all three children at their natural width.
2. Apply the row's width query to its preferred width, including padding and gaps.
3. Reserve the icon width, button width, padding, and gaps. Query the label at
   exactly the remaining width, saturating at zero.
4. Use measured baselines and descents to propose a row height.
5. During final layout, negotiate again using the actual allocated width.
6. Lay out each child at its assigned size and obtain its final baseline and
   content extent. Align the label and button using these final outputs, then
   place the icon vertically centered in the resulting content band.
7. Center that band vertically if the row receives extra height. Report the
   final row baseline and content extent independently of the allocated box.

The row preserves the icon and button's natural sizes. If those sizes and the
gaps exceed the available width, the label receives zero width and content can
extend outside the row. A nonempty synthetic wrapping label uses at least one
cell per line, even in a zero-width allocation. This is an explicit fixture
overflow policy; clipping, elision, hiding, and responsive rearrangement are not
implemented. A reported content extent is layout data, not a pixel damage bound.

## Final layout and placement

Measurement returns suggested size and baseline. Final layout receives an
allocated `Size` and returns a baseline plus content extent. The framework owns
the allocated size; a child cannot silently replace it with its preferred size.
Baselines use the child's local coordinates and can lie outside an undersized
allocated box, but must be within its reported content height.

The parent first calls `layout_child` to obtain final output, then calls
`place_child` with an origin. Each child must be laid out and placed exactly once
in an attempt. This allows baseline alignment without guessing a child's final
baseline or rerunning its layout solely to change its origin.

Both operations affect only the unpublished journal. The existing failure rules
remain: caught child errors or panics still abort the attempt; no geometry is
published until the whole operation succeeds. A cache hit can stage a child's
root entry for a new origin while retaining all descendant geometry.

## Cache and work accounting

The full width-query variant and value form the measurement key. Natural,
AtMost(80), and Exact(80) are different queries even if a fixture happens to
return the same size. Input and ancestor invalidation clear all cached queries
for an affected node. A failed query cannot publish a result.

A fixed three-entry cache per node covers the row's natural and assigned-width
queries without unbounded growth. Replacement is FIFO; the capacity is an
experimental choice, not a framework guarantee. A cache-disabled mode remains
the correctness reference. Counters distinguish query requests, cache hits,
measurement bodies, and layout bodies. These are work counts, not frame timings.

The framework caches metrics only. A cache hit does not restore a node's private
measurement state. Final layout must calculate from the assigned size or use a
behavior-owned cache with a matching key; it cannot assume that the last executed
measurement prepared the right state. The synthetic leaves calculate from the
assignment. Ownership of real shaping results remains an unresolved text-engine
integration concern.

## Acceptance

- Independently check row dimensions, allocated child widths, baseline alignment,
  and overflow for wide, narrow, and zero-width cases.
- Allocate a width or height different from measurement; require final wrapping,
  baseline, and content extent to follow the allocation.
- Alternate Natural, AtMost, and Exact queries, including equal numeric limits;
  check key separation, bounded replacement, and reuse across layout.
- Change label length, button text length, available width, and assigned height;
  compare every incremental result with cache-disabled recomputation.
- Preserve the earlier locality, failure, panic, and publication checks under the
  revised private interface.

The milestone ends with this row and its contract checks. It does not select a
rendering backend, implement real font shaping, or integrate the examples into
the public core runtime.

## Implementation and evidence

The [row and button fixtures](../crates/solor-core/examples/layout_contract/negotiation.rs)
use the revised private layout context. The
[negotiation tests](../crates/solor-core/examples/layout_contract/negotiation_tests.rs)
include 384 consecutive edits and query/allocation changes, each compared with
cache-disabled recomputation. The earlier 256-step vertical comparison and
failure checks also run with this interface.

The known-value row has 2 units of padding, gaps of 3, a 10-by-10 icon, a label
of 20 cells with advance 2, and a button initially measuring 12 by 10. At widths
80 and 50, the row measures 14 and 21 units high. Increasing the button's natural
width to 20 at row width 50 makes the label narrower and the row 26 units high.
At row width 20 or zero, the content extent is 40 wide while the allocated width
remains the requested value.

With unchanged inputs, increasing only the allocated row height from 14 to 34
causes no measurement bodies and no child layout bodies to execute. The row's
layout executes once, moves the three children, and returns its updated baseline.
All measurement requests in that attempt hit cached results. The executable also
prints query, hit, measurement-body, and layout-body counts for a label edit with
a final allocation different from its natural measurement.

Three slots bound cache storage and avoid repeated work in these scenarios, but
increase per-node resident storage relative to one slot. This experiment does
not establish an optimal cache capacity or a CPU/memory advantage for real UI
workloads. No performance claim is based solely on query-hit counts.
The separate [layout cost experiment](layout-cost.md) measures this tradeoff
with identical fixtures compiled at one and three inline slots.

```sh
cargo run --release -p solor-core --example layout_contract --offline
cargo test -p solor-core --example layout_contract --offline
```
