use super::{
    DamageAudit, Mapping, Wayland, Window, WindowView,
    activation::{Activation, ActivationTarget},
    buffer::PixelOrder,
    floating,
    parent::Toplevels,
    pointer,
    role::{Request, Role},
};
use crate::{
    layer_shell::LayerShellOptions,
    popup::PopupHandle,
    surface_control::{Binding, Control, SurfaceRect, region_rectangles},
};
use anyhow::Context as _;
use rustix::event::PollFlags;
use solor::{
    platform::{
        PlatformWindow, WindowBackground, WindowHandle, WindowHandler, WindowOptions, WindowState,
        WindowSubscription,
    },
    raster::Rgba8,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io,
    rc::{Rc, Weak},
    time::Instant,
};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    backend::{ObjectId, WaylandError},
    globals::{GlobalList, registry_queue_init},
    protocol::{wl_compositor, wl_shm, wl_surface},
};
use wayland_protocols::wp::{
    cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
    fractional_scale::v1::client::wp_fractional_scale_manager_v1,
    presentation_time::client::wp_presentation, viewporter::client::wp_viewporter,
};

#[derive(Default)]
pub(super) struct Desktop {
    pub(super) menus: Rc<super::menu::Menus>,
    pub(super) locks: Rc<super::session_lock::Locks>,
    pub(super) effects: Option<super::background_effect::Manager>,
    pub(super) file_drags: HashMap<u32, super::clipboard::Drag>,
    pub(super) file_drop_hover: smallvec::SmallVec<[(ObjectId, solor::NodeId); 1]>,
    pub(super) file_drops: Vec<super::clipboard::PendingDrop>,
    pub(super) clipboard: Option<Rc<super::clipboard::Clipboard>>,
    pub(super) text_inputs: Option<super::text_input::Manager>,
    pub(super) activation: Option<Rc<Activation>>,
    pub(super) popup_only: bool,
    pub(super) monotonic_presentation: bool,
    pub(super) trace_submissions: bool,
    pub(super) windows: HashMap<ObjectId, Window>,
    pub(super) seats: Vec<pointer::Seat>,
    pub(super) outputs: Option<super::output::OutputScales>,
    pub(super) cursor_shapes: Option<WpCursorShapeManagerV1>,
    pub(super) rgb_opaque: bool,
    pub(super) rgb_transparent: bool,
}

impl Desktop {
    fn order(&self, background: WindowBackground) -> PixelOrder {
        match background {
            WindowBackground::Opaque if self.rgb_opaque => PixelOrder::Rgb,
            WindowBackground::Transparent if self.rgb_transparent => PixelOrder::Rgb,
            _ => PixelOrder::Bgr,
        }
    }
}

/// A UI-thread handle for opening windows on a running Wayland connection.
#[derive(Clone)]
pub struct WaylandApplication(Weak<WindowFactory>);

type WindowErrorHandler = dyn Fn(WindowHandle, &anyhow::Error);

impl WaylandApplication {
    /// Sets the UI-thread callback for failures that close an ordinary window.
    pub fn on_window_error(
        &self,
        handler: impl Fn(WindowHandle, &anyhow::Error) + 'static,
    ) -> anyhow::Result<()> {
        self.factory()?
            .error_handler
            .replace(Some(Rc::new(handler)));
        Ok(())
    }

