use anyhow::{Result, ensure};
use std::ops::Range;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Destination for moving a logical insertion point in committed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretMove {
    /// One extended grapheme towards the beginning.
    Previous,
    /// One extended grapheme towards the end.
    Next,
    /// Start of the nearest Unicode word to the left of the caret.
    PreviousWord,
    /// End of the nearest Unicode word to the right of the caret.
    NextWord,
    /// Beginning of the buffer.
    Start,
    /// End of the buffer.
    End,
}

/// A directed selection in UTF-8 byte offsets. Equal endpoints form a caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    /// The endpoint that stays in place when extending the selection.
    pub anchor: usize,
    /// The moving endpoint.
    pub head: usize,
}

impl TextSelection {
    /// Places both endpoints at the same byte offset.
    pub fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    /// Returns the selected byte range in document order.
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(super) fn validate(self, text: &str) -> Result<()> {
        ensure!(
            text.is_char_boundary(self.anchor) && text.is_char_boundary(self.head),
            "selection endpoints must be UTF-8 boundaries within the text"
        );
        Ok(())
    }
}

/// Provisional input displayed in place of the committed selection.
#[derive(Debug)]
pub struct Preedit {
    /// Text supplied by the input method.
    pub text: String,
    /// Byte offsets within the provisional text. `None` hides the preedit caret.
    pub selection: Option<TextSelection>,
}

/// Editable UTF-8 text with a directed selection and optional IME composition.
#[derive(Debug, Default)]
pub struct TextInputState {
    text: String,
    selection: TextSelection,
    preedit: Option<Preedit>,
}

impl TextInputState {
    /// Moves the caret. A directional move collapses an existing selection to that edge.
    pub fn move_caret(&mut self, movement: CaretMove) {
        if self.preedit.is_some() {
            return;
        }
        let range = self.selection.range();
        let offset = match movement {
            CaretMove::Previous | CaretMove::PreviousWord if !range.is_empty() => range.start,
            CaretMove::Next | CaretMove::NextWord if !range.is_empty() => range.end,
            _ => self.destination(movement),
        };
        self.selection = TextSelection::caret(offset);
    }

    /// Moves the head of the selection while keeping its anchor fixed.
    pub fn extend_selection(&mut self, movement: CaretMove) {
        if self.preedit.is_none() {
            self.selection.head = self.destination(movement);
        }
    }

    fn destination(&self, movement: CaretMove) -> usize {
        let head = self.selection.head;
        // Keep the whole word around the caret for Unicode boundary decisions.
        const SEPARATORS: [char; 4] = [' ', '\t', '\r', '\n'];
        match movement {
            CaretMove::Start => 0,
            CaretMove::End => self.text.len(),
            CaretMove::Previous => self.previous_boundary(head),
            CaretMove::Next => self.next_boundary(head),
            CaretMove::PreviousWord => {
                let end = self.text[head..]
                    .find(SEPARATORS)
                    .map_or(self.text.len(), |i| head + i);
                self.text[..end]
                    .unicode_word_indices()
                    .rfind(|(start, _)| *start < head)
                    .map_or(0, |(start, _)| start)
            }
            CaretMove::NextWord => {
                let start = self.text[..head].rfind(SEPARATORS).unwrap_or(0);
                self.text[start..]
                    .unicode_word_indices()
                    .map(|(i, word)| start + i + word.len())
                    .find(|end| *end > head)
                    .unwrap_or(self.text.len())
            }
        }
    }

    fn previous_boundary(&self, byte: usize) -> usize {
        GraphemeCursor::new(byte, self.text.len(), true)
            .prev_boundary(&self.text, 0)
            .expect("complete text")
            .unwrap_or(0)
    }

    fn next_boundary(&self, byte: usize) -> usize {
        GraphemeCursor::new(byte, self.text.len(), true)
            .next_boundary(&self.text, 0)
            .expect("complete text")
            .unwrap_or(self.text.len())
    }

    /// Deletes the selection or the grapheme immediately after the caret.
    pub fn delete_forward(&mut self) {
        if self.preedit.is_some() {
            return;
        }
        if self.selection.range().is_empty() {
            self.selection.head = self.next_boundary(self.selection.head);
        }
        self.commit("");
    }

    /// Deletes the selection or the grapheme immediately before the caret.
    /// While composing, the input method owns backspace.
    pub fn backspace(&mut self) {
        if self.preedit.is_some() {
            return;
        }
        if self.selection.anchor == self.selection.head {
            self.selection.anchor = self.previous_boundary(self.selection.head);
        }
        self.commit("");
    }

