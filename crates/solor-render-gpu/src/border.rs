use super::{GradientBrush, Material, rgba};
use solor::{
    Fill,
    layout::{Origin, Size},
    raster::{BorderMarks, DeviceSize, RasterScale},
    style::{BorderPaint, BorderStyle},
};

pub(super) fn quad(
    origin: Origin,
    extent: Size,
    paint: &BorderPaint,
    bounds: [f32; 4],
    size: DeviceSize,
    scale: RasterScale,
    stops: &mut Vec<[[f32; 4]; 2]>,
) -> Result<([[f32; 4]; 6], Material), &'static str> {
    let fills = paint.visible_fills();
    let mut edge_paints = [[0.; 4]; 4];
    let mut gradient_mask = 0u32;
    for (side, fill) in fills.iter().copied().enumerate() {
        let Some(fill) = fill else { continue };
        edge_paints[side] = match fill {
            Fill::Solid(color) => {
                let [r, g, b, a] = rgba(*color);
                [r * a, g * a, b * a, a]
            }
            Fill::Gradient(gradient) => {
                gradient_mask |= 1 << side;
                if let Some(previous) = fills[..side].iter().position(|f| *f == Some(fill)) {
                    edge_paints[previous]
                } else {
                    let brush = GradientBrush::prepare(gradient, extent, stops);
                    let index = stops.len();
                    stops.push([brush.params, brush.ramp]);
                    [index as f32, brush.oklab, 0., 0.]
                }
            }
            _ => return Err("unsupported border fill"),
        };
    }

    let paints_start = stops.len() as f32;
    let [t, r, b, l] = edge_paints;
    stops.extend([[t, r], [b, l]]);
    let r = paint.radii.fit_to(extent);
    let w = paint.widths;
    let radii = [r.top_left, r.top_right, r.bottom_right, r.bottom_left].map(solor::Pixels::get);
    let widths = [w.top, w.right, w.bottom, w.left].map(solor::Pixels::get);
    let pattern_index = record_patterns(paint, extent, widths, radii, scale, stops);
    let quad = [
        [
            origin.x.get(),
            origin.y.get(),
            extent.width.get(),
            extent.height.get(),
        ],
        radii,
        bounds,
        widths,
        [size.width as f32, size.height as f32, scale.0 as f32, 0.],
        [paints_start, gradient_mask as f32, pattern_index, 0.],
    ];
    Ok((
        quad,
        Material::Border {
            patterned: pattern_index != 0.,
        },
    ))
}

fn record_patterns(
    paint: &BorderPaint,
    extent: Size,
    widths: [f32; 4],
    radii: [f32; 4],
    scale: RasterScale,
    stops: &mut Vec<[[f32; 4]; 2]>,
) -> f32 {
    let styles = paint.styles;
    let styles = [styles.top, styles.right, styles.bottom, styles.left];
    let flags = styles.map(|s| match s {
        BorderStyle::Dashed => 1.,
        BorderStyle::Dotted => 2.,
        _ => 0.,
    });
    if flags == [0.; 4] {
        return 0.;
    }
    let device = |v: f32| f64::from(v) * scale.0;
    let radii = radii.map(device);
    let widths = widths.map(device);
    let lengths = [
        f64::from(extent.width) * scale.0,
        f64::from(extent.height) * scale.0,
    ];
    let fills = paint.visible_fills();
    let sides = std::array::from_fn(|side| {
        if flags[side] == 0. || fills[side].is_none() {
            return [0.; 4];
        }
        let next = (side + 1) % 4;
        let Some(marks) = BorderMarks::fit(
            styles[side],
            lengths[side % 2],
            widths[side],
            [radii[side], radii[next]],
            [widths[(side + 3) % 4], widths[next]],
        ) else {
            return [0.; 4];
        };
        [
            marks.first as f32,
            marks.period as f32,
            marks.intervals as f32,
            flags[side],
        ]
    });
    if sides == [[0.; 4]; 4] {
        return 0.;
    }
    let index = stops.len();
    let [t, r, b, l] = sides;
    stops.extend([[t, r], [b, l]]);
    index as f32
}
