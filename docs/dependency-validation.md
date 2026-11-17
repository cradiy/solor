# Dependency Result Validation

## Boundary

The private headless experiment can audit the child results consumed by a cached
measurement or published layout. It does not stop invalidation, skip a parent
callback, or publish speculative geometry. Normal layout remains responsible for
completing every update. The public core is unchanged.

The problem is to distinguish a changed dependency from an unchanged dependency
without mistaking missing evidence for independence. Acceptance includes exact
query matching, final baseline and content comparison, owner-input changes,
cache eviction, incomplete records, internal movement, and retry after errors or
panics. Stop at that contract before integrating propagation pruning.

## Capturing and checking evidence

`Scene::capture_dependencies(owner, target)` returns an owned `Checkpoint` for
one currently valid result. `CheckTarget` is either `Measure(Query)` or
`Layout(Size)`. Capture requires the first successful layout, which locks topology.
A dirty result, absent measurement-cache entry, or different published allocation
returns `None`. Invalid or foreign handles return an error.

A checkpoint copies that result's direct-child reads and retains its owner,
request, and owner-input revision. Later cache eviction does not destroy this
evidence. Callers own its lifetime; capture is explicit and does not run on the
normal update path. A complete empty record remains distinct from an incomplete
record whose buffer was discarded at the recording limit.

`Scene::check_dependencies(checkpoint, target)` checks the requested target and
owner revision before replaying reads in their original order:

| Outcome | Meaning |
| --- | --- |
| `Unchanged` | All recorded child results are equal, with complete evidence and matching owner inputs and request. |
| `Changed` | A consumed child result differs; checking stops at the first difference. |
| `Unverifiable` | The owner inputs or request differ, or the record is incomplete. No child callbacks run. |

Measurement reads compare complete `Metrics`, including baseline, for the exact
recorded query. Layout reads compare complete `LayoutOutput`, including baseline
and content extent, for the recorded allocation. `Changed` does not establish
that the parent's outward result must change. `Unchanged` does not establish
that descendants have no geometry to publish.

The request argument describes the result being audited, not necessarily the
scene's latest root request. Historical requests can be replayed explicitly;
their verdict is not permission to reuse a result for a different request.

Each node has a `u64` owner-input revision. `edit_layout` increments it before
the mutation callback, including callbacks that panic. Ancestor invalidation
does not increment ancestors' own revisions. Revision exhaustion rejects the
edit before mutation. Paint-only edits do not change this revision. As with the
existing caches, behavior must compute from its declared inputs and child reads;
untracked external changes or application-input mutation inside layout callbacks
are outside the contract.

## Publication and failures

Before nonempty replay, the owner and its ancestors are conservatively marked
dirty. Child measurements may update private caches. Child layouts use the
existing scratch journal, but every draft is discarded after the check. No
geometry or published layout records change, even on success.

Errors propagate and panics unwind through the scoped node loans and scratch
guard. The last published state remains intact, pending work remains set, and
retry can reuse successful measurements. Normal layout must run afterward even
when the verdict is `Unchanged`. This preserves internal child movement behind
an unchanged container output; treating the verdict as a clean-subtree flag
would lose that movement.

The report counts checked direct reads and staged journal entries. It is an
observation of that check, not a durable validity token across subsequent edits.

## Validation and work cost

Six focused tests cover separate measurement/layout verdicts, final baseline and
overflow changes, hidden child movement, retained checkpoints after cache eviction,
exact requests, interrupted owner edits, foreign owners, incomplete evidence,
and measurement/layout failure or panic after earlier work. The subsequent normal
layout is compared with cache-disabled recomputation. The example also demonstrates
a drawing growing from width 20 to 21 under a width-10 constraint: measured width
stays equal while the consumed layout content changes.

The cost fixture checks the stable-label workload with both one and three cache
slots. After changing one label from 80 to 81 units, both root checkpoints report
`Unchanged` and publish no geometry. With three slots:

| Nodes | Recorded reads checked | Child queries executed | Measurement bodies | Layout bodies | Staged entries |
| --- | --- | --- | --- | --- | --- |
| 49 | 36 | 32 | 3 | 2 | 15 |
| 10,001 | 7,500 | 5,008 | 3 | 2 | 2,503 |

Counts include both audits, exclude checkpoint capture, and precede normal
reconciliation. The owner callback is not executed by the audit. The read count
includes layout reads, whereas child query counts include nested measurement work.

This is a linear replay reference, not a performance optimization. Checking a
large parent's entire read list still visits unaffected siblings, and discarded
layout work can execute again during normal reconciliation. Each entry gains
one `u64` revision; capture copies read buffers and retains them until checkpoints
are dropped. The per-record cap remains 16,384 reads, with no scene-owned history
of checkpoints. Allocator rounding and scratch capacity affect total live heap.

An external allocator probe on the release three-slot executable measured the
10,001-node checkpoint pair at two allocation calls, 360,000 requested bytes,
and 360,016 additional live usable bytes. Replaying the pair used three allocation
calls requesting 768 bytes, with zero net live-heap increase at the end marker;
the fixture's scratch was already warm. Zero net increase does not mean zero
temporary allocation or zero peak memory. These numbers exclude fixture setup,
snapshot copies, and the following normal layout. A separate Rust allocation
control observed one 123-byte request with 136 usable bytes. The one-slot variant
checked the same 7,500 reads but ran five measurement bodies instead of three.

The cost executable emits separate `dependency-checkpoints` and
`dependency-replay` allocation markers after the timed workloads for the large
fixture. Snapshot comparisons and normal reconciliation are outside these marker
pairs. Timed workload results remain normal updates without audits; they do not
measure an optimized validation-and-pruning path or establish a speedup.

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-core --example layout_contract --offline
cargo run --release -p solor-core --example layout_cost_one --offline
cargo run --release -p solor-core --example layout_cost_three --offline
```

The [selective-check experiment](selective-dependency-checks.md) indexes captured
reads and routes bounded edit history to affected children. It measures full
updates, including checkpoint replacement, while retaining normal reconciliation
for atomic publication. It is not selected for automatic runtime validation.
The [shared-attempt experiment](shared-layout-attempt.md) combines root audits
with normal layout so adopted subtree results can publish in the same update.
