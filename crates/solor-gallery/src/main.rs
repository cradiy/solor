//! Native launcher for the Solor Gallery.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use solor::{
        layout::Size,
        platform::{Platform, WindowOptions},
        px,
    };
    solor_linux::Wayland::default().run_window(
        WindowOptions {
            title: "Solor Gallery",
            app_id: "solor.gallery",
            size: Size {
                width: px(800.0),
                height: px(560.0),
            },
        },
        Box::new(solor_gallery::window()?),
    )
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Solor Gallery requires the Linux Wayland platform")
}
