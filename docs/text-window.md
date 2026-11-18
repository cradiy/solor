# Wayland text window

Run the two-row CPU text fixture in a Linux Wayland session with
`wl_compositor` version 4 or later, XDG shell, viewporter and fractional scale.

```sh
cargo run --offline --release --example text_window
```

The default workload uses eight frame callbacks to drive alternating edits.
The [disclosure mode](disclosure.md) uses the same window for pointer-driven updates. Once the edits finish, the
window waits for events. Resize it to reflow the text or use the compositor's
close action to exit. Set `SOLOR_VERIFY_SHM=1` to read each submitted file back
before attaching it and compare every pixel with an uncached full redraw.

Layout uses logical units. The surface uses the compositor's preferred scale
in 120ths to size the physical buffer and rasterize each glyph outline. At
150%, a 16-unit font uses a 24-pixel mask and a 320 by 100 window uses a
480 by 150 buffer. The viewport maps the buffer to the logical window size.
Mask keys include the physical font size. A scale change invalidates retained
pixels even if rounding leaves the physical buffer dimensions unchanged.

The client keeps one retained RGBA image and up to two shared files. Each file
tracks a single band of physical scanlines that changed since its last upload,
including changes while the compositor held the file. Once the compositor
releases a slot, the client can convert and write the entire band in contiguous
chunks of native-endian XRGB8888. New files start with all rows pending. The
band is cleared after all writes succeed. Its bounds can include unchanged
rows between distant edits and unchanged columns on every affected row.
The surface still reports full-buffer damage to the compositor. The core
library has no Wayland dependency.

## Full upload baseline

On one Niri output at 150%, the release example mapped at 829 by 987 logical
units (1244 by 1481 physical pixels). One audited run completed 12 buffer
commits including eight text edits and three resizes. Seven commits reused
released buffers. All 12 readbacks matched the uncached renderer. A separate
test compares the scaled CJK glyph with independently shaped 24-pixel text
while the cache contains the 16-pixel mask.

Three further processes ran the same sequence without the audit. The first
buffer commit was queued 9.94–22.98 ms after entry to the example, with a
median of 12.58 ms. This timing ends before the compositor displays the
frame. Converting and writing a full reused buffer at the initial size took
1.73–2.04 ms across process medians (seven uploads per process).

After resizing through 192 by 100 to 320 by 100, all three clients had zero
new commits and zero additional scheduler-accounted CPU ticks during a
1.25-second observation. Client PSS from Linux smaps_rollup was 3681–3757
KiB. The two live memfds had logical sizes of 288000 and 172800 bytes. Writes
use file descriptors without mapping the files into the client. Client PSS
does not cover compositor resources or provide a total desktop memory cost.
All four clients exited through the XDG close event. Visual verification is
limited to the submitted buffer sample on this output.

## Upload comparison

The two-row release fixture ran against the full-upload version at `525fbdb`
on the same Niri output at 150%. Each fresh process completed eight alternating
edits at 1244 by 1481 physical pixels. The first two commits populated the two
slots. The next seven reused them. Three pairs of processes alternated which
version ran first. Timing covers conversion and shared-file writes for those
seven commits per process. Pixel auditing was disabled for timing.

| Upload strategy | Bytes per reused buffer | Median upload time across process medians (range) |
| --- | ---: | ---: |
| Full image | 7,369,456 | 1,634 µs (1,303–1,654) |
| Pending rows | 383,152 | 132 µs (119–133) |

Alternating rows left both changes pending in the older slot, spanning 77
physical scanlines. The band reduced written bytes by 94.8% in this fixture.
First use and resize still require full writes. A separate audited run checked
all 12 commits against an uncached full redraw, including the seven reused
buffers and resizes through 192 by 100 and 320 by 100 logical units.

Client PSS after the eight edits, before any resize, was 10,053–10,205 KiB
for pending rows and 10,180–10,257 KiB for full upload. Both versions also
held two 7,369,456-byte shared files outside the client address space. The
timings exclude layout, rasterization, compositor work and display latency.
