use solor_core::px;
use std::rc::Rc;

use crate::{
    disclosure::Disclosure,
    frame::{Frame, Mode, Presentation},
    input::PointerEvent,
    layout::{Result, Size},
};

const ROWS: [[&str; 2]; 2] = [
    ["Solor 中文排版", "Solor 文字换行与基线。"],
    ["Hello 世界！Rust GUI 123", "Hello 世界！测试界面宽度改变。"],
];

pub enum Content {
    Text {
        view: Box<Presentation>,
        edit: usize,
        advance: bool,
    },
    Disclosure {
        row: Box<Disclosure>,
        scroll_y: f64,
    },
}

impl Content {
    pub fn text() -> Result<Self> {
        let mut view = Presentation::new(2, ROWS[0][0], true);
        view.set_content(1, ROWS[1][0])?;
        view.set_color(1, [27, 85, 148, 255])?;
        Ok(Self::Text {
            view: Box::new(view),
            edit: 0,
            advance: false,
        })
    }

    pub fn disclosure(row: Disclosure) -> Self {
        Self::Disclosure {
            row: Box::new(row),
            scroll_y: 0.0,
        }
    }

    pub fn scroll_by(&mut self, delta_y: f64) -> bool {
        let Self::Disclosure { row, scroll_y } = self else {
            return false;
        };
        let old = *scroll_y;
        *scroll_y = (*scroll_y + delta_y).clamp(0.0, f64::from(row.max_scroll_y()));
        old != *scroll_y
    }

    pub fn accepts_pointer(&self) -> bool {
        matches!(self, Self::Disclosure { .. })
    }

    pub fn pointer(&mut self, event: PointerEvent) -> bool {
        match self {
            Self::Disclosure { row, .. } => row.pointer(event),
            Self::Text { .. } => false,
        }
    }

    pub fn edit(&self) -> usize {
        match self {
            Self::Text { edit, .. } => *edit,
            Self::Disclosure { .. } => 0,
        }
    }

    pub fn wants_frame_callback(&self, has_damage: bool) -> bool {
        match self {
            Self::Text { edit, .. } => *edit < 8,
            Self::Disclosure { .. } => has_damage,
        }
    }

    pub fn frame_ready(&mut self) -> bool {
        if let Self::Text {
            advance,
            edit: 0..8,
            ..
        } = self
        {
            *advance = true;
            return true;
        }
        false
    }

    pub fn frame(&mut self, size: Size) -> Result<Rc<Frame>> {
        match self {
            Self::Disclosure { row, scroll_y } => {
                let frame = row.frame(size, px(*scroll_y as f32))?;
                *scroll_y = scroll_y.min(f64::from(row.max_scroll_y()));
                Ok(frame)
            }
            Self::Text {
                view,
                edit,
                advance,
            } => {
                if std::mem::take(advance) {
                    *edit += 1;
                    let row = (*edit - 1) % 2;
                    view.set_content(row, ROWS[row][((*edit - 1) / 2 + 1) % 2])?;
                }
                Ok(view.update(size, Mode::Normal)?.frame)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_frame_keeps_scheduled_edits_running_and_allows_disclosure_to_idle() {
        let size = Size {
            width: px(240.0),
            height: px(120.0),
        };
        let mut text = Content::text().unwrap();
        if let Content::Text { view, .. } = &mut text {
            view.set_content(0, ROWS[0][1]).unwrap();
        }
        let target = crate::pixels::RasterScale::ONE.buffer_size(size).unwrap();
        let clip = crate::pixels::Clip::canvas(target);
        let mut masks = crate::glyph_masks::GlyphMasks::new(64, 32 * 1024);
        let mut pixels =
            crate::damage::RetainedPixels::new(text.frame(size).unwrap(), target, clip, &mut masks)
                .unwrap();
        assert!(text.frame_ready());
        let damage = pixels
            .update(text.frame(size).unwrap(), target, clip, &mut masks)
            .unwrap();
        assert!(damage.is_none());
        assert!(text.wants_frame_callback(damage.is_some()));
        assert!(text.frame_ready());
        assert!(
            pixels
                .update(text.frame(size).unwrap(), target, clip, &mut masks)
                .unwrap()
                .is_some()
        );

        let mut disclosure = Content::disclosure(Disclosure::new(&[], &[]).unwrap());
        let closed = disclosure.frame(size).unwrap();
        assert!(!disclosure.wants_frame_callback(false));
        let inside = crate::input::Point { x: 6.0, y: 10.0 };
        disclosure.pointer(PointerEvent::Down(1, inside));
        assert!(disclosure.pointer(PointerEvent::Up(1, inside)));
        assert!(!closed.same_content(&disclosure.frame(size).unwrap()));
        assert!(disclosure.wants_frame_callback(true));
    }

    #[test]
    fn fractional_scroll_coalesces_and_clamps_after_resize() {
        let row = Disclosure::new(&[], &["Tail"; 10]).unwrap();
        let mut content = Content::disclosure(row);
        let size = Size {
            width: px(240.0),
            height: px(100.0),
        };
        let initial = content.frame(size).unwrap();
        let end = initial.geometry[0].content.height - size.height;
        assert!(!content.scroll_by(-10.0));
        assert!(content.scroll_by(0.25));
        assert!(content.scroll_by(0.25));
        assert!(content.scroll_by(1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, px(1.5));
        assert!(content.scroll_by(500.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, end);
        assert!(!content.scroll_by(500.0));
        assert!(content.scroll_by(-1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, end - px(1.0));
        let tall = Size {
            height: px(500.0),
            ..size
        };
        assert_eq!(content.frame(tall).unwrap().scroll_y, px(0.0));
        assert!(!content.scroll_by(1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, px(0.0));
    }
}
