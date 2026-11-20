//! Launches the gallery on the current system platform.
use solor::{layout::Size, px};
use solor_platform::{WindowOptions, current_platform};

fn main() -> anyhow::Result<()> {
    let platform = current_platform()?;
    let (ui, gallery) = solor_gallery::gallery()?;
    platform.run_window(
        WindowOptions {
            title: "Solor Gallery",
            app_id: "solor.gallery",
            size: Size {
                width: px(800.0),
                height: px(560.0),
            },
        },
        ui,
        gallery,
    )
}
