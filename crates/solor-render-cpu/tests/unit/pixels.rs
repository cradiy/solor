use super::{MAX_SURFACE_PIXELS, Surface};
use solor::raster::{DeviceSize, RasterScale};

#[test]
fn surface_byte_len_accepts_retina_5k_and_rejects_out_of_range_extents() {
    let four_k = DeviceSize {
        width: 3840,
        height: 2160,
    };
    assert_eq!(
        Surface::byte_len(four_k, RasterScale(2.0)).unwrap(),
        3840 * 2160 * 4
    );

    let five_k = DeviceSize {
        width: 5120,
        height: 2880,
    };
    assert_eq!(
        Surface::byte_len(five_k, RasterScale(2.0)).unwrap(),
        5120 * 2880 * 4
    );

    assert!(
        Surface::byte_len(
            DeviceSize {
                width: 0,
                height: 100
            },
            RasterScale::ONE,
        )
        .is_err()
    );
    assert!(
        Surface::byte_len(
            DeviceSize {
                width: (MAX_SURFACE_PIXELS as u32) + 1,
                height: 1,
            },
            RasterScale::ONE,
        )
        .is_err()
    );
}
