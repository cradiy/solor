# Component authoring

`solor-ui` provides component composition, retained element layout, local
paint recording and direct click dispatch. A host creates a `Ui`, mounts a
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

The `text` builder accepts a literal, an owned string, a `SharedString`, or a
`Signal<SharedString>`. Passing a clone of a long shared value lets multiple nodes retain the
same bytes through layout. Short labels fit inline. The [source text
experiment](text-layout.md#shared-source-text) measures both cases.

`Ui::new()` supports custom elements and containers. To mount text, create
the UI with `Ui::with_font(font_bytes, locale)?`. The current text leaf shapes
one supplied face, defaults to size 16 and line height 24, and accepts up to 4096 UTF-8
bytes. A missing glyph produces a layout error.

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
cargo test --offline -p solor-ui
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

A meter with fixed geometry can retain a `Signal<u32>` and sample it inside
`Element::paint` with `cx.read(level)?`. Declare
`element(Meter(level)).repaint_on(level)` to subscribe. At the display boundary
the host calls `flush`, then `collect_repaints(view, &mut nodes)` into a cleared,
reusable vector. Each node appears once until its local commands are successfully
recorded. An idle page with no outstanding requests yields an empty list. Paint
reads sample the current value. Host frame wakeups, geometry damage and successful
window submission have separate lifetimes. `child_when`
registers a structural subscription and can change layout. Applications that
consume every input sample must process those samples before publishing the
value used for display.

The release harness uses 100, 1,000 and 10,000 siblings. It excludes setup
and warm-up, times individual iterations and checks the sampled width,
command count or resulting tree membership. The following ranges span five
independent runs on Linux with default features. The two timing columns give
the range of per-run medians and the range of per-run 95th percentiles.

| Work per update boundary | Median | p95 |
| --- | ---: | ---: |
| One write and one meter paint among 10,000 static siblings | 0.100–0.110 µs | 0.110–0.120 µs |
| 1,000 writes and one caller-selected meter paint among 10,000 static siblings | 3.407–3.487 µs | 3.427–5.020 µs |
| 1,000 writes, collect repaint requests and paint the subscribed meter among 10,000 static siblings | 3.276–3.527 µs | 3.326–3.557 µs |
| One write and 10,000 retained meter paints | 0.630–0.673 ms | 0.701–1.159 ms |
| Toggle a final row and lay out a flat column with 10,000 static siblings | 0.468–0.498 ms | 0.506–0.602 ms |
| 1,000 toggles with 1,000 conditional subscribers, all ending closed | 0.083–0.086 ms | 0.093–0.135 ms |

The meter rows include writes, flush, unchanged layout and local fill
recording. The structural row includes dispatch and layout. In a separate
five-pair alternating run, the shared-subscriber median was 0.786–0.816 ms
when each write walked the subscriber list and 0.087–0.091 ms with the
per-signal queue flag. Rasterization, text reshaping, damage, buffer upload,
compositor pacing and memory usage were outside these timings.

```sh
cargo bench -p solor-ui --bench refresh --offline
```

## Custom drawing and input

The same builder syntax should leave room for an element whose appearance
depends on allocated space. Here the caller supplies playback progress in
the range 0 to 1 and a function accepting a fractional seek position.

```rust
fn seek_bar(
    progress: Signal<f32>,
    seek: impl Fn(f32) + 'static,
) -> impl IntoElement {
    div()
        .fill_width()
        .height(px(20.0))
        .paint(progress, |p, fraction| {
            let track = p.bounds().centered_strip(4);
            p.fill_rect(track, rgb(0x343434));
            p.fill_rect(track.with_width(track.width() * fraction), rgb(0x72aaff));
        })
        .on_drag(move |drag| {
            let width = drag.bounds().width();
            if width > 0.0 {
                seek((drag.local_x() / width).clamp(0.0, 1.0));
            }
        })
}
```

In this sketch the paint callback receives the current bound value by copy and records in local
logical coordinates. A progress update invalidates this paint binding. An
allocation change also invalidates its bounds-dependent drawing. Drag positions
use the accepted frame's local coordinates and bounds. Retirement cancels the
active drag. The caller decides when a seek changes the playback signal.

## Framework boundary

The seek bar is an application function built with the same container as the
expandable section. Its caller can use that container directly to paint a
waveform, add a child label, or choose different pointer handling. A paint
callback records the element's own content before its children are drawn.

Keep layout, text, input, state binding and custom drawing available to
component authors. Buttons, cards, sliders and player controls can live in
application code or a component library built on those same operations.
A new primitive needs a distinct framework responsibility, such as text
measurement or a custom child layout policy. A new appearance alone can be
expressed through composition or painting.
