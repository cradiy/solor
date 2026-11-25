use super::{Basin, Drop, Pebble, Pool};
use anyhow::Result;
use solor::{ImageAsset, raster::DeviceSize};

const TICKS: f64 = 120.0;
const CELL: usize = 4;

#[derive(Clone, Copy, Default)]
struct Ray {
    dx: f32,
    dy: f32,
    light: f32,
}

impl Ray {
    fn towards(self, other: Self, t: f32) -> Self {
        Self {
            dx: self.dx + (other.dx - self.dx) * t,
            dy: self.dy + (other.dy - self.dy) * t,
            light: self.light + (other.light - self.light) * t,
        }
    }
}

pub(super) fn resolution(basin: Basin) -> DeviceSize {
    let scale = (800. / basin.width.max(basin.height)).min(1.);
    DeviceSize {
        width: (basin.width * scale).round().max(1.) as u32,
        height: (basin.height * scale).round().max(1.) as u32,
    }
}

pub(super) struct Water {
    bed: ImageAsset,
    width: usize,
    height: usize,
    levels: Vec<f32>,
    previous: Vec<f32>,
    rays: Vec<Ray>,
    energy: Vec<f32>,
    scan: Vec<Ray>,
    tick: Option<i64>,
}

impl Water {
    pub(super) fn new(bed: ImageAsset) -> Self {
        let dims = bed.dimensions();
        let width = (dims.width as usize).div_ceil(CELL) + 1;
        let height = (dims.height as usize).div_ceil(CELL) + 1;
        let count = width * height;
        Self {
            bed,
            width,
            height,
            levels: vec![0.; count],
            previous: vec![0.; count],
            rays: vec![Ray::default(); count],
            energy: vec![0.; count],
            scan: vec![Ray::default(); width],
            tick: None,
        }
    }

    pub(super) fn grid_size(&self) -> DeviceSize {
        DeviceSize {
            width: self.width as u32,
            height: self.height as u32,
        }
    }

    // Update at fixed physical time steps so propagation is independent of presentation rate.
    fn advance(&mut self, basin: Basin, drops: &[Option<Drop>], now: f64) {
        let tick = (now * TICKS).floor() as i64;
        let Some(recent) = drops
            .iter()
            .flatten()
            .map(|d| (d.contact * TICKS).ceil() as i64)
            .filter(|&t| t <= tick && t >= tick - 600)
            .min()
        else {
            if self.tick.take().is_some() {
                self.levels.fill(0.);
                self.previous.fill(0.);
            }
            return;
        };
        let mut last = self.tick.unwrap_or(recent - 1);
        if tick - last > 600 || tick < last {
            self.levels.fill(0.);
            self.previous.fill(0.);
            last = recent - 1;
        }
        let dx = basin.width / (self.width - 1) as f32;
        let dy = basin.height / (self.height - 1) as f32;
        let kx = (158. / (TICKS as f32 * dx)).min(0.68).powi(2);
        let ky = (158. / (TICKS as f32 * dy)).min(0.68).powi(2);
        for step in last + 1..=tick {
            for drop in drops.iter().flatten() {
                let age = step - (drop.contact * TICKS).ceil() as i64;
                match age {
                    0 => self.displace(*drop, dx, dy, 1., 1.),
                    16 => self.displace(*drop, dx, dy, 0.55, -0.55),
                    _ => {}
                }
            }
            for y in 0..self.height {
                let top = y.saturating_sub(1) * self.width;
                let bottom = (y + 1).min(self.height - 1) * self.width;
                let row = y * self.width;
                for x in 0..self.width {
                    let i = row + x;
                    let h = self.levels[i];
                    let xx = self.levels[row + x.saturating_sub(1)]
                        + self.levels[row + (x + 1).min(self.width - 1)]
                        - 2. * h;
                    let yy = self.levels[top + x] + self.levels[bottom + x] - 2. * h;
                    self.previous[i] = h + 0.992 * (h - self.previous[i]) + kx * xx + ky * yy;
                }
            }
            std::mem::swap(&mut self.previous, &mut self.levels);
        }
        self.tick = Some(tick);
    }

    fn displace(&mut self, drop: Drop, dx: f32, dy: f32, width: f32, strength: f32) {
        let cx = drop.uv[0] * (self.width - 1) as f32;
        let cy = drop.uv[1] * (self.height - 1) as f32;
        let radius = (drop.diameter * 0.235 * width).max(0.01);
        let rx = (2.7 * radius / dx).ceil() as usize;
        let ry = (2.7 * radius / dy).ceil() as usize;
        for y in (cy as usize).saturating_sub(ry)
            ..=((cy as usize).saturating_add(ry)).min(self.height - 1)
        {
            for x in (cx as usize).saturating_sub(rx)
                ..=((cx as usize).saturating_add(rx)).min(self.width - 1)
            {
                let u = (x as f32 - cx) * dx / radius;
                let v = (y as f32 - cy) * dy / radius;
                let r2 = (u + 0.16 * v).powi(2) + (v * 1.12).powi(2);
                // The hollow under the stone displaces its volume into a raised rim.
                let delta = strength * radius * 0.86 * (r2 - 1.) * (-r2).exp();
                let i = y * self.width + x;
                self.levels[i] += delta;
                self.previous[i] += 0.91 * delta;
            }
        }
    }

