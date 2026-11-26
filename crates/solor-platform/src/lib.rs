//! Selects the window system and rendering backend for mounted Solor views.
use anyhow::Result;
use solor::{Assets, Render, Ui};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod system;

pub use solor::platform::{Renderer, SurfaceRect, WindowBackground, WindowHandle, WindowOptions};
#[cfg(target_os = "linux")]
pub use solor_linux::{ApplicationSender, layer_shell, popup, session_lock};

fn default_renderer() -> Renderer {
    #[cfg(feature = "gpu")]
    if std::env::var_os("SOLOR_RENDERER").as_deref() == Some(std::ffi::OsStr::new("gpu")) {
        return Renderer::Gpu;
    }
    Renderer::Cpu
}

/// Hosts application windows with the window system and renderer selected for this target.
pub struct SystemPlatform {
    host: system::Host,
    renderer: Renderer,
    assets: Option<Assets>,
}

/// Selects the platform for this process. Returns an unsupported-platform error
/// on targets without a window implementation.
pub fn current_platform() -> Result<SystemPlatform> {
    Ok(SystemPlatform {
        renderer: default_renderer(),
        host: system::select()?,
        assets: None,
    })
}

/// A cloneable UI-thread handle for opening windows during setup or from event handlers.
/// Keeping a handle does not keep the event loop alive.
#[derive(Clone)]
pub struct Application {
    windows: system::Windows,
    renderer: Renderer,
    assets: Option<Assets>,
}

