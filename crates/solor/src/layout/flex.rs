//! Single-line placement along a container's selected main axis.
use super::engine::{ItemQuery, MeasuredItem};
use super::{
    ContainingBlock, LayoutContext, LayoutError, LayoutNode, LayoutOutput, Margin, Margins,
    MeasureContext, Metrics, Origin, Pixels, Query, Result, Size, px,
};
use crate::style::{Align, Distribution};
mod grow;
mod shrink;
use grow::Growth;
use shrink::{Shrink, ShrinkPlan};

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
    fn is_row(self) -> bool {
        matches!(self, Self::Row | Self::RowReverse)
    }

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
        let mut has_flexible_items = false;
        let mut ascent = None;
        let mut descent = None;
        let available = match query {
            Query::Exact(width) => Some(width),
            _ => None,
        };
        let mut items = LineItems::new(self, cx.child_count(), available, false);
        for index in 0..cx.child_count() {
            let item = items.item(index, |i, mode| cx.measure_item(i, query, mode))?;
            has_flexible_items |= item.style.is_flexible();
            let (child, margins) = (item.metrics, item.margins);
            let top = margins.top.fixed_or_zero();
            let bottom = margins.bottom.fixed_or_zero();
            if index == 0 {
                first = child.baseline.map(|b| {
                    (
                        b,
                        child.size.height,
                        margins,
                        item.style.alignment(self.align_items),
                    )
                });
            } else {
                size.width = add(size.width, self.gap)?;
            }
            size.width = add(size.width, add(child.size.width, margins.horizontal()?)?)?;
            size.height = size.height.max(add(add(top, child.size.height)?, bottom)?);
            if shares_row_baseline(item.style.alignment(self.align_items), margins) {
                let baseline = child.baseline.unwrap_or(child.size.height);
                include_max(&mut ascent, add(top, baseline)?);
                include_max(&mut descent, add(child.size.height - baseline, bottom)?);
            }
        }
        size.width = query.resolve(size.width.max(px(0.0)));
        if has_flexible_items && available.is_none() {
            // A shrink-wrapped row now has a used width. Measure its line height
            // with percentage bases and flexible sizes resolved at that width.
            cx.set_child_containing_block(ContainingBlock {
                width: Some(size.width),
                height: cx.definite_content_height(),
            });
            return self.measure_row(cx, Query::Exact(size.width));
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
        Ok(Metrics {
            size,
            baseline: baseline.filter(|b| b.is_extent() && *b <= size.height),
        })
    }

    fn measure_column(&self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let mut line = LineTotals::default();
        let mut width = px(0.0);
        let mut first = None;
        let mut has_flexible_items = false;
        let mut items = LineItems::new(self, cx.child_count(), cx.definite_content_height(), false);
        for index in 0..cx.child_count() {
            let item = items.item(index, |i, mode| cx.measure_item(i, query, mode))?;
            has_flexible_items |= item.style.is_flexible();
            let (child, margins) = (item.metrics, item.margins);
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
        if has_flexible_items && cx.definite_content_height().is_none() {
            cx.set_child_containing_block(ContainingBlock {
                width: match query {
                    Query::Exact(width) => Some(width),
                    _ => None,
                },
                height: Some(height),
            });
            return self.measure_column(cx, query);
        }
        let baseline = if let Some((local, extent, before)) = first {
            let spacing = line.spacing(add(height, line.occupied * -1.0)?, self.justify_content);
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

    fn layout_line(&self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let row = self.direction.is_row();
        let reverse = matches!(
            self.direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let main_size = if row { assigned.width } else { assigned.height };
        let query = Query::Exact(assigned.width);
        let mut items = LineItems::new(self, cx.child_count(), Some(main_size), true);
        let mut cursor = px(0.0);
        let mut content = Size::default();
        let mut first_baseline = None;
        let mut shared_baseline = None;
        let mut baseline_items = Vec::new();
        for index in 0..cx.child_count() {
            let item = items.item(index, |i, mode| cx.measure_item(i, query, mode))?;
            let (mut child, margins) = (item.metrics, item.margins);
            let align = item.style.alignment(self.align_items);
            if row
                && align == Align::Stretch
                && item.style.preferred.height.is_none()
                && let (Margin::Fixed(top), Margin::Fixed(bottom)) = (margins.top, margins.bottom)
            {
                let available = assigned.height.saturating_sub(add(top, bottom)?);
                child.size.height = item.stretch_height(available, assigned.height)?;
            }
            let (extent, leading, trailing) = self.direction.main_axis(child.size, margins);
            let spacing = items
                .plan
                .as_ref()
                .map_or_else(LineSpacing::default, |p| p.spacing);
            if index == 0 {
                cursor = spacing.start;
            } else {
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

#[derive(Clone, Copy)]
enum Allocation {
    Grow(f64),
    Shrink(ShrinkPlan),
}

struct LinePlan {
    allocation: Allocation,
    spacing: LineSpacing,
}

struct LineItems<'a> {
    flex: &'a FlexLayout,
    count: usize,
    available: Option<Pixels>,
    positioning: bool,
    plan: Option<LinePlan>,
}

impl<'a> LineItems<'a> {
    fn new(
        flex: &'a FlexLayout,
        count: usize,
        available: Option<Pixels>,
        positioning: bool,
    ) -> Self {
        Self {
            flex,
            count,
            available,
            positioning,
            plan: None,
        }
    }

    fn mode(&self, extent: Option<Pixels>) -> ItemQuery {
        if self.flex.direction.is_row() {
            ItemQuery::Row(extent)
        } else {
            ItemQuery::Column(self.flex.align_items, extent)
        }
    }

    fn item(
        &mut self,
        index: usize,
        mut measure: impl FnMut(usize, ItemQuery) -> Result<MeasuredItem>,
    ) -> Result<MeasuredItem> {
        let mode = self.mode(None);
        let item = measure(index, mode)?;
        let Some(available) = self.available else {
            return Ok(item);
        };
        let (base, before, after) = self
            .flex
            .direction
            .main_axis(item.metrics.size, item.margins);
        let needs_spacing = self.positioning
            && (!matches!(
                self.flex.justify_content,
                Distribution::Start | Distribution::Stretch
            ) || before == Margin::Auto
                || after == Margin::Auto);
        if self.plan.is_none() && (item.style.is_flexible() || needs_spacing) {
            let mut line = LineTotals::default();
            let mut growth = Growth::default();
            let mut shrink = Shrink::default();
            for i in 0..self.count {
                let measured;
                let child = if i == index {
                    &item
                } else {
                    measured = measure(i, mode)?;
                    &measured
                };
                let (extent, before, after) = self
                    .flex
                    .direction
                    .main_axis(child.metrics.size, child.margins);
                line.push(extent, before, after, self.flex.gap)?;
                growth.push(child.style.grow, extent, child.main_limits.upper());
                shrink.push(extent, child);
            }
            let free = add(available, line.occupied * -1.0)?;
            let (allocation, remaining) = if free < px(0.0) {
                let (plan, remaining) = shrink.resolve(free * -1.0);
                (Allocation::Shrink(plan), remaining)
            } else {
                let (unit, remaining) = growth.resolve(free);
                (Allocation::Grow(unit), remaining)
            };
            self.plan = Some(LinePlan {
                allocation,
                spacing: line.spacing(remaining, self.flex.justify_content),
            });
        }
        if let Some(plan) = &self.plan {
            let used = match plan.allocation {
                Allocation::Shrink(shrink) => shrink.extent(base, &item),
                Allocation::Grow(unit) if unit > 0.0 && item.style.grow > 0.0 => {
                    let grown = f64::from(base) + unit * f64::from(item.style.grow);
                    px(item
                        .main_limits
                        .upper()
                        .map_or(grown, |max| grown.min(f64::from(max)))
                        as f32)
                }
                Allocation::Grow(_) => base,
            };
            if !used.is_extent() {
                return Err(LayoutError::InvalidSize);
            }
            if used != base {
                return measure(index, self.mode(Some(used)));
            }
        }
        Ok(item)
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

    fn spacing(&self, free: Pixels, justify: Distribution) -> LineSpacing {
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
        spacing
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
