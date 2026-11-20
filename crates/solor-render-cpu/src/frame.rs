use solor::{
    NodeId,
    layout::{Geometry, Origin, Size},
    text::Paragraph,
};
use std::rc::Rc;

/// One shaped paragraph and its drawing bounds in document coordinates.
#[derive(Debug)]
pub struct TextDraw {
    /// Stable identity of the source element.
    pub node: NodeId,
    /// Top-left corner in logical pixels.
    pub origin: Origin,
    /// Logical clipping extent.
    pub allocated: Size,
    /// Text color in RGBA byte order.
    pub color: solor::Rgba,
    /// Shared shaped lines and glyph positions.
    pub paragraph: Rc<Paragraph>,
}

/// The sole externally presented state: immutable geometry and owned drawing inputs.
#[derive(Debug)]
pub struct Frame {
    /// Revision assigned by the frame producer.
    pub revision: u64,
    /// Logical distance from the document top to the viewport top.
    pub scroll_y: solor::Pixels,
    /// Retained layout geometry in traversal order.
    pub geometry: Rc<[Geometry]>,
    /// Text commands in paint order.
    pub draws: Vec<TextDraw>,
}

impl Frame {
    /// Independent fixtures have different NodeIds; compare all visual inputs instead.
    pub fn same_content(&self, other: &Self) -> bool {
        self.geometry == other.geometry && self.same_drawing(other)
    }

    /// Compares the visual inputs of both drawing sequences.
    pub fn same_drawing(&self, other: &Self) -> bool {
        self.scroll_y == other.scroll_y
            && self.draws.len() == other.draws.len()
            && self.draws.iter().zip(&other.draws).all(|(a, b)| {
                a.origin == b.origin
                    && a.allocated == b.allocated
                    && a.color == b.color
                    && a.paragraph == b.paragraph
            })
    }

    /// Counts inline frame storage, geometry and reserved draw slots.
    pub fn shallow_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(self.geometry.as_ref())
            + self.draws.capacity() * std::mem::size_of::<TextDraw>()
    }
}
