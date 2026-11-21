# UI interface design

A settings page and an audio meter can share one retained tree. A device rename
can change wrapping and move the rows below it. A level sample can change the
meter's fill while its bounds and its siblings stay fixed. The contract for
each property must identify who owns it, which descendants inherit it, which
stage consumes it, and how a host learns about the resulting work.

Run `cargo run -p solor-gallery --release` in a Linux Wayland session to compare
letter spacing, OpenType ligatures, gradient geometry and flex distribution.
The Lists tab shows the 24-bit RGB color space in a virtual list. Scroll to
browse swatches, or click and drag the position indicator to seek across all
16,777,216 colors. A slim thumb sits over the list’s right edge and widens on hover.
Choose **Tone sheet** to browse nine color families down the rows and nine tones
across the columns. Both scrollbars control the same viewport. Selecting a swatch
shows its name and hex value above the sheet, where they stay visible while scrolling.

The Markets tab runs a deterministic feed of six instruments. In a short window,
the watchlist rows scroll beneath their fixed header. Each host frame
advances 32 ticks per instrument and appends the latest price to a 96-sample
history. The green price line and amber 16-sample moving average share the same
axis. A translucent polygon marks the area under the price line. Scroll vertically over the plot to zoom its latest 12 to 96 samples.
The plot passes horizontal motion and vertical distance beyond either zoom limit
to its enclosing scroll listener. The price axis retains the full history range. Moving the mouse over the plot
marks the nearest visible sample and shows its price and moving average.
Resizing the plot updates the selection under a stationary mouse.
Zooming selects the nearest sample in the new range at the same horizontal
fraction. Each instrument owns a signal
with a fixed-size history updated in place. The chart reads the selected
instrument during paint. Its `on_frame_when(running, handler)` callback advances
the feed while playing. Pausing releases the frame request and keeps the same
chart mounted for selection, hover and zoom. Callback FPS counts intervals between
host frame opportunities over at least one second of elapsed monotonic time.

For unpaced CPU measurements, run
`cargo test --release -p solor-gallery --test navigation market_frame_cost -- --ignored --nocapture`.
The test drives the same mounted page and measures updates, layout, recording,
CPU rasterization and submission acknowledgement at 1× and 1.5× scale,
with the pointer outside the plot and with a sample under inspection.
Window-system upload and display timing are outside this benchmark.

## Implemented boundaries

| Interface | Current behavior |
| --- | --- |
| Composition and ownership | A component renders once per attachment. A conditional owns its branch. Retiring a node releases its subscriptions and local signals. The host owns application signals. |
| Box layout | Lengths in logical pixels, rems and percentages resolve to floating-point geometry. Every primitive supports box sizing and per-edge padding. Content layout, painting and child origins use the inner bounds. A container applies the gap on its main axis. |
| Style refinements | Later declarations override individual specified fields. Text size, line height, alignment and color propagate into fixed and conditional descendants. A bound property follows its signal for the lifetime of its mount. |
| Text | A fixed or signal-bound label owns a shared string and caches shaped paragraphs by width. Updating bound text keeps the element ID. A changed label schedules layout and, when visible, local paint. |
| Fill | A straight-alpha sRGB color has named channels. Every gradient shares an ordered, variable-length stop sequence. Four geometries sample it in sRGB or Oklab. Backgrounds and custom fills carry the same fill value. Equal offsets describe hard edges. Rounded fills use the assigned dimensions. |
| Paint scheduling | Each successful paint subscribes the element to the signals it read. Repeated writes coalesce. Elements default to on-demand recording. A fixed or signal-driven continuous mode samples every host frame. |
| Pointer input | The host accepts a border-box snapshot for hit testing. Positioned events deliver local logical coordinates. A press can capture its pointer until release or cancellation. See [pointer input](pointer-input.md). |
| Platform boundary | `solor` owns the GUI and the platform traits. `solor-linux` owns the Wayland event loop and buffer submission. `solor-render-cpu` rasterizes shaped text. `solor-platform` selects the window system and its rendering backend. |

## Platform and renderer ownership

Applications obtain a host with `solor_platform::current_platform()?` and
call `platform.run_window(options, ui, root)`. The host mounts the root and
owns the pairing of the window system with a compatible renderer. The same
application entry point compiles on other targets. Platform selection returns
an unsupported-platform error where a host is unavailable.

