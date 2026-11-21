//! Selects the window system and rendering backend for mounted Solor views.
use anyhow::Result;
use solor::{Render, Ui};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod system;

pub use solor::platform::{WindowBackground, WindowHandle, WindowOptions};
#[cfg(target_os = "linux")]
pub use solor_linux::layer_shell;

/// Hosts application windows with the window system and renderer selected for this target.
pub struct SystemPlatform(system::Host);

/// Selects the platform for this process. Returns an unsupported-platform error
/// on targets without a window implementation.
pub fn current_platform() -> Result<SystemPlatform> {
    system::select().map(SystemPlatform)
}

/// A cloneable UI-thread handle for opening windows during setup or from event handlers.
/// Keeping a handle does not keep the event loop alive.
#[derive(Clone)]
pub struct Application(system::Windows);

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
        system::open_window(&self.0, options, ui, root)
    }
}

impl SystemPlatform {
    /// Calls the setup function once, then runs the windows together on the calling thread
    /// until the last one closes. Each window owns its UI and frame schedule.
    pub fn run(&self, setup: impl FnOnce(Application) -> Result<()>) -> Result<()> {
        self.0.run(setup)
    }

    /// Mounts the root and hands input, frame scheduling and presentation to the host.
    /// On Linux this runs the event loop until the window closes.
    pub fn run_window(&self, options: WindowOptions<'_>, ui: Ui, root: impl Render) -> Result<()> {
        self.run(|app| app.open_window(options, ui, root).map(drop))
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
        self.0.run_layer_surface(options, ui, root)
    }
}
