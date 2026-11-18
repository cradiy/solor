# Layout Execution Contract Experiment

## Scope

This experiment tests parent/child layout dependencies and completion using the
scoped node store. It is a headless executable, not a public widget API. Its
fixtures include vertical containers, a synthetic wrapping leaf, a fixed-size
drawing leaf, and an icon/label/button row. The wrapping leaf computes integer
cell counts; it does not shape text. The drawing leaf has no renderer. The
[size negotiation contract](size-negotiation.md) specifies the row and query rules.

The sizing domain is deliberately narrow: natural, maximum, or exact width queries,
content-dependent measured height, floating-point logical coordinates, and an optional first
baseline. Zero-width boxes and allocations different from measured sizes are
valid. Content extent can exceed the allocation. General two-axis constraints,
min-content sizing, flex distribution, transforms, and pixel damage remain outside
the experiment. Arithmetic overflow and invalid metrics are explicit errors.

## Ownership and library interface

| Owner | Data |
| --- | --- |
| Node implementation | Typed inputs and behavior-specific state |
| Framework entry | Measurement cache, measurement/layout invalidity, execution counters, last published local geometry |
| Parent implementation | Child sizing queries, assigned sizes, and relative origins |
| Layout attempt | Unpublished geometry, final child outputs, and placement validation |
| Scene coordinator | Pending layout, last successful root query/allocation, and publication boundary |

Node implementations provide both callbacks through `solor_core::layout::LayoutNode`.

```rust,ignore
trait LayoutNode {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query)
        -> Result<Metrics>;
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size)
        -> Result<LayoutOutput>;
}
```

Measurement can query direct children, but cannot lay out or place them. Layout
can measure direct children, execute their layout with an assigned size, and then
place them using final outputs. It cannot edit topology or another node's inputs.
Each direct child must be laid out and placed exactly once. This is not a decision
about virtualized, hidden, portal, or out-of-flow children.

A container measures its children, computes a proposed size, and then negotiates
child assignments using its actual allocation during layout. Child layout returns
the final baseline and content extent; the parent uses them to decide relative
origins. The framework retains the assigned size rather than copying a previous
measurement's size or baseline into published geometry.

## Invalidation and reuse

Input changes that can affect layout invalidate the target and every ancestor's
measurement and layout. The initial policy does not stop propagation when the
remeasured size happens to be unchanged. Baselines and child placement can matter
even when outer dimensions do not change.

Measurement reuse requires a matching query kind/value and no outstanding input/dependency
invalidation. All layout-affecting edits go through a boundary that invalidates
before invoking the mutation callback, so a callback panic cannot leave a clean
cache after a partial edit. Synthetic font metrics are explicit leaf inputs;
changing them uses the same invalidation path.

Layout reuse requires clean layout state and matching assigned size. A parent
can change a child's relative origin without executing that child's layout body
when the child's local geometry remains valid. Descendant origins remain relative
to their own parents; no absolute-coordinate composition is implemented.

An affected container still enumerates its children and queries cached metrics.
Skipping a clean subtree's behavior is not equivalent to eliminating all access
to that subtree. No per-edge dependency graph or changed-size early stop is
implemented. The measurement cache holds at most three query results per node,
using FIFO replacement; input/dependency invalidation discards all three.

A drawing-only ink change does not request layout. This tests layout exclusion,
not paint scheduling, drawing correctness, or damage handling.

## Completion and failure

Successful measurements may remain cached after a later failure: they describe
the current inputs and do not expose new geometry. A failed measurement keeps its
entry invalid. Layout results are journaled and are published only after the
entire root operation succeeds. Layout invalidity is cleared at publication, not
when an individual child returns.

On a returned error or panic unwinding:

- The node-store guard restores all loaned payloads.
- Previously published geometry remains unchanged.
- The attempt's unpublished changes are discarded.
- The coordinator remains pending; retry is allowed at the same width.
- Partially mutated behavior state is not rolled back.

A failed context operation is terminal for that attempt, even if node code catches
its returned error or panic and returns success. Contexts retain a failure marker
and reject further child operations. A new attempt is required for retry. This
avoids publishing journal entries left by an abandoned nested operation; recovery
and alternate layout within the same attempt are outside this contract.

The node store has a fixed access-error type, so layout callbacks return a
nested layout result. Its `Status` describes the access callback, not successful
layout. Layout validity and pending work are tracked independently and must not
be inferred from `Status::Ready`.

Publication writes only framework-owned, copyable geometry and flags for known
live nodes. It performs no node callbacks or structural edits. This single-threaded
boundary is not a concurrent scene snapshot or a renderer resource transaction.

## Acceptance

1. Check known dimensions, baselines, and offsets independently of the algorithm.
2. Compare incremental geometry with cache-disabled recomputation after width,
   text-unit, font-metric, drawing-size, padding, and gap changes.
3. Check that ink-only edits execute no measurement or layout, and that unrelated
   subtree bodies remain unexecuted when a sibling changes.
4. Check movement of subsequent siblings and propagation of baseline-only edits.
5. Inject measurement/layout errors and panics after partial work; require an
   unchanged published snapshot and a successful retry matching full recomputation.
6. Reject invalid child placement and invalid sizing without partial publication.

