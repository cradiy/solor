# UI interface design

A settings page and an audio meter can share one retained tree. A device rename
can change wrapping and move the rows below it. A level sample can change the
meter's fill while its bounds and its siblings stay fixed. The contract for
each property must identify who owns it, which descendants inherit it, which
stage consumes it, and how a host learns about the resulting work.

Run `cargo run -p solor-gallery --release` in a Linux Wayland session to compare
letter spacing, OpenType ligatures, gradient geometry and flex distribution.
From the initial Typography page, Tab or Shift+Tab moves between page and specimen
choices. Enter or Space selects the focused choice and leaves focus on its button.
The Lists tab shows the 24-bit RGB color space in a virtual list. Scroll to
browse swatches, or click and drag the position indicator to seek across all
16,777,216 colors. With focus in the RGB list, Up and Down scroll one row, Page Up
and Page Down scroll one viewport, and Home and End jump to the first and last swatches. A slim thumb sits over the list’s right edge and widens on hover.
Choose **Tone sheet** to browse nine color families down the rows and nine tones
across the columns. With focus in the grid, arrow keys pan one column or row
at a time. Shift+Arrow selects the adjacent swatch and scrolls it into view.
Clicking a swatch also selects it. Its label is highlighted, and the header
shows its color and hex value while browsing. To return to it from the grid,
press Shift+Tab to focus **Show selection**, then Enter or Space.

Run `cargo test --release -p solor-gallery --test navigation tone_sheet_recording_cost -- --ignored --nocapture`
to measure frame recording during two-axis scrolling and repeated stationary
recordings. It reports text commands per frame and median times for preparing
hit-test and clip geometry, recording drawing commands against that geometry,
and the two stages together. A separate median covers the whole iteration
through pointer-frame acceptance.
Rasterization and presentation are outside the measurement.

Run `cargo test --release -p solor-gallery --test navigation tone_sheet_cpu_frame_cost -- --ignored --nocapture`
to time the same two-axis motion through the CPU window at 1× and 1.5× scale.
Each sample covers input dispatch, layout, recording, damaged-pixel repair
and local submission acknowledgement. After 64 warm-up frames, the workload
reports the median, 95th percentile and count exceeding the 120 Hz budget
over 256 frames. Window-system upload and display timing are outside this measurement.

A plain on-demand `div()` with fixed styles, no background and no drawing
callback participates in layout, clipping and input without recording commands
of its own. Its children still paint in order, and its frame listeners can
request host frames.

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
the feed while playing. When the feed toggle has focus, Enter or Space pauses or resumes it. Pausing releases the frame request and keeps the same
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
| Keyboard focus | Tab and Shift+Tab cycle through visible, enabled stops by index within each Tab group, with ties in tree order. A key-down handler can prevent default navigation for an individual press. |
| Platform boundary | `solor` owns the GUI and the platform traits. `solor-linux` owns the Wayland event loop and buffer submission. `solor-render-cpu` rasterizes shaped text. `solor-platform` selects the window system and its rendering backend. |

## Platform and renderer ownership

Applications obtain a host with `solor_platform::current_platform()?` and
call `platform.run_window(options, ui, root)`. The host mounts the root and
owns the pairing of the window system with a compatible renderer. The same
application entry point compiles on other targets. Platform selection returns
an unsupported-platform error where a host is unavailable.

For multiple ordinary windows, call `platform.run(|app| { ... })`. Use
`app.open_window(options, ui, root)?` during setup or capture `app.clone()`
in a UI callback to open another window. The call mounts the view and reports
creation errors to its caller. The new window receives events after the
callback returns. Each window owns its UI, focus and redraw schedule.
Closing the last window ends the event loop, even if an application handle
is still held. The [multiple windows example](../crates/solor-platform/examples/multiple_windows.rs)
opens additional notes from a button in any existing note.

Keep the `WindowHandle` returned by `app.open_window(...)?` to manage a
specific window. Use `window.is_open()` to reuse a settings window, and
`window.close()` to close it from another window's callback. A close request
immediately changes the status observed by every clone. Reopening creates a
distinct handle. Dropping a handle leaves its window open.

