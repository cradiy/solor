//! Out-of-flow measurement and placement within the parent's padding box.

use super::super::column::Margin;
use super::sizing::ComputedLength;
use super::{
    Context, Entry, ItemBasis, LayoutError, MeasureInput, Origin, Pixels, Positioning, Query,
    Result, Scratch, Size, layout, measure,
};

struct Axis {
    extent: Pixels,
    start: Option<Pixels>,
    end: Option<Pixels>,
    before: Margin,
    after: Margin,
}

impl Axis {
    fn new(
        extent: Pixels,
        start: Option<ComputedLength>,
        end: Option<ComputedLength>,
        before: Margin,
        after: Margin,
    ) -> Result<Self> {
        let resolve = |value: Option<ComputedLength>| -> Result<Option<Pixels>> {
            let length = value.and_then(|value| value.resolve(Some(extent)));
            if length.is_some_and(|v| !v.get().is_finite()) {
                return Err(LayoutError::InvalidSize);
            }
            Ok(length)
        };
        Ok(Self {
            extent,
            start: resolve(start)?,
            end: resolve(end)?,
            before,
            after,
        })
    }

    fn available(&self) -> Result<Pixels> {
        let remaining = self.extent
            - self.start.unwrap_or_default()
            - self.end.unwrap_or_default()
            - self.before.fixed_or_zero()
            - self.after.fixed_or_zero();
        if !remaining.get().is_finite() {
            return Err(LayoutError::InvalidSize);
        }
        Ok(remaining.max(Pixels::default()))
    }

    fn anchored_at_both_ends(&self) -> bool {
        self.start.is_some() && self.end.is_some()
    }

    fn origin(&self, size: Pixels) -> Result<Pixels> {
        let mut margin = self.before.fixed_or_zero();
        if self.anchored_at_both_ends() && self.before == Margin::Auto {
            let free = self.available()?.saturating_sub(size);
            margin = if self.after == Margin::Auto {
                free / 2.0
            } else {
                free
            };
        }
        Ok(match (self.start, self.end) {
            (Some(start), _) => start + margin,
            (None, Some(end)) => self.extent - end - self.after.fixed_or_zero() - size,
            (None, None) => margin,
        })
    }
}

pub(super) fn place(
    entry: &mut Entry,
    access: &mut Context<'_, Entry>,
    (offset, parent): (Origin, Size),
    scratch: &mut Scratch,
    force: bool,
    parent_content: &mut Size,
) -> Result<()> {
    let sizing = entry
        .item_sizing
        .as_ref()
        .expect("absolute child has sizing");
    let Positioning::Absolute(edges) = sizing.position else {
        unreachable!("layout group contains only absolute children")
    };
    let margins = entry.body.margins(Some(parent.width))?;
    let x = Axis::new(
        parent.width,
        edges.left,
        edges.right,
        margins.left,
        margins.right,
    )?;
    let y = Axis::new(
        parent.height,
        edges.top,
        edges.bottom,
        margins.top,
        margins.bottom,
    )?;
    let limits = sizing
        .width
        .as_ref()
        .map(|width| width.resolve(Some(parent.width)))
        .transpose()?
        .unwrap_or_default();
    let query = if entry.item_style.preferred.width.is_none() {
        let available = limits.clamp(x.available()?);
        if x.anchored_at_both_ends() {
            Query::Exact(available)
        } else {
            Query::AtMost(available)
        }
    } else {
        Query::Natural
    };
    let item_basis = if entry.item_style.preferred.height.is_none() && y.anchored_at_both_ends() {
        ItemBasis::Height(Some(y.available()?))
    } else {
        ItemBasis::Preferred
    };
    let input = MeasureInput {
        query,
        containing_block: parent.into(),
        item_basis,
    };
    let mut size = measure(entry, access, input, force)?.size;
    let width = limits.clamp(size.width);
    if width != size.width {
        size = measure(
            entry,
            access,
            MeasureInput {
                query: Query::Exact(width),
                ..input
            },
            force,
        )?
        .size;
    }
    let origin = Origin {
        x: x.origin(size.width)?,
        y: y.origin(size.height)?,
    };
    let (slot, _) = layout(entry, access, size, scratch, force)?;
    let geometry = &mut scratch.draft[slot].geometry;
    geometry.origin = offset;
    geometry.shift_and_extend(origin, entry.item_style.overflow_clip, parent_content)
}