    /// Delivers background messages on the UI thread, including while no windows are open.
    /// Dropping the last sender drains queued messages and releases the channel.
    pub fn channel<T: Send + 'static>(
        &self,
        capacity: usize,
        receive: impl FnMut(&WaylandApplication, T) + 'static,
    ) -> anyhow::Result<super::ApplicationSender<T>> {
        anyhow::ensure!(
            capacity > 0,
            "application channel requires a positive capacity"
        );
        let factory = self.factory()?;
        let (sender, inbox) = super::channel::channel(capacity, &factory.wake, receive);
        factory.inboxes.borrow_mut().push(inbox);
        Ok(sender)
    }

    /// Creates an XDG toplevel and schedules it for dispatch after the current callback.
    /// Returns an error if the application has exited or window creation fails.
    pub fn open_window(
        &self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<WindowHandle> {
        self.build_window(options, |_| Ok(view))
    }

    /// Builds the window's handler with a weak handle to its newly created toplevel.
    pub fn build_window(
        &self,
        options: WindowOptions<'_>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        self.factory()?
            .open(Request::Toplevel(options), None, build)
    }

    pub(super) fn factory(&self) -> anyhow::Result<Rc<WindowFactory>> {
        self.0.upgrade().context("application has exited")
    }

    /// Opens a layer surface on this event loop.
    pub fn open_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<WindowHandle> {
        self.build_layer_surface(options, |_| Ok(view))
    }

    /// Builds a layer surface with its UI-thread window handle.
    pub fn build_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        self.factory()?.open(
            Request::Layer(options),
            options.handle.map(|h| h.claim()).transpose()?,
            build,
        )
    }

    /// Opens an anchored XDG popup on a mapped toplevel, layer surface or popup.
    pub fn build_popup(
        &self,
        options: crate::popup::PopupOptions,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        self.factory()?.open(Request::Menu(options), None, build)
    }

    /// Requests a new logical size for a mapped anchored popup using xdg-shell v3.
    pub fn resize_popup(
        &self,
        popup: &WindowHandle,
        size: solor::layout::Size,
    ) -> anyhow::Result<()> {
        let factory = self.factory()?;
        factory.menus.menu(popup, true)?.resize(size, &factory.qh)
    }

    /// Sets the pointer and touch region of an anchored popup in surface-local logical pixels.
    /// Use `None` for the full surface or an empty slice to pass all input through.
    pub fn set_popup_input_region(
        &self,
        popup: &WindowHandle,
        rects: Option<&[crate::popup::InputRect]>,
    ) -> anyhow::Result<()> {
        let factory = self.factory()?;
        let menu = factory.menus.menu(popup, false)?;
        menu.region
            .replace(Some(crate::surface_control::InputRegion::from_rects(
                rects,
            )?));
        factory.revisit.set(true);
        Ok(())
    }

    /// Opens an anchored popup using the application's GPU device.
    #[cfg(feature = "gpu")]
    pub fn build_gpu_popup<R: solor::Render>(
        &self,
        options: crate::popup::PopupOptions,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> anyhow::Result<WindowHandle> {
        let background = options.background;
        self.build_gpu(Request::Menu(options), None, background, ui, root)
    }

    /// Mounts a Solor view on a native GPU surface for an XDG toplevel.
    #[cfg(feature = "gpu")]
    pub fn build_gpu_window<R: solor::Render>(
        &self,
        options: WindowOptions<'_>,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> anyhow::Result<WindowHandle> {
        self.build_gpu(
            Request::Toplevel(options),
            None,
            options.background,
            ui,
            root,
        )
    }

    /// Mounts a GPU view on a layer surface using the application's shared device.
    #[cfg(feature = "gpu")]
    pub fn open_gpu_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        ui: solor::Ui,
        root: impl solor::Render,
    ) -> anyhow::Result<WindowHandle> {
        self.build_gpu_layer_surface(options, ui, |_| root)
    }

    /// Builds a GPU layer surface with its UI-thread window handle.
    #[cfg(feature = "gpu")]
    pub fn build_gpu_layer_surface<R: solor::Render>(
        &self,
        options: LayerShellOptions<'_>,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> anyhow::Result<WindowHandle> {
        self.build_gpu(
            Request::Layer(options),
            options.handle.map(|h| h.claim()).transpose()?,
            options.background,
            ui,
            root,
        )
    }

    #[cfg(feature = "gpu")]
    pub(super) fn build_gpu<R: solor::Render>(
        &self,
        request: Request<'_>,
        control: Option<Binding>,
        background: WindowBackground,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> anyhow::Result<WindowHandle> {
        let windows = self.factory()?;
        windows.open_with_surface(request, control, |handle, surface| {
            let presenter = super::gpu::Presenter::new(
                &mut windows.gpu.borrow_mut(),
                surface,
                background,
                windows.trace_submissions,
            )?;
            let max_dimension = presenter.max_dimension();
            Ok(WindowView::Gpu(
                Box::new(solor::platform::UiWindow::with_renderer(
                    ui,
                    root(handle),
                    presenter,
                )?),
                max_dimension,
            ))
        })
    }
}

enum BlurRequests {
    Local(Cell<Option<Vec<[i32; 4]>>>),
    Shared(Control),
}

type Listener<T> = RefCell<Box<dyn FnMut(T)>>;
type ListenerList<T> = RefCell<Vec<Rc<Listener<T>>>>;

#[derive(Default)]
struct WindowListeners<T>(std::cell::OnceCell<Rc<ListenerList<T>>>);

impl<T: Copy + 'static> WindowListeners<T> {
    fn observe(&self, listener: Box<dyn FnMut(T)>) -> WindowSubscription {
        let listeners = self.0.get_or_init(Rc::default);
        let callback = Rc::new(RefCell::new(listener));
        let target = Rc::downgrade(&callback);
        listeners.borrow_mut().push(callback);
        let list = Rc::downgrade(listeners);
        WindowSubscription::new(move || {
            if let Some(list) = list.upgrade() {
                let removed = {
                    let mut entries = list.borrow_mut();
                    entries
                        .iter()
                        .position(|entry| std::ptr::eq(Rc::as_ptr(entry), target.as_ptr()))
                        .map(|index| entries.remove(index))
                };
                drop(removed);
            }
        })
    }

    fn notify(&self, value: T) {
        if let Some(list) = self.0.get() {
            let pending: smallvec::SmallVec<[_; 4]> =
                list.borrow().iter().map(Rc::downgrade).collect();
            for entry in pending {
                if let Some(listener) = entry.upgrade() {
                    listener.borrow_mut()(value);
                }
            }
        }
    }

    fn clear(&self) {
        if let Some(list) = self.0.get() {
            drop(list.take());
        }
    }
}

