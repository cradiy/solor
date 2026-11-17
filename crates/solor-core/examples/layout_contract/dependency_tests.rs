use std::any::Any;

use super::fixtures::{Column, Drawing, Wrapping};
use super::fixtures::{Fault, Phase};
use super::layout::validation::{CheckOutcome, CheckTarget};
use super::layout::{
    LayoutContext, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query, Result, Scene,
    Size,
};
use super::layout::{LayoutError, MAX_RECORDED_READS, Read, ReadSet};
use super::negotiation::fixture;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[path = "selective_tests.rs"]
mod selective_tests;

#[test]
fn validation_separates_measurements_from_final_baseline_and_content() {
    let initial = LayoutOutput {
        baseline: Some(4),
        content: Size {
            width: 12,
            height: 10,
        },
    };
    for output in [
        LayoutOutput {
            baseline: Some(8),
            ..initial
        },
        LayoutOutput {
            content: Size {
                width: 120,
                height: 20,
            },
            ..initial
        },
    ] {
        let mut scene = Scene::new(Column { padding: 0, gap: 0 });
        let root = scene.root();
        let child = scene.append(root, OutputLeaf(initial)).unwrap();
        scene.run(Query::Exact(100), None, false).unwrap();
        let measured = CheckTarget::Measure(Query::Exact(100));
        let allocated = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
        let measurement = scene.capture_dependencies(root, measured).unwrap().unwrap();
        let layout = scene
            .capture_dependencies(root, allocated)
            .unwrap()
            .unwrap();
        let before = scene.snapshot();
        let reads = scene.published_layout_reads(root);
        scene
            .edit_layout::<OutputLeaf>(child, |leaf| leaf.0 = output)
            .unwrap();
        let report = scene.check_dependencies(&measurement, measured).unwrap();
        assert_eq!(report.outcome, CheckOutcome::Unchanged);
        assert_eq!(report.checked_reads, 1);
        let report = scene.check_dependencies(&layout, allocated).unwrap();
        assert_eq!(report.outcome, CheckOutcome::Changed);
        assert_eq!(report.checked_reads, 2);
        assert_eq!(report.staged_geometry, 1);
        assert_eq!(scene.snapshot(), before);
        assert_eq!(scene.published_layout_reads(root), reads);
        assert!(scene.is_pending());
        scene.run(Query::Exact(100), None, false).unwrap();
        let incremental = scene.snapshot();
        scene.run(Query::Exact(100), None, true).unwrap();
        assert_eq!(scene.snapshot(), incremental);
    }
}

#[test]
fn unchanged_dependencies_do_not_publish_internal_movement() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let container = scene.append(root, MovingChild { x: 0 }).unwrap();
    let child = scene.append(container, Drawing::new(5, 5)).unwrap();
    scene.run(Query::Exact(100), None, false).unwrap();
    let target = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
    let checkpoint = scene.capture_dependencies(root, target).unwrap().unwrap();
    let before = scene.snapshot();
    let reads = scene.published_layout_reads(container);
    scene
        .edit_layout::<MovingChild>(container, |parent| parent.x = 7)
        .unwrap();
    let report = scene.check_dependencies(&checkpoint, target).unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unchanged);
    assert_eq!(report.checked_reads, 2);
    assert_eq!(report.staged_geometry, 2);
    assert_eq!(scene.snapshot(), before);
    assert_eq!(scene.published_layout_reads(container), reads);
    assert!(scene.scratch_usage().iter().all(|&(len, _)| len == 0));
    scene.run(Query::Exact(100), None, false).unwrap();
    assert_eq!(scene.geometry(child).unwrap().origin.x, 7);
    let incremental = scene.snapshot();
    scene.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
}

