use super::{Desktop, Role, Window, WindowLife};
use crate::popup::{ConstraintAdjustment, PopupOptions};
use crate::surface_control::InputRegion;
use anyhow::{Context, Result, ensure};
use solor::{Pixels, layout::Size, platform::WindowHandle, px};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
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
    parents: RefCell<HashMap<WindowHandle, Weak<Parent>>>,
    grabbing_root: RefCell<Weak<Menu>>,
}

struct Trigger {
    parent: WindowHandle,
    seat: WlSeat,
    serial: u32,
}

enum ParentRole {
    Panel(ZwlrLayerSurfaceV1, RefCell<Weak<Menu>>),
    Toplevel(XdgSurface, Weak<WindowLife>, RefCell<Weak<Menu>>),
    Menu(Rc<Menu>),
}

pub(super) struct Parent {
    menus: Weak<Menus>,
    handle: WindowHandle,
    role: ParentRole,
    pub(super) bounds: Cell<Option<[i32; 4]>>,
}

impl Parent {
    fn child(&self) -> &RefCell<Weak<Menu>> {
        match &self.role {
            ParentRole::Panel(_, child) | ParentRole::Toplevel(_, _, child) => child,
            ParentRole::Menu(menu) => &menu.child,
        }
    }

    pub(super) fn unmap(&self) {
        self.bounds.set(None);
        if let Some(child) = self.child().take().upgrade() {
            child.dismiss();
        }
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        self.unmap();
        if let Some(menus) = self.menus.upgrade() {
            menus.parents.borrow_mut().remove(&self.handle);
        }
    }
}

struct MenuShell(XdgWmBase);

impl Drop for MenuShell {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

impl Menus {
    pub(super) fn register(
        self: &Rc<Self>,
        role: &Role,
        handle: &WindowHandle,
        life: &Rc<WindowLife>,
    ) -> Option<Rc<Parent>> {
        let role = match role {
            Role::Layer(layer, _) => ParentRole::Panel(layer.clone(), Default::default()),
            Role::Menu(menu) => ParentRole::Menu(menu.clone()),
            Role::Toplevel { surface, .. } => {
                ParentRole::Toplevel(surface.clone(), Rc::downgrade(life), Default::default())
            }
            _ => return None,
        };
        let parent = Rc::new(Parent {
            menus: Rc::downgrade(self),
            handle: handle.clone(),
            role,
            bounds: Cell::new(None),
        });
        self.parents
            .borrow_mut()
            .insert(handle.clone(), Rc::downgrade(&parent));
        Some(parent)
    }

    pub(super) fn menu(&self, handle: &WindowHandle, mapped: bool) -> Result<Rc<Menu>> {
        let parent = self
            .parents
            .borrow()
            .get(handle)
            .and_then(Weak::upgrade)
            .filter(|_| handle.is_open())
            .context("popup is closed or belongs to another application")?;
        let ParentRole::Menu(menu) = &parent.role else {
            anyhow::bail!("window is not an anchored popup");
        };
        ensure!(
            !mapped || parent.bounds.get().is_some(),
            "popup has not been mapped"
        );
        Ok(menu.clone())
    }

    pub fn input(&self, window: &Window, seat: &WlSeat, serial: u32) {
        if let Some(parent) = &window.popup_parent {
            self.input.replace(Some(Trigger {
                parent: parent.handle.clone(),
                seat: seat.clone(),
                serial,
            }));
        }
    }

    pub fn end_input(&self) {
        self.input.take();
    }

    pub(super) fn dismiss_child(&self, handle: &WindowHandle) {
        if let Some(parent) = self.parents.borrow().get(handle).and_then(Weak::upgrade)
            && let Some(child) = parent.child().take().upgrade()
        {
            child.dismiss();
        }
    }

