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

pub(crate) type MarginLengths = Edges<Option<ComputedLength>>;
const ZERO_MARGINS: MarginLengths = Edges::all(Some(ComputedLength::Pixels(px(0.0))));

pub(crate) struct BoxModel {
    padding: Edges,
    pub(crate) border: Edges,
    gutter: Size,
    margins: Option<Box<MarginLengths>>,
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
        self.bounds.content_insets().origin
    }
    pub fn content_size(&self, size: Size) -> Size {
        self.bounds.content_insets().content_size(size)
    }
    fn baseline(&self, baseline: Option<Pixels>) -> Result<Option<Pixels>> {
        baseline.map(|v| add(v, self.origin().y)).transpose()
    }
}

impl BoxModel {
    pub(crate) fn margin_lengths(&self) -> MarginLengths {
        self.margins.as_deref().copied().unwrap_or(ZERO_MARGINS)
    }

    pub(crate) fn set_margin_lengths(&mut self, values: MarginLengths) {
        if values == ZERO_MARGINS {
            self.margins = None;
        } else if let Some(current) = self.margins.as_deref_mut() {
            *current = values;
        } else {
            self.margins = Some(Box::new(values));
        }
    }

    fn border_size(&self) -> Size {
        Size {
            width: self.border.left + self.border.right,
            height: self.border.top + self.border.bottom,
        }
    }

    fn padding_box_insets(&self) -> ContentInsets {
        let size = self.border_size();
        ContentInsets {
            origin: Origin {
                x: self.border.left,
                y: self.border.top,
            },
            total: Size {
                width: size.width + self.gutter.width,
                height: size.height + self.gutter.height,
            },
        }
    }

    pub(crate) fn content_insets(&self) -> ContentInsets {
        let insets = self.padding_box_insets();
        ContentInsets {
            origin: Origin {
                x: insets.origin.x + self.padding.left,
                y: insets.origin.y + self.padding.top,
            },
            total: self.with_padding(insets.total),
        }
    }

    fn with_padding(&self, size: Size) -> Size {
        Size {
            width: self.padding.left + self.padding.right + size.width,
            height: self.padding.top + self.padding.bottom + size.height,
        }
    }

    pub(crate) fn edges(&self, kind: BoxEdges) -> Edges {
        match kind {
            BoxEdges::Border => self.border,
            BoxEdges::Padding => self.padding,
        }
    }

    pub(crate) fn set_edges(&mut self, kind: BoxEdges, value: Edges) -> Result<()> {
        let (padding, border) = match kind {
            BoxEdges::Border => (self.padding, value),
            BoxEdges::Padding => (value, self.border),
        };
        if [value.top, value.right, value.bottom, value.left]
            .iter()
            .any(|v| !v.is_extent())
        {
            return Err(LayoutError::InvalidSize);
        }
        let width = add(add(border.left, border.right)?, self.gutter.width)?;
        let height = add(add(border.top, border.bottom)?, self.gutter.height)?;
        add(add(padding.left, padding.right)?, width)?;
        add(add(padding.top, padding.bottom)?, height)?;
        self.border = border;
        self.padding = padding;
        Ok(())
    }