Work counters distinguish query requests, cache hits, measurement bodies, layout
bodies, and changed geometry entries at publication. They establish execution
locality, not frame-time performance. A staged entry with unchanged geometry can
complete layout without being counted as changed publication.
The journal and placement validation use scratch storage proportional to visited
work. Scene-owned buffers retain capacity across attempts and clear active entries
on every exit. `release_scratch` reclaims capacity without invalidating geometry
or cached measurements. See [scratch reuse](layout-scratch.md) for lifetime rules
and the measured memory tradeoff. No whole-tree geometry clone is required by a
normal layout attempt.

## Implementation and validation

The [library solver](../crates/solor-core/src/layout/engine.rs) backs the text,
frame, pixel and Wayland examples. `Scene::run` measures the root and uses its
preferred size or the caller's allocation. `Scene::run_assigned` arranges it
directly. `Scene::geometry` reads the last successful result in parent-local
coordinates. `edit_layout` checks the handle and concrete type before marking the
node and its ancestors dirty. `body` provides shared access for drawing.

After a successful layout, `Scene::shared_snapshot` returns an owned, immutable
slice of local geometry in surviving insertion order. Positions refer to that
snapshot's topology. Use `Scene::geometry(id)` for a live node lookup.
Construction, pending edits and failed attempts return no current shared
snapshot. A consumer holding an earlier slice can keep reading it throughout
an update and after the scene is dropped.

The first request collects the slice. Repeated requests, including after a
forced layout with equal geometry, clone the handle. Changing any published
geometry or mounting or removing a node invalidates the cached slice. Removing
a zero-height leaf can leave the layout change count at zero and still change
the snapshot's membership. The scene retains at most one shared slice.

In a release allocation probe with 1,024 leaves, 100 warmed alternating height
edits and assigned layouts without snapshot requests made zero heap allocations.
The first request for the 1,025 entries made one allocation of 32,816 bytes.
The next 100 requests made zero allocations. The probe counted allocator
requests and detected its one-allocation, 123-byte positive control.

The default build uses three measurement slots per node. The optional
`layout-audit` feature adds dependency recording, checkpoints and explicit
validated/pruned execution. The historical audit and cache-size fixtures compile
the same solver at their selected capacity. The
[fixtures](../crates/solor-core/examples/layout_contract/fixtures.rs) implement
the three behaviors, and the
[tests](../crates/solor-core/examples/layout_contract/tests.rs) cover the acceptance
cases. Construction-time append closes after the first successful layout.
The [disclosure fixture](disclosure.md) explicitly mounts a new child or
unmounts a leaf between updates. These operations invalidate layout and
measurement up the parent chain and invalidate outstanding audit checkpoints when recording is enabled.
The component fixture uses `mount_at` to insert at a live sibling index and
`unmount_subtree` to retire a complete conditional child. Surviving handles
keep their identity. The scene compacts its node registry on removal.
Previously shared geometry slices remain readable. Layout callbacks cannot edit
the tree.

```sh
cargo test -p solor-core --test layout --offline
cargo run -p solor-core --example layout_contract --features layout-audit --offline
cargo test -p solor-core --example layout_contract --features layout-audit --offline
```

The default-feature library integration test fails the second child after staging
a height change in the first. It checks the prior snapshot, retries at the same
allocation, and compares the result with forced recomputation. A release build
of the default-feature Wayland window exercised the 1,024-row disclosure at
150% scale. All 23 submitted buffers matched the full raster reference, through
scrolling to offset 27,429 and back, opening the detail, and closing it.

Fifty-four focused tests in the audit fixture include a 256-step vertical comparison and a 384-step row
comparison with all measurement and layout caches bypassed. The reference uses
the same fixture algorithms, so known-value assertions independently check
wrapping height, baseline, sibling origins, final allocations, and content extent.
The reference can perform repeated measurements; it is a correctness oracle,
not an optimized full-layout performance baseline.

The seven-node executable fixture reports three measurement bodies, three layout
bodies, and five published entries after a text-unit edit. An ink-only edit
executes neither measurement nor layout. Error/panic tests retain the previous
snapshot and retry successfully, including a failure after an earlier subtree has
finished its layout but has not been published. These results validate the narrow
contract, not real font metrics or rendered output.

The core owns the layout execution contract. Container policies, font shaping and
connections between application signals and layout edits belong to their callers.

The [layout cost experiment](layout-cost.md) measures the same execution path
with larger fixtures, including invalidation, scratch allocation, and publication.
The [dependency-boundary investigation](layout-dependencies.md) adds executable
counterexamples and a stable-geometry workload before ancestor pruning is attempted.
The [recording experiment](dependency-recording.md) attaches reads to individual
measurement results and publishes layout records atomically with geometry.
The [result-validation experiment](dependency-validation.md) audits those records
without pruning ancestors or publishing speculative child geometry.
The [selective-check experiment](selective-dependency-checks.md) compares indexed
checks with linear replay and normal layout across complete updates.
The [shared-attempt experiment](shared-layout-attempt.md) adopts audited subtree
layouts within one update and publishes only drafts used by normal layout.
The [parent-layout skip](parent-layout-skip.md) retains a validated root layout
while publishing audited child changes. It also checks publication identity,
standalone-audit cursors, and preserved placement at nonzero parent origins.
The [fixed-font text integration](text-layout.md) exercises the same contract with
real shaping, line breaks, and owned paragraph caches. Its text fixtures and
performance results remain separate from the synthetic layout workload.
The [text drawing snapshot experiment](text-frame.md) retains geometry and drawing
resources together across preparation failures, even after internal layout succeeds.
