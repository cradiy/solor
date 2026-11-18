//! Paired release timings for membership discovery and complete scope disposal.
use super::{Delivery, Fixture, Label, Mount, NodeId, Runtime, SIZE, Scope};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn owns_by_ascent(state: &Runtime<Option<Label>>, root: NodeId, mount: &Mount) -> bool {
    std::iter::successors(Some(mount.state), |&id| state.tree().parent(id).unwrap())
        .any(|id| id == root)
}

struct Workload {
    window: Fixture,
    targets: Vec<Scope>,
    removed: Vec<Vec<NodeId>>,
    survivors: Vec<NodeId>,
}

impl Workload {
    fn new(total: usize, leaves: usize) -> Self {
        let mut window = Fixture::new();
        let groups = 16.min(total / (2 * leaves));
        assert!(total.is_multiple_of(groups * leaves));
        let width = total / groups;
        let mut targets = Vec::new();
        let mut removed = Vec::new();
        let mut survivors = Vec::new();
        for _ in 0..groups {
            let parent = window.scope(None);
            let target = window.scope(Some(parent));
            let nested = window.scope(Some(target));
            let neighbor = window.scope(Some(parent));
            targets.push(target);
            let mut ids = Vec::new();
            for i in 0..width {
                if i.is_multiple_of(width / leaves) {
                    let owner = if ids.len().is_multiple_of(2) {
                        target
                    } else {
                        nested
                    };
                    ids.push(window.mount(owner, "Target"));
                } else {
                    survivors.push(window.mount(neighbor, "Other"));
                }
            }
            removed.push(ids);
        }
        window.scene.run_assigned(SIZE, false).unwrap();
        drop(window.scene.shared_snapshot().unwrap());
        window.state.flush();
        window.log.borrow_mut().clear();
        window.state.set(window.ink, 70).unwrap();
        Self {
            window,
            targets,
            removed,
            survivors,
        }
    }

    fn lookup(&self, descend: bool) -> f64 {
        let mut lookup = Duration::ZERO;
        for (&scope, expected) in self.targets.iter().zip(&self.removed) {
            // Time discovery alone, including materializing the selected identities.
            let start = Instant::now();
            let mut selected = Vec::new();
            if descend {
                self.window
                    .visit_owned(scope.0, |label| selected.push(label.visual));
            } else {
                selected.extend(
                    self.window
                        .members
                        .iter()
                        .filter(|m| owns_by_ascent(&self.window.state, scope.0, m))
                        .map(|m| m.visual),
                );
            }
            black_box(&selected);
            lookup += start.elapsed();
            assert_eq!(selected.len(), expected.len());
            assert!(expected.iter().all(|id| selected.contains(id)));
        }
        lookup.as_secs_f64() * 1e6 / self.targets.len() as f64
    }

    fn dispose(mut self, descend: bool) -> f64 {
        let mut disposal = Duration::ZERO;
        for scope in &self.targets {
            let start = Instant::now();
            if descend {
                self.window.retire_subtree(scope.0);
            } else {
                self.window
                    .retire_matching(scope.0, |state, m| owns_by_ascent(state, scope.0, m));
            }
            disposal += start.elapsed();
        }

        let window = &mut self.window;
        assert!(
            window
                .members
                .iter()
                .map(|m| m.visual)
                .eq(self.survivors.iter().copied())
        );
        assert_eq!(window.state.binding_count(), self.survivors.len());
        assert_eq!(window.state.pending_count(), self.survivors.len());
        let mut drops: std::collections::HashSet<_> =
            self.removed.iter().flatten().copied().collect();
        for entry in window.log.borrow().iter() {
            assert!(matches!(entry, Delivery::Drop(id) if drops.remove(id)));
        }
        assert!(drops.is_empty());
        window.scene.run_assigned(SIZE, false).unwrap();
        assert_eq!(
            window.scene.shared_snapshot().unwrap().len(),
            1 + self.survivors.len()
        );
        disposal.as_secs_f64() * 1e6 / self.targets.len() as f64
    }
}

#[test]
#[ignore = "release timing experiment"]
fn measure_scope_disposal() {
    for (total, leaves) in [
        (8, 1),
        (128, 1),
        (1024, 4),
        (8192, 1),
        (8192, 4),
        (8192, 64),
        (256, 64),
    ] {
        let mut times = [[Vec::new(), Vec::new()], [Vec::new(), Vec::new()]];
        for batch in 0..8 {
            for offset in 0..2 {
                let mode = (batch + offset) % 2;
                let sample = [
                    Workload::new(total, leaves).lookup(mode == 1),
                    Workload::new(total, leaves).dispose(mode == 1),
                ];
                if batch > 0 {
                    for (times, us) in times[mode].iter_mut().zip(sample) {
                        times.push(us);
                    }
                }
            }
        }
        for (name, stages) in ["ascent", "subtree"].into_iter().zip(times) {
            let [lookup, disposal] = stages.map(|mut samples| {
                samples.sort_by(f64::total_cmp);
                samples[3]
            });
            eprintln!(
                "{total} mounts, {leaves} removed, {name}: lookup {lookup:.3} us, disposal {disposal:.3} us"
            );
        }
    }
}
