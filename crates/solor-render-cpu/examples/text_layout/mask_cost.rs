use std::hint::black_box;
use std::io::Result;
use std::time::{Duration, Instant};

use crate::frame::{Mode, Presentation};
use crate::glyph_masks::GlyphMasks;
use crate::pixels::DeviceSize;
use crate::pixels::{Clip, Surface};

const LABEL: &str = "Hello 世界！Solor 中文排版";
const ALTERNATE: &str = "Hello 世界！文字换行";

pub fn run() -> Result<()> {
    let size = DeviceSize {
        width: 256,
        height: 244,
    };
    for (case, rows, entries, cold) in [
        ("one", 1, 64, false),
        ("eight", 8, 64, false),
        ("cold", 1, 64, true),
        ("pressure", 8, 4, false),
    ] {
        let mut views = [
            Presentation::new(rows, LABEL, true),
            Presentation::new(rows, LABEL, true),
        ];
        let mut cache = GlyphMasks::new(entries, 32 * 1024);
        let mut times = [[Duration::ZERO; 2]; 7];
        for batch in 0..8 {
            for offset in 0..2 {
                let mode = (batch + offset) % 2;
                let start = Instant::now();
                for step in 0..10 {
                    views[mode]
                        .set_content(0, if step % 2 == 0 { ALTERNATE } else { LABEL })
                        .unwrap();
                    let frame = views[mode]
                        .update(size.at_unit_scale(), Mode::Normal)
                        .unwrap()
                        .frame;
                    if mode == 0 {
                        black_box(Surface::draw(&frame, size, Clip::canvas(size))?);
                    } else {
                        if cold {
                            cache.release();
                        }
                        black_box(Surface::draw_cached(
                            &frame,
                            size,
                            Clip::canvas(size),
                            &mut cache,
                        )?);
                    }
                }
                if batch != 0 {
                    times[batch - 1][mode] = start.elapsed();
                }
            }
            if batch == 0 {
                cache.hits = 0;
                cache.misses = 0;
            }
        }
        let median = |mode: usize| {
            let mut samples = times.map(|t| t[mode]);
            samples.sort_unstable();
            samples[3].as_secs_f64() * 100_000.
        };
        eprintln!(
            "mask/{case} edit-to-pixels us plain={:.2} cached={:.2} hits={} misses={} retained={:?}",
            median(0),
            median(1),
            cache.hits,
            cache.misses,
            cache.usage()
        );
        let a = Surface::draw(&views[0].current().unwrap(), size, Clip::canvas(size))?;
        let b = Surface::draw_cached(
            &views[1].current().unwrap(),
            size,
            Clip::canvas(size),
            &mut cache,
        )?;
        assert_eq!(a.rgba, b.rgba);
    }
    Ok(())
}
