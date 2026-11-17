# Layout Execution Contract Experiment

## Scope

This experiment tests parent/child layout dependencies and completion using the
scoped node store. It is a headless executable, not a public widget API. Its
fixtures are vertical containers, a synthetic wrapping leaf, and a fixed-size
drawing leaf. The wrapping leaf computes integer cell counts; it does not shape
text. The drawing leaf has no renderer.

The initial sizing domain is deliberately narrow: a positive available width,
content-dependent height, integer coordinates, and an optional first baseline.
Minimum/maximum and intrinsic-size queries, flex distribution, height constraints,
overflow policy, transforms, and pixel damage are not selected by this experiment.
Container padding must fit the available width. Arithmetic overflow and invalid
metrics are errors, not silently accepted geometry.

## Ownership and candidate interface

| Owner | Data |
| --- | --- |
| Node implementation | Typed inputs and behavior-specific state |
| Framework entry | Measurement cache, measurement/layout invalidity, execution counters, last published local geometry |
| Parent implementation | Child sizing queries, assigned sizes, and relative origins |
| Layout attempt | Unpublished geometry changes and child-placement validation |
| Scene coordinator | Pending layout, last successful root width, and publication boundary |

The experiment uses these private interfaces:

```rust,ignore
trait LayoutNode {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query)
        -> Result<Metrics, LayoutError>;
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size)
        -> Result<(), LayoutError>;
}
```

Measurement can query direct children, but cannot place them. Layout can measure
and place direct children, but cannot edit topology or another node's inputs.
Each direct child must be placed exactly once in this fixture contract. This is
not a decision about virtualized, hidden, portal, or out-of-flow children.

A container measures its children under its inner width, computes its own height
and baseline, then uses those results to assign child sizes and relative origins
during layout. The fixture assigns measured sizes directly. The separation of
query and assignment does not establish support for arbitrary assigned sizes.

## Invalidation and reuse

Input changes that can affect layout invalidate the target and every ancestor's
measurement and layout. The initial policy does not stop propagation when the
remeasured size happens to be unchanged. Baselines and child placement can matter
even when outer dimensions do not change.

Measurement reuse requires a matching width and no outstanding input/dependency
invalidation. All layout-affecting edits go through a boundary that invalidates
before invoking the mutation callback, so a callback panic cannot leave a clean
cache after a partial edit. Synthetic font metrics are explicit leaf inputs;
changing them uses the same invalidation path.

Layout reuse requires clean layout state and matching assigned metrics. A parent
can change a child's relative origin without executing that child's layout body
when the child's local geometry remains valid. Descendant origins remain relative
to their own parents; no absolute-coordinate composition is implemented.

An affected container still enumerates its children and queries cached metrics.
Skipping a clean subtree's behavior is not equivalent to eliminating all access
to that subtree. No per-edge dependency graph or changed-size early stop is
implemented. The measurement cache holds one query result per node.

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

The access prototype has a fixed access-error type, so layout callbacks return a
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

Work counters distinguish measurement bodies, layout bodies, and published
geometry entries. They establish execution locality, not frame-time performance.
The journal and placement validation use scratch storage proportional to visited
work. No whole-tree geometry clone is required by a normal layout attempt.

## Implementation and validation

The private [execution code](../crates/solor-core/examples/layout_contract/layout.rs)
uses the existing scoped access prototype. The
[fixtures](../crates/solor-core/examples/layout_contract/fixtures.rs) implement
the three behaviors, and the
[tests](../crates/solor-core/examples/layout_contract/tests.rs) cover the acceptance
cases. Nodes can be appended for fixture construction; topology is locked after
the first successful layout. Layout updates do not add or remove nodes.

```sh
cargo run -p solor-core --example layout_contract --offline
cargo test -p solor-core --example layout_contract --offline
```

Eleven focused tests include a 256-step comparison with all measurement and layout
caches bypassed. The reference uses the same fixture algorithms, so known-value
assertions independently check wrapping height, baseline, and sibling origins.
The reference can perform repeated measurements; it is a correctness oracle,
not an optimized full-layout performance baseline.

The seven-node executable fixture reports three measurement bodies, three layout
bodies, and five published entries after a text-unit edit. An ink-only edit
executes neither measurement nor layout. Error/panic tests retain the previous
snapshot and retry successfully, including a failure after an earlier subtree has
finished its layout but has not been published. These results validate the narrow
contract, not real font metrics or rendered output.

The milestone ends at these checks. Public API selection, layout algorithms for
real controls, actual font measurement, topology edits during updates, and
integration with signals remain separate decisions.
