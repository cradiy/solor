use anyhow::{Context, Result, ensure};
use solor::{
    Pixels,
    layout::Size,
    platform::{PlatformWindow, WindowOptions},
    px,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, globals::GlobalList,
    protocol::wl_surface::WlSurface,
};
use wayland_protocols::xdg::dialog::v1::client::{
    xdg_dialog_v1::XdgDialogV1, xdg_wm_dialog_v1::XdgWmDialogV1,
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Event, ZwlrLayerSurfaceV1},
};

use super::{Desktop, decorations::Decorations, stacking::Topmost};
use crate::layer_shell::{
    Anchor, KeyboardInteractivity, Layer, LayerChanges, LayerRules, LayerShellOptions, extent,
    units,
};

type AssignPopup<'a> = Box<dyn FnOnce(&WlSurface) -> Result<Box<dyn std::any::Any>> + 'a>;

pub(super) enum Request<'a> {
    Menu(crate::popup::PopupOptions),
    Popup(AssignPopup<'a>),
    Toplevel(WindowOptions<'a>),
    Layer(LayerShellOptions<'a>),
}

pub(super) enum Role {
    Menu(std::rc::Rc<super::menu::Menu>),
    Popup {
        _owner: Box<dyn std::any::Any>,
    },
    Toplevel {
        resizable: bool,
        shell: XdgWmBase,
        surface: XdgSurface,
        top: XdgToplevel,
        decorations: Option<Box<Decorations>>,
        dialog: Option<Box<XdgDialogV1>>,
        topmost: Option<Topmost>,
    },
    Layer(ZwlrLayerSurfaceV1, LayerPlacement),
}

pub(super) struct LayerPlacement {
    pub(super) movable: bool,
    layer: Layer,
    pub(super) anchor: Anchor,
    size: (u32, u32),
    pub(super) margin: [i32; 4],
    zone: i32,
    keyboard: KeyboardInteractivity,
}

impl LayerPlacement {
    pub(super) fn update(
        &mut self,
        changes: LayerChanges,
        surface: Option<&ZwlrLayerSurfaceV1>,
    ) -> bool {
        let mut changed = false;
        if let Some(size) = changes.size
            && self.size != size
        {
            self.size = size;
            if let Some(surface) = surface
                && !self.movable
            {
                surface.set_size(size.0, size.1);
            }
            changed = true;
        }
        if let Some(zone) = changes.zone
            && self.zone != zone
        {
            self.zone = zone;
            if let Some(surface) = surface {
                surface.set_exclusive_zone(zone);
            }
            changed = true;
        }
        if let Some(keyboard) = changes.keyboard
            && self.keyboard != keyboard
        {
            self.keyboard = keyboard;
            if let Some(surface) = surface {
                surface.set_keyboard_interactivity(keyboard);
            }
            changed = true;
        }
        changed
    }

    pub(super) fn apply(&self, surface: &ZwlrLayerSurfaceV1, remapping: bool) {
        if remapping && surface.version() >= 2 {
            surface.set_layer(self.layer);
        }
        if self.movable {
            surface.set_anchor(Anchor::all());
            surface.set_size(0, 0);
        } else {
            surface.set_anchor(self.anchor);
            surface.set_size(self.size.0, self.size.1);
            let [top, right, bottom, left] = self.margin;
            surface.set_margin(top, right, bottom, left);
        }
        surface.set_exclusive_zone(self.zone);
        surface.set_keyboard_interactivity(self.keyboard);
    }
}

impl Request<'_> {
    pub(super) fn create(
        self,
        globals: &GlobalList,
        qh: &QueueHandle<Desktop>,
        surface: &WlSurface,
        menus: &super::menu::Menus,
        handle: solor::platform::WindowHandle,
    ) -> Result<(Role, Size)> {
        match self {
            Self::Menu(options) => menus.create(options, globals, qh, surface, handle),
            Self::Popup(assign) => Ok((
                Role::Popup {
                    _owner: assign(surface)?,
                },
                Size {
                    width: px(1.),
                    height: px(1.),
                },
            )),
            Self::Toplevel(options) => {
                ensure!(
                    !options.title.contains('\0'),
                    "window title contains a null byte"
                );
                ensure!(
                    options.app_id.is_none_or(|id| !id.contains('\0')),
                    "application identifier contains a null byte"
                );
                let sizing = ToplevelSizing::new(&options)?;
                let shell: XdgWmBase = globals.bind(qh, 1..=1, ())?;
                let topmost = if options.always_on_top {
                    Topmost::request(globals, qh, surface)
                } else {
                    None
                };
                let xdg_surface = shell.get_xdg_surface(surface, qh, surface.id());
                let top = xdg_surface.get_toplevel(qh, surface.id());
                sizing.apply(&top);
                let decorations =
                    Decorations::bind(globals, qh, &top, surface.id(), options.native_decorations)?;
                let dialog = if options.modal {
                    globals
                        .bind::<XdgWmDialogV1, _, _>(qh, 1..=1, ())
                        .ok()
                        .map(|manager| {
                            let dialog = manager.get_xdg_dialog(&top, qh, ());
                            manager.destroy();
                            dialog.set_modal();
                            Box::new(dialog)
                        })
                } else {
                    None
                };
                top.set_title(options.title.into());
                if let Some(app_id) = options.app_id {
                    top.set_app_id(app_id.into());
                }
                Ok((
                    Role::Toplevel {
                        resizable: sizing.resizable,
                        shell,
                        surface: xdg_surface,
                        top,
                        decorations: decorations.map(Box::new),
                        dialog,
                        topmost,
                    },
                    sizing.initial,
                ))
            }
            Self::Layer(options) => {
                let (width, height) = requested_size(&options)?;
                let margin = options.margin;
                let (top, right, bottom, left) = (
                    units(margin.top)?,
                    units(margin.right)?,
                    units(margin.bottom)?,
                    units(margin.left)?,
                );
                let zone = options.exclusive_zone.units()?;
                ensure!(
                    !options.namespace.contains('\0'),
                    "layer namespace contains a null byte"
                );
                let minimum = if options.keyboard_interactivity == KeyboardInteractivity::OnDemand {
                    4
                } else {
                    1
                };
                let output = options
                    .output
                    .map(|name| super::output::NamedOutput::find(globals, name))
                    .transpose()?;
                let shell: ZwlrLayerShellV1 =
                    globals.bind(qh, minimum..=4, ()).with_context(|| {
                        format!(
                            "layer surface requires zwlr_layer_shell_v1 version {minimum} or later"
                        )
                    })?;
                let layer = shell.get_layer_surface(
                    surface,
                    output.as_ref().map(|output| &output.0),
                    options.layer,
                    options.namespace.into(),
                    qh,
                    surface.id(),
                );
                if shell.version() >= zwlr_layer_shell_v1::REQ_DESTROY_SINCE {
                    shell.destroy();
                }
                let placement = LayerPlacement {
                    movable: options.movable,
                    layer: options.layer,
                    anchor: options.anchor,
                    size: (width, height),
                    margin: [top, right, bottom, left],
                    zone,
                    keyboard: options.keyboard_interactivity,
                };
                // A zero configure dimension lets the client choose its size for that axis.
                let size = Size {
                    width: px(width.max(1) as f32),
                    height: px(height.max(1) as f32),
                };
                Ok((Role::Layer(layer, placement), size))
            }
        }
    }
}

struct ToplevelSizing {
    resizable: bool,
    initial: Size,
    min: Option<(i32, i32)>,
    max: Option<(i32, i32)>,
}

impl ToplevelSizing {
    fn new(options: &WindowOptions<'_>) -> Result<Self> {
        let convert = |size: Size, round: fn(f64) -> f64| -> Result<(i32, i32)> {
            let axis = |p: Pixels| {
                let value = round(f64::from(p));
                ensure!(
                    p.is_extent() && value <= f64::from(i32::MAX),
                    "window size outside the Wayland coordinate range"
                );
                Ok(value as i32)
            };
            Ok((axis(size.width)?, axis(size.height)?))
        };
        ensure!(
            options.size.width.get() >= 1.0 && options.size.height.get() >= 1.0,
            "invalid initial window size"
        );
        let (width, height) = convert(options.size, f64::round)?;
        let min = options
            .min_size
            .map(|s| convert(s, f64::ceil))
            .transpose()?;
        let max = options
            .max_size
            .map(|s| convert(s, f64::floor))
            .transpose()?;
        let (min_w, min_h) = min.unwrap_or((1, 1));
        let (max_w, max_h) = max.unwrap_or((i32::MAX, i32::MAX));
        let (min_w, min_h) = (min_w.max(1), min_h.max(1));
        ensure!(
            min_w <= max_w && min_h <= max_h,
            "no whole logical window size fits the requested bounds"
        );
        let width = width.clamp(min_w, max_w);
        let height = height.clamp(min_h, max_h);
        let fixed = Some((width, height));
        Ok(Self {
            resizable: options.resizable && (min_w, min_h) != (max_w, max_h),
            initial: Size {
                width: px(width as f32),
                height: px(height as f32),
            },
            min: if options.resizable { min } else { fixed },
            max: if options.resizable { max } else { fixed },
        })
    }

    fn apply(&self, top: &XdgToplevel) {
        if let Some((width, height)) = self.min {
            top.set_min_size(width, height);
        }
        if let Some((width, height)) = self.max {
            top.set_max_size(width, height);
        }
    }
}

fn requested_size(options: &LayerShellOptions<'_>) -> Result<(u32, u32)> {
    ensure!(
        Anchor::all().contains(options.anchor),
        "unknown layer surface anchor"
    );
    let size = (extent(options.width)?, extent(options.height)?);
    LayerRules {
        anchor: options.anchor,
        movable: options.movable,
        version: 4,
    }
    .validate(LayerChanges {
        size: Some(size),
        zone: Some(options.exclusive_zone.units()?),
        keyboard: None,
    })?;
    Ok(size)
}

impl Drop for Role {
    fn drop(&mut self) {
        match self {
            Self::Toplevel {
                decorations,
                topmost,
                dialog,
                top,
                surface,
                shell,
                ..
            } => {
                if let Some(dialog) = dialog.take() {
                    dialog.destroy();
                }
                drop(topmost.take());
                drop(decorations.take());
                top.destroy();
                surface.destroy();
                shell.destroy();
            }
            Self::Layer(surface, _) => surface.destroy(),
            Self::Menu(_) | Self::Popup { .. } => {}
        }
    }
}

delegate_noop!(Desktop: ignore XdgDialogV1);
delegate_noop!(Desktop: ignore XdgWmDialogV1);
delegate_noop!(Desktop: ignore ZwlrLayerShellV1);

impl Dispatch<ZwlrLayerSurfaceV1, wayland_client::backend::ObjectId> for Desktop {
    fn event(
        desktop: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: Event,
        id: &wayland_client::backend::ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(state) = desktop.windows.get_mut(id) else {
            return;
        };
        match event {
            Event::Configure {
                serial,
                width,
                height,
            } => {
                if !state.mapping.configure() {
                    return;
                }
                surface.ack_configure(serial);
                if let Some(panel) = &mut state.floating {
                    let size = panel.configure(width, height);
                    state.resize(size);
                    return;
                }
                let Role::Layer(_, placement) = &state.role else {
                    return;
                };
                let axis = |configured, requested, current| match (configured, requested) {
                    (0, 0) => current,
                    (0, requested) => px(requested as f32),
                    (configured, _) => px(configured as f32),
                };
                let size = Size {
                    width: axis(width, placement.size.0, state.size.width),
                    height: axis(height, placement.size.1, state.size.height),
                };
                state.resize(size);
            }
            Event::Closed => state.life.close(),
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/layer_shell.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/toplevel.rs"]
mod toplevel_tests;
