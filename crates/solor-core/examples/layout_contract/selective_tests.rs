use super::*;
use crate::layout::validation::MAX_TRACKED_EDITS;

#[test]
fn refreshing_equal_child_order_replaces_results_and_request() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let child = scene.append(root, Drawing::new(20, 5)).unwrap();
    scene.run(Query::Exact(10), None, false).unwrap();
    let old = CheckTarget::Layout(Size {
        width: 10,
        height: 5,
    });
    let mut checkpoint = scene
        .capture_selective_dependencies(root, old)
        .unwrap()
        .unwrap();
    scene
        .edit_layout::<Drawing>(child, |leaf| leaf.size.width = 21)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, old)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    scene.run(Query::Exact(30), None, false).unwrap();
    let current = CheckTarget::Layout(Size {
        width: 30,
        height: 5,
    });
    assert!(
        scene
            .refresh_selective_dependencies(&mut checkpoint, current)
            .unwrap()
    );
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, old)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
    scene.edit_layout::<Drawing>(child, |_| {}).unwrap();
    let report = scene
        .check_selective_dependencies(&mut checkpoint, current)
        .unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unchanged);
    assert_eq!(report.checked_reads, 2);
    // Returning to the original value differs from the refreshed comparison value.
    scene
        .edit_layout::<Drawing>(child, |leaf| leaf.size.width = 20)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, current)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    scene.run(Query::Exact(30), None, false).unwrap();
    let incremental = scene.snapshot();
    scene.run(Query::Exact(30), None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
}

#[test]
fn refresh_rebuilds_conditional_reads_and_distinguishes_incomplete_from_empty() {
    let (mut scene, children) = reading_fixture();
    let root = scene.root();
    let target = CheckTarget::Measure(Query::Natural);
    scene.run(Query::Natural, None, false).unwrap();
    let mut checkpoint = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    for repeats in [1, 5, 0, MAX_RECORDED_READS + 1, 1] {
        scene
            .edit_layout::<ReadingParent>(root, |parent| {
                parent.selected = 1;
                parent.repeats = repeats;
            })
            .unwrap();
        assert!(
            !scene
                .refresh_selective_dependencies(&mut checkpoint, target)
                .unwrap()
        );
        scene.run(Query::Natural, None, false).unwrap();
        assert!(
            scene
                .refresh_selective_dependencies(&mut checkpoint, target)
                .unwrap()
        );
        scene
            .edit_layout::<Drawing>(children[0], |leaf| leaf.size.width += 1)
            .unwrap();
        let report = scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap();
        assert_eq!(report.checked_reads, 0);
        assert_eq!(
            report.outcome,
            if repeats > MAX_RECORDED_READS {
                CheckOutcome::Unverifiable
            } else {
                CheckOutcome::Unchanged
            }
        );
        scene.edit_layout::<Drawing>(children[1], |_| {}).unwrap();
        let report = scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap();
        assert_eq!(
            report.checked_reads,
            if repeats > MAX_RECORDED_READS {
                0
            } else {
                repeats
            }
        );
    }
    scene
        .edit_layout::<Drawing>(children[1], |leaf| leaf.size.width += 1)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
}

#[test]
fn rejected_refresh_preserves_evidence_and_cursor_through_failure() {
    for fault in [Fault::Error, Fault::Panic] {
        let (mut scene, children) = reading_fixture();
        let root = scene.root();
        scene.run(Query::Natural, None, false).unwrap();
        let target = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
        let mut checkpoint = scene
            .capture_selective_dependencies(root, target)
            .unwrap()
            .unwrap();
        // A request not represented by published geometry cannot replace evidence.
        assert!(
            !scene
                .refresh_selective_dependencies(
                    &mut checkpoint,
                    CheckTarget::Layout(Size {
                        width: 90,
                        height: 90
                    })
                )
                .unwrap()
        );
        let mut foreign = Scene::new(Drawing::new(5, 5));
        foreign.run(Query::Natural, None, false).unwrap();
        assert!(
            foreign
                .refresh_selective_dependencies(&mut checkpoint, target)
                .is_err()
        );
        scene
            .edit_layout::<Drawing>(children[1], |leaf| {
                leaf.fault = Some((Phase::Layout, fault))
            })
            .unwrap();
        let original = scene.snapshot();
        let result = catch_unwind(AssertUnwindSafe(|| scene.run(Query::Natural, None, false)));
        match fault {
            Fault::Error => assert!(result.unwrap().is_err()),
            Fault::Panic => assert!(result.is_err()),
        }
        assert!(
            !scene
                .refresh_selective_dependencies(&mut checkpoint, target)
                .unwrap()
        );
        assert_eq!(scene.snapshot(), original);
        scene.body_mut::<Drawing>(children[1]).unwrap().fault = None;
        let report = scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap();
        assert_eq!(report.outcome, CheckOutcome::Unchanged);
        assert_eq!(report.checked_reads, 1);
        scene.run(Query::Natural, None, false).unwrap();
        assert!(
            scene
                .refresh_selective_dependencies(&mut checkpoint, target)
                .unwrap()
        );
        assert_eq!(scene.snapshot(), original);
    }
}