Pass `WindowOptions::new("Inspector").parent(&main)` to associate an inspector
window with its main window. The parent must be an open ordinary window in the
same application run. To capture the main window's own handle in a button
callback, build its root with `app.open_window_with(options, ui, |main| ...)`.
Both windows can also open during setup. On Wayland, the child waits for its
parent's first buffer before requesting the native relationship. Each window
continues to accept input and can close independently.

Use `WindowOptions::new("Discard changes?").modal_for(&editor)` for a dialog
that blocks its parent. Pointer and keyboard input resume when the last modal
child closes. The parent keeps rendering and other windows remain interactive.
Closing the parent through its handle closes its modal descendants. On Wayland
the host filters input and native close requests to blocked windows and sends
the compositor a modal hint when `xdg-dialog-v1` is available. In the
[settings window example](../crates/solor-platform/examples/settings_window.rs),
**Open settings** blocks the workspace until the settings dialog closes.

Call `window.activate()` when the user selects an already open window.
For a blocked window, activation follows the most recently opened modal child
at each level. If that dialog closes, subsequent requests follow the remaining
open dialogs. When the last one closes, activation targets the window itself.
On Wayland the host requests activation through `xdg_activation_v1` when
available. Requests from pointer button and keyboard callbacks identify the
input event and the requesting surface. The compositor decides whether to
transfer focus. A closed handle ignores activation requests.

`WindowOptions::new("Settings")` requests an 800 × 600 logical-pixel window.
Use `.size(px(480.0), px(320.0))` for a different preference and
`.app_id("org.example.settings")` to identify the application to the window system.

Use `.min_size(width, height)` and `.max_size(width, height)` to bound the
preferred size. All three sizes use logical pixels. For example,
`WindowOptions::new("Player").size(px(480.0), px(270.0)).resizable(false)`
requests a fixed-size window. The standard arrow appears over its resize
grips, and explicit cursor styles still apply. Its custom resize and maximize
controls issue no size changes. On Wayland, minima round up and maxima round down to whole
logical pixels. Opening fails when the range contains no usable size. The
compositor can still assign a different size, for example when tiling.

Use `WindowOptions::new("Player").always_on_top()` to request that an ordinary
window stay above other ordinary windows. Decorations, application identity
and interactive movement use the usual toplevel options. The Wayland host
sends this request when the compositor advertises version 2 of
`treeland_dde_shell_manager_v2`. Other compositors open the ordinary window at its
usual level. The compositor and the user retain control of the effective
stacking order.

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

On Linux, `run_window` opens an XDG toplevel. Use
`current_platform()?.run_layer_surface(options, ui, root)` for a panel,
wallpaper or overlay. Both entry points mount the same types of components and
run until the surface closes. The driver borrows the CPU renderer's output for
one submission.

Run `cargo run -p solor-platform --example layer_panel` to open a 300 × 36
logical-pixel panel centered on the top edge. Set the panel's dimensions in
`LayerShellOptions`. Its root element receives the configured surface size.
`LayerShellOptions::new(namespace, layer)` spans the compositor-selected output
and leaves keyboard focus with the active application. Set a fixed axis with
`Some(px(...))` or leave it as `None` with both opposite edges anchored.
For instance, a top bar can stretch horizontally and use the same logical
height for its fixed height and its reserved space. A corner overlay can set
both dimensions and leave the exclusive zone at its default. Layer surfaces
require `zwlr_layer_shell_v1`; on-demand keyboard focus requires version 4.

To open a draggable panel above ordinary windows, select `Layer::Top`, set
`movable: true` and supply a fixed width and height. Mark the draggable
element with `.window_drag_region()`. Anchors and margins set its starting
position. The panel moves within the available area on one output and
cannot reserve desktop space. Input regions remain panel-local after a move.
Try `cargo run -p solor-platform --example floating_panel` and drag the
label or the space beside it. The Close button handles its own clicks.

For a translucent window or layer panel, call `.transparent()` on its creation
options. Unpainted areas reveal the desktop, and fills, glyphs and antialiased
edges retain per-pixel alpha. The default CPU canvas is opaque white. Configure
the input region separately when clicks should pass through.