    fn optics(&mut self, basin: Basin, wind: &ImageAsset) {
        let dx = basin.width / (self.width - 1) as f32;
        let dy = basin.height / (self.height - 1) as f32;
        let scale = self.bed.dimensions().width as f32 / basin.width;
        let shelf = (basin.width * 0.22).min(98.);

        for y in 0..self.height {
            let row = y * self.width;
            let top = y.saturating_sub(1) * self.width;
            let bot = (y + 1).min(self.height - 1) * self.width;
            for x in 0..self.width {
                let i = row + x;
                let wind = wind.pixels()[i];
                let sx = (f32::from(wind[0]) - 127.5) / (255. * 1.9)
                    + (self.levels[row + (x + 1).min(self.width - 1)]
                        - self.levels[row + x.saturating_sub(1)])
                        / (2. * dx);
                let sy = (f32::from(wind[1]) - 127.5) / (255. * 1.9)
                    + (self.levels[bot + x] - self.levels[top + x]) / (2. * dy);
                let depth = 13. + 39. * ((x as f32 * dx - shelf) / 16.).clamp(0., 1.);
                let highlight = ((-sx * 0.76 - sy * 0.65 - 0.09) * 3.8).clamp(0., 1.);
                self.rays[i] = Ray {
                    dx: sx.clamp(-1., 1.) * depth * scale,
                    dy: sy.clamp(-1., 1.) * depth * scale,
                    light: (sx * 0.42 - sy * 0.8) * 12.
                        + highlight * highlight * 57.
                        + (f32::from(wind[2]) / 255. - 0.5) * 9.,
                };
            }
        }
        self.focus_sunlight(dx * scale, dy * scale);
    }

    fn focus_sunlight(&mut self, dx: f32, dy: f32) {
        self.energy.fill(0.);
        // Four rays per surface cell resolve narrow folds without imprinting the mesh on the light.
        for y in 0..self.height - 1 {
            for x in 0..self.width - 1 {
                let i = y * self.width + x;
                let a = self.rays[i];
                let b = self.rays[i + 1];
                let c = self.rays[i + self.width];
                let d = self.rays[i + self.width + 1];
                for fy in [0.25, 0.75] {
                    let left = a.towards(c, fy);
                    let right = b.towards(d, fy);
                    for fx in [0.25, 0.75] {
                        let ray = left.towards(right, fx);
                        let bx = x as f32 + fx - ray.dx * (1.83 / dx);
                        let by = y as f32 + fy - ray.dy * (1.83 / dy);
                        let ix = bx.floor() as isize;
                        let iy = by.floor() as isize;
                        if ix < 0
                            || iy < 0
                            || ix as usize >= self.width - 1
                            || iy as usize >= self.height - 1
                        {
                            continue;
                        }
                        let u = bx - ix as f32;
                        let v = by - iy as f32;
                        let k = iy as usize * self.width + ix as usize;
                        self.energy[k] += 0.25 * (1. - u) * (1. - v);
                        self.energy[k + 1] += 0.25 * u * (1. - v);
                        self.energy[k + self.width] += 0.25 * (1. - u) * v;
                        self.energy[k + self.width + 1] += 0.25 * u * v;
                    }
                }
            }
        }
        for (ray, energy) in self.rays.iter_mut().zip(&self.energy) {
            ray.light += (energy.min(5.5) - 1.) * 19.;
        }
    }

