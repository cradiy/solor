# CPU text pixels

The private `text_pixels` example consumes the published [text frame](text-frame.md)
directly. Each call starts with opaque white RGBA8 pixels, loads face zero from
the snapshot's fixed font bytes, and paints the supplied glyph IDs in command
order. Swash supplies unhinted grayscale outline masks. PNG encoding is used for
the sample output. Both dependencies are development-only.

The offscreen benchmark uses device scale one. Glyph origins include the command origin, the line
baseline and the size-scaled shaping offsets. The renderer rounds the absolute
glyph origin to the nearest pixel and applies the signed bitmap bearings. A
half-open scissor intersects the surface, the allocated text box and each glyph
mask. Coverage modulates text alpha, and source-over blending operates on the
encoded color channels of the opaque destination. These are explicit policies
of this fixture. The [Wayland text window](text-window.md) uses the same renderer at
the preferred surface scale. Transparent targets remain outside this experiment.

Each draw creates one surface (1 through 4,194,304 pixels), a scale context and
one reusable scratch mask. The uncached path rasterizes each visible command’s glyphs anew.
The 256 by 244 timing surface occupies 249,856 bytes before rasterizer scratch.
Repeated redraws can be performed after the layout scene and its caches are dropped.

Four pixel tests check equal-geometry CJK replacement with unchanged sibling
pixels and exact replay of an older frame after scene destruction, integer
translation and scissor equivalence, two identical lines with a 24-pixel
baseline interval and a lowercase descender, and ordered translucent composition
clipped to the text allocation. The blend reference derives coverage from a
separate black-on-white rendering of the same glyph.

## Release measurement

Each iteration edits the first mixed Latin/CJK label, publishes its snapshot,
then redraws a complete 256 by 244 surface. The other seven labels in the larger
fixture also draw on each iteration. Timings include allocation, clearing and
dropping the RGBA surface and rasterizer scratch. PNG encoding and scene
construction are outside the timed region. Each process reports medians of seven
ten-update batches with two warmup updates per batch.

| Text rows | Edit through snapshot (µs) | Full CPU redraw (µs) |
| --- | ---: | ---: |
| 1 | 15.30 (14.36–20.34) | 39.28 (37.26–53.69) |
| 8 | 15.81 (15.80–15.83) | 324.39 (313.45–326.11) |

Values give the median and range across three sequential release processes.
Peak RSS for the executable including both workloads and PNG output was
5,788–5,836 KiB. This includes the fixed font subset, layout, encoder, and
process runtime. Raster time dominates the eight-row fixture even when only
one label changes. These figures provide a full-redraw baseline for future
comparisons on the same viewport and text.

## Reusing glyph masks

The renderer can retain outline masks across frames. The fixture uses face zero,
integer-pixel origins, unhinted outlines and scale one. Within an immutable font
byte slice, glyph ID and pixel size identify the mask and its bearings. Binding
a different font slice drops the entries. Position, clip, color and opacity
apply during compositing. Every update still paints a fresh white surface.

The experimental FIFO uses at most 64 entries and 32 KiB of mask data capacity.
The entry limit also bounds metadata for empty glyphs. Oversized masks are drawn
from scratch. A later small mask can still enter the cache. Release drops the
entry queue and its backing allocations. The byte budget covers retained mask
arrays only. Queue metadata, the frame surface and temporary Swash allocations
have separate lifetimes.

Pixel comparisons cover 24 mixed updates with size changes, movement, clipping,
color, eviction and replay after dropping the scene. A separate test exercises
the byte budget, oversized scratch reuse, font rebinding and a failing miss.
The cached PNG also matches the committed baseline experiment’s sample byte for
byte.

The table reports edit through finished RGBA output in microseconds. Both modes
edit the same first label, publish a snapshot and redraw the whole 256 by 244
surface. Ten warmup edits precede seven batches of ten measured edits. The
order alternates each batch. Values are medians of three release process medians.

| Workload | Uncached | Cached | Hits / misses across 70 edits |
| --- | ---: | ---: | ---: |
| 1 row, warm | 85.80 | 29.09 | 1,120 / 0 |
| 8 rows, warm | 296.11 | 60.54 | 10,430 / 0 |
| 1 row, release before each draw | 47.81 | 44.81 | 210 / 910 |
| 8 rows, 4-entry budget | 311.78 | 293.84 | 1,085 / 9,345 |

The warmed eight-row case retained 17 masks with 2,773 bytes of pixel-array
capacity. Its cached process medians ranged from 59.66 to 61.94 µs and its
uncached medians from 293.06 to 303.98 µs. Single-row timings varied more
(17.56–36.43 µs cached and 55.30–87.30 µs uncached). Cold frames still reuse
repeated letters within one draw. The four-entry case measures frequent eviction.
The repetitive corpus establishes a benefit for recurring glyphs with these
raster settings. Selecting the cache budget for broader text needs a larger corpus.

