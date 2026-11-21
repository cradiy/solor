# Pointer input

A button needs one activation. A seek bar needs the pointer's position while
it moves, including outside the track. Both can update the same application
signals through their event context. Platform adapters deliver normalized
input with `Ui::dispatch_pointer`. A host can also activate a known node
directly with `Ui::click(view, target)`.

Each view starts with no focused element. Mark a control with `.focusable()`
to give it logical focus on a primary press. The nearest focusable element in
the hit path wins, so a label can forward focus to its control and a nested
focusable control can take focus itself. Click handlers are selected independently.
A primary press on the background clears focus. Secondary presses and presses
redirected by pointer capture preserve it.

Hosts can call `Ui::focus(view, node)`, `Ui::focused(view)` and `Ui::blur(view)`.
Only a visible, mounted, focusable element in the same view can receive focus.
Hiding its subtree or removing its mount clears focus during reconciliation.
Showing it again starts it unfocused.

Tab moves forward and Shift+Tab moves backward among visible, mounted
`.focusable()` elements. Traversal visits a parent before its children and
siblings in their current tree order. It wraps within the view. With no current
focus, Tab starts at the first stop and Shift+Tab at the last. Virtual lists
participate through their currently mounted rows, including overscan.
Hosts can use `ui.focus_next(view)` and `ui.focus_previous(view)` directly.

Use `.focus_indicator(|paint| ...)` to make an element focusable and draw its
focus cue. Tab traversal and explicit `Ui::focus` show the cue. A primary
press that assigns focus hides it, including a press on the same control.
Hover and pointer motion preserve the cue. The painter runs above the current
background and below the element's content and children, with its origin at
the border box. Its signal reads subscribe the element to repaint while the
indicator is visible.

```rust
div().focus_indicator(|paint| {
    paint.fill_rounded_rect(Origin::default(), paint.size, px(6.0), rgb(0x31558a))
})
```

Bind application commands once on the UI and handle them on the element that
owns the operation. For example, an editor can bind Ctrl+S to its save action.

```rust
use solor::keyboard::{Key, KeyBinding, Modifier};

struct Save;
ui.bind_keys([KeyBinding::new(Key::S.with(Modifier::Ctrl), Save)]);

div().focusable().on_action(move |_: &Save, cx| save_document(cx))
```

A shortcut matches the logical key and exact modifier set on key-down. ASCII
letters match either case. Shift must be bound explicitly when held. Actions
can carry data. Dispatch starts at the focused element and walks towards the
view root until a handler accepts the action type. At the same element, the
most recently bound matching action wins.

Use `.on_key_down(...)` and `.on_key_up(...)` on the focused element to handle
individual transitions. The key-down handler runs first and can call
`cx.prevent_default()` to consume the press. Tab and Shift+Tab move focus when
the press remains unconsumed. Wayland resolves keys through the active layout.

Custom window hosts forward `WindowEvent::Key(event)` through
`ui.dispatch_key(view, &event)`. The CPU window does this automatically.

## Authoring

Buttons use the context-only click callback. Controls that need coordinates use `on_pointer_down`, `on_pointer_move`, `on_pointer_up`
on the common element builder. They accept `|event, cx| -> Result<()>`.
`on_pointer_cancel` receives the pointer ID. The context provides the same state operations
as a click handler. During a down callback it can also capture the current
pointer for the receiving element.

For a control drawn with `div().paint(...)`, use `event.content_position()`
and `event.content_size()` to map input to its drawing area. Both use the padding
from the accepted frame. Positions are unclamped, so a drag can cross either
end of a track. An axis has zero content size when padding consumes the full
extent. `local_position()` and `bounds()` measure from the border box.

```rust
let dragging = cx.signal(None);
div()
    .on_pointer_down(move |event, cx| {
        if event.is_primary_action() && cx.read(dragging)?.is_none() {
            cx.capture_pointer();
            cx.set(dragging, Some(event.pointer_id()))?;
        }
        Ok(())
    })
    .on_pointer_move(move |event, cx| {
        let width = event.content_size().width.get();
        if *cx.read(dragging)? == Some(event.pointer_id()) && width > 0.0 {
            let fraction = (event.content_position().x.get() / width).clamp(0.0, 1.0);
            cx.set(progress, fraction)?;
        }
        Ok(())
    })
    .on_pointer_up(move |event, cx| {
        if event.is_primary_action() && *cx.read(dragging)? == Some(event.pointer_id()) {
            cx.set(dragging, None)?;
        }
        Ok(())
    })
    .on_pointer_cancel(move |id, cx| {
        if *cx.read(dragging)? == Some(id) {
            cx.set(dragging, None)?;
        }
        Ok(())
    })
```

