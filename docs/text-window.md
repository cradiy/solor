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
The audit also compares consecutive full redraws outside the reported damage.

Layout uses logical units. The surface uses the compositor's preferred scale
in 120ths to size the physical buffer and rasterize each glyph outline. At
150%, a 16-unit font uses a 24-pixel mask and a 320 by 100 window uses a
480 by 150 buffer. The viewport maps the buffer to the logical window size.
Mask keys include the physical font size. A scale change invalidates retained
pixels even if rounding leaves the physical buffer dimensions unchanged.

The client keeps one retained RGBA image and up to two shared files. Each file
tracks the bounding rectangle of all pixel changes since its last upload,
including changes while the compositor held it. New files start with the entire
image pending. The uploader converts pixels to native-endian XRGB8888 using
a 16 KiB staging buffer and clears the rectangle after every write succeeds.

For a gap of at least 2,048 bytes between the end of one affected row span and
the start of the next, the uploader writes the spans separately. For smaller
gaps it writes the contiguous range from the first affected pixel to the last.
This also handles full-width updates. Both paths can include unmodified pixels
within the combined bounds of distant edits.

The window sends the retained rasterizer's physical rectangle through
`wl_surface.damage_buffer`. The rectangle uses the current buffer’s coordinates and
bounds the pixel changes since the preceding surface commit. A slot can need
more scanlines repaired than the rectangle reported to the compositor. A new
logical size or scale damages the full target.

When the retained rasterizer returns an empty damage region, the window keeps
the current buffer attached and preserves the pending rectangle in each slot. An
event-driven disclosure then waits for input. If another application update
needs a frame callback, the window requests it and commits the callback alone.
The next visible change repairs the selected file before attaching it. The
`commit=` log counts buffer submissions. With auditing enabled, even skipped
updates are checked against an uncached full raster.

## Surface damage verification

On Niri at 150%, the scanline-upload fixture started with a 1244 by 1481
physical buffer. The eight alternating edits sent rectangles of 258 by 36
and 353 by 36 pixels. The first edit populated a second shared file in full.
Each later edit repaired a 77-scanline band in a reused file. All 12 buffer
readbacks, including three resizes, matched uncached rasterization.

In the disclosure fixture, a color edit reported a 278 by 36 rectangle and
adding a line reported 278 by 113. The temporary native driver restored the
bound label before presentation twice, first with another edit scheduled and
then with the window idle. The protocol trace showed only a frame request
and commit in the first case, and no surface request in the second. The next
callback recolored the label. A real pointer click woke the idle window and
collapsed the detail. All submitted buffers, including resizes, passed the
full-raster check.

Two paired release runs of the scanline uploader against `38dfb11` used the same driver, with run
order reversed for the second pair. At 1244 by 1481 physical pixels, both
empty updates wrote zero bytes. The baseline wrote 562,288 and 358,272 bytes
respectively. From initial mapping through the click, including deferred
repairs, shared-file writes totaled 16,624,816 bytes versus 17,341,360 bytes
in each pair. These figures measure client writes to the shared files.

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

## Bounding rectangle uploads

At 1244 by 1481 physical pixels, alternating edits of two labels leave a
353 by 77 rectangle pending in the older file. The scanline uploader in
`988598e` converts 383,152 bytes on each reuse. Restricting the upload to
the horizontal bounds reduces that to 108,724 bytes. A full buffer still
takes 7,369,456 bytes. On Niri at 150%, all 12 buffers in each audited
two-row run matched uncached rasterization through edits and resizes.
The 1,024-row disclosure also matched through scrolling, expansion and collapse.

Four paired release runs used the same Niri output, with both executables on
the same filesystem, auditing disabled and pair order alternating. Each fresh
process contributed the median upload time for its seven reused buffers.

| Upload strategy | Bytes per reuse | Median of process medians (range) |
| --- | ---: | ---: |
| Full-width scanline band | 383,152 | 136.5 µs (110–157) |
| Bounding rectangle | 108,724 | 81.5 µs (80–85) |

Upload time starts before pixel conversion and ends after the shared-file
writes return. In all eight processes client anonymous PSS was 7,708 KiB
after the edits and 1,188 KiB after resizing to 320 by 100 logical units.
Total PSS after resizing ranged from 3,889–3,929 KiB for scanline bands
and 3,825–3,961 KiB for bounding rectangles.

The gap heuristic was checked separately by writing to prefilled memfds in
release builds. For a 288-pixel buffer with a 250 by 160 edit, writing every
partial row separately took 43.7 µs versus 21.5 µs for the scanline band in
the initial trial. Bridging the short gaps brought the two paths within
1 µs in three subsequent processes. For a 12 by 1,300 edit in a
1,244-pixel buffer, separate row spans took 233–252 µs versus 962–1,103 µs
for the scanline band. Each process alternated the two paths over one warmup
and seven measured batches of 96 uploads per path. These timings cover pixel
conversion and file writes to warmed targets.