`WindowOptions::new("Settings")` requests an 800 × 600 logical-pixel window.
Use `.size(px(480.0), px(320.0))` for a different preference and
`.app_id("org.example.settings")` to identify the application to the window system.

Windows request the native frame and title bar by default. Pass a boolean to
`WindowOptions::new("Settings").decorations(enabled)` or use
`.without_decorations()` to start with a frameless window. A title bar can be composed from ordinary elements.

```rust
div()
    .flex_row()
    .items_center()
    .justify_between()
    .window_drag_region()
    .child(text("Settings"))
    .child(div().window_decorations_toggle().child(text("Toggle frame")))
    .child(div().window_close_button().child(text("Close")))
```

A primary press in the drag region requests an interactive move with the
originating input event. Interactive children receive their own input.
`window_minimize_button()`, `window_maximize_button()` and `window_close_button()`
act on a completed click. Maximize toggles the compositor-confirmed state.
The host uses the same submitted hit geometry for window controls and application buttons.
A completed click on `window_decorations_toggle()` reverses the decoration
preference for that open window. On Wayland the compositor chooses the final
mode. Without decoration negotiation the window remains client-decorated.

Use `.window_resize_region(ResizeEdge::BottomRight)` on a sized element for
a corner grip. All four edges and four corners are supported. Regions use the
accepted hit geometry and respect ancestor clips and interactive children.
The primary press starts a native resize unless its handler captures the
pointer. The window system supplies the resulting sizes through normal
window configuration. The gallery uses the top edge and both top corners.

The Linux host pairs Wayland with the CPU renderer and runs one XDG toplevel
until close. The driver accepts a `WindowHandler<Rgba8>` and borrows its
output for one submission.

Pointer positions and scroll distances arrive in logical pixels. The Linux
adapter currently delivers mouse buttons, motion, cancellation and two-axis
scroll input. Keyboard modifiers are currently reported as empty. An event
handler can request a redraw. The host supplies the logical allocation and
device scale at frame preparation, validates the returned buffer, then
acknowledges submission with `frame_submitted`. A successful empty-damage
frame receives the same acknowledgement. The compositor controls display timing.

## Recording a mounted view

After flushing signals and laying out a view, call `Ui::record_frame(view)`.
The returned `ViewFrame` owns the ordered commands, retains the font data and
shaped paragraphs, and carries the pointer geometry for that layout. Both
painting and hit testing use the same view-local node origins and stacking order. Each
text command includes its allocated content box for clipping. A failed recording
preserves outstanding repaint requests for retry.

`Surface::draw_view` in the CPU backend replays the frame onto an opaque white
RGBA8 image. Supply the physical target size and its device scale. Gradients
are sampled in the original fill box even when the target clips it. After
successful presentation, pass `frame.into_pointer_frame()` to
`Ui::accept_pointer_frame`. Discarding the frame leaves the accepted input
geometry in place. This path records the whole view and rasterizes a fresh
image on each call.

`PaintContext::stroke_polyline(points, width, color)` draws connected points
relative to the element's content origin. Width is in logical pixels. The CPU
backend antialiases round caps and joins, clips the stroke to inherited overflow
bounds and composites a translucent stroke once at self-intersections.

`PaintContext::fill_polygon(points, fill)` closes the contour between the last
and first vertices. An odd number of edge crossings places a sample inside the
polygon. Solid colors and gradients use the same antialiased contour. Gradient
coordinates cover the full vertex bounds and stay fixed during partial repairs.

## Presenting a mounted view

Inside the Linux host, `CpuWindow::mount(ui, component)` owns the UI and
mounts the root once. The host passes this handler to the Wayland driver. It
dispatches pointer events against the accepted layout and requests a frame
when signal bindings have work to flush. Each frame records the view and
compares the ordered commands with the previous submission. The window keeps
one RGBA surface and repairs a bounded set of rectangles covering changed commands,
including the space vacated by shrinking or removed content. Inserting or removing
a command leaves an unchanged suffix outside the damage calculation. Draws
overlapping each repair are replayed in paint order. Fully covering opaque base colors initialize the repaired pixels in a single pass. Identical commands
produce empty damage. Changes to the allocation or scale and retries after a discarded
prepared image require full damage. The platform acknowledges successful
submission before the handler publishes its prepared hit geometry. On the next
frame callback the window continues if the submitted frame requested another
frame or signal bindings have pending work. Otherwise it returns to idle.

The CPU window reuses glyph masks across full frames and partial repairs.
Each window owns a cache for its font face, keyed by glyph and device font size.
It retains up to 64 masks and 32 KiB of mask pixel storage. Text color and
clip are applied when compositing each mask.

