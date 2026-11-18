use solor_core::px;
use std::hint::black_box;
use std::time::{Duration, Instant};

use solor_core::tree::NodeId;

use super::MEASUREMENT_CACHE_SLOTS;
use super::fixtures::{Column, Drawing, Wrapping};
use super::layout::validation::{CheckOutcome, CheckTarget};
use super::layout::{Query, Scene, Size};
use super::negotiation::{Button, Row};

const SAMPLES: usize = 7;
const WARMUP: usize = 100;

#[path = "selection.rs"]
mod selection;

#[derive(Clone, Copy, Debug)]
enum Change {
    Clean,
    Ink,
    OneLabel,
    StableLabel,
    TenthLabels,
    WidthPair,
    WidthSweep,
    Height,
}

const CHANGES: [Change; 8] = [
    Change::Clean,
    Change::Ink,
    Change::OneLabel,
    Change::StableLabel,
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
    fn prepare(&mut self, change: Change) {
        if matches!(change, Change::StableLabel) {
            for &id in &self.labels {
                self.scene
                    .edit_layout::<Wrapping>(id, |label| label.units = 80)
                    .unwrap();
            }
            self.scene.run(Query::Exact(px(96.0)), None, false).unwrap();
        }
    }

    fn new(rows: usize) -> Self {
        let mut scene = Scene::new(Column::new(px(4.0), px(2.0)));
        let root = scene.root();
        let mut labels = Vec::with_capacity(rows);
        let mut inks = Vec::with_capacity(rows);
        for index in 0..rows {
            let row = scene
                .append(
                    root,
                    Row {
                        padding: px(2.0),
                        gap: px(3.0),
                    },
                )
                .unwrap();
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
                        advance: px(2.0),
                        line_height: px(6.0),
                        ascent: px(4.0),
                        padding: px(2.0),
                    },
                )
                .unwrap();
        }
        scene.run(Query::Exact(px(96.0)), None, false).unwrap();
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
            Change::StableLabel => {
                let id = self.labels[step * 37 % self.labels.len()];
                self.scene
                    .edit_layout::<Wrapping>(id, |label| {
                        label.units = if label.units == 80 { 81 } else { 80 };
                    })
                    .unwrap();
            }
            Change::WidthPair => {
                width = if step.is_multiple_of(2) { 96 } else { 120 };
            }
            Change::WidthSweep => {
                width = 80 + (step % 24) as u32 * 2;
            }
            Change::Height => {
                assigned = Some(Size {
                    width: px(96.0),
                    height: px(100_000.0 + (step % 2) as f32),
                });
            }
        }
        self.scene
            .run(Query::Exact(px(width as f32)), assigned, force)
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
                    geometry.metrics.baseline.unwrap_or(px(-1.0)),
                    geometry.origin.x,
                    geometry.origin.y,
                    geometry.content.width,
                    geometry.content.height,
                ] {
                    hash = hash
                        .rotate_left(7)
                        .wrapping_add(u64::from(value.get().to_bits()))
                        ^ 0x9e3779b97f4a7c15;
                }
                hash
            })
    }
}

fn verify() {
    selection::verify();
    verify_dependency_audit(12);
    for change in CHANGES {
        let mut incremental = Fixture::new(12);
        let mut reference = Fixture::new(12);
        incremental.prepare(change);
        reference.prepare(change);
        let initial = incremental.scene.snapshot();
        for step in 0..48 {
            let published = incremental.step(change, step, false);
            reference.step(change, step, true);
            assert_eq!(
                incremental.scene.snapshot(),
                reference.scene.snapshot(),
                "{change:?}, step {step}"
            );
            if matches!(change, Change::Clean | Change::Ink | Change::StableLabel) {
                assert_eq!(published, 0);
                assert_eq!(incremental.scene.snapshot(), initial);
            }
        }
    }
}

