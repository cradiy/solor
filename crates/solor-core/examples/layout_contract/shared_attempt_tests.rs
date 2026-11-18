use super::*;
use crate::layout::validation::{MAX_TRACKED_EDITS, SelectiveCheckpoint};

fn checkpoints(scene: &mut Scene, query: Query, assigned: Size) -> [SelectiveCheckpoint; 2] {
    let root = scene.root();
    [
        scene
            .capture_selective_dependencies(root, CheckTarget::Measure(query))
            .unwrap()
            .unwrap(),
        scene
            .capture_selective_dependencies(root, CheckTarget::Layout(assigned))
            .unwrap()
            .unwrap(),
    ]
}

#[test]
fn shared_attempt_publishes_internal_movement_without_repeating_subtree_layout() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let container = scene.append(root, MovingChild { x: 0 }).unwrap();
    let child = scene.append(container, Drawing::new(5, 5)).unwrap();
    let query = Query::Exact(100);
    let assigned = Size {
        width: 100,
        height: 20,
    };
    scene.run(query, Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, query, assigned);
    for x in [7, 9, 2] {
        let before = scene.node_counts(container).unwrap();
        scene
            .edit_layout::<MovingChild>(container, |node| node.x = x)
            .unwrap();
        let report = scene
            .run_validated(query, assigned, &mut measurement, &mut placement)
            .unwrap();
        assert_eq!(
            report.checks.map(|check| check.outcome),
            [CheckOutcome::Unchanged; 2]
        );
        assert_eq!(report.published, 1);
        assert_eq!(report.reused_layouts, 1);
        assert_eq!(
            scene.node_counts(container).unwrap().layouts,
            before.layouts + 1
        );
        assert_eq!(scene.geometry(child).unwrap().origin.x, x);
        assert!(scene.scratch_usage().iter().all(|&(len, _)| len == 0));
        assert!(scene.staged_usage().iter().all(|&(len, _)| len == 0));
        assert!(!scene.is_pending());
        let snapshot = scene.snapshot();
        let counts = scene.counts();
        let clean = scene
            .run_validated(query, assigned, &mut measurement, &mut placement)
            .unwrap();
        assert_eq!(clean.published, 0);
        assert_eq!(scene.counts(), counts);
        scene.release_scratch();
        assert_eq!(scene.staged_usage(), [(0, 0); 2]);
        scene.run(query, Some(assigned), true).unwrap();
        assert_eq!(scene.snapshot(), snapshot);
    }
}

#[test]
fn errors_and_panics_in_audit_or_after_adoption_preserve_both_cursors() {
    for stop_audit_early in [false, true] {
        for fault in [Fault::Error, Fault::Panic] {
            let mut scene = Scene::new(Column { padding: 0, gap: 0 });
            let root = scene.root();
            let output = LayoutOutput {
                baseline: None,
                content: Size {
                    width: 12,
                    height: 10,
                },
            };
            let a = scene.append(root, OutputLeaf(output)).unwrap();
            let b = scene.append(root, Drawing::new(5, 5)).unwrap();
            let query = Query::Exact(100);
            let assigned = Size {
                width: 100,
                height: 15,
            };
            scene.run(query, Some(assigned), false).unwrap();
            let [mut measurement, mut placement] = checkpoints(&mut scene, query, assigned);
            let snapshot = scene.snapshot();
            let root_reads = scene.published_layout_reads(root);
            let child_reads = scene.published_layout_reads(a);
            scene
                .edit_layout::<OutputLeaf>(a, |leaf| {
                    if stop_audit_early {
                        leaf.0.content.width = 120;
                    }
                })
                .unwrap();
            scene
                .edit_layout::<Drawing>(b, |leaf| leaf.fault = Some((Phase::Layout, fault)))
                .unwrap();
            let before = scene.node_counts(a).unwrap().layouts;
            let result = catch_unwind(AssertUnwindSafe(|| {
                scene.run_validated(query, assigned, &mut measurement, &mut placement)
            }));
            match fault {
                Fault::Error => assert!(matches!(
                    result.unwrap(),
                    Err(LayoutError::Node("injected"))
                )),
                Fault::Panic => assert!(result.is_err()),
            }
            assert_eq!(scene.node_counts(a).unwrap().layouts, before + 1);
            assert_eq!(scene.snapshot(), snapshot);
            assert_eq!(scene.published_layout_reads(root), root_reads);
            assert_eq!(scene.published_layout_reads(a), child_reads);
            assert!(scene.scratch_usage().iter().all(|&(len, _)| len == 0));
            assert!(scene.staged_usage().iter().all(|&(len, _)| len == 0));
            assert!(scene.is_pending());
            // No new edit notification: failed attempts must not consume either cursor.
            scene.body_mut::<Drawing>(b).unwrap().fault = None;
            let retry = scene
                .run_validated(query, assigned, &mut measurement, &mut placement)
                .unwrap();
            assert_eq!(retry.checks[0].checked_reads, 2);
            assert!(retry.checks[1].checked_reads >= 2);
            assert_eq!(scene.node_counts(a).unwrap().layouts, before + 2);
            assert!(retry.reused_layouts >= 1);
            let incremental = scene.snapshot();
            scene.run(query, Some(assigned), true).unwrap();
            assert_eq!(scene.snapshot(), incremental);
        }
    }
}

