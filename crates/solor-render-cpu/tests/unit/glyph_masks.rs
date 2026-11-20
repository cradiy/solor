#[test]
fn budgets_retain_small_masks_bypass_large_masks_and_distinguish_faces() {
    use crate::glyph_masks::GlyphMasks;
    use swash::scale::image::Image;
    let mut cache = GlyphMasks::new(3, 32);
    let mut scratch = Image::new();
    for glyph in 1..=3 {
        cache
            .resolve(1, 16, glyph, &mut scratch, |m| {
                m.data = vec![glyph as u8; 16];
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(cache.usage(), (2, 32));
    let large = cache
        .resolve(1, 16, 7, &mut scratch, |m| {
            m.data = vec![42; 64];
            Ok(())
        })
        .unwrap();
    assert_eq!(large.data.len(), 64);
    assert_eq!(cache.usage(), (2, 32));
    let hit = cache
        .resolve(1, 16, 3, &mut scratch, |_| panic!("expected hit"))
        .unwrap();
    assert_eq!(hit.data[0], 3);
    cache
        .resolve(1, 16, 8, &mut scratch, |m| {
            m.data.resize(8, 1);
            Ok(())
        })
        .unwrap();
    assert_eq!(cache.usage(), (2, 24));
    cache
        .resolve(1, 16, 8, &mut scratch, |_| {
            panic!("small mask should be retained")
        })
        .unwrap();
    let before = cache.usage();
    assert!(
        cache
            .resolve(2, 16, 3, &mut scratch, |_| Err(std::io::Error::other(
                "missing outline"
            )))
            .is_err()
    );
    assert_eq!(cache.usage(), before);
}
