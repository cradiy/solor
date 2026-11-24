use super::{
    Clip, Damage, Draw, Fill, Origin, RasterScale, Result, Size, clip_to_content, device_bounds,
};
use crate::path::PathBounds;
use solor::{ContentClip, FillRule, Ink, Pixels, ShaderFill};
use std::{ops::Range, rc::Rc};

#[derive(PartialEq)]
enum Mask<'a> {
    Box(Size, &'a solor::style::Corners<Pixels>),
    Glyphs(Size, *const solor::text::Paragraph),
    Svg(Size, &'a solor::SvgAsset, Option<&'a solor::RoundedClip>),
    Wave(Size),
    Polygon(&'a [Rc<[Origin]>], FillRule),
    Stroke(&'a [Rc<[Origin]>], Pixels, bool),
}

#[derive(PartialEq)]
struct Geometry<'a> {
    origin: Origin,
    clip: Option<&'a ContentClip>,
    mask: Mask<'a>,
}

impl Geometry<'_> {
    fn paint_box(&self, scale: RasterScale) -> Result<([f64; 2], Size)> {
        let (origin, size) = match self.mask {
            Mask::Polygon(contours, _) => {
                let bounds = PathBounds::new(self.origin, contours, scale)?;
                return Ok(([bounds.left, bounds.top], bounds.logical_size(scale)));
            }
            Mask::Stroke(contours, width, _) => {
                crate::polyline::paint_box(self.origin, contours, width)?
            }
            Mask::Box(size, _) | Mask::Glyphs(size, _) | Mask::Svg(size, ..) | Mask::Wave(size) => {
                (self.origin, size)
            }
        };
        Ok((
            [f64::from(origin.x) * scale.0, f64::from(origin.y) * scale.0],
            size,
        ))
    }
}

fn shader_paint(draw: &Draw) -> Option<(Geometry<'_>, &ShaderFill)> {
    let (origin, clip, mask, shader) = match draw {
        Draw::Fill {
            origin,
            size,
            clip,
            corner_radii,
            fill: Fill::Shader(shader),
        } => (origin, clip, Mask::Box(*size, corner_radii), shader),
        Draw::Text {
            origin,
            size,
            clip,
            paragraph,
            color: Ink::Shader(shader),
        } => (
            origin,
            clip,
            Mask::Glyphs(*size, Rc::as_ptr(paragraph)),
            shader,
        ),
        Draw::Svg {
            origin,
            size,
            clip,
            rounded_clip,
            asset,
            tint: Some(Ink::Shader(shader)),
        } => (
            origin,
            clip,
            Mask::Svg(*size, asset, rounded_clip.as_deref()),
            shader,
        ),
        Draw::WavyUnderline {
            origin,
            size,
            clip,
            color: Ink::Shader(shader),
        } => (origin, clip, Mask::Wave(*size), shader),
        Draw::Polygon {
            origin,
            clip,
            contours,
            rule,
            fill: Fill::Shader(shader),
        } => (origin, clip, Mask::Polygon(contours, *rule), shader),
        Draw::Polyline {
            origin,
            clip,
            contours,
            closed,
            width,
            ink: Ink::Shader(shader),
        } => (
            origin,
            clip,
            Mask::Stroke(contours, *width, *closed),
            shader,
        ),
        _ => return None,
    };
    Some((
        Geometry {
            origin: *origin,
            clip: clip.as_deref(),
            mask,
        },
        shader,
    ))
}

pub(super) fn changed_regions<'a>(
    old: &'a ShaderFill,
    new: &'a ShaderFill,
) -> Option<[&'a Range<[f32; 2]>; 2]> {
    let regions = [old.region()?, new.region()?];
    (old.shader == new.shader
        && regions.iter().all(|r| {
            (0..2).all(|i| r.start[i].is_finite() && r.end[i].is_finite() && r.start[i] <= r.end[i])
        }))
    .then_some(regions)
}

pub(super) fn project(
    [left, top]: [f64; 2],
    size: Size,
    scale: RasterScale,
    uv: &Range<[f32; 2]>,
) -> Clip {
    let x = |u: f32| left + f64::from(size.width) * scale.0 * f64::from(u);
    let y = |v: f32| top + f64::from(size.height) * scale.0 * f64::from(v);
    Clip {
        left: x(uv.start[0]).floor() as i64,
        right: x(uv.end[0]).ceil() as i64,
        top: y(uv.start[1]).floor() as i64,
        bottom: y(uv.end[1]).ceil() as i64,
    }
}

pub(super) fn include(
    before: &Draw,
    after: &Draw,
    scale: RasterScale,
    canvas: Clip,
    damage: &mut Damage,
) -> Result<bool> {
    let Some((previous, old)) = shader_paint(before) else {
        return Ok(false);
    };
    let Some((current, new)) = shader_paint(after) else {
        return Ok(false);
    };
    let Some(regions) = changed_regions(old, new) else {
        return Ok(false);
    };
    if previous != current {
        return Ok(false);
    }
    let (origin, size) = current.paint_box(scale)?;
    let project = |region| project(origin, size, scale, region);
    let visible = match current.mask {
        Mask::Polygon(..) | Mask::Stroke(..) => {
            clip_to_content(project(&([0.; 2]..[1.; 2])), current.clip, scale)
        }
        _ => device_bounds(after, scale)?,
    }
    .intersect(canvas);
    for region in regions {
        damage.include(project(region).intersect(visible));
    }
    Ok(true)
}
