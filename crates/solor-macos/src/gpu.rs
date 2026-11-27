//! Optional Metal GPU presentation via `wgpu` and `CAMetalLayer`.
use anyhow::{Context, Result, bail};
use objc2::rc::Retained;
use objc2_quartz_core::CAMetalLayer;
use solor::{
    ViewFrame,
    layout::Size,
    platform::{GpuMemoryPolicy, Presentation, ViewRenderer, WindowBackground},
    raster::{DeviceSize, RasterScale},
};
use solor_render_gpu::Renderer;
use std::cell::OnceCell;
use wgpu::CurrentSurfaceTexture as Acquisition;

pub(crate) struct Native;

pub(crate) struct GpuFrame {
    pub(crate) queue: wgpu::Queue,
    pub(crate) texture: wgpu::SurfaceTexture,
    pub(crate) request_callback: bool,
}

impl Presentation for Native {
    type Frame<'a> = Option<GpuFrame>;
}

pub(crate) struct GpuContext {
    memory_policy: GpuMemoryPolicy,
    instance: OnceCell<wgpu::Instance>,
    hardware: Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)>,
}

impl GpuContext {
    pub(crate) fn new(memory_policy: GpuMemoryPolicy) -> Self {
        Self {
            memory_policy,
            instance: OnceCell::new(),
            hardware: None,
        }
    }
}

pub(crate) struct Presenter {
    // Drop `surface` before `metal_layer` so Core Animation layer remains valid for teardown.
    surface: wgpu::Surface<'static>,
    instance: wgpu::Instance,
    metal_layer: Retained<CAMetalLayer>,
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
    layer: &CAMetalLayer,
) -> Result<wgpu::Surface<'static>> {
    let raw_layer = (layer as *const CAMetalLayer).cast_mut().cast();
    // SAFETY: `layer` is a live `CAMetalLayer` retained by `Presenter` for longer than `surface`.
    Ok(unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(raw_layer))?
    })
}

fn select_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats.iter().copied().find(|format| {
        matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
        )
    })
}

fn select_alpha_mode(
    background: WindowBackground,
    modes: &[wgpu::CompositeAlphaMode],
) -> Result<(wgpu::CompositeAlphaMode, wgpu::Color)> {
    match background {
        WindowBackground::Opaque => {
            let mode = if modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                modes
                    .first()
                    .copied()
                    .context("GPU surface exposes no composite alpha modes")?
            };
            Ok((mode, wgpu::Color::WHITE))
        }
        WindowBackground::Transparent => {
            let mode = if modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                wgpu::CompositeAlphaMode::PreMultiplied
            } else if modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
                // Metal surfaces expose `PostMultiplied` to disable layer opacity (`setOpaque(false)`).
                wgpu::CompositeAlphaMode::PostMultiplied
            } else {
                bail!("GPU surface does not support transparent composition")
            };
            Ok((mode, wgpu::Color::TRANSPARENT))
        }
    }
}

pub(crate) fn budgeted_gpu_target(
    size: Size,
    preferred: RasterScale,
    max_dimension: u32,
) -> Result<(RasterScale, DeviceSize)> {
    let physical = preferred.buffer_size(size)?;
    if physical.width <= max_dimension && physical.height <= max_dimension {
        return Ok((preferred, physical));
    }
    let max_logical = f64::from(size.width).max(f64::from(size.height)).max(1.0);
    let mut factor = (f64::from(max_dimension) / max_logical)
        .min(preferred.0)
        .max(f64::EPSILON);
    for _ in 0..16 {
        let candidate = RasterScale(factor);
        let candidate_size = candidate.buffer_size(size)?;
        if candidate_size.width <= max_dimension && candidate_size.height <= max_dimension {
            return Ok((candidate, candidate_size));
        }
        factor *= 0.98;
    }
    let fallback = RasterScale(factor);
    Ok((fallback, fallback.buffer_size(size)?))
}

impl Presenter {
    pub(crate) fn max_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub(crate) fn new(
        context: &mut GpuContext,
        metal_layer: Retained<CAMetalLayer>,
        background: WindowBackground,
        trace: bool,
    ) -> Result<Self> {
        let instance = context
            .instance
            .get_or_init(|| {
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
            })
            .clone();
        let surface = native_surface(&instance, &metal_layer)?;
        if context.hardware.is_none() {
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                }))?;
            if trace {
                eprintln!("gpu_adapter={:?}", adapter.get_info());
            }
            let descriptor = wgpu::DeviceDescriptor {
                memory_hints: match context.memory_policy {
                    GpuMemoryPolicy::Performance => wgpu::MemoryHints::Performance,
                    GpuMemoryPolicy::MemoryUsage => wgpu::MemoryHints::MemoryUsage,
                },
                ..Default::default()
            };
            let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))?;
            context.hardware = Some((adapter, device, queue));
        }
        let (adapter, device, queue) = context.hardware.as_ref().expect("initialized GPU hardware");
        let caps = surface.get_capabilities(adapter);
        let mut config = surface
            .get_default_config(adapter, 1, 1)
            .context("GPU cannot present to this Metal surface")?;
        config.format = select_surface_format(&caps.formats)
            .context("surface requires an unsupported color format")?;
        let (alpha_mode, clear) = select_alpha_mode(background, &caps.alpha_modes)?;
        config.alpha_mode = alpha_mode;
        let fills = Renderer::new(device, queue, config.format)?;
        Ok(Self {
            surface,
            instance,
            metal_layer,
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
        let request_callback = frame.requests_animation_frame();
        let size = scale.buffer_size(size)?;
        self.metal_layer.setContentsScale(scale.0);
        if !self.configured || (self.config.width, self.config.height) != (size.width, size.height)
        {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
            self.configured = true;
        }
        let texture = match self.surface.get_current_texture() {
            Acquisition::Success(texture) => texture,
            Acquisition::Suboptimal(texture) => {
                self.configured = false;
                texture
            }
            Acquisition::Outdated => {
                self.configured = false;
                return Ok(None);
            }
            Acquisition::Lost => {
                self.surface = native_surface(&self.instance, &self.metal_layer)?;
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
        Ok(Some(GpuFrame {
            queue: self.queue.clone(),
            texture,
            request_callback,
        }))
    }
}

#[cfg(test)]
#[path = "../tests/unit/gpu.rs"]
mod tests;