pub(super) struct WindowLife {
    blur: BlurRequests,
    pub(super) activation: Option<ActivationTarget>,
    open: Cell<bool>,
    should_close: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    state: Cell<Option<WindowState>>,
    state_listeners: WindowListeners<WindowState>,
    client_decorations: Cell<bool>,
    decoration_listeners: WindowListeners<bool>,
    modal_parent: Weak<Self>,
    modal_children: RefCell<Vec<Weak<Self>>>,
    cancel_pointer: Cell<bool>,
    pub(super) revisit: Rc<Cell<bool>>,
}

impl WindowLife {
    fn new(
        activation: Option<ActivationTarget>,
        revisit: Rc<Cell<bool>>,
        modal_parent: Weak<Self>,
        control: Option<&Binding>,
    ) -> Rc<Self> {
        Rc::new(Self {
            blur: match control {
                Some(binding) => BlurRequests::Shared(binding.control()),
                None => BlurRequests::Local(Cell::default()),
            },
            activation,
            open: Cell::new(true),
            should_close: Default::default(),
            state: Default::default(),
            state_listeners: Default::default(),
            client_decorations: Cell::new(false),
            decoration_listeners: Default::default(),
            revisit,
            modal_parent,
            modal_children: RefCell::default(),
            cancel_pointer: Cell::default(),
        })
    }

    pub(super) fn request_close(&self) {
        if self.is_open() && !self.blocked() {
            let handler = self.should_close.borrow().clone();
            if handler.is_none_or(|allow| allow()) {
                self.close();
            }
        }
    }

    pub(super) fn confirm_state(&self, state: WindowState) -> bool {
        if self.state.replace(Some(state)) == Some(state) {
            return false;
        }
        self.state_listeners.notify(state);
        true
    }

    pub(super) fn set_client_decorations(&self, enabled: bool) -> bool {
        if self.client_decorations.replace(enabled) == enabled {
            return false;
        }
        self.decoration_listeners.notify(enabled);
        self.revisit.set(true);
        true
    }

    fn handle(self: &Rc<Self>) -> WindowHandle {
        let target = Rc::downgrade(self);
        WindowHandle::new(target)
    }

    fn attach_modal(self: &Rc<Self>, parent: &Self) {
        if !parent.is_open() {
            self.close();
        } else if self.is_open() {
            let mut children = parent.modal_children.borrow_mut();
            if children.is_empty() {
                parent.cancel_pointer.set(true);
            }
            children.push(Rc::downgrade(self));
            if let Some(source) = &parent.activation {
                let modal = self.modal_activation_window();
                if let Some(target) = &modal.as_deref().unwrap_or(self).activation {
                    source.redirect_pending(target);
                }
            }
        }
    }

    /// Follow the newest open modal at each level to a window that accepts input.
    fn modal_activation_window(&self) -> Option<Rc<Self>> {
        let mut window = self.modal_children.borrow().last()?.upgrade()?;
        loop {
            let next = window
                .modal_children
                .borrow()
                .last()
                .and_then(Weak::upgrade);
            match next {
                Some(child) => window = child,
                None => return Some(window),
            }
        }
    }

    pub(super) fn blocked(&self) -> bool {
        !self.modal_children.borrow().is_empty()
    }

    pub(super) fn take_pointer_cancel(&self) -> bool {
        self.cancel_pointer.replace(false)
    }
}

impl PlatformWindow for WindowLife {
    fn set_close_guard(&self, handler: Rc<dyn Fn() -> bool>) {
        self.should_close.replace(Some(handler));
    }

    fn state(&self) -> Option<WindowState> {
        self.state.get()
    }
    fn observe_state(&self, listener: Box<dyn FnMut(WindowState)>) -> WindowSubscription {
        self.state_listeners.observe(listener)
    }

    fn is_open(&self) -> bool {
        self.open.get()
    }

    fn set_background_blur_region(&self, rects: &[SurfaceRect]) -> io::Result<()> {
        match &self.blur {
            BlurRequests::Shared(control) => control.set_blur(rects),
            BlurRequests::Local(pending) => {
                pending.set(Some(region_rectangles(rects)?));
                self.revisit.set(true);
                Ok(())
            }
        }
    }

    fn client_decorations(&self) -> bool {
        self.client_decorations.get()
    }

    fn observe_client_decorations(&self, listener: Box<dyn FnMut(bool)>) -> WindowSubscription {
        self.decoration_listeners.observe(listener)
    }

    fn activate(&self) {
        let modal = self.modal_activation_window();
        if let Some(target) = &modal.as_deref().unwrap_or(self).activation {
            target.request();
        }
    }

    fn close(&self) {
        if self.open.replace(false) {
            self.state_listeners.clear();
            self.decoration_listeners.clear();
            if let Some(target) = &self.activation {
                target.cancel();
            }

            if let Some(parent) = self.modal_parent.upgrade() {
                parent
                    .modal_children
                    .borrow_mut()
                    .retain(|child| !std::ptr::eq(child.as_ptr(), self));
            }
            let children = self.modal_children.take();
            for child in children.into_iter().filter_map(|child| child.upgrade()) {
                child.close();
            }
            self.revisit.set(true);
        }
    }
}