## Redrawing a damaged rectangle

The retained path holds one surface and the frame that produced it. It compares
ordered commands in the two snapshots. The frame scroll offset translates
document bounds into viewport pixels at the current scale. A change of device
scroll offset, node, color, origin,
allocation or paragraph handle includes both old and new visible allocations
in a single bounding rectangle. The renderer clips all text ink to these
allocations, which makes them conservative pixel bounds. Identical frame handles
allow a constant-time skip. A size, scissor or font change triggers a fresh
full-surface render.

The rectangle starts white in a temporary RGBA patch. All current commands
intersecting it draw in their original order, including unchanged translucent
layers. After successful preparation, the renderer copies patch rows to the
retained surface and installs the new frame. A preparation error leaves both
previous pixels and frame in place. The surface is borrowed from the presenter.
Its previous frame handle becomes available for release on successful update.

The eight-row comparison uses the same 64-entry, 32 KiB glyph-mask policy for
both paths. Both edit the scene, publish the frame and produce current pixels.
The full path allocates and redraws a complete surface every call. The retained
path includes comparison, patch allocation, clear, rasterization and row copies.
Each release process warms up for ten updates and alternates path order across
seven batches of ten edits. Values are medians of three process medians in µs.
The last column counts RGBA pixel locations prepared per update.

| Update | Full | Retained | Prepared pixels |
| --- | ---: | ---: | ---: |
| One row changes text | 64.94 | 20.82 | 4,776 |
| One row moves 12 pixels | 49.72 | 7.50 | 5,064 |
| First and last rows change color | 47.95 | 49.99 | 42,387 |
| Width alternates 192 / 256 | 39.30 | 39.39 | 54,656 (mean) |
| Same snapshot submitted again | 46.76 | 0.01 | 0 |

For text edits, process medians ranged from 64.61–65.41 µs full and
20.65–21.01 µs retained. The retained 256 by 244 surface owns 249,856 bytes of
pixel data. The single-row text patch uses another 19,104 bytes and the spread
edit uses 169,548 bytes. Scratch lives for one update. Resizing prepares the new
surface while the old surface is still available. These byte counts cover RGBA
payloads only. The idle comparison models repeated presentation requests.

Fifty-six mixed updates compare every RGBA byte with uncached full redraws,
including clips, resize, transparent text, movement, and font-size changes.
Replaying an older frame after scene destruction, failed resize and overlapping
commands in different orders also have focused checks. Local edits benefit in
this fixture. Widely separated edits expose the extra raster and copy cost of
the single bounding rectangle. Keep this as the baseline for localized updates.

## Reproduce

Run the example with stdout redirected to an image. Timings go to stderr.

```sh
cargo test -p solor-render-cpu --example text_pixels --locked --offline
cargo run --release -p solor-render-cpu --example text_pixels --locked --offline > text-pixels.png
```

## Mounted CPU window measurements

A mounted meter changes width on each pointer move. It draws a translucent fill
over a track, beside four static labels. The viewport is 320 by 180 logical
pixels at 1.25 scale (400 by 225 device pixels). The handler receives the same
sequence in both builds. Timing includes input delivery, flush, layout, full
command recording, rasterization and local submission acknowledgement.
Wayland uploads, compositor scheduling and display latency are outside this timing.

Six pairs of release processes alternated run order. Each discarded 20 warm-up
frames and measured 200 updates. Values below are the medians of the six
per-process statistics. The full replay executable used the same workload
with the window implementation at `6f378a8`.

| Window rasterization | Median frame (range across runs) | p95 frame | Mean reported damage |
| --- | ---: | ---: | ---: |
| Full replay | 782 µs (748–788 µs) | 1,236 µs | 90,000 pixels |
| Retained image with clipped replay | 42 µs (38–76 µs) | 96 µs | 2,059 pixels |

The full command list is still recorded and compared on each frame. Damage is
one rectangle, so changes far apart can include the intervening area.

Run the timing workload with
`cargo test --release -p solor-render-cpu --test meter_updates --offline -- --ignored --nocapture`.

In a separate nested Wayland run of the mounted settings example, the initial
submission reported 260 by 110 pixels. Expanding and collapsing the detail row
each reported 200 by 32 at (12, 50). Window captures confirmed the row appeared
and disappeared, and all three buffer submissions passed the readback audit. The
second submission initialized a fresh shared file, writing 114,400 bytes.
The third reused a file and wrote 33,040 bytes.