impl Application {
    /// Delivers messages on the UI thread. Each sender keeps the application running.
    #[cfg(target_os = "linux")]
    pub fn channel<T: Send + 'static>(
        &self,
        capacity: usize,
        mut receive: impl FnMut(&Application, T) + 'static,
    ) -> Result<ApplicationSender<T>> {
        let app = self.clone();
        self.windows
            .channel(capacity, move |_, msg| receive(&app, msg))
    }

    /// Mounts an independent view and opens a window on the current event loop.
    /// Returns an error if the application has exited or window creation fails.
    pub fn open_window(
        &self,
        options: WindowOptions<'_>,
        ui: Ui,
        root: impl Render,
    ) -> Result<WindowHandle> {
        self.open_window_with(options, ui, |_| root)
    }

    /// Builds the root with a handle to its own window, for use in UI callbacks.
    pub fn open_window_with<R: Render>(
        &self,
        options: WindowOptions<'_>,
        ui: Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<WindowHandle> {
        system::open_window(
            options.renderer.unwrap_or(self.renderer),
            &self.windows,
            options,
            configured_ui(ui, &self.assets),
            root,
        )
    }

    /// Builds one opaque lock view per output, using the host renderer and shared assets.
    /// The callback receives the Wayland output registry name and this session's lock handle.
    #[cfg(target_os = "linux")]
    pub fn lock_session<R: Render>(
        &self,
        mut build: impl FnMut(u32, &session_lock::SessionLock) -> Result<(Ui, R)> + 'static,
        notify: impl FnMut(session_lock::SessionLockState) + 'static,
    ) -> Result<session_lock::SessionLock> {
        let assets = self.assets.clone();
        system::lock_session(
            self.renderer,
            &self.windows,
            move |output, lock| {
                let (ui, root) = build(output, lock)?;
                Ok((configured_ui(ui, &assets), root))
            },
            notify,
        )
    }

    /// Opens an XDG popup, replacing the previous child of the same parent.
    /// A grabbing popup must open during a pointer button event on its parent.
    #[cfg(target_os = "linux")]
    pub fn open_popup<R: Render>(
        &self,
        options: popup::PopupOptions,
        ui: Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<WindowHandle> {
        system::open_popup(
            self.renderer,
            &self.windows,
            options,
            configured_ui(ui, &self.assets),
            root,
        )
    }

    /// Requests a new logical size for a mapped popup, keeping its anchor and edge adjustments.
    /// Requires xdg-shell v3. The compositor determines the final placement and size.
    #[cfg(target_os = "linux")]
    pub fn resize_popup(&self, popup: &WindowHandle, size: solor::layout::Size) -> Result<()> {
        self.windows.resize_popup(popup, size)
    }

    /// Sets the popup's input rectangles in surface-local logical pixels.
    /// `None` accepts the entire surface and an empty slice passes all input through.
    #[cfg(target_os = "linux")]
    pub fn set_popup_input_region(
        &self,
        popup: &WindowHandle,
        rects: Option<&[popup::InputRect]>,
    ) -> Result<()> {
        self.windows.set_popup_input_region(popup, rects)
    }

    /// Opens a Wayland layer surface alongside the application's other windows.
    /// The returned handle controls this surface from any thread.
    #[cfg(target_os = "linux")]
    pub fn open_layer_surface(
        &self,
        options: layer_shell::LayerShellOptions<'_>,
        ui: Ui,
        root: impl Render,
    ) -> Result<layer_shell::LayerSurfaceHandle> {
        self.open_layer_surface_with(options, ui, |_| root)
    }

    /// Builds the panel with its UI-thread window handle for anchoring native popups.
    #[cfg(target_os = "linux")]
    pub fn open_layer_surface_with<R: Render>(
        &self,
        options: layer_shell::LayerShellOptions<'_>,
        ui: Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<layer_shell::LayerSurfaceHandle> {
        let handle = match options.handle {
            Some(handle) => handle.clone(),
            None => layer_shell::LayerSurfaceHandle::new(true)?,
        };
        let options = layer_shell::LayerShellOptions {
            handle: Some(&handle),
            ..options
        };
        system::open_layer_surface(
            self.renderer,
            &self.windows,
            options,
            configured_ui(ui, &self.assets),
            root,
        )?;
        Ok(handle)
    }
}

fn configured_ui(ui: Ui, assets: &Option<Assets>) -> Ui {
    match assets {
        Some(assets) => ui.with_assets(assets.clone()),
        None => ui,
    }
}

impl SystemPlatform {
    /// Selects the default renderer for this host, overriding `SOLOR_RENDERER`.
    #[must_use]
    pub fn with_renderer(mut self, renderer: Renderer) -> Self {
        self.renderer = renderer;
        self
    }

    /// Supplies the shared resource pack for all windows and layer surfaces on this host.
    pub fn with_assets(mut self, source: impl Into<Assets>) -> Self {
        self.assets = Some(source.into());
        self
    }

    /// Calls the setup function once, then runs the windows together on the calling thread
    /// until all windows, application channels and session locks close.
    pub fn run(&self, setup: impl FnOnce(Application) -> Result<()>) -> Result<()> {
        self.host.run(|windows| {
            setup(Application {
                windows,
                renderer: self.renderer,
                assets: self.assets.clone(),
            })
        })
    }

    /// Mounts the root and hands input, frame scheduling and presentation to the host.
    /// On Linux this runs the event loop until the window closes.
    pub fn run_window(&self, options: WindowOptions<'_>, ui: Ui, root: impl Render) -> Result<()> {
        self.run(|app| app.open_window(options, ui, root).map(drop))
    }

    /// Runs a client-sized popup on the panel thread while the host dispatches its
    /// existing Wayland connection. The factory assigns the surface role before
    /// the first commit. Its return value releases the role on drop.
    #[cfg(target_os = "linux")]
    pub fn run_wayland_popup<R: 'static>(
        &self,
        connection: popup::Connection,
        control: &popup::PopupHandle,
        assign: impl FnOnce(&popup::WlSurface) -> Result<R>,
        ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        self.host.run_popup(
            self.renderer,
            connection,
            control,
            assign,
            configured_ui(ui, &self.assets),
            root,
        )
    }

    /// Mounts a Wayland desktop panel or overlay on the compositor-selected output.
    /// Returns an error if the requested layer-shell protocol is unavailable.
    #[cfg(target_os = "linux")]
    pub fn run_layer_surface(
        &self,
        options: layer_shell::LayerShellOptions<'_>,
        ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        self.run(|app| app.open_layer_surface(options, ui, root).map(drop))
    }
}
