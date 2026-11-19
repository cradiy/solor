use solor_core::px;
use std::rc::Rc;

use solor_core::tree::NodeId;

use crate::font_fixture::{FONT, fixture_engine};
#[cfg(feature = "layout-audit")]
use crate::layout::validation::{CheckTarget, SelectiveCheckpoint};
use crate::layout::{
    Counts, Geometry, LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext,
    Metrics, Origin, Query, Result, Scene, Size,
};
use solor_ui::layout::Column;
use solor_ui::text::{Paragraph, SharedEngine, Text, TextStyle};

const RESERVE: u32 = 16;
pub const STYLE: TextStyle = TextStyle {
    size: solor_ui::px(16.0),
    line_height: solor_ui::px(24.0),
};

// A fixed fixture reserves room for internal movement without changing its output.
struct Shift {
    x: u32,
}

impl LayoutNode for Shift {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let inner = match query {
            Query::Natural => Query::Natural,
            Query::AtMost(w) => Query::AtMost(w.saturating_sub(px(RESERVE as f32))),
            Query::Exact(w) => Query::Exact(w.saturating_sub(px(RESERVE as f32))),
        };
        let metrics = cx.measure_child(0, inner)?;
        Ok(Metrics {
            size: Size {
                width: query.resolve(
                    metrics
                        .size
                        .width
                        .checked_add(px(RESERVE as f32))
                        .ok_or(LayoutError::InvalidSize)?,
                ),
                height: metrics.size.height,
            },
            baseline: metrics.baseline,
        })
    }
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let output = cx.layout_child(
            0,
            Size {
                width: assigned.width.saturating_sub(px(RESERVE as f32)),
                height: assigned.height,
            },
        )?;
        cx.place_child(
            0,
            Origin {
                x: px(self.x as f32),
                y: px(0.0),
            },
        )?;
        Ok(LayoutOutput {
            baseline: output.baseline,
            content: Size {
                width: output
                    .content
                    .width
                    .checked_add(px(RESERVE as f32))
                    .ok_or(LayoutError::InvalidSize)?,
                height: output.content.height,
            },
        })
    }
}

#[derive(Debug)]
pub struct TextDraw {
    pub node: NodeId,
    pub origin: Origin,
    pub allocated: Size,
    pub color: [u8; 4],
    pub paragraph: Rc<Paragraph>,
}

/// The sole externally presented state: immutable geometry and owned drawing inputs.
#[derive(Debug)]
pub struct Frame {
    pub revision: u64,
    /// Logical distance from the document top to the viewport top.
    pub scroll_y: solor_core::Pixels,
    pub font: &'static [u8],
    pub geometry: Rc<[Geometry]>,
    pub draws: Vec<TextDraw>,
}

impl Frame {
    /// Independent fixtures have different NodeIds; compare all visual inputs instead.
    pub fn same_content(&self, other: &Self) -> bool {
        self.geometry == other.geometry && self.same_drawing(other)
    }

    pub fn same_drawing(&self, other: &Self) -> bool {
        self.scroll_y == other.scroll_y
            && self.font == other.font
            && self.draws.len() == other.draws.len()
            && self.draws.iter().zip(&other.draws).all(|(a, b)| {
                a.origin == b.origin
                    && a.allocated == b.allocated
                    && a.color == b.color
                    && a.paragraph == b.paragraph
            })
    }

    pub fn shallow_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(self.geometry.as_ref())
            + self.draws.capacity() * std::mem::size_of::<TextDraw>()
    }
}

struct Row {
    group: NodeId,
    text: NodeId,
    color: [u8; 4],
    text_dirty: bool,
}

#[derive(Clone, Copy)]
pub enum Mode {
    Normal,
    #[cfg(feature = "layout-audit")]
    Pruned,
    Forced,
}

// Run the same frame and pixel assertions for both library configurations.
#[cfg(all(test, feature = "layout-audit"))]
pub const INCREMENTAL: Mode = Mode::Pruned;
#[cfg(all(test, not(feature = "layout-audit")))]
pub const INCREMENTAL: Mode = Mode::Normal;

