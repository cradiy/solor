//! Shared content-box adapter for every retained element.
use crate::layout::{
    AxisLimits, ComputedLength, ContainingBlock, FlexBasis, ItemBasis, ItemSizing, LayoutContext,
    LayoutError, LayoutNode, LayoutOutput, Margin, Margins, MeasureContext, Metrics, Origin,
    Positioning, Query, RelativeOffset, Result, Size, border_extent,
};
use crate::length::UnitResolver;
use crate::style::{BoxSizing, Position};
use crate::{Edges, Length, Pixels, Style, px};

pub(crate) struct StyledBody<E> {
    pub content: E,
    pub(crate) bounds: BoxModel,
}

pub(crate) struct BoxModel {
    padding_box_insets: ContentInsets,
    padding_and_border: Size,
    origin: Origin,
    inset: Size,
    margins: Option<Box<Edges<Option<ComputedLength>>>>,
    box_sizing: BoxSizing,
    aspect_ratio: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContentInsets {
    pub(crate) origin: Origin,
    total: Size,
}

impl ContentInsets {
    pub(crate) fn content_size(self, border: Size) -> Size {
        Size {
            width: border.width.saturating_sub(self.total.width),
            height: border.height.saturating_sub(self.total.height),
        }
    }
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
    pub fn new(content: E, style: &Style, units: UnitResolver, gutter: Size) -> Result<Self> {
        Ok(Self {
            content,
            bounds: BoxModel::resolve(style, units, gutter)?,
        })
    }

    pub fn origin(&self) -> Origin {
        self.bounds.origin
    }
    pub fn content_size(&self, size: Size) -> Size {
        self.bounds.content_insets().content_size(size)
    }
    fn baseline(&self, baseline: Option<Pixels>) -> Result<Option<Pixels>> {
        baseline.map(|v| add(v, self.bounds.origin.y)).transpose()
    }
}

impl BoxModel {
    pub(crate) fn content_insets(&self) -> ContentInsets {
        ContentInsets {
            origin: self.origin,
            total: self.inset,
        }
    }

    pub(crate) fn resolve(style: &Style, units: UnitResolver, gutter: Size) -> Result<Self> {
        let aspect_ratio = style.layout.as_ref().and_then(|layout| layout.aspect_ratio);
        if aspect_ratio.is_some_and(|ratio| !ratio.is_finite() || ratio <= 0.0) {
            return Err(LayoutError::InvalidSize);
        }
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
        let b = style.border_widths(units)?;
        let border_width = add(b.left, b.right)?;
        let border_height = add(b.top, b.bottom)?;
        let padding_and_border = Size {
            width: add(add(p.left, p.right)?, border_width)?,
            height: add(add(p.top, p.bottom)?, border_height)?,
        };
        let padding_box_insets = ContentInsets {
            origin: Origin {
                x: b.left,
                y: b.top,
            },
            total: Size {
                width: add(border_width, gutter.width)?,
                height: add(border_height, gutter.height)?,
            },
        };
        Ok(Self {
            padding_box_insets,
            padding_and_border,
            margins,
            aspect_ratio,
            origin: Origin {
                x: add(p.left, b.left)?,
                y: add(p.top, b.top)?,
            },
            inset: Size {
                width: add(padding_and_border.width, gutter.width)?,
                height: add(padding_and_border.height, gutter.height)?,
            },
            box_sizing: style
                .layout
                .as_ref()
                .and_then(|layout| layout.box_sizing)
                .unwrap_or(BoxSizing::BorderBox),
        })
    }

    fn sizing_extra(&self, inset: Pixels) -> Pixels {
        match self.box_sizing {
            BoxSizing::BorderBox => Pixels::default(),
            BoxSizing::ContentBox => inset,
        }
    }

