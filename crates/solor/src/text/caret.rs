use super::{Line, Paragraph, TextInputState};
use crate::{
    Draw, NodeId,
    layout::{Origin, Size},
    px,
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

struct Cluster {
    bytes: std::ops::Range<usize>,
    leading: f32,
    trailing: f32,
}

impl Line {
    fn caret_clusters(&self) -> impl Iterator<Item = Cluster> {
        self.glyphs
            .chunk_by(|a, b| a.start == b.start && a.end == b.end)
            .filter_map(|glyphs| {
                let first = &glyphs[0];
                if first.start == first.end {
                    return None;
                }
                let (left, right) = glyphs
                    .iter()
                    .skip(1)
                    .fold((first.x, first.x + first.width), |(l, r), g| {
                        (l.min(g.x), r.max(g.x + g.width))
                    });
                let (leading, trailing) = if first.rtl {
                    (right, left)
                } else {
                    (left, right)
                };
                Some(Cluster {
                    bytes: self.source_start + first.start..self.source_start + first.end,
                    leading,
                    trailing,
                })
            })
    }
}

impl Paragraph {
    pub(crate) fn closest_caret(&self, point: Origin) -> Option<usize> {
        let y = point.y.get();
        let line = self.lines.iter().min_by(|a, b| {
            let distance = |l: &Line| (l.top - y).max(y - l.top - l.height).max(0.);
            distance(a).total_cmp(&distance(b))
        })?;
        let x = point.x.get();
        let mut nearest = (f32::INFINITY, line.source_start);
        for Cluster {
            bytes,
            leading,
            trailing,
        } in line.caret_clusters()
        {
            if x > leading.min(trailing) && x < leading.max(trailing) {
                let text = self.text.get(bytes.clone())?;
                let fraction = (x - leading) / (trailing - leading);
                let index = (fraction * text.graphemes(true).count() as f32).round() as usize;
                return Some(
                    text.grapheme_indices(true)
                        .nth(index)
                        .map_or(bytes.end, |(i, _)| bytes.start + i),
                );
            }
            for (position, byte) in [(leading, bytes.start), (trailing, bytes.end)] {
                let distance = (x - position).abs();
                if distance < nearest.0 {
                    nearest = (distance, byte);
                }
            }
        }
        Some(nearest.1)
    }

    pub(crate) fn caret_position(&self, offset: usize) -> Option<(f32, &Line)> {
        let mut end = None;
        for line in &self.lines {
            if offset < line.source_start {
                break;
            }
            if line.glyphs.is_empty() && offset == line.source_start {
                return Some((0., line));
            }
            for Cluster {
                bytes,
                leading,
                trailing,
            } in line.caret_clusters()
            {
                if offset == bytes.start {
                    return Some((leading, line));
                }
                if bytes.contains(&offset) {
                    let local = offset - bytes.start;
                    let cluster = self.text.get(bytes)?;
                    let mut boundaries = cluster.grapheme_indices(true).peekable();
                    let mut before = 0;
                    while boundaries.next_if(|(i, _)| *i < local).is_some() {
                        before += 1;
                    }
                    let fraction = before as f32 / (before + boundaries.count()) as f32;
                    return Some((leading + (trailing - leading) * fraction, line));
                }
                if offset == bytes.end {
                    end = Some((trailing, line));
                }
            }
        }
        end
    }
}

pub(crate) struct FrameCaret {
    label: NodeId,
    byte: usize,
    visible: bool,
    pub(crate) bounds: Option<CaretBounds>,
}

impl FrameCaret {
    pub(crate) fn new(label: NodeId, input: &TextInputState) -> Self {
        let (byte, visible) = input.label_caret();
        Self {
            label,
            byte,
            visible,
            bounds: None,
        }
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
