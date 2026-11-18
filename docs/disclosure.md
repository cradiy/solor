# Disclosure interaction

Open the CPU text window in its interactive mode on Linux:

```sh
SOLOR_DISCLOSURE=1 cargo run --offline --release --example text_window
```

Click **Show details** to mount a detail label. Click **Hide details** to
remove it. The title's allocated text box is the hit target. Moving out of
the title during a press cancels that press, even if it returns before release.
Set `SOLOR_VERIFY_SHM=1` to check submitted pixels against a full redraw.

## Component and host

The fixture's private interface accepts primary pointer events in surface-local
logical units. A pointer retains the same identity from press through release
or cancellation. A host can drive the component in two separate callbacks:

```rust
// Input callback
needs_frame |= row.pointer(event);

// Frame callback, when the host can submit the result
let frame = row.frame(logical_size)?;
```

The component tests input against its last published title geometry, clipped
to the logical viewport. Only the pointer that started the press can activate
it. Leaving the surface or losing the input device cancels the press. A
successful release invokes the same semantic `activate()` action a different
input adapter could call. Keyboard focus, native touch and accessibility have
no adapters in this experiment.

A typed signal updates the title. The detail node subscribes to its content
on mount, and removing the subtree drops its binding. The title's state node
survives both transitions. The font engine is shared across transitions. The
layout scene contains a column for the title and detail. Expanding appends a
text leaf inside that column. Collapsing removes the leaf. The enclosing column
places its children using the current measured heights. The last published
frame stays valid if text preparation fails.

The Wayland host translates seat input, coalesces frame requests and waits
for compositor pacing and a free shared buffer. It converts the preferred
scale in 120ths into device pixels per logical unit for rasterization. The
component receives logical coordinates and viewport size.

## Verification

In a release build on Niri at 150%, a compositor virtual pointer clicked
the title four times. The two-row and one-row states alternated twice. A click
below the title and a press dragged outside and back produced zero additional
buffer commits. Every submitted buffer matched a full redraw. The original
text fixture also completed its initial commit and eight scripted edits.

Headless tests exercise mismatched pointer identities, explicit cancellation,
the exclusive right edge, and twelve mount/unmount transitions. They compare
geometry, text and retained pixels with independently prepared frames and
check that each removed detail handle is stale and its binding is gone.

## Dynamic list

```sh
SOLOR_DISCLOSURE=list cargo run --offline --release --example text_window
```

The interactive row sits between two static labels. All three persistent
labels keep their layout handles. The changed title is measured again.
Unchanged neighbor allocations reuse their measurement and layout bodies'
cached results. The lower label receives a new origin after the middle item's
height changes. Each frame retains the scene geometry and collects text commands through
the bottom of the logical viewport. Removing a leaf scans the scene's node index.

At widths of 240, 80 and 240 logical units, six alternating edits at each width
preserved both neighbors' paragraph handles and added zero executions of their
measurement or layout bodies. Tests compare every resulting geometry and pixel
buffer with a fresh forced layout and full raster at 150%. A separate test
checks rejection of obsolete dependency evidence and recovery after an
unrenderable detail. In a 400 by 220 logical Niri window at 150%, four clicks
produced 4, 3, 4 and 3 draws after the initial three. All submitted buffers
matched full rasterization. An outside click and a canceled drag added no
buffer commits.

### Update cost

```sh
cargo test --release --offline --example text_window measure_list_updates -- --ignored --nocapture
```

The release timing fixture uses a fixed 240 by 160 logical viewport. Each batch
alternates 80 open/close operations and prepares the resulting text frames.
The comparison reconstructs the same list for each transition with a warmed,
shared font engine. One warmup batch precedes seven measured batches. Execution
order alternates by batch. Across three processes, the median of process
medians was 16.4 µs with retained rows (16.3–16.8 µs) and 32.9 µs with
reconstruction (32.5–33.4 µs). This timing starts at the layout edit and ends
with the prepared snapshot. State dispatch, rasterization, uploads and
compositor presentation are outside it. These measurements cover reconstruction of the three-row fixture.

## Long list snapshot collection

```sh
SOLOR_DISCLOSURE=long-list cargo run --offline --release --example text_window
cargo test --release --offline --example text_window measure_snapshot_collection -- --ignored --nocapture
```

The long list has 1,024 labels when collapsed. Its second row expands. Both
columns place labels from top to bottom and clip ink to its allocation. The
snapshot builder stops when a row starts at or below the viewport bottom. It
keeps the partially visible last row and its full paragraph. A resize requests
a new snapshot. A held frame can still be replayed at its original viewport
after subsequent edits and destruction of the scene.

