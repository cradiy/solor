//! Launches the gallery on the current system platform.
use solor::px;
use solor_platform::{WindowOptions, current_platform};

fn main() -> anyhow::Result<()> {
    let platform = current_platform()?;
    let window = WindowOptions::new("Solor Gallery")
        .without_decorations()
        .app_id("solor.gallery")
        .size(px(800.0), px(560.0));
    let (ui, gallery) = solor_gallery::gallery()?;
    platform.run_window(window, ui, gallery)
}
