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
    layout::{Result, Size},
};

pub struct Disclosure {
    labels: Runtime<Rc<str>>,
    expanded: Signal<bool>,
    detail_text: Signal<Rc<str>>,
    detail: Option<NodeId>,
    mounted: Option<NodeId>,
    layout: DisclosureLayout,
    pressed: Option<u64>,
}

impl Disclosure {
    pub fn new(detail: &str, before: &[&str], after: &[&str]) -> Result<Self> {
        let mut labels = Runtime::new(Rc::from("Show details"));
        let expanded = labels.create_signal(false);
        let detail_text = labels.create_signal(Rc::from(detail));
        labels
            .bind(expanded, labels.tree().root(), |open, title| {
                *title = Rc::from(if *open {
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
            detail_text,
            detail: None,
            mounted: None,
            layout: DisclosureLayout::new(before, after)?,
            pressed: None,
        })
    }

    /// Semantic action. The caller schedules a frame after activation.
    pub fn activate(&mut self) {
        let root = self.labels.tree().root();
        let open = self.detail.is_none();
        self.labels.set(self.expanded, open).expect("owned signal");
        if let Some(child) = self.detail.take() {
            self.labels.remove_subtree(child).expect("mounted detail");
        } else {
            let child = self.labels.append(root, Rc::from("")).expect("owned title");
            self.labels
                .bind(self.detail_text, child, |text, label| {
                    *label = Rc::clone(text)
                })
                .expect("owned detail and signal");
            self.detail = Some(child);
        }
        self.labels.flush();
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

    pub fn frame(&mut self, size: Size, scroll_y: u32) -> Result<Rc<Frame>> {
        if self.mounted != self.detail {
            let labels = self.labels.tree();
            let title = labels.get(labels.root()).expect("owned title");
            let detail = self
                .detail
                .map(|child| labels.get(child).expect("mounted detail").as_ref());
            self.layout.set_detail(title, detail)?;
            self.mounted = self.detail;
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

    const SIZE: Size = Size {
        width: 240,
        height: 84,
    };
    const INSIDE: Point = Point { x: 10.5, y: 12.0 };

    #[test]
    fn only_matching_release_inside_committed_title_activates() {
        use PointerEvent::*;
        let mut row = Disclosure::new("Detail", &[], &[]).unwrap();
        assert!(!row.pointer(Down(1, INSIDE)));
        let published = row.frame(SIZE, 0).unwrap();
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
        assert!(Rc::ptr_eq(&published, &row.frame(SIZE, 0).unwrap()));
        assert!(!row.pointer(Down(1, INSIDE)));
        assert!(!row.pointer(Down(2, INSIDE)));
        assert!(!row.pointer(Cancel(2)));
        assert!(!row.pointer(Up(2, INSIDE)));
        assert!(row.pointer(Up(1, INSIDE)));
        assert!(!row.pointer(Up(1, INSIDE)));
        assert_eq!(row.frame(SIZE, 0).unwrap().draws.len(), 2);
    }

    #[test]
    fn scrolled_hit_testing_uses_the_visible_part_of_the_published_title() {
        use PointerEvent::{Down, Up};
        let mut row = Disclosure::new("Detail", &["Before"], &["After"; 10]).unwrap();
        let size = Size {
            width: 240,
            height: 64,
        };
        let first = row.frame(size, 0).unwrap();
        let title = &first.draws[1];
        let old = Point {
            x: 6.0,
            y: f64::from(title.origin.y + 2),
        };
        assert!(!row.pointer(Down(1, old)));
        row.frame(size, 100).unwrap();
        row.frame(size, 0).unwrap();
        assert!(!row.pointer(Up(1, old)));
        row.frame(size, 100).unwrap();
        assert!(!row.pointer(Down(1, old)));
        assert!(!row.pointer(Up(1, old)));

        let top = title.origin.y + 8;
        row.frame(size, top).unwrap();
        for y in [-0.5, f64::from(title.allocated.height - 8)] {
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
    fn toggling_releases_detail_bindings_and_matches_fresh_layout_and_pixels() {
        let detail = "Hello 世界！Rust GUI 123";
        let mut row = Disclosure::new(detail, &[], &[]).unwrap();
        let title_id = row.labels.tree().root();
        let physical = Size {
            width: 360,
            height: 126,
        };
        let clip = Clip::canvas(physical);
        let scale = RasterScale(1.5);
        let first = row.frame(SIZE, 0).unwrap();
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
            assert_eq!(row.labels.pending_count(), 0);

            let frame = row.frame(SIZE, 0).unwrap();
            assert!(frame.revision > previous);
            previous = frame.revision;
            assert_eq!(frame.draws.len(), count);
            let mut reference = DisclosureLayout::new(&[], &[]).unwrap();
            if open {
                reference.set_detail("Hide details", Some(detail)).unwrap();
            }
            let reference = reference.frame(SIZE, 0, true).unwrap();
            assert!(frame.same_content(&reference));
            retained.update(frame, physical, clip, &mut masks).unwrap();
            let oracle = Surface::raster(&reference, physical, clip, (0, 0), scale, None).unwrap();
            assert_eq!(retained.surface().rgba, oracle.rgba);
        }
        assert_eq!(first.draws[0].paragraph.text, "Show details");
    }
}