#[test]
fn checkpoints_replay_original_queries_after_cache_eviction() {
    let (mut scene, ids) = fixture();
    let root = scene.root();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let checkpoint = scene.capture_dependencies(root, target).unwrap().unwrap();
    let exact = scene.probe(ids.label, Query::Exact(18)).unwrap();
    for width in [19, 20, 21, 22] {
        scene.probe(ids.label, Query::Exact(width)).unwrap();
        scene.probe(root, Query::Exact(width)).unwrap();
    }
    assert!(scene.capture_dependencies(root, target).unwrap().is_none());
    assert_eq!(
        scene
            .check_dependencies(&checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unchanged
    );
    scene
        .edit_layout::<Wrapping>(ids.label, |label| label.units = 21)
        .unwrap();
    assert_eq!(scene.probe(ids.label, Query::Exact(18)).unwrap(), exact);
    assert_eq!(
        scene
            .check_dependencies(&checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    scene.run(Query::Natural, None, false).unwrap();
    assert_eq!(scene.geometry(root).unwrap().metrics.size.width, 74);
}

#[test]
fn validation_requires_matching_request_and_unchanged_owner_inputs() {
    let mut scene = Scene::new(Drawing::new(5, 5));
    let root = scene.root();
    let target = CheckTarget::Measure(Query::Natural);
    assert!(scene.capture_dependencies(root, target).unwrap().is_none());
    scene.run(Query::Natural, None, false).unwrap();
    let checkpoint = scene.capture_dependencies(root, target).unwrap().unwrap();
    scene.set_ink(root, 17).unwrap();
    let before = scene.counts();
    assert_eq!(
        scene
            .check_dependencies(&checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unchanged
    );
    for mismatch in [
        CheckTarget::Measure(Query::Exact(5)),
        CheckTarget::Layout(Size {
            width: 5,
            height: 5,
        }),
    ] {
        assert_eq!(
            scene
                .check_dependencies(&checkpoint, mismatch)
                .unwrap()
                .outcome,
            CheckOutcome::Unverifiable
        );
    }
    assert_eq!(scene.counts(), before);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            scene
                .edit_layout::<Drawing>(root, |leaf| {
                    leaf.size.width = 8;
                    panic!("interrupted owner mutation");
                })
                .unwrap();
        }))
        .is_err()
    );
    assert!(scene.capture_dependencies(root, target).unwrap().is_none());
    assert_eq!(
        scene
            .check_dependencies(&checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
    scene
        .edit_layout::<Drawing>(root, |leaf| leaf.size.width = 5)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    assert_eq!(
        scene
            .check_dependencies(&checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
    let mut foreign = Scene::new(Drawing::new(5, 5));
    assert!(foreign.check_dependencies(&checkpoint, target).is_err());
}

#[test]
fn incomplete_evidence_never_validates_as_empty() {
    let (mut scene, _) = reading_fixture();
    let root = scene.root();
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.repeats = MAX_RECORDED_READS + 1)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let checkpoint = scene.capture_dependencies(root, target).unwrap().unwrap();
    let before = scene.counts();
    let report = scene.check_dependencies(&checkpoint, target).unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unverifiable);
    assert_eq!(report.checked_reads, 0);
    assert_eq!(scene.counts(), before);
}

#[test]
fn failed_validation_discards_staging_and_preserves_retry() {
    for phase in [Phase::Measure, Phase::Layout] {
        for fault in [Fault::Error, Fault::Panic] {
            let mut scene = Scene::new(Column { padding: 0, gap: 0 });
            let root = scene.root();
            let first = scene.append(root, Drawing::new(5, 5)).unwrap();
            let second = scene.append(root, Drawing::new(5, 5)).unwrap();
            scene.run(Query::Natural, None, false).unwrap();
            let target = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
            let checkpoint = scene.capture_dependencies(root, target).unwrap().unwrap();
            let before = scene.snapshot();
            let reads = scene.published_layout_reads(root);
            scene.edit_layout::<Drawing>(first, |_| {}).unwrap();
            scene
                .edit_layout::<Drawing>(second, |leaf| leaf.fault = Some((phase, fault)))
                .unwrap();
            let result = catch_unwind(AssertUnwindSafe(|| {
                scene.check_dependencies(&checkpoint, target)
            }));
            match fault {
                Fault::Error => assert_eq!(result.unwrap(), Err(LayoutError::Injected)),
                Fault::Panic => assert!(result.is_err()),
            }
            assert_eq!(scene.snapshot(), before);
            assert_eq!(scene.published_layout_reads(root), reads);
            assert!(scene.scratch_usage().iter().all(|&(len, _)| len == 0));
            assert!(scene.is_pending());
            scene.body_mut::<Drawing>(second).unwrap().fault = None;
            assert_eq!(
                scene
                    .check_dependencies(&checkpoint, target)
                    .unwrap()
                    .outcome,
                CheckOutcome::Unchanged
            );
            scene.run(Query::Natural, None, false).unwrap();
            assert!(!scene.is_pending());
            assert_eq!(scene.snapshot(), before);
            scene.run(Query::Natural, None, true).unwrap();
            assert_eq!(scene.snapshot(), before);
        }
    }
}

#[test]
fn records_belong_to_each_query_and_survive_child_cache_eviction() {
    let (mut scene, ids) = fixture();
    let root = scene.root();
    scene.probe(root, Query::Natural).unwrap();
    let natural = scene.measurement_reads(root, Query::Natural).unwrap();
    assert!(natural.complete);
    assert_eq!(natural.reads.len(), 4);
    assert!(natural.reads.iter().any(|read| matches!(read,
        Read::Measure { node, query: Query::Exact(40), .. } if *node == ids.label)));
    scene.probe(root, Query::Exact(50)).unwrap();
    let narrow = scene.measurement_reads(root, Query::Exact(50)).unwrap();
    assert!(narrow.reads.iter().any(|read| matches!(read,
        Read::Measure { node, query: Query::Exact(18), .. } if *node == ids.label)));
    assert_eq!(
        scene.measurement_reads(root, Query::Natural),
        Some(natural.clone())
    );
    let count = scene.node_counts(root).measures;
    scene.probe(root, Query::Natural).unwrap();
    assert_eq!(scene.node_counts(root).measures, count);
    assert_eq!(
        scene.measurement_reads(root, Query::Natural),
        Some(natural.clone())
    );
    for width in [1, 2, 3, 4] {
        scene.probe(ids.label, Query::Exact(width)).unwrap();
    }
    assert!(scene.measurement_reads(ids.label, Query::Natural).is_none());
    assert_eq!(scene.measurement_reads(root, Query::Natural), Some(natural));
    // Parent eviction removes that result's evidence, not another query's evidence.
    scene.probe(root, Query::AtMost(60)).unwrap();
    scene.probe(root, Query::Exact(70)).unwrap();
    assert!(scene.measurement_reads(root, Query::Natural).is_none());
    assert_eq!(
        scene.measurement_reads(root, Query::Exact(50)),
        Some(narrow)
    );
    scene.run(Query::Exact(80), None, false).unwrap();
    let label_count = scene.node_counts(ids.label).layouts;
    scene
        .run(
            Query::Exact(80),
            Some(Size {
                width: 80,
                height: 34,
            }),
            false,
        )
        .unwrap();
    assert_eq!(scene.node_counts(ids.label).layouts, label_count);
    let reads = scene.published_layout_reads(root).unwrap();
    assert!(reads.complete);
    assert_eq!(
        reads
            .reads
            .iter()
            .filter(|read| matches!(read, Read::Measure { .. }))
            .count(),
        4
    );
    assert_eq!(
        reads
            .reads
            .iter()
            .filter(|read| matches!(read, Read::Layout { .. }))
            .count(),
        3
    );
    let label = scene.geometry(ids.label).unwrap();
    assert!(reads.reads.contains(&Read::Layout {
        node: ids.label,
        assigned: label.metrics.size,
        output: LayoutOutput {
            baseline: label.metrics.baseline,
            content: label.content
        },
    }));
    let before_release = scene.published_layout_reads(root);
    let measured_before_release = scene.measurement_reads(root, Query::Exact(80));
    scene.release_scratch();
    assert_eq!(scene.published_layout_reads(root), before_release);
    assert_eq!(
        scene.measurement_reads(root, Query::Exact(80)),
        measured_before_release
    );
}

struct ReadingParent {
    selected: usize,
    repeats: usize,
    fault: Option<Fault>,
}

impl LayoutNode for ReadingParent {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        if self.repeats == 0 {
            return Ok(Metrics {
                size: Size {
                    width: query.resolve(5),
                    height: 5,
                },
                baseline: None,
            });
        }
        let mut result = cx.measure_child(self.selected, query)?;
        for _ in 1..self.repeats {
            result = cx.measure_child(self.selected, query)?;
        }
        match self.fault {
            Some(Fault::Error) => return Err(LayoutError::Injected),
            Some(Fault::Panic) => panic!("intentional recording failure"),
            None => {}
        }
        Ok(result)
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let _ = cx.measure_child(self.selected, Query::Natural)?;
        for index in 0..cx.child_count() {
            cx.layout_child(
                index,
                Size {
                    width: 5,
                    height: 5,
                },
            )?;
            cx.place_child(index, Origin::default())?;
        }
        Ok(LayoutOutput {
            baseline: None,
            content: assigned,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn reading_fixture() -> (Scene, [solor_core::tree::NodeId; 2]) {
    let mut scene = Scene::new(ReadingParent {
        selected: 0,
        repeats: 1,
        fault: None,
    });
    let root = scene.root();
    let a = scene.append(root, Drawing::new(5, 5)).unwrap();
    let b = scene.append(root, Drawing::new(7, 5)).unwrap();
    (scene, [a, b])
}

fn measurement_nodes(reads: &ReadSet) -> Vec<solor_core::tree::NodeId> {
    reads
        .reads
        .iter()
        .filter_map(|read| match read {
            Read::Measure { node, .. } => Some(*node),
            Read::Layout { .. } => None,
        })
        .collect()
}

#[test]
fn conditional_reads_replace_old_edges_and_cache_hits_are_recorded() {
    let (mut scene, children) = reading_fixture();
    let root = scene.root();
    scene.run(Query::Natural, None, false).unwrap();
    assert_eq!(
        measurement_nodes(&scene.published_layout_reads(root).unwrap()),
        vec![children[0]]
    );
    let child_measures = scene.node_counts(children[1]).measures;
    scene.probe(children[1], Query::Natural).unwrap();
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.selected = 1)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    assert_eq!(scene.node_counts(children[1]).measures, child_measures + 1);
    assert_eq!(
        measurement_nodes(&scene.measurement_reads(root, Query::Natural).unwrap()),
        vec![children[1]]
    );
    assert_eq!(
        measurement_nodes(&scene.published_layout_reads(root).unwrap()),
        vec![children[1]]
    );
    assert_eq!(
        scene.measurement_reads(children[1], Query::Natural),
        Some(ReadSet::default())
    );
}

#[test]
fn failed_measurements_never_expose_partial_records_and_retry_replaces_them() {
    for fault in [Fault::Error, Fault::Panic] {
        let (mut scene, children) = reading_fixture();
        let root = scene.root();
        scene.run(Query::Natural, None, false).unwrap();
        let layout_reads = scene.published_layout_reads(root);
        scene
            .edit_layout::<ReadingParent>(root, |parent| {
                parent.selected = 1;
                parent.fault = Some(fault);
            })
            .unwrap();
        let outcome = catch_unwind(AssertUnwindSafe(|| scene.run(Query::Natural, None, false)));
        match fault {
            Fault::Error => assert_eq!(outcome.unwrap(), Err(LayoutError::Injected)),
            Fault::Panic => assert!(outcome.is_err()),
        }
        assert!(scene.measurement_reads(root, Query::Natural).is_none());
        assert_eq!(scene.published_layout_reads(root), layout_reads);
        scene.body_mut::<ReadingParent>(root).unwrap().fault = None;
        scene.run(Query::Natural, None, false).unwrap();
        assert_eq!(
            measurement_nodes(&scene.measurement_reads(root, Query::Natural).unwrap()),
            vec![children[1]]
        );
    }
}

#[test]
fn later_failure_does_not_publish_an_earlier_subtrees_new_dependency_record() {
    for fault in [Fault::Error, Fault::Panic] {
        let mut scene = Scene::new(Column { padding: 0, gap: 0 });
        let root = scene.root();
        let branch = scene
            .append(
                root,
                ReadingParent {
                    selected: 0,
                    repeats: 1,
                    fault: None,
                },
            )
            .unwrap();
        scene.append(branch, Drawing::new(5, 5)).unwrap();
        let selected = scene.append(branch, Drawing::new(7, 5)).unwrap();
        let last = scene.append(root, Drawing::new(5, 5)).unwrap();
        scene.run(Query::Exact(100), None, false).unwrap();
        let published = scene.snapshot();
        let root_reads = scene.published_layout_reads(root);
        let branch_reads = scene.published_layout_reads(branch);
        scene
            .edit_layout::<ReadingParent>(branch, |parent| parent.selected = 1)
            .unwrap();
        scene
            .edit_layout::<Drawing>(last, |drawing| drawing.fault = Some((Phase::Layout, fault)))
            .unwrap();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            scene.run(Query::Exact(100), None, false)
        }));
        match fault {
            Fault::Error => assert_eq!(outcome.unwrap(), Err(LayoutError::Injected)),
            Fault::Panic => assert!(outcome.is_err()),
        }
        assert_eq!(scene.snapshot(), published);
        assert_eq!(scene.published_layout_reads(root), root_reads);
        assert_eq!(scene.published_layout_reads(branch), branch_reads);
        scene.body_mut::<Drawing>(last).unwrap().fault = None;
        scene.run(Query::Exact(100), None, false).unwrap();
        assert_eq!(
            measurement_nodes(&scene.published_layout_reads(branch).unwrap()),
            vec![selected]
        );
    }
}

#[test]
fn exceeding_the_recording_budget_is_explicit_and_can_recover() {
    let (mut scene, _) = reading_fixture();
    let root = scene.root();
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.repeats = MAX_RECORDED_READS + 1)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    let reads = scene.measurement_reads(root, Query::Natural).unwrap();
    assert!(!reads.complete);
    assert!(scene.has_record_block(root));
    assert!(reads.reads.is_empty());
    assert_eq!(reads.reads.capacity(), 0);
    assert_eq!(scene.dependency_stats().2, 1);
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.repeats = 1)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    let reads = scene.measurement_reads(root, Query::Natural).unwrap();
    assert!(reads.complete);
    assert_eq!(reads.reads.len(), 1);
    assert_eq!(scene.dependency_stats().2, 0);
}