Run `cargo run -p solor-platform --example mounted_window` in a Linux
Wayland session. Click **Audio settings** to expand the row, then click
**Close settings** to collapse it. The newly mounted row has its own handler.

## Style declarations

`Styled` supplies the same builders on containers, text, custom element
wrappers and reusable `StyleRefinement` values. Each setter writes the
individual fields it names. For example, `.m(rem(1.0)).mx_auto().mt(px(-2.5))`
leaves the bottom margin at one rem and both horizontal margins automatic.
A later `.m(...)` replaces all four. Margins and position offsets accept
logical pixels, rems, percentages and automatic values.

| Property family | Available declarations |
| --- | --- |
| Box and placement | Display, box sizing, width and height limits, aspect ratio, per-edge margins, position, four insets, stacking order and visibility |
| Flex and grid | Direction, wrap, order, basis, grow/shrink, item and content alignment, row and column gaps, explicit and implicit track lists, automatic placement, line numbers and spans |
| Overflow and input | Independent horizontal and vertical overflow, scrollbar width, cursor and pointer targeting |
| Paint | Per-edge border widths, fills and patterns, individual corner radii, outlines, shadow layers, opacity, backdrop blur, a 2D transform list and origin, object fit and position |
| Text | Family, fallbacks, OpenType features, weight, slant, spacing, alignment, whitespace, truncation, line limit, run background, underline and strikethrough |

A custom element receives its resolved style in `Element::prepare_content` at
attachment and when a bound style input changes. Text attributes cascade from the
parent, including when a conditional child is attached later. An explicit
`clear_line_clamp()` removes an inherited line limit.

Explicit width and height include padding by default. With
`.box_sizing(BoxSizing::ContentBox)`, they describe the content area, and
padding adds to the preferred outer size. Percentages use the parent's
definite content dimension on the same axis. Text wraps within the allocated
content width.

Use `.aspect_ratio(width / height)` to derive an automatic dimension from
a definite one, or `.aspect_square()` for a ratio of one. Ratios must be
positive and finite. By default the ratio applies to the border box. With
`BoxSizing::ContentBox` it applies inside the padding. A flex allocation
can supply the definite dimension, so a growing preview can reserve its
height at its final width. Explicit dimensions, size limits and stretch
allocations take precedence. When both dimensions are intrinsic, the
content supplies the preferred size.

Use `.min_w(...)` and `.max_w(...)` to limit a child's preferred or stretched
width. Both use the selected box model. The minimum takes precedence when the
limits conflict and can make a flex item overflow the space between its
margins. A percentage limit applies once the parent content width is definite.
Custom layouts that call `measure_child` directly retain control of the
maximum or exact width they pass. The host assigns the root's viewport size.

`.min_h(...)` and `.max_h(...)` bound preferred height and row stretch using
the same box model. Percentage limits use the parent's definite content
height. If the minimum exceeds the maximum, the minimum wins. When a limit
determines a container's height, its children use the resulting content height
for percentage sizing and alignment. Following siblings advance by the used
box height. Content bounds can extend farther, and a label keeps the full
paragraph when its box is capped. A baseline outside the assigned box is
omitted.

A `div()` uses vertical flow by default. Both `.flex_row()` and `.flex_col()`
set its direction through the same style refinement, before or after adding
children. Horizontal flow uses the column gap (`gap_x`), vertical flow uses
the row gap (`gap_y`), and `gap` sets both. Reverse directions start placement
at the far edge while keeping child identity and declaration order.

Use `.flex_basis(length)` to set a child's starting width in a row or
height in a column. A definite basis takes precedence over the preferred
size on that axis and uses the child's box sizing and min/max limits.
Percentages use the parent's content extent along the main axis. During
intrinsic measurement with an indefinite extent, a percentage basis uses
the child's content. `Length::Auto` uses the usual preferred size. Gaps and
margins are counted separately when the line is positioned.

Use `.flex_grow(factor)` to share positive main-axis space after the line's
measured item sizes, fixed margins and gaps. Growth starts at each item's
measured size, including its min/max constraints. Items that reach their
maximum stop growing, and the other items can use the surplus. A total
factor below one uses that fraction of the surplus. Main-axis auto margins
and justification receive the space left after growth. The `.flex_1()` preset
sets both grow and shrink to one with a zero-percent basis.

