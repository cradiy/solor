//! A single disclosure with a stable title and a conditionally mounted detail.
use std::rc::Rc;

use solor_core::{
    state::{Runtime, Signal},
    tree::NodeId,
};

use crate::{
    disclosure_layout::DisclosureLayout,
    frame::Frame,
    input::{Point, PointerEvent},
    layout::{LayoutError, Result, Size},
};
use solor_ui::text::MAX_TEXT_BYTES;

pub const DETAIL_COLOR: [u8; 4] = [27, 85, 148, 255];

#[derive(Clone, PartialEq)]
struct Label {
    text: Rc<str>,
    color: [u8; 4],
}

pub struct Disclosure {
    labels: Runtime<Label>,
    expanded: Signal<bool>,
    detail_label: Signal<Label>,
    sync_pending: bool,
    detail: Option<NodeId>,
    mounted: Option<NodeId>,
    layout: DisclosureLayout,
    pressed: Option<u64>,
}

impl Disclosure {
    pub fn new(before: &[&str], after: &[&str]) -> Result<Self> {
        let mut labels = Runtime::new(Label {
            text: Rc::from("Show details"),
            color: [20, 30, 40, 255],
        });
        let expanded = labels.create_signal(false);
        let detail_label = labels.create_signal(Label {
            text: Rc::from(""),
            color: DETAIL_COLOR,
        });
        labels
            .bind(expanded, labels.tree().root(), |open, title| {
                title.text = Rc::from(if *open {
                    "Hide details"
                } else {
                    "Show details"
                });
            })
            .expect("title belongs to the component");
        labels.flush();
        Ok(Self {
            labels,
            expanded,
            detail_label,
            sync_pending: false,
            detail: None,
            mounted: None,
            layout: DisclosureLayout::new(before, after)?,
            pressed: None,
        })
    }

