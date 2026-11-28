use anyhow::Result;
use solor::{Render, Ui, platform::WindowHandler, raster::Rgba8};
use solor_macos::{MacApplication, MacOs};
use solor_render_cpu::CpuWindow;

use super::{Renderer, WindowBackground, WindowHandle, WindowOptions};

pub(super) struct Host(MacOs);

pub(super) fn select() -> Result<Host> {
    Ok(Host(MacOs {
        gpu_memory_policy: Default::default(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
    }))
}

pub(super) type Windows = MacApplication;

#[cfg(not(feature = "gpu"))]
fn gpu_unavailable<T>() -> Result<T> {
    anyhow::bail!("GPU rendering requires the solor-platform `gpu` feature")
}

fn mount(
    ui: Ui,
    root: impl Render,
    background: WindowBackground,
) -> Result<Box<dyn WindowHandler<Rgba8>>> {
    Ok(Box::new(CpuWindow::mount_with_background(
        ui, root, background,
    )?))
}

pub(super) fn open_window<R: Render>(
    renderer: Renderer,
    app: &Windows,
    options: WindowOptions<'_>,
    mut ui: Ui,
    root: impl FnOnce(WindowHandle) -> R,
) -> Result<WindowHandle> {
    ui.set_system_font_provider(solor_macos::system_ui_font);
    match renderer {
        Renderer::Cpu => {
            let background = options.background;
            app.build_window(options, |handle| mount(ui, root(handle), background))
        }
        #[cfg(feature = "gpu")]
        Renderer::Gpu => app.build_gpu_window(options, ui, root),
        #[cfg(not(feature = "gpu"))]
        Renderer::Gpu => gpu_unavailable(),
    }
}

impl Host {
    pub(super) fn set_gpu_memory_policy(&mut self, policy: super::GpuMemoryPolicy) {
        self.0.gpu_memory_policy = policy;
    }

    pub(super) fn run(&self, setup: impl FnOnce(Windows) -> Result<()>) -> Result<()> {
        self.0.run(setup)
    }
}

pub(super) use solor_macos::select_files;
