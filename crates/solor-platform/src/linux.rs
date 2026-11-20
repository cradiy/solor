use anyhow::Result;
use solor::{Render, Ui, platform::Platform};
use solor_linux::Wayland;
use solor_render_cpu::CpuWindow;

use super::WindowOptions;

pub(super) struct Host(Wayland);

pub(super) fn select() -> Result<Host> {
    Ok(Host(Wayland {
        verify_shm: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
    }))
}

impl Host {
    pub(super) fn run_window(
        &self,
        options: WindowOptions<'_>,
        mut ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        ui.set_system_font_provider(solor_linux::system_ui_font);
        let window = CpuWindow::mount(ui, root)?;
        self.0.run_window(options, Box::new(window))
    }
}