impl Drop for WindowLife {
    fn drop(&mut self) {
        self.close();
    }
}

struct EventLoop {
    windows: Rc<WindowFactory>,
    connection: Connection,
    queue: EventQueue<Desktop>,
    desktop: Desktop,
}

pub(super) struct WindowFactory {
    error_handler: RefCell<Option<Rc<WindowErrorHandler>>>,
    pub(super) locks: Rc<super::session_lock::Locks>,
    wake: std::sync::Arc<super::wake::EventWake>,
    inboxes: RefCell<Vec<super::channel::Inbox>>,
    menus: Rc<super::menu::Menus>,
    #[cfg(feature = "gpu")]
    gpu: RefCell<super::gpu::GpuContext>,
    clipboard: Option<Rc<super::clipboard::Clipboard>>,
    toplevels: Toplevels,
    activation: Option<Rc<Activation>>,
    pub(super) qh: QueueHandle<Desktop>,
    pending: RefCell<Vec<Window>>,
    pub(super) revisit: Rc<Cell<bool>>,
    pub(super) globals: Option<GlobalList>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    pub(super) viewporter: Option<wp_viewporter::WpViewporter>,
    scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    presentation: Option<wp_presentation::WpPresentation>,
    verify_shm: bool,
    trace_submissions: bool,
}

impl Wayland {
    /// Opens the initial windows, then dispatches until all windows, channels and session locks close.
    pub fn run(
        &self,
        setup: impl FnOnce(WaylandApplication) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut events = EventLoop::new(self, Connection::connect_to_env()?, false)?;
        setup(WaylandApplication(Rc::downgrade(&events.windows)))?;
        events.dispatch()
    }

    /// Runs a desktop layer surface until the compositor or the application closes it.
    pub fn run_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        self.run(|app| app.open_layer_surface(options, view).map(drop))
    }

    /// Runs a transparent, client-sized popup on the supplied connection. The
    /// host keeps reading the connection on its own thread. The factory assigns
    /// a role that can present without a configure handshake; its return value
    /// owns that role and must destroy it on drop.
    pub fn run_popup<R: 'static>(
        &self,
        connection: Connection,
        handle: &PopupHandle,
        assign: impl FnOnce(&wl_surface::WlSurface) -> anyhow::Result<R>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> anyhow::Result<()> {
        self.run_popup_with(connection, handle, assign, |app, request, control| {
            app.factory()?.open(request, Some(control), |_| Ok(view))
        })
    }

    /// Runs a GPU-rendered transparent popup on the supplied host connection.
    #[cfg(feature = "gpu")]
    pub fn run_gpu_popup<R: 'static>(
        &self,
        connection: Connection,
        handle: &PopupHandle,
        assign: impl FnOnce(&wl_surface::WlSurface) -> anyhow::Result<R>,
        ui: solor::Ui,
        root: impl solor::Render,
    ) -> anyhow::Result<()> {
        self.run_popup_with(connection, handle, assign, |app, request, control| {
            app.build_gpu(
                request,
                Some(control),
                WindowBackground::Transparent,
                ui,
                |_| root,
            )
        })
    }

    fn run_popup_with<'a, R: 'static>(
        &self,
        connection: Connection,
        handle: &PopupHandle,
        assign: impl FnOnce(&wl_surface::WlSurface) -> anyhow::Result<R> + 'a,
        open: impl FnOnce(&WaylandApplication, Request<'a>, Binding) -> anyhow::Result<WindowHandle>,
    ) -> anyhow::Result<()> {
        let control = handle.0.claim()?;
        let mut events = EventLoop::new(self, connection, true)?;
        let request = Request::Popup(Box::new(move |surface| Ok(Box::new(assign(surface)?))));
        open(
            &WaylandApplication(Rc::downgrade(&events.windows)),
            request,
            control,
        )?;
        events.dispatch()
    }
}

