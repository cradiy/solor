use std::any::Any;

use super::fixtures::Wrapping;
use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Scene, Size,
};
use super::negotiation::{Button, fixture};

#[test]
fn row_widths_baselines_and_overflow_have_known_values() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Exact(80), None, false).unwrap();
    let geometry: Vec<_> = scene.snapshot().into_iter().map(Option::unwrap).collect();
    assert_eq!(
        geometry[0].metrics,
        Metrics {
            size: Size {
                width: 80,
                height: 14
            },
            baseline: Some(8)
        }
    );
    assert_eq!(geometry[1].origin, Origin { x: 2, y: 2 });
    assert_eq!(
        geometry[2].metrics.size,
        Size {
            width: 48,
            height: 5
        }
    );
    assert_eq!(geometry[2].origin, Origin { x: 15, y: 4 });
    assert_eq!(geometry[3].origin, Origin { x: 66, y: 2 });
    assert_eq!(
        geometry[2].origin.y + geometry[2].metrics.baseline.unwrap(),
        geometry[3].origin.y + geometry[3].metrics.baseline.unwrap()
    );

    scene.run(Query::Exact(50), None, false).unwrap();
    assert_eq!(
        scene.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: 18,
            height: 15
        }
    );
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.size.height,
        21
    );
    scene
        .edit_layout::<Button>(ids.button, |button| button.units = 8)
        .unwrap();
    scene.run(Query::Exact(50), None, false).unwrap();
    assert_eq!(
        scene.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: 10,
            height: 20
        }
    );
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.size.height,
        26
    );
    for width in [20, 0] {
        scene.run(Query::Exact(width), None, false).unwrap();
        let root = scene.geometry(scene.root()).unwrap();
        assert_eq!(root.metrics.size, Size { width, height: 106 });
        assert_eq!(
            root.content,
            Size {
                width: 40,
                height: 106
            }
        );
        assert_eq!(scene.geometry(ids.label).unwrap().metrics.size.width, 0);
        assert_eq!(scene.geometry(ids.label).unwrap().content.width, 2);
    }
}

#[test]
fn final_allocation_controls_wrapping_and_baseline_instead_of_the_measurement() {
    let mut text = Scene::new(Wrapping {
        units: 10,
        advance: 2,
        line_height: 5,
        ascent: 4,
    });
    let measured = text.probe(text.root(), Query::Natural).unwrap();
    assert_eq!(
        measured.size,
        Size {
            width: 20,
            height: 5
        }
    );
    text.run(
        Query::Natural,
        Some(Size {
            width: 8,
            height: 20,
        }),
        false,
    )
    .unwrap();
    let geometry = text.geometry(text.root()).unwrap();
    assert_eq!(
        geometry.metrics.size,
        Size {
            width: 8,
            height: 20
        }
    );
    assert_eq!(
        geometry.content,
        Size {
            width: 8,
            height: 15
        }
    );

    let mut button = Scene::new(Button {
        units: 4,
        advance: 2,
        line_height: 6,
        ascent: 4,
        padding: 2,
    });
    assert_eq!(
        button
            .probe(button.root(), Query::Natural)
            .unwrap()
            .baseline,
        Some(6)
    );
    button
        .run(
            Query::Natural,
            Some(Size {
                width: 12,
                height: 30,
            }),
            false,
        )
        .unwrap();
    assert_eq!(
        button.geometry(button.root()).unwrap().metrics.baseline,
        Some(16)
    );
    button
        .run(
            Query::Natural,
            Some(Size {
                width: 0,
                height: 0,
            }),
            false,
        )
        .unwrap();
    let geometry = button.geometry(button.root()).unwrap();
    assert_eq!(geometry.metrics.size, Size::default());
    assert_eq!(geometry.metrics.baseline, Some(6));
    assert_eq!(
        geometry.content,
        Size {
            width: 12,
            height: 10
        }
    );

    let (mut row, ids) = fixture();
    row.run(
        Query::Natural,
        Some(Size {
            width: 50,
            height: 40,
        }),
        false,
    )
    .unwrap();
    assert_eq!(
        row.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: 18,
            height: 15
        }
    );
    let root = row.geometry(row.root()).unwrap();
    assert_eq!(
        root.metrics,
        Metrics {
            size: Size {
                width: 50,
                height: 40
            },
            baseline: Some(17)
        }
    );
    assert_eq!(
        root.content,
        Size {
            width: 50,
            height: 30
        }
    );
}

