//! CPU rasterization of mounted views and incremental rendering of shaped text frames.
mod bitmap;
mod checkerboard;
/// Incremental raster updates.
pub mod damage;
/// Immutable input to the text rasterizer.
pub mod frame;
/// Bounded glyph mask storage.
pub mod glyph_masks;
mod mask;
mod path;
/// CPU pixel surfaces.
pub mod pixels;
mod polygon;
mod polyline;
mod rounded_clip;
#[cfg(feature = "shader")]
mod shader;
#[cfg(feature = "shader")]
pub use shader::rasterize_shader;
mod stripes;
mod svg;
mod view;
mod window;
pub use window::CpuWindow;
