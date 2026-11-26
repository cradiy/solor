//! Encoded bitmap inputs share the same premultiplied pixel contract as raw pixels.
use anyhow::Result;
use solor::ImageAsset;

#[cfg(feature = "png")]
#[test]
fn png_preserves_translucent_pixels() -> Result<()> {
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(&[240, 100, 20, 128], 1, 1, ExtendedColorType::Rgba8)?;
    let asset = ImageAsset::from_bytes(&png)?;
    assert_eq!(asset.pixels(), &[[120, 50, 10, 128]]);
    assert_eq!(asset.clone(), asset);
    assert!(ImageAsset::from_bytes(&png[..12]).is_err());
    Ok(())
}

#[cfg(feature = "jpeg")]
#[test]
fn jpeg_applies_exif_orientation() -> Result<()> {
    use image::{ExtendedColorType, ImageEncoder, codecs::jpeg::JpegEncoder};
    use solor::raster::DeviceSize;
    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg).write_image(&[97; 6], 2, 1, ExtendedColorType::Rgb8)?;
    // An EXIF IFD with orientation 6 rotates the two-pixel scanline clockwise.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let len = u16::try_from(exif.len() + 2)?.to_be_bytes();
    jpeg.splice(
        2..2,
        [0xff, 0xe1, len[0], len[1]]
            .into_iter()
            .chain(exif.iter().copied()),
    );
    let photo = ImageAsset::from_bytes(&jpeg)?;
    assert_eq!(
        photo.dimensions(),
        DeviceSize {
            width: 1,
            height: 2
        }
    );
    assert!(
        photo
            .pixels()
            .iter()
            .all(|p| p[3] == 255 && p[0].abs_diff(97) <= 1)
    );
    Ok(())
}

#[test]
fn raw_rgba_validates_pixel_count_and_premultiplies_alpha() -> Result<()> {
    assert!(ImageAsset::from_rgba8(2, 1, vec![0; 4]).is_err());
    let asset = ImageAsset::from_rgba8(1, 1, vec![32, 64, 255, 128])?;
    assert_eq!(asset.pixels(), &[[16, 32, 128, 128]]);
    Ok(())
}

#[cfg(feature = "webp")]
#[test]
fn file_element_repaints_at_webp_deadlines_and_rests_on_the_final_frame() -> Result<()> {
    use solor::{Draw, Fill, Ui, component, image, layout::Size, px};
    use std::{path::Path, time::Duration};

    // Two passes, each with a 70 ms red frame and a 130 ms blue frame.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/red-blue.webp");
    let mut ui = Ui::new();
    let view = ui.mount(component(move |_| image(path)))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(2.),
            height: px(1.),
        },
    )?;
    let pixel = |ui: &Ui| -> Result<[u8; 4]> {
        let frame = ui.record_frame(view)?;
        assert!(!frame.requests_animation_frame());
        let [
            Draw::Fill {
                fill: Fill::Image(fill),
                ..
            },
        ] = frame.draws()
        else {
            anyhow::bail!("expected a decoded frame");
        };
        Ok(fill.asset().pixels()[0])
    };
    assert_eq!(pixel(&ui)?, [255, 0, 0, 255]);
    let first_boundary = ui.next_redraw(view).expect("second frame scheduled");
    let last_boundary = first_boundary + Duration::from_millis(200);
    std::thread::sleep(last_boundary.saturating_duration_since(std::time::Instant::now()));
    assert_eq!(pixel(&ui)?, [0, 0, 255, 255]);
    assert!(ui.next_redraw(view).is_none());
    Ok(())
}
