use solor_core::px;
use std::rc::Rc;

use crate::font_fixture::fixture_engine;
use crate::layout::{LayoutError, Query, Scene, Size};
use crate::workload::{CHANGED, SAMPLE, STYLE};
use solor_ui::text::{CACHE_SLOTS, MAX_TEXT_BYTES, Text, TextStyle};

#[test]
fn fixed_font_has_real_variable_advances_cjk_wrap_and_line_baselines() {
    let engine = fixture_engine();
    let mut text = Text::new(Rc::clone(&engine), "中文", STYLE, true).unwrap();
    let natural = text.resolve(Query::Natural).unwrap();
    assert_eq!(natural.lines.len(), 1);
    assert!((natural.lines[0].width - 32.0).abs() < 0.01);
    assert!(
        natural
            .baseline
            .is_some_and(|b| b > px(0.0) && b <= px(24.0))
    );
    let wrapped = text.resolve(Query::Exact(px(16.0))).unwrap();
    assert_eq!(wrapped.lines.len(), 2);
    assert!((wrapped.lines[1].baseline - wrapped.lines[0].baseline - 24.0).abs() < 0.01);
    let zero = text.resolve(Query::Exact(px(0.0))).unwrap();
    assert_eq!(zero.metrics(Query::Exact(px(0.0))).size.width, px(0.0));
    assert!(zero.content.width > px(0.0));
    text.set_content("WWW").unwrap();
    let wide = text.resolve(Query::Natural).unwrap();
    text.set_content("iii").unwrap();
    assert!(wide.content.width > text.resolve(Query::Natural).unwrap().content.width);
    text.set_content("Hello 世界！\n中文").unwrap();
    let multiline = text.resolve(Query::Exact(px(400.0))).unwrap();
    assert_eq!(multiline.lines.len(), 2);
    assert_eq!(multiline.lines[1].source_line, 1);
    assert_eq!(multiline.lines[1].glyphs[0].start, 0);
    assert_eq!(multiline.lines[1].glyphs[0].end, 3);
    text.set_content("").unwrap();
    let empty = text.resolve(Query::Natural).unwrap();
    assert_eq!(empty.content.width, px(0.0));
    assert!(empty.lines.iter().all(|l| l.glyphs.is_empty()));
}

#[test]
fn query_modes_share_only_compatible_paragraphs_and_allocation_is_authoritative() {
    let engine = fixture_engine();
    let text = Text::new(Rc::clone(&engine), SAMPLE, STYLE, true).unwrap();
    let bounded = text.resolve(Query::AtMost(px(100.0))).unwrap();
    let exact = text.resolve(Query::Exact(px(100.0))).unwrap();
    assert!(Rc::ptr_eq(&bounded, &exact));
    assert!(bounded.metrics(Query::AtMost(px(100.0))).size.width <= px(100.0));
    assert_eq!(exact.metrics(Query::Exact(px(100.0))).size.width, px(100.0));
    let mut scene = Scene::new(text);
    let root = scene.root();
    let natural = scene.probe(root, Query::Natural).unwrap();
    scene.probe(root, Query::Exact(px(180.0))).unwrap();
    assert_eq!(scene.probe(root, Query::Natural).unwrap(), natural);
    scene
        .run(
            Query::Natural,
            Some(Size {
                width: px(60.0),
                height: px(10.0),
            }),
            false,
        )
        .unwrap();
    let reference = Text::new(fixture_engine(), SAMPLE, STYLE, false)
        .unwrap()
        .resolve(Query::Exact(px(60.0)))
        .unwrap();
    let geometry = scene.geometry(root).unwrap();
    assert_eq!(
        geometry.metrics.size,
        Size {
            width: px(60.0),
            height: px(10.0)
        }
    );
    assert_eq!(geometry.content, reference.content);
    assert_eq!(geometry.metrics.baseline, reference.baseline);
    assert!(geometry.content.height > px(10.0));
}

