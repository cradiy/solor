//! Conversion from declared box dimensions to child measurement constraints.
use super::{ContainingBlock, ItemQuery, LayoutError, Pixels, Query, Result, Size};

#[derive(Clone, Copy, Debug)]
pub(crate) enum ComputedLength {
    Pixels(Pixels),
    Fraction(f32),
}

impl ComputedLength {
    pub(crate) fn resolve(self, basis: Option<Pixels>) -> Option<Pixels> {
        match self {
            Self::Pixels(value) => Some(value),
            Self::Fraction(fraction) => basis.map(|value| value * fraction),
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

pub(crate) struct WidthLimits {
    pub(crate) preferred: Option<ComputedLength>,
    pub(crate) bounds: AxisLimits,
}

impl WidthLimits {
    // A direct query from a layout body is a hard constraint. Flow and stretch
    // offer the space left after margins, which an item's minimum may exceed.
    pub(super) fn constrain(
        &self,
        query: Query,
        basis: Option<Pixels>,
        mode: ItemQuery,
        item_basis: ItemBasis,
    ) -> Result<Option<(Query, Pixels)>> {
        query.validate()?;
        let direct = matches!(mode, ItemQuery::Direct);
        let SizeRange { min, max } = self.bounds.resolve(basis)?;
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
            && let Some(preferred) = border_extent(self.preferred, basis, self.bounds.extra)?
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

pub(crate) struct ItemSizing {
    pub(crate) shrink_inset: Size,
    pub(crate) basis: Option<FlexBasis>,
    pub(crate) width: Option<WidthLimits>,
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
