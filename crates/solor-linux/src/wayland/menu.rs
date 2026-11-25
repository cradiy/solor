use super::{Desktop, Mapping, Role, Window};
use crate::popup::{ConstraintAdjustment, PopupOptions};
use anyhow::{Context, Result, ensure};
use solor::{
    Pixels,
    layout::Size,
    platform::{PlatformWindow, WindowHandle},
    px,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop,
    globals::GlobalList,
    protocol::{wl_seat::WlSeat, wl_surface::WlSurface},
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup::{self, XdgPopup},
    xdg_positioner::XdgPositioner,
    xdg_surface::XdgSurface,
    xdg_wm_base::XdgWmBase,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

#[derive(Default)]
pub(super) struct Menus {
    input: RefCell<Option<Trigger>>,
    root: RefCell<Weak<Menu>>,
}

struct Trigger {
    parent: Parent,
    bounds: [i32; 4],
    seat: WlSeat,
    serial: u32,
}

enum Parent {
    Panel(ObjectId, ZwlrLayerSurfaceV1),
    Menu(Rc<Menu>),
}

struct MenuShell(XdgWmBase);

impl Drop for MenuShell {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

impl Menus {
    pub fn input(&self, window: &Window, seat: &WlSeat, serial: u32) {
        if window.mapping != Mapping::Mapped || !window.life.is_open() {
            return;
        }
        let parent = match &window.role {
            Role::Layer(layer, _) => Parent::Panel(window.surface.id(), layer.clone()),
            Role::Menu(menu) => Parent::Menu(menu.clone()),
            _ => return,
        };
        let bounds = window
            .floating
            .as_ref()
            .map(|panel| panel.geometry.bounds())
            .unwrap_or([
                0,
                0,
                window.size.width.get() as i32,
                window.size.height.get() as i32,
            ]);
        self.input.replace(Some(Trigger {
            parent,
            bounds,
            seat: seat.clone(),
            serial,
        }));
    }

    pub fn end_input(&self) {
        self.input.take();
    }

    pub fn dismiss_current(&self) {
        if let Some(menu) = self.root.borrow().upgrade() {
            menu.dismiss();
        }
    }

    pub fn dismiss_child(&self, parent: &ObjectId) {
        if let Some(menu) = self.root.borrow().upgrade()
            && &menu.panel == parent
        {
            menu.dismiss();
        }
    }

    pub fn create(
        &self,
        options: PopupOptions,
        globals: &GlobalList,
        qh: &QueueHandle<Desktop>,
        surface: &WlSurface,
        handle: WindowHandle,
    ) -> Result<(Role, Size)> {
        let trigger = self
            .input
            .take()
            .context("a popup must open during a pointer button event on a mapped panel or menu")?;
        ensure!(
            ConstraintAdjustment::all().contains(options.constraints),
            "unknown popup constraint adjustment"
        );
        let w = crate::layer_shell::units(options.size.width)?;
        let h = crate::layer_shell::units(options.size.height)?;
        ensure!(
            w > 0 && h > 0,
            "popup size must round to positive logical pixels"
        );
        let (origin, size) = options.anchor_rect;
        let [ox, oy, pw, ph] = trigger.bounds;
        let (x, width) = anchor_axis(origin.x, size.width, pw)?;
        let (y, height) = anchor_axis(origin.y, size.height, ph)?;
        let (shell, panel) = match &trigger.parent {
            Parent::Panel(id, _) => (Rc::new(MenuShell(globals.bind(qh, 1..=1, ())?)), id.clone()),
            Parent::Menu(parent) => {
                ensure!(
                    parent.seat == trigger.seat,
                    "a submenu requires its parent's seat"
                );
                (parent.shell.clone(), parent.panel.clone())
            }
        };
        let positioner = shell.0.create_positioner(qh, ());
        positioner.set_size(w, h);
        positioner.set_anchor_rect(ox + x, oy + y, width, height);
        positioner.set_anchor(options.anchor);
        positioner.set_gravity(options.gravity);
        positioner.set_constraint_adjustment(options.constraints);
        let xdg = shell.0.get_xdg_surface(surface, qh, surface.id());
        let slot = match &trigger.parent {
            Parent::Panel(..) => &self.root,
            Parent::Menu(parent) => &parent.child,
        };
        // A new grab must extend the topmost popup. Retire the previous branch first.
        if let Some(previous) = slot.borrow().upgrade() {
            previous.dismiss();
        }
        let popup = match &trigger.parent {
            Parent::Panel(_, layer) => {
                let popup = xdg.get_popup(None, &positioner, qh, surface.id());
                layer.get_popup(&popup);
                popup
            }
            Parent::Menu(parent) => xdg.get_popup(Some(&parent.xdg), &positioner, qh, surface.id()),
        };
        popup.grab(&trigger.seat, trigger.serial);
        positioner.destroy();
        let menu = Rc::new(Menu {
            panel,
            child: Default::default(),
            seat: trigger.seat,
            shell,
            xdg,
            popup,
            handle,
            dismissed: Cell::new(false),
        });
        slot.replace(Rc::downgrade(&menu));
        Ok((
            Role::Menu(menu),
            Size {
                width: px(w as f32),
                height: px(h as f32),
            },
        ))
    }
}

fn anchor_axis(origin: Pixels, extent: Pixels, limit: i32) -> Result<(i32, i32)> {
    let start = f64::from(origin);
    let extent = f64::from(extent);
    ensure!(
        start.is_finite() && extent.is_finite() && extent > 0.,
        "invalid popup anchor rectangle"
    );
    let end = (start + extent).ceil().min(f64::from(limit));
    let start = start.floor().max(0.);
    ensure!(end > start, "popup anchor is outside the panel");
    Ok((start as i32, (end - start) as i32))
}

pub(super) struct Menu {
    panel: ObjectId,
    child: RefCell<Weak<Menu>>,
    seat: WlSeat,
    shell: Rc<MenuShell>,
    xdg: XdgSurface,
    popup: XdgPopup,
    handle: WindowHandle,
    dismissed: Cell<bool>,
}

impl Menu {
    pub fn dismiss(&self) {
        if !self.dismissed.replace(true) {
            if let Some(child) = self.child.take().upgrade() {
                child.dismiss();
            }
            self.popup.destroy();
            self.handle.close();
        }
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        self.dismiss();
        self.xdg.destroy();
    }
}

delegate_noop!(Desktop: ignore XdgPositioner);

impl Dispatch<XdgPopup, ObjectId> for Desktop {
    fn event(
        desktop: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(window) = desktop.windows.get_mut(id) else {
            return;
        };
        let Role::Menu(menu) = &window.role else {
            return;
        };
        match event {
            xdg_popup::Event::Configure { width, height, .. } if width > 0 && height > 0 => {
                window.pending_size = Some(Size {
                    width: px(width as f32),
                    height: px(height as f32),
                });
            }
            xdg_popup::Event::PopupDone => menu.dismiss(),
            _ => {}
        }
    }
}