#[test]
fn empty_evidence_avoids_a_record_block_and_releases_an_unused_block() {
    let mut leaf = Scene::new(Drawing::new(5, 5));
    leaf.run(Query::Natural, None, false).unwrap();
    assert!(!leaf.has_record_block(leaf.root()));
    assert_eq!(
        leaf.measurement_reads(leaf.root(), Query::Natural),
        Some(ReadSet::default())
    );
    assert_eq!(
        leaf.published_layout_reads(leaf.root()),
        Some(ReadSet::default())
    );

    let (mut scene, children) = reading_fixture();
    let root = scene.root();
    for _ in 0..3 {
        scene
            .edit_layout::<ReadingParent>(root, |parent| parent.repeats = 1)
            .unwrap();
        scene.probe(root, Query::Natural).unwrap();
        assert!(scene.has_record_block(root));
        assert!(!scene.has_record_block(children[0]));
        scene
            .edit_layout::<ReadingParent>(root, |parent| parent.repeats = 0)
            .unwrap();
        scene.probe(root, Query::Natural).unwrap();
        assert!(!scene.has_record_block(root));
        assert_eq!(
            scene.measurement_reads(root, Query::Natural),
            Some(ReadSet::default())
        );
    }
    // A layout record has its own lifetime even when measurement reads disappear.
    scene.run(Query::Natural, None, false).unwrap();
    let published = scene.published_layout_reads(root);
    assert!(scene.has_record_block(root));
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.repeats = 0)
        .unwrap();
    scene.probe(root, Query::Natural).unwrap();
    assert!(scene.has_record_block(root));
    assert_eq!(scene.published_layout_reads(root), published);
}

