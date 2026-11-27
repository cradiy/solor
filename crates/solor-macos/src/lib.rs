//! macOS platform implementation for Solor using AppKit, Core Graphics, and optional Metal.
#![cfg(target_os = "macos")]

mod app;
mod buffer;
mod clipboard;
mod cursor;
mod display_link;
mod events;
mod font;
#[cfg(feature = "gpu")]
mod gpu;
mod life;
mod wake;
mod window;

pub use app::{MacApplication, MacOs};
pub use font::system_ui_font;