Use `.overflow_y_scroll()` for a vertical list, `.overflow_x_scroll()` for a
horizontal strip, or `.overflow_scroll()` for both axes. Give the container a
bounded size. Its children scroll in logical pixels until their content edge
reaches the viewport edge. Any remaining distance on either axis goes to the
next scrollable ancestor or scroll listener. A resize or a shorter list clamps
the offset to the new range. Clicks and hover follow the submitted frame.

Use `.on_scroll(|event, cx| ...)` to override a container's wheel behavior or
to handle wheel or trackpad
input. `event.delta.x` and `event.delta.y` are signed logical pixels, positive
right and down. `event.pointer.local_position()` gives the position inside the
listening element. A handler consumes the event by default. Call `cx.propagate()`
to forward the entire delta. Use `cx.propagate_remaining(remainder)?` when a
control handles one axis of a diagonal gesture or reaches a limit partway
through a scroll. Each component of the remainder must lie between zero and the
received value on that axis. A zero remainder ends delivery.

After the handler succeeds, the next listening ancestor receives the forwarded
distance in `event.delta`, with coordinates relative to its own accepted box.
On Wayland, axes reported in one pointer frame arrive together.

Use `.on_hover(|inside, cx| ...)` on any element to react to mouse presence.
Entering a child keeps its ancestors hovered. The first mouse to enter a
subtree triggers `true`. The last one to leave triggers `false`. Motion within
the subtree is silent. A stationary mouse is hit-tested again when the host
accepts changed geometry. Capture controls drag delivery independently.

```rust
let fill = cx.signal(Fill::from(rgb(0x526058)));
div().bg(fill).on_hover(move |inside, cx| {
    cx.set(fill, Fill::from(if inside { rgb(0x687970) } else { rgb(0x526058) }))
})
```

For a crosshair or sample readout, use `.on_hover_position(|hover, cx| ...)`.
While a mouse is over the subtree, `hover` contains its position relative to
the content origin and the content size in logical pixels. The callback runs
on entry and when either value changes, including after a resize with the
mouse at rest. With several mice over the same subtree, the most recently
sampled one supplies the position. Leaving reveals the remaining mouse or
delivers `None` when the subtree is empty.

### Text tooltips

Attach `.tooltip("Show or hide the native window frame")` to a control in a
text-enabled UI. The deepest hint on the hovered branch appears after its configured delay (500 ms by default).
The text is shaped on demand and drawn in a passive overlay within the viewport.
A press, scroll or exit dismisses the hint. Removing its owner cancels it.
Hosts use `WindowHandler::next_redraw()` to wake once at the deadline.
Ordinary string hints inherit `Ui::tooltip_style()`. The default uses a neutral
charcoal background, white text at 14 logical pixels on a 20-pixel line,
8 pixels of horizontal padding, 4 pixels of vertical padding, a 4-pixel corner
radius, and a maximum box width of 280 pixels. It prefers the owner's lower
left edge with a 4-pixel gap. Placement flips and clamps to the viewport.

`Ui::set_tooltip_style` applies to pending, visible and future hints in that UI.
A pending hint measures its delay from the original entry time. Updating a
visible hint's style schedules a repaint. Each `Tooltip` can override individual
fields and continue to inherit the rest.

```rust
ui.set_tooltip_style(TooltipStyle {
    background: rgb(0x252e29).into(),
    text_color: rgb(0xf8f7f2),
    ..TooltipStyle::default()
})?;

div().tooltip(Tooltip::new("Close this window").placement(TooltipPlacement::Above))
```

The adapter supplies a view-scoped pointer ID, device kind, modifiers and
view-local position in floating-point logical pixels. Down and up carry the
changed button. The primary action is the primary mouse button, touch contact
or pen tip. A cancel needs only the pointer ID. For positioned events the
router derives both border-box and content-box coordinates from the same
accepted frame. Captured coordinates may fall outside the box and its ancestor clips.

## Host boundary

After flushing and laying out a view, call `ui.prepare_pointer_frame(view)?`
alongside its paint recording. Keep the returned snapshot with that submission.
On successful presentation call `ui.accept_pointer_frame(frame)?`. Drop a
failed submission's snapshot. Pointer input before the first acceptance has no target.
If the newly accepted snapshot omits a live press owner, acceptance cancels
that sequence. A cancel callback error leaves the new snapshot in effect.