    /// Change the detail inputs. A visible subscriber needs a frame when its value changes.
    pub fn set_detail(&mut self, text: &str, color: [u8; 4]) -> Result<bool> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(LayoutError::InvalidSize);
        }
        let old = self.labels.read(self.detail_label).expect("owned signal");
        let text = if old.text.as_ref() == text {
            if old.color == color {
                return Ok(false);
            }
            old.text.clone()
        } else {
            Rc::from(text)
        };
        self.labels
            .set(self.detail_label, Label { text, color })
            .expect("owned signal");
        Ok(self.detail.is_some())
    }

    /// Semantic action. The caller schedules a frame after activation.
    pub fn activate(&mut self) {
        let root = self.labels.tree().root();
        let open = self.detail.is_none();
        self.labels.set(self.expanded, open).expect("owned signal");
        if let Some(child) = self.detail.take() {
            self.labels.remove_subtree(child).expect("mounted detail");
        } else {
            let label = self
                .labels
                .read(self.detail_label)
                .expect("owned signal")
                .clone();
            let child = self.labels.append(root, label).expect("owned title");
            self.labels
                .bind(self.detail_label, child, |value, label| {
                    label.clone_from(value)
                })
                .expect("owned detail and signal");
            self.detail = Some(child);
        }
    }

    fn hits_title(&self, p: Point) -> bool {
        let Some((title, top, size)) = self.layout.published_title() else {
            return false;
        };
        let x = f64::from(title.origin.x);
        let y = f64::from(title.origin.y) - f64::from(top);
        p.x >= x
            && p.y >= y.max(0.0)
            && p.x < (x + f64::from(title.allocated.width)).min(f64::from(size.width))
            && p.y < (y + f64::from(title.allocated.height)).min(f64::from(size.height))
    }

    /// Only a completed primary press on the title requests a new frame.
    pub fn pointer(&mut self, event: PointerEvent) -> bool {
        match event {
            PointerEvent::Down(id, p) if self.pressed.is_none() && self.hits_title(p) => {
                self.pressed = Some(id)
            }
            PointerEvent::Move(id, p) if self.pressed == Some(id) && !self.hits_title(p) => {
                self.pressed = None
            }
            PointerEvent::Cancel(id) if self.pressed == Some(id) => self.pressed = None,
            PointerEvent::Up(id, p) if self.pressed == Some(id) => {
                self.pressed = None;
                if self.hits_title(p) {
                    self.activate();
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Largest vertical offset of the last successfully published viewport.
    pub fn max_scroll_y(&self) -> solor_core::Pixels {
        self.layout.max_scroll_y
    }

    pub fn frame(&mut self, size: Size, scroll_y: solor_core::Pixels) -> Result<Rc<Frame>> {
        // Keep synchronization pending until all bound inputs have reached the layout scene.
        self.sync_pending |= self.labels.flush() != 0;
        if self.sync_pending {
            let labels = self.labels.tree();
            let detail = self
                .detail
                .map(|child| labels.get(child).expect("mounted detail"));
            if self.mounted != self.detail {
                let title = labels.get(labels.root()).expect("owned title");
                self.layout
                    .set_detail(&title.text, detail.map(|label| label.text.as_ref()))?;
                self.mounted = self.detail;
            } else if let Some(label) = detail {
                self.layout.update_detail_text(&label.text)?;
            }
            if let Some(label) = detail {
                self.layout.set_detail_color(label.color);
            }
            self.sync_pending = false;
        }
        let frame = self.layout.frame(size, scroll_y, false)?;
        if self.pressed.is_some() && self.layout.published_title().is_none() {
            self.pressed = None;
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        damage::RetainedPixels,
        glyph_masks::GlyphMasks,
        pixels::{Clip, RasterScale, Surface},
    };
    use solor_core::px;

    const SIZE: Size = Size {
        width: px(240.0),
        height: px(84.0),
    };
    const INSIDE: Point = Point { x: 10.5, y: 12.0 };

    fn disclosure(text: &str, before: &[&str], after: &[&str]) -> Disclosure {
        let mut row = Disclosure::new(before, after).unwrap();
        row.set_detail(text, DETAIL_COLOR).unwrap();
        row
    }

    #[test]
    fn only_matching_release_inside_committed_title_activates() {
        use PointerEvent::*;
        let mut row = disclosure("Detail", &[], &[]);
        assert!(!row.pointer(Down(1, INSIDE)));
        let published = row.frame(SIZE, px(0.0)).unwrap();
        assert!(!row.pointer(Up(1, INSIDE)));
        let draw = &published.draws[0];
        let edge = Point {
            x: f64::from(draw.origin.x + draw.allocated.width),
            y: INSIDE.y,
        };
        for event in [
            Down(1, edge),
            Up(1, INSIDE),
            Down(1, INSIDE),
            Cancel(1),
            Up(1, INSIDE),
            Down(1, INSIDE),
            Move(1, edge),
            Move(1, INSIDE),
            Up(1, INSIDE),
        ] {
            assert!(!row.pointer(event));
        }
        assert!(Rc::ptr_eq(&published, &row.frame(SIZE, px(0.0)).unwrap()));
        assert!(!row.pointer(Down(1, INSIDE)));
        assert!(!row.pointer(Down(2, INSIDE)));
        assert!(!row.pointer(Cancel(2)));
        assert!(!row.pointer(Up(2, INSIDE)));
        assert!(row.pointer(Up(1, INSIDE)));
        assert!(!row.pointer(Up(1, INSIDE)));
        assert_eq!(row.frame(SIZE, px(0.0)).unwrap().draws.len(), 2);
    }

    #[test]
    fn scrolled_hit_testing_uses_the_visible_part_of_the_published_title() {
        use PointerEvent::{Down, Up};
        let mut row = disclosure("Detail", &["Before"], &["After"; 10]);
        let size = Size {
            width: px(240.0),
            height: px(64.0),
        };
        let first = row.frame(size, px(0.0)).unwrap();
        let title = &first.draws[1];
        let old = Point {
            x: 6.0,
            y: f64::from(title.origin.y + px(2.0)),
        };
        assert!(!row.pointer(Down(1, old)));
        row.frame(size, px(100.0)).unwrap();
        row.frame(size, px(0.0)).unwrap();
        assert!(!row.pointer(Up(1, old)));
        row.frame(size, px(100.0)).unwrap();
        assert!(!row.pointer(Down(1, old)));
        assert!(!row.pointer(Up(1, old)));

        let top = title.origin.y + px(8.0);
        row.frame(size, top).unwrap();
        for y in [-0.5, f64::from(title.allocated.height - px(8.0))] {
            let outside = Point { x: 6.0, y };
            assert!(!row.pointer(Down(1, outside)));
            assert!(!row.pointer(Up(1, outside)));
        }
        let visible = Point { x: 6.0, y: 0.0 };
        assert!(!row.pointer(Down(1, visible)));
        assert!(row.pointer(Up(1, visible)));
        assert!(
            row.frame(size, top)
                .unwrap()
                .draws
                .iter()
                .any(|draw| draw.paragraph.text == "Detail")
        );
    }

    #[test]
    fn bound_detail_edits_reflow_or_repaint_the_same_leaf() {
        let size = Size {
            width: px(120.0),
            height: px(240.0),
        };
        let physical = crate::pixels::DeviceSize {
            width: 180,
            height: 360,
        };
        let scale = RasterScale(1.5);
        let clip = Clip::canvas(physical);
        let mut row = disclosure("Detail", &["Before"], &["After"]);
        row.activate();
        let initial = row.frame(size, px(0.0)).unwrap();
        let mut masks = GlyphMasks::new(64, 32 * 1024);
        let mut pixels =
            RetainedPixels::new_scaled(initial.clone(), physical, clip, scale, &mut masks).unwrap();

        let text = "Hello 世界！Rust GUI 123 Hello 世界！";
        let counts = row.layout.layout_counts();
        assert!(row.set_detail("🦀", DETAIL_COLOR).unwrap());
        assert!(row.set_detail(text, DETAIL_COLOR).unwrap());
        assert_eq!(row.labels.pending_count(), 1);
        let wrapped = row.frame(size, px(0.0)).unwrap();
        assert_eq!(wrapped.draws[2].node, initial.draws[2].node);
        let growth = wrapped.draws[2].allocated.height - initial.draws[2].allocated.height;
        assert!(growth > px(0.0));
        assert_eq!(
            wrapped.draws[3].origin.y,
            initial.draws[3].origin.y + growth
        );
        let reflow = row.layout.layout_counts();
        assert!(reflow.measures > counts.measures && reflow.layouts > counts.layouts);
        let mut reference = DisclosureLayout::new(&["Before"], &["After"]).unwrap();
        reference.set_detail("Hide details", Some(text)).unwrap();
        let expected = reference.frame(size, px(0.0), true).unwrap();
        assert!(wrapped.same_content(&expected));
        pixels
            .update(wrapped.clone(), physical, clip, &mut masks)
            .unwrap();
        let raster =
            |frame: &Frame| Surface::raster(frame, physical, clip, (0, 0), scale, None).unwrap();
        assert_eq!(pixels.surface().rgba, raster(&expected).rgba);

        let ink = [170, 35, 70, 255];
        assert!(row.set_detail(text, ink).unwrap());
        let painted = row.frame(size, px(0.0)).unwrap();
        assert_eq!(row.layout.layout_counts(), reflow);
        assert!(Rc::ptr_eq(&painted.geometry, &wrapped.geometry));
        assert_eq!(painted.draws[2].node, wrapped.draws[2].node);
        assert_eq!(painted.draws[2].color, ink);
        assert_eq!(wrapped.draws[2].color, DETAIL_COLOR);
        assert!(Rc::ptr_eq(
            &wrapped.draws[2].paragraph,
            &painted.draws[2].paragraph
        ));
        assert_eq!(
            pixels
                .update(painted.clone(), physical, clip, &mut masks)
                .unwrap(),
            Some(scale.bounds(&painted.draws[2]).intersect(clip))
        );
        let painted_pixels = raster(&painted);
        assert_ne!(painted_pixels.rgba, raster(&wrapped).rgba);
        assert_eq!(pixels.surface().rgba, painted_pixels.rgba);
        assert!(!row.set_detail(text, ink).unwrap());
        assert!(row.set_detail("Superseded", DETAIL_COLOR).unwrap());
        assert!(row.set_detail(text, ink).unwrap());
        assert!(Rc::ptr_eq(&painted, &row.frame(size, px(0.0)).unwrap()));
    }

    #[test]
    fn failed_and_canceled_writes_do_not_leak_into_the_next_detail() {
        let mut row = disclosure("Old", &[], &[]);
        row.activate();
        let old = row.frame(SIZE, px(0.0)).unwrap();
        let old_node = old.draws[1].node;
        assert!(row.set_detail("🦀", DETAIL_COLOR).unwrap());
        assert!(row.frame(SIZE, px(0.0)).is_err());
        assert!(std::ptr::eq(
            row.layout.published_title().unwrap().0,
            &old.draws[0]
        ));
        assert!(row.frame(SIZE, px(0.0)).is_err());
        assert!(row.set_detail("New", DETAIL_COLOR).unwrap());
        let repaired = row.frame(SIZE, px(0.0)).unwrap();
        assert_eq!(repaired.draws[1].node, old_node);
        assert_eq!(repaired.draws[1].paragraph.text, "New");
        assert_eq!(old.draws[1].paragraph.text, "Old");

        assert!(row.set_detail("Queued", DETAIL_COLOR).unwrap());
        row.activate();
        assert!(!row.set_detail("While closed", DETAIL_COLOR).unwrap());
        row.activate();
        let reopened = row.frame(SIZE, px(0.0)).unwrap();
        assert_ne!(reopened.draws[1].node, old_node);
        assert_eq!(reopened.draws[1].paragraph.text, "While closed");
        assert_eq!(row.labels.binding_count(), 2);
    }

    #[test]
    fn toggling_releases_detail_bindings_and_matches_fresh_layout_and_pixels() {
        let detail = "Hello 世界！Rust GUI 123";
        let mut row = disclosure(detail, &[], &[]);
        let title_id = row.labels.tree().root();
        let physical = crate::pixels::DeviceSize {
            width: 360,
            height: 126,
        };
        let clip = Clip::canvas(physical);
        let scale = RasterScale(1.5);
        let first = row.frame(SIZE, px(0.0)).unwrap();
        let mut masks = GlyphMasks::new(64, 32 * 1024);
        let mut retained =
            RetainedPixels::new_scaled(first.clone(), physical, clip, scale, &mut masks).unwrap();
        let mut previous = first.revision;
        for _ in 0..12 {
            let old_child = row.detail;
            row.activate();
            assert_eq!(row.labels.tree().root(), title_id);
            if let Some(old) = old_child {
                assert!(row.labels.tree().get(old).is_none());
            }
            let open = old_child.is_none();
            let count = if open { 2 } else { 1 };
            assert_eq!(row.labels.binding_count(), count);
            let frame = row.frame(SIZE, px(0.0)).unwrap();
            assert_eq!(row.labels.pending_count(), 0);
            assert!(frame.revision > previous);
            previous = frame.revision;
            assert_eq!(frame.draws.len(), count);
            let mut reference = DisclosureLayout::new(&[], &[]).unwrap();
            if open {
                reference.set_detail("Hide details", Some(detail)).unwrap();
            }
            let reference = reference.frame(SIZE, px(0.0), true).unwrap();
            assert!(frame.same_content(&reference));
            retained.update(frame, physical, clip, &mut masks).unwrap();
            let oracle = Surface::raster(&reference, physical, clip, (0, 0), scale, None).unwrap();
            assert_eq!(retained.surface().rgba, oracle.rgba);
        }
        assert_eq!(first.draws[0].paragraph.text, "Show details");
    }
}
