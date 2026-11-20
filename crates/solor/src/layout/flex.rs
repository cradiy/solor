//! Single-line placement along a container's selected main axis.
use super::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, Margin, Margins, MeasureContext, Metrics,
    Origin, Pixels, Query, Result, Size, px,
};
use crate::style::{Align, Distribution};

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

impl FlexDirection {
    fn main_axis(self, size: Size, margins: Margins) -> (Pixels, Margin, Margin) {
        match self {
            Self::Row => (size.width, margins.left, margins.right),
            Self::RowReverse => (size.width, margins.right, margins.left),
            Self::Column => (size.height, margins.top, margins.bottom),
            Self::ColumnReverse => (size.height, margins.bottom, margins.top),
        }
    }
}

/// Measures and places a single line of children in declaration order.
pub struct FlexLayout {
    /// Axis and starting edge.
    pub direction: FlexDirection,
    /// Space between adjacent items along the main axis.
    pub gap: Pixels,
    /// Distribution of space left on the main axis.
    pub justify_content: Distribution,
    /// Default cross-axis alignment for items in this line.
    pub align_items: Align,
}

fn add(a: Pixels, b: Pixels) -> Result<Pixels> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

fn shares_row_baseline(align: Align, margins: Margins) -> bool {
    align == Align::Baseline && margins.top != Margin::Auto && margins.bottom != Margin::Auto
}

fn include_max(maximum: &mut Option<Pixels>, value: Pixels) {
    *maximum = Some(maximum.map_or(value, |previous| previous.max(value)));
}

impl FlexLayout {
    fn measure_row(&self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let mut size = Size::default();
        let mut first = None;
        let mut ascent = None;
        let mut descent = None;
        for index in 0..cx.child_count() {
            let (child, margins, item) = cx.measure_row_item(index, query)?;
            let top = margins.top.fixed_or_zero();
            let bottom = margins.bottom.fixed_or_zero();
            if index == 0 {
                first = child.baseline.map(|b| {
                    (
                        b,
                        child.size.height,
                        margins,
                        item.alignment(self.align_items),
                    )
                });
            } else {
                size.width = add(size.width, self.gap)?;
            }
            size.width = add(size.width, add(child.size.width, margins.horizontal()?)?)?;
            size.height = size.height.max(add(add(top, child.size.height)?, bottom)?);
            if shares_row_baseline(item.alignment(self.align_items), margins) {
                let baseline = child.baseline.unwrap_or(child.size.height);
                include_max(&mut ascent, add(top, baseline)?);
                include_max(&mut descent, add(child.size.height - baseline, bottom)?);
            }
        }
        if let (Some(above), Some(below)) = (ascent, descent) {
            size.height = size.height.max(add(above, below)?);
        }
        size.height = cx.definite_content_height().unwrap_or(size.height);
        let baseline = if let Some(shared) = ascent {
            Some(shared)
        } else if let Some((local, height, margins, align)) = first {
            Some(add(
                local,
                cross_offset(size.height, height, margins.top, margins.bottom, align)?,
            )?)
        } else {
            None
        };
        size.width = query.resolve(size.width.max(px(0.0)));
        Ok(Metrics {
            size,
            baseline: baseline.filter(|b| b.is_extent() && *b <= size.height),
        })
    }

    fn measure_column(&self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let mut line = LineTotals::default();
        let mut width = px(0.0);
        let mut first = None;
        for index in 0..cx.child_count() {
            let (child, margins) = cx.measure_column_item(index, query, self.align_items)?;
            let (extent, before, after) = self.direction.main_axis(child.size, margins);
            line.push(extent, before, after, self.gap)?;
            width = width.max(add(child.size.width, margins.horizontal()?)?);
            if index == 0 {
                first = child.baseline.map(|local| (local, extent, before));
            }
        }
        let height = cx
            .definite_content_height()
            .unwrap_or(line.occupied.max(px(0.0)));
        let baseline = if let Some((local, extent, before)) = first {
            let spacing = line.spacing(height, self.justify_content)?;
            let start = add(spacing.start, spacing.margin(before))?;
            let y = if self.direction == FlexDirection::ColumnReverse {
                height - add(start, extent)?
            } else {
                start
            };
            Some(add(y, local)?)
        } else {
            None
        };
        Ok(Metrics {
            size: Size {
                width: query.resolve(width),
                height,
            },
            baseline: baseline.filter(|b| b.is_extent() && *b <= height),
        })
    }

