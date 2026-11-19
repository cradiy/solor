use std::io::{Error, Result};

use crate::pixels::{Clip, DeviceSize, RasterScale, Surface, TextRaster, blend};
use solor::{
    Draw, Fill, Pixels, ViewFrame,
    layout::{Origin, Size},
};

impl Surface {
    /// Replays a complete mounted view in paint order onto an opaque white surface.
    pub fn draw_view(frame: &ViewFrame, size: DeviceSize, scale: RasterScale) -> Result<Self> {
        let mut surface = Self::white(size, scale)?;
        let mut text = None;
        for draw in frame.draws() {
            match draw {
                Draw::Fill {
                    origin,
                    size,
                    fill,
                    radius,
                } => {
                    surface.fill_box(*origin, *size, fill, *radius, scale)?;
                }
                Draw::Text {
                    origin,
                    size: allocated,
                    paragraph,
                    color,
                } => {
                    let raster = match &mut text {
                        Some(raster) => raster,
                        None => text.insert(TextRaster::new(
                            frame
                                .font()
                                .ok_or_else(|| Error::other("text without a font face"))?,
                            scale,
                            (0, 0),
                            Clip::canvas(size),
                            None,
                        )?),
                    };
                    raster.draw(&mut surface, *origin, *allocated, paragraph, *color)?;
                }
            }
        }
        Ok(surface)
    }

    fn fill_box(
        &mut self,
        origin: Origin,
        size: Size,
        fill: &Fill,
        radius: Pixels,
        scale: RasterScale,
    ) -> Result<()> {
        if !size.width.is_extent() || !size.height.is_extent() || !radius.is_extent() {
            return Err(Error::other("invalid fill extent"));
        }
        let left = f64::from(origin.x) * scale.0;
        let top = f64::from(origin.y) * scale.0;
        let width = f64::from(size.width) * scale.0;
        let height = f64::from(size.height) * scale.0;
        let radius = (f64::from(radius) * scale.0).min(width.min(height) / 2.0);
        if width == 0.0 || height == 0.0 {
            return Ok(());
        }
        let clip = Clip::canvas(self.size).intersect(Clip {
            left: left.floor() as i64,
            right: (left + width).ceil() as i64,
            top: top.floor() as i64,
            bottom: (top + height).ceil() as i64,
        });
        for y in clip.top..clip.bottom {
            for x in clip.left..clip.right {
                let dx = x as f64 - left;
                let dy = y as f64 - top;
                let coverage = coverage(dx, dy, width, height, radius);
                if coverage == 0 {
                    continue;
                }
                let color = match fill {
                    Fill::Solid(color) => *color,
                    Fill::Gradient(gradient) => gradient
                        .sample_at(
                            ((dx + 0.5) / scale.0) as f32,
                            ((dy + 0.5) / scale.0) as f32,
                            size,
                        )
                        .map_err(Error::other)?,
                    _ => {
                        return Err(Error::new(
                            std::io::ErrorKind::Unsupported,
                            "unsupported CPU fill",
                        ));
                    }
                };
                let i = (y as usize * self.size.width as usize + x as usize) * 4;
                blend(&mut self.rgba[i..i + 4], color, coverage);
            }
        }
        Ok(())
    }
}

// Exact rectangle area along straight edges. Sample only the corner regions.
fn coverage(x: f64, y: f64, width: f64, height: f64, radius: f64) -> u8 {
    if radius == 0.0
        || (x >= radius && x + 1.0 <= width - radius)
        || (y >= radius && y + 1.0 <= height - radius)
    {
        let w = ((x + 1.0).min(width) - x.max(0.0)).clamp(0.0, 1.0);
        let h = ((y + 1.0).min(height) - y.max(0.0)).clamp(0.0, 1.0);
        return (w * h * 255.0).round() as u8;
    }
    let mut inside = 0;
    for sy in [0.125, 0.375, 0.625, 0.875] {
        for sx in [0.125, 0.375, 0.625, 0.875] {
            let (px, py) = (x + sx, y + sy);
            let dx = (radius - px).max(px - (width - radius)).max(0.0);
            let dy = (radius - py).max(py - (height - radius)).max(0.0);
            inside += u16::from(dx * dx + dy * dy <= radius * radius);
        }
    }
    ((inside * 255 + 8) / 16) as u8
}
