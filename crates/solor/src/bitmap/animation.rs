use super::{ImageAsset, ImageFill};
use anyhow::{Context as _, Result, ensure};
use image::{AnimationDecoder, DynamicImage, ImageDecoder};
use std::{mem::size_of, sync::Arc, time::Duration};

pub(super) struct Animation {
    first_end: Duration,
    following: Vec<(Duration, ImageAsset)>,
    plays: Option<u32>,
}

impl ImageAsset {
    pub(crate) fn is_animated(&self) -> bool {
        self.0.animation.is_some()
    }

    pub(super) fn decode_animation<'a>(
        mut decoder: impl AnimationDecoder<'a> + ImageDecoder + 'a,
        plays: Option<u32>,
        budget: u64,
    ) -> Result<Self> {
        let (w, h) = decoder.dimensions();
        let canvas = (u64::from(w) * u64::from(h)).saturating_mul(4);
        // Leave space for the codec's compositing canvas, a pending frame and orientation.
        let scratch = canvas.saturating_mul(3);
        // Include the Arc counters and spare Vec capacity for the timeline entries.
        let overhead = size_of::<super::Bitmap>()
            + 2 * size_of::<usize>()
            + 2 * size_of::<(Duration, ImageAsset)>();
        let retained = canvas.saturating_add(overhead as u64);
        let capacity = budget.saturating_sub(scratch) / retained;
        ensure!(capacity > 0, "animation exceeds the decode budget");
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(scratch);
        decoder.set_limits(limits)?;
        let orientation = decoder.orientation()?;
        let mut frames = decoder.into_frames();
        let convert = |frame: image::Frame| -> Result<_> {
            let delay = Duration::from(frame.delay()).max(Duration::from_millis(10));
            let mut pixels = DynamicImage::ImageRgba8(frame.into_buffer());
            pixels.apply_orientation(orientation);
            let rgba = pixels.into_rgba8();
            Ok((
                delay,
                Self::from_rgba8(rgba.width(), rgba.height(), rgba.into_raw())?,
            ))
        };
        let (first_end, mut first) = convert(frames.next().context("animation has no frames")??)?;
        let mut end = first_end;
        let mut following = Vec::new();
        for frame in frames {
            ensure!(
                (following.len() as u64) + 1 < capacity,
                "animation exceeds the decode budget"
            );
            let (delay, asset) = convert(frame?)?;
            end += delay;
            following.push((end, asset));
        }
        if !following.is_empty() {
            let timeline = Animation {
                first_end,
                following,
                plays,
            };
            Arc::get_mut(&mut first.0)
                .expect("newly decoded poster")
                .animation = Some(Box::new(timeline));
        }
        Ok(first)
    }
}

impl ImageFill {
    // Returns elapsed time at the next frame boundary, relative to the start of playback.
    pub(crate) fn sample_animation(&mut self, elapsed: Duration) -> Option<Duration> {
        let animation = self.asset.0.animation.as_ref()?;
        let (cycle, last) = animation.following.last()?;
        let cycle_ns = cycle.as_nanos();
        let ns = elapsed.as_nanos();
        let pass = ns / cycle_ns;
        if animation
            .plays
            .is_some_and(|plays| pass >= u128::from(plays))
        {
            self.asset = last.clone();
            return None;
        }
        let position = ns % cycle_ns;
        let index = animation
            .following
            .partition_point(|(end, _)| end.as_nanos() <= position);
        let mut end = animation.first_end;
        let mut finished = false;
        if position >= end.as_nanos() {
            let (frame_end, frame) = &animation.following[index];
            end = *frame_end;
            finished = end == *cycle
                && animation
                    .plays
                    .is_some_and(|plays| pass + 1 == u128::from(plays));
            self.asset = frame.clone();
        }
        (!finished).then(|| elapsed + Duration::from_nanos((end.as_nanos() - position) as u64))
    }
}
