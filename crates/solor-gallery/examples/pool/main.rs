//! Drag river stones from the coping into a sunlit pool.
mod material;
mod water;

use anyhow::Result;
use material::Materials;
use solor::{
    Div, Render, Styled, Ui, component, div,
    layout::Size,
    pointer::{PointerEvent, PointerId},
    px, rgb, text,
};
use solor_platform::{WindowOptions, current_platform};

const AIR_TIME: f32 = 0.19;
const FALL: f32 = 14.0;

#[derive(Clone, Copy)]
struct Drop {
    uv: [f32; 2],
    contact: f64,
    fall: f32,
    diameter: f32,
}

#[derive(Clone, Copy)]
struct Drag {
    pointer: PointerId,
    stone: usize,
    offset: [f32; 2],
    center: [f32; 2],
}

#[derive(Default)]
struct Pool {
    landed: [Option<Drop>; 2],
    drag: Option<Drag>,
    waves: [Option<Drop>; 8],
}

#[derive(Clone, Copy)]
struct Basin {
    width: f32,
    height: f32,
    ledge: f32,
}

impl Basin {
    fn new(size: Size) -> Self {
        let ledge = (size.height.get() * 0.22).min(72.0);
        Self {
            width: size.width.get(),
            height: size.height.get() - ledge,
            ledge,
        }
    }

    fn stone_diameter(self, i: usize) -> f32 {
        (if i == 0 { 62.0_f32 } else { 50.0 }).min(self.ledge)
    }

    fn contact(self, drop: Drop) -> [f32; 2] {
        [drop.uv[0] * self.width, drop.uv[1] * self.height]
    }
}

struct Pebble {
    center: [f32; 2],
    diameter: f32,
    wet: f32,
    depth: f32,
    lift: f32,
}

impl Pool {
    fn pebble(&self, i: usize, basin: Basin, now: f64) -> Pebble {
        let diameter = basin.stone_diameter(i);
        if let Some(drag) = self.drag.filter(|drag| drag.stone == i) {
            return Pebble {
                center: drag.center,
                diameter,
                wet: 0.,
                depth: 0.,
                lift: 1.,
            };
        }
        if let Some(drop) = self.landed[i] {
            let [x, y] = basin.contact(drop);
            let age = (now - drop.contact) as f32;
            let falling = ((age + AIR_TIME) / AIR_TIME).clamp(0., 1.);
            let lift = 1.0 - falling * falling;
            let descent = 1.0 - (-age.max(0.) * (3.4 + drop.diameter * 0.014)).exp();
            let entry_time = drop.contact as f32;
            let current_x = 6.0 * (y * 0.023 - entry_time * 0.71).sin();
            let current_y = 3.0 * (x * 0.017 + entry_time * 0.51).sin();
            return Pebble {
                center: [
                    (x + current_x * descent).clamp(0., basin.width),
                    (y - drop.fall * lift + (9.0 + current_y) * descent).min(basin.height),
                ],
                diameter: diameter * (1. - 0.20 * descent),
                wet: (descent * 2.0).min(1.),
                depth: descent,
                lift,
            };
        }
        Pebble {
            center: [
                basin.width * 0.5 + (i as f32 - 0.5) * 74.0_f32.min(basin.width * 0.4),
                basin.height + basin.ledge * 0.48,
            ],
            diameter,
            wet: 0.,
            depth: 0.,
            lift: 0.,
        }
    }

    fn grab(&mut self, pointer: PointerId, point: [f32; 2], basin: Basin, now: f64) -> bool {
        if self.drag.is_some() {
            return false;
        }
        for i in (0..self.landed.len()).rev() {
            if self.landed[i].is_some_and(|drop| now < drop.contact) {
                continue;
            }
            let stone = self.pebble(i, basin, now);
            let offset = [stone.center[0] - point[0], stone.center[1] - point[1]];
            if (offset[0] / (stone.diameter * 0.39)).hypot(offset[1] / (stone.diameter * 0.31))
                <= 1.0
            {
                self.drag = Some(Drag {
                    pointer,
                    stone: i,
                    offset,
                    center: stone.center,
                });
                return true;
            }
        }
        false
    }

