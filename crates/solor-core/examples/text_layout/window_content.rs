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
        let old = scroll_y.round();
        *scroll_y = (*scroll_y + delta_y).clamp(0.0, f64::from(row.max_scroll_y()));
        old != scroll_y.round()
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

    pub fn wants_frame_callback(&self) -> bool {
        matches!(
            self,
            Self::Disclosure { .. } | Self::Text { edit: 0..8, .. }
        )
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
                let frame = row.frame(size, scroll_y.round() as u32)?;
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
    fn fractional_scroll_coalesces_and_clamps_after_resize() {
        let row = Disclosure::new("A detail", &[], &["Tail"; 10]).unwrap();
        let mut content = Content::disclosure(row);
        let size = Size {
            width: 240,
            height: 100,
        };
        let initial = content.frame(size).unwrap();
        let end = initial.geometry[0].content.height - size.height;
        assert!(!content.scroll_by(-10.0));
        assert!(!content.scroll_by(0.25));
        assert!(content.scroll_by(0.25));
        assert!(content.scroll_by(1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, 2);
        assert!(content.scroll_by(500.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, end);
        assert!(!content.scroll_by(500.0));
        assert!(content.scroll_by(-1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, end - 1);
        let tall = Size {
            height: 500,
            ..size
        };
        assert_eq!(content.frame(tall).unwrap().scroll_y, 0);
        assert!(!content.scroll_by(1.0));
        assert_eq!(content.frame(size).unwrap().scroll_y, 0);
    }
}
