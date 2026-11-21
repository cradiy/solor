use std::{
    io::{Error, Result},
    rc::Rc,
};

use crate::glyph_masks::GlyphMasks;
use crate::pixels::{Clip, DeviceSize, RasterScale, Surface, TextRaster, blend, blend_solid_span};
use solor::{
    Draw, Fill, Pixels, ViewFrame,
    layout::{Origin, Size},
    raster::{Damage, OpaqueLinearGradient},
    style::Corners,
};

pub(super) struct ViewCaches {
    images: solor::ImageCache,
    glyphs: GlyphMasks,
    svgs: crate::svg::SvgCache,
}

impl Default for ViewCaches {
    fn default() -> Self {
        Self {
            images: Default::default(),
            glyphs: GlyphMasks::new(64, 32 * 1024),
            svgs: Default::default(),
        }
    }
}

enum Brush<'a, F> {
    Solid(solor::Rgba),
    Region(crate::bitmap::RegionSampler),
    Image(crate::bitmap::ImageSampler<'a>),
    Gradient(F, Option<OpaqueLinearGradient<'a>>),
}

impl Surface {
    /// Replays a complete mounted view in paint order onto an opaque white surface.
    pub fn draw_view(frame: &ViewFrame, size: DeviceSize, scale: RasterScale) -> Result<Self> {
        Self::raster_view(frame, size, scale, 255, &mut ViewCaches::default())
    }

    pub(super) fn raster_view(
        frame: &ViewFrame,
        size: DeviceSize,
        scale: RasterScale,
        clear_byte: u8,
        caches: &mut ViewCaches,
    ) -> Result<Self> {
        let mut surface = Self::cleared(size, scale, clear_byte)?;
        surface.replay(frame.draws(), scale, Clip::canvas(size), caches)?;
        Ok(surface)
    }

    pub(super) fn repair(
        &mut self,
        frame: &ViewFrame,
        scale: RasterScale,
        clip: Clip,
        clear_byte: u8,
        caches: &mut ViewCaches,
    ) -> Result<()> {
        let mut draws = frame.draws();
        let mut background = [clear_byte; 4];
        while let Some((draw, tail)) = draws.split_first() {
            let bounds = device_bounds(draw, scale)?.intersect(clip);
            if !bounds.empty() {
                let Draw::Fill {
                    fill: Fill::Solid(color),
                    origin,
                    size,
                    corner_radii,
                    ..
                } = draw
                else {
                    break;
                };
                let left = f64::from(origin.x) * scale.0;
                let top = f64::from(origin.y) * scale.0;
                if bounds != clip
                    || color.alpha != 255
                    || *corner_radii != Corners::default()
                    || left > clip.left as f64
                    || top > clip.top as f64
                    || left + f64::from(size.width) * scale.0 < clip.right as f64
                    || top + f64::from(size.height) * scale.0 < clip.bottom as f64
                {
                    break;
                }
                background = [color.red, color.green, color.blue, 255];
            }
            draws = tail;
        }
        // Initialize the repaired pixels once, even with several opaque background layers.
        let stride = self.size.width as usize * 4;
        let start = clip.top as usize * stride + clip.left as usize * 4;
        let row = start..start + (clip.right - clip.left) as usize * 4;
        self.rgba[row.clone()]
            .as_chunks_mut::<4>()
            .0
            .fill(background);
        for y in 1..(clip.bottom - clip.top) as usize {
            self.rgba.copy_within(row.clone(), start + y * stride);
        }
        self.replay(draws, scale, clip, caches)
    }

    fn replay(
        &mut self,
        draws: &[Draw],
        scale: RasterScale,
        clip: Clip,
        caches: &mut ViewCaches,
    ) -> Result<()> {
        let mut masks = Some(&mut caches.glyphs);
        let mut text = None;
        for draw in draws {
            let visible = device_bounds(draw, scale)?;
            let bounds = visible.intersect(clip);
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
                    let fill = caches.images.resolve(fill).map_err(Error::other)?;
                    self.fill_box(*origin, *size, &fill, *corner_radii, scale, bounds)?;
                }
                Draw::Polygon {
                    origin,
                    points,
                    fill,
                    ..
                } => {
                    let fill = caches.images.resolve(fill).map_err(Error::other)?;
                    self.fill_polygon(*origin, points, &fill, scale, bounds)?;
                }
                Draw::Polyline {
                    origin,
                    points,
                    width,
                    color,
                    ..
                } => {
                    self.stroke_polyline(*origin, points, *width, *color, scale, bounds);
                }
                Draw::Svg {
                    origin,
                    size,
                    asset,
                    tint,
                    ..
                } => {
                    let viewport = scale.bounds(*origin, *size);
                    let visible = visible.intersect(Clip::canvas(self.size));
                    caches
                        .svgs
                        .draw(self, asset, *tint, viewport, visible, bounds)?;
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
                        None => text.insert(TextRaster::new(scale, (0, 0), masks.take())),
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
        let mut brush = match fill {
            Fill::Solid(color) => Brush::Solid(*color),
            Fill::CustomImage(image) => {
                let Some(sampler) =
                    crate::bitmap::RegionSampler::new(image, [left, top], size, scale, clip)?
                else {
                    return Ok(());
                };
                Brush::Region(sampler)
            }
            Fill::Image(image) => {
                Brush::Image(crate::bitmap::ImageSampler::new(image, size, scale)?)
            }
            Fill::Gradient(gradient) => {
                // Tiny damage on a dense ramp should not scan the complete stop list.
                let spans = if gradient.stops().len() <= (clip.right - clip.left) as usize {
                    OpaqueLinearGradient::new(gradient, origin, size, scale)
                } else {
                    None
                };
                Brush::Gradient(gradient.sampler(size).map_err(Error::other)?, spans)
            }
            _ => {
                return Err(Error::new(
                    std::io::ErrorKind::Unsupported,
                    "unsupported CPU fill",
                ));
            }
        };
        let stride = self.size.width as usize * 4;
        for y in clip.top..clip.bottom {
            let dy = y as f64 - top;
            let row_has_arcs = (top_radius > 0.0 && dy < top_radius)
                || (bottom_radius > 0.0 && height - dy - 1.0 < bottom_radius);
            let row_height = ((dy + 1.0).min(height) - dy.max(0.0)).clamp(0.0, 1.0);
            let row_start = y as usize * stride;
            let row = &mut self.rgba
                [row_start + clip.left as usize * 4..row_start + clip.right as usize * 4];
            let pixels = row.as_chunks_mut::<4>().0;
            let local_y = ((dy + 0.5) / scale.0) as f32;
            let local_x = |x: i64| ((x as f64 - left + 0.5) / scale.0) as f32;
            // Between the corners a solid row has uniform coverage, even on a fractional edge.
            let (from, to) = if row_height == 1.0 || matches!(brush, Brush::Solid(_)) {
                let inset = if row_has_arcs {
                    top_radius.max(bottom_radius)
                } else {
                    0.0
                };
                let from = ((left + inset).ceil() as i64).clamp(clip.left, clip.right);
                let to = ((left + width - inset).floor() as i64).clamp(from, clip.right);
                (from, to)
            } else {
                (clip.left, clip.left)
            };
            let (leading, rest) = pixels.split_at_mut((from - clip.left) as usize);
            let (middle, trailing) = rest.split_at_mut((to - from) as usize);
            match &mut brush {
                Brush::Solid(c) => {
                    let coverage = (row_height * 255.0).round() as u8;
                    blend_solid_span(middle, *c, coverage);
                }
                Brush::Region(image) => {
                    for (x, pixel) in (from..to).zip(middle) {
                        image.blend(pixel, x, y, scale, 255);
                    }
                }
                Brush::Image(image) => {
                    for (x, pixel) in (from..to).zip(middle) {
                        image.blend(
                            pixel,
                            (x as f64 + 0.5 - left) / scale.0,
                            (dy + 0.5) / scale.0,
                            255,
                        );
                    }
                }
                Brush::Gradient(_, Some(linear)) => linear
                    .write_span(from as u32, y as u32, middle)
                    .map_err(Error::other)?,
                Brush::Gradient(sample, None) => {
                    for (x, pixel) in (from..to).zip(middle) {
                        let c = sample(local_x(x), local_y).map_err(Error::other)?;
                        if c.alpha == 255 {
                            *pixel = [c.red, c.green, c.blue, 255];
                        } else {
                            blend(pixel, c, 255);
                        }
                    }
                }
            }
            for (start, edge) in [(clip.left, leading), (to, trailing)] {
                for (x, pixel) in (start..).zip(edge) {
                    let dx = x as f64 - left;
                    let coverage = if row_has_arcs {
                        coverage(dx, dy, width, height, row_height, radii)
                    } else {
                        rectangle_coverage(dx, width, row_height)
                    };
                    if coverage != 0 {
                        let c = match &mut brush {
                            Brush::Solid(c) => *c,
                            Brush::Region(image) => {
                                image.blend(pixel, x, y, scale, coverage);
                                continue;
                            }
                            Brush::Image(image) => {
                                image.blend(
                                    pixel,
                                    (dx + 0.5) / scale.0,
                                    (dy + 0.5) / scale.0,
                                    coverage,
                                );
                                continue;
                            }
                            Brush::Gradient(sample, _) => {
                                sample(local_x(x), local_y).map_err(Error::other)?
                            }
                        };
                        blend(pixel, c, coverage);
                    }
                }
            }
        }
        Ok(())
    }
}

// Fill coverage includes fractional edge pixels. Text is clipped to rounded layout edges.
fn device_bounds(draw: &Draw, scale: RasterScale) -> Result<Clip> {
    let mut bounds = primitive_bounds(draw, scale)?;
    let (Draw::Fill { clip, .. }
    | Draw::Polygon { clip, .. }
    | Draw::Polyline { clip, .. }
    | Draw::Svg { clip, .. }
    | Draw::Text { clip, .. }) = draw;
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
        Draw::Polygon { origin, points, .. } => {
            crate::path::PathBounds::new(*origin, points, scale)?.clip(0.0)
        }
        Draw::Polyline {
            origin,
            points,
            width,
            ..
        } => crate::polyline::bounds(*origin, points, *width, scale),
        Draw::Svg { origin, size, .. } | Draw::Text { origin, size, .. } => {
            Ok(scale.bounds(*origin, *size))
        }
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

fn include_image_change(
    before: &Draw,
    after: &Draw,
    scale: RasterScale,
    canvas: Clip,
    damage: &mut Damage,
) -> Result<bool> {
    let (
        Draw::Fill {
            origin,
            size,
            fill: Fill::CustomImage(previous),
            corner_radii,
            clip,
        },
        Draw::Fill {
            origin: at,
            size: extent,
            fill: Fill::CustomImage(next),
            corner_radii: radii,
            clip: next_clip,
        },
    ) = (before, after)
    else {
        return Ok(false);
    };
    if origin != at || size != extent || corner_radii != radii || clip != next_clip {
        return Ok(false);
    }
    let Some(regions) = next.changed_regions_since(previous) else {
        return Ok(false);
    };
    let project = crate::bitmap::region_projection(next, *size, *origin, scale)?;
    let visible = device_bounds(after, scale)?.intersect(canvas);
    for &region in regions {
        damage.include(project(region).intersect(visible));
    }
    Ok(true)
}

pub(super) fn damage(
    old: &[Draw],
    new: &[Draw],
    scale: RasterScale,
    canvas: Clip,
) -> Result<Damage> {
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
    let mut area = Damage::default();
    for i in 0..old.len().max(new.len()) {
        let (before, after) = (old.get(i), new.get(i));
        if let Some((a, b)) = before.zip(after) {
            if same_draw(a, b) {
                continue;
            }
            if include_image_change(a, b, scale, canvas, &mut area)? {
                continue;
            }
        }
        for draw in before.into_iter().chain(after) {
            let bounds = device_bounds(draw, scale)?.intersect(canvas);
            area.include(bounds);
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
    // Moving inward along either axis can only increase corner coverage.
    // Test the extremes of the same sample grid before counting boundary samples.
    let all_corners = |left, top, right, bottom| {
        inside_corner(left, top, radii.top_left)
            && inside_corner(right, top, radii.top_right)
            && inside_corner(right, bottom, radii.bottom_right)
            && inside_corner(left, bottom, radii.bottom_left)
    };
    let (left_min, top_min, right_min, bottom_min) =
        (x + 0.125, y + 0.125, right - 0.875, bottom - 0.875);
    if left_min >= 0.0
        && top_min >= 0.0
        && right_min >= 0.0
        && bottom_min >= 0.0
        && all_corners(left_min, top_min, right_min, bottom_min)
    {
        return 255;
    }
    if !all_corners(x + 0.875, y + 0.875, right - 0.125, bottom - 0.125) {
        return 0;
    }
    let mut inside = 0;
    for sy in [0.125, 0.375, 0.625, 0.875] {
        for sx in [0.125, 0.375, 0.625, 0.875] {
            let (left, top, right, bottom) = (x + sx, y + sy, right - sx, bottom - sy);
            inside += u16::from(
                left >= 0.0
                    && right >= 0.0
                    && top >= 0.0
                    && bottom >= 0.0
                    && all_corners(left, top, right, bottom),
            );
        }
    }
    ((inside * 255 + 8) / 16) as u8
}