#[test]
fn missing_history_and_new_requests_complete_normal_layout_without_stale_adoption() {
    let (mut scene, children) = reading_fixture();
    let query = Query::Natural;
    let assigned = Size {
        width: 5,
        height: 5,
    };
    scene.run(query, Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, query, assigned);
    for _ in 0..=MAX_TRACKED_EDITS {
        scene
            .edit_layout::<Drawing>(children[0], |leaf| leaf.size.width += 1)
            .unwrap();
    }
    let report = scene
        .run_validated(query, assigned, &mut measurement, &mut placement)
        .unwrap();
    assert_eq!(
        report.checks.map(|check| check.outcome),
        [CheckOutcome::Unverifiable; 2]
    );
    assert_eq!(report.reused_layouts, 0);
    let snapshot = scene.snapshot();
    scene.run(query, Some(assigned), true).unwrap();
    assert_eq!(scene.snapshot(), snapshot);
    assert!(
        scene
            .refresh_selective_dependencies(&mut measurement, CheckTarget::Measure(query))
            .unwrap()
    );
    assert!(
        scene
            .refresh_selective_dependencies(&mut placement, CheckTarget::Layout(assigned))
            .unwrap()
    );
    let resized = Size {
        width: 20,
        height: 30,
    };
    let report = scene
        .run_validated(Query::Exact(20), resized, &mut measurement, &mut placement)
        .unwrap();
    assert_eq!(
        report.checks.map(|check| check.outcome),
        [CheckOutcome::Unverifiable; 2]
    );
    assert_eq!(report.reused_layouts, 0);
    let snapshot = scene.snapshot();
    scene.run(Query::Exact(20), Some(resized), true).unwrap();
    assert_eq!(scene.snapshot(), snapshot);
}

#[test]
fn shared_attempt_rejects_non_root_and_foreign_checkpoints_before_work() {
    let (mut scene, children) = reading_fixture();
    let assigned = Size {
        width: 5,
        height: 5,
    };
    scene.run(Query::Natural, Some(assigned), false).unwrap();
    let [mut measurement, mut placement] = checkpoints(&mut scene, Query::Natural, assigned);
    let mut child = scene
        .capture_selective_dependencies(children[0], CheckTarget::Measure(Query::Natural))
        .unwrap()
        .unwrap();
    let before = scene.counts();
    assert!(matches!(
        scene.run_validated(Query::Natural, assigned, &mut child, &mut placement),
        Err(LayoutError::InvalidCheckpointOwner)
    ));
    let mut foreign = Scene::new(Drawing::new(5, 5));
    assert!(matches!(
        foreign.run_validated(Query::Natural, assigned, &mut measurement, &mut placement),
        Err(LayoutError::InvalidCheckpointOwner)
    ));
    assert_eq!(scene.counts(), before);
    assert!(!scene.is_pending());
}
