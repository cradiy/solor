//! Layout for a title and optional detail in an ordered column of labels.
use std::{any::Any, rc::Rc};

use solor_core::tree::NodeId;

use crate::{
    fixtures::Column,
    frame::{Frame, STYLE, TextDraw},
    layout::{
        Geometry, LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics,
        Origin, Query, Result, Scene, Size,
    },
    text::{Engine, FONT, SharedEngine, Text},
};

/// Keep the tail's wrapping constraint identical to that of flat siblings.
struct GroupedList {
    column: Column,
    tail_index: usize,
}

impl LayoutNode for GroupedList {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        self.column.measure(cx, query)
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        self.column.layout_rows(cx, assigned, Some(self.tail_index))
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn translated(origin: Origin, parent: Origin) -> Result<Origin> {
    Ok(Origin {
        x: parent
            .x
            .checked_add(origin.x)
            .ok_or(LayoutError::InvalidSize)?,
        y: parent
            .y
            .checked_add(origin.y)
            .ok_or(LayoutError::InvalidSize)?,
    })
}

pub struct DisclosureLayout {
    scene: Scene,
    engine: SharedEngine,
    before: Vec<NodeId>,
    item: NodeId,
    title: NodeId,
    detail: Option<NodeId>,
    tail_group: Option<NodeId>,
    after: Vec<NodeId>,
    published: Option<Rc<Frame>>,
    viewport: Option<Size>,
    dirty: bool,
}

impl DisclosureLayout {
    pub fn new(before: &[&str], after: &[&str]) -> Result<Self> {
        Self::with_engine(before, after, Engine::new(), after.len() > 1)
    }

    fn with_engine(
        before: &[&str],
        after: &[&str],
        engine: SharedEngine,
        group_tail: bool,
    ) -> Result<Self> {
        let column = Column { padding: 2, gap: 3 };
        let mut scene = if group_tail {
            Scene::new(GroupedList {
                column,
                tail_index: before.len() + 1,
            })
        } else {
            Scene::new(column)
        };
        let root = scene.root();
        let label = |scene: &mut Scene, parent, value| {
            scene.append(parent, Text::new(engine.clone(), value, STYLE, true)?)
        };
        let before = before
            .iter()
            .map(|value| label(&mut scene, root, *value))
            .collect::<Result<Vec<_>>>()?;
        let item = scene.append(root, Column { padding: 0, gap: 3 })?;
        let title = label(&mut scene, item, "Show details")?;
        let tail_group = group_tail
            .then(|| scene.append(root, Column { padding: 0, gap: 3 }))
            .transpose()?;
        let after = after
            .iter()
            .map(|value| label(&mut scene, tail_group.unwrap_or(root), *value))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            scene,
            engine,
            before,
            item,
            title,
            detail: None,
            tail_group,
            after,
            published: None,
            viewport: None,
            dirty: true,
        })
    }

    pub fn set_detail(&mut self, title: &str, detail: Option<&str>) -> Result<()> {
        let text = detail
            .map(|s| Text::new(self.engine.clone(), s, STYLE, true))
            .transpose()?;
        let mut result = Ok(());
        self.scene
            .edit_layout::<Text>(self.title, |text| result = text.set_content(title))?;
        result?;
        self.dirty = true;
        if let Some(child) = self.detail {
            self.scene.unmount_leaf(child)?;
            self.detail = None;
        }
        if let Some(text) = text {
            self.detail = Some(self.scene.mount(self.item, text)?);
        }
        Ok(())
    }

    pub fn published_title(&self) -> Option<(&TextDraw, Size)> {
        Some((
            self.published.as_ref()?.draws.get(self.before.len())?,
            self.viewport?,
        ))
    }

    fn geometry(&self, node: NodeId) -> Result<Geometry> {
        self.scene
            .geometry(node)
            .ok_or(LayoutError::MissingLayout(0))
    }

    pub fn frame(&mut self, size: Size, force: bool) -> Result<Rc<Frame>> {
        if !force
            && !self.dirty
            && self.viewport == Some(size)
            && let Some(frame) = &self.published
        {
            return Ok(frame.clone());
        }
        let revision = self
            .published
            .as_ref()
            .map_or(0, |f| f.revision)
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        self.scene.run_assigned(size, force)?;
        let frame = Rc::new(self.collect_frame(revision, size.height)?);
        self.published = Some(frame.clone());
        self.viewport = Some(size);
        self.dirty = false;
        Ok(frame)
    }

    fn collect_frame(&mut self, revision: u64, bottom: u32) -> Result<Frame> {
        let item_origin = self.geometry(self.item)?.origin;
        let tail_origin = match self.tail_group {
            Some(group) => self.geometry(group)?.origin,
            None => Origin::default(),
        };
        let top_level = |id: &NodeId| (*id, Origin::default());
        let rows = self
            .before
            .iter()
            .map(top_level)
            .chain(
                [Some(self.title), self.detail]
                    .into_iter()
                    .flatten()
                    .map(|id| (id, item_origin)),
            )
            .chain(self.after.iter().map(|&id| (id, tail_origin)));
        let estimated_rows = bottom.div_ceil(u32::from(STYLE.line_height)) as usize + 1;
        let mut draws =
            Vec::with_capacity(estimated_rows.min(self.before.len() + self.after.len() + 2));
        for (node, parent_origin) in rows {
            let geometry = self.geometry(node)?;
            let allocated = geometry.metrics.size;
            let origin = translated(geometry.origin, parent_origin)?;
            // Rows are ordered top-to-bottom and clipped to their allocations.
            if origin.y >= bottom {
                break;
            }
            let paragraph = self
                .scene
                .body_mut::<Text>(node)?
                .resolve(Query::Exact(allocated.width))?;
            let color = if Some(node) == self.detail {
                [27, 85, 148, 255]
            } else {
                [20, 30, 40, 255]
            };
            draws.push(TextDraw {
                node,
                origin,
                allocated,
                paragraph,
                color,
            });
        }
        Ok(Frame {
            revision,
            draws,
            geometry: self.snapshot_geometry(tail_origin)?,
            font: FONT,
        })
    }

    fn snapshot_geometry(&self, tail_origin: Origin) -> Result<Vec<Geometry>> {
        if self.tail_group.is_none() {
            return self
                .scene
                .snapshot()
                .into_iter()
                .collect::<Option<_>>()
                .ok_or(LayoutError::MissingLayout(0));
        }
        // Publish the same logical nodes and coordinate spaces for either hierarchy.
        let mut geometry = Vec::with_capacity(
            3 + self.before.len() + self.after.len() + usize::from(self.detail.is_some()),
        );
        geometry.push(self.geometry(self.scene.root())?);
        for &node in self.before.iter().chain([&self.item, &self.title]) {
            geometry.push(self.geometry(node)?);
        }
        for &node in &self.after {
            let mut row = self.geometry(node)?;
            row.origin = translated(row.origin, tail_origin)?;
            geometry.push(row);
        }
        if let Some(detail) = self.detail {
            geometry.push(self.geometry(detail)?);
        }
        Ok(geometry)
    }
}

