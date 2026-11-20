//! Conversion from declared box dimensions to child measurement constraints.
use super::{LayoutError, Pixels, Query, Result};

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
    pub(crate) min: Option<ComputedLength>,
    pub(crate) max: Option<ComputedLength>,
    pub(crate) extra: Pixels,
}

impl WidthLimits {
    // A direct query from a layout body is a hard constraint. Flow and stretch
    // offer the space left after margins, which an item's minimum may exceed.
    pub(super) fn constrain(
        &self,
        query: Query,
        basis: Option<Pixels>,
        direct: bool,
    ) -> Result<Option<(Query, Pixels)>> {
        query.validate()?;
        let min = border_extent(self.min, basis, self.extra)?;
        let max = border_extent(self.max, basis, self.extra)?;
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
            && let Some(preferred) = border_extent(self.preferred, basis, self.extra)?
        {
            query = Query::Exact(query.resolve(preferred).max(min));
        }
        Ok(Some((query, min)))
    }
}
