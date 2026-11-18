use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::frame::{Fault, Mode, Presentation};
use crate::layout::{LayoutError, Size};

const SIZE: Size = Size {
    width: 160,
    height: 200,
};

#[test]
fn same_size_text_change_publishes_new_glyphs_while_parent_is_skipped() {
    let mut view = Presentation::new(2, "中文", true);
    let old = view.update(SIZE, Mode::Pruned).unwrap().frame;
    view.set_content(0, "文字").unwrap();
    let update = view.update(SIZE, Mode::Pruned).unwrap();
    assert!(update.published && update.skipped_parent);
    assert_eq!(update.resolved, 1);
    assert_eq!(old.geometry, update.frame.geometry);
    assert_eq!(old.draws[0].node, update.frame.draws[0].node);
    assert_eq!(old.draws[0].paragraph.text, "中文");
    assert_eq!(update.frame.draws[0].paragraph.text, "文字");
    assert_ne!(
        old.draws[0].paragraph.lines[0].glyphs[0].id,
        update.frame.draws[0].paragraph.lines[0].glyphs[0].id
    );
    assert!(Rc::ptr_eq(
        &old.draws[1].paragraph,
        &update.frame.draws[1].paragraph
    ));
    let clean = view.update(SIZE, Mode::Pruned).unwrap();
    assert!(!clean.published);
    assert!(Rc::ptr_eq(&clean.frame, &update.frame));
}

#[test]
fn color_only_update_does_not_execute_layout_or_text_resolution() {
    let mut view = Presentation::new(2, "中文", true);
    let old = view.update(SIZE, Mode::Normal).unwrap().frame;
    let counts = view.layout_counts();
    let shapes = view.shape_count();
    view.set_color(1, [80, 40, 20, 128]).unwrap();
    let update = view.update(SIZE, Mode::Normal).unwrap();
    assert!(update.published);
    assert_eq!(update.resolved, 0);
    assert_eq!(view.layout_counts(), counts);
    assert_eq!(view.shape_count(), shapes);
    assert_eq!(update.frame.geometry, old.geometry);
    assert_eq!(old.draws[1].color, [20, 30, 40, 255]);
    assert_eq!(update.frame.draws[1].color, [80, 40, 20, 128]);
    assert!(Rc::ptr_eq(
        &old.draws[1].paragraph,
        &update.frame.draws[1].paragraph
    ));
    view.set_color(1, [80, 40, 20, 128]).unwrap();
    assert!(!view.update(SIZE, Mode::Normal).unwrap().published);
}

#[test]
fn subtree_motion_updates_absolute_origin_and_retains_immutable_paragraph() {
    let mut view = Presentation::new(2, "中文", true);
    let old = view.update(SIZE, Mode::Pruned).unwrap().frame;
    let shapes = view.shape_count();
    assert_eq!(old.draws[1].origin.x, 2);
    assert_eq!(old.draws[1].origin.y, 29);
    view.set_shift(1, 9).unwrap();
    let update = view.update(SIZE, Mode::Pruned).unwrap();
    assert!(update.skipped_parent);
    assert_eq!(update.resolved, 0);
    assert_eq!(view.shape_count(), shapes);
    assert_eq!(update.frame.draws[1].origin.x, 11);
    assert_eq!(update.frame.draws[1].origin.y, 29);
    assert_eq!(old.draws[1].origin.x, 2);
    assert!(Rc::ptr_eq(
        &old.draws[1].paragraph,
        &update.frame.draws[1].paragraph
    ));
    assert!(
        update
            .frame
            .same_content(&view.update(SIZE, Mode::Forced).unwrap().frame)
    );
}

