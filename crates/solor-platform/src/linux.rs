use anyhow::Result;
use solor::{Render, Ui};
use solor_linux::{Wayland, layer_shell::LayerShellOptions};
use solor_render_cpu::CpuWindow;

use super::{Application, WindowBackground, WindowHandle, WindowOptions};

pub(super) struct Host(Wayland);

pub(super) fn select() -> Result<Host> {
    Ok(Host(Wayland {
        verify_shm: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
    }))
}

pub(super) type Windows = solor_linux::WaylandApplication;

pub(super) fn open_window(
    app: &Windows,
    options: WindowOptions<'_>,
    ui: Ui,
    root: impl Render,
) -> Result<WindowHandle> {
    app.open_window_with(options, || {
        Ok(Box::new(mount(ui, root, options.background)?))
    })
}

impl Host {
    pub(super) fn run(&self, setup: impl FnOnce(Application) -> Result<()>) -> Result<()> {
        self.0.run(|app| setup(Application(app)))
    }

    pub(super) fn run_layer_surface(
        &self,
        options: LayerShellOptions<'_>,
        ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        self.0
            .run_layer_surface(options, Box::new(mount(ui, root, options.background)?))
    }
}

fn mount(mut ui: Ui, root: impl Render, background: WindowBackground) -> Result<CpuWindow> {
    ui.set_system_font_provider(solor_linux::system_ui_font);
    CpuWindow::mount_with_background(ui, root, background)
}
