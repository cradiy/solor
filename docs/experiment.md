# Experiment Scope

## Purpose

Evaluate a Rust GUI foundation with fine-grained updates, low resource usage,
and support for custom rendering and animation.

## Current boundary

The implemented foundation consists of a [persistent node tree](node-tree.md)
and [explicit state-to-node updates](state-updates.md), including typed signals,
deduplicated pending work, and lifecycle cleanup.

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
The [node execution model draft](node-execution.md) remains the broader proposal.

The following are outside this milestone:

- Publishing a node behavior API or UI property facade, or replacing the backing map.
- Moving existing nodes, immutable tree snapshots, and serialization.
- Derived signals, automatic dependency tracking, and multi-source bindings.
- Integration with signal callbacks, recursive dependency pruning, general paint
  invalidation, and pixel damage tracking.
- General layout algorithms, font fallback, rich text, rendering, input, and windows.
- Widgets, showcase applications, and application-level performance comparisons.
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
Cross-platform support is a later decision, not a current deliverable.

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