    pub(crate) fn resolve(style: &Style, units: UnitResolver, gutter: Size) -> Result<Self> {
        let aspect_ratio = style.layout.as_ref().and_then(|layout| layout.aspect_ratio);
        if aspect_ratio.is_some_and(|ratio| !ratio.is_finite() || ratio <= 0.0) {
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
        let mut bounds = Self {
            border: style.border_widths(units)?,
            padding: Edges::default(),
            gutter,
            margins,
            aspect_ratio,
            box_sizing: style
                .layout
                .as_ref()
                .and_then(|layout| layout.box_sizing)
                .unwrap_or(BoxSizing::BorderBox),
        };
        bounds.set_edges(BoxEdges::Padding, style.padding)?;
        Ok(bounds)
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
        reserve_shrink: bool,
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
        let inset = self.content_insets().total;
        let width = bounds(layout.min_width, layout.max_width, inset.width)?;
        let height = bounds(layout.min_height, layout.max_height, inset.height)?;
        let basis = units
            .dimension(layout.flex_basis.unwrap_or_default())?
            .map(|length| FlexBasis {
                length,
                extra: Size {
                    width: self.sizing_extra(inset.width),
                    height: self.sizing_extra(inset.height),
                },
            });
        let shrink_inset = if reserve_shrink || layout.flex_shrink.unwrap_or_default() > 0.0 {
            inset
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
            || reserve_shrink
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
            let distance = length.resolve_at(inline_width.unwrap_or_default());
            if distance.get().is_finite() {
                Ok(Margin::Fixed(distance))
            } else {
                Err(LayoutError::InvalidSize)
            }
        })
    }
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let inset = self.bounds.content_insets().total;
        let inner = |w: Pixels| w.saturating_sub(inset.width);
        let basis = cx.containing_block();
        let mut width = match cx.item_basis {
            ItemBasis::Width(width) => width,
            _ => border_extent(
                cx.preferred_size.width,
                basis.width,
                self.bounds.sizing_extra(inset.width),
            )?,
        };
        let range = cx.height_range;
        let mut height = match cx.item_basis {
            ItemBasis::Height(height) => height,
            _ => border_extent(
                cx.preferred_size.height,
                basis.height,
                self.bounds.sizing_extra(inset.height),
            )?,
        }
        .map(|h| range.clamp(h))
        .or_else(|| range.fixed());
        if let Some(ratio) = self.bounds.aspect_ratio {
            let used_width = match query {
                Query::Exact(w) => Some(w),
                _ => width.map(|w| query.resolve(w)),
            };
            let inset_x = self.bounds.sizing_extra(inset.width);
            let inset_y = self.bounds.sizing_extra(inset.height);
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
                height: height.map(|h| h.saturating_sub(inset.height)),
            });
            self.content
                .measure(cx, content_query)?
                .validate_for(content_query)
        };
        let mut metrics = measure(cx, height)?;
        let height = match height {
            Some(h) => h,
            None => {
                let natural = add(metrics.size.height, inset.height)?;
                let used = range.clamp(natural);
                if range.reaches_limit(natural) && cx.child_count() > 0 {
                    metrics = measure(cx, Some(used))?;
                }
                used
            }
        };
        let width = match width {
            Some(w) => w,
            None => add(metrics.size.width, inset.width)?,
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
        let insets = self.bounds.padding_box_insets();
        cx.padding_box = (insets.origin, insets.content_size(assigned));
        cx.set_child_containing_block(content_size.into());
        let out = self.content.layout(cx, content_size)?.validate()?;
        let origin = self.origin();
        if origin != Origin::default() {
            cx.offset_children(origin)?;
        }
        let outer = self.bounds.with_padding(self.bounds.border_size());
        Ok(LayoutOutput {
            content: Size {
                width: add(out.content.width, outer.width)?,
                height: add(out.content.height, outer.height)?,
            },
            baseline: self
                .baseline(out.baseline)?
                .filter(|&b| b <= assigned.height),
        })
    }
}

pub(crate) type ApplyMargin =
    fn(&mut crate::layout::Scene, crate::NodeId, MarginLengths) -> Result<()>;

pub(crate) fn apply_margin<E: LayoutNode>(
    scene: &mut crate::layout::Scene,
    id: crate::NodeId,
    values: MarginLengths,
) -> Result<()> {
    if scene.body::<StyledBody<E>>(id)?.bounds.margin_lengths() != values {
        scene.edit_layout::<StyledBody<E>>(id, |body| body.bounds.set_margin_lengths(values))?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BoxEdges {
    Padding,
    Border,
}

impl BoxEdges {
    pub(crate) const ALL: [Self; 2] = [Self::Padding, Self::Border];

    pub(crate) fn property(self) -> crate::TransitionProperty {
        match self {
            Self::Padding => crate::TransitionProperty::Padding,
            Self::Border => crate::TransitionProperty::BorderWidth,
        }
    }
}

pub(crate) type ApplyBoxEdges =
    fn(&mut crate::layout::Scene, crate::NodeId, BoxEdges, Edges) -> Result<Option<ContentInsets>>;

pub(crate) fn apply_box_edges<E: LayoutNode>(
    scene: &mut crate::layout::Scene,
    id: crate::NodeId,
    kind: BoxEdges,
    value: Edges,
) -> Result<Option<ContentInsets>> {
    if scene.body::<StyledBody<E>>(id)?.bounds.edges(kind) == value {
        return Ok(None);
    }
    let mut updated = Ok(());
    scene.edit_layout::<StyledBody<E>>(id, |body| updated = body.bounds.set_edges(kind, value))?;
    updated?;
    let bounds = &scene.body::<StyledBody<E>>(id)?.bounds;
    let insets = bounds.content_insets();
    let extra = Size {
        width: bounds.sizing_extra(insets.total.width),
        height: bounds.sizing_extra(insets.total.height),
    };
    scene.set_item_insets(id, insets.total, extra)?;
    Ok(Some(insets))
}
