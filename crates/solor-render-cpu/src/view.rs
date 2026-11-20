use std::{
    io::{Error, Result},
    rc::Rc,
};

use crate::glyph_masks::GlyphMasks;
use crate::pixels::{Clip, DeviceSize, RasterScale, Surface, TextRaster, blend};
use solor::{
    Draw, Fill, Pixels, ViewFrame,
    layout::{Origin, Size},
    style::Corners,
};

impl Surface {
    /// Replays a complete mounted view in paint order onto an opaque white surface.
    pub fn draw_view(frame: &ViewFrame, size: DeviceSize, scale: RasterScale) -> Result<Self> {
        Self::raster_view(frame, size, scale, None)
    }

    pub(super) fn raster_view(
        frame: &ViewFrame,
        size: DeviceSize,
        scale: RasterScale,
        masks: Option<&mut GlyphMasks>,
    ) -> Result<Self> {
        let mut surface = Self::white(size, scale)?;
        surface.replay(frame, scale, Clip::canvas(size), masks)?;
        Ok(surface)
    }

    pub(super) fn repair(
        &mut self,
        frame: &ViewFrame,
        scale: RasterScale,
        clip: Clip,
        masks: &mut GlyphMasks,
    ) -> Result<()> {
        let stride = self.size.width as usize * 4;
        for y in clip.top as usize..clip.bottom as usize {
            self.rgba[y * stride + clip.left as usize * 4..y * stride + clip.right as usize * 4]
                .fill(255);
        }
        self.replay(frame, scale, clip, Some(masks))
    }

    fn replay(
        &mut self,
        frame: &ViewFrame,
        scale: RasterScale,
        clip: Clip,
        mut masks: Option<&mut GlyphMasks>,
    ) -> Result<()> {
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
                    corner_radii,
                    ..
                } => {
                    self.fill_box(*origin, *size, fill, *corner_radii, scale, bounds)?;
                }
                Draw::Text {
                    origin,
                    size: allocated,
                    paragraph,
                    color,
                    ..
                } => {
                    let raster = match &mut text {
                        Some(raster) => raster,
                        None => text.insert(TextRaster::new(
                            frame
                                .font()
                                .ok_or_else(|| Error::other("text without a font face"))?,
                            scale,
                            (0, 0),
                            masks.take(),
                        )?),
                    };
                    raster.draw(self, *origin, *allocated, paragraph, *color, bounds)?;
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
        corner_radii: Corners<Pixels>,
        scale: RasterScale,
        clip: Clip,
    ) -> Result<()> {
        let left = f64::from(origin.x) * scale.0;
        let top = f64::from(origin.y) * scale.0;
        let width = f64::from(size.width) * scale.0;
        let height = f64::from(size.height) * scale.0;
        let radii = corner_radii.map(|radius| f64::from(radius) * scale.0);
        let top_radius = radii.top_left.max(radii.top_right);
        let bottom_radius = radii.bottom_left.max(radii.bottom_right);
        for y in clip.top..clip.bottom {
            let dy = y as f64 - top;
            let row_has_arcs = (top_radius > 0.0 && dy < top_radius)
                || (bottom_radius > 0.0 && height - dy - 1.0 < bottom_radius);
            let row_height = ((dy + 1.0).min(height) - dy.max(0.0)).clamp(0.0, 1.0);
            for x in clip.left..clip.right {
                let dx = x as f64 - left;
                let coverage = if row_has_arcs {
                    coverage(dx, dy, width, height, row_height, radii)
                } else {
                    rectangle_coverage(dx, width, row_height)
                };
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
    let mut bounds = primitive_bounds(draw, scale)?;
    let (Draw::Fill { clip, .. } | Draw::Text { clip, .. }) = draw;
    if let Some(clip) = clip {
        if let Some(x) = &clip.x {
            bounds.left = bounds.left.max(scale.edge(f64::from(x.start)));
            bounds.right = bounds.right.min(scale.edge(f64::from(x.end)));
        }
        if let Some(y) = &clip.y {
            bounds.top = bounds.top.max(scale.edge(f64::from(y.start)));
            bounds.bottom = bounds.bottom.min(scale.edge(f64::from(y.end)));
        }
    }
    Ok(bounds)
}

fn primitive_bounds(draw: &Draw, scale: RasterScale) -> Result<Clip> {
    match draw {
        Draw::Text { origin, size, .. } => Ok(scale.bounds(*origin, *size)),
        Draw::Fill {
            origin,
            size,
            corner_radii,
            ..
        } => {
            if !size.width.is_extent()
                || !size.height.is_extent()
                || [
                    corner_radii.top_left,
                    corner_radii.top_right,
                    corner_radii.bottom_right,
                    corner_radii.bottom_left,
                ]
                .iter()
                .any(|r| !r.is_extent())
            {
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
                clip,
            },
            Draw::Text {
                origin: at,
                size: extent,
                color: ink,
                paragraph: shaped,
                clip: other_clip,
            },
        ) => {
            origin == at
                && size == extent
                && color == ink
                && clip == other_clip
                && Rc::ptr_eq(paragraph, shaped)
        }
        _ => a == b,
    }
}

pub(super) fn damage(
    old: &[Draw],
    new: &[Draw],
    scale: RasterScale,
    canvas: Clip,
) -> Result<Option<Clip>> {
    // An insertion or removal can shift identical trailing commands to new indices.
    // They still replay in paint order wherever the changed draws cause damage.
    let suffix = if old.len() != new.len() {
        old.iter()
            .rev()
            .zip(new.iter().rev())
            .take_while(|(before, after)| same_draw(before, after))
            .count()
    } else {
        0
    };
    let old = &old[..old.len() - suffix];
    let new = &new[..new.len() - suffix];
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

fn rectangle_coverage(x: f64, width: f64, row_height: f64) -> u8 {
    let w = ((x + 1.0).min(width) - x.max(0.0)).clamp(0.0, 1.0);
    (w * row_height * 255.0).round() as u8
}

// Sample only the corner regions.
fn coverage(x: f64, y: f64, width: f64, height: f64, row_height: f64, radii: Corners<f64>) -> u8 {
    let right = width - x;
    let bottom = height - y;
    let touches = |dx: f64, dy: f64, r: f64| r > 0.0 && dx < r && dy < r;
    if !touches(x, y, radii.top_left)
        && !touches(right - 1.0, y, radii.top_right)
        && !touches(right - 1.0, bottom - 1.0, radii.bottom_right)
        && !touches(x, bottom - 1.0, radii.bottom_left)
    {
        return rectangle_coverage(x, width, row_height);
    }

    let inside_corner = |dx: f64, dy: f64, r: f64| {
        dx >= r || dy >= r || (r - dx).powi(2) + (r - dy).powi(2) <= r * r
    };
    let mut inside = 0;
    for sy in [0.125, 0.375, 0.625, 0.875] {
        for sx in [0.125, 0.375, 0.625, 0.875] {
            let (left, top, right, bottom) = (x + sx, y + sy, right - sx, bottom - sy);
            inside += u16::from(
                left >= 0.0
                    && right >= 0.0
                    && top >= 0.0
                    && bottom >= 0.0
                    && inside_corner(left, top, radii.top_left)
                    && inside_corner(right, top, radii.top_right)
                    && inside_corner(right, bottom, radii.bottom_right)
                    && inside_corner(left, bottom, radii.bottom_left),
            );
        }
    }
    ((inside * 255 + 8) / 16) as u8
}
