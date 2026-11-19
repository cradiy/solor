# Pointer input

A button needs one activation. A seek bar needs the pointer's position while
it moves, including outside the track. Both can update the same application
signals through their event context. Platform adapters deliver normalized
input with `Ui::dispatch_pointer`. A host can also activate a known node
directly with `Ui::click(view, target)`.

## Authoring

Buttons use the context-only click callback. Controls that need coordinates use `on_pointer_down`, `on_pointer_move`, `on_pointer_up`
on the common element builder. They accept `|event, cx| -> Result<()>`.
`on_pointer_cancel` receives the pointer ID. The context provides the same state operations
as a click handler. During a down callback it can also capture the current
pointer for the receiving element.

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
        let width = event.bounds().width.get();
        if *cx.read(dragging)? == Some(event.pointer_id()) && width > 0.0 {
            let fraction = (event.local_position().x.get() / width).clamp(0.0, 1.0);
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

The adapter supplies a view-scoped pointer ID, device kind, modifiers and
view-local position in floating-point logical pixels. Down and up carry the
changed button. The primary action is the primary mouse button, touch contact
or pen tip. A cancel needs only the pointer ID. For positioned events the
router derives border-box-local coordinates and bounds from the same accepted
frame. Captured coordinates may fall outside the box.

## Host boundary

After flushing and laying out a view, call `ui.prepare_pointer_frame(view)?`
alongside its paint recording. Keep the returned snapshot with that submission.
On successful presentation call `ui.accept_pointer_frame(frame)?`. Drop a
failed submission's snapshot. Input before the first acceptance has no target.
If the newly accepted snapshot omits a live press owner, acceptance cancels
that sequence. A cancel callback error leaves the new snapshot in effect.

Recordings of the same mounted tree share its immutable hit geometry until a
node is inserted or removed, or layout publishes changed geometry. The next
recording then builds a new snapshot. Accepting a submission selects its own
snapshot for input, even if the view has already prepared a different layout.

Hit testing uses the view rectangle and axis-aligned border boxes, in reverse
declaration paint order. `PointerEvents::None` skips that box as a target;
its children remain eligible. Captured motion looks up its owner directly.
Transformed or rounded clips and z-index ordering require frame composition.
The host supplies `Leave` on surface exit, `Cancel` on loss of one pointer,
and calls `cancel_pointers(view)` on deactivation.

## Routing and lifetime

| Event | Behavior |
| --- | --- |
| Down | Hit-test the host's accepted frame. Resolve the frontmost hit to its nearest live interactive ancestor within the view, including the hit node itself. Keep one recipient for the primary press so a text label inside a button activates the button. A stale frontmost hit consumes the hit without targeting an element underneath. |
| Move | Route to the current hit unless this pointer is captured. Capture routes moves to its owner and suppresses click recognition for that press. For an uncaptured press, leaving its target cancels click eligibility through release. |
| Up | Deliver the raw up first, to the capture owner or the current hit. After a successful callback, a matching primary release over the original, still-eligible target invokes its click handler once. The release of the initiating button ends capture even on callback failure. An unmatched up cannot activate a target. |
| Cancel | Device loss, host cancellation or view deactivation clears the sequence. Notify its live owner once. A late up after cancel has no action. |
| Removal | Retire press and capture records with their mount. No later event can reach a retired handler or a replacement at the same position. Dropping a view clears all of its pointer records. |

Capture is per pointer within a view. On ordinary surface leave, cancel an
uncaptured press. A host that can continue tracking a captured pointer delivers
its moves and release outside the surface. A host that loses the sequence
sends cancel. Handler storage belongs to the mount. The router needs records
only for active presses or captures.

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

The integration tests drive the public host API with logical coordinates. Focus,
keyboard activation and ancestor event propagation have separate contracts.
The existing disclosure and mount-lifetime experiments provide the starting
fixtures. [Disclosure interaction](disclosure.md) and [mount lifetime](node-execution.md#removing-a-pressed-child).
