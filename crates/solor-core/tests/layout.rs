//! Measurement, placement and geometry publication cases.
use solor_core::px;
use std::rc::Rc;

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
        let mut y = px(0.0);
        for index in 0..cx.child_count() {
            let child = cx.measure_child(index, Query::Exact(assigned.width))?;
            cx.layout_child(index, child.size)?;
            cx.place_child(index, Origin { x: px(0.0), y })?;
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
                width: query.resolve(px(12.0)),
                height: px(self.height as f32),
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
        width: px(24.0),
        height: px(6.0),
    };
    assert_eq!(scene.run_assigned(viewport, false)?, 3);
    assert_eq!(scene.node_counts(root)?.measures, 0);
    let original = scene.snapshot();
    let held = scene.shared_snapshot().unwrap();
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
    assert!(scene.shared_snapshot().is_none());

    assert_eq!(scene.run_assigned(viewport, false)?, 3);
    assert_eq!(scene.geometry(first).unwrap().metrics.size.height, px(11.0));
    assert_eq!(scene.geometry(second).unwrap().origin.y, px(11.0));
    assert_eq!(scene.geometry(root).unwrap().content.height, px(14.0));
    assert_eq!(scene.geometry(root).unwrap().metrics.size, viewport);
    let updated = scene.shared_snapshot().unwrap();
    assert_eq!(held[2].origin.y, px(5.0));
    assert_eq!(updated[2].origin.y, px(11.0));
    let counts = scene.counts();
    assert_eq!(scene.run_assigned(viewport, false)?, 0);
    assert_eq!(scene.counts(), counts);
    assert_eq!(scene.run(Query::Exact(px(24.0)), Some(viewport), true)?, 0);
    assert_eq!(scene.node_counts(root)?.measures, 1);
    assert!(Rc::ptr_eq(&updated, &scene.shared_snapshot().unwrap()));
    Ok(())
}

#[test]
fn removing_a_zero_height_leaf_changes_membership_without_moving_any_node() -> Result<()> {
    let mut scene = Scene::new(Rows);
    assert!(scene.shared_snapshot().is_none());
    let size = Size {
        width: px(10.0),
        height: px(10.0),
    };
    let leaf = scene.mount(
        scene.root(),
        Leaf {
            height: 0,
            fail_once: false,
        },
    )?;
    scene.run_assigned(size, false)?;
    let held = scene.shared_snapshot().unwrap();
    scene.unmount_leaf(leaf)?;
    assert!(scene.shared_snapshot().is_none());
    assert_eq!(scene.run_assigned(size, false)?, 0);
    let remaining = scene.shared_snapshot().unwrap();
    assert_eq!(remaining.as_ref(), &held[..1]);

    let replacement = scene.mount(
        scene.root(),
        Leaf {
            height: 0,
            fail_once: true,
        },
    )?;
    assert_ne!(replacement, leaf);
    assert!(scene.run_assigned(size, false).is_err());
    assert!(scene.shared_snapshot().is_none());
    scene.run_assigned(size, false)?;
    let remounted = scene.shared_snapshot().unwrap();
    drop(scene);
    assert_eq!(held.len(), 2);
    assert_eq!(remounted, held);
    assert!(!Rc::ptr_eq(&remounted, &held));
    Ok(())
}

#[test]
fn indexed_mount_and_subtree_removal_preserve_sibling_handles_and_snapshot_lifetimes() -> Result<()>
{
    let mut scene = Scene::new(Rows);
    let root = scene.root();
    let leaf = |height| Leaf {
        height,
        fail_once: false,
    };
    let tail = scene.mount(root, leaf(5))?;
    let viewport = Size {
        width: px(32.0),
        height: px(64.0),
    };
    scene.run_assigned(viewport, false)?;
    let closed = scene.shared_snapshot().unwrap();
    assert!(scene.mount_at(root, 2, Rows).is_err());
    assert!(!scene.is_pending());
    assert!(Rc::ptr_eq(&closed, &scene.shared_snapshot().unwrap()));

    let section = scene.mount_at(root, 0, Rows)?;
    let child = scene.mount(section, leaf(12))?;
    assert!(scene.shared_snapshot().is_none());
    scene.run_assigned(viewport, false)?;
    let opened = scene.shared_snapshot().unwrap();
    assert_eq!(scene.geometry(tail).unwrap().origin.y, px(12.0));
    assert_eq!(opened.len(), 4);
    assert_eq!(
        scene.unmount_leaf(section),
        Err(LayoutError::UnexpectedChildCount)
    );
    assert!(Rc::ptr_eq(&opened, &scene.shared_snapshot().unwrap()));
    scene.unmount_subtree(section)?;
    assert!(scene.geometry(child).is_none());
    assert!(scene.geometry(section).is_none());
    assert!(scene.shared_snapshot().is_none());
    scene.run_assigned(viewport, false)?;
    assert_eq!(scene.geometry(tail).unwrap().origin.y, px(0.0));
    assert_eq!(scene.shared_snapshot().unwrap(), closed);
    assert_eq!(opened.len(), 4);
    Ok(())
}