The following release measurements use a 240 by 160 logical viewport, eight
batches of 80 alternating edits, one warmup batch and alternating strategy
order. Both paths use retained layout and copy all published geometry. The
full reference collects commands for all rows. Times are medians of five
process medians. The edit interval includes topology changes, measurement,
placement, snapshot collection and replacement of the previous frame. It ends
before rasterization and native presentation.

| 1,024-row workload | Full command list | Viewport command list |
| --- | ---: | ---: |
| Commands in the collapsed frame | 1,024 | 6 |
| Snapshot collection and replacement | 19.38 µs | 2.08 µs |
| Edit through snapshot | 111.69 µs | 88.63 µs |
| Collapsed frame and vector capacity | 82,104 B | 33,288 B |

The byte counts exclude paragraph allocations, layout storage, and window
buffers. The three-row control retained all three commands and took 0.12–0.13
µs for snapshot collection in both paths. The flat layout visits the column's
children, and copying the complete geometry still takes work proportional to
the total node count. This experiment uses a viewport anchored at the top.

Tests compare damage updates with uncached rendering of a complete command
list after wrapping, height changes, detail edits and replay of a held frame.
At a boundary exactly at the title origin, hit testing has no published title.
In a 132 by 220 logical Niri window at 150%, opening and closing the detail
twice changed visible command counts from 9 to 8 and back. Shared-buffer
readback matched full rasterization of each submitted frame.

## Host-assigned root size

The disclosure window receives a logical size from the host. Its column uses
the assigned width to measure children for wrapping and to place each row.
The window can therefore omit the root's preferred-size query. A caller that
needs the root's preferred size can still request it, including immediately
after layout at the same allocation. Measurement evidence becomes available
when the corresponding query succeeds.

The `measure_snapshot_collection` command above also compares these two root
requests with the same viewport-limited commands and complete geometry.
The strategies rotate execution order by batch. Times below are medians
of five release-process medians, in microseconds per edit through snapshot
replacement. Brackets give the range of process medians.

| Collapsed rows | Measure root, then place | Place at host size |
| ---: | ---: | ---: |
| 3 | 16.52 [16.25–17.62] | 16.27 [15.99–17.49] |
| 1,024 | 86.67 [86.42–89.79] | 74.28 [73.22–76.68] |

The host-sized path reduced the long-list median by 14%. The three-row
measurements overlap. Placement and geometry collection still visit the
flat column’s rows on an edit. The comparison ends before rasterization or
compositor submission. Offscreen tests compare host-sized frames against
forced, measured layout and uncached rasterization across edits and resizes.

## Moving the rows after a disclosure

When two or more labels follow the disclosure, a column holds those labels at
group-local positions. The group receives the same inner width as the flat
list so wrapping uses the original constraint. Opening the detail moves the
group's origin. At the same width its children keep their identities, layout
and shaped paragraphs. The frame stores each tail row in list coordinates
and includes rows outside the viewport. The group itself is private to layout.

```sh
cargo test --release --offline --example text_window disclosure_layout::viewport_tests::measure -- --ignored --nocapture --test-threads=1
```

Both columns below use the host allocation. Each entry is the median of five
release-process medians, in microseconds from edit or resize through snapshot
replacement. Each process warms up for one batch and measures seven batches.
The edit workload rotates strategies and toggles the second row 80 times per
batch at 240 by 160. The resize workload alternates strategy order and uses
20 frames per batch, switching between widths of 84 and 240 at height 160.
Its tail alternates short labels and wrapping bilingual text.

| Workload | Flat rows | Grouped tail |
| --- | ---: | ---: |
| Toggle, 4 rows | 17.55 | 17.45 |
| Toggle, 1,024 rows | 75.14 | 22.49 |
| Resize, 1,024 rows, warmed widths | 63.41 | 61.42 |

For long-list toggles, process medians ranged from 74.12–76.00 µs flat and
22.25–23.51 µs grouped. Converting group-local positions back to list
coordinates increased snapshot collection from 2.08 to 3.76 µs. Total cost
fell because the parent could move all 1,022 following rows without querying
or placing them individually. On resize both hierarchies revisit the rows.
Their process medians overlapped (58.28–65.32 µs flat, 58.50–63.97 µs
grouped). The four-row edit ranges also overlapped. These timings end before
rasterization and presentation.
