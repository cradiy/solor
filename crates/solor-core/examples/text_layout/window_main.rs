//! Linux Wayland presentation of the fixed two-row text experiment.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[allow(dead_code)]
mod damage;
#[allow(dead_code)]
#[path = "../layout_contract/fixtures.rs"]
mod fixtures;
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod glyph_masks;
#[allow(dead_code)]
#[path = "../layout_contract/layout.rs"]
mod layout;
#[allow(dead_code)]
mod pixels;
#[allow(dead_code)]
#[path = "../node_store/store.rs"]
mod store;
#[allow(dead_code)]
mod text;

const MEASUREMENT_CACHE_SLOTS: usize = 3;

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    wayland::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("text_window requires Linux with a Wayland compositor");
    std::process::ExitCode::FAILURE
}
