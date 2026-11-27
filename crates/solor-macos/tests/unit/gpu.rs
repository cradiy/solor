use super::{budgeted_gpu_target, select_alpha_mode, select_surface_format};
use solor::{layout::Size, platform::WindowBackground, px, raster::RasterScale};
use wgpu::{Color, CompositeAlphaMode, TextureFormat};

#[test]
fn surface_format_prefers_unorm_targets() {
    assert_eq!(
        select_surface_format(&[TextureFormat::Bgra8UnormSrgb, TextureFormat::Bgra8Unorm]),
        Some(TextureFormat::Bgra8Unorm)
    );
    assert_eq!(
        select_surface_format(&[TextureFormat::Rgba8Unorm]),
        Some(TextureFormat::Rgba8Unorm)
    );
    assert_eq!(
        select_surface_format(&[TextureFormat::Bgra8UnormSrgb, TextureFormat::Rgba16Float]),
        None
    );
}

#[test]
fn alpha_mode_supports_metal_post_multiplied_transparency() {
    let metal_modes = [
        CompositeAlphaMode::Opaque,
        CompositeAlphaMode::PostMultiplied,
    ];
    let (opaque_mode, opaque_clear) =
        select_alpha_mode(WindowBackground::Opaque, &metal_modes).unwrap();
    assert_eq!(opaque_mode, CompositeAlphaMode::Opaque);
    assert_eq!(opaque_clear, Color::WHITE);

    let (transparent_mode, transparent_clear) =
        select_alpha_mode(WindowBackground::Transparent, &metal_modes).unwrap();
    assert_eq!(transparent_mode, CompositeAlphaMode::PostMultiplied);
    assert_eq!(transparent_clear, Color::TRANSPARENT);

    let premultiplied = [
        CompositeAlphaMode::Opaque,
        CompositeAlphaMode::PreMultiplied,
        CompositeAlphaMode::PostMultiplied,
    ];
    let (preferred, _) = select_alpha_mode(WindowBackground::Transparent, &premultiplied).unwrap();
    assert_eq!(preferred, CompositeAlphaMode::PreMultiplied);

    assert!(
        select_alpha_mode(WindowBackground::Transparent, &[CompositeAlphaMode::Opaque]).is_err()
    );
}

#[test]
fn gpu_target_preserves_6k_retina_and_clamps_huge_extents() {
    let xdr_6k = Size {
        width: px(3008.0),
        height: px(1692.0),
    };
    let (scale, physical) = budgeted_gpu_target(xdr_6k, RasterScale(2.0), 16384).unwrap();
    assert_eq!(scale, RasterScale(2.0));
    assert_eq!(physical.width, 6016);
    assert_eq!(physical.height, 3384);

    let oversized = Size {
        width: px(12000.0),
        height: px(3000.0),
    };
    let (clamped_scale, clamped_size) =
        budgeted_gpu_target(oversized, RasterScale(2.0), 16384).unwrap();
    assert!(clamped_scale.0 > 0.0 && clamped_scale.0 < 2.0);
    assert!(clamped_size.width <= 16384);
    assert!(clamped_size.height <= 16384);
}
