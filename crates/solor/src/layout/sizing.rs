//! Conversion from declared box dimensions to child measurement constraints.
use super::super::column::Edges;
use super::{ContainingBlock, ItemQuery, LayoutError, Origin, Pixels, Query, Result, Size};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ComputedLength {
    Pixels(Pixels),
    Fraction(f32),
    Mixed(Pixels, f32),
}

impl ComputedLength {
    pub(crate) fn components(self) -> (Pixels, f32) {
        match self {
            Self::Pixels(value) => (value, 0.0),
            Self::Fraction(value) => (Pixels::default(), value),
            Self::Mixed(px, fraction) => (px, fraction),
        }
    }

    pub(crate) fn resolve(self, basis: Option<Pixels>) -> Option<Pixels> {
        match self {
            Self::Pixels(value) => Some(value),
            Self::Fraction(fraction) => basis.map(|value| value * fraction),
            Self::Mixed(px, fraction) => basis.map(|value| px + value * fraction),
        }
    }
}

pub(crate) fn border_extent(
    length: Option<ComputedLength>,
    basis: Option<Pixels>,
    extra: Pixels,
) -> Result<Option<Pixels>> {
    let Some(value) = length.and_then(|v| v.resolve(basis)) else {
        return Ok(None);
    };
    if !value.is_extent() {
        return Err(LayoutError::InvalidSize);
    }
    value
        .checked_add(extra)
        .map(Some)
        .ok_or(LayoutError::InvalidSize)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Dimension {
    Width,
    Height,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    FlexBasis,
}

impl Dimension {
    pub(crate) const ALL: [Self; 7] = [
        Self::Width,
        Self::Height,
        Self::MinWidth,
        Self::MaxWidth,
        Self::MinHeight,
        Self::MaxHeight,
        Self::FlexBasis,
    ];

    pub(crate) fn value(
        self,
        preferred: PreferredSize,
        sizing: Option<&ItemSizing>,
    ) -> Option<ComputedLength> {
        match self {
            Self::Width => preferred.width,
            Self::Height => preferred.height,
            Self::MinWidth => sizing?.width?.min,
            Self::MaxWidth => sizing?.width?.max,
            Self::MinHeight => sizing?.height?.min,
            Self::MaxHeight => sizing?.height?.max,
            Self::FlexBasis => Some(sizing?.basis.as_ref()?.length),
        }
    }

    pub(crate) fn value_mut<'a>(
        self,
        preferred: &'a mut PreferredSize,
        sizing: Option<&'a mut ItemSizing>,
    ) -> Option<&'a mut ComputedLength> {
        match self {
            Self::Width => &mut preferred.width,
            Self::Height => &mut preferred.height,
            Self::MinWidth => &mut sizing?.width.as_mut()?.min,
            Self::MaxWidth => &mut sizing?.width.as_mut()?.max,
            Self::MinHeight => &mut sizing?.height.as_mut()?.min,
            Self::MaxHeight => &mut sizing?.height.as_mut()?.max,
            Self::FlexBasis => return Some(&mut sizing?.basis.as_mut()?.length),
        }
        .as_mut()
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PreferredSize {
    pub(crate) width: Option<ComputedLength>,
    pub(crate) height: Option<ComputedLength>,
}

impl AxisLimits {
    // A direct query from a layout body is a hard constraint. Flow and stretch
    // offer the space left after margins, which an item's minimum may exceed.
    pub(super) fn constrain_width(
        &self,
        query: Query,
        basis: Option<Pixels>,
        preferred: Option<ComputedLength>,
        mode: ItemQuery,
        item_basis: ItemBasis,
    ) -> Result<Option<(Query, Pixels)>> {
        query.validate()?;
        let direct = matches!(mode, ItemQuery::Direct);
        let SizeRange { min, max } = self.resolve(basis)?;
        if (min.is_none() && max.is_none()) || (direct && matches!(query, Query::Exact(_))) {
            return Ok(None);
        }
        let mut min = min.unwrap_or_default();
        if direct && let Query::AtMost(available) = query {
            min = min.min(available);
        }
        let available = match query {
            Query::Natural => max,
            Query::AtMost(w) | Query::Exact(w) => Some(max.map_or(w, |limit| w.min(limit))),
        };
        let mut query = match available {
            Some(w) if w <= min => Query::Exact(min),
            Some(w) if matches!(query, Query::Exact(_)) => Query::Exact(w),
            Some(w) => Query::AtMost(w),
            None => Query::Natural,
        };
        if !matches!(query, Query::Exact(_))
            && !matches!(item_basis, ItemBasis::Width(_))
            && let Some(preferred) = border_extent(preferred, basis, self.extra)?
        {
            query = Query::Exact(query.resolve(preferred).max(min));
        }
        Ok(Some((query, min)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum ItemBasis {
    #[default]
    Preferred,
    Width(Option<Pixels>),
    Height(Option<Pixels>),
}

pub(crate) struct FlexBasis {
    pub(crate) length: ComputedLength,
    pub(crate) extra: Size,
}

impl FlexBasis {
    pub(super) fn resolve(&self, mode: ItemQuery, parent: ContainingBlock) -> Result<ItemBasis> {
        Ok(match mode {
            ItemQuery::Row(_) => ItemBasis::Width(border_extent(
                Some(self.length),
                parent.width,
                self.extra.width,
            )?),
            ItemQuery::Column(_, _) => ItemBasis::Height(border_extent(
                Some(self.length),
                parent.height,
                self.extra.height,
            )?),
            _ => ItemBasis::Preferred,
        })
    }
}

pub(crate) struct RelativeOffset {
    x: Option<ComputedLength>,
    y: Option<ComputedLength>,
}

impl RelativeOffset {
    pub(crate) fn from_edges(edges: Edges<Option<ComputedLength>>) -> Option<Self> {
        fn axis(
            start: Option<ComputedLength>,
            end: Option<ComputedLength>,
        ) -> Option<ComputedLength> {
            start.or_else(|| {
                end.map(|value| match value {
                    ComputedLength::Pixels(px) => ComputedLength::Pixels(px * -1.0),
                    ComputedLength::Fraction(f) => ComputedLength::Fraction(-f),
                    ComputedLength::Mixed(px, f) => ComputedLength::Mixed(px * -1.0, -f),
                })
            })
        }
        let x = axis(edges.left, edges.right);
        let y = axis(edges.top, edges.bottom);
        (x.is_some() || y.is_some()).then_some(Self { x, y })
    }

    pub(super) fn resolve(&self, parent: ContainingBlock) -> Origin {
        Origin {
            x: self
                .x
                .and_then(|v| v.resolve(parent.width))
                .unwrap_or_default(),
            y: self
                .y
                .and_then(|v| v.resolve(parent.height))
                .unwrap_or_default(),
        }
    }
}

pub(crate) enum Positioning {
    Flow,
    Relative(RelativeOffset),
    Absolute(Edges<Option<ComputedLength>>),
}

pub(crate) struct ItemSizing {
    pub(crate) position: Positioning,
    pub(crate) shrink_inset: Size,
    pub(crate) basis: Option<FlexBasis>,
    pub(crate) width: Option<AxisLimits>,
    pub(crate) height: Option<AxisLimits>,
}

#[derive(Clone, Copy)]
pub(crate) struct AxisLimits {
    pub(crate) min: Option<ComputedLength>,
    pub(crate) max: Option<ComputedLength>,
    pub(crate) extra: Pixels,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SizeRange {
    min: Option<Pixels>,
    max: Option<Pixels>,
}

impl AxisLimits {
    pub(crate) fn resolve(self, basis: Option<Pixels>) -> Result<SizeRange> {
        Ok(SizeRange {
            min: border_extent(self.min, basis, self.extra)?,
            max: border_extent(self.max, basis, self.extra)?,
        })
    }
}

impl SizeRange {
    pub(crate) fn lower(self) -> Pixels {
        self.min.unwrap_or_default()
    }

    pub(crate) fn upper(self) -> Option<Pixels> {
        self.max.map(|max| max.max(self.min.unwrap_or_default()))
    }

    pub(crate) fn fixed(self) -> Option<Pixels> {
        self.min
            .filter(|&min| self.max.is_some_and(|max| max <= min))
    }

    pub(crate) fn clamp(self, value: Pixels) -> Pixels {
        self.max
            .map_or(value, |max| max.min(value))
            .max(self.min.unwrap_or_default())
    }

    pub(crate) fn reaches_limit(self, value: Pixels) -> bool {
        self.min.is_some_and(|min| value <= min) || self.max.is_some_and(|max| value >= max)
    }
}

impl ItemSizing {
    pub(super) fn is_absolute(sizing: Option<&Self>) -> bool {
        sizing.is_some_and(|s| matches!(s.position, Positioning::Absolute(_)))
    }
}
