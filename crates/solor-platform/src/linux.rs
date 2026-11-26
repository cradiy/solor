use anyhow::Result;
use solor::{Render, Ui, platform::WindowHandler, raster::Rgba8};
use solor_linux::{Wayland, layer_shell::LayerShellOptions};
use solor_render_cpu::CpuWindow;

use super::{Renderer, WindowBackground, WindowHandle, WindowOptions};

pub(super) struct Host(Wayland);

pub(super) fn select() -> Result<Host> {
    Ok(Host(Wayland {
        verify_shm: std::env::var_os("SOLOR_VERIFY_SHM").is_some(),
        trace_submissions: std::env::var_os("SOLOR_TRACE_SUBMISSIONS").is_some(),
        gpu_memory_policy: Default::default(),
    }))
}

pub(super) type Windows = solor_linux::WaylandApplication;

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
    ui.set_system_font_provider(solor_linux::system_ui_font);
    match renderer {
        Renderer::Cpu => app.build_window(options, |handle| {
            mount(ui, root(handle), options.background)
        }),
        #[cfg(feature = "gpu")]
        Renderer::Gpu => app.build_gpu_window(options, ui, root),
        #[cfg(not(feature = "gpu"))]
        Renderer::Gpu => gpu_unavailable(),
    }
}

pub(super) fn lock_session<R: Render>(
    renderer: Renderer,
    app: &Windows,
    mut build: impl FnMut(u32, &super::session_lock::SessionLock) -> Result<(Ui, R)> + 'static,
    notify: impl FnMut(super::session_lock::SessionLockState) + 'static,
) -> Result<super::session_lock::SessionLock> {
    let mut build = move |output, lock: &super::session_lock::SessionLock| -> Result<(Ui, R)> {
        let (mut ui, root) = build(output, lock)?;
        ui.set_system_font_provider(solor_linux::system_ui_font);
        Ok((ui, root))
    };
    match renderer {
        Renderer::Cpu => app.lock_session(
            move |output, lock| {
                let (ui, root) = build(output, lock)?;
                mount(ui, root, WindowBackground::Opaque)
            },
            notify,
        ),
        #[cfg(feature = "gpu")]
        Renderer::Gpu => app.lock_gpu_session(build, notify),
        #[cfg(not(feature = "gpu"))]
        Renderer::Gpu => gpu_unavailable(),
    }
}

pub(super) fn open_popup<R: Render>(
    renderer: Renderer,
    app: &Windows,
    options: solor_linux::popup::PopupOptions,
    mut ui: Ui,
    root: impl FnOnce(WindowHandle) -> R,
) -> Result<WindowHandle> {
    ui.set_system_font_provider(solor_linux::system_ui_font);
    match renderer {
        Renderer::Cpu => {
            let background = options.background;
            app.build_popup(options, |handle| mount(ui, root(handle), background))
        }
        #[cfg(feature = "gpu")]
        Renderer::Gpu => app.build_gpu_popup(options, ui, root),
        #[cfg(not(feature = "gpu"))]
        Renderer::Gpu => gpu_unavailable(),
    }
}

pub(super) fn open_layer_surface<R: Render>(
    renderer: Renderer,
    app: &Windows,
    options: LayerShellOptions<'_>,
    mut ui: Ui,
    root: impl FnOnce(WindowHandle) -> R,
) -> Result<()> {
    ui.set_system_font_provider(solor_linux::system_ui_font);
    match renderer {
        Renderer::Cpu => app.build_layer_surface(options, |handle| {
            mount(ui, root(handle), options.background)
        }),
        #[cfg(feature = "gpu")]
        Renderer::Gpu => app.build_gpu_layer_surface(options, ui, root),
        #[cfg(not(feature = "gpu"))]
        Renderer::Gpu => gpu_unavailable(),
    }
    .map(drop)
}

impl Host {
    pub(super) fn set_gpu_memory_policy(&mut self, policy: super::GpuMemoryPolicy) {
        self.0.gpu_memory_policy = policy;
    }

    pub(super) fn run(&self, setup: impl FnOnce(Windows) -> Result<()>) -> Result<()> {
        self.0.run(setup)
    }

    pub(super) fn run_popup<R: 'static>(
        &self,
        renderer: Renderer,
        connection: solor_linux::popup::Connection,
        control: &solor_linux::popup::PopupHandle,
        assign: impl FnOnce(&solor_linux::popup::WlSurface) -> Result<R>,
        mut ui: Ui,
        root: impl Render,
    ) -> Result<()> {
        ui.set_system_font_provider(solor_linux::system_ui_font);
        match renderer {
            Renderer::Cpu => self.0.run_popup(
                connection,
                control,
                assign,
                mount(ui, root, WindowBackground::Transparent)?,
            ),
            #[cfg(feature = "gpu")]
            Renderer::Gpu => self.0.run_gpu_popup(connection, control, assign, ui, root),
            #[cfg(not(feature = "gpu"))]
            Renderer::Gpu => gpu_unavailable(),
        }
    }
}
