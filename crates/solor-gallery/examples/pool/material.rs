use super::{
    Basin, Pool,
    water::{Water, resolution},
};
use anyhow::Result;
use solor::{
    EffectShader, ImageAsset, PaintContext,
    layout::{Origin, Size},
    px,
    raster::DeviceSize,
};
use solor_render_cpu::rasterize_shader;
use std::cell::RefCell;

fn origin(x: f32, y: f32) -> Origin {
    Origin { x: px(x), y: px(y) }
}

fn size(w: f32, h: f32) -> Size {
    Size {
        width: px(w),
        height: px(h),
    }
}

pub(super) struct Materials {
    floor: EffectShader,
    wind: EffectShader,
    water: RefCell<Option<(Size, Water)>>,
    stones: [ImageAsset; 2],
    stone: EffectShader,
    coping: EffectShader,
    crown: EffectShader,
}

impl Materials {
    pub(super) fn load() -> Result<Self> {
        let stone = EffectShader::wgsl(include_str!("stone.wgsl"))?;
        let sprite = |kind| {
            rasterize_shader(
                &stone.with_params([kind, 0., 0., 0.]),
                size(96., 96.),
                DeviceSize {
                    width: 96,
                    height: 96,
                },
            )
        };
        Ok(Self {
            floor: EffectShader::wgsl(include_str!("bed.wgsl"))?,
            wind: EffectShader::wgsl(include_str!("wind.wgsl"))?,
            water: RefCell::default(),
            stones: [sprite(0.)?, sprite(1.)?],
            stone,
            coping: EffectShader::wgsl(include_str!("limestone.wgsl"))?,
            crown: EffectShader::wgsl(include_str!("splash.wgsl"))?,
        })
    }

    pub(super) fn paint(&self, cx: &mut PaintContext<'_>, pool: &Pool, now: f64) -> Result<()> {
        let basin = Basin::new(cx.size);
        if basin.width <= 0. || basin.height <= 0. {
            return Ok(());
        }
        let water_size = size(basin.width, basin.height);
        let mut cache = self.water.borrow_mut();
        if cache.as_ref().is_none_or(|(s, _)| *s != water_size) {
            let bed = rasterize_shader(
                &self.floor.with_params(0.).into(),
                water_size,
                resolution(basin),
            )?;
            *cache = Some((water_size, Water::new(bed)));
        }
        if let Some((_, water)) = cache.as_mut() {
            let wind = rasterize_shader(
                &self.wind.with_params(now as f32).into(),
                water_size,
                water.grid_size(),
            )?;
            cx.fill(
                water_size,
                water.frame(basin, pool, now, &wind, &self.stones)?,
            );
        }
        cx.with_offset(origin(0., basin.height), |cx| {
            cx.fill(size(basin.width, basin.ledge), self.coping.with_params(0.0));
            Ok(())
        })?;
        let held = pool.drag.map(|d| d.stone);
        for i in (0..pool.landed.len())
            .filter(|i| Some(*i) != held)
            .chain(held)
        {
            let stone = pool.pebble(i, basin, now);
            let d = stone.diameter;
            if stone.wet >= 1.0 {
                continue;
            }
            let upper_left = origin(stone.center[0] - d * 0.5, stone.center[1] - d * 0.5);
            let draw = |cx: &mut PaintContext<'_>| {
                cx.with_offset(upper_left, |cx| {
                    cx.fill(
                        size(d, d),
                        self.stone.with_params([i as f32, stone.lift, 0., 0.]),
                    );
                    Ok(())
                })
            };
            if stone.wet > 0.0 {
                cx.with_clip(upper_left, size(d, d * (1. - stone.wet)), draw)?;
            } else {
                draw(cx)?;
            }
        }
        for &drop in pool.waves.iter().flatten() {
            let age = (now - drop.contact) as f32;
            if (0.0..0.46).contains(&age) {
                let [x, y] = basin.contact(drop);
                cx.with_clip(Origin::default(), water_size, |cx| {
                    if age < 0.23 {
                        let side = drop.diameter * 1.62;
                        cx.with_offset(origin(x - side * 0.5, y - side * 0.62), |cx| {
                            cx.fill(size(side, side), self.crown.with_params(age));
                            Ok(())
                        })?;
                    }
                    splash(cx, [x, y], age, drop.diameter / 62.)
                })?;
            }
        }
        Ok(())
    }
}

fn splash(cx: &mut PaintContext<'_>, [x, y]: [f32; 2], age: f32, scale: f32) -> Result<()> {
    for i in 0..10 {
        let angle = i as f32 * 2.4;
        let flight = 0.24 + 0.023 * (i % 8) as f32;
        let t = age / flight;
        if t >= 1.0 {
            continue;
        }
        let radius = (10.0 + age * (38.0 + 3.5 * i as f32)) * angle.cos();
        let height = (18. + 1.4 * i as f32) * 4. * t * (1. - t);
        let cx_pos = x + radius * scale;
        let cy_pos = y + (angle.sin() * (8.0 + age * 32.0) - height) * scale;
        let r = (0.8 + 0.32 * (i % 4) as f32) * (1. - t * t) * scale;
        cx.fill_circle(origin(cx_pos, cy_pos), px(r), solor::rgba(0x6fb9b4dd))?;
        cx.fill_circle(
            origin(cx_pos - r * 0.23, cy_pos - r * 0.33),
            px(r * 0.52),
            solor::rgba(0xe9faf0df),
        )?;
    }
    Ok(())
}