impl EventLoop {
    fn new(host: &Wayland, connection: Connection, guest: bool) -> anyhow::Result<Self> {
        let (globals, queue) = registry_queue_init::<Desktop>(&connection)?;
        let qh = queue.handle();
        let viewporter = globals.bind(&qh, 1..=1, ()).ok();
        let scale_manager = viewporter
            .as_ref()
            .and_then(|_| globals.bind(&qh, 1..=1, ()).ok());
        let mut desktop = Desktop {
            outputs: scale_manager.is_none().then(Default::default),
            popup_only: guest,
            trace_submissions: host.trace_submissions,
            clipboard: if guest {
                None
            } else {
                super::clipboard::Clipboard::bind(&globals, &qh)
            },
            text_inputs: if guest {
                None
            } else {
                globals.bind(&qh, 1..=1, ()).ok()
            },
            activation: if guest {
                None
            } else {
                Activation::bind(&globals, &qh)
            },
            cursor_shapes: globals.bind(&qh, 1..=1, ()).ok(),
            ..Desktop::default()
        };
        for global in globals.contents().clone_list() {
            if global.interface == "wl_output"
                && let Some(outputs) = &mut desktop.outputs
            {
                outputs.bind(globals.registry(), global.name, global.version, &qh);
            }
            desktop.bind_background_effects(
                globals.registry(),
                global.name,
                &global.interface,
                &qh,
            );
            if global.interface == "wl_seat" {
                desktop.add_seat(globals.registry(), global.name, global.version, &qh);
            }
        }
        let windows = WindowFactory {
            error_handler: Default::default(),
            locks: desktop.locks.clone(),
            wake: super::wake::EventWake::new()?,
            inboxes: Default::default(),
            menus: desktop.menus.clone(),
            #[cfg(feature = "gpu")]
            gpu: RefCell::new(super::gpu::GpuContext::new(host.gpu_memory_policy)),
            clipboard: desktop.clipboard.clone(),
            toplevels: Toplevels::default(),
            activation: desktop.activation.clone(),
            compositor: globals.bind(&qh, 4..=4, ())?,
            shm: globals.bind(&qh, 1..=1, ())?,
            viewporter,
            scale_manager,
            presentation: globals.bind(&qh, 1..=1, ()).ok(),
            trace_submissions: host.trace_submissions,
            verify_shm: host.verify_shm,
            qh,
            globals: Some(globals),
            pending: RefCell::default(),
            revisit: Rc::default(),
        };
        Ok(Self {
            windows: Rc::new(windows),
            connection,
            queue,
            desktop,
        })
    }
}

impl WindowFactory {
    fn fail_window(
        &self,
        window: &Window,
        error: anyhow::Error,
        first_error: &mut Option<anyhow::Error>,
    ) -> anyhow::Result<()> {
        let error = error.context(format!("window {}", window.surface.id()));
        if matches!(window.role, Role::Lock { .. }) {
            return Err(error);
        }
        window.life.close();
        log::error!("{error:#}");
        let handler = self.error_handler.borrow().clone();
        if let Some(handler) = handler {
            handler(window.life.handle(), &error);
        }
        first_error.get_or_insert(error);
        Ok(())
    }

