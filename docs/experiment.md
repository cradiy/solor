# Experiment Scope

## Purpose

Evaluate a Rust GUI foundation with fine-grained updates, low resource usage,
and support for custom rendering and animation.

## Current boundary

The core library provides a [persistent node tree](node-tree.md),
[scoped execution of node payloads](node-store-experiment.md), and
[explicit state-to-node updates](state-updates.md) with typed signals,
deduplicated pending work, and lifecycle cleanup. Its [layout module](layout-contract.md) owns measurement,
child allocation, atomic publication of local geometry and on-demand sharing
of completed geometry across frames. The Linux window uses
this module with default features. Dependency recording and checkpoint-based
pruning are opt-in through `layout-audit`.

The [access experiment](node-store-experiment.md) records scoped payload borrowing,
failure behavior, and measured costs for scalar and heterogeneous workloads.
The [layout contract experiment](layout-contract.md) includes
[size negotiation](size-negotiation.md): explicit width-query semantics,
final allocation distinct from measurement, child layout outputs before placement,
and bounded multi-query caching. A synthetic icon/label/button row exercises
wrapping, baseline alignment, and content overflow while retaining conservative
ancestor invalidation and publication only after a complete successful layout.
The [layout cost experiment](layout-cost.md) measures the complete
headless update path with one and three inline cache slots, including local
edits, batch edits, width changes, temporary allocation, and live fixture storage.
The [scratch reuse experiment](layout-scratch.md) uses scene-owned
publication and child-state buffers, with cleanup on every exit and explicit
capacity reclamation. It compares complete updates with the fresh-scratch baseline.
The [dependency-boundary investigation](layout-dependencies.md) establishes
counterexamples to size-only propagation checks and measures updates that change
content while preserving all geometry. The current
[dependency-recording experiment](dependency-recording.md) retains per-query
measurement reads and atomically published layout reads. The record representation
has substantial measured overhead.
The [record-storage evaluation](record-storage-evaluation.md) compares a pooled
candidate with this reference and adds warmed live-heap probes. The pool is not
selected because local-update CPU cost and active retained storage regress.
The current representation uses optional per-node record blocks: complete empty
evidence is implicit, and unused blocks are released. It reduces warmed heap
without changing recording during callbacks or ancestor traversal.
The [dependency-result audit](dependency-validation.md) checks captured reads
against current child results. Owner-input revisions and exact requests gate the
check; incomplete evidence is unverifiable. Speculative geometry is discarded,
and normal layout still completes the update without propagation pruning.
The [selective-check experiment](selective-dependency-checks.md) routes bounded
edit history through a checkpoint-local child/read index. Stable local audits
visit fewer reads. [Checkpoint refresh](checkpoint-refresh.md) reuses an index
when the ordered child reads match, reducing replacement costs for changed
results and resizing. The [shared layout attempt](shared-layout-attempt.md) lets
normal layout adopt completed audit subtrees within one update. Only adopted
drafts publish after root success; failed attempts retain both checkpoint cursors.
The audit remains opt-in and is not selected for automatic runtime validation.
The [parent-layout skip](parent-layout-skip.md) uses complete unchanged layout
dependencies to retain the current root layout while publishing audited child
changes. Evidence must be anchored to that publication; standalone checks cannot
hide unpublished work. Root measurement remains conservative.
The [fixed-font text experiment](text-layout.md) connects real shaping and line
layout to these measurement/allocation semantics. Text nodes own bounded caches
of immutable paragraphs; geometry and glyph results are checked against fresh
buffers and forced layout. It uses one bundled font subset and remains headless.
The [text drawing snapshot experiment](text-frame.md) publishes geometry, paragraph
handles, colors, and absolute positions together. It retains the old frame after
preparation failures, including when internal layout has already succeeded.
The [CPU text pixel experiment](text-pixels.md) renders snapshot glyphs to an
opaque RGBA surface and checks placement, clipping, blending and replay against
pixel references. It measures full CPU redraw separately from snapshot preparation. A bounded glyph
mask cache is evaluated on warm, cold and eviction-heavy full redraws against
the uncached pixel output. Retained CPU pixels support a single damage rectangle
for clipped text commands, with old and new bounds, ordered overlap replay,
and full redraw on target changes.
The [Wayland text window](text-window.md) presents these pixels at the preferred
surface scale. Acceptance covers a mapped two-row window, alternating row edits,
released buffer reuse, compositor-driven resizing, agreement with uncached
physical pixels, idle commits and normal close. Record client PSS and shared
buffer sizes separately. The partial upload experiment tracks the bounding rectangle of missed changes
per shared file. Compare bytes and conversion/write time for bounded row spans,
full-width scanline bands and full uploads
under identical content and compositor damage, and verify the complete file
after edits and resizing. The window reports surface damage in physical buffer
coordinates. Check that a new target reports full damage, local edits cover
every changed pixel, and a slot can be repaired independently of that rectangle.
For empty damage, verify zero shared-file writes, progress of scheduled frame
callbacks, wakeup on input, and correct pixels on the next buffer submission.
The [disclosure experiment](disclosure.md) connects primary pointer activation,
explicit state bindings and conditional detail lifetime to the CPU window.
Acceptance covers logical hit testing, cancellation, frame requests, repeated
mount and removal, and complete pixel agreement. A mounted detail accepts bound
text and color edits. Verification requires stable node identity, sibling movement
after wrapping, and a paint-only edit with unchanged layout counters and pixels
matching full rasterization. The component and input
contract remain private to the example. Its three-item list mounts the middle
item's detail between frames. Acceptance requires stable surviving layout
handles, cached neighbor layout under unchanged allocation, correct downstream
placement, invalidation of old checkpoints and agreement with full layout
and rasterization across widths. A 1,024-row variant compares full and
viewport-limited drawing commands for the same retained layout. Acceptance
includes partially visible rows, resize, hit testing and pixel replay of a
held frame. The frame keeps geometry for every logical row, including rows
outside the viewport.

