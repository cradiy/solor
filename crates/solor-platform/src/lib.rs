//! Selects the window system and rendering backend for mounted Solor views.
use anyhow::Result;
use solor::{Render, Ui};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod system;

pub use solor::platform::WindowOptions;

/// Hosts application windows with the window system and renderer selected for this target.
pub struct SystemPlatform(system::Host);

/// Selects the platform for this process. Returns an unsupported-platform error
/// on targets without a window implementation.
pub fn current_platform() -> Result<SystemPlatform> {
    system::select().map(SystemPlatform)
}

impl SystemPlatform {
    /// Mounts the root and hands input, frame scheduling and presentation to the host.
    /// On Linux this runs the event loop until the window closes.
    pub fn run_window(&self, options: WindowOptions<'_>, ui: Ui, root: impl Render) -> Result<()> {
        self.0.run_window(options, ui, root)
    }
}
