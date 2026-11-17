# Node Execution Model Draft

## Status and scope

This is a proposed execution contract, not an implemented UI API. The implemented
core provides a persistent tree and explicit signal bindings. No node behavior
trait, layout system, paint recorder, or rendering backend is selected by this
draft.

An isolated [node-store access experiment](node-store-experiment.md) implements
scoped payload loans over the existing tree. A separate
[layout contract experiment](layout-contract.md) validates a narrow sizing domain,
conservative invalidation, and geometry publication. Neither example selects the
public node API or implements the full stage and lifecycle contracts proposed here.

The proposed direction is a persistent tree of heterogeneous nodes, with explicit
mutation and framework-controlled execution stages. Validate the contract with a
vertical container, a wrapping text leaf, and a custom drawing leaf before
expanding it into a widget toolkit.

## Ownership

| Owner | Data and responsibility |
| --- | --- |
| Component scope | Application state, subscriptions, and the nodes created by a component; composition need not add a runtime node |
| Node store | Live node identities, canonical parent-child relationships, node implementations, and framework metadata |
| Node implementation | Type-specific inputs and private caches, such as text content, shaping data, or custom drawing parameters |
| Frame coordinator | Pending execution work and the ordering of update, layout, composition, and paint recording |
| Scene and resource storage | Retained drawing descriptions and resource leases needed by current or in-flight output |
| Rendering backend | Device-specific resources, execution, and presentation |

These are responsibilities, not a requirement for separate crates, duplicate
trees, or a heap allocation for every conceptual object. The initial model has
one canonical parent-child topology. Nodes refer to children by handles; their
payloads do not independently own a second child hierarchy.

Shared layout and appearance inputs may exist alongside type-specific node data.
They must remain distinct from computed geometry and backend resources. No common
property struct is required to contain every node's state.

The node store owns payload lifetime. A copied handle does not keep a node alive.
Removing a node cancels its node-targeted bindings. A shared application signal or
image resource is not destroyed merely because one consumer is removed. Component
scope disposal and shared signal ownership need a separate contract; the existing
runtime's explicit signal removal remains its current behavior.

## Stage contracts

| Stage | Inputs and outputs | Mutation permissions |
| --- | --- | --- |
| Update | Actions or state changes become node input changes and execution requests | Edit the target's inputs; queue structural changes for an execution boundary |
| Measure | Sizing queries and content produce dimensions and layout metrics | Update private measurement caches; request child measurements through the context |
| Layout | Assigned space produces child sizes and relative placements | Update layout outputs; invoke child layout through the context |
| Compose | Relative geometry, scrolling, transforms, and clips produce effective geometry | Update framework geometry and associated hit-test information |
| Record paint | Stable inputs and geometry produce local drawing descriptions and paint bounds | Update private recording caches; no application-state or topology mutation |
| Submit | Scene changes and resources are executed by the backend | Backend resources and presentation state only |

Measurement and layout are cooperating operations, not necessarily two global
tree sweeps. A container may measure a child, decide its assigned size, lay it out,
and place it. Some containers need several sizing queries. The design does not
promise one query per child or universally linear layout.

Framework traversal composes node drawing with children in a defined order:
background/content, children in sibling order, then overlays. A node's local paint
record must not recursively flatten all descendant drawing into its own cache.
The exact recording API remains open.

The initial scheduler should process updates at explicit boundaries, then finish
the necessary geometry work before recording paint. Late update requests are
queued for the next update boundary, not executed recursively from layout or
painting. Event dispatch must not observe partially computed geometry. Any future
synchronous geometry query needs an explicit completion barrier.

## Layout inputs and dependencies

Keep three categories distinct:

- Requested layout: fixed or content-based dimensions, limits, padding, and
  container policy. The initial vocabulary is still to be selected.
- Measurement: what a node reports for a particular sizing query, including
  dimensions and any baseline information the parent actually uses.
- Geometry: the assigned size, relative placement, transforms, and clipping used
  by subsequent stages.

A parent sends a sizing query to a child and records which layout outputs it
uses. Child placement belongs to the parent. The child controls its own content
and local painting within the resulting geometry and clipping contract.

Cache validity includes the sizing query, relevant node-input revisions, child
dependencies, and resource revisions. External inputs such as font availability
must invalidate the affected node through an update notification. Global state
read without a revision or notification cannot safely participate in a cache.

