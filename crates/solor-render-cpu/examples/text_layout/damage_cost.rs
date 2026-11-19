use crate::damage::RetainedPixels;
use crate::frame::{Mode, Presentation};
use crate::glyph_masks::GlyphMasks;
use crate::pixels::DeviceSize;
use crate::pixels::{Clip, Surface};
use std::{
    hint::black_box,
    io::Result,
    time::{Duration, Instant},
};

const LABEL: &str = "Hello 世界！Solor 中文排版";
const SIZE: DeviceSize = DeviceSize {
    width: 256,
    height: 244,
};

#[derive(Clone, Copy, Debug)]
enum Edit {
    Text,
    Motion,
    Spread,
    Resize,
    Idle,
}

impl Edit {
    fn apply(self, view: &mut Presentation, step: usize) -> DeviceSize {
        let even = step.is_multiple_of(2);
        match self {
            Self::Text => view
                .set_content(
                    3,
                    if even {
                        "Hello 世界！文字换行"
                    } else {
                        LABEL
                    },
                )
                .unwrap(),
            Self::Motion => view.set_shift(3, if even { 12 } else { 0 }).unwrap(),
            Self::Spread => {
                let color = if even {
                    [180, 20, 40, 200]
                } else {
                    [0, 0, 0, 255]
                };
                for i in [0, 7] {
                    view.set_color(i, color).unwrap();
                }
            }
            Self::Resize if even => return DeviceSize { width: 192, ..SIZE },
            Self::Resize | Self::Idle => {}
        }
        SIZE
    }
}

pub fn run() -> Result<()> {
    for edit in [
        Edit::Text,
        Edit::Motion,
        Edit::Spread,
        Edit::Resize,
        Edit::Idle,
    ] {
        let mut views = std::array::from_fn::<_, 2, _>(|_| Presentation::new(8, LABEL, true));
        let mut masks = std::array::from_fn::<_, 2, _>(|_| GlyphMasks::new(64, 32 * 1024));
        let initial = views[1]
            .update(SIZE.at_unit_scale(), Mode::Normal)
            .unwrap()
            .frame;
        let mut retained = RetainedPixels::new(initial, SIZE, Clip::canvas(SIZE), &mut masks[1])?;
        let mut samples = [[Duration::ZERO; 2]; 7];
        let mut pixels = 0usize;
        for batch in 0..8 {
            for offset in 0..2 {
                let mode = (batch + offset) % 2;
                let start = Instant::now();
                for step in 0..10 {
                    let size = edit.apply(&mut views[mode], step);
                    let frame = views[mode]
                        .update(size.at_unit_scale(), Mode::Normal)
                        .unwrap()
                        .frame;
                    let clip = Clip::canvas(size);
                    if mode == 0 {
                        black_box(Surface::draw_cached(&frame, size, clip, &mut masks[0])?);
                    } else {
                        let area = retained.update(frame, size, clip, &mut masks[1])?;
                        if batch > 0
                            && let Some(rect) = area
                        {
                            pixels +=
                                ((rect.right - rect.left) * (rect.bottom - rect.top)) as usize;
                        }
                        black_box(retained.surface());
                    }
                }
                if batch > 0 {
                    samples[batch - 1][mode] = start.elapsed();
                }
            }
        }
        let reference = Surface::draw(&views[0].current().unwrap(), SIZE, Clip::canvas(SIZE))?;
        assert_eq!(retained.surface().rgba, reference.rgba);
        let median = |mode: usize| {
            let mut times = samples.map(|t| t[mode]);
            times.sort_unstable();
            times[3].as_secs_f64() * 1e5
        };
        eprintln!(
            "damage/{edit:?} edit-to-pixels us full={:.2} retained={:.2} pixels={}",
            median(0),
            median(1),
            pixels / 70
        );
    }
    Ok(())
}