To start hidden, create a `LayerSurfaceHandle::new(false)?` and pass
`handle: Some(&handle)` in the layer options. A worker can keep a clone and
call `set_visible(has_candidates)?` as candidates change. The event loop
wakes and applies the latest request. Hiding releases the occupied desktop
space and suspends frame requests. Showing restores the placement and renders
the mounted view after the compositor configures it. Use `close()?` to end
the run even while the surface is hidden.

Use the same handle to set the input region before showing the panel or while
it is visible. Candidate lists with a separate picker can provide both bounds
in surface-local logical pixels. The compositor passes pointer and touch input
outside those rectangles to the surface underneath.

```rust
use solor::px;
use solor_platform::layer_shell::InputRect;

handle.set_input_region(Some(&[
    InputRect::new(px(8.0), px(4.0), px(240.0), px(32.0)),
    InputRect::new(px(8.0), px(40.0), px(144.0), px(24.0)),
]))?;
```

An empty slice makes the whole panel click-through. Passing `None` restores
the default full-surface region. Region changes on an idle panel take effect
without requesting a redraw. They are independent of the keyboard focus policy.

Pointer positions and scroll distances arrive in logical pixels. The Linux
adapter currently delivers mouse buttons, motion, cancellation and two-axis
scroll input. Key presses and releases use the seat’s XKB layout. Key and
pointer samples carry the seat’s effective Shift, Control, Alt and Super modifiers. An event
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

Inside the Linux host, `CpuWindow` owns the UI and
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

Explicit width and height include border and padding by default. With
`.box_sizing(BoxSizing::ContentBox)`, they describe the content area, and
border and padding add to the preferred outer size. Percentages use the parent's
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

Use `.text_color(ink)` with an `Rgba`, a `Gradient`, or a signal of either type
on a label or an ancestor. A `Signal<Ink>` can switch between the two.
Descendants inherit the foreground until they supply their own. A gradient
spans the paragraph's content box across all lines. Custom elements read the
resolved `Ink` with `cx.text_color()?` during paint. A color change
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

Use `.underline()` or `.underline_wavy()` beneath text. Either can be combined
with `.line_through()`. Bind `.text_underline(signal)` to an `UnderlineStyle`
to switch between a straight line, a wave and no underline. Each
visible line is decorated across its shaped advances inside the text content
box. The face supplies the baseline offsets and natural thickness, with
em-based defaults for missing metrics. Strokes follow the text foreground.
Use `.decoration_color(color)` and `.decoration_thickness(px(1.5))` for both
strokes. Set either stroke independently with `.underline_color(...)`,
`.underline_thickness(...)`, `.strikethrough_color(...)` or
`.strikethrough_thickness(...)`. Highlight and stroke paints accept solid colors,
gradients and signals. Each gradient spans the full decorated line before clipping.
Thickness also accepts rem units. Children can disable an inherited
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

Use `.absolute().top(px(4.0)).right(px(8.0))` to anchor a child inside the
parent's padding box. It contributes to scrollable overflow and keeps its
sibling paint order. Flow sizing and gaps use only in-flow children.
Percent insets use the parent's width or height on the corresponding axis.
Opposing insets stretch an automatic dimension. A definite dimension keeps
its size, with the start edge taking precedence. An axis with both insets
automatic starts at its leading margin.

Use `.hidden()` or `.display(Display::None)` to remove a mounted subtree from
layout, drawing and focus navigation. Siblings close the gap. Restoring its
display through a signal reuses the same nodes and component state.

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
independently on each axis. The element's own background inherits its ancestor clip.
View recordings omit plain, fixed text leaves whose border and text boxes are
both completely outside an ancestor clip. Partially visible labels retain their
full text geometry and effective clip. Local
recordings from `Ui::paint` use the element's own coordinate system and apply
its own overflow setting. The CPU backend rounds clip edges to device pixels.

Use `.overflow_y_scroll().scrollbar_width(px(8.0))` to reserve eight logical
pixels on the right. Horizontal scrolling reserves space at the bottom. The
gutter stays in place even when the content fits. Children lay out, paint and
receive pointer input inside the reduced viewport. The default width is zero.
`Overflow::Auto` uses overlay scrolling.

