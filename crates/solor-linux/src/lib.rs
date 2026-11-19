//! Linux window systems for Solor.
#![cfg(target_os = "linux")]

mod wayland;
pub use wayland::Wayland;
