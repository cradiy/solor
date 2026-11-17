use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::fixtures::{Column, Drawing, Fault, Ids, Phase, Wrapping, nested};
use super::layout::{
    LayoutContext, LayoutError, LayoutNode, MeasureContext, Metrics, Origin, Query, Result, Scene,
    Size,
};

#[test]
fn known_wrapping_dimensions_baselines_and_sibling_positions() {
    let mut scene = Scene::new(Column { padding: 2, gap: 3 });
    let root = scene.root();
    let text = scene
        .append(
            root,
            Wrapping {
                units: 9,
                advance: 2,
                line_height: 5,
                ascent: 4,
            },
        )
        .unwrap();
    let drawing = scene.append(root, Drawing::new(6, 4)).unwrap();
    assert_eq!(scene.run(12, false), Ok(3));
    let root_geometry = scene.geometry(root).unwrap();
    assert_eq!(
        root_geometry.metrics,
        Metrics {
            size: Size {
                width: 12,
                height: 26
            },
            baseline: Some(6)
        }
    );
    let text_geometry = scene.geometry(text).unwrap();
    assert_eq!(text_geometry.origin, Origin { x: 2, y: 2 });
    assert_eq!(
        text_geometry.metrics.size,
        Size {
            width: 8,
            height: 15
        }
    );
    assert_eq!(
        scene.geometry(drawing).unwrap().origin,
        Origin { x: 2, y: 20 }
    );

    scene
        .edit_layout::<Wrapping>(text, |text| text.units = 17)
        .unwrap();
    scene.run(12, false).unwrap();
    assert_eq!(scene.geometry(root).unwrap().metrics.size.height, 36);
    assert_eq!(scene.geometry(drawing).unwrap().origin.y, 30);
    assert_eq!(scene.node_counts(drawing).layouts, 1);
    assert_eq!(
        scene.append(root, Drawing::new(1, 1)),
        Err(LayoutError::StructureLocked)
    );
}

#[test]
fn text_edit_moves_siblings_without_executing_unrelated_subtree_bodies() {
    let (mut scene, ids) = nested();
    let (mut reference, reference_ids) = nested();
    scene.run(100, false).unwrap();
    let before = scene.counts();
    let right = scene.node_counts(ids.right);
    let drawing = scene.node_counts(ids.drawing);
    let right_origin = scene.geometry(ids.right).unwrap().origin;
    scene
        .edit_layout::<Wrapping>(ids.text, |text| text.units = 100)
        .unwrap();
    reference
        .edit_layout::<Wrapping>(reference_ids.text, |text| text.units = 100)
        .unwrap();
    scene.run(100, false).unwrap();
    reference.run(100, true).unwrap();
    assert_eq!(scene.snapshot(), reference.snapshot());
    assert_eq!(scene.node_counts(ids.right), right);
    assert_eq!(scene.node_counts(ids.drawing), drawing);
    assert!(scene.geometry(ids.right).unwrap().origin.y > right_origin.y);
    assert_eq!(scene.counts().measures - before.measures, 3);
    assert_eq!(scene.counts().layouts - before.layouts, 3);
}

#[test]
fn ink_edits_and_clean_frames_do_no_layout_work() {
    let (mut scene, ids) = nested();
    scene.run(100, false).unwrap();
    let before = scene.counts();
    let geometry = scene.snapshot();
    scene.set_ink(ids.drawing, 0xff00ff).unwrap();
    assert_eq!(scene.run(100, false), Ok(0));
    assert_eq!(scene.run(100, false), Ok(0));
    assert_eq!(scene.counts(), before);
    assert_eq!(scene.snapshot(), geometry);
    assert!(!scene.is_pending());
}

#[test]
fn baseline_only_changes_reach_ancestors_even_when_sizes_match() {
    let (mut scene, ids) = nested();
    scene.run(100, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Wrapping>(ids.text, |text| text.ascent -= 1)
        .unwrap();
    scene.run(100, false).unwrap();
    for (old, new) in before.iter().zip(scene.snapshot()) {
        assert_eq!(old.unwrap().metrics.size, new.unwrap().metrics.size);
        assert_eq!(old.unwrap().origin, new.unwrap().origin);
    }
    for id in [scene.root(), ids.left, ids.text] {
        assert_eq!(scene.node_counts(id).measures, 2);
        assert_eq!(scene.node_counts(id).layouts, 2);
    }
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.baseline,
        Some(12)
    );
}

