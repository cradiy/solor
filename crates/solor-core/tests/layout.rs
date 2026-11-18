use solor_core::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Scene, Size,
};

struct Rows;

impl LayoutNode for Rows {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let mut size = Size::default();
        for index in 0..cx.child_count() {
            let child = cx.measure_child(index, query)?.size;
            size.width = size.width.max(child.width);
            size.height += child.height;
        }
        Ok(Metrics {
            size,
            baseline: None,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let mut y = 0;
        for index in 0..cx.child_count() {
            let child = cx.measure_child(index, Query::Exact(assigned.width))?;
            cx.layout_child(index, child.size)?;
            cx.place_child(index, Origin { x: 0, y })?;
            y += child.size.height;
        }
        Ok(LayoutOutput {
            content: Size {
                width: assigned.width,
                height: y,
            },
            baseline: None,
        })
    }
}

struct Leaf {
    height: u32,
    fail_once: bool,
}

impl LayoutNode for Leaf {
    fn measure(&mut self, _: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(Metrics {
            size: Size {
                width: query.resolve(12),
                height: self.height,
            },
            baseline: None,
        })
    }

    fn layout(&mut self, _: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        if std::mem::take(&mut self.fail_once) {
            return Err(LayoutError::Node("leaf unavailable"));
        }
        Ok(LayoutOutput {
            content: assigned,
            baseline: None,
        })
    }
}

#[test]
fn late_child_failure_preserves_geometry_and_same_allocation_retries() -> Result<()> {
    let mut scene = Scene::new(Rows);
    let root = scene.root();
    let first = scene.append(
        root,
        Leaf {
            height: 5,
            fail_once: false,
        },
    )?;
    let second = scene.append(
        root,
        Leaf {
            height: 3,
            fail_once: false,
        },
    )?;
    let viewport = Size {
        width: 24,
        height: 6,
    };
    assert_eq!(scene.run_assigned(viewport, false)?, 3);
    assert_eq!(scene.node_counts(root)?.measures, 0);
    let original = scene.snapshot();
    assert_eq!(
        scene.edit_layout::<Leaf>(root, |_| unreachable!()),
        Err(LayoutError::WrongNodeType)
    );
    assert!(!scene.is_pending());

    scene.edit_layout::<Leaf>(first, |node| node.height = 11)?;
    scene.edit_layout::<Leaf>(second, |node| node.fail_once = true)?;
    assert_eq!(
        scene.run_assigned(viewport, false),
        Err(LayoutError::Node("leaf unavailable"))
    );
    assert_eq!(scene.snapshot(), original);
    assert!(scene.is_pending());

    assert_eq!(scene.run_assigned(viewport, false)?, 3);
    assert_eq!(scene.geometry(first).unwrap().metrics.size.height, 11);
    assert_eq!(scene.geometry(second).unwrap().origin.y, 11);
    assert_eq!(scene.geometry(root).unwrap().content.height, 14);
    assert_eq!(scene.geometry(root).unwrap().metrics.size, viewport);
    let counts = scene.counts();
    assert_eq!(scene.run_assigned(viewport, false)?, 0);
    assert_eq!(scene.counts(), counts);
    assert_eq!(scene.run(Query::Exact(24), Some(viewport), true)?, 0);
    assert_eq!(scene.node_counts(root)?.measures, 1);
    Ok(())
}
