use super::*;
use crate::layout::validation::SelectiveCheckpoint;

fn checkpoints(scene: &mut Scene, assigned: Size) -> [SelectiveCheckpoint; 2] {
    let root = scene.root();
    [
        scene
            .capture_selective_dependencies(root, CheckTarget::Measure(Query::Exact(100)))
            .unwrap()
            .unwrap(),
        scene
            .capture_selective_dependencies(root, CheckTarget::Layout(assigned))
            .unwrap()
            .unwrap(),
    ]
}

fn moving_fixture() -> (Scene, [solor_core::tree::NodeId; 3]) {
    let mut scene = Scene::new(Column { padding: 3, gap: 4 });
    let root = scene.root();
    let prefix = scene.append(root, Drawing::new(5, 7)).unwrap();
    let parent = scene.append(root, MovingChild { x: 0 }).unwrap();
    let child = scene.append(parent, Drawing::new(5, 5)).unwrap();
    (scene, [prefix, parent, child])
}

struct OverflowLayout;

impl LayoutNode for OverflowLayout {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Column { padding: 0, gap: 0 }.measure(cx, query)
    }
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        for _ in 0..=MAX_RECORDED_READS {
            cx.measure_child(0, Query::Natural)?;
        }
        Column { padding: 0, gap: 0 }.layout(cx, assigned)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn incomplete_layout_evidence_cannot_skip_when_measurement_dependencies_are_equal() {
    let mut scene = Scene::new(OverflowLayout);
    let root = scene.root();
    let child = scene.append(root, Drawing::new(5, 5)).unwrap();
    let assigned = Size {
        width: 100,
        height: 10,
    };
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
    let before = scene.node_counts(root).layouts;
    scene.edit_layout::<Drawing>(child, |_| {}).unwrap();
    let report = scene
        .run_pruned(
            Query::Exact(100),
            assigned,
            &mut measurement,
            &mut placement,
        )
        .unwrap();
    assert_eq!(report.checks[0].outcome, CheckOutcome::Unchanged);
    assert_eq!(report.checks[1].outcome, CheckOutcome::Unverifiable);
    assert!(!report.skipped_parent_layout);
    assert_eq!(scene.node_counts(root).layouts, before + 1);
    let snapshot = scene.snapshot();
    scene.run(Query::Exact(100), Some(assigned), true).unwrap();
    assert_eq!(scene.snapshot(), snapshot);
}

#[test]
fn skipping_parent_preserves_nonzero_placement_and_new_internal_movement() {
    let (mut scene, [_, parent, child]) = moving_fixture();
    let (mut reference, [_, reference_parent, _]) = moving_fixture();
    let assigned = Size {
        width: 100,
        height: 80,
    };
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    reference
        .run(Query::Exact(100), Some(assigned), true)
        .unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
    let root = scene.root();
    let root_geometry = scene.geometry(root);
    let parent_geometry = scene.geometry(parent);
    assert_eq!(parent_geometry.unwrap().origin, Origin { x: 3, y: 14 });
    for x in [7, 9, 2] {
        let before = scene.node_counts(root);
        scene
            .edit_layout::<MovingChild>(parent, |node| node.x = x)
            .unwrap();
        reference
            .edit_layout::<MovingChild>(reference_parent, |node| node.x = x)
            .unwrap();
        let report = scene
            .run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement,
            )
            .unwrap();
        assert!(report.skipped_parent_layout);
        assert_eq!(scene.node_counts(root).layouts, before.layouts);
        assert_eq!(scene.node_counts(root).measures, before.measures + 1);
        assert_eq!(scene.geometry(root), root_geometry);
        assert_eq!(scene.geometry(parent), parent_geometry);
        assert_eq!(scene.geometry(child).unwrap().origin.x, x);
        assert_eq!(report.published, 1);
        reference
            .run(Query::Exact(100), Some(assigned), true)
            .unwrap();
        assert_eq!(scene.snapshot(), reference.snapshot());
        assert!(scene.staged_usage().iter().all(|&(len, _)| len == 0));
        scene.release_scratch();
        assert_eq!(scene.staged_usage(), [(0, 0); 2]);
    }
}

#[test]
fn standalone_audit_cannot_hide_unpublished_movement_from_a_later_pruned_run() {
    let (mut scene, [prefix, parent, child]) = moving_fixture();
    let assigned = Size {
        width: 100,
        height: 80,
    };
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
    scene
        .edit_layout::<MovingChild>(parent, |node| node.x = 9)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut placement, CheckTarget::Layout(assigned))
            .unwrap()
            .outcome,
        CheckOutcome::Unchanged
    );
    assert_eq!(scene.geometry(child).unwrap().origin.x, 0);
    // Select a different subtree after the audit advanced its cursor.
    scene.edit_layout::<Drawing>(prefix, |_| {}).unwrap();
    let report = scene
        .run_pruned(
            Query::Exact(100),
            assigned,
            &mut measurement,
            &mut placement,
        )
        .unwrap();
    assert_eq!(report.checks[1].outcome, CheckOutcome::Unchanged);
    assert!(!report.skipped_parent_layout);
    assert_eq!(scene.geometry(child).unwrap().origin.x, 9);
    let snapshot = scene.snapshot();
    scene.run(Query::Exact(100), Some(assigned), true).unwrap();
    assert_eq!(scene.snapshot(), snapshot);
}

