# UI interface design

A settings page and an audio meter can share one retained tree. A device rename
can change wrapping and move the rows below it. A level sample can change the
meter's fill while its bounds and its siblings stay fixed. The contract for
each property must identify who owns it, which descendants inherit it, which
stage consumes it, and how a host learns about the resulting work.

## Implemented boundaries

| Interface | Current behavior |
| --- | --- |
| Composition and ownership | A component renders once per attachment. A conditional owns its branch. Retiring a node releases its subscriptions and local signals. The host owns application signals. |
| Box layout | Lengths in logical pixels, rems and percentage box sizes resolve to floating-point geometry. Every primitive receives the same border-box sizing and per-edge padding. Content layout, painting and child origins use the inner bounds. A container applies the gap on its main axis. |
| Style refinements | Later declarations override individual specified fields. Text size, line height and color propagate into fixed and conditional descendants. Each element resolves its own box and background at mount. |
| Text | A fixed or signal-bound label owns a shared string and caches shaped paragraphs by width. Updating bound text keeps the element ID. A changed label schedules layout and local paint. |
| Fill | A straight-alpha sRGB color has named channels. Every gradient shares an ordered, variable-length stop sequence. Four geometries sample it in sRGB or Oklab. Backgrounds and custom fills carry the same fill value. Equal offsets describe hard edges. Rounded fills use the assigned dimensions. |
| Paint scheduling | Each successful paint subscribes the element to the signals it read. Repeated writes coalesce. Elements default to on-demand recording. A fixed or signal-driven continuous mode samples every host frame. |
| Pointer input | The host accepts a border-box snapshot for hit testing. Positioned events deliver local logical coordinates. A press can capture its pointer until release or cancellation. See [pointer input](pointer-input.md). |
| Platform boundary | `solor` owns the GUI and the platform traits. `solor-linux` owns the Wayland event loop and buffer submission. `solor-render-cpu` rasterizes shaped text. The window example pairs them using the RGBA8 presentation format. |

## Platform and renderer ownership

`Platform::run_window` takes a window handler for the platform's presentation
format. The Linux implementation drives one XDG toplevel until close. Its
handler implements `WindowHandler<Rgba8>` and can borrow a retained image for
one submission. Rendering code depends on the presentation format. Only the
application entry point selects Wayland.

Pointer positions and scroll distances arrive in logical pixels. The Linux
adapter currently delivers mouse buttons, motion, cancellation and vertical
wheel input. Keyboard modifiers are currently reported as empty. An event
handler can request a redraw. The host supplies the logical allocation and
device scale at frame preparation, validates the returned buffer, then
acknowledges submission with `frame_submitted`. A successful empty-damage
frame receives the same acknowledgement. The compositor controls display timing.

## Recording a mounted view

After flushing signals and laying out a view, call `Ui::record_frame(view)`.
The returned `ViewFrame` owns the ordered commands, retains the font data and
shaped paragraphs, and carries the pointer geometry for that layout. Both
painting and hit testing use the same view-local node origins. Each text
command includes its allocated content box for clipping. A failed recording
preserves outstanding repaint requests for retry.

`Surface::draw_view` in the CPU backend replays the frame onto an opaque white
RGBA8 image. Supply the physical target size and its device scale. Gradients
are sampled in the original fill box even when the target clips it. After
successful presentation, pass `frame.into_pointer_frame()` to
`Ui::accept_pointer_frame`. Discarding the frame leaves the accepted input
geometry in place. This path records the whole view and rasterizes a fresh
image on each call.

## Presenting a mounted view

`CpuWindow::mount(ui, component)` owns the UI and mounts the root once. Pass
the resulting handler to `Platform::run_window` on an RGBA8 platform. It
dispatches pointer events against the accepted layout and requests a frame
when signal bindings have work to flush. Each frame records the view and
compares the ordered commands with the previous submission. The window keeps
one RGBA surface and repairs the bounding rectangle of changed commands,
including the space vacated by shrinking or removed content. Unchanged draws
overlapping that area are replayed in paint order. Identical commands produce
empty damage. Changes to the allocation or scale and retries after a discarded
prepared image require full damage. The platform acknowledges successful
submission before the handler publishes its prepared hit geometry. On the next
frame callback the window continues if the submitted frame requested another
frame or signal bindings have pending work. Otherwise it returns to idle.

Run `cargo run -p solor-render-cpu --example mounted_window` in a Linux
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

At attachment, a custom element receives its local layout and appearance
declarations in `Element::prepare_content`. Text attributes cascade from the
parent, including when a conditional child is attached later. An explicit
`clear_line_clamp()` removes an inherited line limit.

A `div()` uses vertical flow by default. Both `.flex_row()` and `.flex_col()`
set its direction through the same style refinement, before or after adding
children. Horizontal flow uses the column gap (`gap_x`), vertical flow uses
the row gap (`gap_y`), and `gap` sets both. Reverse directions start placement
at the far edge while keeping child identity and declaration order.

Horizontal flow measures each child against the container's content width
less its fixed margins. The line can overflow when the combined preferred
widths and gaps exceed its allocation. Automatic horizontal margins share
positive space left over across the entire line. Flex wrapping and proportional
growth or shrinkage are declarations pending layout support.

Vertical flow measures and places child border boxes with their margins.
Adjacent vertical margins add to the row gap. Signed margins can overlap
siblings. Fixed horizontal margins reduce the width available to the child.
One automatic horizontal margin takes the remaining positive space, and two
share it equally. Automatic vertical margins contribute zero. Percentages on
all four edges use the column's content width. Their intrinsic contribution
is zero while that width is indefinite, and placement resolves them against
the assigned width. Root margins do not change the host's viewport allocation.

The background painter uses a
uniform corner radius and draws square corners for mixed radii. Additional
layout, paint, text shaping and input behaviors in this table are declarations
pending engine support. In particular, `.absolute().top(px(4.0)).right(px(8.0))`
expresses placement but does not yet move a child or remove it from column flow.

## Contracts to establish next

| Priority and capability | Required behavior and acceptance case |
| --- | --- |
| 1. Frame composition, clipping and hit testing | Compose parent-local coordinates, scale, clips and sibling order into one accepted frame. Move a rounded card over another target, fail one submission, then retry. Pixels and click targets must agree with the last accepted frame. Old bounds must be repaired when a wrapped label moves its siblings. |
| 2. Sizing, placement and scrolling | Apply min/max sizing, positioning and per-edge borders during measurement and placement. Resolve row and column alignment, stretch, grow/shrink and overflow. Validate a long label between a fixed icon and an action at narrow and wide widths. Scrolling must share the visual clip and hit-test transform. |
| 3. Input and state-dependent styles | Connect platform pointer adapters and define propagation, keyboard focus, tab order and actions. Define precedence for hover, press, focus and disabled states, including removing an override. Validate a pressed element that moves, loses capture or unmounts before release. |
| 4. Text, editing and semantics | Apply font selection and fallback, weight, alignment, wrapping/truncation and decoration. Define editable ranges, composition, caret bounds and accessibility actions. A bilingual field must expose the same selection to keyboard, IME and assistive input. Preserve explicit units when converting platform text ranges. |
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