A host can read a laid-out overflow container with `ui.scroll_metrics(view, node)`.
The snapshot contains its current two-axis offset, maximum scroll distance and
viewport size in logical pixels. To restore a saved position, call
`ui.set_scroll_offset(view, node, saved_offset)?` after layout. The setter clamps
each axis to the current bounds and requests a redraw when the offset changes.
Readback reflects the new offset immediately. Pointer hits switch to the new
geometry when the host accepts the frame.

`.opacity(0.5)` or `.opacity(signal)` applies to the element and all its
descendants as one group. On the CPU backend the group is drawn against
transparent black, then blended once with the pixels behind it. Nested groups
compose from the inside out. Finite values are clamped to 0–1 and rounded to
8-bit alpha. An opacity of zero preserves layout and hit testing.

A border is drawn inside the border box, above its background. Set its
width and paint together, for example `.border_b(px(1.0)).border_bottom_color(ink)`.
Each edge accepts its own width and fill. Gradients span the whole box. Border
widths reserve space outside the padding, including edges without paint.
Use `.border_dashed()` or `.border_dotted()` for the whole box, or set a
single edge with `.border_bottom_style(BorderStyle::Dotted)`. Dash length
and dot diameter scale with the edge width.

Use `.outline_width(px(2.0)).outline_color(ink)` to draw a ring outside the
border without changing layout or pointer bounds. For example,
`.outline_offset(px(2.0))` leaves a two-pixel gap. A negative offset moves
the ring inward. The ring follows the box corners and is clipped by ancestors.
Use `.outline_style(BorderStyle::Dashed)` for a dashed ring.

Use `.shadow([BoxShadow::new(px(0.), px(2.), ink).blur(px(6.))])` for a soft
outer shadow. Layers are declared front to back. Outer shadows follow the box
corners and stay outside the border box, including on transparent elements.
Use `.inset()` for a shadow above the background and beneath the border.
Offsets, blur and spread accept pixels or rems.

Each background corner has an independent circular radius. If two adjacent
radii exceed their shared edge, the frame scales all four by the same factor
until they fit. The CPU renderer applies this shape to solid and gradient
backgrounds and borders. Overflow clipping and pointer hit boxes remain
rectangular. Additional layout, paint, text shaping and input behaviors in
this table are declarations pending engine support.

## Contracts to establish next

| Priority and capability | Required behavior and acceptance case |
| --- | --- |
| 1. Frame composition, clipping and hit testing | Compose parent-local coordinates, scale, clips and sibling order into one accepted frame. Move a rounded card over another target, fail one submission, then retry. Pixels and click targets must agree with the last accepted frame. Old bounds must be repaired when a wrapped label moves its siblings. |
| 2. Sizing, placement and scrolling | Apply positioning and per-edge borders during measurement and placement. Resolve scrolling and overflow. Validate a long label between a fixed icon and an action at narrow and wide widths. Scrolling must share the visual clip and hit-test transform. |
| 3. Input and state-dependent styles | Connect platform pointer adapters and define propagation, custom tab ordering and actions. Define precedence for hover, press, focus and disabled states, including removing an override. Validate a pressed element that moves, loses capture or unmounts before release. |
| 4. Text, editing and semantics | Apply font selection and fallback, weight and richer inline styles. Define editable ranges, composition, caret bounds and accessibility actions. A bilingual field must expose the same selection to keyboard, IME and assistive input. Preserve explicit units when converting platform text ranges. |
| 5. Paint and animation | Define border and shadow paint order, transformed and rounded clipping, and animation timing. A frame-clock repaint must stop on unmount. CPU replay must draw every gradient stop, preserve hard edges and match the declared interpolation. A GPU representation needs equivalent output and a resource lifetime through submission. |
| 6. Large collections and resources | Define keyed identity, viewport materialization, scroll anchoring, image/font resource ownership and cancellation. Replace and reorder items while keeping focus on a surviving key. Late resource completion must not invalidate a recycled node. |

