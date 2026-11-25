use anyhow::{Context, Result, bail};
use solor::{
    ViewFrame,
    layout::Size,
    platform::{Presentation, ViewRenderer, WindowBackground},
    raster::RasterScale,
};
use solor_render_gpu::Renderer;
use std::ptr::NonNull;
use wayland_client::{Proxy, backend::Backend, protocol::wl_surface::WlSurface};
use wgpu::{
    CurrentSurfaceTexture as Acquisition,
    rwh::{WaylandDisplayHandle, WaylandWindowHandle},
};

pub(super) struct Native;
impl Presentation for Native {
    type Frame<'a> = Option<(wgpu::Queue, wgpu::SurfaceTexture)>;
}

pub(super) struct GpuContext {
    instance: wgpu::Instance,
    hardware: Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)>,
}
impl Default for GpuContext {
    fn default() -> Self {
        Self {
            instance: wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            hardware: None,
        }
    }
}

pub(super) struct Presenter {
    surface: wgpu::Surface<'static>,
    instance: wgpu::Instance,
    window: WlSurface,
    connection: Backend,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    fills: Renderer,
    clear: wgpu::Color,
}

#[allow(unsafe_code)]
fn native_surface(
    instance: &wgpu::Instance,
    window: &WlSurface,
    connection: &Backend,
) -> Result<wgpu::Surface<'static>> {
    let display =
        NonNull::new(connection.display_ptr().cast()).context("Wayland connection closed")?;
    let surface = NonNull::new(window.id().as_ptr().cast()).context("Wayland window closed")?;
    // SAFETY: Both pointers come from this live system-backend proxy. Presenter keeps
    // the connection alive and drops the WSI surface before Window destroys wl_surface.
    Ok(unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(WaylandDisplayHandle::new(display).into()),
            raw_window_handle: WaylandWindowHandle::new(surface).into(),
        })?
    })
}

impl Presenter {
    pub(super) fn max_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub(super) fn new(
        context: &mut GpuContext,
        window: &WlSurface,
        background: WindowBackground,
        trace: bool,
    ) -> Result<Self> {
        let connection = window
            .backend()
            .upgrade()
            .context("Wayland connection closed")?;
        let instance = context.instance.clone();
        let surface = native_surface(&instance, window, &connection)?;
        if context.hardware.is_none() {
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                }))?;
            if trace {
                eprintln!("gpu_adapter={:?}", adapter.get_info());
            }
            let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
            context.hardware = Some((adapter, device, queue));
        }
        let (adapter, device, queue) = context.hardware.as_ref().unwrap();
        let caps = surface.get_capabilities(adapter);
        let mut config = surface
            .get_default_config(adapter, 1, 1)
            .context("GPU cannot present to this Wayland surface")?;
        config.format = caps
            .formats
            .into_iter()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .context("surface requires an unsupported color format")?;
        let clear = match background {
            WindowBackground::Transparent => {
                anyhow::ensure!(
                    caps.alpha_modes
                        .contains(&wgpu::CompositeAlphaMode::PreMultiplied),
                    "GPU surface does not support premultiplied alpha"
                );
                config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
                wgpu::Color::TRANSPARENT
            }
            WindowBackground::Opaque => wgpu::Color::WHITE,
        };
        let fills = Renderer::new(device, queue, config.format)?;
        Ok(Self {
            surface,
            instance,
            window: window.clone(),
            connection,
            config,
            configured: false,
            device: device.clone(),
            queue: queue.clone(),
            fills,
            clear,
        })
    }
}

impl ViewRenderer for Presenter {
    type Target = Native;
    fn render(
        &mut self,
        frame: ViewFrame,
        size: Size,
        scale: RasterScale,
        _reset: bool,
    ) -> Result<<Native as Presentation>::Frame<'_>> {
        let size = scale.buffer_size(size)?;
        if !self.configured || (self.config.width, self.config.height) != (size.width, size.height)
        {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
            self.configured = true;
        }
        let texture = match self.surface.get_current_texture() {
            Acquisition::Success(t) => t,
            Acquisition::Suboptimal(t) => {
                self.configured = false;
                t
            }
            Acquisition::Outdated => {
                self.configured = false;
                return Ok(None);
            }
            Acquisition::Lost => {
                self.surface = native_surface(&self.instance, &self.window, &self.connection)?;
                self.configured = false;
                return Ok(None);
            }
            Acquisition::Timeout | Acquisition::Occluded => return Ok(None),
            Acquisition::Validation => bail!("GPU surface acquisition failed validation"),
        };
        self.fills.draw(
            &frame,
            &texture.texture.create_view(&Default::default()),
            size,
            scale,
            self.clear,
        )?;
        Ok(Some((self.queue.clone(), texture)))
    }
}