pub struct Report {
    pub frame: Rc<Frame>,
    pub published: bool,
    pub skipped_parent: bool,
    pub resolved: usize,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub enum Fault {
    Error(usize),
    Panic(usize),
}

/// Fixed root -> shift group -> text topology, in paint order. Not a widget API.
pub struct Presentation {
    scene: Scene,
    engine: SharedEngine,
    rows: Vec<Row>,
    frame: Option<Rc<Frame>>,
    request: Option<Size>,
    pending: bool,
    #[cfg(feature = "layout-audit")]
    checks: Option<[SelectiveCheckpoint; 2]>,
    #[cfg(test)]
    pub fault: Option<Fault>,
}

fn add_origin(a: Origin, b: Origin) -> Result<Origin> {
    Ok(Origin {
        x: a.x.checked_add(b.x).ok_or(LayoutError::InvalidSize)?,
        y: a.y.checked_add(b.y).ok_or(LayoutError::InvalidSize)?,
    })
}

impl Presentation {
    pub fn new(count: usize, content: &str, cached: bool) -> Self {
        assert!(count > 0);
        let engine = fixture_engine();
        let mut scene = Scene::new(Column::new(px(2.0), px(3.0)));
        let root = scene.root();
        let rows = (0..count)
            .map(|_| {
                let group = scene.append(root, Shift { x: 0 }).unwrap();
                let text = scene
                    .append(
                        group,
                        Text::new(Rc::clone(&engine), content, STYLE, cached).unwrap(),
                    )
                    .unwrap();
                Row {
                    group,
                    text,
                    color: [20, 30, 40, 255],
                    text_dirty: true,
                }
            })
            .collect();
        Self {
            scene,
            engine,
            rows,
            frame: None,
            request: None,
            pending: true,
            #[cfg(feature = "layout-audit")]
            checks: None,
            #[cfg(test)]
            fault: None,
        }
    }

    pub fn current(&self) -> Option<Rc<Frame>> {
        self.frame.as_ref().map(Rc::clone)
    }
    pub fn layout_counts(&self) -> Counts {
        self.scene.counts()
    }
    pub fn shape_count(&self) -> usize {
        self.engine.borrow().counts.shapes
    }

    pub fn set_content(&mut self, row: usize, content: &str) -> Result<()> {
        let row = self
            .rows
            .get_mut(row)
            .ok_or(LayoutError::MissingChild(row))?;
        self.pending = true;
        row.text_dirty = true;
        let mut result = Ok(());
        self.scene
            .edit_layout::<Text>(row.text, |text| result = text.set_content(content))?;
        result
    }

    pub fn set_style(&mut self, row: usize, style: TextStyle) -> Result<()> {
        let row = self
            .rows
            .get_mut(row)
            .ok_or(LayoutError::MissingChild(row))?;
        self.pending = true;
        row.text_dirty = true;
        let mut result = Ok(());
        self.scene
            .edit_layout::<Text>(row.text, |text| result = text.set_style(style))?;
        result
    }

    pub fn set_color(&mut self, index: usize, color: [u8; 4]) -> Result<()> {
        let row = self
            .rows
            .get_mut(index)
            .ok_or(LayoutError::MissingChild(index))?;
        if row.color != color {
            row.color = color;
            self.pending = true;
        }
        Ok(())
    }

    pub fn set_shift(&mut self, index: usize, x: u32) -> Result<()> {
        if x > RESERVE {
            return Err(LayoutError::InvalidSize);
        }
        let row = self
            .rows
            .get(index)
            .ok_or(LayoutError::MissingChild(index))?;
        self.pending = true;
        self.scene
            .edit_layout::<Shift>(row.group, |group| group.x = x)
    }

