use super::{TextInputState, TextSelection};
use std::borrow::Cow;

/// Committed context around the caret, with byte offsets relative to the snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct SurroundingText<'a> {
    text: Cow<'a, str>,
    selection: TextSelection,
    // Distinguish equal excerpts at different locations in the same field.
    document_selection: TextSelection,
}

impl SurroundingText<'_> {
    /// Returns the UTF-8 excerpt that can be sent to an input method.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the selection within the excerpt, collapsed during composition.
    pub fn selection(&self) -> TextSelection {
        self.selection
    }

    /// Retains the snapshot across platform event-loop turns.
    pub fn into_owned(self) -> SurroundingText<'static> {
        SurroundingText {
            text: Cow::Owned(self.text.into_owned()),
            selection: self.selection,
            document_selection: self.document_selection,
        }
    }

    /// Checks a deletion on either side of the selection against this excerpt.
    pub fn permits_deletion(&self, before: usize, after: usize) -> bool {
        let range = self.selection.range();
        range
            .start
            .checked_sub(before)
            .is_some_and(|i| self.text.is_char_boundary(i))
            && range
                .end
                .checked_add(after)
                .is_some_and(|i| self.text.is_char_boundary(i))
    }
}

impl TextInputState {
    /// Borrows up to the byte budget around the complete selection. During composition
    /// the replacement range is omitted. An oversized selection or embedded NUL
    /// makes the excerpt unavailable.
    pub fn surrounding_text(&self, max_bytes: usize) -> Option<SurroundingText<'_>> {
        let text = self.text();
        let selected = self.selection();
        let range = selected.range();
        let composing = self.preedit().is_some();
        let visible_len = if composing { 0 } else { range.len() };
        let budget = max_bytes.checked_sub(visible_len)?;
        let suffix = (text.len() - range.end).min(budget - range.start.min(budget / 2));
        let start = text.ceil_char_boundary(range.start.saturating_sub(budget - suffix));
        let end = text.floor_char_boundary(range.end + suffix);
        let excerpt = if composing && !range.is_empty() {
            Cow::Owned([&text[start..range.start], &text[range.end..end]].concat())
        } else {
            Cow::Borrowed(&text[start..end])
        };
        if excerpt.contains('\0') {
            return None;
        }
        let selection = if composing {
            TextSelection::caret(range.start - start)
        } else {
            TextSelection {
                anchor: selected.anchor - start,
                head: selected.head - start,
            }
        };
        Some(SurroundingText {
            text: excerpt,
            selection,
            document_selection: selected,
        })
    }
}