Use `.flex_shrink(factor)` to let an item give up main-axis space on a
single line. The default factor is zero. Each participating item's weight
is its factor times its measured content extent before allocation. Padding
and minimum sizes limit how much the border box can give up. When a minimum
binds, the remaining items share the outstanding deficit. If their factors
sum to less than one, they absorb at most that fraction of the original
deficit. Justification positions any remaining overflow. A row measures a
shrinkable label at its allocated width so following rows account for wrapping.

Use `.justify_center()` or `.justify_end()` to place the whole line, and
`.justify_between()`, `.justify_around()` or `.justify_evenly()` to distribute
surplus space in addition to the declared gap. Main-axis auto margins share
positive free space before alignment. When the line overflows, distributed
spacing falls back to the start edge. Center and end keep their alignment
even when this places some content before the start edge.

Use `.items_start()`, `.items_center()` or `.items_end()` to position children
on the cross axis. A child can override its container using `.self_start()`,
`.self_center()` or `.self_end()`. The default is start. Alignment uses the
child's margin box within the container's padded content area. Auto margins
on that axis take precedence. Reversing the main axis preserves the cross-axis
direction.

Use `.items_stretch()` for a line or `.self_stretch()` for one child. A child
with an automatic cross size fills the content area between its fixed margins.
An explicit cross size or an automatic cross-axis margin keeps the child's
measured size. A column with a definite width measures stretched children at
their allocated width, so wrapped text contributes its resulting height to
the line.

In a row, `.items_baseline()` aligns the first baselines of its children.
Use `.self_baseline()` to join that group from an individual child. A child
without a baseline participates at its bottom border edge. A vertical auto
margin takes precedence. The preferred row height accommodates the largest
measured extent above the baseline and the largest below it, including fixed
margins. A nested row exposes its shared baseline to its parent. In a column,
baseline alignment uses the start edge of the horizontal axis.

Horizontal flow normally measures a child against the container's content width
less its fixed margins. A shrinkable child with an automatic basis starts at
its preferred width before the row distributes the deficit. The line can overflow when the combined preferred
widths and gaps exceed its allocation. Automatic horizontal margins share
positive space left over across the entire line. Flex wrapping is a declaration pending layout support.

Vertical flow measures and places child border boxes with their margins.
Adjacent vertical margins add to the row gap. Signed margins can overlap
siblings. Fixed horizontal margins reduce the width available to the child.
One automatic horizontal margin takes the remaining positive space, and two
share it equally. Automatic vertical margins share surplus height during placement. Percentages on
all four edges use the column's content width. Their intrinsic contribution
is zero while that width is indefinite, and placement resolves them against
the assigned width. Root margins do not change the host's viewport allocation.

Use `.text_color(ink)` with a color or a `Signal<Rgba>` on a label or an
ancestor. Descendants inherit the source until they supply their own color.
Custom elements read it with `cx.text_color()?` during paint. A color change
repaints the elements that read it and reuses their shaped paragraphs and layout.

Set OpenType features on a label or container with
`.font_features([FontFeature { tag: *b"liga", value: 0 }])` to disable
standard ligatures. The active font determines which tags it supports.
Descendants inherit the ordered list. A local list replaces it, and
`.font_features([])` uses the font's shaping defaults. Measurement, wrapping
and painting use the same shaped glyphs.

Use `.text_bg(color)` to highlight the occupied width of each visual line.
The highlight fills the line height behind glyphs and decorations and clips
to the label's content box. Empty lines leave a gap. Descendants inherit the
color and can clear it with `.text_bg(rgba(0))`.

Use `.underline()` and `.line_through()` independently or together. Each
visible line is decorated across its shaped advances inside the text content
box. The face supplies the baseline offsets and natural thickness, with
em-based defaults for missing metrics. Strokes follow the text foreground.
Use `.decoration_color(color)` and `.decoration_thickness(px(1.5))` to override
them. Thickness also accepts rem units. Children can disable an inherited
stroke with `.text_underline(false)` or `.text_strikethrough(false)`.

`.letter_spacing(px(0.75))` adds 0.75 logical pixels to each shaped
glyph's advance, including the final advance in the line width. A ligature
receives one adjustment. Negative values tighten the text. Values in rems
use the mounted view's rem size. Descendant labels inherit the spacing and
can reset it with `.letter_spacing(px(0.0))`. Wrapping and ellipsis placement
use the adjusted advances.

