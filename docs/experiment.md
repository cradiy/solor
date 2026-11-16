# Experiment Scope

## Purpose

Evaluate whether a new Rust GUI foundation can improve application development
enough to justify the effort of building and maintaining it. Replacing GPUI Fusion
is not an established requirement.

## Current boundary

The current milestone is limited to development guidelines and a buildable Cargo
workspace with one empty core library. It does not include functional GUI code.

The following are outside this milestone:

- Node storage, persistent trees, signals, and invalidation tracking.
- Layout, rendering, text, input, and window integration.
- Widgets, showcase applications, and performance benchmarks.
- Selecting a rendering backend or committing to a complete framework design.

Scaffold acceptance consists of successful formatting, compilation, and lint
checks. These checks establish project setup only.

## Candidate investigations

Local updates and baseline resource usage are candidate investigations, not
implemented capabilities or an approved implementation sequence. A retained
tree, signals, a particular arena, and CPU rendering remain design options.

The first functional experiment should address one concrete limitation observed
in application development. Its scope and acceptance criteria must be established
before implementation. Local update propagation and CPU rendering should be
evaluated separately so their costs and benefits can be identified.

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
Successful scaffolding alone does not justify further architectural commitments.

No comparative performance result, fixed implementation schedule, or commitment
to replace an existing framework has been established.