#[test]
fn query_kind_is_part_of_the_bounded_cache_key_and_edits_invalidate_all_entries() {
    let mut scene = Scene::new(Wrapping {
        units: 10,
        advance: 2,
        line_height: 5,
        ascent: 4,
    });
    let root = scene.root();
    let queries = [Query::Natural, Query::AtMost(20), Query::Exact(20)];
    for query in queries {
        assert_eq!(scene.probe(root, query).unwrap().size.width, 20);
    }
    assert_eq!(scene.counts().measures, 3);
    assert_eq!(scene.cached_queries(root), 3);
    for query in queries {
        scene.probe(root, query).unwrap();
    }
    assert_eq!(scene.counts().measures, 3);
    assert_eq!(scene.counts().queries, 6);
    assert_eq!(scene.counts().cache_hits, 3);
    assert_eq!(scene.probe(root, Query::Exact(8)).unwrap().size.height, 15);
    scene.probe(root, Query::Natural).unwrap();
    assert_eq!(scene.counts().measures, 5);
    for width in 0..256 {
        scene.probe(root, Query::Exact(width)).unwrap();
        assert!(scene.cached_queries(root) <= 3);
    }
    scene
        .edit_layout::<Wrapping>(root, |text| text.units = 20)
        .unwrap();
    assert_eq!(scene.cached_queries(root), 0);
    assert_eq!(scene.probe(root, Query::Natural).unwrap().size.width, 40);
    assert_eq!(scene.cached_queries(root), 1);
    assert_eq!(scene.probe(root, Query::AtMost(80)).unwrap().size.width, 40);
    assert_eq!(scene.probe(root, Query::Exact(80)).unwrap().size.width, 80);
}

#[test]
fn repeated_queries_are_reused_when_only_the_rows_allocated_height_changes() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Exact(80), None, false).unwrap();
    let before = scene.counts();
    let label = scene.node_counts(ids.label);
    let button = scene.node_counts(ids.button);
    assert_eq!(
        scene.run(
            Query::Exact(80),
            Some(Size {
                width: 80,
                height: 34
            }),
            false
        ),
        Ok(4)
    );
    let after = scene.counts();
    assert_eq!(after.measures, before.measures);
    assert_eq!(after.layouts - before.layouts, 1);
    assert_eq!(
        after.queries - before.queries,
        after.cache_hits - before.cache_hits
    );
    assert_eq!(scene.node_counts(ids.label).layouts, label.layouts);
    assert_eq!(scene.node_counts(ids.button).layouts, button.layouts);
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.baseline,
        Some(18)
    );
}

#[test]
fn mixed_row_queries_inputs_and_allocations_match_full_recomputation() {
    let (mut scene, ids) = fixture();
    let (mut reference, other) = fixture();
    for step in 0..384_u32 {
        let units = step * 7 % 45;
        let button_units = step * 3 % 13;
        scene
            .edit_layout::<Wrapping>(ids.label, |label| label.units = units)
            .unwrap();
        reference
            .edit_layout::<Wrapping>(other.label, |label| label.units = units)
            .unwrap();
        scene
            .edit_layout::<Button>(ids.button, |button| button.units = button_units)
            .unwrap();
        reference
            .edit_layout::<Button>(other.button, |button| button.units = button_units)
            .unwrap();
        let width = step * 11 % 101;
        let query = match step % 3 {
            0 => Query::Natural,
            1 => Query::AtMost(width),
            _ => Query::Exact(width),
        };
        let assigned = if step % 4 == 0 {
            Some(Size {
                width: step * 13 % 83,
                height: step % 51,
            })
        } else {
            None
        };
        scene.run(query, assigned, false).unwrap();
        reference.run(query, assigned, true).unwrap();
        assert_eq!(scene.snapshot(), reference.snapshot(), "step {step}");
        assert!(!scene.is_pending());
    }
}

#[test]
fn failed_natural_measurement_keeps_the_last_complete_row_and_retries() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Exact(80), None, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Button>(ids.button, |button| button.units = u32::MAX)
        .unwrap();
    assert_eq!(
        scene.run(Query::Exact(50), None, false),
        Err(LayoutError::InvalidSize)
    );
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene
        .edit_layout::<Button>(ids.button, |button| button.units = 4)
        .unwrap();
    scene.run(Query::Exact(80), None, false).unwrap();
    assert_eq!(scene.snapshot(), before);
}

struct InvalidMeasurement;

impl LayoutNode for InvalidMeasurement {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(5) + 1,
                height: 5,
            },
            baseline: None,
        })
    }
    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<LayoutOutput> {
        panic!("invalid measurement must not reach layout")
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[test]
fn measured_boxes_must_obey_the_query_even_when_content_may_overflow() {
    for query in [Query::AtMost(4), Query::Exact(4), Query::Exact(0)] {
        let mut scene = Scene::new(InvalidMeasurement);
        assert_eq!(scene.run(query, None, false), Err(LayoutError::InvalidSize));
        assert_eq!(scene.snapshot(), vec![None]);
        assert_eq!(scene.cached_queries(scene.root()), 0);
    }
}
