use super::*;
use crate::shared::layout::validation::{Checkpoint, SelectiveCheckpoint};

#[derive(Clone, Copy, Debug)]
enum Mode {
    Plain,
    Linear,
    Selective,
    Shared,
}

#[derive(Clone, Copy, Debug)]
enum Update {
    StableOne,
    StableBatch,
    ChangingOne,
    Resize,
}

const MODES: [Mode; 4] = [Mode::Plain, Mode::Linear, Mode::Selective, Mode::Shared];
const UPDATES: [Update; 4] = [
    Update::StableOne,
    Update::StableBatch,
    Update::ChangingOne,
    Update::Resize,
];

enum Evidence {
    None,
    Linear(Checkpoint, Checkpoint),
    Selective(SelectiveCheckpoint, SelectiveCheckpoint),
}

struct Runner {
    fixture: Fixture,
    mode: Mode,
    evidence: Evidence,
    reads: usize,
    staged: usize,
    refreshes: usize,
    reused: usize,
}

fn targets(width: u32) -> (CheckTarget, CheckTarget, Size) {
    let assigned = Size {
        width,
        height: 100_000,
    };
    (
        CheckTarget::Measure(Query::Exact(width)),
        CheckTarget::Layout(assigned),
        assigned,
    )
}

impl Runner {
    fn new(rows: usize, mode: Mode) -> Self {
        let mut fixture = Fixture::new(rows);
        fixture.prepare(Change::StableLabel);
        fixture
            .scene
            .run(Query::Exact(96), Some(targets(96).2), false)
            .unwrap();
        let mut runner = Self {
            fixture,
            mode,
            evidence: Evidence::None,
            reads: 0,
            staged: 0,
            refreshes: 0,
            reused: 0,
        };
        runner.capture(96);
        runner
    }

    fn capture(&mut self, width: u32) {
        let scene = &mut self.fixture.scene;
        let root = scene.root();
        let (measure, layout, _) = targets(width);
        if let Evidence::Selective(a, b) = &mut self.evidence {
            assert!(scene.refresh_selective_dependencies(a, measure).unwrap());
            assert!(scene.refresh_selective_dependencies(b, layout).unwrap());
            return;
        }
        self.evidence = match self.mode {
            Mode::Plain => Evidence::None,
            Mode::Linear => Evidence::Linear(
                scene.capture_dependencies(root, measure).unwrap().unwrap(),
                scene.capture_dependencies(root, layout).unwrap().unwrap(),
            ),
            Mode::Selective | Mode::Shared => Evidence::Selective(
                scene
                    .capture_selective_dependencies(root, measure)
                    .unwrap()
                    .unwrap(),
                scene
                    .capture_selective_dependencies(root, layout)
                    .unwrap()
                    .unwrap(),
            ),
        };
    }

    fn step(&mut self, update: Update, step: usize, force: bool) -> Option<[CheckOutcome; 2]> {
        let width = if matches!(update, Update::Resize) && step.is_multiple_of(2) {
            120
        } else {
            96
        };
        let edits = match update {
            Update::StableBatch => (self.fixture.labels.len() / 10).max(1),
            Update::Resize => 0,
            _ => 1,
        };
        for offset in 0..edits {
            let id = self.fixture.labels[(step * 37 + offset) % self.fixture.labels.len()];
            self.fixture
                .scene
                .edit_layout::<Wrapping>(id, |leaf| {
                    leaf.units = if matches!(update, Update::ChangingOne) {
                        if leaf.units == 10 { 100 } else { 10 }
                    } else if leaf.units == 80 {
                        81
                    } else {
                        80
                    };
                })
                .unwrap();
        }
        let (measure, layout, assigned) = targets(width);
        let scene = &mut self.fixture.scene;
        let shared = matches!(self.mode, Mode::Shared);
        let reports = match &mut self.evidence {
            Evidence::None => None,
            Evidence::Linear(a, b) => Some([
                scene.check_dependencies(a, measure).unwrap(),
                scene.check_dependencies(b, layout).unwrap(),
            ]),
            Evidence::Selective(a, b) if shared => {
                let report = scene
                    .run_validated(Query::Exact(width), assigned, a, b)
                    .unwrap();
                self.reused += report.reused_layouts;
                Some(report.checks)
            }
            Evidence::Selective(a, b) => Some([
                scene.check_selective_dependencies(a, measure).unwrap(),
                scene.check_selective_dependencies(b, layout).unwrap(),
            ]),
        };
        if let Some(reports) = reports {
            self.reads += reports.iter().map(|r| r.checked_reads).sum::<usize>();
            self.staged += reports.iter().map(|r| r.staged_geometry).sum::<usize>();
        }
        if !shared {
            scene
                .run(Query::Exact(width), Some(assigned), force)
                .unwrap();
        }
        let outcomes = reports.map(|reports| reports.map(|r| r.outcome));
        if outcomes.is_some_and(|outcomes| outcomes.iter().any(|&o| o != CheckOutcome::Unchanged)) {
            self.capture(width);
            self.refreshes += 1;
        }
        outcomes
    }
}

