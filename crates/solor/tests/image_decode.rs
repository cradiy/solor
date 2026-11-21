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