#[test]
fn preparation_failure_preserves_entire_frame_and_retries_after_layout_succeeded() {
    for fault in [
        Fault::Error(0),
        Fault::Panic(0),
        Fault::Error(1),
        Fault::Panic(1),
    ] {
        let mut view = Presentation::new(2, "中文", true);
        let old = view.update(SIZE, Mode::Pruned).unwrap().frame;
        view.set_content(0, "文字").unwrap();
        view.set_shift(1, 7).unwrap();
        view.set_color(1, [255, 0, 0, 255]).unwrap();
        view.fault = Some(fault);
        let result = catch_unwind(AssertUnwindSafe(|| view.update(SIZE, Mode::Pruned)));
        match fault {
            Fault::Error(_) => assert!(matches!(result.unwrap(), Err(LayoutError::Injected))),
            Fault::Panic(_) => assert!(result.is_err()),
        }
        assert!(Rc::ptr_eq(&old, &view.current().unwrap()));
        assert_eq!(old.draws[0].paragraph.text, "中文");
        assert_eq!(old.draws[1].origin.x, 2);
        assert_eq!(old.draws[1].color, [20, 30, 40, 255]);
        let counts = view.layout_counts();
        view.fault = None;
        let retry = view.update(SIZE, Mode::Pruned).unwrap();
        assert_eq!(view.layout_counts(), counts);
        assert_eq!(retry.frame.revision, old.revision + 1);
        assert_eq!(retry.frame.draws[0].paragraph.text, "文字");
        assert_eq!(retry.frame.draws[1].origin.x, 9);
        assert_eq!(retry.frame.draws[1].color, [255, 0, 0, 255]);
        assert_eq!(old.draws[0].paragraph.text, "中文");
        assert!(
            retry
                .frame
                .same_content(&view.update(SIZE, Mode::Forced).unwrap().frame)
        );
    }
}

#[test]
fn missing_glyph_keeps_the_old_frame_even_after_another_input_changed() {
    let mut view = Presentation::new(2, "中文", true);
    let old = view.update(SIZE, Mode::Normal).unwrap().frame;
    view.set_color(0, [255, 0, 0, 255]).unwrap();
    view.set_content(1, "🦀").unwrap();
    assert!(matches!(
        view.update(SIZE, Mode::Normal),
        Err(LayoutError::TextUnavailable)
    ));
    assert!(Rc::ptr_eq(&old, &view.current().unwrap()));
    view.set_content(1, "文字").unwrap();
    let new = view.update(SIZE, Mode::Normal).unwrap().frame;
    assert_eq!(new.draws[0].color, [255, 0, 0, 255]);
    assert_eq!(new.draws[1].paragraph.text, "文字");
}

#[test]
fn resize_failure_keeps_old_allocation_and_owned_frames_outlive_caches_and_scene() {
    let mut view = Presentation::new(1, "Hello 世界！中文排版文字换行与基线", true);
    let old = view.update(SIZE, Mode::Normal).unwrap().frame;
    let old_weak = Rc::downgrade(&old.draws[0].paragraph);
    let narrow = Size {
        width: 50,
        height: 200,
    };
    view.fault = Some(Fault::Error(0));
    assert!(matches!(
        view.update(narrow, Mode::Normal),
        Err(LayoutError::Injected)
    ));
    assert!(Rc::ptr_eq(&old, &view.current().unwrap()));
    view.fault = None;
    let new = view.update(narrow, Mode::Normal).unwrap().frame;
    assert_ne!(new.draws[0].allocated.width, old.draws[0].allocated.width);
    assert!(new.draws[0].paragraph.lines.len() > old.draws[0].paragraph.lines.len());
    view.release_caches();
    assert!(old_weak.upgrade().is_some());
    let weak = Rc::downgrade(&new.draws[0].paragraph);
    drop(view);
    assert!(!new.font.is_empty());
    assert!(weak.upgrade().is_some());
    drop(old);
    assert!(old_weak.upgrade().is_none());
    drop(new);
    assert!(weak.upgrade().is_none());
}

#[test]
fn mixed_updates_match_fresh_paragraphs_and_complete_frame_rebuilding() {
    crate::frame_cost::verify();
}

#[test]
fn first_preparation_failure_has_no_frame_and_can_publish_on_retry() {
    let mut view = Presentation::new(1, "中文", true);
    view.fault = Some(Fault::Error(0));
    assert!(matches!(
        view.update(SIZE, Mode::Normal),
        Err(LayoutError::Injected)
    ));
    assert!(view.current().is_none());
    let counts = view.layout_counts();
    view.fault = None;
    let retry = view.update(SIZE, Mode::Normal).unwrap();
    assert_eq!(view.layout_counts(), counts);
    assert_eq!(retry.frame.revision, 1);
    assert_eq!(retry.frame.draws[0].paragraph.text, "中文");
}