For a host-assigned viewport, the disclosure root can lay out its children
directly at the given size. Compare edit-through-snapshot time against a
preferred-size query followed by the same allocation. Require agreement with
forced layout and full rasterization, and verify that a subsequent preferred-size
request still measures the root after an edit.

For a fixed run of rows following the disclosure, compare moving one parent
with placing each row separately. Require stable row identities and local
positions during a toggle, agreement with forced flat layout across wrap
widths, and lower long-list edit-through-snapshot cost. Measure width changes
separately. This experiment groups the known suffix of one disclosure list.

The scrolling experiment takes an absolute logical offset on the disclosure
frame request. Require agreement with a crop of the full document at partially
visible rows, hit testing against the published viewport and stable measurement
and layout counts for scroll-only frames. Scroll-only snapshots share published
geometry. Verify that variable-height rows remain visible through their final
logical pixel and that held frames survive a topology change. Compare snapshot
time at the start, middle and end of the same list. The Linux host maps focused vertical pointer
axis input to that offset. Native acceptance covers repeated movement, fractional
accumulation, idle commits at both bounds, reverse input at the bottom,
title activation after returning to the top, and shared-buffer pixel agreement.
Horizontal scrolling and kinetic scrolling are outside this experiment.

The [component lifecycle fixture](node-execution.md#headless-lifecycle-evidence)
exercises single and nested scopes owning sibling visual leaves. Acceptance covers cancellation
before binding delivery, press retirement across a failed frame and remount,
retained pixel replay, child-first and parent-first disposal, and delayed disposal
after remount. Unrelated scopes retain their bindings and presses. Frame acceptance is
simulated in one window. [Disposal timings](node-execution.md#cost-of-disposing-a-scope)
compare membership discovery and full removal across tree sizes, with checks on
surviving order, binding cancellation and payload drops.

The [headless window acceptance experiment](window-frame-commit.md#headless-acceptance-evidence)
injects an upload failure after rasterization. It checks hit targets, binding
delivery and submitted bytes through retry, resize and replacement with an
identical-looking mount.

The [solor authoring library](component-api.md#lifecycle-checks)
tests initialization during mount, immediate event writes, ordered conditional
subtrees at the next update boundary, and signal ownership across remounts.
It checks targeted branch delivery, gap placement and surviving sibling
identity across subtree insertion and removal. A caller-defined meter uses
`Render` for composition and a retained `Element` for layout and local paint
recording. The [mounted frame tests](../crates/solor-render-cpu/tests/view_frame.rs)
check nested placement, clipped gradients, glyphs, frame-owned resources and
pointer delivery before and after acceptance. The [mounted window](ui-contracts.md#presenting-a-mounted-view)
uses the same frame path on Wayland. In a nested Niri session, pointer input
opened and closed the conditional row. A window capture showed the detail
background after expansion and the original pixel color after collapse.
Clicking the retired row's location produced no further buffer submission. The [refresh workload](component-api.md#high-frequency-updates)
measures local paint, shared subscriptions and structural layout at an explicit update boundary.
The [mounted meter workload](../crates/solor-render-cpu/tests/meter_updates.rs)
compares window updates with independent full rasterization at fractional
scale, including shrinking fills, overlapping text and submission retries.
See [mounted CPU window measurements](text-pixels.md#mounted-cpu-window-measurements).
The [UI contracts](ui-contracts.md) distinguish implemented authoring behavior
from open frame, input and platform boundaries.
The [shared source text experiment](text-layout.md#shared-source-text) checks
source identity across nodes and widths and measures short-label construction.

The following are outside this milestone:

- A general UI property facade or replacement of the core node store.
- Moving existing nodes, immutable tree snapshots, and serialization.
- Derived signals and automatic signal tracking outside element paint.
- Recursive dependency pruning, general paint
  invalidation, and general pixel damage tracking.
- General layout algorithms, font fallback, rich text, GPU rendering, general input routing, and a general window abstraction.
- A standard widget collection, showcase applications, and application-level performance comparisons.
- Selecting a rendering backend or committing to a complete framework design.

Acceptance requires independent per-query records, cache-hit observations,
conditional replacement, eviction, bounded recording, and no partial layout-record
publication after errors or panics. Geometry and execution counts must agree with
the unrecorded baseline; CPU, allocation, and retained-storage costs are measured
separately. Storage candidates must also measure heap after repeated
updates and explicit reclamation; fewer allocations alone does not justify a
regression in the target workload. Existing failure and scratch-reclamation
guarantees remain required. Optional record blocks must distinguish empty from incomplete evidence
and preserve published layout evidence when measurement records are cleared. These
audits must distinguish measured metrics from final layout output, retain internal
movement behind unchanged outputs, survive cache eviction, and leave published
geometry and records intact after errors or panics. Audit work and checkpoint
storage must be reported separately from the normal update path. Selection must
retain exact queries and execution order, reject missing edit history, and keep
pending descendant changes when an audit cursor advances. Complete update costs
must include checkpoint replacement as well as repeated layout work. Index reuse
requires an unchanged child sequence and valid evidence. Refresh must preserve
old evidence on rejection. Shared attempts must reject mismatched allocations,
discard obsolete drafts, preserve internal movement, and leave geometry and
checkpoint cursors unchanged after failure. Staged results cannot survive into
another update. Skipping root layout requires unchanged owner inputs, allocation,
complete consumed results, and evidence for the current publication. Preserve
direct-child placement and newly computed descendant movement. Historical equality
or a cursor advanced by standalone auditing must fall back to normal layout.
Measure complete updates and retained storage, including cases that cannot skip.
This bounded headless milestone is sufficient when its selective benefit and
fallback costs are understood; it does not require further micro-optimization.
The separate text milestone requires fixed-font mixed-script wrapping and baseline
checks, correct final allocation after intervening measurement queries, explicit
missing-glyph errors, bounded cache replacement, owned result lifetimes, and full
recomputation agreement. Measure content edits, style edits, repeated widths,
width churn, engine construction, and retained heap with explicit reclamation.
The drawing snapshot milestone requires color-only changes without layout work,
same-size glyph replacement, nested absolute placement, atomic frame replacement,
retry after completed layout, and resource lifetimes independent of node caches.
Compare complete updates with layout-only work and measure reclamation while a
consumer holds the last frame. These checks do not validate pixel output or a
complete incremental layout system.

## Candidate investigations

The current explicit update mechanism does not settle automatic dependency
tracking or rendering choices. Baseline application resource usage remains a
separate investigation.

The next experiment needs its own concrete problem and acceptance criteria.
Local update propagation and CPU rendering should be evaluated separately so
their costs and benefits can be identified.

The initial native desktop investigation is limited to Linux with Wayland.
The platform contract belongs to the shared framework. This experiment validates
the native implementation on Linux.

## Decision gates

For each functional experiment, record:

1. The concrete problem and relevant baseline behavior.
2. The smallest mechanism needed to investigate it.
3. Correctness requirements and measurable acceptance criteria.
4. A bounded effort budget and explicit stopping conditions.
5. Results, unresolved limitations, and a decision to continue, revise, or stop.

Continue when the evidence supports a useful improvement and the next milestone
fits the available effort. Revise or stop when the expected benefit is absent,
the comparison is inconclusive, or the required scope exceeds the agreed budget.
Successful storage tests alone do not justify further architectural commitments.

No application-level comparative performance result, fixed implementation
schedule, or commitment to replace an existing framework has been established.