    pub(crate) fn item_sizing(
        &self,
        style: &Style,
        units: UnitResolver,
    ) -> Result<Option<ItemSizing>> {
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
        let width = bounds(layout.min_width, layout.max_width, self.inset.width)?;
        let height = bounds(layout.min_height, layout.max_height, self.inset.height)?;
        let basis = units
            .dimension(layout.flex_basis.unwrap_or_default())?
            .map(|length| FlexBasis {
                length,
                extra: Size {
                    width: self.sizing_extra(self.inset.width),
                    height: self.sizing_extra(self.inset.height),
                },
            });
        let shrink_inset = if layout.flex_shrink.unwrap_or_default() > 0.0 {
            self.inset
        } else {
            Size::default()
        };
        let insets = || map_edges(layout.inset, |v| units.offset(v.unwrap_or_default()));
        let position = match layout.position {
            Some(Position::Absolute) => Positioning::Absolute(insets()?),
            Some(Position::Relative) => RelativeOffset::from_edges(insets()?)
                .map_or(Positioning::Flow, Positioning::Relative),
            _ => Positioning::Flow,
        };
        Ok((!matches!(position, Positioning::Flow)
            || width.is_some()
            || height.is_some()
            || basis.is_some()
            || shrink_inset != Size::default())
        .then_some(ItemSizing {
            position,
            shrink_inset,
            width,
            height,
            basis,
        }))
    }
}

impl<E: LayoutNode> LayoutNode for StyledBody<E> {
    fn margins(&self, inline_width: Option<Pixels>) -> Result<Margins> {
        let Some(margins) = &self.bounds.margins else {
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
        let inner = |w: Pixels| w.saturating_sub(self.bounds.inset.width);
        let basis = cx.containing_block();
        let mut width = match cx.item_basis {
            ItemBasis::Width(width) => width,
            _ => border_extent(
                cx.preferred_size.width,
                basis.width,
                self.bounds.sizing_extra(self.bounds.inset.width),
            )?,
        };
        let range = cx.height_range;
        let mut height = match cx.item_basis {
            ItemBasis::Height(height) => height,
            _ => border_extent(
                cx.preferred_size.height,
                basis.height,
                self.bounds.sizing_extra(self.bounds.inset.height),
            )?,
        }
        .map(|h| range.clamp(h))
        .or_else(|| range.fixed());
        if let Some(ratio) = self.bounds.aspect_ratio {
            let used_width = match query {
                Query::Exact(w) => Some(w),
                _ => width.map(|w| query.resolve(w)),
            };
            let inset_x = self.bounds.sizing_extra(self.bounds.inset.width);
            let inset_y = self.bounds.sizing_extra(self.bounds.inset.height);
            match (used_width, height) {
                (Some(w), None) => {
                    let derived = add(w.saturating_sub(inset_x) / ratio, inset_y)?;
                    height = Some(range.clamp(derived));
                }
                (None, Some(h)) => {
                    width = Some(add(h.saturating_sub(inset_y) * ratio, inset_x)?);
                }
                _ => {}
            }
        }
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
                height: height.map(|h| h.saturating_sub(self.bounds.inset.height)),
            });
            self.content
                .measure(cx, content_query)?
                .validate_for(content_query)
        };
        let mut metrics = measure(cx, height)?;
        let height = match height {
            Some(h) => h,
            None => {
                let natural = add(metrics.size.height, self.bounds.inset.height)?;
                let used = range.clamp(natural);
                if range.reaches_limit(natural) && cx.child_count() > 0 {
                    metrics = measure(cx, Some(used))?;
                }
                used
            }
        };
        let width = match width {
            Some(w) => w,
            None => add(metrics.size.width, self.bounds.inset.width)?,
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
        cx.padding_box = (
            self.bounds.padding_box_insets.origin,
            self.bounds.padding_box_insets.content_size(assigned),
        );
        cx.set_child_containing_block(content_size.into());
        let out = self.content.layout(cx, content_size)?.validate()?;
        let origin = self.origin();
        if origin != Origin::default() {
            cx.offset_children(origin)?;
        }
        Ok(LayoutOutput {
            content: Size {
                width: add(out.content.width, self.bounds.padding_and_border.width)?,
                height: add(out.content.height, self.bounds.padding_and_border.height)?,
            },
            baseline: self
                .baseline(out.baseline)?
                .filter(|&b| b <= assigned.height),
        })
    }
}
