# Wayland text window

Run the two-row CPU text fixture in a Linux Wayland session with
`wl_compositor` version 4 or later, XDG shell, viewporter and fractional scale.

```sh
cargo run --offline --release --example text_window
```

Eight frame callbacks drive alternating edits. Once the edits finish, the
window waits for events. Resize it to reflow the text or use the compositor's
close action to exit. Set `SOLOR_VERIFY_SHM=1` to read each submitted file back
before attaching it and compare every pixel with an uncached full redraw.

Layout uses logical units. The surface uses the compositor's preferred scale
in 120ths to size the physical buffer and rasterize each glyph outline. At
150%, a 16-unit font uses a 24-pixel mask and a 320 by 100 window uses a
480 by 150 buffer. The viewport maps the buffer to the logical window size.
Mask keys include the physical font size. A scale change invalidates retained
pixels even if rounding leaves the physical buffer dimensions unchanged.

The client keeps one retained RGBA image and up to two shared files. It
repairs the RGBA image using damage bounds in physical pixels, converts the
whole image to native-endian XRGB8888, and writes it to a free file. Each
submitted slot stays busy until the compositor releases it. A free slot with
a different size receives a fresh buffer. This provides a reference for later
experiments with partial uploads. The core library has no Wayland dependency.

## Measurements

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
