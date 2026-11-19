//! Single-line placement along a container's selected main axis.
use super::{
    Column, LayoutContext, LayoutError, LayoutNode, LayoutOutput, Margin, Margins, MeasureContext,
    Metrics, Origin, Pixels, Query, Result, Size, px,
};

/// Main axis and placement order for a container's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    /// Place children from left to right.
    Row,
    /// Place children from right to left.
    RowReverse,
    /// Place children from top to bottom.
    Column,
    /// Place children from bottom to top.
    ColumnReverse,
}

/// Measures and places a single line of children in declaration order.
pub struct FlexLayout {
    /// Axis and starting edge.
    pub direction: FlexDirection,
    /// Space between adjacent items along the main axis.
    pub gap: Pixels,
}

fn add(a: Pixels, b: Pixels) -> Result<Pixels> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

#[derive(Default)]
struct RowMetrics {
    width: Pixels,
    height: Pixels,
    baseline: Option<Pixels>,
    auto_edges: usize,
}

impl FlexLayout {
    fn measure_row(
        &self,
        count: usize,
        mut measure: impl FnMut(usize) -> Result<(Metrics, Margins)>,
    ) -> Result<RowMetrics> {
        let mut row = RowMetrics::default();
        for index in 0..count {
            let (child, margins) = measure(index)?;
            if index == 0 {
                row.baseline = child
                    .baseline
                    .map(|b| add(b, margins.top.fixed_or_zero()))
                    .transpose()?;
            } else {
                row.width = add(row.width, self.gap)?;
            }
            row.width = add(row.width, add(child.size.width, margins.horizontal()?)?)?;
            let height = add(
                add(margins.top.fixed_or_zero(), child.size.height)?,
                margins.bottom.fixed_or_zero(),
            )?;
            row.height = row.height.max(height);
            row.auto_edges += usize::from(margins.left == Margin::Auto)
                + usize::from(margins.right == Margin::Auto);
        }
        Ok(row)
    }

    fn layout_row(
        &self,
        cx: &mut LayoutContext<'_, '_>,
        assigned: Size,
        reverse: bool,
    ) -> Result<LayoutOutput> {
        let query = Query::Exact(assigned.width);
        let count = cx.child_count();
        let mut auto_share = None;
        let mut cursor = px(0.0);
        let mut content = Size::default();
        let mut baseline = None;
        for index in 0..count {
            let (child, margins) = cx.measure_stack_item(index, query)?;
            if auto_share.is_none()
                && (margins.left == Margin::Auto || margins.right == Margin::Auto)
            {
                // Space can be distributed only after accounting for every item and gap.
                let row = self.measure_row(count, |i| cx.measure_stack_item(i, query))?;
                let free = assigned.width.saturating_sub(row.width);
                auto_share = Some(free / row.auto_edges as f32);
            }
            let used = |edge: Margin| match edge {
                Margin::Fixed(value) => value,
                Margin::Auto => auto_share.unwrap_or_default(),
            };
            if index > 0 {
                cursor = add(cursor, self.gap)?;
            }
            let (leading, trailing) = if reverse {
                (margins.right, margins.left)
            } else {
                (margins.left, margins.right)
            };
            cursor = add(cursor, used(leading))?;
            let end = add(cursor, child.size.width)?;
            let x = if reverse {
                assigned.width - end
            } else {
                cursor
            };
            let y = match margins.top {
                Margin::Fixed(top) => top,
                Margin::Auto => {
                    let free = assigned
                        .height
                        .saturating_sub(add(child.size.height, margins.bottom.fixed_or_zero())?);
                    if margins.bottom == Margin::Auto {
                        free / 2.0
                    } else {
                        free
                    }
                }
            };
            let output = cx.layout_child(index, child.size)?;
            cx.place_child(index, Origin { x, y })?;
            if index == 0 {
                baseline = output.baseline.map(|b| add(y, b)).transpose()?;
            }
            let right = add(child.size.width, used(margins.right))?
                .max(child.size.width)
                .max(output.content.width);
            let bottom = add(child.size.height, margins.bottom.fixed_or_zero())?
                .max(child.size.height)
                .max(output.content.height);
            content.width = content.width.max(add(x, right)?);
            content.height = content.height.max(add(y, bottom)?);
            cursor = add(end, used(trailing))?;
        }
        Ok(LayoutOutput {
            content,
            baseline: baseline.filter(|b| b.is_extent() && *b <= content.height),
        })
    }
}

impl LayoutNode for FlexLayout {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        match self.direction {
            FlexDirection::Column | FlexDirection::ColumnReverse => Column::new(px(0.0), self.gap)
                .measure_stack(cx, query, self.direction == FlexDirection::ColumnReverse),
            FlexDirection::Row | FlexDirection::RowReverse => {
                let row =
                    self.measure_row(cx.child_count(), |i| cx.measure_stack_item(i, query))?;
                Ok(Metrics {
                    size: Size {
                        width: query.resolve(row.width.max(px(0.0))),
                        height: row.height,
                    },
                    baseline: row.baseline.filter(|b| b.is_extent() && *b <= row.height),
                })
            }
        }
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        match self.direction {
            FlexDirection::Column | FlexDirection::ColumnReverse => Column::new(px(0.0), self.gap)
                .arrange(
                    cx,
                    assigned,
                    None,
                    self.direction == FlexDirection::ColumnReverse,
                ),
            FlexDirection::Row | FlexDirection::RowReverse => {
                self.layout_row(cx, assigned, self.direction == FlexDirection::RowReverse)
            }
        }
    }
}
