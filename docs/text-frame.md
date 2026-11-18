# Atomic Text Drawing Snapshots

## Scope

The private `text_frame` example publishes an immutable drawing snapshot containing
geometry, ordered text commands, absolute origins, allocated sizes, RGBA values,
owned paragraph handles, and the embedded font bytes. It uses a fixed topology:
a column of shift groups, each containing one text leaf. A shift group reserves
16 units for internal horizontal movement without changing its layout output.

This is a publication and ownership experiment, not a public UI API, generic
scene graph, display-list optimizer, renderer, or native window. It does not
rasterize glyphs, apply clipping, blend colors, track pixel damage, implement
transforms, or transfer frames to another thread.

Acceptance requires same-size text changes, paint-only color changes, nested
movement, coherent geometry/resource publication, failure and panic recovery,
owned snapshot lifetimes, full-recomputation agreement, and measured collection
cost. The complete-snapshot implementation deliberately establishes a baseline
before any incremental command-list design.

## State and publication boundary

`Presentation` owns a core layout scene. Its externally presented state is
only `current()`, an optional `Rc<Frame>`. Input edits can advance internal layout
and text caches without changing that frame. Each successful publication replaces
one complete frame and increments its checked revision.

An update performs these steps under exclusive access:

1. Run normal, root-pruned, or forced layout using the requested allocation.
2. Request the scene's shared geometry in the fixture's stable topology order.
3. Prepare every text command with an absolute origin, allocated size, current
   color, and the paragraph for that allocation. Check its logical content extent
   and baseline against the corresponding geometry.
4. Replace the current frame only after all preparation succeeds, then clear
   pending drawing work and text-content dirty flags.

No fallible preparation or user callback runs after the replacement point. An
error or panic during preparation drops the candidate commands and preserves the
previous frame, including its geometry, colors, and paragraphs. Before the first
successful publication, `current()` remains empty.

Internal layout may already be ahead of the presented frame after a drawing
failure. That is intentional: the frame is the consumer's coherent publication
boundary; the layout scene is not externally exposed as current presentation.
Successful speculative text caches may also remain. A separate pending flag
ensures retry still prepares drawing commands when layout itself is already clean.
Layout checkpoints describe this internal layout state and need not roll back
when drawing preparation fails. Tests verify that retry publishes the complete
new frame without executing additional layout callbacks.

## Independent drawing updates

Content and style edits invalidate layout and mark the row's paragraph dirty.
A geometry change count of zero does not suppress drawing publication. The
same-size `中文` to `文字` case skips root layout while publishing new glyph IDs.

Color edits request a frame without invalidating layout or text. A moved group
changes descendant absolute origins without changing the paragraph. An unchanged
paragraph is reused from the previous frame when node identity, content/style
state, and allocated width remain compatible. This reuse survives clearing the
node's text cache. Width changes resolve the matching paragraph; final allocation
is authoritative.

The fixed fixture's command order is row order. Absolute text origins add root,
group, and text-local origins with checked arithmetic. Frames share the scene's
immutable geometry slice while membership and geometry are unchanged. A color
edit rebuilds the full command vector and shares the existing paragraphs. Its
command preparation remains linear in the row count. Clean updates return the
same frame handle.

The bundled font has static lifetime. Paragraphs and frame vectors have owned
lifetimes independent of the layout scene and font engine. Replacing the current
frame does not keep an internal history. External consumers may retain old frames;
their referenced paragraphs remain live until those handles are dropped. This
single-threaded `Rc` experiment does not define render-thread synchronization or
queue backpressure.

## Correctness

Eight focused tests cover:

- Same-size glyph replacement through root layout pruning, stable node identity,
  reuse of unaffected paragraphs, and the clean-frame fast path.
- Color-only updates with no layout callbacks, paragraph resolution, or shaping.
- Nested movement preserving paragraph identity and updating absolute coordinates.
- Errors and panics after the first or last command has been prepared, preserving
  every part of the old frame and successfully retrying after completed layout.
- A missing glyph after another row's color changed, with no partial publication.
- A failed resize, followed by successful matching geometry/paragraph publication;
  old frames surviving cache release and scene destruction, then releasing data.
- Failure before the first publication and a successful retry without layout work.
- 96 mixed updates compared with forced layout, fresh text buffers, and full command
  rebuilding. The oracle compares geometry, glyph data, absolute positions, colors,
  and font data while ignoring fixture-specific NodeIds and publication revisions.

The target also runs the existing staged-layout allocation test. The workspace
passes 99 tests, plus formatting, all-target checking, and clippy. This is evidence
about drawing inputs and transaction semantics, not rendered pixels.

## Complete update cost

