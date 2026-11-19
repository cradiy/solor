//! CPU rasterization, glyph caching and damage tracking for shaped text frames.
/// Incremental raster updates.
pub mod damage;
/// Immutable input to the text rasterizer.
pub mod frame;
/// Bounded glyph mask storage.
pub mod glyph_masks;
/// CPU pixel surfaces.
pub mod pixels;
