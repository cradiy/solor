use super::fixtures::Wrapping;
use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Scene, Size,
};
use super::negotiation::{Button, fixture};
use solor_core::px;

#[test]
fn row_widths_baselines_and_overflow_have_known_values() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Exact(px(80.0)), None, false).unwrap();
    let geometry: Vec<_> = scene.snapshot().into_iter().map(Option::unwrap).collect();
    assert_eq!(
        geometry[0].metrics,
        Metrics {
            size: Size {
                width: px(80.0),
                height: px(14.0)
            },
            baseline: Some(px(8.0))
        }
    );
    assert_eq!(
        geometry[1].origin,
        Origin {
            x: px(2.0),
            y: px(2.0)
        }
    );
    assert_eq!(
        geometry[2].metrics.size,
        Size {
            width: px(48.0),
            height: px(5.0)
        }
    );
    assert_eq!(
        geometry[2].origin,
        Origin {
            x: px(15.0),
            y: px(4.0)
        }
    );
    assert_eq!(
        geometry[3].origin,
        Origin {
            x: px(66.0),
            y: px(2.0)
        }
    );
    assert_eq!(
        geometry[2].origin.y + geometry[2].metrics.baseline.unwrap(),
        geometry[3].origin.y + geometry[3].metrics.baseline.unwrap()
    );

    scene.run(Query::Exact(px(50.0)), None, false).unwrap();
    assert_eq!(
        scene.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: px(18.0),
            height: px(15.0)
        }
    );
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.size.height,
        px(21.0)
    );
    scene
        .edit_layout::<Button>(ids.button, |button| button.units = 8)
        .unwrap();
    scene.run(Query::Exact(px(50.0)), None, false).unwrap();
    assert_eq!(
        scene.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: px(10.0),
            height: px(20.0)
        }
    );
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.size.height,
        px(26.0)
    );
    for width in [20, 0] {
        scene
            .run(Query::Exact(px(width as f32)), None, false)
            .unwrap();
        let root = scene.geometry(scene.root()).unwrap();
        assert_eq!(
            root.metrics.size,
            Size {
                width: px(width as f32),
                height: px(106.0)
            }
        );
        assert_eq!(
            root.content,
            Size {
                width: px(40.0),
                height: px(106.0)
            }
        );
        assert_eq!(
            scene.geometry(ids.label).unwrap().metrics.size.width,
            px(0.0)
        );
        assert_eq!(scene.geometry(ids.label).unwrap().content.width, px(2.0));
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
            width: px(20.0),
            height: px(5.0)
        }
    );
    text.run(
        Query::Natural,
        Some(Size {
            width: px(8.0),
            height: px(20.0),
        }),
        false,
    )
    .unwrap();
    let geometry = text.geometry(text.root()).unwrap();
    assert_eq!(
        geometry.metrics.size,
        Size {
            width: px(8.0),
            height: px(20.0)
        }
    );
    assert_eq!(
        geometry.content,
        Size {
            width: px(8.0),
            height: px(15.0)
        }
    );

    let mut button = Scene::new(Button {
        units: 4,
        advance: px(2.0),
        line_height: px(6.0),
        ascent: px(4.0),
        padding: px(2.0),
    });
    assert_eq!(
        button
            .probe(button.root(), Query::Natural)
            .unwrap()
            .baseline,
        Some(px(6.0))
    );
    button
        .run(
            Query::Natural,
            Some(Size {
                width: px(12.0),
                height: px(30.0),
            }),
            false,
        )
        .unwrap();
    assert_eq!(
        button.geometry(button.root()).unwrap().metrics.baseline,
        Some(px(16.0))
    );
    button
        .run(
            Query::Natural,
            Some(Size {
                width: px(0.0),
                height: px(0.0),
            }),
            false,
        )
        .unwrap();
    let geometry = button.geometry(button.root()).unwrap();
    assert_eq!(geometry.metrics.size, Size::default());
    assert_eq!(geometry.metrics.baseline, Some(px(6.0)));
    assert_eq!(
        geometry.content,
        Size {
            width: px(12.0),
            height: px(10.0)
        }
    );

    let (mut row, ids) = fixture();
    row.run(
        Query::Natural,
        Some(Size {
            width: px(50.0),
            height: px(40.0),
        }),
        false,
    )
    .unwrap();
    assert_eq!(
        row.geometry(ids.label).unwrap().metrics.size,
        Size {
            width: px(18.0),
            height: px(15.0)
        }
    );
    let root = row.geometry(row.root()).unwrap();
    assert_eq!(
        root.metrics,
        Metrics {
            size: Size {
                width: px(50.0),
                height: px(40.0)
            },
            baseline: Some(px(17.5))
        }
    );
    assert_eq!(
        root.content,
        Size {
            width: px(50.0),
            height: px(30.5)
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
    let queries = [
        Query::Natural,
        Query::AtMost(px(20.0)),
        Query::Exact(px(20.0)),
    ];
    for query in queries {
        assert_eq!(scene.probe(root, query).unwrap().size.width, px(20.0));
    }
    assert_eq!(scene.counts().measures, 3);
    assert_eq!(scene.cached_queries(root), 3);
    for query in queries {
        scene.probe(root, query).unwrap();
    }
    assert_eq!(scene.counts().measures, 3);
    assert_eq!(scene.counts().queries, 6);
    assert_eq!(scene.counts().cache_hits, 3);
    assert_eq!(
        scene
            .probe(root, Query::Exact(px(8.0)))
            .unwrap()
            .size
            .height,
        px(15.0)
    );
    scene.probe(root, Query::Natural).unwrap();
    assert_eq!(scene.counts().measures, 5);
    for width in 0..256 {
        scene.probe(root, Query::Exact(px(width as f32))).unwrap();
        assert!(scene.cached_queries(root) <= 3);
    }
    scene
        .edit_layout::<Wrapping>(root, |text| text.units = 20)
        .unwrap();
    assert_eq!(scene.cached_queries(root), 0);
    assert_eq!(
        scene.probe(root, Query::Natural).unwrap().size.width,
        px(40.0)
    );
    assert_eq!(scene.cached_queries(root), 1);
    assert_eq!(
        scene
            .probe(root, Query::AtMost(px(80.0)))
            .unwrap()
            .size
            .width,
        px(40.0)
    );
    assert_eq!(
        scene
            .probe(root, Query::Exact(px(80.0)))
            .unwrap()
            .size
            .width,
        px(80.0)
    );
}

#[test]
fn repeated_queries_are_reused_when_only_the_rows_allocated_height_changes() {
    let (mut scene, ids) = fixture();
    scene.run(Query::Exact(px(80.0)), None, false).unwrap();
    let before = scene.counts();
    let label = scene.node_counts(ids.label).unwrap();
    let button = scene.node_counts(ids.button).unwrap();
    assert_eq!(
        scene.run(
            Query::Exact(px(80.0)),
            Some(Size {
                width: px(80.0),
                height: px(34.0)
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
    assert_eq!(scene.node_counts(ids.label).unwrap().layouts, label.layouts);
    assert_eq!(
        scene.node_counts(ids.button).unwrap().layouts,
        button.layouts
    );
    assert_eq!(
        scene.geometry(scene.root()).unwrap().metrics.baseline,
        Some(px(18.0))
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
            1 => Query::AtMost(px(width as f32)),
            _ => Query::Exact(px(width as f32)),
        };
        let assigned = if step % 4 == 0 {
            Some(Size {
                width: px((step * 13 % 83) as f32),
                height: px((step % 51) as f32),
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
    scene.run(Query::Exact(px(80.0)), None, false).unwrap();
    let before = scene.snapshot();
    scene
        .edit_layout::<Button>(ids.button, |button| button.advance = px(f32::MAX))
        .unwrap();
    assert_eq!(
        scene.run(Query::Exact(px(50.0)), None, false),
        Err(LayoutError::InvalidSize)
    );
    assert_eq!(scene.snapshot(), before);
    assert!(scene.is_pending());
    scene
        .edit_layout::<Button>(ids.button, |button| button.advance = px(2.0))
        .unwrap();
    scene.run(Query::Exact(px(80.0)), None, false).unwrap();
    assert_eq!(scene.snapshot(), before);
}

struct InvalidMeasurement;

impl LayoutNode for InvalidMeasurement {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(px(5.0)) + px(1.0),
                height: px(5.0),
            },
            baseline: None,
        })
    }
    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<LayoutOutput> {
        panic!("invalid measurement must not reach layout")
    }
}

#[test]
fn measured_boxes_must_obey_the_query_even_when_content_may_overflow() {
    for query in [
        Query::AtMost(px(4.0)),
        Query::Exact(px(4.0)),
        Query::Exact(px(0.0)),
    ] {
        let mut scene = Scene::new(InvalidMeasurement);
        assert_eq!(scene.run(query, None, false), Err(LayoutError::InvalidSize));
        assert_eq!(scene.snapshot(), vec![None]);
        assert_eq!(scene.cached_queries(scene.root()), 0);
    }
}
