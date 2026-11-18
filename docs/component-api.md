# Component authoring draft

These Rust sketches explore the author-facing API. The names and signatures
are candidates. The existing core provides signals, layout nodes and shared
geometry. A private headless fixture compiles the expandable section below. Custom drawing
and dragging remain sketches.

## Composition

Start with a general container and a text leaf. The container arranges children
and can receive input or record custom drawing in its allocated bounds. The
text leaf owns shaping and measurement. A row or column is a layout choice
on the container. These capabilities can be used directly in application code.

A stateful section can declare its mount boundary explicitly. In this sketch
`component` takes a factory. The framework invokes it once when attaching
each instance. The scope itself adds no layout box.

```rust
component(|mount| {
    let open = mount.signal(false);
    let volume = mount.signal(40_u8);

    div()
        .column()
        .gap(8)
        .child(
            div()
                .padding(6)
                .child(text("Audio settings"))
                .on_click(move |event| event.update(open, |v| *v = !*v)),
        )
        .child_when(open, || {
            div().column()
                .child(text("Output device"))
                .child(text("System default"))
        })
        .child(
            div().child(text("Volume"))
                .on_click(move |event| event.update(volume, |v| *v = v.saturating_add(1))),
        )
})
```

The private `text` builder accepts a literal, an owned string or an existing
`SmolStr`. Passing a clone of a long shared value lets multiple nodes retain the
same bytes through layout. Short labels fit inline. The [source text
experiment](text-layout.md#shared-source-text) measures both cases.

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

Each call to `mount.signal(value)` creates a distinct `Signal<T>`, including
when two calls have identical types and initial values. Handlers capture the
typed handles they use. The instance keeps a list of signal identities for
disposal. Ordinary Rust code can hold the typed handle in a model and pass it
to the same runtime's `read` and `set` methods. In the playback fixture a model
method toggles one signal shared by two views. Each write is immediately
readable. The next host flush updates each remaining subscriber. The model
retains its externally created signal after both views unmount and explicitly
removes it when finished. Copying a component-owned handle outside its mount
does not extend its lifetime.

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

The factory may return text or a container. A conditional child reserves a
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

The private fixture uses one scene per instance. It stores declaration paths,
counts mounted preceding siblings on insertion, and removes retired entries
on close. The cost on deep or wide trees, native hit testing and frame
presentation are outside this check.

```sh
cargo test --offline --example text_window component_authoring
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
        .height(20)
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

The paint callback receives the current bound value by copy and records in local
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
