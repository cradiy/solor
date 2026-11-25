#[test]
fn budgets_retain_small_masks_bypass_large_masks_and_distinguish_faces() {
    use crate::glyph_masks::GlyphMasks;
    use solor::raster::glyphs::GlyphMask as Image;
    let mut cache = GlyphMasks::new(3, 32);
    let mut scratch = Image::new();
    for glyph in 1..=3 {
        cache
            .resolve(1, 16, glyph, 0, &mut scratch, |m| {
                m.data = vec![glyph as u8; 16];
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(cache.usage(), (2, 32));
    let large = cache
        .resolve(1, 16, 7, 0, &mut scratch, |m| {
            m.data = vec![42; 64];
            Ok(())
        })
        .unwrap();
    assert_eq!(large.data.len(), 64);
    assert_eq!(cache.usage(), (2, 32));
    let hit = cache
        .resolve(1, 16, 3, 0, &mut scratch, |_| panic!("expected hit"))
        .unwrap();
    assert_eq!(hit.data[0], 3);
    cache
        .resolve(1, 16, 8, 0, &mut scratch, |m| {
            m.data.resize(8, 1);
            Ok(())
        })
        .unwrap();
    assert_eq!(cache.usage(), (2, 24));
    cache
        .resolve(1, 16, 8, 0, &mut scratch, |_| {
            panic!("small mask should be retained")
        })
        .unwrap();
    let before = cache.usage();
    assert!(
        cache
            .resolve(2, 16, 3, 0, &mut scratch, |_| Err(std::io::Error::other(
                "missing outline"
            )))
            .is_err()
    );
    assert_eq!(cache.usage(), before);
}

#[test]
fn cached_outline_follows_fractional_baseline_across_pixel_boundaries() -> anyhow::Result<()> {
    use crate::{
        frame::{Frame, TextDraw},
        glyph_masks::GlyphMasks,
        pixels::Surface,
    };
    use solor::{
        Ui,
        layout::{Origin, Query},
        px,
        raster::{Clip, DeviceSize},
        rgb,
        tree::Tree,
    };

    let mut ui = Ui::with_font(
        include_bytes!("../../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let paragraph = ui
        .context()
        .shape_text("A", &Default::default(), Query::Natural)?;
    let size = DeviceSize {
        width: 40,
        height: 42,
    };
    let mut frame = Frame {
        revision: 0,
        scroll_y: px(0.),
        geometry: [].into(),
        draws: vec![TextDraw {
            node: Tree::new(()).root(),
            origin: Origin::default(),
            allocated: paragraph.content,
            color: rgb(0),
            paragraph,
        }],
    };
    let mut masks = GlyphMasks::new(16, 8192);
    let mut first = None;
    for shift in [0., 0.25, 0.5, 0.75, 1., 0.25] {
        frame.draws[0].origin.y = px(4. + shift);
        let image = Surface::draw_cached(&frame, size, Clip::canvas(size), &mut masks)?;
        let mut ink = 0.;
        let mut moment = 0.;
        for (y, row) in image.rgba.chunks_exact(size.width as usize * 4).enumerate() {
            let mass: f64 = row
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| f64::from(255 - p[0]))
                .sum();
            ink += mass;
            moment += y as f64 * mass;
        }
        let center = moment / ink;
        let displacement = center - *first.get_or_insert(center);
        assert!(
            (displacement - f64::from(shift)).abs() < 0.06,
            "{shift}: {displacement}"
        );
    }
    Ok(())
}
