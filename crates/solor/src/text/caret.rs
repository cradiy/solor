use super::{Line, Paragraph, TextInputState};
use crate::{
    Draw, NodeId,
    layout::{Origin, Size},
    px,
};
use std::ops::Range;
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

impl Cluster {
    fn x_at(&self, text: &str, byte: usize) -> f32 {
        if byte <= self.bytes.start {
            return self.leading;
        }
        if byte >= self.bytes.end {
            return self.trailing;
        }
        let text = &text[self.bytes.clone()];
        let offset = byte - self.bytes.start;
        let mut boundaries = text.grapheme_indices(true).peekable();
        let mut before = 0;
        while boundaries.next_if(|(i, _)| *i < offset).is_some() {
            before += 1;
        }
        let fraction = before as f32 / (before + boundaries.count()) as f32;
        self.leading + (self.trailing - self.leading) * fraction
    }
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
            for cluster in line.caret_clusters() {
                if cluster.bytes.contains(&offset) {
                    return Some((cluster.x_at(&self.text, offset), line));
                }
                if offset == cluster.bytes.end {
                    end = Some((cluster.trailing, line));
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
    selected: Range<usize>,
    selection_color: crate::Rgba,
    pub(crate) bounds: Option<CaretBounds>,
}

impl FrameCaret {
    pub(crate) fn new(label: NodeId, input: &TextInputState, selection_color: crate::Rgba) -> Self {
        let (byte, visible) = input.label_caret();
        let selection = match input.preedit() {
            Some(preedit) => preedit.selection,
            None => Some(input.selection()),
        };
        let anchor = selection.map_or(byte, |s| {
            byte.saturating_sub(s.head.saturating_sub(s.anchor))
                .saturating_add(s.anchor.saturating_sub(s.head))
        });
        Self {
            selected: byte.min(anchor)..byte.max(anchor),
            selection_color,
            label,
            byte,
            visible,
            bounds: None,
        }
    }

    fn highlights(
        &self,
        paragraph: &Paragraph,
        origin: Origin,
        size: Size,
        clip: &Option<std::sync::Arc<crate::ContentClip>>,
    ) -> smallvec::SmallVec<[Draw; 1]> {
        let mut fills = smallvec::SmallVec::new();
        let mut spans = smallvec::SmallVec::<[Range<f32>; 8]>::new();
        for line in &paragraph.lines {
            let top = px(line.top).max(px(0.));
            let bottom = px(line.top + line.height).min(size.height);
            if bottom <= top {
                continue;
            }
            for cluster in line.caret_clusters() {
                if self.selected.start >= cluster.bytes.end
                    || self.selected.end <= cluster.bytes.start
                {
                    continue;
                }
                let a = cluster.x_at(&paragraph.text, self.selected.start);
                let b = cluster.x_at(&paragraph.text, self.selected.end);
                let start = a.min(b).max(0.);
                let end = a.max(b).min(size.width.get());
                if start < end {
                    if let Some(last) = spans.last_mut()
                        && start <= last.end
                        && end >= last.start
                    {
                        last.start = last.start.min(start);
                        last.end = last.end.max(end);
                    } else {
                        spans.push(start..end);
                    }
                }
            }
            for span in spans.drain(..) {
                let origin = Origin {
                    x: origin.x + px(span.start),
                    y: origin.y + top,
                };
                let size = Size {
                    width: px(span.end - span.start),
                    height: bottom - top,
                };
                if clip.as_ref().is_none_or(|c| c.intersects(origin, size)) {
                    fills.push(Draw::Fill {
                        origin,
                        size,
                        clip: clip.clone(),
                        fill: self.selection_color.into(),
                        corner_radii: Default::default(),
                    });
                }
            }
        }
        fills
    }

    pub(crate) fn record(&mut self, id: NodeId, commands: &mut Vec<Draw>, start: usize) {
        if id != self.label {
            return;
        }
        let Some(offset) = commands[start..]
            .iter()
            .position(|c| matches!(c, Draw::Text { .. }))
        else {
            return;
        };
        let index = start + offset;
        let Draw::Text {
            paragraph,
            origin,
            size,
            clip,
            color,
        } = &commands[index]
        else {
            return;
        };
        let caret = paragraph
            .caret_position(self.byte)
            .and_then(|(x, line)| CaretBounds::clipped(x, line, *origin, *size, clip.as_deref()));
        self.bounds = caret;
        let marker = caret.filter(|_| self.visible).map(|bounds| Draw::Fill {
            clip: clip.clone(),
            origin: bounds.origin,
            size: bounds.size,
            fill: color.clone().into(),
            corner_radii: Default::default(),
        });
        if !self.selected.is_empty() && self.selection_color.alpha != 0 {
            let highlights = self.highlights(paragraph, *origin, *size, clip);
            commands.splice(index..index, highlights);
        }
        commands.extend(marker);
    }
}

impl CaretBounds {
    fn clipped(
        x: f32,
        line: &Line,
        origin: Origin,
        size: Size,
        clip: Option<&crate::ContentClip>,
    ) -> Option<Self> {
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
            return None;
        }
        Some(Self {
            origin: Origin { x: left, y: top },
            size: Size {
                width: right - left,
                height: bottom - top,
            },
        })
    }
}