fn verify_dependency_audit(rows: usize) {
    let mut fixture = Fixture::new(rows);
    fixture.prepare(Change::StableLabel);
    let root = fixture.scene.root();
    let published = fixture.scene.snapshot();
    let measured = CheckTarget::Measure(Query::Exact(px(96.0)));
    let allocated = CheckTarget::Layout(published[0].unwrap().metrics.size);
    println!("begin dependency-checkpoints/{rows}");
    let measurement = fixture
        .scene
        .capture_dependencies(root, measured)
        .unwrap()
        .unwrap();
    let layout = fixture
        .scene
        .capture_dependencies(root, allocated)
        .unwrap()
        .unwrap();
    println!("end dependency-checkpoints/{rows}");
    fixture
        .scene
        .edit_layout::<Wrapping>(fixture.labels[0], |label| label.units = 81)
        .unwrap();
    let before = fixture.scene.counts();
    println!("begin dependency-replay/{rows}");
    let measure_report = fixture
        .scene
        .check_dependencies(&measurement, measured)
        .unwrap();
    let layout_report = fixture
        .scene
        .check_dependencies(&layout, allocated)
        .unwrap();
    println!("end dependency-replay/{rows}");
    let after = fixture.scene.counts();
    assert_eq!(measure_report.outcome, CheckOutcome::Unchanged);
    assert_eq!(layout_report.outcome, CheckOutcome::Unchanged);
    assert_eq!(measure_report.checked_reads, rows);
    assert_eq!(layout_report.checked_reads, 2 * rows);
    assert_eq!(layout_report.staged_geometry, rows + 3);
    assert_eq!(fixture.scene.snapshot(), published);
    println!(
        "dependency-audit/{rows}: reads={} staged={} queries={} measures={} layouts={} published=0",
        measure_report.checked_reads + layout_report.checked_reads,
        layout_report.staged_geometry,
        after.queries - before.queries,
        after.measures - before.measures,
        after.layouts - before.layouts,
    );
    assert_eq!(
        fixture.scene.run(Query::Exact(px(96.0)), None, false),
        Ok(0)
    );
    let incremental = fixture.scene.snapshot();
    fixture
        .scene
        .run(Query::Exact(px(96.0)), None, true)
        .unwrap();
    assert_eq!(fixture.scene.snapshot(), incremental);
    assert_eq!(incremental, published);
}

fn measure(rows: usize, change: Change) {
    let mut fixture = Fixture::new(rows);
    fixture.prepare(change);
    let stable_geometry = matches!(change, Change::StableLabel).then(|| fixture.fingerprint());
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
    if let Some(initial) = stable_geometry {
        assert_eq!(publications, 0);
        assert_eq!(fixture.fingerprint(), initial);
    }
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
    let (reads, bytes, incomplete) = fixture.scene.dependency_stats();
    println!(
        "dependencies/{rows}/{change:?}: reads={reads} reserved_bytes={bytes} incomplete={incomplete}"
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
    // Run reclamation probes after timing so they cannot change its allocator history.
    println!("begin reclaimable/2500");
    let mut fixture = Fixture::new(2_500);
    println!("end reclaimable/2500");
    println!("begin release/2500");
    fixture.scene.release_scratch();
    println!("end release/2500");
    println!("begin released-clean/2500");
    assert_eq!(fixture.step(Change::Clean, 0, false), 0);
    println!("end released-clean/2500");
    println!("begin rebuild/2500");
    black_box(fixture.step(Change::OneLabel, 1, false));
    println!("end rebuild/2500");
    println!("begin release-rebuilt/2500");
    fixture.scene.release_scratch();
    println!("end release-rebuilt/2500");
    println!("begin reclaimed-destruction/2500");
    drop(fixture);
    println!("end reclaimed-destruction/2500");
    for change in [Change::StableLabel, Change::WidthPair, Change::WidthSweep] {
        println!("begin warmed-storage/2500/{change:?}");
        let mut fixture = Fixture::new(2_500);
        fixture.prepare(change);
        for step in 0..WARMUP {
            black_box(fixture.step(change, step, false));
        }
        println!("end warmed-storage/2500/{change:?}");
        println!("begin warmed-release/2500/{change:?}");
        fixture.scene.release_scratch();
        println!("end warmed-release/2500/{change:?}");
        println!("begin warmed-destruction/2500/{change:?}");
        drop(fixture);
        println!("end warmed-destruction/2500/{change:?}");
    }
    verify_dependency_audit(2_500);
    selection::run();
}

#[cfg(test)]
mod tests {
    #[test]
    fn timed_workloads_match_full_recomputation() {
        super::verify();
    }
}