Paragraph alignment uses the label's content width after padding. Set
`.text_center()` on a container to center each visual line in its labels.
A label can override it with `.text_left()` or `.text_right()`. The default,
`.text_start()`, and its counterpart `.text_end()` follow each paragraph's
writing direction. `.text_justify()` distributes extra space between words on
wrapped lines and leaves the last line at the start edge.

Use `.line_clamp(n)` to limit a label to a nonzero number of visual lines.
Wrapped lines and explicit line breaks count toward the same limit. The label
measures and paints the visible lines, so a shorter value also takes less
height when the box is auto-sized. The paragraph retains the full source
string. Descendant labels inherit the limit and can remove it with
`.clear_line_clamp()`.

Use `.truncate()` for a single-line label with a trailing ellipsis. For a
filename whose extension matters, use `.truncate().text_ellipsis_middle()` to
keep both ends, or `.truncate().text_ellipsis_start()` to keep the suffix.
The marker appears when the line exceeds its content width. The full source
string and original UTF-8 offsets remain available in the recorded paragraph.

Labels preserve explicit line breaks and wrap by default. Both
`.whitespace_nowrap()` and `.whitespace_normal()` fold runs of ASCII whitespace
into a single space and trim the ends. The former keeps the result on one
line, and the latter allows wrapping. `WhiteSpace::Pre` preserves whitespace
and disables soft wrapping.

For a wrapped preview, combine `.line_clamp(n).text_ellipsis()`. When another
visual line follows, the last visible line fits a trailing marker inside its
content width. A hard break also counts, even if the last visible line is
empty. The preceding lines keep their original wrapping. Start and middle
ellipsis placement apply to unwrapped lines.

Use `.relative()` on a child to shift its entire subtree from the position
assigned by its parent. The child keeps its original space in the line.
For example, `.relative().left(percent(25.0)).top(rem(0.5))` moves it right
by a quarter of the parent's content width and down by half a rem. Vertical
percentages use the parent's content height. Left and top take precedence
over right and bottom when both opposing offsets are definite. An automatic
edge defers to its opposite. The parent includes the shifted subtree's far
edges in its content extent. The view root stays at the viewport origin.

Use `.invisible()` to keep a subtree mounted and reserve its layout space.
A hidden meter can receive layout updates while its painting and frame
requests are suspended. Signal-bound text still reflows and can move visible
siblings. The host omits the entire subtree from drawing and pointer hit
testing, including children attached later. A child marked `.visible()` appears
only under visible ancestors.

Use `.visible_when(show_meter)` with a boolean signal to switch visibility on
an existing mount. The initial value applies at attachment. `Ui::flush()`
applies later writes together, preserving the node's identity and layout.
Hidden elements release their paint subscriptions. Showing an element records
its current values and resumes its refresh policy. A local recorder can use
`collect_repaints` to find changed nodes and replace a hidden node's commands
with the empty result of `Ui::paint`. Pointer targeting switches when the
host accepts the new frame.

Use `.z_index(i32)` to order overlapping siblings. Each parent paints its own
content first, then its child subtrees in ascending layer order. The default
layer is zero. Equal layers keep declaration order. A child's layer applies
within its parent, so moving a card in front carries its labels and controls
with it. Flex placement continues to use the declared layout direction.

The `Hidden`, `Scroll` and `Auto` overflow policies clip content and descendant
hits to the element's border box, including its padding. Use `.overflow_hidden()`
to clip both axes, or set an axis independently with a value or signal such as
`.overflow_y(Overflow::Scroll)`. Containers with `Scroll` or `Auto` overflow
move their children along the enabled axes in response to wheel input. The
background and the container's own drawing stay fixed in the viewport. Each viewport keeps its full local content extent in `Ui::geometry`.
On a clipped axis, the child's border edge bounds its contribution to the parent's
content extent, including when the child is relatively positioned. On a visible
axis, descendant overflow can extend the parent's content. Nested clips intersect
independently on each axis. The element's own background inherits its ancestor clip. View recordings attach the effective clip to each draw command. Local
recordings from `Ui::paint` use the element's own coordinate system and apply
its own overflow setting. The CPU backend rounds clip edges to device pixels.

A host can read a laid-out overflow container with `ui.scroll_metrics(view, node)`.
The snapshot contains its current two-axis offset, maximum scroll distance and
viewport size in logical pixels. To restore a saved position, call
`ui.set_scroll_offset(view, node, saved_offset)?` after layout. The setter clamps
each axis to the current bounds and requests a redraw when the offset changes.
Readback reflects the new offset immediately. Pointer hits switch to the new
geometry when the host accepts the frame.

