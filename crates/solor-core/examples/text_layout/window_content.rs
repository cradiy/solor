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
    Disclosure(Box<Disclosure>),
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

    pub fn accepts_pointer(&self) -> bool {
        matches!(self, Self::Disclosure(_))
    }

    pub fn pointer(&mut self, event: PointerEvent) -> bool {
        match self {
            Self::Disclosure(row) => row.pointer(event),
            Self::Text { .. } => false,
        }
    }

    pub fn edit(&self) -> usize {
        match self {
            Self::Text { edit, .. } => *edit,
            Self::Disclosure(_) => 0,
        }
    }

    pub fn wants_frame_callback(&self) -> bool {
        matches!(self, Self::Disclosure(_) | Self::Text { edit: 0..8, .. })
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
            Self::Disclosure(row) => row.frame(size, 0),
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