    fn layout(&mut self, size: Size, mode: Mode) -> Result<bool> {
        let query = Query::Exact(size.width);
        #[cfg(feature = "layout-audit")]
        if matches!(mode, Mode::Pruned) {
            let skipped = if let Some([measure, placement]) = &mut self.checks {
                self.scene
                    .run_pruned(query, size, measure, placement)?
                    .skipped_parent_layout
            } else {
                self.scene.run(query, Some(size), false)?;
                false
            };
            let targets = [CheckTarget::Measure(query), CheckTarget::Layout(size)];
            if let Some(checks) = &mut self.checks {
                for (check, target) in checks.iter_mut().zip(targets) {
                    if !self.scene.refresh_selective_dependencies(check, target)? {
                        return Err(LayoutError::InvalidCheckpointOwner);
                    }
                }
            } else {
                let root = self.scene.root();
                self.checks = Some([
                    self.scene
                        .capture_selective_dependencies(root, targets[0])?
                        .ok_or(LayoutError::InvalidCheckpointOwner)?,
                    self.scene
                        .capture_selective_dependencies(root, targets[1])?
                        .ok_or(LayoutError::InvalidCheckpointOwner)?,
                ]);
            }
            return Ok(skipped);
        }
        self.scene
            .run(query, Some(size), matches!(mode, Mode::Forced))?;
        Ok(false)
    }

    pub fn update(&mut self, size: Size, mode: Mode) -> Result<Report> {
        if !self.pending && self.request == Some(size) && !matches!(mode, Mode::Forced) {
            return Ok(Report {
                frame: self.current().unwrap(),
                published: false,
                skipped_parent: false,
                resolved: 0,
            });
        }
        self.pending = true;
        let revision = self
            .frame
            .as_ref()
            .map_or(0, |f| f.revision)
            .checked_add(1)
            .ok_or(LayoutError::RevisionExhausted)?;
        let skipped_parent = self.layout(size, mode)?;
        let geometry = self
            .scene
            .shared_snapshot()
            .ok_or(LayoutError::MissingLayout(0))?;
        let mut draws = Vec::with_capacity(self.rows.len());
        let mut resolved = 0;
        for (index, row) in self.rows.iter().enumerate() {
            let group = geometry[1 + index * 2];
            let text = geometry[2 + index * 2];
            let old = self.frame.as_ref().and_then(|f| f.draws.get(index));
            let paragraph = if !matches!(mode, Mode::Forced)
                && !row.text_dirty
                && let Some(old) = old
                && old.node == row.text
                && old.allocated.width == text.metrics.size.width
            {
                Rc::clone(&old.paragraph)
            } else {
                resolved += 1;
                self.scene
                    .body::<Text>(row.text)?
                    .resolve(Query::Exact(text.metrics.size.width))?
            };
            if paragraph.content != text.content || paragraph.baseline != text.metrics.baseline {
                return Err(LayoutError::InvalidSize);
            }
            draws.push(TextDraw {
                node: row.text,
                origin: add_origin(add_origin(geometry[0].origin, group.origin)?, text.origin)?,
                allocated: text.metrics.size,
                color: row.color,
                paragraph,
            });
            #[cfg(test)]
            match self.fault {
                Some(Fault::Error(at)) if at == index => return Err(LayoutError::Node("injected")),
                Some(Fault::Panic(at)) if at == index => {
                    panic!("injected drawing preparation failure")
                }
                _ => {}
            }
        }
        let frame = Rc::new(Frame {
            revision,
            scroll_y: px(0.0),
            font: FONT,
            geometry,
            draws,
        });
        // No callbacks or fallible preparation after this publication point.
        self.frame = Some(Rc::clone(&frame));
        self.request = Some(size);
        self.pending = false;
        for row in &mut self.rows {
            row.text_dirty = false;
        }
        Ok(Report {
            frame,
            published: true,
            skipped_parent,
            resolved,
        })
    }

    /// Benchmark baseline only: advance internal layout without collecting a frame.
    pub fn layout_only(&mut self, size: Size) -> Result<()> {
        self.layout(size, Mode::Normal).map(|_| ())
    }

    pub fn release_caches(&mut self) {
        for row in &self.rows {
            self.scene.body::<Text>(row.text).unwrap().release_cache();
        }
        self.scene.release_scratch();
    }
}