    pub(super) fn open(
        &self,
        request: Request<'_>,
        control: Option<Binding>,
        build: impl FnOnce(WindowHandle) -> anyhow::Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> anyhow::Result<WindowHandle> {
        self.open_with_surface(request, control, |h, _| build(h).map(WindowView::Shm))
    }

    fn open_with_surface(
        &self,
        request: Request<'_>,
        control: Option<Binding>,
        build: impl FnOnce(WindowHandle, &wl_surface::WlSurface) -> anyhow::Result<WindowView>,
    ) -> anyhow::Result<WindowHandle> {
        let modal_parent = match &request {
            Request::Toplevel(options) => {
                anyhow::ensure!(
                    !options.modal || options.parent.is_some(),
                    "a modal window requires a parent"
                );
                options
                    .parent
                    .map(|handle| {
                        self.toplevels
                            .parent_life(handle)
                            .map(|life| (handle, life))
                    })
                    .transpose()?
                    .filter(|_| options.modal)
            }
            Request::Layer(_) | Request::Popup(_) | Request::Menu(_) | Request::Lock(..) => None,
        };
        let surface = self.compositor.create_surface(&self.qh, ());
        match self.create_window(
            &surface,
            request,
            control,
            modal_parent
                .as_ref()
                .map(|(_, life)| Rc::downgrade(life))
                .unwrap_or_default(),
            build,
        ) {
            Ok((window, handle)) => {
                if let Some((parent_handle, parent)) = modal_parent {
                    window.life.attach_modal(&parent);
                    if window.life.is_open() {
                        self.menus.dismiss_child(parent_handle);
                    }
                }
                self.pending.borrow_mut().push(window);
                Ok(handle)
            }
            Err(error) => {
                surface.destroy();
                Err(error)
            }
        }
    }

    fn create_window(
        &self,
        surface: &wl_surface::WlSurface,
        request: Request<'_>,
        control: Option<Binding>,
        modal_parent: Weak<WindowLife>,
        build: impl FnOnce(WindowHandle, &wl_surface::WlSurface) -> anyhow::Result<WindowView>,
    ) -> anyhow::Result<(Window, WindowHandle)> {
        let qh = &self.qh;
        let background = match &request {
            Request::Menu(options) => options.background,
            Request::Popup(_) => WindowBackground::Transparent,
            Request::Lock(..) => WindowBackground::Opaque,
            Request::Layer(options) => options.background,
            Request::Toplevel(options) => options.background,
        };
        let parent = match &request {
            Request::Toplevel(options) => options.parent,
            Request::Layer(_) | Request::Popup(_) | Request::Menu(_) | Request::Lock(..) => None,
        };
        let activation = match (&request, &self.activation) {
            (Request::Toplevel(options), Some(activation)) => Some(ActivationTarget {
                activation: activation.clone(),
                id: surface.id(),
                app_id: options.app_id.map(str::to_owned),
            }),
            _ => None,
        };
        let globals = self.globals.as_ref().expect("live window factory");
        let life = WindowLife::new(
            activation,
            self.revisit.clone(),
            modal_parent,
            control.as_ref(),
        );
        let handle = life.handle();
        let mut input_region = super::input_region::SurfaceInputRegion::default();
        if matches!(&request, Request::Menu(options) if options.input_transparent) {
            input_region.update(Some(crate::surface_control::InputRegion::Rects(Vec::new())));
        }
        let requested_csd = match &request {
            Request::Toplevel(options) => !options.native_decorations,
            _ => false,
        };
        let (role, size) = request.create(globals, qh, surface, &self.menus, handle.clone())?;
        if matches!(
            &role,
            Role::Toplevel {
                decorations: None,
                ..
            }
        ) {
            life.set_client_decorations(requested_csd);
        }
        if let (Some(control), Role::Layer(layer, placement)) = (&control, &role) {
            control.set_layer_rules(crate::layer_shell::LayerRules {
                anchor: placement.anchor,
                movable: placement.movable,
                version: layer.version(),
            })?;
        }
        let registration = match &role {
            Role::Toplevel { top, .. } => {
                Some(self.toplevels.register(handle.clone(), &life, top, parent))
            }
            Role::Layer(..) | Role::Popup { .. } | Role::Menu(_) | Role::Lock { .. } => None,
        };
        let floating = match &role {
            Role::Layer(_, p) if p.movable => Some(floating::FloatingPanel::new(
                globals,
                &self.compositor,
                &self.shm,
                self.viewporter
                    .as_ref()
                    .context("movable layer surfaces require wp_viewporter")?,
                surface,
                floating::Geometry::new(size, p.anchor, p.margin),
                qh,
            )?),
            _ => None,
        };
        let content = floating.as_ref().map_or(surface, |p| &p.content);
        let popup_parent = self.menus.register(&role, &handle, &life);
        let mut view = build(handle.clone(), content)?;
        let buffer_size = view
            .buffer_size(120, size)?
            .context("initial size exceeds render target limits")?;
        if let Some(hub) = &self.clipboard {
            view.set_clipboard(hub.window(surface.id()));
        }
        view.set_window_resizable(matches!(
            &role,
            Role::Toplevel {
                resizable: true,
                ..
            }
        ))?;
        let viewport = self
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(content, qh, ()));

        let mapping = if matches!(role, Role::Lock { .. }) {
            Mapping::AwaitingConfigure
        } else {
            Mapping::Hidden
        };
        let window = Window {
            popup_parent,
            control,
            preedit_owner: None,
            role,
            floating,
            pending_state: None,
            presentation: self.presentation.clone(),
            frame_clock: Default::default(),
            fractional: self
                .scale_manager
                .as_ref()
                .map(|m| m.get_fractional_scale(surface, qh, surface.id())),
            entered_outputs: Default::default(),
            viewport,
            scale_120: 120,
            buffer_size: Some(buffer_size),
            surface: surface.clone(),
            shm: self.shm.clone(),
            background,
            pixel_order: PixelOrder::Bgr,
            slots: [None, None],
            next_slot: 0,
            view,
            full_damage: true,
            failure: None,
            trace: self.trace_submissions,
            size,
            pending_size: None,
            mapping,
            input_region,
            blur: None,
            dirty: true,
            #[cfg(feature = "gpu")]
            acquire_retry: None,
            waiting: None,
            life,
            registration,
            commits: 0,
            audit: self.verify_shm.then(DamageAudit::default),
            start: Instant::now(),
        };
        Ok((window, handle))
    }
}