#[test]
fn equal_exact_metrics_do_not_cover_a_parents_natural_query() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Natural, None, false).unwrap();
    let root = scene.root();
    let before = scene.geometry(root).unwrap();
    let exact = scene.probe(ids.label, Query::Exact(18)).unwrap();
    assert_eq!(before.metrics.size.width, 72);
    // Evict Natural from the three-slot measurement cache. It is still a
    // dependency of the parent's published sizing result.
    for width in [19, 20, 21, 22] {
        scene.probe(ids.label, Query::Exact(width)).unwrap();
    }
    scene
        .edit_layout::<Wrapping>(ids.label, |label| label.units = 21)
        .unwrap();
    assert_eq!(scene.probe(ids.label, Query::Exact(18)).unwrap(), exact);
    scene.run(Query::Natural, None, false).unwrap();
    let after = scene.geometry(root).unwrap();
    assert_eq!(after.metrics.size.width, 74);
    assert_ne!(before, after);
    let incremental = scene.snapshot();
    scene.run(Query::Natural, None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
}

struct OutputLeaf(LayoutOutput);

impl LayoutNode for OutputLeaf {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(12),
                height: 10,
            },
            baseline: Some(4),
        })
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<LayoutOutput> {
        Ok(self.0)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn equal_measurements_do_not_cover_final_baseline_or_content() {
    let initial = LayoutOutput {
        baseline: Some(4),
        content: Size {
            width: 12,
            height: 10,
        },
    };
    for output in [
        LayoutOutput {
            baseline: Some(8),
            ..initial
        },
        LayoutOutput {
            content: Size {
                width: 120,
                height: 20,
            },
            ..initial
        },
    ] {
        let mut scene = Scene::new(Column { padding: 0, gap: 0 });
        let root = scene.root();
        let child = scene.append(root, OutputLeaf(initial)).unwrap();
        scene.run(Query::Exact(100), None, false).unwrap();
        let before = scene.geometry(root).unwrap();
        let measured = scene.probe(root, Query::Exact(100)).unwrap();
        scene
            .edit_layout::<OutputLeaf>(child, |leaf| leaf.0 = output)
            .unwrap();
        assert_eq!(scene.probe(root, Query::Exact(100)).unwrap(), measured);
        scene.run(Query::Exact(100), None, false).unwrap();
        let after = scene.geometry(root).unwrap();
        assert_eq!(after.metrics.size, before.metrics.size);
        assert_eq!(after.metrics.baseline, output.baseline);
        assert_eq!(after.content, output.content);
        assert_ne!(after, before);
        let incremental = scene.snapshot();
        scene.run(Query::Exact(100), None, true).unwrap();
        assert_eq!(scene.snapshot(), incremental);
    }
}

struct MovingChild {
    x: u32,
}

impl LayoutNode for MovingChild {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(20),
                height: 20,
            },
            baseline: None,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        cx.layout_child(
            0,
            Size {
                width: 5,
                height: 5,
            },
        )?;
        cx.place_child(0, Origin { x: self.x, y: 0 })?;
        Ok(LayoutOutput {
            baseline: None,
            content: assigned,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn stable_boundary_output_does_not_discard_internal_geometry_changes() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let container = scene.append(root, MovingChild { x: 0 }).unwrap();
    let child = scene.append(container, Drawing::new(5, 5)).unwrap();
    scene.run(Query::Exact(100), None, false).unwrap();
    let root_before = scene.geometry(root);
    let boundary_before = scene.geometry(container);
    scene
        .edit_layout::<MovingChild>(container, |node| node.x = 7)
        .unwrap();
    assert_eq!(scene.run(Query::Exact(100), None, false), Ok(1));
    assert_eq!(scene.geometry(root), root_before);
    assert_eq!(scene.geometry(container), boundary_before);
    assert_eq!(scene.geometry(child).unwrap().origin, Origin { x: 7, y: 0 });
    let incremental = scene.snapshot();
    scene.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
}
