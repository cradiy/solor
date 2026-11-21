use super::{ImageAsset, Placement};
use crate::{Fill, ImageSource, raster::DeviceSize, style::ObjectFit};
use anyhow::{Context as _, Result, ensure};
use image::{DynamicImage, ImageDecoder, ImageReader, metadata::Orientation};
use std::{
    fs::File,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const DECODE_BUDGET: u64 = 128 * 1024 * 1024;

/// A filesystem path shared by bitmap elements and backgrounds. Construction performs no I/O.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageFile {
    pub(super) path: Arc<Path>,
    pub(crate) placement: Placement,
}

impl ImageFile {
    /// Refers to a local image. The path must be absolute when the image is used.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::from(path.into().into_boxed_path()),
            placement: Placement::centered(ObjectFit::Cover),
        }
    }

    fn reader(&self) -> Result<ImageReader<BufReader<File>>> {
        ensure!(self.path.is_absolute(), "expected an absolute image path");
        let file = File::open(&self.path)?;
        let meta = file.metadata()?;
        ensure!(meta.is_file(), "expected a regular image file");
        ensure!(meta.len() <= FILE_LIMIT, "encoded image exceeds 64 MiB");
        let mut reader = ImageReader::new(BufReader::new(file)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(DECODE_BUDGET);
        reader.limits(limits);
        Ok(reader)
    }

    pub(crate) fn dimensions(&self) -> Result<DeviceSize> {
        self.probe()
            .with_context(|| format!("reading image dimensions from {}", self.path.display()))
    }

    fn probe(&self) -> Result<DeviceSize> {
        let reader = self.reader()?;
        // The general JPEG adapter buffers the encoded stream. Probe the JPEG headers directly.
        #[cfg(feature = "jpeg")]
        if reader.format() == Some(image::ImageFormat::Jpeg) {
            let mut jpeg = zune_jpeg::JpegDecoder::new(reader.into_inner());
            jpeg.decode_headers().map_err(|e| anyhow::anyhow!("{e}"))?;
            let (w, h) = jpeg.dimensions().context("missing JPEG dimensions")?;
            let orientation = jpeg
                .exif()
                .and_then(|exif| Orientation::from_exif_chunk(exif));
            return Ok(oriented(
                w as u32,
                h as u32,
                orientation.unwrap_or(Orientation::NoTransforms),
            ));
        }
        let mut decoder = reader.into_decoder()?;
        let (w, h) = decoder.dimensions();
        Ok(oriented(w, h, decoder.orientation()?))
    }

    pub(super) fn decode(&self) -> Result<ImageAsset> {
        let decode = || {
            let decoder = self.reader()?.into_decoder()?;
            ImageAsset::decode(decoder, DECODE_BUDGET)
        };
        decode().with_context(|| format!("decoding image from {}", self.path.display()))
    }
}

fn oriented(w: u32, h: u32, orientation: Orientation) -> DeviceSize {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => DeviceSize {
            width: h,
            height: w,
        },
        _ => DeviceSize {
            width: w,
            height: h,
        },
    }
}

impl From<ImageFile> for ImageSource {
    fn from(file: ImageFile) -> Self {
        Self::File(file)
    }
}

crate::assets::source_inputs!(ImageSource, |path| Self::File(ImageFile::new(path)));

impl From<ImageFile> for Fill {
    fn from(file: ImageFile) -> Self {
        Self::ImageFile(file)
    }
}

impl ImageAsset {
    /// Decodes caller-supplied bytes immediately and applies the encoded orientation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let decoder = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()?
            .into_decoder()?;
        Self::decode(
            decoder,
            image::Limits::default().max_alloc.unwrap_or(u64::MAX),
        )
    }

    fn decode(mut decoder: impl ImageDecoder, budget: u64) -> Result<Self> {
        let (w, h) = decoder.dimensions();
        let rgba = (u64::from(w) * u64::from(h)).saturating_mul(4);
        let native = decoder.total_bytes();
        let orientation = decoder.orientation()?;
        let rotate = matches!(
            orientation,
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        );
        let conversion = if decoder.color_type() == image::ColorType::Rgba8 {
            0
        } else {
            rgba
        };
        let working = native.saturating_add(conversion.max(if rotate { native } else { 0 }));
        ensure!(working <= budget, "image pixels exceed the decode budget");
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(budget - working);
        decoder.set_limits(limits)?;
        let mut decoded = DynamicImage::from_decoder(decoder)?;
        decoded.apply_orientation(orientation);
        let rgba = decoded.into_rgba8();
        Self::from_rgba8(rgba.width(), rgba.height(), rgba.into_raw())
    }
}
