use std::hint::black_box;
use std::time::{Duration, Instant};

use solor_core::tree::NodeId;

use super::MEASUREMENT_CACHE_SLOTS;
use super::fixtures::{Column, Drawing, Wrapping};
use super::layout::{Query, Scene, Size};
use super::negotiation::{Button, Row};

const SAMPLES: usize = 7;
const WARMUP: usize = 100;

#[derive(Clone, Copy, Debug)]
enum Change {
    Clean,
    Ink,
    OneLabel,
    TenthLabels,
    WidthPair,
    WidthSweep,
    Height,
}

const CHANGES: [Change; 7] = [
    Change::Clean,
    Change::Ink,
    Change::OneLabel,
    Change::TenthLabels,
    Change::WidthPair,
    Change::WidthSweep,
    Change::Height,
];

struct Fixture {
    scene: Scene,
    labels: Vec<NodeId>,
    inks: Vec<NodeId>,
}

impl Fixture {
    fn new(rows: usize) -> Self {
        let mut scene = Scene::new(Column { padding: 4, gap: 2 });
        let root = scene.root();
        let mut labels = Vec::with_capacity(rows);
        let mut inks = Vec::with_capacity(rows);
        for index in 0..rows {
            let row = scene.append(root, Row { padding: 2, gap: 3 }).unwrap();
            inks.push(scene.append(row, Drawing::new(10, 10)).unwrap());
            labels.push(
                scene
                    .append(
                        row,
                        Wrapping {
                            units: 20 + index as u32 % 80,
                            advance: 2,
                            line_height: 5,
                            ascent: 4,
                        },
                    )
                    .unwrap(),
            );
            scene
                .append(
                    row,
                    Button {
                        units: 4 + index as u32 % 5,
                        advance: 2,
                        line_height: 6,
                        ascent: 4,
                        padding: 2,
                    },
                )
                .unwrap();
        }
        scene.run(Query::Exact(96), None, false).unwrap();
        Self {
            scene,
            labels,
            inks,
        }
    }

    fn step(&mut self, change: Change, step: usize, force: bool) -> usize {
        let mut width = 96;
        let mut assigned = None;
        match change {
            Change::Clean => {}
            Change::Ink => {
                let id = self.inks[step * 37 % self.inks.len()];
                self.scene.set_ink(id, step as u32).unwrap();
            }
            Change::OneLabel | Change::TenthLabels => {
                let count = if matches!(change, Change::OneLabel) {
                    1
                } else {
                    (self.labels.len() / 10).max(1)
                };
                for offset in 0..count {
                    let index = (step * 37 + offset * 10) % self.labels.len();
                    let units = 20 + ((step * 13 + offset * 7) % 80) as u32;
                    self.scene
                        .edit_layout::<Wrapping>(self.labels[index], |label| label.units = units)
                        .unwrap();
                }
            }
            Change::WidthPair => {
                width = if step.is_multiple_of(2) { 96 } else { 120 };
            }
            Change::WidthSweep => {
                width = 80 + (step % 24) as u32 * 2;
            }
            Change::Height => {
                assigned = Some(Size {
                    width: 96,
                    height: 100_000 + (step % 2) as u32,
                });
            }
        }
        self.scene
            .run(Query::Exact(width), assigned, force)
            .unwrap()
    }

    fn fingerprint(&self) -> u64 {
        self.scene
            .snapshot()
            .into_iter()
            .fold(0_u64, |mut hash, geometry| {
                let geometry = geometry.expect("fixture has a complete layout");
                for value in [
                    geometry.metrics.size.width,
                    geometry.metrics.size.height,
                    geometry.metrics.baseline.unwrap_or(u32::MAX),
                    geometry.origin.x,
                    geometry.origin.y,
                    geometry.content.width,
                    geometry.content.height,
                ] {
                    hash = hash.rotate_left(7).wrapping_add(u64::from(value)) ^ 0x9e3779b97f4a7c15;
                }
                hash
            })
    }
}

fn verify() {
    for change in CHANGES {
        let mut incremental = Fixture::new(12);
        let mut reference = Fixture::new(12);
        for step in 0..48 {
            let published = incremental.step(change, step, false);
            reference.step(change, step, true);
            assert_eq!(
                incremental.scene.snapshot(),
                reference.scene.snapshot(),
                "{change:?}, step {step}"
            );
            if matches!(change, Change::Clean | Change::Ink) {
                assert_eq!(published, 0);
            }
        }
    }
}

fn measure(rows: usize, change: Change) {
    let mut fixture = Fixture::new(rows);
    let node_count = rows * 4 + 1;
    let iterations = if matches!(change, Change::Clean | Change::Ink) {
        100_000
    } else {
        (500_000 / node_count).max(20)
    };
    for step in 0..WARMUP {
        black_box(fixture.step(change, step, false));
    }
    let before = fixture.scene.counts();
    let mut samples = [Duration::ZERO; SAMPLES];
    let mut publications = 0;
    println!("begin layout/{rows}/{change:?}");
    for (sample, duration) in samples.iter_mut().enumerate() {
        let start = Instant::now();
        for offset in 0..iterations {
            publications += black_box(black_box(&mut fixture).step(
                change,
                WARMUP + sample * iterations + offset,
                false,
            ));
        }
        *duration = start.elapsed();
    }
    println!("end layout/{rows}/{change:?}");
    let after = fixture.scene.counts();
    samples.sort_unstable();
    let ns = |duration: Duration| duration.as_secs_f64() * 1e9 / iterations as f64;
    let total = (iterations * SAMPLES) as f64;
    println!(
        "layout/{rows}/{change:?}: median_ns={:.1} range_ns=[{:.1},{:.1}] iterations={iterations} samples={SAMPLES} queries={:.1} hits={:.1} measures={:.1} layouts={:.1} changed={:.1} fingerprint={:016x}",
        ns(samples[3]),
        ns(samples[0]),
        ns(samples[6]),
        (after.queries - before.queries) as f64 / total,
        (after.cache_hits - before.cache_hits) as f64 / total,
        (after.measures - before.measures) as f64 / total,
        (after.layouts - before.layouts) as f64 / total,
        publications as f64 / total,
        fixture.fingerprint()
    );
}

pub fn run() {
    println!("measurement_cache_slots={MEASUREMENT_CACHE_SLOTS}");
    verify();
    for rows in [100, 2_500] {
        for change in CHANGES {
            measure(rows, change);
        }
        // Keep the live fixture across the end marker for external heap probes.
        println!("begin storage/{rows}");
        let fixture = black_box(Fixture::new(rows));
        println!("end storage/{rows}");
        black_box(&fixture);
        println!("begin destruction/{rows}");
        drop(fixture);
        println!("end destruction/{rows}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn timed_workloads_match_full_recomputation() {
        super::verify();
    }
}