    fn line_spacing(
        &self,
        cx: &mut LayoutContext<'_, '_>,
        query: Query,
        available: Pixels,
        column_align: Option<Align>,
    ) -> Result<LineSpacing> {
        let mut line = LineTotals::default();
        for i in 0..cx.child_count() {
            let (child, margins, _) = cx.measure_flex_item(i, query, column_align)?;
            let (extent, before, after) = self.direction.main_axis(child.size, margins);
            line.push(extent, before, after, self.gap)?;
        }
        line.spacing(available, self.justify_content)
    }

    fn layout_line(&self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let row = matches!(
            self.direction,
            FlexDirection::Row | FlexDirection::RowReverse
        );
        let reverse = matches!(
            self.direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let main_size = if row { assigned.width } else { assigned.height };
        let query = Query::Exact(assigned.width);
        let column_align = (!row).then_some(self.align_items);
        let mut spacing = match self.justify_content {
            Distribution::Start | Distribution::Stretch => None,
            _ => Some(self.line_spacing(cx, query, main_size, column_align)?),
        };
        let mut cursor = spacing.unwrap_or_default().start;
        let mut content = Size::default();
        let mut first_baseline = None;
        let mut shared_baseline = None;
        let mut baseline_items = Vec::new();
        for index in 0..cx.child_count() {
            let (mut child, margins, item) = cx.measure_flex_item(index, query, column_align)?;
            let align = item.alignment(self.align_items);
            if row
                && align == Align::Stretch
                && item.auto_height
                && let (Margin::Fixed(top), Margin::Fixed(bottom)) = (margins.top, margins.bottom)
            {
                child.size.height = assigned.height.saturating_sub(add(top, bottom)?);
            }
            let (extent, leading, trailing) = self.direction.main_axis(child.size, margins);
            // Start-aligned lines only need a totals pass when a main-axis margin is automatic.
            if spacing.is_none() && (leading == Margin::Auto || trailing == Margin::Auto) {
                spacing = Some(self.line_spacing(cx, query, main_size, column_align)?);
            }
            let spacing = spacing.unwrap_or_default();
            if index > 0 {
                cursor = add(cursor, add(self.gap, spacing.extra_gap)?)?;
            }
            cursor = add(cursor, spacing.margin(leading))?;
            let end = add(cursor, extent)?;
            let main = if reverse { main_size - end } else { cursor };
            let output = cx.layout_child(index, child.size)?;
            let x = if row {
                main
            } else {
                cross_offset(
                    assigned.width,
                    child.size.width,
                    margins.left,
                    margins.right,
                    align,
                )?
            };
            let right = if row {
                spacing.margin(margins.right)
            } else {
                margins.right.fixed_or_zero()
            };
            let bottom = if row {
                margins.bottom.fixed_or_zero()
            } else {
                spacing.margin(margins.bottom)
            };
            let width = add(child.size.width, right)?
                .max(child.size.width)
                .max(output.content.width);
            let height = add(child.size.height, bottom)?
                .max(child.size.height)
                .max(output.content.height);
            content.width = content.width.max(add(x, width)?);
            if row && shares_row_baseline(align, margins) {
                let local = output.baseline.unwrap_or(child.size.height);
                include_max(
                    &mut shared_baseline,
                    add(margins.top.fixed_or_zero(), local)?,
                );
                baseline_items.push((index, x, local, height));
            } else {
                let y = if row {
                    cross_offset(
                        assigned.height,
                        child.size.height,
                        margins.top,
                        margins.bottom,
                        align,
                    )?
                } else {
                    main
                };
                cx.place_child(index, Origin { x, y })?;
                content.height = content.height.max(add(y, height)?);
                if index == 0 {
                    first_baseline = output.baseline.map(|b| add(y, b)).transpose()?;
                }
            }
            cursor = add(end, spacing.margin(trailing))?;
        }
        if let Some(shared) = shared_baseline {
            // A child's final layout can report a different baseline from its preferred measurement.
            for (index, x, local, height) in baseline_items {
                let y = shared - local;
                cx.place_child(index, Origin { x, y })?;
                content.height = content.height.max(add(y, height)?);
            }
        }
        Ok(LayoutOutput {
            content,
            baseline: shared_baseline
                .or(first_baseline)
                .filter(|b| b.is_extent() && *b <= content.height),
        })
    }
}

#[derive(Default)]
struct LineTotals {
    count: usize,
    auto_edges: usize,
    occupied: Pixels,
}

impl LineTotals {
    fn push(&mut self, extent: Pixels, before: Margin, after: Margin, gap: Pixels) -> Result<()> {
        if self.count > 0 {
            self.occupied = add(self.occupied, gap)?;
        }
        self.occupied = add(
            add(add(self.occupied, before.fixed_or_zero())?, extent)?,
            after.fixed_or_zero(),
        )?;
        self.auto_edges += usize::from(before == Margin::Auto) + usize::from(after == Margin::Auto);
        self.count += 1;
        Ok(())
    }

