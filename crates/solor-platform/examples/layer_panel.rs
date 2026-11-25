//! A fixed-width Wayland panel centered on the top edge.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::{
        current_platform,
        layer_shell::{Anchor, ExclusiveZone, Layer, LayerShellOptions, LayerSurfaceHandle},
        popup::PopupOptions,
    };

    let height = px(36.0);
    let handle = LayerSurfaceHandle::new(true)?;
    let options = LayerShellOptions {
        handle: Some(&handle),
        anchor: Anchor::Top,
        width: Some(px(300.0)),
        height: Some(height),
        exclusive_zone: ExclusiveZone::Reserve(height),
        ..LayerShellOptions::new("solor-panel", Layer::Top)
    }
    .transparent();
    current_platform()?.run(|app| {
        let menus = app.clone();
        let panel_handle = handle.clone();
        let panel = component(move |_| {
            let density =
                div()
                    .px(px(8.0))
                    .child(text("Density"))
                    .on_pointer_down(move |event, _| {
                        if event.is_primary_action() {
                            let panel = panel_handle.clone();
                            menus.open_popup(
                                PopupOptions::below(event, px(144.0), px(72.0)),
                                Ui::new(),
                                move |menu| density_menu(panel, menu),
                            )?;
                        }
                        Ok(())
                    });
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
                .child(density)
                .child(div().window_close_button().child(text("x")))
        });
        app.open_layer_surface(options, Ui::new(), panel)?;
        Ok(())
    })
}

#[cfg(target_os = "linux")]
fn density_menu(
    panel: solor_platform::layer_shell::LayerSurfaceHandle,
    menu: solor_platform::WindowHandle,
) -> impl solor::Render {
    use solor::{Styled, component, div, px, rgb, text};
    use solor_platform::layer_shell::ExclusiveZone;

    component(move |_| {
        let items = [("Compact", 30.0), ("Comfortable", 44.0)].map(|(label, height)| {
            let panel = panel.clone();
            let menu = menu.clone();
            div()
                .h(px(36.0))
                .w_full()
                .px(px(12.0))
                .justify_center()
                .child(text(label))
                .on_click(move |_| {
                    panel.set_size(Some(px(300.0)), Some(px(height)))?;
                    panel.set_exclusive_zone(ExclusiveZone::Reserve(px(height)))?;
                    menu.close();
                    Ok(())
                })
        });
        div()
            .h_full()
            .w_full()
            .flex_col()
            .bg(rgb(0x252b30))
            .text_color(rgb(0xf0f3eb))
            .children(items)
    })
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the layer panel requires Wayland on Linux")
}