Release comparisons use 64 and 512 text rows (129 and 1,025 layout nodes). Both
paths use the same shifted-column topology, content, text caching, and normal
layout mode. The baseline applies the same input edits and completes internal
layout, but does not collect or publish frames. It is an overhead baseline, not
an alternative rendering implementation. The root-pruned path is checked by the
correctness oracle separately.

After 20 warmup updates, seven samples of 31 updates alternate the two paths' order.
Timing includes edits, layout, snapshot construction, command preparation, and
publication as applicable. It excludes fixture construction, content comparison,
and retained-capacity reporting. Three sequential process runs exclude allocator
instrumentation. Values are medians of process medians in microseconds per update;
parentheses show the range of process medians.

| Rows / update | Layout only | Layout and frame |
| --- | --- | --- |
| 64 / color | 0.01 (0.01-0.01) | 1.26 (1.19-1.27) |
| 64 / content | 19.20 (19.13-20.33) | 20.50 (19.95-21.61) |
| 64 / internal motion | 4.56 (4.06-4.65) | 5.91 (5.38-5.99) |
| 64 / style | 25.89 (25.08-26.07) | 27.80 (26.87-28.82) |
| 64 / alternating width | 9.00 (8.96-9.13) | 10.47 (10.45-10.79) |
| 512 / color | 0.01 (0.01-0.01) | 9.91 (9.77-9.92) |
| 512 / content | 49.29 (48.89-50.62) | 61.11 (59.95-61.92) |
| 512 / internal motion | 35.09 (34.37-35.10) | 44.35 (44.01-44.70) |
| 512 / style | 64.68 (64.00-65.08) | 78.11 (75.67-79.28) |
| 512 / alternating width | 79.64 (78.19-79.94) | 87.65 (87.21-88.70) |

Idle calls publish nothing and report about 0.01 us in the frame path; resolution
at that scale does not support a detailed timing claim. Color and motion updates
resolve zero paragraphs. Local content/style edits resolve one, and width changes
resolve all rows. Resolution may be a paragraph-cache hit, not a shaping execution.

The measured overhead is about 1-2 us at 64 rows and 8-13 us at 512 rows. It is
acceptable for this complete-snapshot baseline, but scales with the full fixture.
These numbers exclude rasterization and must not be compared directly to the
unshifted topology in the earlier text experiment.

Four paired release processes compared the same color workload against
`1ec7331`, alternating execution order. At 64 rows the median frame time
fell from 1.10 to 0.53 µs, and at 512 rows from 9.26 to 4.06 µs.
The 512-row process medians ranged from 8.29 to 11.68 µs and 4.04 to
4.19 µs respectively. A separate allocator probe on a warmed 64-row color
edit counted four allocations requesting 7,320 bytes at the baseline and
two requesting 3,160 bytes with shared scene geometry.

## Storage and reclamation

The following allocation measurements describe the complete-snapshot baseline
with an inline vector header. Current per-frame shallow storage counts the
geometry slice payload once per reported frame, even when frames share it.
It excludes reference counts and allocator overhead.

An independent allocator probe passes its Rust positive control (one 123-byte
request, 136 usable bytes). A 64-row frame's geometry and command payload is
7,272 bytes, excluding shared paragraphs and allocator headers; 512 rows require
57,448 bytes. A warmed color update makes three allocations requesting 7,288
bytes and changes live usable heap by 16 bytes. An idle update makes zero
allocations and leaves live heap unchanged.

For 64 rows, fixture construction plus 100 mixed updates retains 468,200 usable
bytes. After the color update, two further blocks of 1,000 mixed updates change
retained storage by +55,400 and +4,128 bytes. Different text-cache contents and
capacity high-water marks remain; these observations do not establish zero growth
or global leak freedom. The fixture retains one current frame, bounded per-node
paragraph caches, and engine/layout storage.

At the final boundary, live measured storage totals 527,744 bytes. Clearing node
caches and layout scratch releases 234,400 bytes. Destroying the scene while a
consumer retains the current frame releases 156,760 bytes. Dropping that final
frame releases the remaining 136,584 bytes. Thus the remaining drawing resources
are owned by the snapshot, and all measured fixture storage is accounted for.
These are allocation traffic and usable-heap observations, not peak memory or
RSS/PSS. Caller-retained frame history is a separate ownership cost.

## Decision and reproduction

The publication boundary meets this milestone: drawing changes are independent
of layout equality, consumers retain a coherent old frame on failure, and owned
resources live for exactly as long as their handles require. Keep full snapshot
replacement as the measured baseline. The [CPU text pixel experiment](text-pixels.md) checks clipping and raster output independently; this result does not require command-list optimization.

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-ui --example text_frame --offline
```

The executable runs the mixed-update oracle, paired timing, and separate allocator
markers. It produces no window or image.