    pub fn dismiss_all(&self) {
        for parent in self.parents.borrow().values().filter_map(Weak::upgrade) {
            parent.unmap();
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
        let input = self.input.borrow();
        let parent_handle = options
            .parent
            .as_ref()
            .or_else(|| input.as_ref().map(|t| &t.parent))
            .context("a popup needs a parent window or a current pointer button event")?;
        let parent = self
            .parents
            .borrow()
            .get(parent_handle)
            .and_then(Weak::upgrade)
            .filter(|p| p.handle.is_open())
            .context("popup parent must be an open window in this application")?;
        let bounds = parent.bounds.get().context("popup parent is not mapped")?;
        if let ParentRole::Toplevel(_, life, _) = &parent.role {
            ensure!(
                life.upgrade().is_some_and(|life| !life.blocked()),
                "popup parent is blocked by a modal window"
            );
        }
        let grab = if options.grab {
            let trigger = input
                .as_ref()
                .filter(|t| t.parent == *parent_handle)
                .context("a popup grab requires a pointer button event on its parent")?;
            if let ParentRole::Menu(menu) = &parent.role {
                ensure!(
                    menu.seat.as_ref() == Some(&trigger.seat),
                    "a grabbing submenu needs a parent grab on the same seat"
                );
            }
            Some((trigger.seat.clone(), trigger.serial))
        } else {
            None
        };
        drop(input);
        ensure!(
            ConstraintAdjustment::all().contains(options.constraints),
            "unknown popup constraint adjustment"
        );
        let [w, h] = popup_size(options.size)?;
        let shell =
            match &parent.role {
                ParentRole::Panel(..) | ParentRole::Toplevel(..) => Rc::new(MenuShell(
                    globals.bind(qh, 1..=xdg_popup::REQ_REPOSITION_SINCE, ())?,
                )),
                ParentRole::Menu(menu) => menu.shell.clone(),
            };
        let positioner = positioner(&shell, qh, &options, bounds, [w, h])?;
        let xdg = shell.0.get_xdg_surface(surface, qh, surface.id());
        let grabbing_root = grab.is_some() && !matches!(parent.role, ParentRole::Menu(_));
        if grabbing_root && let Some(previous) = self.grabbing_root.take().upgrade() {
            previous.dismiss();
        }
        let slot = parent.child();
        if let Some(previous) = slot.take().upgrade() {
            previous.dismiss();
        }
        let popup = match &parent.role {
            ParentRole::Panel(layer, _) => {
                let popup = xdg.get_popup(None, &positioner, qh, surface.id());
                layer.get_popup(&popup);
                popup
            }
            ParentRole::Toplevel(parent, ..) => {
                xdg.get_popup(Some(parent), &positioner, qh, surface.id())
            }
            ParentRole::Menu(parent) => {
                xdg.get_popup(Some(&parent.xdg), &positioner, qh, surface.id())
            }
        };
        if let Some((seat, serial)) = &grab {
            self.input.take();
            popup.grab(seat, *serial);
        }
        positioner.destroy();
        let menu = Rc::new(Menu {
            child: Default::default(),
            seat: grab.map(|(seat, _)| seat),
            shell,
            xdg,
            popup,
            handle,
            dismissed: Cell::new(false),
            parent: Rc::downgrade(&parent),
            placement: options,
            requested_size: Cell::new([w, h]),
            token: Cell::new(0),
            reposition: Cell::new(Reposition::Idle),
            region: Default::default(),
        });
        slot.replace(Rc::downgrade(&menu));
        if grabbing_root {
            self.grabbing_root.replace(Rc::downgrade(&menu));
        }
        Ok((
            Role::Menu(menu),
            Size {
                width: px(w as f32),
                height: px(h as f32),
            },
        ))
    }
}

fn popup_size(size: Size) -> Result<[i32; 2]> {
    let w = crate::layer_shell::units(size.width)?;
    let h = crate::layer_shell::units(size.height)?;
    ensure!(
        w > 0 && h > 0,
        "popup size must round to positive logical pixels"
    );
    Ok([w, h])
}

fn positioner(
    shell: &MenuShell,
    qh: &QueueHandle<Desktop>,
    options: &PopupOptions,
    [ox, oy, pw, ph]: [i32; 4],
    [w, h]: [i32; 2],
) -> Result<XdgPositioner> {
    let (origin, size) = options.anchor_rect;
    let (x, width) = anchor_axis(origin.x, size.width, pw)?;
    let (y, height) = anchor_axis(origin.y, size.height, ph)?;
    let positioner = shell.0.create_positioner(qh, ());
    positioner.set_size(w, h);
    positioner.set_anchor_rect(ox + x, oy + y, width, height);
    positioner.set_anchor(options.anchor);
    positioner.set_gravity(options.gravity);
    positioner.set_constraint_adjustment(options.constraints);
    Ok(positioner)
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
    ensure!(end > start, "popup anchor is outside the parent window");
    Ok((start as i32, (end - start) as i32))
}

pub(super) struct Menu {
    child: RefCell<Weak<Menu>>,
    seat: Option<WlSeat>,
    shell: Rc<MenuShell>,
    xdg: XdgSurface,
    popup: XdgPopup,
    handle: WindowHandle,
    dismissed: Cell<bool>,
    parent: Weak<Parent>,
    placement: PopupOptions,
    requested_size: Cell<[i32; 2]>,
    token: Cell<u32>,
    reposition: Cell<Reposition>,
    pub(super) region: RefCell<Option<InputRegion>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Reposition {
    Idle,
    Waiting(u32),
    Configuring,
    Configured,
}

impl Menu {
    pub(super) fn resize(&self, size: Size, qh: &QueueHandle<Desktop>) -> Result<()> {
        if self.popup.version() < xdg_popup::REQ_REPOSITION_SINCE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "resizing a popup requires xdg-shell v3",
            )
            .into());
        }
        let size = popup_size(size)?;
        if self.requested_size.get() == size {
            return Ok(());
        }
        let bounds = self
            .parent
            .upgrade()
            .and_then(|p| p.bounds.get())
            .context("popup parent is not mapped")?;
        let positioner = positioner(&self.shell, qh, &self.placement, bounds, size)?;
        let token = self.token.get().wrapping_add(1);
        self.popup.reposition(&positioner, token);
        positioner.destroy();
        self.token.set(token);
        self.reposition.set(Reposition::Waiting(token));
        self.requested_size.set(size);
        Ok(())
    }

    pub(super) fn take_update(&self) -> (Option<InputRegion>, bool) {
        match self.reposition.get() {
            Reposition::Waiting(_) | Reposition::Configuring => (None, false),
            Reposition::Idle => (self.region.take(), false),
            Reposition::Configured => {
                self.reposition.set(Reposition::Idle);
                (self.region.take(), true)
            }
        }
    }

    pub(super) fn finish_configure(&self) {
        if self.reposition.get() == Reposition::Configuring {
            self.reposition.set(Reposition::Configured);
        }
    }

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
            xdg_popup::Event::Repositioned { token }
                if menu.reposition.get() == Reposition::Waiting(token) =>
            {
                menu.reposition.set(Reposition::Configuring);
            }
            xdg_popup::Event::PopupDone => menu.dismiss(),
            _ => {}
        }
    }
}
