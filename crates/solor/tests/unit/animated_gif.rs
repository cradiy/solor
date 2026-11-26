use crate::{ImageAsset, ImageFill};
use std::{borrow::Cow, time::Duration};

#[test]
fn gif_composites_partial_frames_and_observes_repetitions() -> anyhow::Result<()> {
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    for repeat in [
        None,
        Some(gif::Repeat::Finite(1)),
        Some(gif::Repeat::Infinite),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 2, 1, &[255, 0, 0, 0, 0, 255])?;
            if let Some(repeat) = repeat {
                encoder.set_repeat(repeat)?;
            }
            encoder.write_frame(&gif::Frame {
                width: 2,
                height: 1,
                delay: 3,
                buffer: Cow::Borrowed(&[0, 0]),
                ..Default::default()
            })?;
            encoder.write_frame(&gif::Frame {
                left: 1,
                width: 1,
                height: 1,
                delay: 7,
                buffer: Cow::Borrowed(&[1]),
                ..Default::default()
            })?;
        }
        let asset = ImageAsset::from_bytes(&bytes)?;
        let sample = |ms| {
            let mut fill = ImageFill::from(&asset);
            let next = fill.sample_animation(Duration::from_millis(ms));
            (
                fill.asset().pixels().to_vec(),
                next.map(|time| time.as_millis()),
            )
        };
        assert_eq!(sample(29), (vec![red, red], Some(30)));
        assert_eq!(sample(30), (vec![red, blue], repeat.map(|_| 100)));
        if repeat.is_some() {
            assert_eq!(sample(100), (vec![red, red], Some(130)));
        }
        let forever = repeat == Some(gif::Repeat::Infinite);
        assert_eq!(sample(550), (vec![red, blue], forever.then_some(600)));
    }
    Ok(())
}
