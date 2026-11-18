use std::rc::Rc;

use crate::layout::{LayoutError, Query, Scene, Size};
use crate::text::{CACHE_SLOTS, Engine, MAX_TEXT_BYTES, Style, Text};
use crate::workload::{CHANGED, SAMPLE, STYLE};

#[test]
fn fixed_font_has_real_variable_advances_cjk_wrap_and_line_baselines() {
    let engine = Engine::new();
    let mut text = Text::new(Rc::clone(&engine), "中文", STYLE, true).unwrap();
    let natural = text.resolve(Query::Natural).unwrap();
    assert_eq!(natural.lines.len(), 1);
    assert!((natural.lines[0].width - 32.0).abs() < 0.01);
    assert!(natural.baseline.is_some_and(|b| b > 0 && b <= 24));
    let wrapped = text.resolve(Query::Exact(16)).unwrap();
    assert_eq!(wrapped.lines.len(), 2);
    assert!((wrapped.lines[1].baseline - wrapped.lines[0].baseline - 24.0).abs() < 0.01);
    let zero = text.resolve(Query::Exact(0)).unwrap();
    assert_eq!(zero.metrics(Query::Exact(0)).size.width, 0);
    assert!(zero.content.width > 0);
    text.set_content("WWW").unwrap();
    let wide = text.resolve(Query::Natural).unwrap();
    text.set_content("iii").unwrap();
    assert!(wide.content.width > text.resolve(Query::Natural).unwrap().content.width);
    text.set_content("Hello 世界！\n中文").unwrap();
    let multiline = text.resolve(Query::Exact(400)).unwrap();
    assert_eq!(multiline.lines.len(), 2);
    assert_eq!(multiline.lines[1].source_line, 1);
    assert_eq!(multiline.lines[1].glyphs[0].start, 0);
    assert_eq!(multiline.lines[1].glyphs[0].end, 3);
    text.set_content("").unwrap();
    let empty = text.resolve(Query::Natural).unwrap();
    assert_eq!(empty.content.width, 0);
    assert!(empty.lines.iter().all(|l| l.glyphs.is_empty()));
}

#[test]
fn query_modes_share_only_compatible_paragraphs_and_allocation_is_authoritative() {
    let engine = Engine::new();
    let text = Text::new(Rc::clone(&engine), SAMPLE, STYLE, true).unwrap();
    let bounded = text.resolve(Query::AtMost(100)).unwrap();
    let exact = text.resolve(Query::Exact(100)).unwrap();
    assert!(Rc::ptr_eq(&bounded, &exact));
    assert!(bounded.metrics(Query::AtMost(100)).size.width <= 100);
    assert_eq!(exact.metrics(Query::Exact(100)).size.width, 100);
    let mut scene = Scene::new(text);
    let root = scene.root();
    let natural = scene.probe(root, Query::Natural).unwrap();
    scene.probe(root, Query::Exact(180)).unwrap();
    assert_eq!(scene.probe(root, Query::Natural).unwrap(), natural);
    scene
        .run(
            Query::Natural,
            Some(Size {
                width: 60,
                height: 10,
            }),
            false,
        )
        .unwrap();
    let reference = Text::new(Engine::new(), SAMPLE, STYLE, false)
        .unwrap()
        .resolve(Query::Exact(60))
        .unwrap();
    let geometry = scene.geometry(root).unwrap();
    assert_eq!(
        geometry.metrics.size,
        Size {
            width: 60,
            height: 10
        }
    );
    assert_eq!(geometry.content, reference.content);
    assert_eq!(geometry.metrics.baseline, reference.baseline);
    assert!(geometry.content.height > 10);
}

