//! Layout for a title and optional detail in an ordered column of labels.
use solor_core::px;
use std::rc::Rc;

use solor_core::tree::NodeId;
use solor_ui::Column;

use crate::{
    font_fixture::{FONT, fixture_engine},
    frame::{Frame, STYLE, TextDraw},
    layout::{
        Geometry, LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics,
        Origin, Query, Result, Scene, Size,
    },
};
use solor_ui::text::{SharedEngine, Text};

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
    pub(super) max_scroll_y: solor_core::Pixels,
    detail_color: [u8; 4],
    dirty: bool,
}

impl DisclosureLayout {
    pub fn new(before: &[&str], after: &[&str]) -> Result<Self> {
        Self::with_engine(before, after, fixture_engine(), after.len() > 1)
    }

    fn with_engine(
        before: &[&str],
        after: &[&str],
        engine: SharedEngine,
        group_tail: bool,
    ) -> Result<Self> {
        let column = Column::new(px(2.0), px(3.0));
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
        let item = scene.append(root, Column::new(px(0.0), px(3.0)))?;
        let title = label(&mut scene, item, "Show details")?;
        let tail_group = group_tail
            .then(|| scene.append(root, Column::new(px(0.0), px(3.0))))
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
            max_scroll_y: px(0.0),
            detail_color: crate::disclosure::DETAIL_COLOR,
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

    pub fn update_detail_text(&mut self, content: &str) -> Result<()> {
        let node = self
            .detail
            .ok_or(LayoutError::Node("detail is not mounted"))?;
        if self.scene.body::<Text>(node)?.content() != content {
            let mut result = Ok(());
            self.scene
                .edit_layout::<Text>(node, |text| result = text.set_content(content))?;
            result?;
            self.dirty = true;
        }
        Ok(())
    }

    pub fn set_detail_color(&mut self, color: [u8; 4]) {
        if self.detail_color != color {
            self.detail_color = color;
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub(super) fn layout_counts(&self) -> crate::layout::Counts {
        self.scene.counts()
    }

    pub fn published_title(&self) -> Option<(&TextDraw, solor_core::Pixels, Size)> {
        let frame = self.published.as_ref()?;
        Some((
            frame.draws.iter().find(|draw| draw.node == self.title)?,
            frame.scroll_y,
            self.viewport?,
        ))
    }

    fn geometry(&self, node: NodeId) -> Result<Geometry> {
        self.scene
            .geometry(node)
            .ok_or(LayoutError::MissingLayout(0))
    }

    pub fn frame(
        &mut self,
        size: Size,
        requested_y: solor_core::Pixels,
        force: bool,
    ) -> Result<Rc<Frame>> {
        self.scene.run_assigned(size, force)?;
        let content_height = self.geometry(self.scene.root())?.content.height;
        let max_scroll_y = content_height.saturating_sub(size.height);
        let top = requested_y.min(max_scroll_y);
        let previous = self
            .published
            .as_ref()
            .filter(|_| !force && !self.dirty && self.viewport == Some(size));
        if let Some(frame) = previous
            && frame.scroll_y == top
        {
            return Ok(frame.clone());
        }
        let revision = self
            .published
            .as_ref()
            .map_or(0, |f| f.revision)
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        let frame = Rc::new(self.collect_frame(revision, top, size.height)?);
        self.published = Some(frame.clone());
        self.viewport = Some(size);
        self.max_scroll_y = max_scroll_y;
        self.dirty = false;
        Ok(frame)
    }

    fn collect_frame(
        &mut self,
        revision: u64,
        top: solor_core::Pixels,
        height: solor_core::Pixels,
    ) -> Result<Frame> {
        let geometry = self
            .scene
            .shared_snapshot()
            .ok_or(LayoutError::MissingLayout(0))?;
        // Insertion order: root, preceding rows, item, title, optional group, tail rows, optional detail.
        let (before, item_and_rest) = geometry[1..].split_at(self.before.len());
        let item_origin = item_and_rest[0].origin;
        let mut tail = &item_and_rest[2..];
        let tail_origin = if self.tail_group.is_some() {
            let (group, children) = tail.split_first().expect("mounted tail group");
            tail = children;
            group.origin
        } else {
            Origin::default()
        };
        let (after, detail) = tail.split_at(self.after.len());
        let item_rows = std::iter::once((self.title, &item_and_rest[1]))
            .chain(self.detail.into_iter().zip(detail))
            .map(|(id, row)| (id, row, item_origin));
        let rows = rows_from_offset(&self.before, before, Origin::default(), top)
            .chain(item_rows)
            .chain(rows_from_offset(&self.after, after, tail_origin, top));
        let bottom = top + height;
        let estimated_rows =
            ((height.get() / STYLE.line_height.get()).ceil() as usize).saturating_add(1);
        let mut draws =
            Vec::with_capacity(estimated_rows.min(self.before.len() + self.after.len() + 2));
        for (node, row, parent_origin) in rows {
            let allocated = row.metrics.size;
            let origin = translated(row.origin, parent_origin)?;
            // Rows are ordered top-to-bottom and clipped to their allocations.
            if height == px(0.0) || origin.y >= bottom {
                break;
            }
            if origin.y + allocated.height <= top {
                continue;
            }
            let paragraph = self
                .scene
                .body::<Text>(node)?
                .resolve(Query::Exact(allocated.width))?;
            let color = if Some(node) == self.detail {
                self.detail_color
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
            scroll_y: top,
            draws,
            geometry,
            font: FONT,
        })
    }
}

/// Both runs of sibling labels are sorted by their parent-local bottom edges.
fn rows_from_offset<'a>(
    nodes: &'a [NodeId],
    geometry: &'a [Geometry],
    parent: Origin,
    top: solor_core::Pixels,
) -> impl Iterator<Item = (NodeId, &'a Geometry, Origin)> {
    let local_top = top.saturating_sub(parent.y);
    let start = if local_top == px(0.0) {
        0
    } else {
        geometry.partition_point(|row| row.origin.y + row.metrics.size.height <= local_top)
    };
    nodes[start..]
        .iter()
        .zip(&geometry[start..])
        .map(move |(&id, row)| (id, row, parent))
}

#[cfg(test)]
#[path = "disclosure_viewport_tests.rs"]
mod viewport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "layout-audit")]
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
            let size = Size {
                width: px(width as f32),
                height: px(300.0),
            };
            let surface = crate::pixels::DeviceSize {
                width: width * 3 / 2,
                height: 450,
            };
            let clip = Clip::canvas(surface);
            let scale = RasterScale(1.5);
            let mut masks = GlyphMasks::new(64, 32 * 1024);
            let closed = list.frame(size, px(0.0), false).unwrap();
            let mut pixels =
                RetainedPixels::new_scaled(closed.clone(), surface, clip, scale, &mut masks)
                    .unwrap();
            let mut prev = closed.clone();
            for step in 0..6 {
                let open = step % 2 == 0;
                let neighbors = [closed.draws[0].node, closed.draws[2].node];
                let counts = neighbors.map(|id| list.scene.node_counts(id).unwrap());
                let title = if open { "Hide details" } else { "Show details" };
                list.set_detail(title, open.then_some(DETAIL)).unwrap();
                let frame = list.frame(size, px(0.0), false).unwrap();
                assert_eq!(frame.draws.len(), if open { 4 } else { 3 });
                assert_eq!(frame.draws[1].node, closed.draws[1].node);
                assert_eq!(frame.draws[1].origin, closed.draws[1].origin);
                assert_eq!(frame.draws[0].origin, closed.draws[0].origin);
                let last = frame.draws.last().unwrap();
                assert_eq!(
                    last.origin.y,
                    closed.draws[2].origin.y
                        + if open {
                            frame.draws[2].allocated.height
                                + px(3.0)
                                + frame.draws[1].allocated.height
                                - closed.draws[1].allocated.height
                        } else {
                            px(0.0)
                        }
                );
                for (i, draw) in [&frame.draws[0], last].into_iter().enumerate() {
                    assert_eq!(draw.node, neighbors[i]);
                    assert!(Rc::ptr_eq(&draw.paragraph, &closed.draws[i * 2].paragraph));
                    let now = list.scene.node_counts(draw.node).unwrap();
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
                let fresh = reference.frame(size, px(0.0), true).unwrap();
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
            width: px(240.0),
            height: px(160.0),
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
                        black_box(fresh.frame(size, px(0.0), false).unwrap());
                        *list = fresh;
                    } else {
                        list.set_detail(
                            if open { "Hide details" } else { "Show details" },
                            open.then_some(DETAIL),
                        )
                        .unwrap();
                        black_box(list.frame(size, px(0.0), false).unwrap());
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

    #[cfg(feature = "layout-audit")]
    #[test]
    fn topology_edits_reject_old_checkpoints_and_keep_last_frame_on_failure() {
        let size = Size {
            width: px(200.0),
            height: px(160.0),
        };
        let mut list = DisclosureLayout::new(BEFORE, AFTER).unwrap();
        let closed = list.frame(size, px(0.0), false).unwrap();
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
        assert!(list.frame(size, px(0.0), false).is_err());
        assert!(Rc::ptr_eq(list.published.as_ref().unwrap(), &closed));
        assert_eq!(
            list.scene
                .check_dependencies(&before, target)
                .unwrap()
                .outcome,
            CheckOutcome::Unverifiable
        );
        list.set_detail("Hide details", Some(DETAIL)).unwrap();
        let good = list.frame(size, px(0.0), false).unwrap();
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
        assert!(
            list.frame(size, px(0.0), false)
                .unwrap()
                .same_content(&closed)
        );
    }
}
