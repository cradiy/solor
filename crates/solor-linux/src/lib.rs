//! Linux window systems for Solor.
#![cfg(target_os = "linux")]

mod desktop_font;
pub mod layer_shell;
mod wayland;
pub use desktop_font::system_ui_font;
pub use wayland::{Wayland, WaylandApplication};
