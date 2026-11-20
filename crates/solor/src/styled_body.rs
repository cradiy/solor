//! Shared content-box adapter for every retained element.
use crate::layout::{
    AxisLimits, ComputedLength, ContainingBlock, LayoutContext, LayoutError, LayoutNode,
    LayoutOutput, Margin, Margins, MeasureContext, Metrics, Origin, Query, Result, Size,
    SizeLimits, WidthLimits, border_extent,
};
use crate::length::UnitResolver;
use crate::style::BoxSizing;
use crate::{Edges, Length, Pixels, Style, px};

pub(crate) struct StyledBody<E> {
    pub content: E,
    padding: Edges,
    margins: Option<Box<Edges<Option<ComputedLength>>>>,
    inset: Size,
    box_sizing: BoxSizing,
    width: Option<ComputedLength>,
    height: Option<ComputedLength>,
}

fn add(a: Pixels, b: Pixels) -> Result<Pixels> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

fn map_edges<T, U>(edges: Edges<T>, mut f: impl FnMut(T) -> Result<U>) -> Result<Edges<U>> {
    Ok(Edges {
        top: f(edges.top)?,
        right: f(edges.right)?,
        bottom: f(edges.bottom)?,
        left: f(edges.left)?,
    })
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
        let margins = style
            .layout
            .as_ref()
            .map(|layout| layout.margin)
            .filter(|edges| *edges != Edges::default())
            .map(|edges| map_edges(edges, |value| units.offset(value.unwrap_or(px(0.0).into()))))
            .transpose()?
            .map(Box::new);
        Ok(Self {
            margins,
            content,
            padding: p,
            box_sizing: style
                .layout
                .as_ref()
                .and_then(|layout| layout.box_sizing)
                .unwrap_or(BoxSizing::BorderBox),
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

    fn sizing_extra(&self, inset: Pixels) -> Pixels {
        match self.box_sizing {
            BoxSizing::BorderBox => Pixels::default(),
            BoxSizing::ContentBox => inset,
        }
    }

    pub(crate) fn size_limits(
        &self,
        style: &Style,
        units: UnitResolver,
    ) -> Result<Option<SizeLimits>> {
        let Some(layout) = &style.layout else {
            return Ok(None);
        };
        let bounds = |min: Option<Length>, max: Option<Length>, inset| -> Result<_> {
            let min = units.dimension(min.unwrap_or_default())?;
            let max = units.dimension(max.unwrap_or_default())?;
            Ok((min.is_some() || max.is_some()).then_some(AxisLimits {
                min,
                max,
                extra: self.sizing_extra(inset),
            }))
        };
        let width = bounds(layout.min_width, layout.max_width, self.inset.width)?.map(|bounds| {
            WidthLimits {
                preferred: self.width,
                bounds,
            }
        });
        let height = bounds(layout.min_height, layout.max_height, self.inset.height)?;
        Ok((width.is_some() || height.is_some()).then_some(SizeLimits { width, height }))
    }
}

impl<E: LayoutNode> LayoutNode for StyledBody<E> {
    fn margins(&self, inline_width: Option<Pixels>) -> Result<Margins> {
        let Some(margins) = &self.margins else {
            return Ok(Margins::default());
        };
        map_edges(**margins, |value| {
            let Some(length) = value else {
                return Ok(Margin::Auto);
            };
            let distance = length.resolve(inline_width).unwrap_or_default();
            if distance.get().is_finite() {
                Ok(Margin::Fixed(distance))
            } else {
                Err(LayoutError::InvalidSize)
            }
        })
    }
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let inner = |w: Pixels| w.saturating_sub(self.inset.width);
        let basis = cx.containing_block();
        let width = border_extent(self.width, basis.width, self.sizing_extra(self.inset.width))?;
        let range = cx.height_range;
        let height = border_extent(
            self.height,
            basis.height,
            self.sizing_extra(self.inset.height),
        )?
        .map(|h| range.clamp(h))
        .or_else(|| range.fixed());
        let content_query = match (query, width) {
            (_, Some(w)) => Query::Exact(inner(query.resolve(w))),
            (Query::Natural, None) => Query::Natural,
            (Query::AtMost(w), None) => Query::AtMost(inner(w)),
            (Query::Exact(w), None) => Query::Exact(inner(w)),
        };
        let mut measure = |cx: &mut MeasureContext<'_, '_>, height: Option<Pixels>| {
            cx.set_child_containing_block(ContainingBlock {
                width: match content_query {
                    Query::Exact(w) => Some(w),
                    _ => None,
                },
                height: height.map(|h| h.saturating_sub(self.inset.height)),
            });
            self.content
                .measure(cx, content_query)?
                .validate_for(content_query)
        };
        let mut metrics = measure(cx, height)?;
        let height = match height {
            Some(h) => h,
            None => {
                let natural = add(metrics.size.height, self.inset.height)?;
                let used = range.clamp(natural);
                if range.reaches_limit(natural) && cx.child_count() > 0 {
                    metrics = measure(cx, Some(used))?;
                }
                used
            }
        };
        let width = match width {
            Some(w) => w,
            None => add(metrics.size.width, self.inset.width)?,
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
            baseline: self
                .baseline(out.baseline)?
                .filter(|&b| b <= assigned.height),
        })
    }
}
