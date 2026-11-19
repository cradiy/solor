# Node Execution Model Draft

## Status and scope

This draft proposes how components participate in the GUI runtime. The core
already provides a persistent tree, scoped node execution, explicit signal
bindings, and transactional layout. The Linux disclosure example connects these
pieces with private state and input handling. Component ownership and event
delivery across structural edits are design proposals.

See the [node store](node-store-experiment.md), [layout contract](layout-contract.md)
and [disclosure interaction](disclosure.md) for the implemented boundaries.

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
image resource is not destroyed merely because one consumer is removed. The lifecycle proposal below defines ownership at mount and disposal. The
existing state runtime uses explicit signal removal.

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
synchronous geometry query needs an explicit completion barrier. The
[window frame acceptance proposal](window-frame-commit.md) defines which
snapshot drives hit testing and who retries an incomplete frame.

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

### Component ownership and node identity

The proposed unit of lifetime is a **mounted component scope**. A scope can
install several sibling nodes under an existing container and own their
bindings. A pure composition helper can create nodes in its caller's scope.
A component contributes a container node when it needs to arrange children,
clip drawing, or provide an input target of its own. Grouping two siblings
under one owner adds no measurement callback or hit region.

Each mounted node has one owning scope and one position in the visual tree.
The visual parent determines layout and paint order. The scope determines
which registrations and child scopes end together. Removing an optional node
cancels work targeting that node while its owner's other nodes remain live.
Disposing the scope removes all its contributions, including sibling roots.
A signal supplied by a parent remains usable by surviving subscribers. A
signal created for the scope's own lifetime is released on disposal.

