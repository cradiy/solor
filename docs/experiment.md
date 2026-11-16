# Experiment Scope

## Purpose

Evaluate a Rust GUI foundation with fine-grained updates, low resource usage,
and support for custom rendering and animation.

## Current boundary

The current milestone is explicit state-to-node updates: typed signals, bindings,
deduplicated pending work, and lifecycle cleanup. The runtime owns the
[persistent node tree](node-tree.md). Its scheduling contract and acceptance
criteria are in [state-updates.md](state-updates.md).

The following are outside this milestone:

- Moving existing nodes, immutable snapshots, and serialization.
- Derived signals, automatic dependency tracking, and multi-source bindings.
- Writes to runtime state from update callbacks and layout/paint invalidation.
- Layout, rendering, text, input, and window integration.
- Widgets, showcase applications, and application-level performance comparisons.
- Selecting a rendering backend or committing to a complete framework design.

Acceptance covers local callback execution, batch coalescing, equality suppression,
and cancellation of pending work when bindings, signals, or nodes are removed.
Focused tests cover panic behavior and index consistency. Release benchmarks
measure update dispatch across node counts and workloads, separately from rendering.

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

No comparative performance result, fixed implementation schedule, or commitment
to replace an existing framework has been established.
