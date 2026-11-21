use super::*;

#[test]
fn delayed_slot_repairs_disjoint_edits_and_retries_after_io_error() {
    for order in [PixelOrder::Bgr, PixelOrder::Rgb] {
        for (width, expected_bytes) in [(5, 68), (1024, 8 * 4)] {
            let size = DeviceSize { width, height: 6 };
            let stride = width as usize * 4;
            let mut rgba = vec![255; stride * 6];
            fn frame(size: DeviceSize, rgba: &[u8]) -> RasterFrame<'_> {
                RasterFrame {
                    size,
                    rgba,
                    damage: Default::default(),
                    request_callback: false,
                }
            }
            let file = File::from(memfd_create("solor-test", MemfdFlags::CLOEXEC).unwrap());
            let mut dirty = PendingDamage(Damage::from_rect(Clip::canvas(size)));
            assert_eq!(
                dirty.write(&file, &frame(size, &rgba), order).unwrap(),
                rgba.len()
            );

            // The slot misses three edits in different rows and columns.
            for (x, y) in [(2, 4), (1, 1), (2, 3)] {
                let start = y * stride + x * 4;
                rgba[start..start + 4].copy_from_slice(&[y as u8, 20, 80, 127]);
                dirty.include(Clip {
                    left: x as i64,
                    top: y as i64,
                    right: x as i64 + 1,
                    bottom: y as i64 + 1,
                });
            }
            let read_only = File::open("/dev/null").unwrap();
            assert!(dirty.write(&read_only, &frame(size, &rgba), order).is_err());
            assert_eq!(
                dirty.write(&file, &frame(size, &rgba), order).unwrap(),
                expected_bytes
            );
            assert_eq!(dirty.write(&file, &frame(size, &rgba), order).unwrap(), 0);

            let mut data = vec![0; rgba.len()];
            file.read_exact_at(&mut data, 0).unwrap();
            for (word, rgba) in data.as_chunks::<4>().0.iter().zip(rgba.as_chunks::<4>().0) {
                let expected = match order {
                    PixelOrder::Rgb => *rgba,
                    PixelOrder::Bgr => [rgba[2], rgba[1], rgba[0], rgba[3]],
                };
                assert_eq!(*word, expected);
            }
        }
    }
}

#[test]
fn sparse_upload_skips_distant_gaps_on_every_row() {
    let size = DeviceSize {
        width: 4096,
        height: 8,
    };
    let rgba = vec![143; size.width as usize * size.height as usize * 4];
    let frame = RasterFrame {
        size,
        rgba: &rgba,
        damage: Damage::default(),
        request_callback: false,
    };
    let file = File::from(memfd_create("solor-test", MemfdFlags::CLOEXEC).unwrap());
    file.set_len(rgba.len() as u64).unwrap();
    let mut dirty = PendingDamage(Damage::default());
    for left in [10, 3000] {
        dirty.include(Clip {
            left,
            top: 2,
            right: left + 8,
            bottom: 5,
        });
    }
    let count = dirty.write(&file, &frame, PixelOrder::Rgb).unwrap();
    assert_eq!(count, 2 * 3 * 8 * 4);
    let mut result = vec![0; rgba.len()];
    file.read_exact_at(&mut result, 0).unwrap();
    for (i, pixel) in result.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % 4096, i / 4096);
        let changed = (2..5).contains(&y) && ((10..18).contains(&x) || (3000..3008).contains(&x));
        assert_eq!(*pixel, if changed { [143; 4] } else { [0; 4] });
    }
}