fn edit(scene: &mut Scene, ids: &Ids, step: u32) {
    match step % 8 {
        0 => scene.edit_layout::<Wrapping>(ids.text, |text| text.units = step * 3),
        1 => scene.edit_layout::<Wrapping>(ids.text, |text| {
            text.advance = 1 + step % 7;
            text.line_height = 2 + step % 11;
            text.ascent = text.line_height - 1;
        }),
        2 => scene.edit_layout::<Drawing>(ids.drawing, |drawing| {
            drawing.size = Size {
                width: 1 + step % 40,
                height: step % 30,
            }
        }),
        3 => scene.edit_layout::<Column>(ids.left, |column| {
            column.padding = step % 4;
            column.gap = step % 6;
        }),
        4 => scene.edit_layout::<Column>(ids.right, |column| column.gap = step % 9),
        5 => scene.set_ink(ids.drawing, step),
        6 => Ok(()),
        _ => scene.edit_layout::<Wrapping>(ids.text, |text| {
            text.ascent = (text.ascent + 1) % (text.line_height + 1)
        }),
    }
    .unwrap();
}

#[test]
fn mixed_edits_match_cache_disabled_recomputation_after_every_step() {
    let (mut scene, ids) = nested();
    let (mut reference, reference_ids) = nested();
    for step in 0..256 {
        edit(&mut scene, &ids, step);
        edit(&mut reference, &reference_ids, step);
        let width = 16 + ((step / 3) * 17) % 200;
        scene.run(width, false).unwrap();
        reference.run(width, true).unwrap();
        assert_eq!(scene.snapshot(), reference.snapshot(), "step {step}");
        assert!(!scene.is_pending());
    }
}

#[test]
fn errors_and_panics_preserve_publication_and_retry_incomplete_work() {
    for phase in [Phase::Measure, Phase::Layout] {
        for fault in [Fault::Error, Fault::Panic] {
            let (mut scene, ids) = nested();
            let (mut reference, reference_ids) = nested();
            scene.run(100, false).unwrap();
            let before = scene.snapshot();
            scene
                .edit_layout::<Wrapping>(ids.text, |text| text.units = 100)
                .unwrap();
            scene
                .edit_layout::<Drawing>(ids.drawing, |drawing| drawing.fault = Some((phase, fault)))
                .unwrap();
            let outcome = catch_unwind(AssertUnwindSafe(|| scene.run(100, false)));
            match fault {
                Fault::Error => assert_eq!(outcome.unwrap(), Err(LayoutError::Injected)),
                Fault::Panic => assert!(outcome.is_err()),
            }
            assert_eq!(scene.snapshot(), before);
            assert!(scene.is_pending());
            // Remove only the fault injector: retry must rely on retained invalidity.
            scene.body_mut::<Drawing>(ids.drawing).unwrap().fault = None;
            scene.run(100, false).unwrap();
            reference
                .edit_layout::<Wrapping>(reference_ids.text, |text| text.units = 100)
                .unwrap();
            reference.run(100, true).unwrap();
            assert_eq!(scene.snapshot(), reference.snapshot());
            assert!(!scene.is_pending());
        }
    }
}

#[test]
fn interrupted_input_edit_invalidates_before_the_callback_runs() {
    let (mut scene, ids) = nested();
    let (mut reference, reference_ids) = nested();
    scene.run(100, false).unwrap();
    let before = scene.snapshot();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            scene.edit_layout::<Wrapping>(ids.text, |text| {
                text.units = 100;
                panic!("intentional edit panic");
            })
        }))
        .is_err()
    );
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene.run(100, false).unwrap();
    reference
        .edit_layout::<Wrapping>(reference_ids.text, |text| text.units = 100)
        .unwrap();
    reference.run(100, true).unwrap();
    assert_eq!(scene.snapshot(), reference.snapshot());
}