    pub(super) fn frame(
        &mut self,
        basin: Basin,
        pool: &Pool,
        now: f64,
        wind: &ImageAsset,
        stones: &[ImageAsset; 2],
    ) -> Result<ImageAsset> {
        self.advance(basin, &pool.waves, now);
        self.optics(basin, wind);
        let dims = self.bed.dimensions();
        let w = dims.width as usize;
        let h = dims.height as usize;
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let gy = y / CELL;
            let fy = (y % CELL) as f32 / CELL as f32;
            for (x, r) in self.scan.iter_mut().enumerate() {
                *r = self.rays[gy * self.width + x]
                    .towards(self.rays[(gy + 1) * self.width + x], fy);
            }
            refract_row(self.bed.pixels(), dims, y, &self.scan, &mut pixels);
        }
        for (index, sprite) in stones.iter().enumerate() {
            let stone = pool.pebble(index, basin, now);
            if stone.wet > 0. {
                self.submerged_stone(pixels.as_chunks_mut::<4>().0, basin, stone, sprite);
            }
        }
        ImageAsset::from_rgba8(dims.width, dims.height, pixels)
    }

    fn submerged_stone(
        &self,
        pixels: &mut [[u8; 4]],
        basin: Basin,
        stone: Pebble,
        sprite: &ImageAsset,
    ) {
        let dims = self.bed.dimensions();
        let scale = dims.width as f32 / basin.width;
        let d = stone.diameter * scale;
        let left = stone.center[0] * scale - d * 0.5;
        let top = stone.center[1] * (dims.height as f32 / basin.height) - d * 0.5;
        let side = sprite.dimensions().width as usize;
        let waterline = (1. - stone.wet) * d;
        let reach = 52. * scale * stone.depth;
        for y in (top + waterline - reach).max(0.) as usize
            ..((top + d + reach).ceil() as usize).min(dims.height as usize)
        {
            let fy = (y % CELL) as f32 / CELL as f32;
            for x in (left - reach).max(0.) as usize
                ..((left + d + reach).ceil() as usize).min(dims.width as usize)
            {
                let i = (y / CELL) * self.width + x / CELL;
                let fx = (x % CELL) as f32 / CELL as f32;
                let ray = self.rays[i].towards(self.rays[i + 1], fx).towards(
                    self.rays[i + self.width].towards(self.rays[i + self.width + 1], fx),
                    fy,
                );
                let u = x as f32 + 0.5 - left + ray.dx * stone.depth;
                let v = y as f32 + 0.5 - top + ray.dy * stone.depth;
                if !(0.0..d).contains(&u) || !(waterline..d).contains(&v) {
                    continue;
                }
                let src = sample_rgba(
                    sprite.pixels(),
                    side,
                    side,
                    ((u / d * side as f32 - 0.5) * 256.) as i32,
                    ((v / d * side as f32 - 0.5) * 256.) as i32,
                );
                if src[3] == 0 {
                    continue;
                }
                let alpha = f32::from(src[3]) / 255.;
                let dst = &mut pixels[y * dims.width as usize + x];
                for k in 0..3 {
                    let transmission = 1. - stone.depth * [0.38, 0.20, 0.16][k];
                    let scatter = stone.depth * [8., 26., 27.][k];
                    let light = ray.light * (0.35 + stone.depth * 0.65);
                    let color = f32::from(src[k]) * transmission * (1. + light / 256.)
                        + alpha * (scatter + light * [0.85, 0.61, 0.35][k]);
                    dst[k] = (color + f32::from(dst[k]) * (1. - alpha)) as u8;
                }
            }
        }
    }
}

fn refract_row(bed: &[[u8; 4]], dims: DeviceSize, y: usize, scan: &[Ray], pixels: &mut Vec<u8>) {
    let w = dims.width as usize;
    let h = dims.height as usize;
    for (block, pair) in scan.windows(2).enumerate() {
        let left = pair[0];
        let right = pair[1];
        let x = block * CELL;
        let mut bx = (x as i32 * 256) + (left.dx * 256.) as i32;
        let mut by = (y as i32 * 256) + (left.dy * 256.) as i32;
        let mut light = (left.light * 256.) as i32;
        let step_x = 256 + ((right.dx - left.dx) * (256. / CELL as f32)) as i32;
        let step_y = ((right.dy - left.dy) * (256. / CELL as f32)) as i32;
        let step_light = ((right.light - left.light) * (256. / CELL as f32)) as i32;
        for _ in 0..CELL.min(w - x) {
            let [r, g, b, _] = sample_rgba(bed, w, h, bx, by).map(i32::from);
            let sun = light >> 8;
            pixels.extend_from_slice(&[
                (r + ((sun * (r + 218)) >> 8)).clamp(0, 255) as u8,
                (g + ((sun * (g + 155)) >> 8)).clamp(0, 255) as u8,
                (b + ((sun * (b + 90)) >> 8)).clamp(0, 255) as u8,
                255,
            ]);
            bx += step_x;
            by += step_y;
            light += step_light;
        }
    }
}

#[inline]
fn sample_rgba(bed: &[[u8; 4]], w: usize, h: usize, bx: i32, by: i32) -> [u8; 4] {
    let u = bx.clamp(0, (w as i32 - 1) * 256) as usize;
    let v = by.clamp(0, (h as i32 - 1) * 256) as usize;
    let ix = u >> 8;
    let iy = v >> 8;
    let fx = (u & 255) as u32;
    let fy = (v & 255) as u32;
    let right = (ix + 1).min(w - 1);
    let top = iy * w;
    let bottom = (iy + 1).min(h - 1) * w;
    let a = u32::from_le_bytes(bed[top + ix]);
    let b = u32::from_le_bytes(bed[top + right]);
    let c = u32::from_le_bytes(bed[bottom + ix]);
    let d = u32::from_le_bytes(bed[bottom + right]);
    let rb = blend_pair(blend_pair(a, b, fx), blend_pair(c, d, fx), fy);
    let green = blend_pair(
        blend_pair(a >> 8, b >> 8, fx),
        blend_pair(c >> 8, d >> 8, fx),
        fy,
    );
    [
        (rb & 255) as u8,
        (green & 255) as u8,
        (rb >> 16) as u8,
        (green >> 16) as u8,
    ]
}

// Interpolate two independent eight-bit channels in the low bytes of two u16 lanes.
fn blend_pair(a: u32, b: u32, t: u32) -> u32 {
    const MASK: u32 = 0x00ff00ff;
    (((a & MASK) * (256 - t) + (b & MASK) * t + 0x0080_0080) >> 8) & MASK
}
