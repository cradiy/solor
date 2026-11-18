//! Shared content-box adapter for every retained element.
use crate::layout::{
    ContainingBlock, LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics,
    Origin, Query, Result, Size,
};
use crate::length::{ComputedLength, UnitResolver};
use crate::{Edges, Pixels, Style};

pub(crate) struct StyledBody<E> {
    pub content: E,
    padding: Edges,
    inset: Size,
    width: Option<ComputedLength>,
    height: Option<ComputedLength>,
}

fn add(a: Pixels, b: Pixels) -> Result<Pixels> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

impl<E> StyledBody<E> {
    pub fn new(content: E, style: &Style, units: UnitResolver) -> Result<Self> {
        let p = style.padding;
        if [p.top, p.right, p.bottom, p.left]
            .iter()
            .any(|v| !v.is_extent())
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(Self {
            content,
            padding: p,
            width: units.dimension(style.width)?,
            height: units.dimension(style.height)?,
            inset: Size {
                width: add(p.left, p.right)?,
                height: add(p.top, p.bottom)?,
            },
        })
    }
    pub fn origin(&self) -> Origin {
        Origin {
            x: self.padding.left,
            y: self.padding.top,
        }
    }
    pub fn content_size(&self, size: Size) -> Size {
        Size {
            width: size.width.saturating_sub(self.inset.width),
            height: size.height.saturating_sub(self.inset.height),
        }
    }
    fn baseline(&self, baseline: Option<Pixels>) -> Result<Option<Pixels>> {
        baseline.map(|v| add(v, self.padding.top)).transpose()
    }
}

impl<E: LayoutNode> LayoutNode for StyledBody<E> {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let inner = |w: Pixels| w.saturating_sub(self.inset.width);
        let basis = cx.containing_block();
        let width = self.width.and_then(|v| v.resolve(basis.width));
        let height = self.height.and_then(|v| v.resolve(basis.height));
        if width
            .into_iter()
            .chain(height)
            .any(|extent| !extent.is_extent())
        {
            return Err(LayoutError::InvalidSize);
        }
        let content_query = match (query, width) {
            (_, Some(w)) => Query::Exact(inner(query.resolve(w))),
            (Query::Natural, None) => Query::Natural,
            (Query::AtMost(w), None) => Query::AtMost(inner(w)),
            (Query::Exact(w), None) => Query::Exact(inner(w)),
        };
        cx.set_child_containing_block(ContainingBlock {
            width: match content_query {
                Query::Exact(w) => Some(w),
                _ => None,
            },
            height: height.map(|h| h.saturating_sub(self.inset.height)),
        });
        let metrics = self
            .content
            .measure(cx, content_query)?
            .validate_for(content_query)?;
        let width = match width {
            Some(w) => w,
            None => add(metrics.size.width, self.inset.width)?,
        };
        let height = match height {
            Some(h) => h,
            None => add(metrics.size.height, self.inset.height)?,
        };
        Ok(Metrics {
            size: Size {
                width: query.resolve(width),
                height,
            },
            baseline: self.baseline(metrics.baseline)?.filter(|&v| v <= height),
        })
    }
    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let content_size = self.content_size(assigned);
        cx.set_child_containing_block(content_size.into());
        let out = self.content.layout(cx, content_size)?.validate()?;
        let origin = self.origin();
        if origin != Origin::default() {
            cx.offset_children(origin)?;
        }
        Ok(LayoutOutput {
            content: Size {
                width: add(out.content.width, self.inset.width)?,
                height: add(out.content.height, self.inset.height)?,
            },
            baseline: self.baseline(out.baseline)?,
        })
    }
}
