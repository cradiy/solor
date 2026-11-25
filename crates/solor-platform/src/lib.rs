//! Selects the window system and rendering backend for mounted Solor views.
use anyhow::Result;
use solor::{Assets, Render, Ui};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod system;

pub use solor::platform::{WindowBackground, WindowHandle, WindowOptions};
#[cfg(target_os = "linux")]
pub use solor_linux::{layer_shell, popup};

/// Hosts application windows with the window system and renderer selected for this target.
pub struct SystemPlatform {
    host: system::Host,
    assets: Option<Assets>,
}

/// Selects the platform for this process. Returns an unsupported-platform error
/// on targets without a window implementation.
pub fn current_platform() -> Result<SystemPlatform> {
    Ok(SystemPlatform {
        host: system::select()?,
        assets: None,
    })
}

/// A cloneable UI-thread handle for opening windows during setup or from event handlers.
/// Keeping a handle does not keep the event loop alive.
#[derive(Clone)]
pub struct Application {
    windows: system::Windows,
    assets: Option<Assets>,
}

impl Application {
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
            &self.windows,
            options,
            configured_ui(ui, &self.assets),
            root,
        )
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
        let handle = match options.handle {
            Some(handle) => handle.clone(),
            None => layer_shell::LayerSurfaceHandle::new(true)?,
        };
        let options = layer_shell::LayerShellOptions {
            handle: Some(&handle),
            ..options
        };
        system::open_layer_surface(
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
    /// Supplies the shared resource pack for all windows and layer surfaces on this host.
    pub fn with_assets(mut self, source: impl Into<Assets>) -> Self {
        self.assets = Some(source.into());
        self
    }

    /// Calls the setup function once, then runs the windows together on the calling thread
    /// until the last one closes. Each window owns its UI and frame schedule.
    pub fn run(&self, setup: impl FnOnce(Application) -> Result<()>) -> Result<()> {
        self.host.run(|windows| {
            setup(Application {
                windows,
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