    fn label_window(&self, segments: [&str; 3]) -> (usize, usize) {
        let caret = self.preedit.as_ref().map_or(self.selection.head, |p| {
            segments[0].len() + p.selection.map_or(p.text.len(), |s| s.head)
        });
        let total = segments.iter().map(|s| s.len()).sum::<usize>();
        let start = caret
            .saturating_sub(super::MAX_TEXT_BYTES / 2)
            .min(total.saturating_sub(super::MAX_TEXT_BYTES));
        (start, caret)
    }

    pub(crate) fn label_caret(&self) -> (usize, bool) {
        let segments = self.display_segments();
        let (mut start, caret) = self.label_window(segments);
        let mut skipped = 0;
        for segment in segments {
            skipped += segment.ceil_char_boundary(start.min(segment.len()));
            start = start.saturating_sub(segment.len());
        }
        (
            caret - skipped,
            self.preedit.as_ref().is_none_or(|p| p.selection.is_some()),
        )
    }

    pub(crate) fn label_content(&self) -> crate::SharedString {
        let segments = self.display_segments();
        let (start, _) = self.label_window(segments);
        let end = start.saturating_add(super::MAX_TEXT_BYTES);
        let mut pos = 0;
        let slices = segments.map(|s| {
            let from = s.ceil_char_boundary(start.saturating_sub(pos).min(s.len()));
            let to = s.floor_char_boundary(end.saturating_sub(pos).min(s.len()));
            pos += s.len();
            &s[from..to]
        });
        if slices[1].is_empty() && slices[2].is_empty() {
            slices[0].into()
        } else {
            slices.concat().into()
        }
    }
    /// Creates a buffer with the caret at the end of the initial text.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            selection: TextSelection::caret(text.len()),
            text,
            preedit: None,
        }
    }

    /// Returns the committed value throughout composition.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the selection in the committed value.
    pub fn selection(&self) -> TextSelection {
        self.selection
    }

    /// Selects a range in the committed value and ends any active composition.
    pub fn select(&mut self, selection: TextSelection) -> Result<()> {
        selection.validate(&self.text)?;
        self.selection = selection;
        self.cancel_preedit();
        Ok(())
    }

    /// Returns the current provisional text and its local selection.
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// Replaces the provisional text over the current selection. Empty text clears it.
    pub fn set_preedit(
        &mut self,
        text: impl Into<String>,
        selection: Option<TextSelection>,
    ) -> Result<()> {
        let text = text.into();
        if let Some(selection) = selection {
            selection.validate(&text)?;
        }
        self.replace_preedit(Preedit { text, selection });
        Ok(())
    }

    pub(super) fn replace_preedit(&mut self, preedit: Preedit) {
        self.preedit = (!preedit.text.is_empty()).then_some(preedit);
    }

    /// Discards provisional input and reveals the original text and selection.
    pub fn cancel_preedit(&mut self) {
        self.preedit = None;
    }

    /// Replaces the committed selection, ends composition and places the caret after the insertion.
    pub fn commit(&mut self, text: &str) {
        let range = self.selection.range();
        let start = range.start;
        self.text.replace_range(range, text);
        self.selection = TextSelection::caret(start + text.len());
        self.cancel_preedit();
    }

    /// Deletes UTF-8 bytes on either side of the committed selection, preserving
    /// its direction and ending composition. Invalid byte ranges leave it intact.
    pub fn delete_surrounding(&mut self, before: usize, after: usize) -> Result<()> {
        let selected = self.selection.range();
        let start = selected.start.checked_sub(before);
        let end = selected.end.checked_add(after);
        let (Some(start), Some(end)) = (start, end) else {
            anyhow::bail!("surrounding deletion outside the text");
        };
        ensure!(
            self.text.is_char_boundary(start) && self.text.is_char_boundary(end),
            "surrounding deletion must end at UTF-8 boundaries within the text"
        );
        self.text.replace_range(selected.end..end, "");
        self.text.replace_range(start..selected.start, "");
        self.selection.anchor -= before;
        self.selection.head -= before;
        self.cancel_preedit();
        Ok(())
    }

    /// Borrows the visible prefix, preedit and suffix. Outside composition the first
    /// slice contains the whole committed value.
    pub fn display_segments(&self) -> [&str; 3] {
        match &self.preedit {
            Some(preedit) => {
                let range = self.selection.range();
                [
                    &self.text[..range.start],
                    &preedit.text,
                    &self.text[range.end..],
                ]
            }
            None => [&self.text, "", ""],
        }
    }
}
