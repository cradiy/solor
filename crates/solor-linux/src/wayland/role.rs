use anyhow::{Context, Result, ensure};
use solor::{Pixels, layout::Size, platform::WindowOptions, px};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, globals::GlobalList,
    protocol::wl_surface::WlSurface,
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Event, ZwlrLayerSurfaceV1},
};

use super::{Window, decorations::Decorations};
use crate::layer_shell::{Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerShellOptions};

pub(super) enum Request<'a> {
    Toplevel(WindowOptions<'a>),
    Layer(LayerShellOptions<'a>),
}

pub(super) enum Role {
    Toplevel {
        shell: XdgWmBase,
        surface: XdgSurface,
        top: XdgToplevel,
        decorations: Option<Decorations>,
    },
    Layer(ZwlrLayerSurfaceV1, LayerPlacement),
}

pub(super) struct LayerPlacement {
    layer: Layer,
    anchor: Anchor,
    size: (u32, u32),
    margin: [i32; 4],
    zone: i32,
    keyboard: KeyboardInteractivity,
}

impl LayerPlacement {
    pub(super) fn apply(&self, surface: &ZwlrLayerSurfaceV1, remapping: bool) {
        if remapping && surface.version() >= 2 {
            surface.set_layer(self.layer);
        }
        surface.set_anchor(self.anchor);
        surface.set_size(self.size.0, self.size.1);
        let [top, right, bottom, left] = self.margin;
        surface.set_margin(top, right, bottom, left);
        surface.set_exclusive_zone(self.zone);
        surface.set_keyboard_interactivity(self.keyboard);
    }
}

impl Request<'_> {
    pub(super) fn create(
        self,
        globals: &GlobalList,
        qh: &QueueHandle<Window>,
        surface: &WlSurface,
    ) -> Result<(Role, Size)> {
        match self {
            Self::Toplevel(options) => {
                let size = options.size;
                ensure!(
                    size.width.is_extent()
                        && size.height.is_extent()
                        && size.width.get() >= 1.0
                        && size.height.get() >= 1.0,
                    "invalid initial window size"
                );
                let size = Size {
                    width: px(size.width.get().round()),
                    height: px(size.height.get().round()),
                };
                let shell: XdgWmBase = globals.bind(qh, 1..=1, ())?;
                let xdg_surface = shell.get_xdg_surface(surface, qh, ());
                let top = xdg_surface.get_toplevel(qh, ());
                let decorations = Decorations::bind(globals, qh, &top, options.native_decorations)?;
                top.set_title(options.title.into());
                if let Some(app_id) = options.app_id {
                    top.set_app_id(app_id.into());
                }
                Ok((
                    Role::Toplevel {
                        shell,
                        surface: xdg_surface,
                        top,
                        decorations,
                    },
                    size,
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
                let zone = match options.exclusive_zone {
                    ExclusiveZone::Avoid => 0,
                    ExclusiveZone::Ignore => -1,
                    ExclusiveZone::Reserve(value) => {
                        let distance = units(value)?;
                        ensure!(distance > 0, "reserved distance must be positive");
                        distance
                    }
                };
                ensure!(
                    !options.namespace.contains('\0'),
                    "layer namespace contains a null byte"
                );
                let minimum = if options.keyboard_interactivity == KeyboardInteractivity::OnDemand {
                    4
                } else {
                    1
                };
                let shell: ZwlrLayerShellV1 =
                    globals.bind(qh, minimum..=4, ()).with_context(|| {
                        format!(
                            "layer surface requires zwlr_layer_shell_v1 version {minimum} or later"
                        )
                    })?;
                let layer = shell.get_layer_surface(
                    surface,
                    None,
                    options.layer,
                    options.namespace.into(),
                    qh,
                    (),
                );
                if shell.version() >= zwlr_layer_shell_v1::REQ_DESTROY_SINCE {
                    shell.destroy();
                }
                let placement = LayerPlacement {
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

fn units(pixels: Pixels) -> Result<i32> {
    let rounded = f64::from(pixels).round();
    ensure!(
        (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&rounded),
        "layer surface length is outside the Wayland coordinate range"
    );
    Ok(rounded as i32)
}

fn requested_size(options: &LayerShellOptions<'_>) -> Result<(u32, u32)> {
    ensure!(
        Anchor::all().contains(options.anchor),
        "unknown layer surface anchor"
    );
    let axis = |length, edges| -> Result<u32> {
        match length {
            Some(value) => {
                let units = units(value)?;
                ensure!(
                    units > 0,
                    "layer surface extent must round to a positive logical size"
                );
                Ok(units as u32)
            }
            None => {
                ensure!(
                    options.anchor.contains(edges),
                    "an automatic layer surface extent requires both opposite anchors"
                );
                Ok(0)
            }
        }
    };
    Ok((
        axis(options.width, Anchor::Left | Anchor::Right)?,
        axis(options.height, Anchor::Top | Anchor::Bottom)?,
    ))
}

impl Drop for Role {
    fn drop(&mut self) {
        match self {
            Self::Toplevel {
                decorations,
                top,
                surface,
                shell,
            } => {
                drop(decorations.take());
                top.destroy();
                surface.destroy();
                shell.destroy();
            }
            Self::Layer(surface, _) => surface.destroy(),
        }
    }
}

delegate_noop!(Window: ignore ZwlrLayerShellV1);

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Window {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
                let mut size = state.size;
                if width > 0 {
                    size.width = px(width as f32);
                }
                if height > 0 {
                    size.height = px(height as f32);
                }
                state.resize(size);
            }
            Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/layer_shell.rs"]
mod tests;
