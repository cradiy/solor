use solor::{
    ContentClip,
    raster::{BoxShadowGeometry, DeviceSize, RasterScale},
};

pub(super) fn quad(
    geometry: &BoxShadowGeometry,
    clip: Option<&ContentClip>,
    size: DeviceSize,
    scale: RasterScale,
    data: &mut Vec<[[f32; 4]; 2]>,
) -> Option<[[f32; 4]; 6]> {
    let bounds = super::clipped_pixels(geometry.bounds(), clip, size, scale);
    if bounds.empty() {
        return None;
    }
    let index = data.len();
    data.push([
        geometry.caster.bounds.map(|v| v as f32),
        geometry.caster.corners.map(|r| r[0] as f32),
    ]);
    Some([
        [
            bounds.left as f32,
            bounds.top as f32,
            (bounds.right - bounds.left) as f32,
            (bounds.bottom - bounds.top) as f32,
        ],
        geometry.base.corners.map(|r| r[0] as f32),
        geometry.base.bounds.map(|v| v as f32),
        super::rgba(geometry.color),
        [
            size.width as f32,
            size.height as f32,
            1.,
            f32::from(geometry.inset),
        ],
        [index as f32, geometry.blur as f32, 0., 0.],
    ])
}
