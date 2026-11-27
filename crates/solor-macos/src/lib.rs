//! macOS platform implementation for Solor using AppKit and Core Graphics.
#![cfg(target_os = "macos")]

mod app;
mod buffer;
mod clipboard;
mod cursor;
mod display_link;
mod events;
mod font;
mod life;
mod wake;
mod window;

pub use app::{MacApplication, MacOs};
pub use font::system_ui_font;
