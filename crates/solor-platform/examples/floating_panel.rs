//! A floating Wayland panel. Drag its body to move it within the output.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::{
        current_platform,
        layer_shell::{Anchor, Layer, LayerShellOptions},
    };

    let height = px(36.0);
    let options = LayerShellOptions {
        movable: true,
        anchor: Anchor::Top | Anchor::Right,
        width: Some(px(300.0)),
        height: Some(height),
        ..LayerShellOptions::new("solor-floating-panel", Layer::Top)
    }
    .transparent();
    let panel = component(move |_| {
        div()
            .w_full()
            .h_full()
            .flex_row()
            .items_center()
            .rounded_lg()
            .justify_between()
            .px(px(12.0))
            .bg(rgb(0x252b30))
            .text_color(rgb(0xf0f3eb))
            .window_drag_region()
            .child(text("Drag to move"))
            .child(div().window_close_button().child(text("Close")))
    });
    current_platform()?.run_layer_surface(options, Ui::new(), panel)
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the layer panel requires Wayland on Linux")
}
