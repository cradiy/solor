//! Fixed-font CPU text raster experiment. PNG goes to stdout; timings go to stderr.
mod damage_cost;
#[allow(dead_code)]
mod frame;
use solor::layout;
use solor_render_cpu::{damage, glyph_masks, pixels};
#[allow(dead_code)]
mod font_fixture;
mod mask_cost;
#[cfg(test)]
mod pixels_tests;

use frame::{Mode, Presentation};
use pixels::DeviceSize;
use pixels::{Clip, Surface};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    mask_cost::run()?;
    damage_cost::run()?;
    let size = DeviceSize {
        width: 256,
        height: 244,
    };
    for rows in [1, 8] {
        let mut view = Presentation::new(rows, "Hello 世界！Solor 中文排版", true);
        let mut samples = [[Duration::ZERO; 2]; 7];
        for sample in &mut samples {
            for step in 0..12 {
                let start = Instant::now();
                view.set_content(
                    0,
                    if step % 2 == 0 {
                        "Hello 世界！文字换行"
                    } else {
                        "Hello 世界！Solor 中文排版"
                    },
                )
                .unwrap();
                let frame = view
                    .update(size.at_unit_scale(), Mode::Normal)
                    .unwrap()
                    .frame;
                let prepared = Instant::now();
                black_box(Surface::draw(&frame, size, Clip::canvas(size))?);
                if step >= 2 {
                    sample[0] += prepared - start;
                    sample[1] += prepared.elapsed();
                }
            }
        }
        let us = |mut values: [Duration; 7]| {
            values.sort_unstable();
            values[3].as_secs_f64() * 1e5
        };
        eprintln!(
            "{rows} rows: edit+snapshot {:.2} us, full raster {:.2} us",
            us(samples.map(|s| s[0])),
            us(samples.map(|s| s[1]))
        );
    }
    let logical = layout::Size {
        width: solor::px(240.0),
        height: solor::px(58.0),
    };
    let size = pixels::RasterScale::ONE.buffer_size(logical)?;
    let mut view = Presentation::new(2, "Solor 中文排版", true);
    view.set_content(1, "Hello 世界！Rust GUI 123").unwrap();
    view.set_color(1, [27, 85, 148, 255]).unwrap();
    let frame = view.update(logical, Mode::Normal).unwrap().frame;
    drop(view);
    let mut cache = glyph_masks::GlyphMasks::new(64, 32 * 1024);
    let image = Surface::draw_cached(&frame, size, Clip::canvas(size), &mut cache)?;
    let mut encoder = png::Encoder::new(std::io::stdout().lock(), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;
    writer.finish()?;
    Ok(())
}
