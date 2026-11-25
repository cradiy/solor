use anyhow::Result;
use rayon::{ThreadPool, ThreadPoolBuilder, prelude::*};
use solor::{ImageAsset, ImageRegion, ShaderFill, layout::Size, raster::DeviceSize};
use solor_shader::Op;
use std::sync::OnceLock;

const LANES: usize = 32;
type Batch = [f32; LANES];

pub(super) fn render(fill: &ShaderFill, size: Size, region: ImageRegion) -> Result<ImageAsset> {
    let program = solor::raster::shader_program(&fill.shader);
    let params = fill.params;
    let size = [size.width.get(), size.height.get()];
    let [width, height] = program.sample_extent(region.output.width, region.output.height);
    let (column_ops, row_ops) = program.column_plan();
    let execute = |registers: &mut [Batch], ops: &[usize], u: Batch, v: f32| {
        for &dst in ops {
            let r = &*registers;
            registers[dst] = match program.instructions()[dst] {
                Op::U => u,
                Op::V => [v; LANES],
                Op::Size(axis) => [size[axis]; LANES],
                Op::Param(index) => [params[index]; LANES],
                Op::Constant(value) => [value; LANES],
                Op::Abs(a) => r[a].map(f32::abs),
                Op::Sqrt(a) => r[a].map(f32::sqrt),
                Op::Pow(a, b) if !program.varies_across_row(dst) => [r[a][0].powf(r[b][0]); LANES],
                Op::Pow(a, b) => std::array::from_fn(|i| r[a][i].powf(r[b][i])),
                Op::ZeroIfNegative(test, value) => {
                    std::array::from_fn(|i| if r[test][i] < 0.0 { 0.0 } else { r[value][i] })
                }
                Op::Sin(a) if !program.varies_across_row(a) => [r[a][0].sin(); LANES],
                Op::Cos(a) if !program.varies_across_row(a) => [r[a][0].cos(); LANES],
                Op::Sin(a) => r[a].map(f32::sin),
                Op::Cos(a) => r[a].map(f32::cos),
                Op::Neg(a) => r[a].map(|x| -x),
                Op::Add(a, b) => std::array::from_fn(|i| r[a][i] + r[b][i]),
                Op::Sub(a, b) => std::array::from_fn(|i| r[a][i] - r[b][i]),
                Op::Mul(a, b) => std::array::from_fn(|i| r[a][i] * r[b][i]),
                Op::Div(a, b) => std::array::from_fn(|i| r[a][i] / r[b][i]),
                Op::Min(a, b) => std::array::from_fn(|i| r[a][i].min(r[b][i])),
                Op::Max(a, b) => std::array::from_fn(|i| r[a][i].max(r[b][i])),
                Op::Clamp(a, lo, hi) => {
                    std::array::from_fn(|i| r[a][i].max(r[lo][i]).min(r[hi][i]))
                }
                Op::Mix(a, b, c) => {
                    std::array::from_fn(|i| r[a][i] * (1. - r[c][i]) + r[b][i] * r[c][i])
                }
                Op::SmoothStep(a, b, c) => std::array::from_fn(|i| {
                    let t = ((r[c][i] - r[a][i]) / (r[b][i] - r[a][i])).clamp(0., 1.);
                    t * t * (3. - 2. * t)
                }),
            };
        }
    };
    rasterize(
        region,
        DeviceSize { width, height },
        || vec![[0.; LANES]; program.instructions().len()],
        |registers, u| execute(registers, column_ops, u, 0.),
        |registers, v, out| {
            execute(registers, row_ops, [0.; LANES], v);
            let [red, green, blue, alpha] = program.outputs().map(|r| &registers[r]);
            for (i, pixel) in out.iter_mut().enumerate() {
                *pixel = [
                    unorm(red[i]),
                    unorm(green[i]),
                    unorm(blue[i]),
                    unorm(alpha[i]),
                ];
            }
        },
    )
}

fn unorm(value: f32) -> u8 {
    // Integer half-up rounding also preserves values immediately below half a byte.
    (((value.clamp(0., 1.) * 510.) as u16 + 1) >> 1) as u8
}

fn rasterize<S>(
    region: ImageRegion,
    samples: DeviceSize,
    scratch: impl Fn() -> S + Sync,
    column: impl Fn(&mut S, Batch) + Sync,
    shade: impl Fn(&mut S, f32, &mut [[u8; 4]]) + Sync,
) -> Result<ImageAsset> {
    let DeviceSize { width, height } = samples;
    let mut bytes = vec![0; width as usize * height as usize * 4];
    let columns: Vec<Batch> = (0..(width as usize).div_ceil(LANES))
        .map(|block| {
            std::array::from_fn(|i| {
                (region.source_origin[0]
                    + ((block * LANES + i) as f64 + 0.5) * region.source_step[0])
                    as f32
            })
        })
        .collect();
    let stride = width as usize * 4;
    let render_rows = |first: usize, rows: &mut [u8]| {
        let mut scratch = scratch();
        // Keep the output tile in cache while reusing each horizontal batch down its rows.
        const TILE_ROWS: usize = 16;
        for (tile, rows) in rows
            .chunks_mut(stride.saturating_mul(TILE_ROWS))
            .enumerate()
        {
            let first_y = first + tile * TILE_ROWS;
            for (block, &u) in columns.iter().enumerate() {
                column(&mut scratch, u);
                let x = block * LANES;
                for (index, row) in rows.chunks_exact_mut(stride).enumerate() {
                    let v = (region.source_origin[1]
                        + ((first_y + index) as f64 + 0.5) * region.source_step[1])
                        as f32;
                    let pixels =
                        &mut row.as_chunks_mut::<4>().0[x..(x + LANES).min(width as usize)];
                    shade(&mut scratch, v, pixels);
                }
            }
        }
    };
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    let pool = if bytes.len() >= 512 * 1024 && height > 1 {
        POOL.get_or_init(|| {
            let threads = std::thread::available_parallelism().ok()?.get().min(4);
            if threads < 2 {
                return None;
            }
            ThreadPoolBuilder::new().num_threads(threads).build().ok()
        })
        .as_ref()
    } else {
        None
    };
    if let Some(pool) = pool {
        let rows = (height as usize).div_ceil(pool.current_num_threads());
        pool.install(|| {
            bytes
                .par_chunks_mut(rows * stride)
                .enumerate()
                .for_each(|(i, band)| render_rows(i * rows, band))
        });
    } else {
        render_rows(0, &mut bytes);
    }

    ImageAsset::from_rgba8(width, height, bytes)
}

#[cfg(test)]
#[path = "../tests/unit/shader.rs"]
mod tests;