#[test]
fn content_style_eviction_and_release_preserve_owned_results() {
    let engine = Engine::new();
    let mut text = Text::new(Rc::clone(&engine), SAMPLE, STYLE, true).unwrap();
    let owned = text.resolve(Query::Exact(100)).unwrap();
    let weak = Rc::downgrade(&owned);
    for width in [80, 90, 120, 130, 140] {
        text.resolve(Query::Exact(width)).unwrap();
    }
    assert_eq!(text.cache_usage().0, CACHE_SLOTS);
    text.set_content(CHANGED).unwrap();
    assert_eq!(text.cache_usage(), (0, 0));
    assert_eq!(owned.text, SAMPLE);
    let new = text.resolve(Query::Exact(100)).unwrap();
    assert_eq!(new.text, CHANGED);
    text.set_style(Style {
        size: 20,
        line_height: 30,
    })
    .unwrap();
    assert_eq!(text.cache_usage(), (0, 0));
    let large = text.resolve(Query::Exact(100)).unwrap();
    assert_eq!(large.style.size, 20);
    assert_ne!(large.lines, new.lines);
    text.release_cache();
    drop(text);
    drop(engine);
    assert_eq!(owned.text, SAMPLE);
    drop(owned);
    assert!(weak.upgrade().is_none());
}

#[test]
fn missing_glyph_failure_retains_published_geometry_and_retries() {
    let text = Text::new(Engine::new(), SAMPLE, STYLE, true).unwrap();
    let mut scene = Scene::new(text);
    let root = scene.root();
    scene.run(Query::Exact(100), None, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Text>(root, |t| t.set_content("🦀").unwrap())
        .unwrap();
    assert!(matches!(
        scene.run(Query::Exact(100), None, false),
        Err(LayoutError::Node("glyph unavailable"))
    ));
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene
        .edit_layout::<Text>(root, |t| t.set_content(SAMPLE).unwrap())
        .unwrap();
    scene.run(Query::Exact(100), None, false).unwrap();
    assert_eq!(scene.snapshot(), before);
    assert!(!scene.is_pending());
}

#[test]
fn rejected_input_preserves_text_and_cached_result() {
    let mut text = Text::new(Engine::new(), SAMPLE, STYLE, true).unwrap();
    let before = text.resolve(Query::Exact(100)).unwrap();
    assert_eq!(
        text.set_style(Style {
            size: 0,
            line_height: 24
        }),
        Err(LayoutError::InvalidSize)
    );
    assert_eq!(
        text.set_content(&"a".repeat(MAX_TEXT_BYTES + 1)),
        Err(LayoutError::InvalidSize)
    );
    assert!(matches!(
        text.resolve(Query::Exact(u32::MAX)),
        Err(LayoutError::InvalidSize)
    ));
    assert!(Rc::ptr_eq(
        &before,
        &text.resolve(Query::Exact(100)).unwrap()
    ));
}

#[test]
fn mixed_updates_match_fresh_text_buffers_and_forced_layout() {
    crate::workload::verify();
}

#[cfg(feature = "layout-audit")]
#[test]
fn equal_geometry_still_produces_new_glyphs_when_parent_layout_is_skipped() {
    use crate::column::Column;
    use crate::layout::validation::CheckTarget;
    let mut scene = Scene::new(Column { padding: 2, gap: 3 });
    let root = scene.root();
    let child = scene
        .append(root, Text::new(Engine::new(), "中文", STYLE, true).unwrap())
        .unwrap();
    let assigned = Size {
        width: 100,
        height: 60,
    };
    scene.run(Query::Exact(100), Some(assigned), false).unwrap();
    let old = scene
        .body::<Text>(child)
        .unwrap()
        .resolve(Query::Exact(96))
        .unwrap();
    let geometry = scene.snapshot();
    let mut measurement = scene
        .capture_selective_dependencies(root, CheckTarget::Measure(Query::Exact(100)))
        .unwrap()
        .unwrap();
    let mut placement = scene
        .capture_selective_dependencies(root, CheckTarget::Layout(assigned))
        .unwrap()
        .unwrap();
    scene
        .edit_layout::<Text>(child, |t| t.set_content("文字").unwrap())
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
    assert_eq!(scene.snapshot(), geometry);
    let current = scene
        .body::<Text>(child)
        .unwrap()
        .resolve(Query::Exact(96))
        .unwrap();
    assert_ne!(old.lines[0].glyphs[0].id, current.lines[0].glyphs[0].id);
    assert_eq!(old.text, "中文");
    assert_eq!(current.text, "文字");
}