Recordings of the same mounted tree share its immutable hit geometry. Changes
to scroll offsets, layout geometry, tree structure or input styles invalidate
the snapshot. The next recording resolves each viewport's clip against its
ancestors. Descendants that inherit the same clip share it within that snapshot.
Accepting a submission selects its own snapshot for input, even if the view
has already prepared a different layout.

Hit testing visits axis-aligned border boxes in reverse paint order within the
view rectangle. Higher sibling `z_index` values are in front. For ties the
later sibling is in front. Children stay within their parent's layer.
An ancestor with `Hidden`, `Scroll` or `Auto` overflow limits descendant hits on the specified axes,
including the leading edge and excluding the trailing edge. A newly sized clip
takes effect for input when the host accepts that frame.
An invisible subtree leaves the visible boxes behind it eligible for hits.
`PointerEvents::None` skips that box as a target while its children remain
eligible. Captured motion looks up its owner directly.
The host supplies `Leave` on surface exit, `Cancel` on loss of one pointer,
and calls `cancel_pointers(view)` on deactivation.

## System cursors

Set `.cursor_pointer()` on a button, `.cursor_text()` on a text selection area,
or `.cursor(Cursor::Crosshair)` on a plot. The nearest explicit cursor on the
frontmost hit branch applies to each mouse independently. A resize region
supplies the cursor for its edge when it receives pointer input and the branch
has no explicit style. A child with its own click action gets its own cursor.
Mouse capture redirects event delivery while the cursor follows the physical hit.

`WindowHandler::pointer_cursor(id)` reads the resolved cursor after an input
event or an accepted geometry change. A stationary mouse follows the newly
accepted layout. Changing only the cursor requires no application redraw.
On Wayland, compositors advertising `wp_cursor_shape_manager_v1` supply the
system shapes. Without the extension the host leaves the cursor unchanged.

## Routing and lifetime

| Event | Behavior |
| --- | --- |
| Down | Hit-test the host's accepted frame. Resolve the frontmost hit to its nearest live interactive ancestor within the view, including the hit node itself. Keep one recipient for the primary press so a text label inside a button activates the button. A stale frontmost hit consumes the hit without targeting an element underneath. |
| Scroll | Hit-test the accepted frame once. Start with the nearest listener or scrollable container. Route the remaining distance up the same branch. A child button can receive clicks while its parent receives scroll. Capture affects the press sequence independently. |
| Move | Route to the current hit unless this pointer is captured. Capture routes moves to its owner and suppresses click recognition for that press. For an uncaptured press, leaving its target cancels click eligibility through release. |
| Up | Deliver the raw up first, to the capture owner or the current hit. After a successful callback, a matching primary release over the original, still-eligible target invokes its click handler once. The release of the initiating button ends capture even on callback failure. An unmatched up cannot activate a target. |
| Cancel | Device loss, host cancellation or view deactivation clears the sequence. Notify its live owner once. A late up after cancel has no action. |
| Removal | Retire press and capture records with their mount. No later event can reach a retired handler or a replacement at the same position. Dropping a view clears all of its pointer records. |

Capture is per pointer within a view. On ordinary surface leave, cancel an
uncaptured press. A host that can continue tracking a captured pointer delivers
its moves and release outside the surface. A host that loses the sequence
sends cancel. Handler storage belongs to the mount. The router keeps mouse positions until surface exit, cancellation or deactivation.

## Input and signal delivery

Dispatch callbacks synchronously. Signal writes are immediately readable by the
next callback. The host calls `flush()` after dispatch at its update boundary,
then lays out and paints pending work. A batch of moves can therefore run each
handler while the displayed signal advances to its latest value. A failed
frame submission leaves the last accepted frame authoritative for input.

## Headless validation

- Press the label inside a button, release over the same button and observe
  one activation. Move outside and back before release and observe zero.
- Capture a seek bar, move beyond either edge and resize it. Check the
  fractional value against the bounds of each accepted frame. Hover and a
  second pointer must not advance that drag.
- Remove a captured element at flush, fail the next submission, and mount a
  replacement at the same coordinates. A late release must reach neither the
  retired handler nor the replacement. A fresh press can reach the replacement
  after the host accepts its frame.

The integration tests drive the public host API with logical coordinates. Key callbacks follow logical focus independently of pointer capture.
The existing disclosure and mount-lifetime experiments provide the starting
fixtures. [Disclosure interaction](disclosure.md) and [mount lifetime](node-execution.md#removing-a-pressed-child).
