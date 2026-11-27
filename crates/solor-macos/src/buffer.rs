//! CPU RGBA8 frame upload into Core Animation layers via Core Graphics images.
use anyhow::{Context as _, Result};
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_quartz_core::{CALayer, CATransaction};
use solor::{
    platform::WindowBackground,
    raster::{DeviceSize, RasterFrame, RasterScale},
};

/// Uploads retained CPU RGBA8 frames into a view's backing `CALayer`.
pub struct CpuLayerPresenter {
    color_space: CFRetained<CGColorSpace>,
}

impl CpuLayerPresenter {
    /// Initializes the presenter with an sRGB color space.
    pub fn new() -> Result<Self> {
        #[allow(unsafe_code)]
        // SAFETY: `kCGColorSpaceSRGB` is a static Core Graphics constant.
        let srgb = unsafe { kCGColorSpaceSRGB };
        let color_space =
            CGColorSpace::with_name(Some(srgb)).context("failed to create sRGB CGColorSpace")?;
        Ok(Self { color_space })
    }

    /// Presents `frame` onto `layer` when damaged or forced.
    pub fn present(
        &self,
        layer: &CALayer,
        frame: &RasterFrame<'_>,
        expected: DeviceSize,
        scale: RasterScale,
        background: WindowBackground,
        force_upload: bool,
    ) -> Result<()> {
        frame.validate(expected)?;
        if !force_upload && frame.damage.is_empty() {
            return Ok(());
        }
        let image =
            create_cg_image_with_space(frame.rgba, expected, background, &self.color_space)?;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        layer.setContentsScale(scale.0);
        layer.setOpaque(background == WindowBackground::Opaque);
        #[allow(unsafe_code)]
        // SAFETY: `CGImage` is toll-free bridged / valid as `CALayer.contents` per Core Animation docs.
        unsafe {
            let contents = (&*image as *const CGImage).cast::<AnyObject>();
            layer.setContents(Some(&*contents));
        }
        CATransaction::commit();
        Ok(())
    }
}

/// Builds an immutable Core Graphics image wrapping tightly packed RGBA8 pixel bytes.
#[cfg(test)]
pub(crate) fn create_cg_image(
    rgba: &[u8],
    size: DeviceSize,
    background: WindowBackground,
) -> Result<CFRetained<CGImage>> {
    #[allow(unsafe_code)]
    // SAFETY: `kCGColorSpaceSRGB` is a static Core Graphics constant.
    let srgb = unsafe { kCGColorSpaceSRGB };
    let color_space =
        CGColorSpace::with_name(Some(srgb)).context("failed to create sRGB CGColorSpace")?;
    create_cg_image_with_space(rgba, size, background, &color_space)
}

fn create_cg_image_with_space(
    rgba: &[u8],
    size: DeviceSize,
    background: WindowBackground,
    color_space: &CGColorSpace,
) -> Result<CFRetained<CGImage>> {
    let bytes_per_row = usize::try_from(size.width)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .context("image width overflows byte stride")?;
    let expected_len = bytes_per_row
        .checked_mul(usize::try_from(size.height).context("image height out of range")?)
        .context("image byte count overflows usize")?;
    anyhow::ensure!(
        size.width > 0 && size.height > 0 && rgba.len() == expected_len,
        "invalid RGBA8 buffer size: got {} bytes, expected {expected_len}",
        rgba.len()
    );

    let data = CFData::from_bytes(rgba);
    let provider =
        CGDataProvider::with_cf_data(Some(&data)).context("failed to create CGDataProvider")?;

    let alpha_info = match background {
        WindowBackground::Opaque => CGImageAlphaInfo::NoneSkipLast,
        WindowBackground::Transparent => CGImageAlphaInfo::PremultipliedLast,
    };
    let bitmap_info = CGBitmapInfo(CGImageByteOrderInfo::OrderDefault.0 | alpha_info.0);

    #[allow(unsafe_code)]
    // SAFETY: `decode` is null; `provider` owns a retained `CFData` copy of `rgba` with `expected_len` bytes.
    let image = unsafe {
        CGImage::new(
            size.width as usize,
            size.height as usize,
            8,
            32,
            bytes_per_row,
            Some(color_space),
            bitmap_info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .context("failed to create CGImage")?;
    Ok(image)
}

#[cfg(test)]
#[path = "../tests/unit/buffer.rs"]
mod tests;