pub(super) fn verify() {
    let mut linear = Runner::new(12, Mode::Linear);
    let mut selective = Runner::new(12, Mode::Selective);
    let mut shared = Runner::new(12, Mode::Shared);
    let mut reference = Runner::new(12, Mode::Plain);
    for step in 0..384 {
        let update = UPDATES[(step / 5) % UPDATES.len()];
        let expected = linear.step(update, step, false);
        assert_eq!(selective.step(update, step, false), expected);
        assert_eq!(shared.step(update, step, false), expected);
        reference.step(update, step, true);
        assert_eq!(
            linear.fixture.scene.snapshot(),
            reference.fixture.scene.snapshot()
        );
        assert_eq!(
            selective.fixture.scene.snapshot(),
            reference.fixture.scene.snapshot()
        );
        assert_eq!(
            shared.fixture.scene.snapshot(),
            reference.fixture.scene.snapshot()
        );
    }
}

fn measure_selection(rows: usize, update: Update) {
    let mut runners = MODES.map(|mode| Runner::new(rows, mode));
    for step in 0..30 {
        for runner in &mut runners {
            runner.step(update, step, false);
        }
    }
    let before = runners.each_ref().map(|r| {
        (
            r.fixture.scene.counts(),
            r.reads,
            r.staged,
            r.refreshes,
            r.reused,
        )
    });
    let iterations = if rows == 100 { 300 } else { 30 };
    let mut samples = [[Duration::ZERO; MODES.len()]; SAMPLES];
    for (sample, times) in samples.iter_mut().enumerate() {
        for offset in 0..MODES.len() {
            let mode = (sample + offset) % MODES.len();
            let start = Instant::now();
            for step in 0..iterations {
                black_box(runners[mode].step(update, 30 + sample * iterations + step, false));
            }
            times[mode] = start.elapsed();
        }
    }
    let expected = runners[0].fixture.scene.snapshot();
    for (index, runner) in runners.iter().enumerate() {
        assert_eq!(runner.fixture.scene.snapshot(), expected);
        let mut times = samples.map(|sample| sample[index]);
        times.sort_unstable();
        let ns = |time: Duration| time.as_secs_f64() * 1e9 / iterations as f64;
        let total = (iterations * SAMPLES) as f64;
        let after = runner.fixture.scene.counts();
        let (counts, reads, staged, refreshes, reused) = before[index];
        println!(
            "selection/{rows}/{update:?}/{:?}: median_ns={:.1} range_ns=[{:.1},{:.1}] reads={:.1} staged={:.1} measures={:.1} layouts={:.1} refreshes={:.1} reused={:.1}",
            runner.mode,
            ns(times[3]),
            ns(times[0]),
            ns(times[6]),
            (runner.reads - reads) as f64 / total,
            (runner.staged - staged) as f64 / total,
            (after.measures - counts.measures) as f64 / total,
            (after.layouts - counts.layouts) as f64 / total,
            (runner.refreshes - refreshes) as f64 / total,
            (runner.reused - reused) as f64 / total,
        );
    }
}

pub(super) fn run() {
    for rows in [100, 2_500] {
        for update in UPDATES {
            measure_selection(rows, update);
        }
    }
    // Allocation probes are outside timing and retain evidence across each end marker.
    for mode in MODES {
        println!("begin selection-storage/2500/{mode:?}");
        let mut runner = Runner::new(2_500, mode);
        println!("end selection-storage/2500/{mode:?}");
        for update in UPDATES {
            runner.step(update, 0, false);
            println!("begin selection-update/2500/{mode:?}/{update:?}");
            runner.step(update, 1, false);
            println!("end selection-update/2500/{mode:?}/{update:?}");
        }
        println!("begin selection-destruction/2500/{mode:?}");
        drop(runner);
        println!("end selection-destruction/2500/{mode:?}");
        println!("begin selection-warmed/2500/{mode:?}");
        let mut runner = Runner::new(2_500, mode);
        for step in 0..100 {
            runner.step(Update::StableBatch, step, false);
        }
        println!("end selection-warmed/2500/{mode:?}");
        println!("begin selection-release/2500/{mode:?}");
        runner.fixture.scene.release_scratch();
        println!("end selection-release/2500/{mode:?}");
        println!("begin selection-warmed-destruction/2500/{mode:?}");
        drop(runner);
        println!("end selection-warmed-destruction/2500/{mode:?}");
    }
}
