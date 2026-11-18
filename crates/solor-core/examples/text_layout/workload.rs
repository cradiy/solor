use std::hint::black_box;
use std::rc::Rc;
use std::time::{Duration, Instant};

use solor_core::tree::NodeId;

use crate::fixtures::Column;
use crate::layout::{Query, Scene, Size};
use crate::text::{Engine, SharedEngine, Style, Text};

pub const SAMPLE: &str = "Solor 中文排版，文字换行与基线。Hello 世界！Rust GUI 123";
#[cfg(test)]
pub const CHANGED: &str = "测试界面宽度改变。Rust GUI 123 Hello 世界！";
pub const STYLE: Style = Style {
    size: 16,
    line_height: 24,
};

pub struct Fixture {
    pub scene: Scene,
    pub engine: SharedEngine,
    pub labels: Vec<NodeId>,
}

impl Fixture {
    pub fn new(rows: usize, cached: bool) -> Self {
        let engine = Engine::new();
        let mut scene = Scene::new(Column { padding: 2, gap: 3 });
        let root = scene.root();
        let labels = (0..rows)
            .map(|_| {
                scene
                    .append(
                        root,
                        Text::new(Rc::clone(&engine), SAMPLE, STYLE, cached).unwrap(),
                    )
                    .unwrap()
            })
            .collect();
        Self {
            scene,
            engine,
            labels,
        }
    }

    pub fn step(&mut self, update: usize, step: usize, force: bool) {
        let node = self.labels[step % self.labels.len()];
        match update {
            0 => {
                self.scene
                    .edit_layout::<Text>(node, |t| {
                        t.set_content(&format!("{SAMPLE} {}", step % 10)).unwrap()
                    })
                    .unwrap();
            }
            1 => {
                self.scene
                    .edit_layout::<Text>(node, |t| {
                        t.set_style(if t.style() == STYLE {
                            Style {
                                size: 18,
                                line_height: 28,
                            }
                        } else {
                            STYLE
                        })
                        .unwrap()
                    })
                    .unwrap();
            }
            _ => {}
        }
        let width = if update == 4 {
            160 + (step % 17) as u32 * 7
        } else if update == 2 && step.is_multiple_of(2) {
            160
        } else {
            240
        };
        let height = if update == 3 && step.is_multiple_of(2) {
            4000
        } else {
            3000
        };
        self.scene
            .run(Query::Exact(width), Some(Size { width, height }), force)
            .unwrap();
    }

    pub fn cache_usage(&mut self) -> (usize, usize) {
        self.labels.iter().fold((0, 0), |(entries, bytes), &id| {
            let (n, b) = self.scene.body_mut::<Text>(id).unwrap().cache_usage();
            (entries + n, bytes + b)
        })
    }

    pub fn release(&mut self) {
        for &id in &self.labels {
            self.scene.body_mut::<Text>(id).unwrap().release_cache();
        }
        self.scene.release_scratch();
    }
}

pub fn verify() {
    let mut cached = Fixture::new(8, true);
    let mut reference = Fixture::new(8, false);
    for step in 0..96 {
        let update = (step / 3) % 5;
        cached.step(update, step, false);
        reference.step(update, step, true);
        let geometry = cached.scene.snapshot();
        assert_eq!(geometry, reference.scene.snapshot());
        for (index, (&a, &b)) in cached.labels.iter().zip(&reference.labels).enumerate() {
            let query = Query::Exact(geometry[index + 1].unwrap().metrics.size.width);
            let a = cached
                .scene
                .body_mut::<Text>(a)
                .unwrap()
                .resolve(query)
                .unwrap();
            let b = reference
                .scene
                .body_mut::<Text>(b)
                .unwrap()
                .resolve(query)
                .unwrap();
            assert_eq!(a, b, "glyph and line results at step {step}, label {index}");
        }
        assert!(cached.cache_usage().0 <= cached.labels.len() * crate::text::CACHE_SLOTS);
        if step % 13 == 0 {
            cached.release();
        }
    }
    println!(
        "text oracle: 96 mixed updates match forced geometry, lines, clusters and glyph positions"
    );
}

fn median(mut times: [Duration; 7], iterations: u32) -> f64 {
    times.sort_unstable();
    times[3].as_secs_f64() * 1e6 / f64::from(iterations)
}

pub fn run() {
    let cold = std::array::from_fn(|_| {
        let start = Instant::now();
        let mut text = Text::new(Engine::new(), SAMPLE, STYLE, true).unwrap();
        black_box(text.resolve(Query::Exact(240)).unwrap());
        start.elapsed()
    });
    println!(
        "text/cold-engine-and-paragraph: median_us={:.2}",
        median(cold, 1)
    );
    for (update, name) in ["content", "style", "width", "height-only", "width-churn"]
        .iter()
        .enumerate()
    {
        let mut fixtures = [Fixture::new(64, false), Fixture::new(64, true)];
        for fixture in &mut fixtures {
            for step in 0..16 {
                fixture.step(update, step, false);
            }
        }
        let before = fixtures.each_ref().map(|f| f.engine.borrow().counts);
        let mut times = [[Duration::ZERO; 2]; 7];
        for (sample, times) in times.iter_mut().enumerate() {
            for offset in 0..2 {
                let mode = (sample + offset) % 2;
                let start = Instant::now();
                for step in 0..31 {
                    fixtures[mode].step(update, 16 + sample * 31 + step, false);
                }
                times[mode] = start.elapsed();
            }
        }
        assert_eq!(fixtures[0].scene.snapshot(), fixtures[1].scene.snapshot());
        for (mode, fixture) in fixtures.iter_mut().enumerate() {
            let after = fixture.engine.borrow().counts;
            let (entries, bytes) = fixture.cache_usage();
            println!(
                "text/64/{name}/{}: median_us={:.2} shapes_per_update={:.2} hits_per_update={:.2} cache_entries={entries} cache_payload_bytes={bytes}",
                if mode == 0 { "uncached" } else { "cached" },
                median(times.map(|t| t[mode]), 31),
                (after.shapes - before[mode].shapes) as f64 / 217.0,
                (after.hits - before[mode].hits) as f64 / 217.0
            );
        }
    }
    // Allocator markers are outside CPU timing; never compare instrumented timings.
    println!("begin text-engine");
    let engine = Engine::new();
    println!("end text-engine");
    println!("begin text-engine-drop");
    drop(engine);
    println!("end text-engine-drop");
    for cached in [false, true] {
        println!("begin text-warmed/{cached}");
        let mut fixture = Fixture::new(64, cached);
        for step in 0..100 {
            fixture.step(step % 5, step, false);
        }
        // Settle the root width before isolating a local content update.
        fixture.step(0, 101, false);
        println!("end text-warmed/{cached}");
        println!("begin text-update/{cached}");
        fixture.step(0, 102, false);
        println!("end text-update/{cached}");
        // A second long mixed workload checks retention after the initial high-water mark.
        println!("begin text-steady/{cached}");
        for step in 103..1103 {
            fixture.step(step % 5, step, false);
        }
        println!("end text-steady/{cached}");
        println!("begin text-steady-again/{cached}");
        for step in 1103..2103 {
            fixture.step(step % 5, step, false);
        }
        println!("end text-steady-again/{cached}");
        println!("begin text-release/{cached}");
        fixture.release();
        println!("end text-release/{cached}");
        assert_eq!(fixture.cache_usage(), (0, 0));
        println!("begin text-drop/{cached}");
        drop(fixture);
        println!("end text-drop/{cached}");
    }
}