Start with conservative explicit layout invalidation to the nearest justified
dependency boundary. A fixed outer size alone does not prove that a container's
internal child placements remain valid. Skipping parent work after remeasurement
requires that every output the parent depends on remains unchanged, including
baselines where applicable. That optimization is not an initial guarantee.

Flutter's layout interface explicitly records whether the parent uses child layout
information through `parentUsesSize`; it illustrates why the dependency belongs
in the layout contract. This draft does not adopt Flutter's exact boundary rules.
[Flutter layout contract](https://api.flutter.dev/flutter/rendering/RenderObject/layout.html)

Masonry's current widget interface separates measurement from layout and documents
the inputs used by its measurement cache. Its layout implementation can check
cached measurements against recomputation in debug builds. This is a useful
validation pattern, not a requirement to copy its sizing vocabulary.
[Widget contract](https://github.com/linebender/xilem/blob/main/masonry_core/src/core/widget.rs),
[layout implementation](https://github.com/linebender/xilem/blob/main/masonry_core/src/passes/layout.rs)

## Three-node walkthrough

Use a vertical container with a wrapping text leaf followed by a custom drawing
leaf. The container has a known available width, padding, and a gap. The text has
content-dependent height. The drawing leaf has an explicit preferred size and
content that can change independently of that size.

### Initial layout and drawing

1. An update boundary attaches the nodes and registers their dependencies.
2. The container derives child sizing queries from its available space and policy.
3. The text node measures with the selected width and font inputs. It keeps any
   expensive text work in its own cache and reports the required layout metrics.
4. The drawing node reports its sizing result without constructing a GPU resource.
5. The container chooses child sizes, invokes child layout, and assigns offsets.
   Its own measured height depends on the children, padding, and gap.
6. Composition establishes effective transforms, clips, and hit-test geometry.
7. Paint recording produces the container decoration, text drawing, and custom
   drawing records. The scene retains their order and resource references.
8. The backend executes the scene. Resource release follows backend completion,
   which can occur later than node mutation or removal.

### Updates

| Change | Required work | Reusable work, subject to the cache contract |
| --- | --- | --- |
| Container background changes | Re-record its decoration and update scene damage | Child measurement and placement |
| Text changes | Invalidate text measurement and paint; reconsider dependent container layout | Unrelated branches; drawing leaf measurements with unchanged inputs |
| Text height changes | Recompute dependent container size and placement of the following leaf | Drawing leaf local paint if its own size and drawing inputs remain unchanged |
| Available width changes | Reconsider container queries, text wrapping, and affected geometry | Any child computations whose complete keys remain unchanged |
| Custom drawing parameters change without affecting size | Re-record that leaf's drawing and paint bounds | Layout, if the leaf's declared layout inputs did not change |
| Scroll translation changes | Update effective transforms, clipping, hit testing, and scene damage | Local drawing records where translation is represented in composition |
| Drawing leaf is removed | Cancel its work, update parent layout, remove its scene contribution | Unrelated nodes and their caches |

Even when text reports the same size, its paint still changes. Baseline or other
parent-visible metrics can also require parent work. A scroll operation can
change which content is visible; retaining local records does not mean that no
pixels need rendering.

## Invalidation and completion

Node-specific mutation methods declare which derived results may be stale.
Requests are idempotent and coalesced. Distinguish a node's own pending work from
the fact that some descendant needs work, so traversal can skip clean subtrees.
Only stages with a defined consumer should acquire a new invalidation category.

The initial design does not compare complete input snapshots or promise to cancel
all work when values return to an earlier value. It must avoid omitted work first;
fine-grained cancellation can be measured as a separate optimization.

Successful stage execution validates the outputs for the input revisions it
consumed. Failed execution must not clear the corresponding request or publish a
partially produced paint record. Restoring a borrowed payload does not roll back
user mutations. Panic recovery and partial layout publication require dedicated
tests before a UI runtime is considered usable.

For rendering, distinguish local recording invalidation from pixel damage.
Movement and removal need previous effective paint bounds, including effects that
extend beyond layout bounds. Translucency and overlap can require replaying other
unchanged drawing records within a damaged region. Bounds are conservatively
provided by the recording layer or by custom content, not inferred solely from a
node's requested size.

Keep scene completion separate from presentation completion. The backend may need
old resources until in-flight work finishes. A node's destruction cannot invalidate
resource references still held by submitted output. Initial backend policy may
redraw a full frame; incremental recordings do not themselves require partial
rasterization.

CPU/GPU implementations should share ordinary drawing semantics. Custom effects
need an explicit capability contract: a GPU-only implementation does not establish
CPU support. No per-node offscreen texture is assumed.

## Node store access and Rust borrowing

A specialized access layer is needed; a new map implementation is not yet justified.
`SlotMap` addresses identity and slot reuse. It does not by itself permit a callback
to hold mutable access to one payload while borrowing the entire store mutably to
execute arbitrary descendants.

The preferred first candidate is a scoped payload loan over the existing arena:

1. Validate the handle and execution stage, then mark the node active.
2. Temporarily move its owned behavior object out of the payload slot. Keep its
   identity, topology, and framework metadata in the store.
3. Invoke it with a restricted context. Child execution follows the same process;
   the parent payload is held outside the storage borrow during that call.
4. Restore the payload and clear the active marker using a scope guard, including
   during panic unwinding.

The access experiment implements this mechanism without stage-specific contexts.
The complete execution model still requires these invariants:

- A vacant payload slot for an active node is distinct from a deleted node.
- Contexts cannot re-enter the active node or an active ancestor's behavior object.
- Contexts expose only the child operations allowed by that stage, not unrestricted
  mutable arena access. Layout cannot insert, remove, or move nodes.
- Deferred structural edits run after all affected payload loans have ended.
- Guard restoration cannot call user code or fail because its slot was removed.
- Cache validity and work flags remain conservative after an interrupted call.
- Deep execution needs an explicit stack/depth policy; nonrecursive tree deletion
  does not prove that recursively invoking layout is stack-safe.

The prototype tests access restrictions, restoration, and depth limits in safe
Rust, and measures lookup, invocation, allocation, and storage costs. Deferred
edits, stage permissions, and cache validity remain unimplemented. Do not trade
away stable identity or efficient lookup without evidence.

A subtree-lending arena is an alternative. The `tree_arena` documentation explains
how its access model separates the current item from child access, and describes
different lookup costs for its safe and unsafe implementations. That tradeoff is
a reason to study its contract, not to replace Solor's storage immediately.
[tree_arena architecture](https://docs.rs/tree_arena/latest/tree_arena/)

## Lifecycle and component integration

Structural updates occur at an update boundary. Creation attaches identity and
topology before scheduling node execution. Removal cancels targeted work and
subscriptions, releases focus or capture when supported, updates parent
dependencies, and removes the node from the scene while retaining any resources
required by outstanding output.

Dynamic lists require a separate identity policy: preserving a logical item should
preserve its node where appropriate, while deletion must invalidate old handles.
The current append/remove tree API does not implement reconciliation or moving
nodes. Those operations should not be simulated by exposing mutable topology to
arbitrary callbacks.

Explicit signals may continue to drive typed node mutations. Their ownership and
the component scope model must be resolved before adding automatic effects or
multi-window shared state. Floem's documented use of effects to deliver updates
through a view identity is a relevant example of separating reactive notification
from view mutation; it does not settle Solor's public component API.
[Floem View updates](https://lapce.dev/floem/floem/view/trait.View.html)

Masonry's phase-specific contexts also provide a reference for limiting access
and mutation during traversal. Solor's proposed stage boundaries need their own
tests rather than assuming identical behavior.
[Masonry pass system](https://docs.rs/masonry_core/latest/masonry_core/doc/internals_01_pass_system/index.html)

## Validation before implementation expands

The isolated access prototype establishes the basic borrowing mechanism. The
headless layout experiment tests synthetic versions of the container and two
leaves, including comparison with full recomputation and failure recovery. It
does not validate real font inputs, paint records, or the broader sizing model.
These examples do not yet justify a public `Ui` facade.

| Contract | Required evidence |
| --- | --- |
| Parent and child access | Compile and execute nested calls without unsafe code or broad interior mutability; reject reentry |
| Interrupted execution | Restore payloads and retain required work after a callback panic |
| Layout consistency | Compare incremental geometry with a cache-disabled full recomputation after the same edits |
| Cache keys | Change width, content, font revision, and baseline inputs independently; compare results |
| Locality | Count measurement, placement, and paint-recording work separately; do not infer it from binding counts |
| Removal | Prove no stale callbacks or scene references survive; validate deferred resource lifetime separately |
| Resource cost | Measure resident storage, allocations, cache growth, and cleanup under repeated edits |

Geometry agreement is not visual acceptance. Pixel output, clipping, overlap,
damage, native input, and platform integration require later dedicated checks.
This document establishes the broader candidate contract. The linked experiment
documents distinguish their implemented subsets and evidence from unresolved
framework capabilities.