    fn release(&mut self, event: &PointerEvent, now: f64) {
        let Some(drag) = self.drag.filter(|drag| drag.pointer == event.pointer_id()) else {
            return;
        };
        self.drag = None;
        let basin = Basin::new(event.content_size());
        let [x, y] = drag.at(point(event));
        if x < 0. || x >= basin.width || y < 0. || y >= basin.height + basin.ledge {
            return;
        }
        if y >= basin.height {
            self.landed[drag.stone] = None;
            return;
        }
        let fall = FALL.min(basin.height - y);
        let drop = Drop {
            uv: [x / basin.width, (y + fall) / basin.height],
            contact: now + f64::from(AIR_TIME),
            fall,
            diameter: basin.stone_diameter(drag.stone),
        };
        self.landed[drag.stone] = Some(drop);
        self.waves.rotate_right(1);
        self.waves[0] = Some(drop);
    }
}

impl Drag {
    fn at(self, p: [f32; 2]) -> [f32; 2] {
        [p[0] + self.offset[0], p[1] + self.offset[1]]
    }
}

fn point(event: &PointerEvent) -> [f32; 2] {
    let p = event.content_position();
    [p.x.get(), p.y.get()]
}

fn scene(cx: &mut solor::Context<'_>, materials: Materials) -> Div {
    let pool = cx.signal(Pool::default());
    let clock = cx.signal(0.0_f64);
    let mut start = None;
    div()
        .flex_1()
        .w_full()
        .overflow_hidden()
        .paint(move |cx| {
            let now = *cx.read(clock)?;
            let state = cx.read(pool)?;
            materials.paint(cx, state, now)
        })
        .on_frame(move |now, cx| {
            let elapsed = now.saturating_sub(*start.get_or_insert(now));
            cx.set(clock, elapsed.as_secs_f64())
        })
        .on_pointer_down(move |event, cx| {
            if event.is_primary_action() {
                let now = *cx.read(clock)?;
                if cx.update_in_place(pool, |p| {
                    p.grab(
                        event.pointer_id(),
                        point(event),
                        Basin::new(event.content_size()),
                        now,
                    )
                })? {
                    cx.capture_pointer();
                }
            }
            Ok(())
        })
        .on_pointer_move(move |event, cx| {
            if cx
                .read(pool)?
                .drag
                .is_some_and(|d| d.pointer == event.pointer_id())
            {
                cx.update_in_place(pool, |p| {
                    if let Some(drag) = &mut p.drag {
                        drag.center = drag.at(point(event));
                    }
                })?;
            }
            Ok(())
        })
        .on_pointer_up(move |event, cx| {
            if event.is_primary_action() {
                let now = *cx.read(clock)?;
                cx.update_in_place(pool, |p| p.release(event, now))?;
            }
            Ok(())
        })
        .on_pointer_cancel(move |id, cx| {
            if cx.read(pool)?.drag.is_some_and(|d| d.pointer == id) {
                cx.update_in_place(pool, |p| p.drag = None)?;
            }
            Ok(())
        })
}

fn view(materials: Materials) -> impl Render {
    component(move |cx| {
        div()
            .w_full()
            .h_full()
            .flex_col()
            .bg(rgb(0xe9e3d7))
            .child(scene(cx, materials))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(18.))
                    .py(px(10.))
                    .child(
                        text("Stillwater")
                            .text_size(px(18.))
                            .text_color(rgb(0x23443f)),
                    )
                    .child(
                        text("Drag a stone into the water")
                            .text_size(px(14.))
                            .text_color(rgb(0x43534b)),
                    ),
            )
    })
}

fn main() -> Result<()> {
    current_platform()?.run_window(
        WindowOptions::new("Stillwater")
            .app_id("org.solor.Stillwater")
            .size(px(680.), px(454.)),
        Ui::new(),
        view(Materials::load()?),
    )
}
