use super::Pixels;
use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, Margin, MeasureContext, Metrics, Origin,
    Query, Result, Size,
};

fn add(a: Pixels, b: Pixels) -> Result<Pixels> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

/// Values for the four sides of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges<T = Pixels> {
    /// Value on the upper edge.
    pub top: T,
    /// Value on the right edge.
    pub right: T,
    /// Value on the lower edge.
    pub bottom: T,
    /// Value on the left edge.
    pub left: T,
}

impl<T: Copy> Edges<T> {
    /// Assigns one value to all four sides.
    pub const fn all(value: T) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// Margins on all four edges of a child's border box.
pub type Margins = Edges<Margin>;

impl Margins {
    pub(crate) fn horizontal(self) -> Result<Pixels> {
        add(self.left.fixed_or_zero(), self.right.fixed_or_zero())
    }

    pub(crate) fn inner_width(self, width: Pixels) -> Result<Pixels> {
        let width = width.saturating_sub(self.horizontal()?);
        if width.is_extent() {
            Ok(width)
        } else {
            Err(LayoutError::InvalidSize)
        }
    }

    fn left_offset(self, available: Pixels, child: Pixels) -> Pixels {
        match self.left {
            Margin::Fixed(value) => value,
            Margin::Auto => {
                let free = available.saturating_sub(child);
                if self.right == Margin::Auto {
                    free / 2.0
                } else {
                    free
                }
            }
        }
    }
}

/// Measures and stacks children from top to bottom in sibling order.
pub struct Column {
    /// Insets between the column bounds and its children.
    pub padding: Edges,
    /// Space between adjacent children in layout units.
    pub gap: Pixels,
}

impl Column {
    /// Creates a column with uniform padding and a gap between rows.
    pub const fn new(padding: Pixels, gap: Pixels) -> Self {
        Self {
            padding: Edges::all(padding),
            gap,
        }
    }

    fn child_query(&self, query: Query) -> Result<Query> {
        let inset = add(self.padding.left, self.padding.right)?;
        Ok(match query {
            Query::Natural => Query::Natural,
            Query::AtMost(width) => Query::AtMost(width.saturating_sub(inset)),
            Query::Exact(width) => Query::Exact(width.saturating_sub(inset)),
        })
    }

    /// A stretched child receives the width left after padding and fixed margins. Content bounds
    /// still describe the measured children and their reported overflow.
    pub fn layout_rows(
        &self,
        cx: &mut LayoutContext<'_, '_>,
        assigned: Size,
        stretch: Option<usize>,
    ) -> Result<LayoutOutput> {
        self.arrange(cx, assigned, stretch, false)
    }

    pub(crate) fn arrange(
        &self,
        cx: &mut LayoutContext<'_, '_>,
        assigned: Size,
        stretch: Option<usize>,
        reverse: bool,
    ) -> Result<LayoutOutput> {
        let inset = add(self.padding.left, self.padding.right)?;
        let inner_width = assigned.width.saturating_sub(inset);
        let query = Query::Exact(inner_width);
        let mut cursor = if reverse {
            self.padding.bottom
        } else {
            self.padding.top
        };
        let mut content = Size {
            width: inset,
            height: self.padding.top,
        };
        let mut baseline = None;
        for index in 0..cx.child_count() {
            if index > 0 {
                cursor = add(cursor, self.gap)?;
            }
            let (child, margin) = cx.measure_stack_item(index, query)?;
            let (leading, trailing) = if reverse {
                (margin.bottom, margin.top)
            } else {
                (margin.top, margin.bottom)
            };
            cursor = add(cursor, leading.fixed_or_zero())?;
            let end = add(cursor, child.size.height)?;
            let y = if reverse {
                assigned.height - end
            } else {
                cursor
            };
            let available = margin.inner_width(inner_width)?;
            let mut size = child.size;
            if stretch == Some(index) {
                size.width = available;
            }
            let output = cx.layout_child(index, size)?;
            let x = add(self.padding.left, margin.left_offset(available, size.width))?;
            cx.place_child(index, Origin { x, y })?;
            if index == 0 {
                baseline = output.baseline.map(|value| add(value, y)).transpose()?;
            }
            let right = add(child.size.width, margin.right.fixed_or_zero())?
                .max(child.size.width)
                .max(output.content.width);
            content.width = content.width.max(add(add(x, right)?, self.padding.right)?);
            content.height = content
                .height
                .max(add(y, child.size.height.max(output.content.height))?);
            cursor = add(end, trailing.fixed_or_zero())?;
            if !reverse {
                content.height = content.height.max(cursor);
            }
        }
        content.height = add(content.height, self.padding.bottom)?;
        Ok(LayoutOutput {
            baseline: baseline.filter(|b| b.is_extent() && *b <= content.height),
            content,
        })
    }

    pub(crate) fn measure_stack(
        &self,
        cx: &mut MeasureContext<'_, '_>,
        query: Query,
        reverse: bool,
    ) -> Result<Metrics> {
        let child_query = self.child_query(query)?;
        let mut height = self.padding.top;
        let mut width = Pixels::default();
        let mut baseline = None;
        for index in 0..cx.child_count() {
            if index > 0 {
                height = add(height, self.gap)?;
            }
            let (child, margin) = cx.measure_stack_item(index, child_query)?;
            height = add(height, margin.top.fixed_or_zero())?;
            if index == 0 {
                baseline = if reverse {
                    // Distance from the bottom edge to the first child's baseline.
                    child
                        .baseline
                        .map(|value| {
                            add(
                                add(self.padding.bottom, margin.bottom.fixed_or_zero())?,
                                child.size.height - value,
                            )
                        })
                        .transpose()?
                } else {
                    child.baseline.map(|value| add(value, height)).transpose()?
                };
            }
            height = add(
                add(height, child.size.height)?,
                margin.bottom.fixed_or_zero(),
            )?;
            width = width.max(add(child.size.width, margin.horizontal()?)?);
        }
        let height = add(height, self.padding.bottom)?.max(Pixels::default());
        if reverse {
            baseline = baseline.map(|distance| height - distance);
        }
        Ok(Metrics {
            size: Size {
                width: query.resolve(add(width, add(self.padding.left, self.padding.right)?)?),
                height,
            },
            baseline: baseline.filter(|b| b.is_extent() && *b <= height),
        })
    }
}

impl LayoutNode for Column {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        self.measure_stack(cx, query, false)
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        self.layout_rows(cx, assigned, None)
    }
}
