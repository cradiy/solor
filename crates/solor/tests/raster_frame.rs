//! Validation of renderer output before a platform reads its pixel rows.
use solor::raster::{Clip, DeviceSize, RasterFrame};

#[test]
fn upload_checks_extent_storage_and_damage_before_accessing_rows() {
    let size = DeviceSize {
        width: 2,
        height: 3,
    };
    let mut frame = RasterFrame {
        size,
        rgba: &[0; 24],
        damage: None,
        request_callback: false,
    };
    frame.validate(size).unwrap();
    assert!(
        frame
            .validate(DeviceSize {
                width: 3,
                height: 2
            })
            .is_err()
    );
    frame.rgba = &[0; 23];
    assert!(frame.validate(size).is_err());
    frame.rgba = &[0; 24];
    frame.damage = Some(Clip {
        left: 1,
        ..Clip::canvas(size)
    });
    frame.validate(size).unwrap();
    frame.damage.as_mut().unwrap().right = 3;
    assert!(frame.validate(size).is_err());
    frame.size = DeviceSize {
        width: u32::MAX,
        height: u32::MAX,
    };
    assert!(frame.validate(frame.size).is_err());
}