#[cfg(test)]
#[path = "disclosure_viewport_tests.rs"]
mod viewport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::validation::{CheckOutcome, CheckTarget};
    use crate::{
        damage::RetainedPixels,
        glyph_masks::GlyphMasks,
        pixels::{Clip, RasterScale, Surface},
    };

    const BEFORE: &[&str] = &["Previous item"];
    const AFTER: &[&str] = &["Next item"];
    const DETAIL: &str = "Hello 世界！Rust GUI 123";

    #[test]
    fn middle_edits_reuse_neighbor_layouts_and_repair_the_shifted_pixels() {
        let mut list = DisclosureLayout::new(BEFORE, AFTER).unwrap();
        let mut previous_detail = None;
        for width in [240, 80, 240] {
            let size = Size { width, height: 300 };
            let surface = Size {
                width: width * 3 / 2,
                height: 450,
            };
            let clip = Clip::canvas(surface);
            let scale = RasterScale(1.5);
            let mut masks = GlyphMasks::new(64, 32 * 1024);
            let closed = list.frame(size, false).unwrap();
            let mut pixels =
                RetainedPixels::new_scaled(closed.clone(), surface, clip, scale, &mut masks)
                    .unwrap();
            let mut prev = closed.clone();
            for step in 0..6 {
                let open = step % 2 == 0;
                let neighbors = [closed.draws[0].node, closed.draws[2].node];
                let counts = neighbors.map(|id| list.scene.node_counts(id));
                let title = if open { "Hide details" } else { "Show details" };
                list.set_detail(title, open.then_some(DETAIL)).unwrap();
                let frame = list.frame(size, false).unwrap();
                assert_eq!(frame.draws.len(), if open { 4 } else { 3 });
                assert_eq!(frame.draws[1].node, closed.draws[1].node);
                assert_eq!(frame.draws[1].origin, closed.draws[1].origin);
                assert_eq!(frame.draws[0].origin, closed.draws[0].origin);
                let last = frame.draws.last().unwrap();
                assert_eq!(
                    last.origin.y,
                    closed.draws[2].origin.y
                        + if open {
                            frame.draws[2].allocated.height + 3 + frame.draws[1].allocated.height
                                - closed.draws[1].allocated.height
                        } else {
                            0
                        }
                );
                for (i, draw) in [&frame.draws[0], last].into_iter().enumerate() {
                    assert_eq!(draw.node, neighbors[i]);
                    assert!(Rc::ptr_eq(&draw.paragraph, &closed.draws[i * 2].paragraph));
                    let now = list.scene.node_counts(draw.node);
                    assert_eq!(
                        (now.measures, now.layouts),
                        (counts[i].measures, counts[i].layouts)
                    );
                }
                if let Some(old) = previous_detail {
                    assert!(list.scene.geometry(old).is_none());
                }
                if open {
                    previous_detail = list.detail;
                } else {
                    assert!(list.detail.is_none());
                }
                let mut reference = DisclosureLayout::new(BEFORE, AFTER).unwrap();
                if open {
                    reference.set_detail(title, Some(DETAIL)).unwrap();
                }
                let fresh = reference.frame(size, true).unwrap();
                assert!(frame.same_content(&fresh));
                pixels
                    .update(frame.clone(), surface, clip, &mut masks)
                    .unwrap();
                assert_eq!(
                    pixels.surface().rgba,
                    Surface::raster(&fresh, surface, clip, (0, 0), scale, None)
                        .unwrap()
                        .rgba
                );
                assert!(frame.revision > prev.revision);
                prev = frame;
            }
        }
    }

    #[test]
    #[ignore = "release timing experiment"]
    fn measure_list_updates() {
        use std::{hint::black_box, time::Instant};

        let size = Size {
            width: 240,
            height: 160,
        };
        let mut lists = [
            DisclosureLayout::new(BEFORE, AFTER).unwrap(),
            DisclosureLayout::new(BEFORE, AFTER).unwrap(),
        ];
        let mut samples = [Vec::new(), Vec::new()];
        for batch in 0..8 {
            for offset in 0..2 {
                let mode = (batch + offset) % 2;
                let list = &mut lists[mode];
                let start = Instant::now();
                for step in 0..80 {
                    let open = step % 2 == 0;
                    if mode == 1 {
                        let mut fresh = DisclosureLayout::with_engine(
                            BEFORE,
                            AFTER,
                            list.engine.clone(),
                            false,
                        )
                        .unwrap();
                        if open {
                            fresh.set_detail("Hide details", Some(DETAIL)).unwrap();
                        }
                        black_box(fresh.frame(size, false).unwrap());
                        *list = fresh;
                    } else {
                        list.set_detail(
                            if open { "Hide details" } else { "Show details" },
                            open.then_some(DETAIL),
                        )
                        .unwrap();
                        black_box(list.frame(size, false).unwrap());
                    }
                }
                if batch > 0 {
                    samples[mode].push(start.elapsed().as_secs_f64() * 1e6 / 80.0);
                }
            }
        }
        for (label, mut values) in ["retained", "rebuilt"].into_iter().zip(samples) {
            values.sort_by(f64::total_cmp);
            eprintln!(
                "{label} microseconds/update: {:.2} [{:.2}, {:.2}]",
                values[3], values[0], values[6]
            );
        }
        assert!(
            lists[0]
                .published
                .as_ref()
                .unwrap()
                .same_content(lists[1].published.as_ref().unwrap())
        );
    }

    #[test]
    fn topology_edits_reject_old_checkpoints_and_keep_last_frame_on_failure() {
        let size = Size {
            width: 200,
            height: 160,
        };
        let mut list = DisclosureLayout::new(BEFORE, AFTER).unwrap();
        let closed = list.frame(size, false).unwrap();
        let root = list.scene.root();
        let target = CheckTarget::Layout(size);
        let mut check = list
            .scene
            .capture_selective_dependencies(root, target)
            .unwrap()
            .unwrap();
        let before = list
            .scene
            .capture_dependencies(root, target)
            .unwrap()
            .unwrap();
        assert_eq!(
            list.scene.unmount_leaf(list.item),
            Err(LayoutError::UnexpectedChildCount)
        );
        assert_eq!(
            list.scene
                .check_dependencies(&before, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unchanged
        );
        list.set_detail("Hide details", Some("🦀")).unwrap();
        assert!(list.frame(size, false).is_err());
        assert!(Rc::ptr_eq(list.published.as_ref().unwrap(), &closed));
        assert_eq!(
            list.scene
                .check_dependencies(&before, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unverifiable
        );
        list.set_detail("Hide details", Some(DETAIL)).unwrap();
        let good = list.frame(size, false).unwrap();
        assert!(good.revision > closed.revision);
        assert_eq!(
            list.scene
                .check_selective_dependencies(&mut check, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unverifiable
        );
        assert!(
            list.scene
                .refresh_selective_dependencies(&mut check, target)
                .unwrap()
        );
        assert_eq!(
            list.scene
                .check_selective_dependencies(&mut check, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unchanged
        );
        list.set_detail("Show details", None).unwrap();
        assert_eq!(
            list.scene
                .check_selective_dependencies(&mut check, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unverifiable
        );
        assert!(list.frame(size, false).unwrap().same_content(&closed));
    }
}