impl EventLoop {
    fn dispatch(&mut self) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        let wake = &self.windows.wake;
        let app = WaylandApplication(Rc::downgrade(&self.windows));
        let mut inboxes = Vec::new();
        let mut first_window_error = None;
        let waker = std::task::Waker::from(wake.clone());
        loop {
            self.windows.revisit.set(false);
            inboxes.append(&mut self.windows.inboxes.borrow_mut());
            let mut more = false;
            inboxes.retain_mut(|receive| match receive(&app) {
                super::channel::Poll::More => {
                    more = true;
                    true
                }
                super::channel::Poll::Idle => true,
                super::channel::Poll::Closed => false,
            });
            if let Some(globals) = &self.windows.globals {
                self.windows.locks.sync_outputs(globals);
            }
            for mut window in self.windows.pending.take() {
                window.pixel_order = self.desktop.order(window.background);
                self.desktop.windows.insert(window.surface.id(), window);
            }
            if self.desktop.popup_only {
                let mut cx = std::task::Context::from_waker(&waker);
                if let std::task::Poll::Ready(result) =
                    self.queue.poll_dispatch_pending(&mut cx, &mut self.desktop)
                {
                    match result? {}
                }
            }
            self.desktop.deliver_file_drops();
            let mut deadline = more.then(Instant::now);
            let mut closed = Vec::new();
            for (id, window) in &mut self.desktop.windows {
                if window.failure.is_none()
                    && window.life.is_open()
                    && let Err(error) = window.view.poll_tasks(&waker)
                {
                    window.record_failure(error.context("polling foreground tasks"));
                }
                if let Some(error) = window.failure.take() {
                    self.windows
                        .fail_window(window, error, &mut first_window_error)?;
                }
                let update = window.control.as_ref().map(Binding::take);
                let Some(visible) = update
                    .as_ref()
                    .map_or(Some(true), |u| u.visible)
                    .filter(|_| window.life.is_open())
                else {
                    if let Some(parent) = &window.popup_parent {
                        parent.unmap();
                    }
                    closed.push(id.clone());
                    continue;
                };
                pointer::cancel_modal_gestures(&mut self.desktop.seats, id, window);
                if let Some(update) = update {
                    if let Some(size) = update.size {
                        window.resize(size);
                    }
                    window.input_region.update(update.region);
                    if let Some(region) = update.blur {
                        window.set_blur(region);
                    }
                    window.update_layer(update.layer, visible);
                }
                if let BlurRequests::Local(pending) = &window.life.blur
                    && let Some(region) = pending.take()
                {
                    window.set_blur(region);
                }
                let mut commit_state = false;
                if let Role::Menu(menu) = &window.role {
                    let (region, repositioned) = menu.take_update();
                    window.input_region.update(region);
                    if window.mapping == Mapping::Mapped && !window.full_damage {
                        commit_state = window.apply_input_region(&self.windows.compositor, &qh)
                            || repositioned;
                    }
                }
                if visible
                    && !self.desktop.popup_only
                    && (window.control.is_some() || window.floating.is_some())
                {
                    commit_state |= window.apply_input_region(&self.windows.compositor, &qh);
                }
                if !visible && window.mapping == Mapping::Mapped {
                    if let Some(parent) = &window.popup_parent {
                        parent.unmap();
                    }
                    pointer::cancel_focus(&mut self.desktop.seats, id, window);
                }
                if window.mapping == Mapping::Hidden
                    && let Some(registration) = &mut window.registration
                    && !registration.prepare_map()
                {
                    continue;
                }
                window.set_visible(visible, &self.connection, &qh);
                if visible {
                    commit_state |= window.apply_blur(
                        self.desktop.effects.as_ref(),
                        &self.windows.compositor,
                        &qh,
                    );
                    if commit_state
                        && window.mapping == Mapping::Mapped
                        && (!window.full_damage || matches!(window.role, Role::Layer(..)))
                    {
                        window.surface.commit();
                    }
                    if let Some(next) = super::keyboard::repeat_keys(
                        &mut self.desktop.seats,
                        id,
                        window,
                        Instant::now(),
                    ) {
                        deadline = Some(deadline.map_or(next, |d: Instant| d.min(next)));
                    }
                    if self.desktop.popup_only && window.input_region.changed() {
                        window.dirty = true;
                    }
                    match window.present_if_due(&self.windows.compositor, &qh) {
                        Ok(Some(next)) => deadline = Some(deadline.map_or(next, |d| d.min(next))),
                        Ok(None) => {}
                        Err(error) => {
                            self.windows.fail_window(
                                window,
                                error.context("presenting frame"),
                                &mut first_window_error,
                            )?;
                        }
                    }
                }
            }
            for id in closed {
                if let Some(mut window) = self.desktop.windows.remove(&id) {
                    pointer::cancel_focus(&mut self.desktop.seats, &id, &mut window);
                    window.destroy();
                }
            }
            self.desktop.sync_cursors(&self.windows.compositor, &qh);
            self.desktop.sync_text_inputs();
            // A newly mapped parent or a callback can make another window ready.
            if self.desktop.sync_file_drop_hover()
                || self.windows.revisit.get()
                || !self.windows.pending.borrow().is_empty()
                || !self.windows.inboxes.borrow().is_empty()
            {
                continue;
            }
            let write_interest = flush_interest(&self.queue)?;
            if self.desktop.windows.is_empty() && inboxes.is_empty() && !self.windows.locks.active()
            {
                return first_window_error.map_or(Ok(()), Err);
            }
            dispatch_until(
                &mut self.queue,
                &mut self.desktop,
                write_interest,
                deadline,
                wake,
            )?;
        }
    }
}

impl Drop for WindowFactory {
    fn drop(&mut self) {
        if let Some(m) = self.scale_manager.take() {
            m.destroy();
        }
        if let Some(v) = self.viewporter.take() {
            v.destroy();
        }
        if let Some(presentation) = self.presentation.take() {
            presentation.destroy();
        }
        let registry = self.globals.take().expect("live window factory");
        let proxy = registry.registry().clone();
        registry.destroy();
        if let Some(backend) = proxy.backend().upgrade() {
            // These bound protocol versions have no destructor request.
            for id in [self.compositor.id(), self.shm.id()] {
                let _ = backend.destroy_object(&id);
            }
            if proxy.is_alive() {
                let _ = backend.destroy_object(&proxy.id());
            }
            let _ = backend.flush();
        }
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        self.desktop.menus.dismiss_all();
        for (_, window) in self.desktop.windows.drain() {
            window.destroy();
        }
        for window in self.windows.pending.take() {
            window.destroy();
        }
        self.desktop.seats.clear();
        if let Some(manager) = self.desktop.cursor_shapes.take() {
            manager.destroy();
        }
    }
}

