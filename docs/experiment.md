# Experiment Scope

## Purpose

Evaluate a Rust GUI foundation with fine-grained updates, low resource usage,
and support for custom rendering and animation.

## Current boundary

The implemented foundation consists of a [persistent node tree](node-tree.md)
and [explicit state-to-node updates](state-updates.md), including typed signals,
deduplicated pending work, and lifecycle cleanup.

The current milestone tests scoped payload loans in an isolated example over the
existing tree. The [access experiment](node-store-experiment.md) records ownership,
failure behavior, correctness checks, and measured costs. It does not change the
public core API. The [node execution model draft](node-execution.md) remains a
proposal for stages, layout dependencies, and cache lifetime.

The following are outside this milestone:

- Publishing a node behavior API or UI property facade, or replacing the backing map.
- Moving existing nodes, immutable snapshots, and serialization.
- Derived signals, automatic dependency tracking, and multi-source bindings.
- Writes to runtime state from update callbacks and layout/paint invalidation.
- Layout, rendering, text, input, and window integration.
- Widgets, showcase applications, and application-level performance comparisons.
- Selecting a rendering backend or committing to a complete framework design.

Acceptance requires nested heterogeneous execution in safe Rust, stable identity
and payload addresses, rejection of reentry and traversal-time structural edits,
restoration after errors and panic unwinding, and an explicit depth limit. Measure
lookup, invocation, allocation, and storage costs before adopting the mechanism.
Stop this milestone at the access layer; public UI design requires a separate
decision. Neither these checks nor state-dispatch tests validate layout or paint.

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