Linux is the first presentation target. Geometry, semantic actions, text ranges
and frame requests need platform-independent meanings for Web, macOS, Windows,
Android and iOS. Native handles, buffer formats and backend resource indices
belong in adapters. Vector paths, custom shaders and 3D
need concrete workloads before their public signatures are chosen.


## Image fills, image elements and SVG

Decode a bitmap once with `ImageAsset::from_bytes`, or supply straight RGBA8
pixels with `ImageAsset::from_rgba8`. Clones share the decoded pixels. The
`solor` crate enables `png` and `jpeg` by default. Add `webp`, `gif`, `bmp`,
`ico`, or `tiff` individually, or enable `image-full` for all seven formats. Use
`default-features = false` to select codecs individually. Raw RGBA input works
without any codec. Each asset contains one still image.

```rust
let photo = ImageFile::new(absolute_path);
let thumbnail = image(photo.clone()).h(px(48.0));
let card = div().size(px(160.0)).rounded_lg().bg(photo);
```

For `svg` and `image`, a string is a resource name, including strings with
a leading slash. Pass an absolute `&Path` or `PathBuf` for a filesystem
lookup. Both elements also accept encoded byte slices and owned byte vectors.

With the optional `network` feature, `image` also accepts a `solor::Url`.
For example, an application can reserve a square for a cover supplied by its server.

```rust
let cover = Url::parse("https://example.org/covers/album.jpg")?;
let artwork = image(cover).size(px(96.0));
```

Attaching the element starts an HTTP or HTTPS request. Download and decode run
on the shared blocking workers using the enabled image codecs. The intrinsic
size starts at zero. When pixels arrive the element remeasures and repaints.
Mounted consumers of the same URL in one UI share the request and decoded
pixels. Removing the last consumer releases the cache entry and cancels
pending work. A worker checks for cancellation between body reads and before
decoding. A read already in progress can last until the request timeout (30 s).

The encoded size limit is 64 MiB and the decode budget is 128 MiB. A failed
request leaves its element empty. Mounting another element with the same URL
can retry the request.

Implement `AssetSource::svg` to supply SVG bytes and `AssetSource::image` to
supply a path, encoded bytes, a custom image, or a URL with the `network` feature. Register a pack for the
application with `current_platform()?.with_assets((Theme, LibraryIcons))`.
A tuple tries its sources from left to right. A missing name advances to the
next source. A match or error ends the search. Tuples can include other tuples.
A standalone `Ui` accepts the same packs through `with_assets`.

Registration stores the providers. Attaching a named element requests its
resource. Windows on the same configured host share the parsed tree of a live
named SVG. A file-backed bitmap probes dimensions for element layout and
decodes synchronously on visible CPU paint. Each window caches up to 64 MiB of
decoded file images by path. File contents should remain stable while in use.
The current whole-image decoder accepts regular files up to 64 MiB and checks
pixel storage against a 128 MiB decode budget. Encoded bytes supplied directly
or returned by a pack are decoded at element attachment.

For an application-owned format, tile pyramid, or generated image, implement
`ImageRenderer` and pass `CustomImage::new(renderer)?` to `image` or `.bg()`.
`dimensions()` supplies the full image extent in source pixels. During CPU
rasterization, `render(region)` receives the output dimensions in device
pixels and the mapping to the source. The requested rectangle covers the
intersection of the fitted image, clip, and damage bounds. Shape coverage,
rounded corners and compositing are applied by the backend.

Output pixel `(x, y)` samples the source at
`source_origin + ([x, y] + 0.5) * source_step`, component by component.
Clamp boundary samples to the image edges. Return an `ImageAsset` the size of
`region.output`. The application controls I/O, resolution level, sampling, and
tile retention. This path works with all file codecs disabled. Each handle
represents one immutable snapshot. For background decoding, render the pixels
currently available and publish a new handle to `Signal<CustomImage>` when
tiles arrive. To publish a tile, use
`let next = current.updated_region(source_rect, renderer)?;` and set the
signal to `next`. When several tiles arrive together, pass their rectangles to
`current.updated_regions(source_rects, renderer)?` and set the signal once.
The rectangles use half-open intrinsic pixel coordinates and must include the
full influence of any filtering. CPU image elements and backgrounds project
each region through fit, alignment, clipping and device scale. They can repair
distant tiles separately when the window still displays the direct predecessor
with the same geometry. A skipped snapshot or a layout change triggers the usual redraw.
The regional callback runs on the rendering thread.

