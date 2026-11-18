# CPU text pixels

The private `text_pixels` example consumes the published [text frame](text-frame.md)
directly. Each call starts with opaque white RGBA8 pixels, loads face zero from
the snapshot's fixed font bytes, and paints the supplied glyph IDs in command
order. Swash supplies unhinted grayscale outline masks. PNG encoding is used for
the sample output. Both dependencies are development-only.

The device scale is one. Glyph origins include the command origin, the line
baseline and the size-scaled shaping offsets. The renderer rounds the local
glyph origin to the nearest pixel and applies the signed bitmap bearings. A
half-open scissor intersects the surface, the allocated text box and each glyph
mask. Coverage modulates text alpha, and source-over blending operates on the
encoded color channels of the opaque destination. These are explicit policies
of this fixture. Fractional device scale, transparent targets and native window
presentation require separate evaluation.

Each draw creates one surface (1 through 4,194,304 pixels), a scale context and
one reusable scratch mask. All visible commands rasterize their glyphs anew.
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

## Reproduce

Run the example with stdout redirected to an image. Timings go to stderr.

```sh
cargo test -p solor-core --example text_pixels --locked --offline
cargo run --release -p solor-core --example text_pixels --locked --offline > text-pixels.png
```
