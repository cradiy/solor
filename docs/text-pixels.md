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

The CPU window compares complete command lists on each frame. Damage is
one rectangle, so changes far apart can include the intervening area.

Run the timing workload with
`cargo test --release -p solor-render-cpu --test meter_updates --offline -- time_mounted_meter_frames --exact --ignored --nocapture`.

In a separate nested Wayland run of the mounted settings example, the initial
submission reported 260 by 110 pixels. Expanding and collapsing the detail row
each reported 200 by 32 at (12, 50). Window captures confirmed the row appeared
and disappeared, and all three buffer submissions passed the readback audit. The
second submission initialized a fresh shared file, writing 114,400 bytes.
The third reused a file and wrote 33,040 bytes.

### Static label count

The same meter and four visible labels stay in place. Additional mounted text
siblings sit below the viewport. A separate check compares all RGBA bytes and
damage rectangles across the four sizes on the first frame and all 220 updates.
Each size reports an average of 2,059 damaged device pixels per measured update.

Six pairs of release processes alternated the two builds. The rebuild baseline
uses the same test with Solor at `0aa6f5b`. Each process ran the label counts in
ascending and descending order. Each pass discarded 20 updates and measured
200. Values are medians of the twelve pass statistics per build, in µs.
Window time includes local submission acknowledgement. A separate mounted view
times command recording; layout reports zero changed nodes for each update.

| Static labels | Window, rebuild | Window, reuse | Window p95, rebuild / reuse | Record, rebuild / reuse |
| ---: | ---: | ---: | ---: | ---: |
| 4 | 40.9 | 39.9 | 63.4 / 61.7 | 0.42 / 0.29 |
| 64 | 45.1 | 43.1 | 67.4 / 65.4 | 3.55 / 2.45 |
| 256 | 58.5 | 53.3 | 81.7 / 76.1 | 13.20 / 8.89 |
| 1,024 | 123.1 | 101.0 | 147.4 / 124.9 | 63.81 / 42.51 |

At 1,024 labels, per-pass window medians ranged from 116.5 to 127.5 µs when
rebuilding and 94.6 to 103.9 µs with reuse. In a separate warm loop, preparing
the pointer snapshot took 17.35 µs and 0.05 µs respectively. Both builds in this comparison record each label individually. The four-label window medians overlapped across runs. This workload
covers steady-state local updates, excluding mount, upload and compositor time.

Run the scaling workload with
`cargo test --release -p solor-render-cpu --test meter_updates --offline -- time_static_label_scaling --exact --ignored --nocapture`.

### Reusing fixed label commands

A view retains the drawing commands of fixed text labels without backgrounds
after a second successful recording with the same geometry. Subsequent frames
copy those commands in paint order. Each cache entry shares the shaped
paragraph and stores the label's position, content size, foreground and order.
On x86-64 the entry occupies 40 bytes, or 40 KiB of entries for 1,024 labels,
excluding vector spare capacity and allocator overhead. Insertion, removal or
changed geometry discards the cache. Bound text, labels with backgrounds and
custom elements follow their normal recording paths.

The same six-pair release comparison uses shared pointer geometry in both
builds. Each measurement below is the median of twelve pass statistics in µs.

| Static labels | Window, individual / cached | Window p95, individual / cached | Record, individual / cached |
| ---: | ---: | ---: | ---: |
| 4 | 39.6 / 38.9 | 62.8 / 61.2 | 0.30 / 0.21 |
| 64 | 44.2 / 41.2 | 66.3 / 63.4 | 2.42 / 0.53 |
| 256 | 53.3 / 46.5 | 76.6 / 69.2 | 8.73 / 1.75 |
| 1,024 | 101.3 / 62.8 | 127.6 / 86.9 | 41.47 / 6.09 |

At 1,024 labels, per-pass window medians ranged from 95.7 to 105.4 µs for
individual recording and 59.4 to 67.5 µs with caching. Damage averaged 2,059
pixels per frame in both builds. The four-label window medians overlapped.

A separate workload alternated the viewport width between 320 and 321 logical
pixels every frame with 1,024 labels. Each of six paired processes per build
discarded 20 updates and timed 200. The median from layout through recording
and local input-frame acceptance was 125.0 µs for individual recording and
128.5 µs with caching (per-run medians 122.3–126.9 and 121.6–135.8 µs).
This workload never reuses a layout, so it exercises the first recording path
and allocates no label cache entries. It excludes rasterization and upload.

Run the resize workload with
`cargo test --release -p solor-render-cpu --test meter_updates --offline -- time_resizing_meter --exact --ignored --nocapture`.

### Retained and continuous elements

One mounted window contains 1,024 labels, a retained signal-driven bar and a
continuous bar that samples the latest value. At 120 updates
per second the budget is 8.33 ms. In the paced release test, a 320 by 180
device-pixel window receives 260 frame opportunities with four independent
changes to the retained bar. Including initial paint, the two bars record
5 and 261 times respectively. Across three processes, after 20 warm-up
updates the CPU p95 was 85–111 µs and p99 was 101–123 µs. All 720 measured
updates finished before the next scheduled tick. This test includes local
rasterization and acknowledgement.

