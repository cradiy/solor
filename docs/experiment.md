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
The current [scratch reuse experiment](layout-scratch.md) uses scene-owned
publication and child-state buffers, with cleanup on every exit and explicit
capacity reclamation. It compares complete updates with the fresh-scratch baseline.
The [node execution model draft](node-execution.md) remains the broader proposal.

The following are outside this milestone:

- Publishing a node behavior API or UI property facade, or replacing the backing map.
- Moving existing nodes, immutable snapshots, and serialization.
- Derived signals, automatic dependency tracking, and multi-source bindings.
- Integration with signal callbacks, automatic layout dependencies, and paint invalidation.
- General layout algorithms, real text measurement, rendering, input, and windows.
- Widgets, showcase applications, and application-level performance comparisons.
- Selecting a rendering backend or committing to a complete framework design.

Acceptance requires preserved sizing, locality, and failure guarantees; matching
geometry and work counts; repeated sequential CPU measurements; and separate
allocation measurements with a positive control. Scratch must remain empty after
errors and panics, and reclamation must preserve clean requests and pending retries.
Stop at the measured allocation/retention tradeoff and remaining costs. These
checks do not validate real text, pixel output, or a complete incremental layout
system.

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
