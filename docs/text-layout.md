# Fixed-font Text Layout

## Scope and acceptance

The private `text_layout` example integrates real font shaping and line layout
with the existing headless measurement/allocation contract. It uses
[cosmic-text 0.19.0](https://docs.rs/cosmic-text/0.19.0/cosmic_text/) as a development
dependency with only its `std` feature enabled. Normal library dependencies remain
unchanged. The experiment does not choose a framework-wide text engine or expose
a widget API.

One bundled [Noto Sans CJK SC subset](../crates/solor-core/examples/text_layout/fonts/README.md)
provides deterministic Latin/CJK fixtures. The database starts empty, loads only
that face, and uses a fixed locale. Missing glyphs fail explicitly. This evaluates
the included corpus, not arbitrary Unicode, font fallback, rich text, editing,
IME, accessibility, rasterization, or cross-platform text fidelity.

Acceptance requires real variable advances, CJK wrapping, baselines, explicit
line breaks, final allocation distinct from measurement, bounded replacement,
owned result lifetimes, and comparison against fresh text buffers plus forced
layout. CPU and retained storage must include unfavorable cache-miss workloads.

## Ownership and invalidation

Each fixture shares one `FontSystem` among its text nodes. A node owns its UTF-8
content, font size, line height, and a three-entry FIFO paragraph cache. Fixed
font identity, locale, shaping mode, wrapping policy, and alignment cannot mutate
in this experiment. Content or style changes clear the whole node cache. A cache
key is the optional width: unconstrained `Natural` differs from every finite
width; `AtMost(w)` and `Exact(w)` share the same paragraph but produce different
box-width answers through `Query::resolve`.

A miss creates a temporary cosmic-text `Buffer`, shapes and lays out the complete
paragraph, then copies line and glyph data into an immutable `Rc<Paragraph>`.
It retains source text, style, line indices, baselines, line heights, glyph IDs,
UTF-8 cluster ranges, positions, advances, and offsets. It borrows no engine or
buffer state. An external result handle survives cache eviction, content edits,
node destruction, and engine destruction; its memory lives until that handle is
dropped. The engine's font data is fixed, so fixture glyph IDs have one meaning.

The framework continues to cache only metrics. Final text layout resolves the
actual allocated width rather than reading whichever paragraph happened to be
measured last. Natural, bounded, and exact requests can therefore interleave,
including framework metric-cache hits that never execute the text callback.

Internal line/glyph positions remain floating point. The existing prototype's
integer sizes and first baseline round upward at the adapter boundary. Content
extent represents logical line boxes, not ink bounds or damage bounds. Zero-width
allocations can have overflowing glyphs; short height allocations preserve the
full content extent. This adapter does not settle the framework's coordinate type.

Input limits are 4,096 UTF-8 bytes, font sizes 1 through 128, line heights from the
font size through 256, and finite widths through 1,048,576. A node retains at most
three paragraphs. `release_cache` frees node cache storage, while caller-held
paragraphs and the shared engine have their own lifetimes. The experiment neither
bounds caller-held handles nor claims a general resource budget for arbitrary
font databases.

Paragraph caching is not publication of a paint result. Failed layout retains the
previous published geometry; content inputs and successful speculative caches may
have changed. A future rendering transaction must retain and commit its own
paragraph handles with the corresponding geometry. The separate
[drawing snapshot experiment](text-frame.md) implements that publication boundary.
The test that changes `中文`
to `文字` confirms equal layout geometry with different glyph IDs while the root
layout is skipped. It does not implement paint invalidation.

## Correctness evidence

Seven text-specific tests cover:

- CJK advances of 32 units for two full-width glyphs at size 16, wrapping into two
  lines at width 16, variable Latin advances, baselines, explicit newlines, empty
  input, zero-width overflow, and byte ranges for Chinese characters.
- Shared compatible paragraph keys with distinct query answers, intervening
  measurement queries, and final width/height different from measurement.
- Content and style invalidation, FIFO eviction, explicit release, and retained
  result handles after owners are destroyed.
- Rejection of oversized/invalid input without losing the previous cached result.
- Missing-glyph failure that preserves geometry and retries successfully.
- Equal geometry with new glyph content through the parent-layout skip path.
- 96 mixed content, style, width, height, and width-churn updates, comparing every
  geometry, line, cluster, and glyph position to fresh buffers and forced layout.

The reference uses the same text engine; it checks integration and cache semantics,
not the independent correctness of that engine. Known-value assertions provide
separate checks for advances, wrapping, and cluster byte ranges. The workspace
includes these text tests; the text target passes eight tests, including the
existing staged-allocation test.

## Shared source text

The private text node and each width-specific paragraph hold a `SmolStr`.
Passing a clone of the same long source to both nodes preserves its backing.
Converting two independent borrowed strings constructs two independent values.
Each paragraph owns its line and glyph vectors. Replacing a node's text or
dropping the node leaves previously returned paragraphs readable.

The release test holds two nodes and all four paragraphs live at once. Its
1,152-byte dynamic source wraps at widths 96 and 240. The String baseline
had six distinct source addresses among those objects. With a shared source,
both nodes and all four paragraphs point to the original source bytes. Four
shaping calls still run, and the narrower paragraphs contain more lines.

A Linux allocator probe counted calls while constructing and dropping 1,024
text nodes per label. The engine and dynamic input were prepared before the
markers. All three release processes per representation gave these counts.

| Input | String calls | String requested bytes | SmolStr calls | SmolStr requested bytes |
| --- | ---: | ---: | ---: | ---: |
| `Audio settings`, 14 UTF-8 bytes | 1,024 | 14,336 | 0 | 0 |
| `设备`, 6 UTF-8 bytes, borrowed from a String | 1,024 | 6,144 | 0 | 0 |

The measured loop contains no layout, shaping or window operations. Run
`cargo test --release --offline --example text_layout shared_source_across_nodes_and_widths`
for the sharing check. An external allocator profiler can count between the
`begin short-label/` and `end short-label/` markers emitted by
`cargo test --release --offline --example text_layout short_label_allocations -- --ignored --nocapture --test-threads=1`.

## CPU measurement

The following CPU and retained-heap measurements use the String baseline.

The release example compares cached and uncached paragraph results in the same
binary. Both paths retain normal framework metric/layout caching and share fonts
within each fixture. The uncached path creates a fresh text buffer on every text
callback; it is not a comparison against another GUI framework or an optimized
persistent-buffer implementation.

Each fixture contains a column of 64 mixed Latin/CJK labels. A content update
changes one label's numeric suffix; a style update toggles one label's font size
and line height. Repeated widths alternate 160 and 240. Width churn cycles through
17 widths from 160 to 272. A height-only update changes root allocation without
changing text width. Changed labels rotate across the fixture.

Measurements include edits, invalidation, paragraph resolution, geometry layout,
and publication. They exclude construction, correctness snapshots, and counter
aggregation. After 16 warmup updates, seven samples of 31 updates alternate path
order. Three sequential process runs establish variability. Allocator-instrumented
runs are separate from timing. The fresh-engine measurement constructs the fixed
font database and resolves one paragraph; it excludes process startup, font file
I/O, system font discovery, and destruction.

Medians of the three process medians, with ranges in brackets, are microseconds
per complete update:

| 64-label workload | Uncached paragraphs | Cached paragraphs |
| --- | --- | --- |
| One content edit | 34.34 [34.13, 36.75] | 34.76 [34.04, 36.00] |
| One style edit | 33.28 [33.23, 34.31] | 34.15 [34.03, 35.25] |
| Two alternating widths | 896.98 [865.38, 910.33] | 5.00 [4.27, 5.24] |
| Height only | 3.05 [3.03, 3.17] | 3.08 [3.08, 3.19] |
| Seventeen changing widths | 1221.55 [1201.24, 1227.18] | 1196.22 [1163.72, 1197.62] |

Fresh engine construction plus one paragraph takes 27.88 us [26.30, 29.52]. This
uses a 25,780-byte font subset and is not a full-font startup estimate.

Content/style edits execute two complete paragraph computations in both paths:
the column first measures at a bounded width, then allocates the resulting
integer content width, which differs in this fixture. Caching does not improve
those updates. After warmup, alternating widths replace 64 paragraph computations
per update with 64 cache hits. Height-only updates require no text callbacks.
Width churn still averages 82.88 computations with caching versus 86.71 without;
its small timing difference is not evidence of a general resizing speedup.

The benchmark caches whole width-specific paragraphs, not a reusable shaping
stage independent of wrapping. Continuous resizing and text edits remain concrete
costs. The large alternating-width gain must not be extrapolated to arbitrary
widths, full font databases, or complete applications.

## Retained heap and reclamation

The independent allocator probe observes one 123-byte Rust allocation as 136
usable bytes in its positive control. Engine construction retains 27,856 usable
bytes and engine destruction releases the same amount. After 100 mixed updates
and one update that settles the root width, fixture storage is:

| Boundary | Uncached | Cached |
| --- | ---: | ---: |
| Warmed live usable bytes | 94,256 | 478,584 |
| Local edit: allocation calls | 142 | 143 |
| Local edit: requested bytes | 81,790 | 81,854 |
| Local edit: live usable delta | +16 | -1,752 |
| Next 1,000 updates: live usable delta | +3,768 | +18,536 |
| Another 1,000 updates: live usable delta | 0 | -736 |
| Cache and scratch release | 11,792 | 408,432 |
| Final destruction | 86,248 | 86,200 |

The warmed cache cost is about 375 KiB for 64 labels. The later workload changes
which paragraphs and vector capacities are retained; its final live storage is
98,040 bytes uncached and 494,632 cached. Release and destruction account for
that remaining storage. The second long block does not show accumulating history;
the explicit three-entry cap and input limits define the node cache bound.

These figures include scene, dependency records, shared engine, and text results.
They are usable heap at markers, not peak memory, static binary/font bytes, or
RSS/PSS. The local edit still requests about 82 KiB of temporary storage. A
fixed-font fixture with short labels does not establish low application memory.

## Decision

The integration and lifetime experiment meets its acceptance boundary. The
existing query/allocation contract can carry real text metrics while nodes own
their complete paragraph results. Different glyph content can coexist with equal
layout output, so paint invalidation must remain independent of layout equality.

Keep the experiment and its two cache modes. Three retained paragraphs are an
evaluation choice, not a public default: repeated-width reuse is valuable, but
storage increases and miss-heavy updates still reconstruct text buffers. Further
text optimization is not required to close this milestone. The
[drawing snapshot experiment](text-frame.md) evaluates committed drawing resources;
the [CPU text pixel experiment](text-pixels.md) checks clipping and raster output.

## Reproduction

```sh
cargo test --workspace --tests --examples --offline
cargo run --release -p solor-core --example text_layout --offline
```

The executable verifies the mixed-update oracle before printing CPU samples and
allocation markers. `cache_payload_bytes` counts owned paragraph/vector capacities;
it excludes shared source backing, allocator rounding, reference-count headers, and engine internals.
Allocator markers separately delimit engine creation/destruction, fixture warmup,
a local edit, two blocks of 1,000 mixed updates, cache/scratch release, and final
destruction. They report allocation traffic and retained usable heap, not RSS/PSS
or a peak-memory guarantee.