#[test]
fn historical_equality_cannot_reuse_a_different_published_parent() {
    let (mut scene, [prefix, parent, child]) = moving_fixture();
    let assigned = Size {
        width: 100,
        height: 80,
    };
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
    let root = scene.root();
    let original = scene.geometry(root);
    scene
        .edit_layout::<Drawing>(prefix, |node| node.size.height = 17)
        .unwrap();
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    assert_eq!(scene.geometry(parent).unwrap().origin.y, 24);
    scene
        .edit_layout::<Drawing>(prefix, |node| node.size.height = 7)
        .unwrap();
    scene
        .edit_layout::<MovingChild>(parent, |node| node.x = 9)
        .unwrap();
    let before = scene.node_counts(root).layouts;
    let report = scene
        .run_pruned(
            Query::Exact(100),
            assigned,
            &mut measurement,
            &mut placement,
        )
        .unwrap();
    assert_eq!(
        report.checks.map(|check| check.outcome),
        [CheckOutcome::Unchanged; 2]
    );
    assert!(!report.skipped_parent_layout);
    assert_eq!(scene.node_counts(root).layouts, before + 1);
    assert_eq!(scene.geometry(root), original);
    assert_eq!(scene.geometry(parent).unwrap().origin.y, 14);
    assert_eq!(scene.geometry(child).unwrap().origin.x, 9);
    // Successful reconciliation anchors the still-equal evidence to this publication.
    scene
        .edit_layout::<MovingChild>(parent, |node| node.x = 2)
        .unwrap();
    assert!(
        scene
            .run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement
            )
            .unwrap()
            .skipped_parent_layout
    );
    let snapshot = scene.snapshot();
    scene.run(Query::Exact(100), Some(assigned), true).unwrap();
    assert_eq!(scene.snapshot(), snapshot);
}

#[test]
fn changed_final_outputs_and_owner_inputs_execute_parent_layout() {
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
        let assigned = Size {
            width: 100,
            height: 30,
        };
        scene.run(Query::Exact(100), Some(assigned), false).unwrap();
        let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
        scene
            .edit_layout::<OutputLeaf>(child, |node| node.0 = output)
            .unwrap();
        let report = scene
            .run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement,
            )
            .unwrap();
        assert!(!report.skipped_parent_layout);
        assert_eq!(report.checks[1].outcome, CheckOutcome::Changed);
        assert_eq!(scene.geometry(root).unwrap().content, output.content);
        assert_eq!(
            scene.geometry(root).unwrap().metrics.baseline,
            output.baseline
        );
        scene
            .refresh_selective_dependencies(
                &mut measurement,
                CheckTarget::Measure(Query::Exact(100)),
            )
            .unwrap();
        scene
            .refresh_selective_dependencies(&mut placement, CheckTarget::Layout(assigned))
            .unwrap();
        scene
            .edit_layout::<Column>(root, |node| node.padding = 2)
            .unwrap();
        let report = scene
            .run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement,
            )
            .unwrap();
        assert_eq!(report.checks[1].outcome, CheckOutcome::Unverifiable);
        assert!(!report.skipped_parent_layout);
        let snapshot = scene.snapshot();
        scene.run(Query::Exact(100), Some(assigned), true).unwrap();
        assert_eq!(scene.snapshot(), snapshot);
    }
}

#[test]
fn failure_after_equal_audits_does_not_commit_parent_or_cursors() {
    for fault in [Fault::Error, Fault::Panic] {
        let (mut scene, children) = reading_fixture();
        let root = scene.root();
        let assigned = Size {
            width: 100,
            height: 5,
        };
        scene.run(Query::Exact(100), Some(assigned), false).unwrap();
        let [mut measurement, mut placement] = checkpoints(&mut scene, assigned);
        let snapshot = scene.snapshot();
        let reads = scene.published_layout_reads(root);
        let before = scene.node_counts(root).layouts;
        scene.edit_layout::<Drawing>(children[0], |_| {}).unwrap();
        scene.body_mut::<ReadingParent>(root).unwrap().fault = Some(fault);
        let result = catch_unwind(AssertUnwindSafe(|| {
            scene.run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement,
            )
        }));
        match fault {
            Fault::Error => assert!(matches!(result.unwrap(), Err(LayoutError::Injected))),
            Fault::Panic => assert!(result.is_err()),
        }
        assert_eq!(scene.snapshot(), snapshot);
        assert_eq!(scene.published_layout_reads(root), reads);
        assert!(scene.is_pending());
        assert!(scene.staged_usage().iter().all(|&(len, _)| len == 0));
        scene.body_mut::<ReadingParent>(root).unwrap().fault = None;
        let retry = scene
            .run_pruned(
                Query::Exact(100),
                assigned,
                &mut measurement,
                &mut placement,
            )
            .unwrap();
        assert!(retry.skipped_parent_layout);
        assert_eq!(retry.checks[0].checked_reads, 1);
        assert_eq!(retry.checks[1].checked_reads, 2);
        assert_eq!(scene.node_counts(root).layouts, before);
        assert_eq!(scene.snapshot(), snapshot);
        assert!(!scene.is_pending());
    }
}
