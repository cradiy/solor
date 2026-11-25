use anyhow::Result;
use solor::{Render, Ui};
use solor_linux::{Wayland, layer_shell::LayerShellOptions};
use solor_render_cpu::CpuWindow;

use super::{WindowBackground, WindowHandle, WindowOptions};

pub(super) struct Host(Wayland);

pub(super) fn select() -> Result<Host> {
    Ok(Host(Wayland {
        verify_shm: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
    }))
}

pub(super) type Windows = solor_linux::WaylandApplication;

pub(super) fn open_window<R: Render>(
    app: &Windows,
    options: WindowOptions<'_>,
    ui: Ui,
    root: impl FnOnce(WindowHandle) -> R,
) -> Result<WindowHandle> {
    #[cfg(feature = "gpu")]
    if gpu_selected() {
        let mut ui = ui;
        ui.set_system_font_provider(solor_linux::system_ui_font);
        return app.build_gpu_window(options, ui, root);
    }
    app.build_window(options, |handle| {
        Ok(Box::new(mount(ui, root(handle), options.background)?))
    })
}

pub(super) fn open_popup<R: Render>(
    app: &Windows,
    options: solor_linux::popup::PopupOptions,
    ui: Ui,
    root: impl FnOnce(WindowHandle) -> R,
) -> Result<WindowHandle> {
    #[cfg(feature = "gpu")]
    if gpu_selected() {
        let mut ui = ui;
        ui.set_system_font_provider(solor_linux::system_ui_font);
        return app.build_gpu_popup(options, ui, root);
    }
    app.build_popup(options, |handle| {
        Ok(Box::new(mount(ui, root(handle), options.background)?))
    })
}

pub(super) fn open_layer_surface(
    app: &Windows,
    options: LayerShellOptions<'_>,
    ui: Ui,
    root: impl Render,
) -> Result<()> {
    #[cfg(feature = "gpu")]
    if gpu_selected() {
        let mut ui = ui;
        ui.set_system_font_provider(solor_linux::system_ui_font);
        return app.open_gpu_layer_surface(options, ui, root).map(drop);
    }
    app.open_layer_surface(options, Box::new(mount(ui, root, options.background)?))
        .map(drop)
}

#[cfg(feature = "gpu")]
fn gpu_selected() -> bool {
    std::env::var_os("SOLOR_RENDERER").as_deref() == Some(std::ffi::OsStr::new("gpu"))
}

impl Host {
    pub(super) fn run(&self, setup: impl FnOnce(Windows) -> Result<()>) -> Result<()> {
        self.0.run(setup)
    }
}

fn mount(mut ui: Ui, root: impl Render, background: WindowBackground) -> Result<CpuWindow> {
    ui.set_system_font_provider(solor_linux::system_ui_font);
    CpuWindow::mount_with_background(ui, root, background)
}

impl Host {
    pub(super) fn run_popup<R: 'static>(
        &self,
        connection: solor_linux::popup::Connection,
        control: &solor_linux::popup::PopupHandle,
        assign: impl FnOnce(&solor_linux::popup::WlSurface) -> Result<R>,
        ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        self.0.run_popup(
            connection,
            control,
            assign,
            Box::new(mount(ui, root, WindowBackground::Transparent)?),
        )
    }
}