    fn spacing(&self, available: Pixels, justify: Distribution) -> Result<LineSpacing> {
        let free = add(available, self.occupied * -1.0)?;
        let mut spacing = LineSpacing::default();
        if self.auto_edges > 0 && free > px(0.0) {
            spacing.auto_margin = free / self.auto_edges as f32;
        } else {
            let positive = free.max(px(0.0));
            (spacing.start, spacing.extra_gap) = match justify {
                Distribution::Start | Distribution::Stretch => (px(0.0), px(0.0)),
                Distribution::End => (free, px(0.0)),
                Distribution::Center => (free / 2.0, px(0.0)),
                Distribution::SpaceBetween if self.count > 1 => {
                    (px(0.0), positive / (self.count - 1) as f32)
                }
                Distribution::SpaceAround if self.count > 0 => {
                    let share = positive / self.count as f32;
                    (share / 2.0, share)
                }
                Distribution::SpaceEvenly => {
                    let share = positive / (self.count as f32 + 1.0);
                    (share, share)
                }
                _ => (px(0.0), px(0.0)),
            };
        }
        Ok(spacing)
    }
}

#[derive(Clone, Copy, Default)]
struct LineSpacing {
    start: Pixels,
    extra_gap: Pixels,
    auto_margin: Pixels,
}

impl LineSpacing {
    fn margin(self, margin: Margin) -> Pixels {
        match margin {
            Margin::Fixed(value) => value,
            Margin::Auto => self.auto_margin,
        }
    }
}

fn cross_offset(
    available: Pixels,
    size: Pixels,
    leading: Margin,
    trailing: Margin,
    align: Align,
) -> Result<Pixels> {
    let start = leading.fixed_or_zero();
    if trailing == Margin::Auto && leading != Margin::Auto {
        return Ok(start);
    }
    if leading == Margin::Auto {
        let free = available.saturating_sub(add(size, trailing.fixed_or_zero())?);
        return Ok(if trailing == Margin::Auto {
            free / 2.0
        } else {
            free
        });
    }
    match align {
        Align::Center | Align::End => {
            let end = available - add(size, trailing.fixed_or_zero())?;
            Ok(if align == Align::Center {
                start * 0.5 + end * 0.5
            } else {
                end
            })
        }
        _ => Ok(start),
    }
}

impl LayoutNode for FlexLayout {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        match self.direction {
            FlexDirection::Column | FlexDirection::ColumnReverse => self.measure_column(cx, query),
            FlexDirection::Row | FlexDirection::RowReverse => self.measure_row(cx, query),
        }
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        self.layout_line(cx, assigned)
    }
}
