use super::super::fixtures::{Column, Drawing, Wrapping};
use super::*;

#[test]
fn changed_allocation_discards_old_parent_and_reuses_only_matching_descendants() {
    let mut scene = Scene::new(Column { padding: 0, gap: 0 });
    let root = scene.root();
    let text = scene
        .append(
            root,
            Wrapping {
                units: 20,
                advance: 1,
                line_height: 1,
                ascent: 1,
            },
        )
        .unwrap();
    let drawing = scene.append(root, Drawing::new(5, 5)).unwrap();
    let original = Size {
        width: 100,
        height: 100,
    };
    scene.run(Query::Exact(100), Some(original), false).unwrap();
    scene
        .edit_layout::<Wrapping>(text, |leaf| leaf.units = 30)
        .unwrap();
    scene.edit_layout::<Drawing>(drawing, |_| {}).unwrap();
    let before_text = scene.node_counts(text).layouts;
    let before_drawing = scene.node_counts(drawing).layouts;
    let assigned = Size {
        width: 10,
        height: 100,
    };
    {
        let attempt = Attempt(&mut scene.scratch);
        let reuse = attempt.0.reuse.get_or_insert_with(Default::default);
        reuse.active = true;
        reuse.auditing = true;
        layout_root(
            &mut scene.store,
            attempt.0,
            RootRequest::Measure(Query::Exact(100), Some(original)),
            false,
        )
        .unwrap();
        attempt.0.reuse.as_mut().unwrap().auditing = false;
        layout_root(
            &mut scene.store,
            attempt.0,
            RootRequest::Measure(Query::Exact(10), Some(assigned)),
            false,
        )
        .unwrap();
        assert_eq!(attempt.0.reuse.as_ref().unwrap().hits, 1);
        assert_eq!(publish_layout(&mut scene.store, attempt.0), 3);
    }
    assert_eq!(scene.node_counts(text).layouts, before_text + 2);
    assert_eq!(scene.node_counts(drawing).layouts, before_drawing + 1);
    assert_eq!(scene.geometry(drawing).unwrap().origin.y, 3);
    assert_eq!(
        scene.geometry(text).unwrap().metrics.size,
        Size {
            width: 10,
            height: 3
        }
    );
    let incremental = scene.snapshot();
    scene.run(Query::Exact(10), Some(assigned), true).unwrap();
    assert_eq!(scene.snapshot(), incremental);
    assert!(scene.staged_usage().iter().all(|&(len, _)| len == 0));
}
