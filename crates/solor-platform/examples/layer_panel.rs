//! A Wayland top panel that reserves one row of desktop space.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::{
        current_platform,
        layer_shell::{Anchor, ExclusiveZone, Layer, LayerShellOptions},
    };

    let height = px(36.0);
    let options = LayerShellOptions {
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        height: Some(height),
        exclusive_zone: ExclusiveZone::Reserve(height),
        ..LayerShellOptions::new("solor-panel", Layer::Top)
    };
    let panel = component(move |_| {
        div()
            .w_full()
            .h(height)
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .bg(rgb(0x252b30))
            .text_color(rgb(0xf0f3eb))
            .child(text("Solor"))
            .child(div().window_close_button().child(text("Close")))
    });
    current_platform()?.run_layer_surface(options, Ui::new(), panel)
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the layer panel requires Wayland on Linux")
}
