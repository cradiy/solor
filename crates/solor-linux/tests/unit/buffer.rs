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
                    damage: None,
                    request_callback: false,
                }
            }
            let file = File::from(memfd_create("solor-test", MemfdFlags::CLOEXEC).unwrap());
            let mut dirty = PendingDamage(Some(Clip::canvas(size)));
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
                    PixelOrder::Bgr => [rgba[2], rgba[1], rgba[0], 255],
                };
                assert_eq!(*word, expected);
            }
        }
    }
}
