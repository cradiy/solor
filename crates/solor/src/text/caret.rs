use super::{Line, Paragraph, Text};
use crate::{
    Draw, Instance, NodeId, StyledBody,
    layout::{Origin, Size},
    px,
    state::Runtime,
};
use unicode_segmentation::UnicodeSegmentation;

/// Bounds of the insertion point in logical window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretBounds {
    /// Top-left corner of the visible caret.
    pub origin: Origin,
    /// Width and line height after clipping.
    pub size: Size,
}

impl Paragraph {
    fn caret_position(&self, offset: usize) -> Option<(f32, &Line)> {
        let mut trailing = None;
        for line in &self.lines {
            let Some(byte) = offset.checked_sub(line.source_start) else {
                break;
            };
            if line.glyphs.is_empty() && byte == 0 {
                return Some((0.0, line));
            }
            for g in &line.glyphs {
                if g.start == g.end {
                    continue;
                }
                let leading = if g.rtl { g.x + g.width } else { g.x };
                let end = if g.rtl { g.x } else { g.x + g.width };
                if byte == g.start {
                    return Some((leading, line));
                }
                if (g.start..g.end).contains(&byte) {
                    let cluster = self
                        .text
                        .get(line.source_start + g.start..line.source_start + g.end)?;
                    let mut graphemes = cluster.grapheme_indices(true).peekable();
                    let mut before = 0;
                    while graphemes.next_if(|(i, _)| *i < byte - g.start).is_some() {
                        before += 1;
                    }
                    let fraction = before as f32 / (before + graphemes.count()) as f32;
                    return Some((leading + (end - leading) * fraction, line));
                }
                if byte == g.end {
                    trailing = Some((end, line));
                }
            }
        }
        trailing
    }
}

pub(crate) struct FrameCaret {
    label: NodeId,
    byte: usize,
    visible: bool,
    pub(crate) bounds: Option<CaretBounds>,
}

impl FrameCaret {
    pub(crate) fn focused(instance: &Instance, state: &Runtime<()>) -> Option<Self> {
        let source = instance.text_input_signal()?;
        let focused = instance.focused?;
        let mut nodes = vec![focused];
        while let Some(id) = nodes.pop() {
            let node = &instance.bodies[&id];
            if !node.visible
                || (id != focused
                    && node
                        .handlers
                        .as_ref()
                        .is_some_and(|h| h.text_input.is_some()))
            {
                continue;
            }
            if instance
                .scene
                .body::<StyledBody<Text>>(id)
                .is_ok_and(|b| b.content.input == Some(source))
            {
                let (byte, visible) = state.read(source).ok()?.label_caret();
                return Some(Self {
                    label: id,
                    byte,
                    visible,
                    bounds: None,
                });
            }
            nodes.extend(instance.scene.children(id).ok()?.rev());
        }
        None
    }

    pub(crate) fn record(&mut self, id: NodeId, commands: &mut Vec<Draw>, start: usize) {
        if id != self.label {
            return;
        }
        let Some(Draw::Text {
            paragraph,
            origin,
            size,
            clip,
            color,
        }) = commands[start..]
            .iter()
            .find(|c| matches!(c, Draw::Text { .. }))
        else {
            return;
        };
        let Some((x, line)) = paragraph.caret_position(self.byte) else {
            return;
        };
        let width = px(1.);
        let mut left = origin.x + px(x).max(px(0.)).min(size.width.saturating_sub(width));
        let mut top = origin.y + px(line.top);
        let mut right = left + width;
        let mut bottom = (top + px(line.height)).min(origin.y + size.height);
        top = top.max(origin.y);
        if let Some(clip) = clip {
            if let Some(x) = &clip.x {
                left = left.max(x.start);
                right = right.min(x.end);
            }
            if let Some(y) = &clip.y {
                top = top.max(y.start);
                bottom = bottom.min(y.end);
            }
        }
        if right <= left || bottom <= top {
            return;
        }
        let bounds = CaretBounds {
            origin: Origin { x: left, y: top },
            size: Size {
                width: right - left,
                height: bottom - top,
            },
        };
        self.bounds = Some(bounds);
        if self.visible {
            commands.push(Draw::Fill {
                clip: clip.clone(),
                origin: bounds.origin,
                size: bounds.size,
                fill: color.clone().into(),
                corner_radii: Default::default(),
            });
        }
    }
}
