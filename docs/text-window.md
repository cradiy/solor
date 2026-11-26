# Wayland text window

Run the two-row CPU text fixture in a Linux Wayland session with
`wl_compositor` version 4 or later and XDG shell.

```sh
cargo run --offline --release --example text_window
```

The default workload uses eight frame callbacks to drive alternating edits.
The [disclosure mode](disclosure.md) uses the same window for pointer-driven updates. Once the edits finish, the
window waits for events. Resize it to reflow the text or use the compositor's
close action to exit. Set `SOLOR_VERIFY_SHM=1` to read each submitted file back
before attaching it and compare every pixel with an uncached full redraw.
The audit also compares consecutive full redraws outside the reported damage.

Layout uses logical units. With viewporter and fractional scale, the surface uses the compositor's preferred scale
in 120ths to size the physical buffer and rasterize each glyph outline. At
150%, a 16-unit font uses a 24-pixel mask and a 320 by 100 window uses a
480 by 150 buffer. The viewport maps the buffer to the logical window size.
With integer scaling, the surface uses the highest scale among its current outputs and starts at 1×.
Mask keys include the physical font size. A scale change invalidates retained
pixels even if rounding leaves the physical buffer dimensions unchanged.

The client keeps one retained RGBA image and up to two shared files. Each file
tracks a bounded set of changed rectangles since its last upload,
including changes while the compositor held it. New files start with the entire
image pending. When the compositor advertises XBGR8888, the uploader writes the
retained RGBA bytes directly. The format ignores the alpha byte. With XRGB8888
the uploader swaps red and blue in a 16 KiB staging buffer. Each file keeps
its format for its lifetime and clears pending damage after all writes succeed.

The uploader walks changed row spans in file-offset order and writes overlapping
bytes once. Gaps shorter than 2,048 bytes can be included in the same write
because the retained image contains the current values of the intervening pixels.

The window sends the current frame's rectangles through `wl_surface.damage_buffer`
in physical buffer coordinates. A reused slot can need more pixels repaired than
the compositor needs for this frame. A new logical size or scale damages the full target.

When the retained rasterizer returns an empty damage region, the window keeps
the current buffer attached and preserves the pending damage in each slot. An
event-driven disclosure then waits for input. If another application update
needs a frame callback, the window requests it and commits the callback alone.
The next visible change repairs the selected file before attaching it. The
`commit=` log counts buffer submissions. With auditing enabled, even skipped
updates are checked against an uncached full raster.

## Surface damage verification

Run the fixture with pixel auditing enabled to check partial uploads, buffer
reuse and resizes against full redraws. The fixture logs the bytes converted
and written to each shared file during ordinary runs.

```sh
SOLOR_VERIFY_SHM=1 cargo run --offline --release --example text_window
```

Upload timing covers pixel conversion and shared-file writes. A compositor
presentation timestamp is needed to measure when the submitted image appears.

## Bounding rectangle uploads

The gap heuristic was checked separately by writing to prefilled memfds in
release builds. For a 288-pixel buffer with a 250 by 160 edit, writing every
partial row separately took 43.7 µs versus 21.5 µs for the scanline band in
the initial trial. Bridging the short gaps brought the two paths within
1 µs in three subsequent processes. For a 12 by 1,300 edit in a
1,244-pixel buffer, separate row spans took 233–252 µs versus 962–1,103 µs
for the scanline band. Each process alternated the two paths over one warmup
and seven measured batches of 96 uploads per path. These timings cover pixel
conversion and file writes to warmed targets.
