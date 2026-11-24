use super::{Clip, Damage, Draw, Fill, Origin, RasterScale, Result, Size, device_bounds};
use solor::{ContentClip, Ink, ShaderFill};
use std::rc::Rc;

#[derive(PartialEq)]
enum Mask<'a> {
    Box(&'a solor::style::Corners<solor::Pixels>),
    Glyphs(*const solor::text::Paragraph),
    Svg(&'a solor::SvgAsset, Option<&'a solor::RoundedClip>),
    Wave,
}

#[derive(PartialEq)]
struct Geometry<'a> {
    origin: Origin,
    size: Size,
    clip: Option<&'a ContentClip>,
    mask: Mask<'a>,
}

fn shader_paint(draw: &Draw) -> Option<(Geometry<'_>, &ShaderFill)> {
    let (origin, size, clip, mask, shader) = match draw {
        Draw::Fill {
            origin,
            size,
            clip,
            corner_radii,
            fill: Fill::Shader(shader),
        } => (origin, size, clip, Mask::Box(corner_radii), shader),
        Draw::Text {
            origin,
            size,
            clip,
            paragraph,
            color: Ink::Shader(shader),
        } => (
            origin,
            size,
            clip,
            Mask::Glyphs(Rc::as_ptr(paragraph)),
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
            size,
            clip,
            Mask::Svg(asset, rounded_clip.as_deref()),
            shader,
        ),
        Draw::WavyUnderline {
            origin,
            size,
            clip,
            color: Ink::Shader(shader),
        } => (origin, size, clip, Mask::Wave, shader),
        _ => return None,
    };
    Some((
        Geometry {
            origin: *origin,
            size: *size,
            clip: clip.as_deref(),
            mask,
        },
        shader,
    ))
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
    if previous != current || old.shader != new.shader {
        return Ok(false);
    }
    let (Some(a), Some(b)) = (old.region(), new.region()) else {
        return Ok(false);
    };
    let Geometry { origin, size, .. } = current;
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
        let y = |v: f32| (f64::from(origin.y) + f64::from(size.height) * f64::from(v)) * scale.0;
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
    Ok(true)
}
