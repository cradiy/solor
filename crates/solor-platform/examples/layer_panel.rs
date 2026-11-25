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
            let active = std::cell::RefCell::new(None::<solor_platform::WindowHandle>);
            let density =
                div()
                    .px(px(8.0))
                    .child(text("Density"))
                    .on_pointer_down(move |event, _| {
                        if event.is_primary_action() {
                            if let Some(menu) = active.take().filter(|menu| menu.is_open()) {
                                menu.close();
                            } else {
                                let panel = panel_handle.clone();
                                let app = menus.clone();
                                let menu = menus.open_popup(
                                    PopupOptions::below(event, px(156.0), px(108.0)),
                                    Ui::new(),
                                    move |menu| density_menu(app, panel, menu),
                                )?;
                                active.replace(Some(menu));
                            }
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
    app: solor_platform::Application,
    panel: solor_platform::layer_shell::LayerSurfaceHandle,
    menu: solor_platform::WindowHandle,
) -> impl solor::Render {
    use solor::{Styled, Ui, component, div, px, rgb, text};
    use solor_platform::popup::PopupOptions;

    component(move |_| {
        let items = [("Compact", 30.0), ("Comfortable", 44.0)]
            .map(|(label, height)| density_item(label, height, panel.clone(), menu.clone()));
        let child = std::rc::Rc::new(std::cell::RefCell::new(
            None::<solor_platform::WindowHandle>,
        ));
        let open = move |event: &solor::pointer::PointerEvent| -> anyhow::Result<()> {
            if child
                .borrow()
                .as_ref()
                .is_some_and(|window| window.is_open())
            {
                return Ok(());
            }
            let panel = panel.clone();
            let root = menu.clone();
            let options = PopupOptions {
                parent: Some(root.clone()),
                grab: false,
                ..PopupOptions::beside(event, px(144.0), px(72.0))
            };
            let submenu = app.open_popup(options, Ui::new(), move |submenu| {
                component(move |_| {
                    div()
                        .w_full()
                        .h_full()
                        .flex_col()
                        .bg(rgb(0x30383e))
                        .text_color(rgb(0xf0f3eb))
                        .child(density_item("Spacious", 54.0, panel, root))
                        .child(
                            div()
                                .w_full()
                                .h(px(36.0))
                                .px(px(12.0))
                                .justify_center()
                                .child(text("Back"))
                                .on_click(move |_| {
                                    submenu.close();
                                    Ok(())
                                }),
                        )
                })
            })?;
            child.replace(Some(submenu));
            Ok(())
        };
        let hover = open.clone();
        let more = div()
            .w_full()
            .h(px(36.0))
            .px(px(12.0))
            .flex_row()
            .items_center()
            .justify_between()
            .child(text("More sizes"))
            .child(text("›"))
            .on_pointer_move(move |event, _| hover(event))
            .on_pointer_down(move |event, _| {
                if event.is_primary_action() {
                    open(event)?;
                }
                Ok(())
            });
        div()
            .h_full()
            .w_full()
            .flex_col()
            .bg(rgb(0x252b30))
            .text_color(rgb(0xf0f3eb))
            .children(items)
            .child(more)
    })
}

#[cfg(target_os = "linux")]
fn density_item(
    label: &'static str,
    height: f32,
    panel: solor_platform::layer_shell::LayerSurfaceHandle,
    menu: solor_platform::WindowHandle,
) -> impl solor::IntoElement {
    use solor::{Styled, div, px, text};
    use solor_platform::layer_shell::ExclusiveZone;
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
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the layer panel requires Wayland on Linux")
}