Each background corner has an independent circular radius. If two adjacent
radii exceed their shared edge, the frame scales all four by the same factor
until they fit. The CPU renderer applies this shape to solid and gradient
backgrounds. Overflow clipping and pointer hit boxes remain rectangular. Additional
layout, paint, text shaping and input behaviors in this table are declarations
pending engine support. In particular, `.absolute().top(px(4.0)).right(px(8.0))`
expresses placement but does not yet move a child or remove it from column flow.

## Contracts to establish next

| Priority and capability | Required behavior and acceptance case |
| --- | --- |
| 1. Frame composition, clipping and hit testing | Compose parent-local coordinates, scale, clips and sibling order into one accepted frame. Move a rounded card over another target, fail one submission, then retry. Pixels and click targets must agree with the last accepted frame. Old bounds must be repaired when a wrapped label moves its siblings. |
| 2. Sizing, placement and scrolling | Apply positioning and per-edge borders during measurement and placement. Resolve scrolling and overflow. Validate a long label between a fixed icon and an action at narrow and wide widths. Scrolling must share the visual clip and hit-test transform. |
| 3. Input and state-dependent styles | Connect platform pointer adapters and define propagation, keyboard focus, tab order and actions. Define precedence for hover, press, focus and disabled states, including removing an override. Validate a pressed element that moves, loses capture or unmounts before release. |
| 4. Text, editing and semantics | Apply font selection and fallback, weight and richer inline styles. Define editable ranges, composition, caret bounds and accessibility actions. A bilingual field must expose the same selection to keyboard, IME and assistive input. Preserve explicit units when converting platform text ranges. |
| 5. Paint and animation | Define border and shadow paint order, opacity of overlapping descendants, transformed and rounded clipping, and animation timing. A frame-clock repaint must stop on unmount. CPU replay must draw every gradient stop, preserve hard edges and match the declared interpolation. A GPU representation needs equivalent output and a resource lifetime through submission. |
| 6. Large collections and resources | Define keyed identity, viewport materialization, scroll anchoring, image/font resource ownership and cancellation. Replace and reorder items while keeping focus on a surviving key. Late resource completion must not invalidate a recycled node. |

Linux is the first presentation target. Geometry, semantic actions, text ranges
and frame requests need platform-independent meanings for Web, macOS, Windows,
Android and iOS. Native handles, buffer formats and backend resource indices
belong in adapters. Vector paths, custom shaders and 3D
need concrete workloads before their public signatures are chosen.


## Image fills and image elements

A background image should paint within the element's assigned box. Loading a
new frame or replacing a background should invalidate paint. A standalone
image element also has an intrinsic size and aspect ratio. If layout used
those metrics, discovering or changing them should invalidate measurement.

Both uses should share a decoded image resource. A fill would carry a resource
reference and placement parameters. Loading, decoding and caching belong to
the resource owner. A paint command must retain its selected frame through
submission. Changing the source or unmounting the element must detach its
pending notification.

The acceptance case is one wide image used both as a rounded cover background
and as an auto-sized image. The background must keep its box, preserve the
source aspect ratio and clip to the rounded destination. The image element
must remeasure when its intrinsic dimensions arrive. A previously recorded
frame must remain valid after either use changes its source. Image fills,
image elements and image loading are future work.

## Uniform lists

Give a vertical list a viewport with a fixed height or a flex allocation. The list assigns
each row its content width and the declared row height. The list's preferred size is zero.
A fixed or signal-bound item count supplies index keys without allocating a collection.

```rust
uniform_list(100_000usize, px(28.0), |index| {
    text(format!("Row {index}"))
})
.flex_1()
```

Pass a vector of unique keys or a `Signal<Vec<K>>` when items have their own identity.
The builder receives the key. A row stays mounted while its key is visible or within the
one-row buffer on either side. Reordering those keys preserves their local state.
Leaving the buffer retires the row and its subscriptions. Keep selection and edits that
outlive the row in application state. Signal changes to the key sequence preserve the
logical scroll distance, clamped to the new extent.

## Variable-height lists

Use `variable_list(items, estimated_height, build_row)` for rows whose height
depends on their content. Each mounted row is measured at the viewport's
content width and must have a positive preferred height. For example, a text
row can grow when the window narrows or the text changes.

