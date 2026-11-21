# Component authoring

`solor` provides component composition, retained element layout, local
paint recording, pointer routing and direct click dispatch. A host creates a `Ui`, mounts a
`Render` component, calls `flush` at its update boundary, then calls `layout`
with the viewport size. It can walk `root` and `children` in sibling order,
read parent-local `geometry` and append individual elements' commands with
`paint`. Dragging and native frame submission remain separate experiments.

## Render and Element

An application component implements `Render`. The host calls `render` once
per attachment. It consumes the declaration and returns ordinary child
composition. The same component can be mounted directly, used in `child`, or
created by `child_when`. The returned tree may allocate local signals through
the component's context.

```rust
impl Render for Meter {
    fn render(self, _cx: &mut Context) -> impl IntoElement {
        element(MeterTrack { percent: self.percent })
            .child(text("Volume"))
            .on_click(move |cx| cx.update(self.active, |v| *v = !*v))
    }
}
```

`IntoElement` converts a declaration into an opaque, single-use element.
Heterogeneous child lists use type erasure internally. A component returning
`div()` or `text(...)` uses the same conversion as a custom primitive.
`component(|cx| ...)` is the closure form of `Render`.

A primitive implements `Element` on its retained body. Measurement and
child placement use the existing `LayoutNode` contract. Its optional `paint`
method records local drawing commands at the last successfully assigned size.
For example, `MeterTrack` reserves a four-unit track above a child label and
records its fill as a fraction of the assigned width. Both remain valid
through resize without running the component factory again. The UI
rejects recording while layout is pending.