#[test]
fn content_style_eviction_and_release_preserve_owned_results() {
    let engine = fixture_engine();
    let mut text = Text::new(Rc::clone(&engine), SAMPLE, STYLE, true).unwrap();
    let owned = text.resolve(Query::Exact(px(100.0))).unwrap();
    let weak = Rc::downgrade(&owned);
    for width in [80, 90, 120, 130, 140] {
        text.resolve(Query::Exact(px(width as f32))).unwrap();
    }
    assert_eq!(text.cache_usage().0, CACHE_SLOTS);
    text.set_content(CHANGED).unwrap();
    assert_eq!(text.cache_usage(), (0, 0));
    assert_eq!(owned.text, SAMPLE);
    let new = text.resolve(Query::Exact(px(100.0))).unwrap();
    assert_eq!(new.text, CHANGED);
    text.set_style(TextStyle {
        size: solor_ui::px(20.0),
        line_height: solor_ui::px(30.0),
    })
    .unwrap();
    assert_eq!(text.cache_usage(), (0, 0));
    let large = text.resolve(Query::Exact(px(100.0))).unwrap();
    assert_eq!(large.style.size, solor_ui::px(20.0));
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
    let text = Text::new(fixture_engine(), SAMPLE, STYLE, true).unwrap();
    let mut scene = Scene::new(text);
    let root = scene.root();
    scene.run(Query::Exact(px(100.0)), None, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Text>(root, |t| t.set_content("🦀").unwrap())
        .unwrap();
    assert!(matches!(
        scene.run(Query::Exact(px(100.0)), None, false),
        Err(LayoutError::Node("glyph unavailable"))
    ));
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene
        .edit_layout::<Text>(root, |t| t.set_content(SAMPLE).unwrap())
        .unwrap();
    scene.run(Query::Exact(px(100.0)), None, false).unwrap();
    assert_eq!(scene.snapshot(), before);
    assert!(!scene.is_pending());
}

#[test]
fn rejected_input_preserves_text_and_cached_result() {
    let mut text = Text::new(fixture_engine(), SAMPLE, STYLE, true).unwrap();
    let before = text.resolve(Query::Exact(px(100.0))).unwrap();
    assert_eq!(
        text.set_style(TextStyle {
            size: solor_ui::px(0.0),
            line_height: solor_ui::px(24.0)
        }),
        Err(LayoutError::InvalidSize)
    );
    assert_eq!(
        text.set_content("a".repeat(MAX_TEXT_BYTES + 1)),
        Err(LayoutError::InvalidSize)
    );
    assert!(matches!(
        text.resolve(Query::Exact(px(f32::MAX))),
        Err(LayoutError::InvalidSize)
    ));
    assert!(Rc::ptr_eq(
        &before,
        &text.resolve(Query::Exact(px(100.0))).unwrap()
    ));
}

#[test]
fn mixed_updates_match_fresh_text_buffers_and_forced_layout() {
    crate::workload::verify();
}

#[cfg(feature = "layout-audit")]
#[test]
fn equal_geometry_still_produces_new_glyphs_when_parent_layout_is_skipped() {
    use crate::layout::validation::CheckTarget;
    use solor_ui::layout::Column;
    let mut scene = Scene::new(Column::new(px(2.0), px(3.0)));
    let root = scene.root();
    let child = scene
        .append(
            root,
            Text::new(fixture_engine(), "中文", STYLE, true).unwrap(),
        )
        .unwrap();
    let assigned = Size {
        width: px(100.0),
        height: px(60.0),
    };
    scene
        .run(Query::Exact(px(100.0)), Some(assigned), false)
        .unwrap();
    let old = scene
        .body::<Text>(child)
        .unwrap()
        .resolve(Query::Exact(px(96.0)))
        .unwrap();
    let geometry = scene.snapshot();
    let mut measurement = scene
        .capture_selective_dependencies(root, CheckTarget::Measure(Query::Exact(px(100.0))))
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
            Query::Exact(px(100.0)),
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
        .resolve(Query::Exact(px(96.0)))
        .unwrap();
    assert_ne!(old.lines[0].glyphs[0].id, current.lines[0].glyphs[0].id);
    assert_eq!(old.text, "中文");
    assert_eq!(current.text, "文字");
}

#[test]
#[ignore = "allocation markers for an external profiler"]
fn short_label_allocations() {
    let engine = fixture_engine();
    let dynamic = String::from("设备");
    for (name, label) in [("literal", "Audio settings"), ("utf8", dynamic.as_str())] {
        println!("begin short-label/{name}");
        for _ in 0..1024 {
            let node = Text::new(engine.clone(), std::hint::black_box(label), STYLE, true).unwrap();
            std::hint::black_box(node);
        }
        println!("end short-label/{name}");
    }
}

#[test]
fn shared_source_across_nodes_and_widths() {
    let engine = fixture_engine();
    let source = solor_ui::SharedString::from(SAMPLE.repeat(16));
    let mut first = Text::new(engine.clone(), source.clone(), STYLE, true).unwrap();
    let second = Text::new(engine.clone(), source.clone(), STYLE, true).unwrap();
    let paragraphs = [&first, &second].map(|node| {
        assert_eq!(node.content().as_ptr(), source.as_ptr());
        [96, 240].map(|width| node.resolve(Query::Exact(px(width as f32))).unwrap())
    });
    assert_eq!(engine.borrow().counts.shapes, 4);
    for [narrow, wide] in &paragraphs {
        assert!(narrow.lines.len() > wide.lines.len());
        assert_eq!(narrow.text.as_ptr(), source.as_ptr());
        assert_eq!(wide.text.as_ptr(), source.as_ptr());
    }

    let replacement = solor_ui::SharedString::from(CHANGED.repeat(16));
    first.set_content(replacement.clone()).unwrap();
    assert_eq!(first.content().as_ptr(), replacement.as_ptr());
    assert_eq!(second.content().as_ptr(), source.as_ptr());
    drop((source, first, second, engine));
    assert_eq!(paragraphs[0][0].text.as_str(), SAMPLE.repeat(16));
}