```rust
variable_list(messages, px(48.0), move |id| {
    text(message_body(id)).line_height(px(20.0)).py(px(4.0))
})
.flex_1()
```

The item sources and keyed row lifetime match `uniform_list`. The estimate
locates distant rows without building them. Choose a typical row height to
reduce the number of mounts needed to fill the viewport. The list retains
measurements for the mounted window. A distant jump can therefore use an
approximate pixel distance until the destination is measured.

On reflow, the list keeps the top row and its within-row offset where the new
bounds allow. When already at the bottom, remeasurement keeps the last row
aligned with the viewport. Editing a vector of keys preserves the top visible
key when it survives. Finding a moved key may scan the vector once per edit.

### Positioning a row

Both list builders accept a `ListScrollHandle`. Keep the handle with the view
that issues navigation commands. For example, a search result can reveal its
zero-based row with the minimum amount of scrolling.

```rust
let list = ListScrollHandle::new();
let results = variable_list(messages, px(48.0), build_message)
    .scroll_handle(&list)
    .flex_1();
div()
    .child(div().child(text("Show match")).on_click(move |_| {
        list.scroll_to(match_index, ScrollAlignment::Nearest);
        Ok(())
    }))
    .child(results)
```

`Top`, `Center` and `Bottom` align the row within the content viewport, clamped
to the document bounds. `Nearest` leaves a fully visible row in place. For a
row taller than the viewport it moves only far enough to fill the viewport.
A variable list refines the destination using the mounted row's actual height.

The most recent request is applied once the list is visible, has items and has
a nonzero content area. The index addresses the order at layout time and an
index past the end selects the last row. Scroll input that moves the list
supersedes a queued request. A handle can be attached to one live list at a time.

### Scroll progress

Use `scroll_to_percent` for a position along the scrollable distance and
`scroll_to_offset_px` for a distance from the top in logical pixels. They share
the deferred request and wheel behavior of `scroll_to`. Finite inputs are clamped
to the scroll bounds. Nonfinite inputs return an error and preserve a pending request.

```rust
list.scroll_to_percent(75.0)?;
```

To seek to a fractional pixel instead, for example during position restoration,
call `list.scroll_to_offset_px(40_000_000.25)?`. Offsets use double precision.

```rust
if let Some(position) = list.position() {
    let percent = position.percent();
    let offset = position.offset_px;
    let limit = position.max_offset_px;
}
```

Readback reflects the last successful view layout, including wheel input, row
navigation, resizes and data changes. A visible list that fits reports zero
scrollable distance and zero percent. Before layout, after unmount, and for a
hidden or zero-sized viewport, the handle returns `None`.

For a progress indicator, create the handle and its position signal together.
Reading the signal in a paint closure subscribes that element to changes.

```rust
let (list, position) = ListScrollHandle::with_position_signal(cx);
let indicator = div().h(px(3.0)).paint(move |cx| {
    if let Some(position) = *cx.read(position)? {
        let mut bar = cx.size;
        bar.width = bar.width * (position.percent() / 100.0) as f32;
        cx.fill(bar, rgb(0x3888dd));
    }
    Ok(())
});
div().flex_col()
    .child(indicator)
    .child(uniform_list(items, px(32.0), build_row).scroll_handle(&list).flex_1())
```

Once list layout settles, a changed position invalidates subscribed drawings
before frame recording. An indicator in the same view follows the list in that
frame. An indicator in another view requests its own redraw. Unmounting the list clears the
signal. Create it in a parent or application context to observe it across mounts.

A uniform list knows the full document height. A variable list sets
`is_estimated` while any rows are unmeasured. A seek uses the available height
information to locate the destination. When measuring those rows changes the
extent, the viewport preserves the destination row and within-row distance.
The reported offset and percentage can consequently change. Zero and 100
percent seek to the document edges.

### Scrollbars

Bind an ordinary overflow container and its vertical scrollbar to a shared
`ScrollHandle`. A stretched flex row gives the track the height of the viewport.

```rust
let scroll = ScrollHandle::new();
div().flex_row().items_stretch().h(px(240.0))
    .child(div().flex_1().overflow_y_scroll().track_scroll(&scroll).children(rows))
    .child(scrollbar(&scroll))
```

`scroll.metrics()` reports the container's offset, limits and viewport in logical
pixels after layout. The bar moves the vertical offset and preserves the horizontal
offset. Each handle tracks one mounted container at a time and can be reused
after unmount.

For a wide table, put a horizontal scrollbar below the content. Both tracks can
share the same handle when the container scrolls on both axes.

