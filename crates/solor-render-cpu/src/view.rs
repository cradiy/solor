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

struct Replay<'a> {
    images: &'a mut solor::ImageCache,
    svgs: &'a mut crate::svg::SvgCache,
    masks: Option<&'a mut GlyphMasks>,
    text: Option<TextRaster<'a>>,
}

impl ViewCaches {
    fn replay(&mut self) -> Replay<'_> {
        Replay {
            images: &mut self.images,
            svgs: &mut self.svgs,
            masks: Some(&mut self.glyphs),
            text: None,
        }
    }
}

enum Brush<'a, F> {
    Solid(solor::Rgba),
    Checkerboard(crate::checkerboard::CheckerBrush),
    Stripes(crate::stripes::StripeBrush),
    Region(crate::bitmap::RegionSampler),
    Image(crate::bitmap::ImageSampler<'a>),
    Gradient(F, Option<OpaqueLinearGradient<'a>>),
}

mod backdrop;
mod border;
mod repair;
mod rounded_box;
mod shadow;
mod underline;

pub(super) use repair::RepairIndex;

trait SampleGradient:
    FnMut(f32, f32) -> std::result::Result<solor::Rgba, solor::GradientError>
{
}
impl<F: FnMut(f32, f32) -> std::result::Result<solor::Rgba, solor::GradientError>> SampleGradient
    for F
{
}

impl<F: SampleGradient> Brush<'_, F> {
    fn blend_at(
        &mut self,
        pixel: &mut [u8; 4],
        x: i64,
        y: i64,
        origin: [f64; 2],
        scale: RasterScale,
        coverage: u8,
    ) -> Result<()> {
        let dx = (x as f64 - origin[0] + 0.5) / scale.0;
        let dy = (y as f64 - origin[1] + 0.5) / scale.0;
        match self {
            Self::Solid(c) => blend(pixel, *c, coverage),
            Self::Stripes(pattern) => {
                pattern.blend_span(std::slice::from_mut(pixel), x, y, coverage)
            }
            Self::Checkerboard(pattern) => {
                pattern.blend_span(std::slice::from_mut(pixel), x, y, coverage)
            }
            Self::Gradient(sample, _) => blend(
                pixel,
                sample(dx as f32, dy as f32).map_err(Error::other)?,
                coverage,
            ),
            Self::Image(image) => image.blend(pixel, dx, dy, coverage),
            Self::Region(image) => image.blend(pixel, x, y, scale, coverage),
        }
        Ok(())
    }
}