impl Window {
    fn present_if_due(
        &mut self,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<Desktop>,
    ) -> anyhow::Result<Option<Instant>> {
        if self.failure.is_some() {
            return Ok(None);
        }
        let Some(size) = self.buffer_size else {
            return Ok(None);
        };
        #[cfg(feature = "gpu")]
        if let Some(deadline) = self.acquire_retry.take()
            && Instant::now() < deadline
        {
            self.acquire_retry = Some(deadline);
            return Ok(Some(deadline));
        }
        let next = self.deadline();
        if next.is_some_and(|deadline| deadline <= Instant::now()) {
            self.dirty = true;
        }
        if self.dirty {
            self.present(size, compositor, qh)?;
            return Ok(self.deadline());
        }
        Ok(next)
    }

    fn deadline(&self) -> Option<Instant> {
        #[cfg(feature = "gpu")]
        if let Some(deadline) = self.acquire_retry {
            return Some(deadline);
        }
        if self.mapping == Mapping::Mapped && !self.dirty && self.waiting.is_none() {
            self.view.next_redraw()
        } else {
            None
        }
    }

    fn destroy(self) {
        drop(self.blur);
        drop(self.view);
        drop(self.popup_parent);
        drop(self.role);
        if let Some(f) = self.fractional {
            f.destroy();
        }
        if let Some(v) = self.viewport {
            v.destroy();
        }
        drop(self.floating);
        self.surface.destroy();
        if self.trace {
            eprintln!("closed commits={}", self.commits);
        }
    }
}

fn flush_interest(queue: &EventQueue<Desktop>) -> Result<PollFlags, WaylandError> {
    match queue.flush() {
        Ok(()) => Ok(PollFlags::empty()),
        Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => Ok(PollFlags::OUT),
        Err(error) => Err(error),
    }
}

// A pending hint can wake an otherwise idle window. Input queued before the
// deadline gets the opportunity to cancel or replace it before presentation.
fn dispatch_until(
    queue: &mut EventQueue<Desktop>,
    desktop: &mut Desktop,
    write_interest: PollFlags,
    deadline: Option<Instant>,
    wake: &super::wake::EventWake,
) -> anyhow::Result<()> {
    if desktop.popup_only {
        wait_for_events(queue, write_interest, deadline, wake, desktop)?;
        return Ok(());
    }
    if queue.dispatch_pending(desktop)? != 0 {
        return Ok(());
    }
    if let Some(guard) = queue.prepare_read()
        && wait_for_events(
            queue,
            PollFlags::IN | write_interest,
            deadline,
            wake,
            desktop,
        )?
    {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
    }
    queue.dispatch_pending(desktop)?;
    Ok(())
}

fn wait_for_events(
    queue: &EventQueue<Desktop>,
    interest: PollFlags,
    deadline: Option<Instant>,
    wake: &super::wake::EventWake,
    desktop: &Desktop,
) -> anyhow::Result<bool> {
    use rustix::event::{PollFd, Timespec, poll};
    let mut transfers = desktop.clipboard.as_ref().map(|c| c.transfers.borrow_mut());
    if let Some(transfers) = &mut transfers {
        transfers.discard_cancelled();
    }
    let mut fds: smallvec::SmallVec<[_; 3]> = smallvec::smallvec![
        PollFd::new(queue, interest),
        PollFd::new(&wake.0, PollFlags::IN)
    ];
    let controls = || {
        desktop
            .windows
            .values()
            .filter_map(|window| window.control.as_ref())
    };
    fds.extend(controls().map(|c| PollFd::new(c.fd(), PollFlags::IN)));
    let clipboard_start = fds.len();
    if let Some(transfers) = &transfers {
        fds.extend(transfers.poll_fds());
    }
    let timeout = deadline
        .map(|t| Timespec::try_from(t.saturating_duration_since(Instant::now())))
        .transpose()?;
    match poll(&mut fds, timeout.as_ref()) {
        Ok(0) | Err(rustix::io::Errno::INTR) => return Ok(false),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    if !fds[1].revents().is_empty() {
        wake.drain()?;
    }
    for (control, fd) in controls().zip(&fds[2..clipboard_start]) {
        if !fd.revents().is_empty() {
            control.drain()?;
        }
    }

    let ready = fds[0].revents();
    let clipboard_ready: smallvec::SmallVec<[usize; 4]> = fds[clipboard_start..]
        .iter()
        .enumerate()
        .filter_map(|(i, fd)| (!fd.revents().is_empty()).then_some(i))
        .collect();
    drop(fds);
    if let Some(transfers) = &mut transfers {
        transfers.progress(&clipboard_ready);
    }
    anyhow::ensure!(
        !ready.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL),
        "Wayland connection closed"
    );
    if ready.contains(PollFlags::OUT) {
        flush_interest(queue)?;
    }
    Ok(ready.contains(PollFlags::IN))
}

#[cfg(test)]
#[path = "../../tests/unit/window_handle.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/dispatch.rs"]
mod dispatch_tests;

#[cfg(test)]
#[path = "../../tests/unit/host.rs"]
pub(super) mod fixture;