```rust
let scroll = ScrollHandle::new();
div().flex_col().items_stretch()
    .child(div().flex_row().flex_1().items_stretch()
        .child(div().flex_1().overflow(Overflow::Auto).track_scroll(&scroll).child(table))
        .child(scrollbar(&scroll)))
    .child(horizontal_scrollbar(&scroll))
```

To show the horizontal midpoint, call
`scroll.scroll_to_percent_on(ScrollAxis::Horizontal, 50.0)?`. A request on one
axis keeps the other axis's pending destination.

For a reading progress indicator, create the handle with
`ScrollHandle::with_metrics_signal(cx)` and attach it to the container as above.
The signal carries both axes, including changes to the viewport and scroll range.

```rust
let (scroll, metrics) = ScrollHandle::with_metrics_signal(cx);
let progress = div().h(px(3.0)).paint(move |cx| {
    if let Some(m) = *cx.read(metrics)? {
        let mut bar = cx.size;
        bar.width = bar.width * (m.vertical_percent() / 100.0) as f32;
        cx.fill(bar, rgb(0x3888dd));
    }
    Ok(())
});
div().flex_col().items_stretch()
    .child(progress)
    .child(div().flex_1().overflow_y_scroll().track_scroll(&scroll).children(rows))
```

Both `horizontal_percent()` and `vertical_percent()` report zero for an axis
whose content fits. Reading the signal during paint subscribes the indicator to
changes. It receives `None` before the first scrollable layout, while the
container is hidden or both scroll axes are disabled, and after unmount.

Use the handle directly from an event handler to jump to a logical-pixel
coordinate or a percentage of the vertical range.

```rust
let jump = scroll.clone();
div().child(text("Jump to latest"))
    .on_click(move |_| jump.scroll_to_percent(100.0))
```

`scroll.scroll_to_offset(Origin { x: px(12.5), y: px(80.0) })` sets both
axes. A percentage request changes only the vertical destination. Requests
can be issued before mount and are resolved against the next scrollable layout.
For example, appending a row and requesting 100 percent in the same handler
reveals the new bottom row. If wheel input arrives before layout, it continues
from the requested position within the current range. Unmounting clears
outstanding requests.

Uniform and variable lists use their own `ListScrollHandle` with the same scrollbar.

```rust
let list = ListScrollHandle::new();
div().flex_row().items_stretch().h(px(320.0))
    .child(uniform_list(items, px(32.0), build_row).scroll_handle(&list).flex_1())
    .child(scrollbar(&list))
```

The thumb represents the visible fraction of the content. Dragging preserves the
grab point, including outside the track. Pressing the track on either side of the
thumb scrolls one viewport along its axis. Wheel input passes the other axis's
delta and any unused distance to the parent. When the content fits along the
track's axis, the track keeps its thickness and the thumb disappears.

By default only a narrow thumb is visible at rest. Hovering the track or
dragging the thumb reveals the rail and widens the thumb inside the same hit
area. Use `scrollbar(&list).auto_hide(true)` to hide the resting thumb as well.

Style the track with the shared element builders. Its padding sets the inset
around the thumb. Thumb fills accept solid colors and gradients.

```rust
scrollbar(&list)
    .w(px(14.0)).p(px(3.0)).hover_track_fill(rgb(0xe8e8e8))
    .thumb_style(ScrollbarThumbStyle {
        fill: rgb(0x60756a).into(),
        hover_fill: rgb(0x485c51).into(),
        drag_fill: rgb(0x34473d).into(),
        radius: px(4.0),
        min_length: px(28.0),
        idle_width: px(3.0),
    })
```

For application-specific rendering, `paint_with` receives the track's paint
context, track axis, scroll position, hover and drag state, and the thumb interval
used for input. The interval starts at `thumb_start` along the track's axis.
For example, an application can reveal a thin indicator while the list is
active and widen it during a drag. Paint-time signal reads subscribe automatically.

```rust
scrollbar(&list).paint_with(move |cx, bar| {
    if !bar.hovered && !bar.dragging && !*cx.read(list_active)? {
        return Ok(());
    }
    let width = cx.size.width.min(px(if bar.dragging { 6.0 } else { 2.0 }));
    cx.fill_rounded_rect(
        Origin { x: cx.size.width - width, y: bar.thumb_start },
        Size { width, height: bar.thumb_length },
        px(3.0),
        *cx.read(accent_color)?,
    )
})
```