fn brush(
    fill: &Fill,
    origin: Origin,
    size: Size,
    scale: RasterScale,
    clip: Clip,
) -> Result<Option<Brush<'_, impl SampleGradient + '_>>> {
    let left = f64::from(origin.x) * scale.0;
    let top = f64::from(origin.y) * scale.0;
    Ok(Some(match fill {
        Fill::Solid(color) => Brush::Solid(*color),
        Fill::DiagonalStripes(pattern) => {
            Brush::Stripes(crate::stripes::StripeBrush::new(*pattern, left, top, scale))
        }
        Fill::Checkerboard(pattern) => Brush::Checkerboard(crate::checkerboard::CheckerBrush::new(
            *pattern, left, top, scale,
        )),
        #[cfg(feature = "shader")]
        Fill::Shader(fill) => {
            let Some(sampler) =
                crate::bitmap::RegionSampler::shader(fill, [left, top], size, scale, clip)?
            else {
                return Ok(None);
            };
            Brush::Region(sampler)
        }
        Fill::CustomImage(image) => {
            let Some(sampler) =
                crate::bitmap::RegionSampler::new(image, [left, top], size, scale, clip)?
            else {
                return Ok(None);
            };
            Brush::Region(sampler)
        }
        Fill::Image(image) => {
            let mut sampler = crate::bitmap::ImageSampler::new(image, size, scale)?;
            sampler.prepare_rows([left, top], scale, clip);
            Brush::Image(sampler)
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
    }))
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

    pub(super) fn replay(
        &mut self,
        draws: &[Draw],
        scale: RasterScale,
        clip: Clip,
        caches: &mut ViewCaches,
    ) -> Result<()> {
        self.replay_commands(draws, scale, clip, &mut caches.replay())
    }

    fn replay_commands(
        &mut self,
        draws: &[Draw],
        scale: RasterScale,
        clip: Clip,
        replay: &mut Replay<'_>,
    ) -> Result<()> {
        for draw in draws {
            let visible = device_bounds(draw, scale)?;
            let bounds = visible.intersect(clip);
            if bounds.empty() {
                continue;
            }
            match draw {
                Draw::Group {
                    alpha,
                    gradient_map,
                    draws,
                } => {
                    self.composite_group(
                        draws,
                        *alpha,
                        gradient_map.as_ref(),
                        scale,
                        bounds,
                        replay,
                    )?;
                }
                Draw::Fill {
                    origin,
                    size,
                    fill,
                    corner_radii,
                    ..
                } => {
                    let fill = replay.images.resolve(fill).map_err(Error::other)?;
                    self.fill_box(*origin, *size, &fill, *corner_radii, scale, bounds)?;
                }
                Draw::BackdropBlur {
                    origin,
                    size,
                    radius,
                    corner_radii,
                    ..
                } => {
                    backdrop::draw(
                        self,
                        *origin,
                        *size,
                        *corner_radii,
                        f64::from(*radius) * scale.0,
                        scale,
                        (clip, bounds),
                    );
                }
                Draw::WavyUnderline {
                    origin,
                    size,
                    color,
                    ..
                } => {
                    underline::draw(self, *origin, *size, color, scale, bounds)?;
                }
                Draw::Shadow {
                    origin,
                    size,
                    corner_radii,
                    shadow,
                    ..
                } => {
                    shadow::Geometry::new(*origin, *size, *corner_radii, *shadow, scale)
                        .draw(self, bounds);
                }
                Draw::Border {
                    origin,
                    size,
                    paint,
                    ..
                } => {
                    self.draw_border(*origin, *size, paint, scale, bounds, replay.images)?;
                }
                Draw::Polygon {
                    origin,
                    contours,
                    fill,
                    rule,
                    ..
                } => {
                    let fill = replay.images.resolve(fill).map_err(Error::other)?;
                    self.fill_polygon(*origin, contours, &fill, *rule, scale, bounds)?;
                }
                Draw::Polyline {
                    origin,
                    contours,
                    closed,
                    width,
                    ink,
                    ..
                } => {
                    let contour = crate::polyline::StrokeContours {
                        rings: contours,
                        closed: *closed,
                    };
                    self.stroke_polyline(*origin, contour, *width, ink, scale, bounds)?;
                }
                Draw::Svg {
                    origin,
                    size,
                    asset,
                    tint,
                    rounded_clip,
                    ..
                } => {
                    let viewport = scale.bounds(*origin, *size);
                    let visible = visible.intersect(Clip::canvas(self.size));
                    let tint = tint
                        .as_ref()
                        .map(|ink| crate::mask::painter(ink, *origin, *size, scale, (0, 0), bounds))
                        .transpose()?;
                    let region = crate::svg::Viewport {
                        bounds: viewport,
                        visible,
                        corners: rounded_clip
                            .as_deref()
                            .map(|clip| crate::svg::CornerMask::new(clip, scale)),
                    };
                    replay.svgs.draw(self, asset, tint, region, bounds)?;
                }
                Draw::Text {
                    origin,
                    size: allocated,
                    paragraph,
                    color,
                    ..
                } => {
                    let raster = match &mut replay.text {
                        Some(raster) => raster,
                        None => {
                            replay
                                .text
                                .insert(TextRaster::new(scale, (0, 0), replay.masks.take()))
                        }
                    };
                    raster.draw(self, *origin, *allocated, paragraph, color, bounds)?;
                }
            }
        }
        Ok(())
    }

    fn composite_group(
        &mut self,
        draws: &[Draw],
        alpha: u8,
        gradient_map: Option<&solor::GradientMap>,
        scale: RasterScale,
        bounds: Clip,
        replay: &mut Replay<'_>,
    ) -> Result<()> {
        // Save the backdrop for the damaged pixels, then use their original device
        // coordinates to draw into transparent black. Limit temporary storage on
        // large surfaces by compositing one horizontal band at a time.
        let width = (bounds.right - bounds.left) as usize;
        let stride = self.size.width as usize * 4;
        let rows = if draws.iter().any(backdrop::reads_backdrop) {
            bounds.bottom - bounds.top
        } else {
            (65536 / width).max(1) as i64
        };
        let mut backdrop =
            Vec::with_capacity(width * (bounds.bottom - bounds.top).min(rows) as usize * 4);
        let mut sampler = gradient_map.map(|map| (map.palette().ramp_sampler(), map.strength()));
        for top in (bounds.top..bounds.bottom).step_by(rows as usize) {
            let band = Clip {
                top,
                bottom: (top + rows).min(bounds.bottom),
                ..bounds
            };
            let row_start = |y| y as usize * stride + bounds.left as usize * 4;
            backdrop.clear();
            for y in band.top..band.bottom {
                let start = row_start(y);
                let row = &mut self.rgba[start..start + width * 4];
                backdrop.extend_from_slice(row);
                row.fill(0);
            }
            self.replay_commands(draws, scale, band, replay)?;
            for (y, saved) in (band.top..band.bottom).zip(backdrop.chunks_exact(width * 4)) {
                let start = row_start(y);
                let pixels = self.rgba[start..start + width * 4].as_chunks_mut::<4>().0;
                if let Some((sample, strength)) = &mut sampler {
                    map_colors(pixels, sample, *strength)?;
                }
                for (pixel, &background) in pixels.iter_mut().zip(saved.as_chunks::<4>().0) {
                    if alpha == 255 && pixel[3] == 255 {
                        continue;
                    }
                    let layer =
                        pixel.map(|v| ((u16::from(v) * u16::from(alpha) + 127) / 255) as u8);
                    *pixel = background;
                    crate::pixels::blend_premultiplied(pixel, layer);
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
        let Some(mut brush) = brush(fill, origin, size, scale, clip)? else {
            return Ok(());
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
                Brush::Checkerboard(pattern) => pattern.blend_span(middle, from, y, 255),
                Brush::Stripes(pattern) => pattern.blend_span(middle, from, y, 255),
                Brush::Region(image) => image.blend_span(middle, from, y, scale),
                Brush::Image(image) => image.blend_span(middle, from, y, [left, top], scale),
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
                        brush.blend_at(pixel, x, y, [left, top], scale, coverage)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn opaque_cover(draw: &Draw, scale: RasterScale, area: Clip) -> Result<Option<solor::Rgba>> {
    let Draw::Fill {
        fill: Fill::Solid(color),
        origin,
        size,
        corner_radii,
        ..
    } = draw
    else {
        return Ok(None);
    };
    if color.alpha != 255 {
        return Ok(None);
    }
    let left = f64::from(origin.x) * scale.0;
    let top = f64::from(origin.y) * scale.0;
    let right = left + f64::from(size.width) * scale.0;
    let bottom = top + f64::from(size.height) * scale.0;
    let inset_x = (area.left as f64 - left).min(right - area.right as f64);
    let inset_y = (area.top as f64 - top).min(bottom - area.bottom as f64);
    if inset_x < 0.0 || inset_y < 0.0 {
        return Ok(None);
    }
    // Either central strip of a rounded rectangle has full coverage.
    let radius = corner_radii
        .top_left
        .max(corner_radii.top_right)
        .max(corner_radii.bottom_left)
        .max(corner_radii.bottom_right);
    if inset_x.max(inset_y) < f64::from(radius) * scale.0 {
        return Ok(None);
    }
    Ok((device_bounds(draw, scale)?.intersect(area) == area).then_some(*color))
}

// Fill coverage includes fractional edge pixels. Text is clipped to rounded layout edges.
fn device_bounds(draw: &Draw, scale: RasterScale) -> Result<Clip> {
    let mut bounds = primitive_bounds(draw, scale)?;
    let (Draw::WavyUnderline { clip, .. }
    | Draw::BackdropBlur { clip, .. }
    | Draw::Shadow { clip, .. }
    | Draw::Border { clip, .. }
    | Draw::Fill { clip, .. }
    | Draw::Polygon { clip, .. }
    | Draw::Polyline { clip, .. }
    | Draw::Svg { clip, .. }
    | Draw::Text { clip, .. }) = draw
    else {
        return Ok(bounds);
    };
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

fn draw_bounds(draws: &[Draw], scale: RasterScale) -> Result<Clip> {
    let mut total: Option<Clip> = None;
    for draw in draws {
        let bounds = device_bounds(draw, scale)?;
        if !bounds.empty() {
            total = Some(total.map_or(bounds, |old| old.union(bounds)));
        }
    }
    Ok(total.unwrap_or(Clip {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }))
}

fn primitive_bounds(draw: &Draw, scale: RasterScale) -> Result<Clip> {
    match draw {
        Draw::Group { draws, .. } => draw_bounds(draws, scale),
        Draw::Polygon {
            origin, contours, ..
        } => crate::path::PathBounds::new(*origin, contours, scale)?.clip(0.0),
        Draw::Polyline {
            origin,
            contours,
            width,
            ..
        } => crate::polyline::bounds(*origin, contours, *width, scale),
        Draw::Shadow {
            origin,
            size,
            corner_radii,
            shadow,
            ..
        } => Ok(shadow::Geometry::new(*origin, *size, *corner_radii, *shadow, scale).bounds()),
        Draw::Svg { origin, size, .. } | Draw::Text { origin, size, .. } => {
            Ok(scale.bounds(*origin, *size))
        }
        Draw::WavyUnderline { origin, size, .. } => {
            box_bounds(*origin, *size, Corners::default(), scale)
        }
        Draw::Border {
            origin,
            size,
            paint,
            ..
        } => box_bounds(*origin, *size, paint.radii, scale),
        Draw::Fill {
            origin,
            size,
            corner_radii,
            ..
        }
        | Draw::BackdropBlur {
            origin,
            size,
            corner_radii,
            ..
        } => box_bounds(*origin, *size, *corner_radii, scale),
    }
}

fn box_bounds(
    origin: Origin,
    size: Size,
    corner_radii: Corners<Pixels>,
    scale: RasterScale,
) -> Result<Clip> {
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

fn map_colors(
    mut row: &mut [[u8; 4]],
    sample: &mut impl FnMut(f32) -> std::result::Result<solor::Rgba, solor::GradientError>,
    strength: f32,
) -> Result<()> {
    while let Some(&source) = row.first() {
        let run = row.iter().take_while(|&&p| p == source).count();
        let (pixels, rest) = row.split_at_mut(run);
        row = rest;
        let [r, g, b, a] = source.map(u32::from);
        if a == 0 {
            continue;
        }
        let luminance = (2126 * r + 7152 * g + 722 * b) as f32 / (10000 * a) as f32;
        let color = sample(luminance).map_err(Error::other)?;
        if a == 255 && color.alpha == 255 && strength == 1.0 {
            pixels.fill([color.red, color.green, color.blue, 255]);
            continue;
        }
        let alpha = color.alpha as f32 * a as f32 / 255.0;
        let mapped = [color.red, color.green, color.blue, 255].map(|v| v as f32 * (alpha / 255.0));
        let result = std::array::from_fn(|i| {
            let original = f32::from(source[i]);
            (original + (mapped[i] - original) * strength).round() as u8
        });
        pixels.fill(result);
    }
    Ok(())
}

fn same_draw(a: &Draw, b: &Draw) -> bool {
    match (a, b) {
        (
            Draw::Group {
                alpha,
                gradient_map,
                draws,
            },
            Draw::Group {
                alpha: other,
                gradient_map: other_map,
                draws: children,
            },
        ) => {
            alpha == other
                && gradient_map == other_map
                && draws.len() == children.len()
                && draws.iter().zip(children).all(|(a, b)| same_draw(a, b))
        }
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

fn include_fill_change(
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
            fill: old_fill,
            corner_radii,
            clip,
        },
        Draw::Fill {
            origin: at,
            size: extent,
            fill: new_fill,
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
    #[cfg(feature = "shader")]
    if let (Fill::Shader(old), Fill::Shader(new)) = (old_fill, new_fill) {
        let (Some(a), Some(b)) = (old.region(), new.region()) else {
            return Ok(false);
        };
        if old.shader != new.shader {
            return Ok(false);
        }
        let visible = device_bounds(after, scale)?.intersect(canvas);
        for region in [a, b] {
            if !(0..2).all(|i| {
                region.start[i].is_finite()
                    && region.end[i].is_finite()
                    && region.start[i] <= region.end[i]
            }) {
                return Ok(false);
            }
            let x = |u: f32| (f64::from(origin.x) + f64::from(size.width) * f64::from(u)) * scale.0;
            let y =
                |v: f32| (f64::from(origin.y) + f64::from(size.height) * f64::from(v)) * scale.0;
            damage.include(
                Clip {
                    left: x(region.start[0]).floor() as i64,
                    right: x(region.end[0]).ceil() as i64,
                    top: y(region.start[1]).floor() as i64,
                    bottom: y(region.end[1]).ceil() as i64,
                }
                .intersect(visible),
            );
        }
        return Ok(true);
    }
    let (Fill::CustomImage(previous), Fill::CustomImage(next)) = (old_fill, new_fill) else {
        return Ok(false);
    };
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

fn include_changes(
    old: &[Draw],
    new: &[Draw],
    scale: RasterScale,
    canvas: Clip,
    area: &mut Damage,
    on_change: &mut dyn FnMut(usize, &Draw) -> Result<()>,
) -> Result<bool> {
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
    let mut changed = old.len() != new.len();
    for i in 0..old.len().max(new.len()) {
        let (before, after) = (old.get(i), new.get(i));
        if let Some((a, b)) = before.zip(after) {
            if let (
                Draw::Group {
                    alpha,
                    gradient_map,
                    draws,
                },
                Draw::Group {
                    alpha: other,
                    gradient_map: other_map,
                    draws: children,
                },
            ) = (a, b)
                && alpha == other
                && gradient_map == other_map
            {
                if include_changes(draws, children, scale, canvas, area, &mut |_, _| Ok(()))? {
                    changed = true;
                    on_change(i, b)?;
                }
                continue;
            }
            if same_draw(a, b) {
                continue;
            }
            changed = true;
            on_change(i, b)?;
            if include_fill_change(a, b, scale, canvas, area)? {
                continue;
            }
        }
        for draw in before.into_iter().chain(after) {
            let bounds = device_bounds(draw, scale)?.intersect(canvas);
            if let Draw::Border {
                origin,
                size,
                paint,
                ..
            } = draw
            {
                for (fill, edge) in paint
                    .visible_fills()
                    .into_iter()
                    .zip(border::edge_clips(*origin, *size, paint, scale, bounds))
                {
                    if fill.is_some() {
                        area.include(edge);
                    }
                }
            } else {
                area.include(bounds);
            }
        }
    }
    Ok(changed)
}

fn rectangle_coverage(x: f64, width: f64, row_height: f64) -> u8 {
    let w = ((x + 1.0).min(width) - x.max(0.0)).clamp(0.0, 1.0);
    (w * row_height * 255.0).round() as u8
}

// Sample only the corner regions.
pub(super) fn coverage(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    row_height: f64,
    radii: Corners<f64>,
) -> u8 {
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