Run `cargo run -p solor-platform --example image --features solor/image-full`
to compare cover and contain placement with a transparent WebP.

A standalone image uses its intrinsic dimensions and defaults to centered
`ObjectFit::Contain`. A bitmap background covers the assigned box, including
padding, and follows its corner radii. Both accept `object_fit`,
`object_position_x` and `object_position_y`. Both `image(signal)` and `.bg(signal)` accept
`Signal<ImageAsset>` or `Signal<CustomImage>`. Replacing a background invalidates
paint. An image leaf
remeasures when the source dimensions change. Recorded frames retain their
own image, and the CPU renderer filters premultiplied pixels before compositing.

Parse self-contained vector artwork once and clone the asset to share it between
elements. Pass a `Signal<SvgAsset>` to `svg` to switch artwork on the mounted
element. A change in intrinsic size also updates layout. By default, `svg(asset)`
uses the inherited foreground across its fitted viewport. Use
`.preserve_colors()` to draw the original fills, strokes and gradients.

```rust
let mark = SvgAsset::from_bytes(include_bytes!("mark.svg"))?;
let row = div().flex_row()
    .child(svg(mark.clone()).size(px(20.0)).text_color(ink))
    .child(svg(mark).h(px(48.0)).preserve_colors());
```

The SVG viewport supplies the intrinsic size and aspect ratio. By default the
artwork is centered and contained within the padded content box. Use
`.object_fit(...)` to change the fit and `.object_position_x(...)` or
`.object_position_y(...)` to change alignment. Position percentages apply to
the space remaining after fitting, including negative space when cropping.

The CPU renderer rasterizes at the device size. Each window caches up to
128 rasters within an 8 MiB pixel budget. Single-color entries store one alpha
byte per pixel and can be reused after a foreground change. Color entries
store premultiplied RGBA. A cache miss uses a temporary RGBA surface in both
modes. Use vector outlines for lettering. SVG text, embedded bitmap images,
and external image files are outside the supported asset subset.

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
outlive the row in application state. By default a uniform list preserves the
logical scroll distance, clamped to the new extent.

Use `.anchor_to_item()` on a keyed uniform list to keep the first visible item
at its current within-row offset during prepends, removals or reordering.
This suits a fixed-height log that loads earlier entries above the reader.
If the key disappears, the row at its former index moves to the top, within
the scroll bounds. Explicit scroll requests take precedence. Variable-height
lists already anchor by item.

### Editing individual items

Create a `SignalVec<T>` when rows have independently changing values. Both
virtual list builders pass the item's stable signal to the row builder.

```rust
let rows = cx.signal_vec([SharedString::from("Ready"), SharedString::from("Queued")]);
let list = uniform_list(rows, px(28.0), |status| text(status));

rows.set(1, SharedString::from("Running"), cx)?;
rows.move_item(1, 0, cx)?;
```

A changed value notifies that item's subscribers. `update_in_place` supports
non-cloneable values and publishes after every edit. For a custom struct the
subscription covers the whole item. Updates that affect row height can still
reflow the viewport. `insert`, `push`, `remove` and `move_item` publish the
order. Use `rows.item(index, cx)?` to keep addressing an item after it moves.

`rows.splice(start..end, values, cx)?` replaces a contiguous range and
publishes the order once. Use `i..i` to insert a batch at index `i`, or pass
an empty iterator to delete the range. Splicing an empty iterator into an
empty range leaves subscribers idle. Each replacement gets a fresh signal,
even when its value equals the previous value. Items outside the range keep
their identity. Use `set` or `update_in_place` to edit an existing row and
preserve its local state.

Each stored item occupies one runtime signal, including items outside the
mounted window. Removing an item releases its value after its last binding
detaches. The collection and later insertions belong to the scope that
created the collection.