#[test]
fn indexed_reads_keep_query_identity_after_cache_eviction() {
    let (mut scene, ids) = fixture();
    let root = scene.root();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let mut checkpoint = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    let exact = scene.probe(ids.label, Query::Exact(18)).unwrap();
    for width in [19, 20, 21, 22] {
        scene.probe(root, Query::Exact(width)).unwrap();
        scene.probe(ids.label, Query::Exact(width)).unwrap();
    }
    scene
        .edit_layout::<Wrapping>(ids.label, |label| label.units = 21)
        .unwrap();
    assert_eq!(scene.probe(ids.label, Query::Exact(18)).unwrap(), exact);
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    scene.run(Query::Natural, None, false).unwrap();
    assert_eq!(scene.geometry(root).unwrap().metrics.size.width, 74);
}

#[test]
fn sparse_checks_keep_internal_movement_and_deduplicate_changed_branches() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let mut children = Vec::new();
    for _ in 0..100 {
        let parent = scene.append(root, MovingChild { x: 0 }).unwrap();
        let leaf = scene.append(parent, Drawing::new(5, 5)).unwrap();
        children.push((parent, leaf));
    }
    scene.run(Query::Exact(100), None, false).unwrap();
    let target = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
    let mut checkpoint = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    let original = scene.snapshot();
    let reads = scene.published_layout_reads(root);
    let (parent, leaf) = children[83];
    scene
        .edit_layout::<MovingChild>(parent, |node| node.x = 7)
        .unwrap();
    scene.edit_layout::<Drawing>(leaf, |_| {}).unwrap();
    let unrelated = scene.node_counts(children[0].0).unwrap();
    let report = scene
        .check_selective_dependencies(&mut checkpoint, target)
        .unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unchanged);
    assert_eq!(report.checked_reads, 2);
    assert_eq!(report.staged_geometry, 2);
    assert_eq!(scene.node_counts(children[0].0).unwrap(), unrelated);
    assert_eq!(scene.snapshot(), original);
    assert_eq!(scene.published_layout_reads(root), reads);
    // Advancing the audit cursor must not clear the pending internal movement.
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .checked_reads,
        0
    );
    assert!(scene.is_pending());
    scene.run(Query::Exact(100), None, false).unwrap();
    assert_eq!(scene.geometry(leaf).unwrap().origin.x, 7);
    let incremental = scene.snapshot();
    scene.run(Query::Exact(100), None, true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
}

#[test]
fn unread_branches_and_other_subtrees_are_ignored_but_owner_edits_are_not() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let owner = scene
        .append(
            root,
            ReadingParent {
                selected: 0,
                repeats: 1,
                fault: None,
            },
        )
        .unwrap();
    let a = scene.append(owner, Drawing::new(5, 5)).unwrap();
    let b = scene.append(owner, Drawing::new(7, 5)).unwrap();
    let outside = scene.append(root, Drawing::new(9, 5)).unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let mut checkpoint = scene
        .capture_selective_dependencies(owner, target)
        .unwrap()
        .unwrap();
    scene
        .edit_layout::<Drawing>(outside, |leaf| leaf.size.width = 11)
        .unwrap();
    scene
        .edit_layout::<Drawing>(b, |leaf| leaf.size.width = 12)
        .unwrap();
    let report = scene
        .check_selective_dependencies(&mut checkpoint, target)
        .unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unchanged);
    assert_eq!(report.checked_reads, 0);
    scene
        .edit_layout::<Drawing>(a, |leaf| leaf.size.width = 8)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    // Changed does not advance the cursor or replace the old consumed result.
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
    scene
        .edit_layout::<ReadingParent>(owner, |parent| parent.selected = 1)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut checkpoint, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
}