Flutter gives widget composition and render objects different roles. React
Fragments group siblings without introducing a wrapper element. These examples motivate grouping by lifetime independently of visual
geometry. [Inside Flutter](https://docs.flutter.dev/resources/inside-flutter#separation-of-the-element-and-renderobject-trees)
and [React Fragments](https://react.dev/reference/react/Fragment).

### Applying the contract to Solor

Today the disclosure binds a label in `Runtime<Label>` and separately mounts a
text leaf in `Scene`. Each store issues tree-local handles. The example tracks
the mounted detail and synchronizes the two during frame preparation. Its press
state stores a pointer ID because the title is its only interactive target.
[Disclosure implementation](../crates/solor-render-cpu/examples/text_layout/disclosure.rs)

The proposal gives each interactive visual node one identity for the lifetime
of that mount. Bindings that update it, input sequences and published hit
records refer to that identity. A geometry slice position can change after a
removal and cannot identify an event recipient. Keep the state and layout
stores in place for the first validation. Where their handles differ, the
mount owns the association and retires it when detached. The validation should
establish whether this association is sufficient before changing either store.

### Removing a pressed child

Consider one scope owning a persistent label and an optional interactive leaf.
Both subscribe to a parent-owned signal. The leaf has a pending update and a
pointer is held down over it.

1. On press, hit-test the frame the host has accepted as current for that
   window. Record the pointer identity and the leaf's live mount identity.
   A frame still being prepared has no authority over input.
2. A callback requests removal. Apply it at the update boundary after active
   callbacks return, before running queued bindings for removed targets.
   Retire the leaf as an input recipient and clear its press. Cancel its
   bindings and pending work, detach it from layout, then release its payload.
   Input dispatch cannot interleave these steps. The label keeps its identity
   and its queued update still runs.
3. The next frame fails during drawing preparation. The previous frame retains
   its geometry and drawing resources and can still be replayed. Its hit record
   refers to the retired identity. A fresh press there yields no target for
   that hit, even if a live sibling now lies underneath. A release from the
   canceled sequence produces no activation. Rendering failure leaves the
   removal in effect and frame preparation pending for retry.
4. Remount a leaf at the same coordinates, even in the same storage slot. It
   receives a fresh identity. After the host accepts the replacement frame,
   a new press can reach it. A release belonging to the original press cannot
   activate it, whether it arrives before or after that frame.

Flutter likewise associates pointer moves and releases with the down hit test.
Solor's proposed live-identity check and removal behavior are explicit choices
for this runtime. [Flutter pointer dispatch](https://api.flutter.dev/flutter/gestures/GestureBinding/handlePointerEvent.html)

### Headless lifecycle evidence

The [lifecycle fixture](../crates/solor-render-cpu/examples/text_layout/lifecycle_tests.rs)
mounts overlapping text leaves under one layout parent. Each mount pairs its
visual handle with a state handle and binding. Leaves subscribe to a
parent-owned color signal. Input resolves the frontmost hit in the accepted
frame, then checks that mount's live membership.

The removal test queues a color change for both leaves, retires the pressed
leaf, and injects a drawing error after layout. The delivery log records one
drop for the retired payload and one binding invocation for its sibling. The
previous frame replays to identical RGBA bytes after failure and after teardown.
Retry uses the updated color. A prepared frame takes over hit testing only
when the simulated host accepts it. Releasing the original press on either
side of accepting a same-position replacement produces zero actions. A fresh
press and release delivers one action to the replacement.

Across 32 whole-scope disposals the test checks both payload drops, empty
membership and press state, zero bindings and queued work, and continued
access to the shared signal. These are live object counts.

In the nested cases each scope has a generational handle to a state-tree node.
Child scopes and bound payloads attach under their owner. All visual leaves
remain siblings under the same layout parent. Disposal walks the state subtree
and marks its bound payloads. It retires affected presses, bindings and visual
leaves before removing the state subtree and flushing remaining bindings. A queued disposal
whose handle is already stale has no effect.

Child-first disposal preserves the parent's queued delivery. A remounted child
survives a late request for its predecessor and is released exactly once with
the parent. Parent-first disposal also removes live child contributions when
their visual positions interleave with an unrelated owner's. That owner's
queued binding and active press still complete. Frame acceptance is simulated.
Native input timing, focus and keyed moves remain open.

```sh
cargo test --offline --example text_window lifecycle_tests
```

### Cost of disposing a scope

The [headless timing test](../crates/solor-render-cpu/examples/text_layout/lifecycle_tests/disposal_cost.rs)
compares walking ancestors for every live mount with visiting the target scope's
existing child links. The latter sets one boolean per visited label, then checks
the marks in visual order while compacting the mount list. It keeps no second
per-scope membership list. Traversal uses a temporary stack of pending nodes.

Each release process runs one warmup and seven measured batches per case,
alternating strategy order. Each batch starts with a completed layout, a cached
snapshot and one queued binding per leaf. Target scopes span the visual order,
with owned leaves interleaved with survivors and distributed across two scope
levels. Discovery and disposal use separately constructed fixtures. Discovery
materializes the affected identities. Disposal removes up to 16 disjoint scopes
in succession and includes finding members, compacting lists, canceling
bindings, detaching layout leaves and dropping state payloads. Subsequent
layout, painting and submission are outside the interval. Checks verify the
surviving order, queued work, dropped identities and final geometry count.

Median of five process medians, in microseconds per scope:

| Initial mounts | Leaves per scope | Discover by ascent | Discover by subtree | Dispose by ascent | Dispose by subtree |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 8 | 1 | 0.14 | 0.19 | 0.50 | 0.59 |
| 128 | 1 | 0.43 | 0.09 | 1.10 | 0.66 |
| 1,024 | 4 | 3.90 | 0.14 | 10.63 | 6.07 |
| 8,192 | 1 | 27.83 | 0.22 | 60.03 | 22.91 |
| 8,192 | 4 | 28.61 | 0.27 | 81.29 | 43.50 |
| 8,192 | 64 | 33.37 | 0.70 | 459.85 | 418.29 |
| 256 | 64 | 1.29 | 0.59 | 27.25 | 25.94 |

For four leaves among 8,192, complete disposal ranged from 80.12 to 87.13 µs
by ascent and 42.57 to 46.07 µs by subtree. For one leaf among eight the
subtree path cost about 0.09 µs more. The fixture uses subtree traversal for
both sizes. The ordered mount list still requires one scan, and each layout
leaf removal scans the scene's node index and its parent's child list. The
timings do not isolate these remaining costs individually.

```sh
cargo test --release --offline --example text_window measure_scope_disposal -- --ignored --nocapture
```

## Validation before implementation expands

The core node store implements scoped parent and child access. The
headless layout experiment tests synthetic versions of the container and two
leaves, including comparison with full recomputation and failure recovery. It
does not validate real font inputs, paint records, or the broader sizing model.
The [authoring library](component-api.md) exercises composition and local paint on the core layout path.

| Contract | Required evidence |
| --- | --- |
| Parent and child access | Compile and execute nested calls without unsafe code or broad interior mutability; reject reentry |
| Interrupted execution | Restore payloads and retain required work after a callback panic |
| Layout consistency | Compare incremental geometry with a cache-disabled full recomputation after the same edits |
| Cache keys | Change width, content, font revision, and baseline inputs independently; compare results |
| Locality | Count measurement, placement, and paint-recording work separately; do not infer it from binding counts |
| Removal | Reject delivery to retired targets and retain the resources required by held frames |
| Resource cost | Measure resident storage, allocations, cache growth, and cleanup under repeated edits |

Geometry agreement is not visual acceptance. Pixel output, clipping, overlap,
damage, native input, and platform integration require later dedicated checks.
This document establishes the broader candidate contract. The linked experiment
documents distinguish their implemented subsets and evidence from unresolved
framework capabilities.