When loading pages into a list with a known maximum size, use
`cx.signal_vec_with_capacity(initial_rows, expected_total)?` to allocate
space for the item handles at construction. The capacity includes the initial
rows. Each later row gets its signal when inserted. The list grows normally
when it exceeds the estimate, and prepending still shifts the existing handles.

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
key when it survives. For a single splice above the top row, the list locates
its key directly from the change in item count. Reordering keys or replacing
the top row may require searching the sequence. Inserting or deleting entries
in a `SignalVec` can also shift item handles after the edited range.

Run `cargo bench -p solor --bench list_splice --offline` to measure edits at
the front and near the end of a variable-height list. The history workload
records the first 128-row prepend separately, then prepends 100 more pages
while checking that the visible nodes and reading position stay stable.
It compares default and preallocated handle storage and reports collection
construction separately from the first edit.
Timings separate collection edits from flushing bindings, laying out the
viewport and recording the offscreen frame. Rasterization and presentation
are outside the measured interval.

### Positioning a row

Use `.follow_tail()` on a uniform or variable-height list for a chat transcript
or live log. The list opens at the bottom and follows appended rows while the
reader stays there. Scrolling up or jumping to an earlier row pauses following.
Scrolling back to the bottom resumes it. A scroll handle is optional.

```rust
variable_list(messages, px(32.0), |message| text(message))
    .follow_tail()
    .flex_1()
```

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

To restore a reading position across mounts, save the position before leaving
the list and pass its row index and within-row offset to the next list:

```rust
let saved = list.position();
// When opening the list again:
if let Some(saved) = saved.filter(|p| p.visible_count > 0) {
    list.scroll_to_row_offset(saved.first_visible_index, saved.offset_in_row_px)?;
}
```

For data that can be reordered while the list is closed, save the item's key
with its within-row offset. For a `SignalVec`, capture the visible item's handle
from the same ordering that produced the snapshot, before editing the list.
When reopening, resolve the handle against the current collection:

```rust
if let Some(index) = messages.index_of(saved_item, cx)? {
    list.scroll_to_row_offset(index, saved_offset_px)?;
}
```

The lookup scans the item handles and returns `None` for a removed or replaced
item. The collection must outlive the view to reuse a signal handle across mounts.
If the order changes again before the requested layout, resolve the item again.
A variable list refines the requested position after measuring that row. If
the saved point is outside its new height, the list aligns the row's top within
the scroll bounds.

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

Use `list.scroll_by_px(distance, cx)?` in an input handler for relative
movement. Its return value is the signed distance consumed. Repeated inputs
accumulate before layout publishes the updated position. To page down, use the
current position’s `viewport_height` as the distance.

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

`position.visible_range()` returns the zero-based row indices intersecting the
list's own viewport. For example, `4..9` includes rows 4 through 8. A partially
visible row counts, while a row touching only the viewport edge is excluded.
The range is `0..0` for an empty list. Use its start index to decide when to
load earlier messages. The position signal also publishes range changes caused
by remeasuring rows at the same scroll offset.

Attach `.on_visible_range_change(...)` to run application logic after a list's
layout settles. The handler receives the initial range and subsequent changes
without requiring a scroll handle. A visible empty list reports `0..0`. A hidden
or zero-sized viewport pauses delivery until it becomes visible again.

```rust
let mut older_page = Some(cached_messages);
variable_list(messages, px(32.0), |message| text(message))
    .on_visible_range_change(move |visible, cx| {
        if visible.start < 5 && let Some(page) = older_page.take() {
            messages.splice(0..0, page, cx)?;
        }
        Ok(())
    })
```

The next flush processes the insertion. If the visible indices change as a
result, the handler receives that new range after layout. Keep pagination
cursors and outstanding requests in application state.

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

Use `scroll.scroll_by(ScrollDelta { x: px(12.5), y: px(-8.0) })?` to pan a
laid-out viewport. Each axis continues from its queued destination. The returned
delta reports the signed distance within the current bounds on each axis.

Use `scroll.scroll_to_reveal(origin, size)?` to show a rectangle identified in
the container's unscrolled logical coordinates. Each axis moves the smallest
distance that fits the target inside the viewport. An oversized target covers
the viewport on that axis and keeps the current offset if it already covers it.
The scroll range limits travel for targets outside the content.

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
