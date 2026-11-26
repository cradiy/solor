//! Launches the gallery on the current system platform.
use clap::{Arg, Command};
use solor::px;
use solor_platform::{GpuMemoryPolicy, WindowOptions, current_platform};

fn main() -> anyhow::Result<()> {
    let args = Command::new("solor-gallery")
        .arg(
            Arg::new("gpu-memory-policy")
                .long("gpu-memory-policy")
                .value_parser(["performance", "memory-usage"])
                .help("Set the GPU device's allocation preference"),
        )
        .get_matches();
    let mut platform = current_platform()?;
    if let Some(policy) = args.get_one::<String>("gpu-memory-policy") {
        let policy = match policy.as_str() {
            "memory-usage" => GpuMemoryPolicy::MemoryUsage,
            "performance" => GpuMemoryPolicy::Performance,
            _ => unreachable!("validated by clap"),
        };
        platform = platform.with_gpu_memory_policy(policy);
    }
    let window = WindowOptions::new("Solor Gallery")
        .without_decorations()
        .app_id("solor.gallery")
        .size(px(800.0), px(560.0));
    let (ui, gallery) = solor_gallery::gallery(solor::Ui::new())?;
    platform.run_window(window, ui, gallery)
}