#[test]
fn later_branch_failure_does_not_complete_an_unpublished_subtree() {
    fn fixture() -> (Scene, solor_core::tree::NodeId, solor_core::tree::NodeId) {
        let mut scene = Scene::new(Column { padding: 2, gap: 3 });
        let root = scene.root();
        let branch = scene.append(root, Column { padding: 1, gap: 1 }).unwrap();
        let text = scene
            .append(
                branch,
                Wrapping {
                    units: 8,
                    advance: 5,
                    line_height: 10,
                    ascent: 7,
                },
            )
            .unwrap();
        let last = scene.append(root, Drawing::new(10, 4)).unwrap();
        (scene, text, last)
    }

    let (mut scene, text, last) = fixture();
    let (mut reference, reference_text, _) = fixture();
    scene.run(100, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Wrapping>(text, |text| text.units = 100)
        .unwrap();
    scene
        .edit_layout::<Drawing>(last, |drawing| {
            drawing.fault = Some((Phase::Layout, Fault::Error))
        })
        .unwrap();
    assert_eq!(scene.run(100, false), Err(LayoutError::Injected));
    assert_eq!(scene.snapshot(), before);
    scene.body_mut::<Drawing>(last).unwrap().fault = None;
    scene.run(100, false).unwrap();
    reference
        .edit_layout::<Wrapping>(reference_text, |text| text.units = 100)
        .unwrap();
    reference.run(100, true).unwrap();
    assert_eq!(scene.snapshot(), reference.snapshot());
}

#[test]
fn invalid_queries_and_overflow_preserve_geometry_and_allow_retry() {
    let (mut scene, ids) = nested();
    scene.run(100, false).unwrap();
    let before = scene.snapshot();
    assert_eq!(scene.run(0, false), Err(LayoutError::InvalidSize));
    assert_eq!(scene.run(4, false), Err(LayoutError::InvalidSize));
    assert_eq!(scene.snapshot(), before);
    scene.run(100, false).unwrap();
    assert_eq!(scene.snapshot(), before);

    scene
        .edit_layout::<Wrapping>(ids.text, |text| {
            text.units = u32::MAX;
            text.line_height = u32::MAX;
        })
        .unwrap();
    assert_eq!(scene.run(100, false), Err(LayoutError::InvalidSize));
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene
        .edit_layout::<Wrapping>(ids.text, |text| {
            text.units = 8;
            text.line_height = 10;
        })
        .unwrap();
    scene.run(100, false).unwrap();
    assert_eq!(scene.snapshot(), before);
}

#[derive(Clone, Copy)]
enum Placement {
    Normal,
    Duplicate,
    Missing,
    InvalidMetrics,
}

struct InvalidParent(Placement);

impl LayoutNode for InvalidParent {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        cx.measure_child(0, query)
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<()> {
        if matches!(self.0, Placement::Missing) {
            return Ok(());
        }
        let mut metrics = cx.measure_child(0, Query::new(assigned.width)?)?;
        if matches!(self.0, Placement::InvalidMetrics) {
            metrics.baseline = Some(metrics.size.height + 1);
        }
        cx.place_child(0, metrics, Origin::default())?;
        if matches!(self.0, Placement::Duplicate) {
            cx.place_child(0, metrics, Origin::default())?;
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn invalid_child_placement_cannot_publish_a_partial_first_frame() {
    for (mode, error) in [
        (Placement::Duplicate, LayoutError::DuplicatePlacement(0)),
        (Placement::Missing, LayoutError::UnplacedChildren),
        (Placement::InvalidMetrics, LayoutError::InvalidSize),
    ] {
        let mut scene = Scene::new(InvalidParent(mode));
        let root = scene.root();
        scene.append(root, Drawing::new(5, 5)).unwrap();
        assert_eq!(scene.run(10, false), Err(error));
        assert_eq!(scene.snapshot(), vec![None, None]);
        assert!(scene.is_pending());
        scene.body_mut::<InvalidParent>(root).unwrap().0 = Placement::Normal;
        assert_eq!(scene.run(10, false), Ok(2));
        assert!(scene.snapshot().iter().all(Option::is_some));
    }
}

struct CatchingParent(Phase);

impl LayoutNode for CatchingParent {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        if self.0 == Phase::Measure {
            let _ = catch_unwind(AssertUnwindSafe(|| cx.measure_child(0, query)));
        }
        Ok(Metrics {
            size: Size {
                width: query.width(),
                height: 5,
            },
            baseline: None,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<()> {
        let child = cx.measure_child(0, Query::new(assigned.width)?)?;
        let _ = catch_unwind(AssertUnwindSafe(|| {
            cx.place_child(0, child, Origin::default())
        }));
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn caught_child_errors_and_panics_still_abort_the_attempt() {
    for phase in [Phase::Measure, Phase::Layout] {
        for fault in [Fault::Error, Fault::Panic] {
            let mut scene = Scene::new(CatchingParent(phase));
            let root = scene.root();
            let mut drawing = Drawing::new(5, 5);
            drawing.fault = Some((phase, fault));
            let child = scene.append(root, drawing).unwrap();
            assert_eq!(scene.run(10, false), Err(LayoutError::FailedChildOperation));
            assert_eq!(scene.snapshot(), vec![None, None]);
            assert!(scene.is_pending());
            scene.body_mut::<Drawing>(child).unwrap().fault = None;
            assert_eq!(scene.run(10, false), Ok(2));
            assert!(!scene.is_pending());
        }
    }
}
