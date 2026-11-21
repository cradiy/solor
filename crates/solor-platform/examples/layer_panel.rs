//! A fixed-width Wayland panel centered on the top edge.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::{
        current_platform,
        layer_shell::{Anchor, ExclusiveZone, Layer, LayerShellOptions},
    };

    let height = px(36.0);
    let options = LayerShellOptions {
        anchor: Anchor::Top,
        width: Some(px(300.0)),
        height: Some(height),
        exclusive_zone: ExclusiveZone::Reserve(height),
        ..LayerShellOptions::new("solor-panel", Layer::Top)
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
            .child(text("Solor"))
            .child(div().window_close_button().child(text("x")))
    });
    current_platform()?.run_layer_surface(options, Ui::new(), panel)
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the layer panel requires Wayland on Linux")
}