Input registration belongs to the composed node. A click runs with a state
context. Removing the node retires its handler and local signals, then releases
its body. Reopening creates a fresh instance. The UI records text and
fills headlessly. The [stage contracts](node-execution.md#stage-contracts)
describe composition, clipping and accepted-frame routing. Raster submission,
focus and accessibility for custom elements still require integration.

## Composition

Start with a general container and a text leaf. The container arranges children
and can receive input or record custom drawing in its allocated bounds. The
text leaf owns shaping and measurement. A row or column is a layout choice
on the container. These capabilities can be used directly in application code.

`children` consumes an iterator in order alongside individual children.
`when` and `when_some` apply a closure while building the declaration and
can use borrowed inputs. Use `child_when` for a branch that follows a signal
after the component has mounted.

```rust
div().flex_col()
    .when(compact, |list| list.gap(px(2.0)))
    .when_some(heading, |list, title| list.child(text(title)))
    .children(devices.into_iter().map(|device| text(device.name)))
```

A stateful section can declare its mount boundary explicitly. In this sketch
`component` takes a factory. The framework invokes it once when attaching
each instance. The scope itself adds no layout box.

```rust
component(|cx| {
    let open = cx.signal(false);
    let volume = cx.signal(40_u8);

    div()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .padding(px(6.0))
                .child(text("Audio settings"))
                .on_click(move |event| event.update(open, |v| *v = !*v)),
        )
        .child_when(open, || {
            div().flex_col()
                .child(text("Output device"))
                .child(text("System default"))
        })
        .child(
            div().child(text("Volume"))
                .on_click(move |event| event.update(volume, |v| *v = v.saturating_add(1))),
        )
})
```

Pass a string directly to `child` when the text should inherit its parent's
typography. Borrowed strings are converted when the declaration is built.
A status row can keep its caption fixed and pass a `Signal<SharedString>`
as its second child to update the device name in place.

```rust
div().flex_row().gap(px(8.0))
    .child("Output device")
    .child(device_name)
```

Use `text(device_name).text_color(accent)` to style the value independently.
The `text` builder accepts a literal, an owned string, a `SharedString`, or a
`Signal<SharedString>`. Passing a clone of a long shared value lets multiple nodes retain the
same bytes through layout. Short labels fit inline. The [source text
experiment](text-layout.md#shared-source-text) measures both cases.

`Ui::new()` discovers installed fonts when the first text element needs them.
Native windows use the platform interface font by default. An application can register additional
fonts before mounting its root and select them by family name:

```rust
let mut fonts = Fonts::system();
let body = fonts.load_font("assets/Body.ttf")?;
let ui = Ui::with_fonts(fonts);
let content = div()
    .font_family(body)
    .child(text("Audio settings"))
    .child(text("48 kHz").font_family(GenericFontFamily::Monospace));
```

Strings passed to `.font_family(...)` are literal names. Use
`.font_family(GenericFontFamily::SystemUi)` on a label or container to select
the native interface font even when its parent specifies another family. The
Linux window host reads the desktop preference at first use of system text.
Standalone text engines and isolated collections resolve the category from
their available fonts.

Use `Fonts::set_generic_font_family(category, Some(name))` before creating
the UI to map a category to a registered family in both primary and fallback
positions. Pass `None` to restore platform resolution. A fallback list can
mix both kinds, e.g. `.font_fallbacks(["Body CJK".into(), GenericFontFamily::Serif.into()])`.

Text inherits its parent's font family, weight, style and ordered
`.font_fallbacks(...)`. An element can override any of these. If the selected
families cannot supply a glyph, the shaper searches the other available faces.
Applications can use `Fonts::add_font(bytes)` for in-memory data and
`set_default_family` for the application-wide default. Registration uses the
family names in the font file. The system locale guides script-specific fallback
and can be overridden with `Fonts::set_locale`.

Solor ships without a runtime font. Applications that need predictable coverage
can supply their own fonts. Text defaults to 16 logical pixels with a line height
of 24 and accepts up to 4096 UTF-8 bytes. A missing glyph produces a layout error.

The inner container owns the padded hit area. Its child can be any element
or composition. Moving the handler to another container changes which area
responds. A keyboard or accessibility activation policy would need its own
semantics on the interactive element.

The boolean starts at `false` when the factory runs. The local `open`
holds a copyable handle. The click handler and conditional branch keep copies
after the factory returns. Each click reads the current boolean and writes its
inverse during event dispatch. The new value is immediately readable. At the
next update boundary the branch receives the latest value and mounts or
removes its detail child. Two clicks before that boundary toggle twice. An
initially closed branch then stays closed.

Layout, painting, and opening or closing the detail preserve the section's
signal. Disposing the section releases its signal and bindings. Mounting
another instance initializes its own value. A caller that needs the value to
survive section removal can own the signal in a longer-lived scope and pass
the handle to an ordinary function that builds the section.

Each call to `cx.signal(value)` creates a distinct `Signal<T>`, including
when two calls have identical types and initial values. Handlers capture the
typed handles they use. The instance keeps a list of signal identities for
disposal. Business methods receive `&mut Context` and use `cx.read`, `cx.set`
or `cx.update`. Application-context signals live with the host. Signals
created during component rendering or a node's click live with that mount.
A playback model updates two views through the same context interface. Its
application-owned signal remains readable after both views retire.

The fixture stores `open` and `muted` as separate booleans alongside a numeric
volume and an owned device name. Its event update helper clones the selected
value before editing it and uses `Runtime::set` to suppress equal writes.

### Signal-backed properties

Style setters accept fixed values or signals. A fixed value is captured during
construction. A signal installs a subscription on the mounted element. Inherited
text properties follow the winning declaration on each descendant, including
children attached later.

```rust
let family = cx.signal(GenericFontFamily::SystemUi);
let width = cx.signal(px(240.0));
div().w(width).font_family(family)
    .child(text("Audio settings"))
    .child(text("48 kHz").font_family(GenericFontFamily::Monospace))
```

A flush applies the latest values in place and preserves node identity. Font and
box changes invalidate layout. Built-in text decorations and colors can repaint
using the current geometry. Custom elements receive updated values through
`prepare_content` and can use them for measurement.

A signal can own any `'static` type. For large buffers, use
`cx.update_in_place(samples, |s| s.push(sample))?`. The closure borrows the
stored value mutably and its return value is passed back to the caller. Each
call schedules subscribers even if the closure leaves the value unchanged.
Use `cx.notify(samples)?` after an external or interior-mutable edit. Repeated
notifications before a flush coalesce. `cx.update` also remains available for
cloneable values where equality should suppress a redundant publication.

### Lifecycle checks

The fixture uses the [state runtime](state-updates.md), column layout and
measured text leaves. It dispatches a click directly to the registered layout
node. The tests check immediate reads after a click, zero detail builds after
two clicks in one batch, stable node identity across resizes, and a fresh
detail node after closing and reopening. They also exercise independent
instances, disposal with a pending update, and remount with a caller-owned
signal.

The factory may return any `IntoElement`. A conditional child reserves a
declaration position and contributes a layout child only while open. The
factory can build another container, including nested conditionals. In the
audio section, opening and closing the middle subtree keeps the volume row's
identity, signal and handler. The column applies one gap between adjacent
mounted children.

Signal delivery queues the affected positions by instance. A flush processes
parents before descendants, picks up bindings created during attachment, and
can retry a failed attachment after removing its partial nodes and handlers.
With two independent sections, one toggle visits one position. A volume write
visits zero. These counts cover conditional dispatch only.

Each mounted view owns a scene. Children keep their original declaration
offsets, so opening a conditional can find its live position by binary search.
The child vector still shifts later siblings on insertion. Removal prunes
retired node IDs, and a flat column measures and places its direct children
on structural changes.

```sh
cargo test --offline -p solor
```

## Styles and bound labels

A reusable style refines only the fields it supplies. Later refinements win
per edge or property. Text size, line height and color propagate to children,
including conditionals attached later. The same box adapter measures padding,
offsets children, records the outer background and gives the element its inner
paint bounds. A custom primitive implements its own content layout and drawing.

```rust
let card = StyleRefinement::default().p_2().gap_1().text_size(px(16.0)).line_height(px(24.0));
div().with_style(&card).pl(px(12.0)).bg(background).rounded_md()
    .child(text(device_name))
```

Here `device_name` is a `Signal<SharedString>` and `background` is a
`Signal<Fill>`. A text change updates the retained label at flush, clears its
shape cache and schedules layout and local paint. Returning to the installed
string within the same batch preserves the cache. A background change requests
only local paint. Box dimensions accept automatic sizing, logical pixels, view-relative rems
and percentages. For example, `w(percent(50.0)).h(rem(2.0)).padding(px(1.25))`
sets a half-width box with a two-rem height. Width and height include padding.
The root receives the viewport allocation. Child percentages use the parent's
content dimension on the matching axis. A percentage with an indefinite basis
uses intrinsic sizing until placement supplies the content box.

`Ui::mount` uses a 16px rem basis. `mount_with_rem_size(component, px(20.0))`
sets a basis for that view's lifetime, including children attached later.
A descendant font-size override only changes typography. Padding, gap, radius
and text metrics accept logical pixels and rems. Spacing presets use steps of
0.25rem. Measurement, cached geometry, baselines and local painting use
floating-point logical pixels. Device scale and pixel snapping apply during rasterization.
Use `w_auto()`,
`h_auto()` or `clear_bg()` to clear an earlier declaration.

Corner presets use `xs`, `sm`, `md`, `lg`, `xl`, `2xl` and `3xl` for
0.125, 0.25, 0.375, 0.5, 0.75, 1 and 1.5rem respectively. Use `none` for
square corners and `rounded_full()` for a pill or circle. Apply a preset to
the whole box, one side (`t`, `r`, `b`, `l`) or one corner (`tl`, `tr`,
`br`, `bl`). For example, `div().rounded_lg().rounded_b_none()` keeps only
the top corners rounded. Each group also accepts an explicit radius, as in
`.rounded_r(px(10.0))`.

```rust
let surface = gradient_left_to_right([
    color_stop(rgb(0x203048), 0.0),
    color_stop(rgb(0x527f96), 0.35),
    color_stop(rgb(0xb9ced0), 0.7),
    color_stop(rgb(0xf8dab5), 1.0),
])?.color_space(ColorSpace::Oklab);
div().bg(surface).rounded_lg()
```

All gradients take an ordered sequence of at least two stops, with no fixed
maximum. A clone shares its stop allocation. Editing a shared stop detaches
the edited copy. Equal offsets form hard edges. A segment midpoint sets the
position of its 50% color mix. Interpolation uses premultiplied alpha in sRGB
or Oklab. The `color` crate supplies those conversions.

`Gradient::new(kind, angle, stops)` supports linear, radial, angular and
diamond geometry. Zero degrees points up and positive angles rotate clockwise.
`phase(turns)` samples a repeating cubic spline through neighboring control
colors and ignores segment midpoints. `angular_seam_width(turns)` blends the
ends of an angular ramp in non-repeating mode. Opacity and phase updates share
the same stops. `sample_at` evaluates local box coordinates on the CPU.
Background and content draws retain the fill for frame composition.

The [UI interface design](ui-contracts.md) gives the acceptance cases for
sizing, composition, input, text, richer paint and platform adapters.

## High-frequency updates

Elements default to `RefreshMode::OnDemand`. During paint, `cx.read(signal)?`
borrows the current value for the paint pass and records a dependency for that
element. A chart can iterate over borrowed samples while issuing draw commands.
Each successful paint replaces its previous set of dependencies. A conditional
read therefore follows the active branch. Equal writes through `set` leave the
element clean. An `update_in_place` schedules its subscribers after every edit,
and multiple notifications before a flush coalesce. Reads in event handlers are
ordinary reads. Layout or attachment changes can also require fresh paint.

A panel can mix both cadences in one retained tree:

```rust
let mode = cx.signal(RefreshMode::OnDemand);
div()
    .flex_col()
    .child(text(device_name))
    .child(element(Meter(level)).refresh_mode(mode))
    .child(div().child(text("Start sampling")).on_click(move |cx| {
        cx.set(mode, RefreshMode::Continuous)
    }))
    .child(div().child(text("Pause sampling")).on_click(move |cx| {
        cx.set(mode, RefreshMode::OnDemand)
    }))
```

The meter can read `level` in its paint method. In continuous mode it also
samples on every host frame when that value stays constant. Switching back
to on demand records the current value and subsequent changes still wake the
meter. For a fixed cadence pass `RefreshMode::Continuous` directly. Each
element keeps its mounted identity across mode changes.

The window coalesces requests and rearms after successful submission. A
`child_when` binding can change structure and a bound label can change its
measured size. Signal reads in custom paint invalidate drawing only. The CPU
backend assembles and compares whole-view command lists and rasterizes
intersecting draws inside damage. See the
[refresh workloads](text-pixels.md#retained-and-continuous-elements).

The release harness uses 100, 1,000 and 10,000 siblings. It excludes setup
and warm-up, times individual iterations and checks the sampled width,
command count or resulting tree membership. The following ranges span five
independent runs on Linux with default features. The two timing columns give
the range of per-run medians and the range of per-run 95th percentiles.

| Work per update boundary | Median | p95 |
| --- | ---: | ---: |
| One write and one meter paint among 10,000 static siblings | 0.181–0.200 µs | 0.201–0.220 µs |
| 1,000 writes and one caller-selected meter paint among 10,000 static siblings | 3.587–4.018 µs | 3.607–4.047 µs |
| 1,000 writes, collect repaint requests and paint the subscribed meter among 10,000 static siblings | 3.246–3.527 µs | 3.266–3.557 µs |
| One write and 10,000 retained meter paints | 1.778–1.912 ms | 2.547–3.431 ms |
| Toggle a final row and lay out a flat column with 10,000 static siblings | 0.703–0.765 ms | 0.747–0.797 ms |
| 1,000 toggles with 1,000 conditional subscribers, all ending closed | 0.093–0.098 ms | 0.099–0.105 ms |

The meter rows include writes, flush, unchanged layout and local fill
recording. The structural row includes dispatch and layout. In a separate
five-pair alternating run, the shared-subscriber median was 0.786–0.816 ms
when each write walked the subscriber list and 0.087–0.091 ms with the
per-signal queue flag. Rasterization, text reshaping, damage, buffer upload,
compositor pacing and memory usage were outside these timings.

```sh
cargo bench -p solor --bench refresh --offline
```

Use `.on_frame(|now, cx| { ... })` to advance application state on host frame
opportunities. A visible listener requests the next callback with its frame.
Delivery uses the last accepted view snapshot and stops when the listener is
hidden or unmounted. All listeners for one callback receive the same monotonic
dispatch timestamp. Signal writes are flushed before layout. A custom element
that samples a stream directly in paint can combine this with
`.refresh_mode(RefreshMode::Continuous)`. Its `PaintContext::fill_rect` takes an
offset in content-local logical pixels and an extent.

## Custom drawing and input

The [pointer input interface](pointer-input.md) shows how a seek bar can
track one drag across its bounds and share the signal update path with a button.

A container can draw directly into its content box. The paint callback runs
after its background and before its children. Coordinates are relative to the
content origin, and `cx.size` excludes padding. Reading a signal subscribes the
container to updates.

```rust
fn progress_bar(progress: Signal<f32>) -> Div {
    div()
        .w_full()
        .h(px(4.0))
        .bg(rgb(0x343434))
        .paint(move |cx| {
            let fraction = cx.read(progress)?.clamp(0.0, 1.0);
            let size = Size { width: cx.size.width * fraction, ..cx.size };
            cx.fill(size, rgb(0x72aaff));
            Ok(())
        })
}
```

Use `cx.fill_circle(center, radius, fill)` for round markers. The center and
radius are logical pixels in the content box, and gradients span the circle's
bounding square.

Use `cx.with_clip(offset, size, |cx| ...)` to crop part of a drawing, such as
the price trace inside a plot. The rectangle uses the same content-local
coordinates as `fill_rect`. Each nested rectangle intersects the active clip
and any ancestor overflow clip. When the callback returns, subsequent commands
use the enclosing clip. Children and pointer hit testing follow the container's
layout and overflow styles.

## Framework boundary

The progress bar is an application function built with the same container as the
expandable section. Its caller can use that container directly to paint a
waveform, add a child label, or choose different pointer handling. A paint
callback records the element's own content before its children are drawn.

Keep layout, text, input, state binding and custom drawing available to
component authors. Buttons, cards, sliders and player controls can live in
application code or a component library built on those same operations.
A new primitive needs a distinct framework responsibility, such as text
measurement or a custom child layout policy. A new appearance alone can be
expressed through composition or painting.
