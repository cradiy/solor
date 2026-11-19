//! CPU rasterization of mounted views and incremental rendering of shaped text frames.
/// Incremental raster updates.
pub mod damage;
/// Immutable input to the text rasterizer.
pub mod frame;
/// Bounded glyph mask storage.
pub mod glyph_masks;
/// CPU pixel surfaces.
pub mod pixels;
mod view;
mod window;
pub use window::CpuWindow;
