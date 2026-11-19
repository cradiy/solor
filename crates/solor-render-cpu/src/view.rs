use std::{
    io::{Error, Result},
    rc::Rc,
};

use crate::pixels::{Clip, DeviceSize, RasterScale, Surface, TextRaster, blend};
use solor::{
    Draw, Fill, Pixels, ViewFrame,
    layout::{Origin, Size},
};

impl Surface {
    /// Replays a complete mounted view in paint order onto an opaque white surface.
    pub fn draw_view(frame: &ViewFrame, size: DeviceSize, scale: RasterScale) -> Result<Self> {
        let mut surface = Self::white(size, scale)?;
        surface.replay(frame, scale, Clip::canvas(size))?;
        Ok(surface)
    }

    pub(super) fn repair(
        &mut self,
        frame: &ViewFrame,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<()> {
        let stride = self.size.width as usize * 4;
        for y in clip.top as usize..clip.bottom as usize {
            self.rgba[y * stride + clip.left as usize * 4..y * stride + clip.right as usize * 4]
                .fill(255);
        }
        self.replay(frame, scale, clip)
    }

    fn replay(&mut self, frame: &ViewFrame, scale: RasterScale, clip: Clip) -> Result<()> {
        let mut text = None;
        for draw in frame.draws() {
            let bounds = device_bounds(draw, scale)?.intersect(clip);
            if bounds.empty() {
                continue;
            }
            match draw {
                Draw::Fill {
                    origin,
                    size,
                    fill,
                    radius,
                } => {
                    self.fill_box(*origin, *size, fill, *radius, scale, bounds)?;
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
                            clip,
                            None,
                        )?),
                    };
                    raster.draw(self, *origin, *allocated, paragraph, *color)?;
                }
            }
        }
        Ok(())
    }

    fn fill_box(
        &mut self,
        origin: Origin,
        size: Size,
        fill: &Fill,
        radius: Pixels,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<()> {
        let left = f64::from(origin.x) * scale.0;
        let top = f64::from(origin.y) * scale.0;
        let width = f64::from(size.width) * scale.0;
        let height = f64::from(size.height) * scale.0;
        let radius = (f64::from(radius) * scale.0).min(width.min(height) / 2.0);
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

// Fill coverage includes fractional edge pixels. Text is clipped to rounded layout edges.
fn device_bounds(draw: &Draw, scale: RasterScale) -> Result<Clip> {
    match draw {
        Draw::Text { origin, size, .. } => Ok(scale.bounds(*origin, *size)),
        Draw::Fill {
            origin,
            size,
            radius,
            ..
        } => {
            if !size.width.is_extent() || !size.height.is_extent() || !radius.is_extent() {
                return Err(Error::other("invalid fill extent"));
            }
            if size.width.get() == 0.0 || size.height.get() == 0.0 {
                return Ok(Clip::canvas(DeviceSize {
                    width: 0,
                    height: 0,
                }));
            }
            let left = f64::from(origin.x) * scale.0;
            let top = f64::from(origin.y) * scale.0;
            Ok(Clip {
                left: left.floor() as i64,
                right: (left + f64::from(size.width) * scale.0).ceil() as i64,
                top: top.floor() as i64,
                bottom: (top + f64::from(size.height) * scale.0).ceil() as i64,
            })
        }
    }
}

fn same_draw(a: &Draw, b: &Draw) -> bool {
    match (a, b) {
        (
            Draw::Text {
                origin,
                size,
                color,
                paragraph,
            },
            Draw::Text {
                origin: at,
                size: extent,
                color: ink,
                paragraph: shaped,
            },
        ) => origin == at && size == extent && color == ink && Rc::ptr_eq(paragraph, shaped),
        _ => a == b,
    }
}

pub(super) fn damage(
    old: &[Draw],
    new: &[Draw],
    scale: RasterScale,
    canvas: Clip,
) -> Result<Option<Clip>> {
    let mut area: Option<Clip> = None;
    for i in 0..old.len().max(new.len()) {
        let (before, after) = (old.get(i), new.get(i));
        if before.zip(after).is_some_and(|(a, b)| same_draw(a, b)) {
            continue;
        }
        for draw in before.into_iter().chain(after) {
            let bounds = device_bounds(draw, scale)?.intersect(canvas);
            if !bounds.empty() {
                area = Some(area.map_or(bounds, |r| r.union(bounds)));
            }
        }
    }
    Ok(area)
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
