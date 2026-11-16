# Experiment Scope

## Purpose

Evaluate a Rust GUI foundation with fine-grained updates, low resource usage,
and support for custom rendering and animation.

## Current boundary

The current milestone is a persistent node tree in `solor-core`: stable live
handles, ordered parent-child relationships, payload access, and subtree removal.
The design and acceptance criteria are in [node-tree.md](node-tree.md).

This tree is a storage foundation. It does not yet establish a UI update model.

The following are outside this milestone:

- Moving existing nodes, immutable snapshots, and serialization.
- Signals, dependency tracking, and invalidation propagation.
- Layout, rendering, text, input, and window integration.
- Widgets, showcase applications, and performance benchmarks.
- Selecting a rendering backend or committing to a complete framework design.

Acceptance covers handle validity, topology consistency, ordered children,
payload lifetime, deep-tree removal, and isolated payload edits. Formatting,
compilation, lint checks, and focused tests validate this milestone.

## Candidate investigations

Local updates and baseline resource usage are candidate investigations, not
implemented capabilities or an approved implementation sequence. SlotMap backs
the current tree experiment; this does not settle reactive or rendering choices.

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