#[test]
fn history_is_bounded_and_equality_can_advance_without_recapturing() {
    let (mut scene, children) = reading_fixture();
    let root = scene.root();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let mut stale = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    let mut active = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    for _ in 0..3 {
        for _ in 0..MAX_TRACKED_EDITS {
            scene.edit_layout::<Drawing>(children[0], |_| {}).unwrap();
        }
        let report = scene
            .check_selective_dependencies(&mut active, target)
            .unwrap();
        assert_eq!(report.outcome, CheckOutcome::Unchanged);
        assert_eq!(report.checked_reads, 1);
        scene.run(Query::Natural, None, false).unwrap();
    }
    let before = scene.counts();
    let report = scene
        .check_selective_dependencies(&mut stale, target)
        .unwrap();
    assert_eq!(report.outcome, CheckOutcome::Unverifiable);
    assert_eq!(scene.counts(), before);
    // Reclaiming layout scratch cannot erase edit history or an active cursor.
    scene.release_scratch();
    scene
        .edit_layout::<Drawing>(children[0], |leaf| leaf.size.width = 6)
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut active, target)
            .unwrap()
            .outcome,
        CheckOutcome::Changed
    );
}

#[test]
fn selective_failures_keep_cursor_and_publication_for_retry() {
    for phase in [Phase::Measure, Phase::Layout] {
        for fault in [Fault::Error, Fault::Panic] {
            let mut scene = Scene::new(Column { padding: 0, gap: 0 });
            let root = scene.root();
            let a = scene.append(root, Drawing::new(5, 5)).unwrap();
            let b = scene.append(root, Drawing::new(5, 5)).unwrap();
            scene.run(Query::Natural, None, false).unwrap();
            let target = CheckTarget::Layout(scene.geometry(root).unwrap().metrics.size);
            let mut checkpoint = scene
                .capture_selective_dependencies(root, target)
                .unwrap()
                .unwrap();
            let before = scene.snapshot();
            let reads = scene.published_layout_reads(root);
            // Reverse edit order must not reverse replay order.
            scene
                .edit_layout::<Drawing>(b, |leaf| leaf.fault = Some((phase, fault)))
                .unwrap();
            scene.edit_layout::<Drawing>(a, |_| {}).unwrap();
            let layouts = scene.node_counts(a).unwrap().layouts;
            let result = catch_unwind(AssertUnwindSafe(|| {
                scene.check_selective_dependencies(&mut checkpoint, target)
            }));
            match fault {
                Fault::Error => assert_eq!(result.unwrap(), Err(LayoutError::Node("injected"))),
                Fault::Panic => assert!(result.is_err()),
            }
            assert_eq!(scene.node_counts(a).unwrap().layouts, layouts + 1);
            assert_eq!(scene.snapshot(), before);
            assert_eq!(scene.published_layout_reads(root), reads);
            assert!(scene.scratch_usage().iter().all(|&(len, _)| len == 0));
            scene.body_mut::<Drawing>(b).unwrap().fault = None;
            let report = scene
                .check_selective_dependencies(&mut checkpoint, target)
                .unwrap();
            assert_eq!(report.outcome, CheckOutcome::Unchanged);
            assert_eq!(report.checked_reads, 4);
            scene.run(Query::Natural, None, false).unwrap();
            assert_eq!(scene.snapshot(), before);
            assert!(!scene.is_pending());
        }
    }
}

#[test]
fn selective_gates_reject_incomplete_foreign_and_mismatched_evidence() {
    let (mut scene, children) = reading_fixture();
    let root = scene.root();
    scene
        .edit_layout::<ReadingParent>(root, |parent| parent.repeats = MAX_RECORDED_READS + 1)
        .unwrap();
    scene.run(Query::Natural, None, false).unwrap();
    let target = CheckTarget::Measure(Query::Natural);
    let mut incomplete = scene
        .capture_selective_dependencies(root, target)
        .unwrap()
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut incomplete, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
    let mut leaf = scene
        .capture_selective_dependencies(children[0], target)
        .unwrap()
        .unwrap();
    assert_eq!(
        scene
            .check_selective_dependencies(&mut leaf, CheckTarget::Measure(Query::Exact(5)))
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
    let mut foreign = Scene::new(Drawing::new(5, 5));
    assert!(
        foreign
            .check_selective_dependencies(&mut leaf, target)
            .is_err()
    );
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            scene
                .edit_layout::<Drawing>(children[0], |node| {
                    node.size.width = 8;
                    panic!("interrupted tracked mutation");
                })
                .unwrap();
        }))
        .is_err()
    );
    assert_eq!(
        scene
            .check_selective_dependencies(&mut leaf, target)
            .unwrap()
            .outcome,
        CheckOutcome::Unverifiable
    );
}
