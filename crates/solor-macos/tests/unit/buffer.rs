use super::*;
use objc2_core_graphics::{CGImage, CGImageAlphaInfo};
use solor::{platform::WindowBackground, raster::DeviceSize};

#[test]
fn creates_opaque_and_transparent_cg_images() {
    let size = DeviceSize {
        width: 2,
        height: 2,
    };
    let rgba = [255_u8; 2 * 2 * 4];

    let opaque = create_cg_image(&rgba, size, WindowBackground::Opaque).expect("opaque CGImage");
    assert_eq!(CGImage::width(Some(&opaque)), 2);
    assert_eq!(CGImage::height(Some(&opaque)), 2);
    assert_eq!(
        CGImage::alpha_info(Some(&opaque)),
        CGImageAlphaInfo::NoneSkipLast
    );

    let transparent =
        create_cg_image(&rgba, size, WindowBackground::Transparent).expect("transparent CGImage");
    assert_eq!(
        CGImage::alpha_info(Some(&transparent)),
        CGImageAlphaInfo::PremultipliedLast
    );
}

#[test]
fn rejects_mismatched_pixel_buffers() {
    let size = DeviceSize {
        width: 2,
        height: 2,
    };
    assert!(create_cg_image(&[0_u8; 12], size, WindowBackground::Opaque).is_err());
    assert!(
        create_cg_image(
            &[],
            DeviceSize {
                width: 0,
                height: 2,
            },
            WindowBackground::Opaque
        )
        .is_err()
    );
}