On a 165 Hz Wayland output the compositor allocated 829 by 987 logical pixels
at scale 1.5 (1,244 by 1,481 device pixels). Three traced release processes
used the same tree and updates. Each received presentation timestamps for
all 240 buffer commits numbered 21 through 260. For those 239 intervals per
process, effective presentation rates were 86.5, 89.6 and 85.9 fps. The median
interval was 12.12 ms and p95 was 18.18 ms in all three runs. Callback entry
through local submission acknowledgement, including shared-memory upload, took
139–199 µs at p95. This configuration missed the 120 fps presentation target.
These timestamps locate the larger interval outside the measured local update;
they do not identify which compositor or display stage consumed it.

Run the paced test with
`cargo test --release -p solor-render-cpu --test refresh_modes --offline -- mixed_updates_at_120_hz --exact --ignored --nocapture`.
On a visible Wayland output, the following command runs the native probe and
prints buffer submissions and available presentation feedback. Feedback timestamps
are in the compositor's presentation clock.

```sh
SOLOR_TRACE_SUBMISSIONS=1 cargo test --release -p solor-render-cpu --test refresh_modes --offline -- wayland_mixed_frame_cadence --exact --ignored --nocapture
```

### Same-size Wayland control

Four pairs of release processes alternated the mixed Solor window and
a standalone libwayland client. Each got an 829 by 987
logical allocation, a 1,244 by 1,481 XRGB8888 buffer, 1.5 scale and the same
application ID. Both alternated two shared-memory buffers, waited for a frame
callback before submitting the next update, changed two bars on the same
schedule and requested presentation feedback. The control wrote the bars
directly into mapped memory over a white background. Solor also drew its
static labels. All 240 measured commits per process (21 through 260) received
presentation timestamps.

| Client | Effective fps across four runs | Median presentation interval | p95 interval | Callback to local commit, p95 |
| --- | ---: | ---: | ---: | ---: |
| Solor mixed window | 78.9–88.2 | 12.12 ms | 18.18 ms | 141–193 µs |
| Direct shared-memory client | 85.0–92.6 | 12.12 ms | 18.18 ms | 30–54 µs |

In three additional control runs, a 120 Hz monotonic timer drove submissions
whenever a buffer was available. Of 240 measured commits per run, 34–58
received discarded feedback. Effective presentation was 74.5–85.1 fps.
Bypassing the callback wait alone did not reach the target on this desktop.
The common 12.12 ms median spans two refresh periods at 165 Hz. These runs
cannot isolate the costs of shared-memory import, compositor scheduling and
output presentation from each other.

### Locating the missed refresh

The native trace associates each buffer commit with its frame callback and
presentation feedback. For buffer commit `i`, subtract `frame_done=i-1`'s
`dispatched_ns` from commit `i`'s `queued_ns` to measure client work.
The enqueue stamp is taken immediately after `wl_surface.commit`.
Both client stamps use Linux `CLOCK_MONOTONIC`. Compare them with the reported
presentation time only when the compositor advertises that same clock
(`presentation_clock_id=1`). Successive `sequence` values count output refreshes.

On a fixed 165 Hz output, three release processes per configuration rendered
the same 829 by 987 logical window with 1,024 labels and two bars. Run order
rotated. Each process yielded feedback for commits 21–260, giving 239 consecutive
intervals. A separate diagnostic build queued a `wl_display.sync` immediately
after each buffer commit and timestamped its reply without waiting before the
next frame. Receipt bounds when the server had processed the protocol request.
The compositor can import the buffer and schedule rendering later.

| Mixed Solor window | Effective fps | Callback to enqueue, p95 | Enqueue to sync reply, p95 | Sync reply to presentation, median |
| --- | ---: | ---: | ---: | ---: |
| Preferred scale, 1,244 × 1,481 buffer | 85.0–89.6 | 158–169 µs | — | — |
| Same buffer with sync probe | 77.5–86.7 | 169–173 µs | 169–183 µs | 15.47–16.51 ms |
| 622 × 740 buffer with sync probe | 81.0–87.4 | 115–152 µs | 165–183 µs | 16.25–16.46 ms |

All nine runs had a median presentation interval of 12.12 ms and no observed
wait for a free buffer. In the full-size sync runs, 543 of 717 intervals spanned
multiple refreshes. In all 543 cases the sync reply had already arrived before
the first predicted refresh after the previous presentation. The median lead was
10.17–10.26 ms across runs. The protocol measurements leave the compositor's
buffer import, render scheduling, GPU execution and display queue unresolved.

A separate CPU-drawn two-bar client alternated shared memory and linear
DMA-BUF submissions across three pairs of runs. Both paths used two released
buffers in rotation, identical XRGB8888 dimensions, pixel writes, damage and
frame callbacks. The DMA-BUF path exported DRM dumb buffers from the
compositor's advertised main device and bracketed CPU writes with
`DMA_BUF_IOCTL_SYNC`. Shared memory reached 85.9–90.7 fps and linear DMA-BUF
reached 77.9–89.2 fps. Both had a 12.12 ms median presentation interval.
This comparison covers CPU-written linear buffers on this desktop.
