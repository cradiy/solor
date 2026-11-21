//! Composition of shared style declarations and delivery to custom elements.
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc};

use anyhow::Result;
use solor::{
    Edges, Element, Fill, Length, Style, StyleRefinement, Styled, Ui, component, div, element,
    layout::{self, Column},
    percent, px, rem, rgb,
    style::{BorderStyle, GridTrack, ObjectFit, Overflow, Position, TrackSize},
};

#[test]
fn local_overrides_preserve_unrelated_edges_axes_and_tracks() {
    let shared = StyleRefinement::default()
        .m(rem(1.0))
        .inset(percent(5.0))
        .relative()
        .border_color(rgb(0x123456))
        .border_dashed()
        .gap(px(4.25))
        .rounded(rem(0.5))
        .grid_cols([GridTrack::Size(TrackSize::Fraction(1.0))]);

    let mut badge = div()
        .with_style(&shared)
        .absolute()
        .mx_auto()
        .margin_top(px(-2.5))
        .right(px(1.25))
        .border_top_color(rgb(0xffffff))
        .gap_x(rem(2.0))
        .rounded_br(px(0.0))
        .overflow_hidden()
        .overflow_y(Overflow::Auto);
    let style = badge.style();
    let layout = style.layout.as_ref().unwrap();
    assert_eq!(layout.position, Some(Position::Absolute));
    assert_eq!(
        layout.margin,
        Edges {
            top: Some(px(-2.5).into()),
            right: Some(Length::Auto),
            bottom: Some(rem(1.0).into()),
            left: Some(Length::Auto),
        }
    );
    assert_eq!(layout.inset.top, Some(percent(5.0)));
    assert_eq!(layout.inset.right, Some(px(1.25).into()));
    assert_eq!(
        (layout.overflow_x, layout.overflow_y),
        (Some(Overflow::Hidden), Some(Overflow::Auto))
    );
    assert_eq!(
        layout.grid_columns,
        shared.layout.as_ref().unwrap().grid_columns
    );
    let paint = style.appearance.as_ref().unwrap();
    assert_eq!(paint.border_fill.top, Some(Fill::from(rgb(0xffffff))));
    assert_eq!(paint.border_fill.left, Some(Fill::from(rgb(0x123456))));
    assert_eq!(paint.border_style, Edges::all(Some(BorderStyle::Dashed)));
    assert_eq!(style.column_gap, Some(rem(2.0)));
    assert_eq!(style.row_gap, Some(px(4.25).into()));
    assert_eq!(style.corner_radii.top_left, Some(rem(0.5)));
    assert_eq!(style.corner_radii.bottom_right, Some(px(0.0).into()));

    // Applying a whole-box declaration later replaces earlier edge declarations.
    let mut restored = badge.with_style(&shared);
    let restored = restored.style();
    assert_eq!(
        restored.layout.as_ref().unwrap().margin,
        Edges::all(Some(rem(1.0).into()))
    );
    assert_eq!(
        restored.appearance.as_ref().unwrap().border_fill.top,
        Some(Fill::from(rgb(0x123456)))
    );
    assert_eq!(
        shared.layout.as_ref().unwrap().position,
        Some(Position::Relative)
    );
}

struct Capture {
    column: Column,
    received: Rc<RefCell<Option<Style>>>,
}
impl layout::LayoutNode for Capture {
    fn measure(
        &mut self,
        cx: &mut layout::MeasureContext<'_, '_>,
        query: layout::Query,
    ) -> layout::Result<layout::Metrics> {
        self.column.measure(cx, query)
    }
    fn layout(
        &mut self,
        cx: &mut layout::LayoutContext<'_, '_>,
        size: layout::Size,
    ) -> layout::Result<layout::LayoutOutput> {
        self.column.layout(cx, size)
    }
}
impl Element for Capture {
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        self.received.replace(Some(style.clone()));
        Ok(())
    }
}

#[test]
fn conditional_custom_element_receives_cascaded_text_and_local_box_declarations() -> Result<()> {
    let received = Rc::new(RefCell::new(None));
    let mut ui = Ui::new();
    let show = ui.context().signal(false);
    ui.mount(component(|_| {
        let received = received.clone();
        div()
            .flex_col()
            .relative()
            .m(px(3.0))
            .font_family("monospace")
            .font_weight(400)
            .line_clamp(NonZeroUsize::new(3).unwrap())
            .child_when(show, move || {
                element(Capture {
                    column: Column::new(px(0.0), px(0.0)),
                    received: received.clone(),
                })
                .absolute()
                .top(percent(10.0))
                .font_weight(650)
                .clear_line_clamp()
            })
    }))?;
    ui.flush()?;
    assert!(received.borrow().is_none());
    ui.context().update(show, |value| *value = true)?;
    ui.flush()?;
    let received = received.borrow();
    let style = received.as_ref().unwrap();
    let layout = style.layout.as_ref().unwrap();
    assert_eq!(layout.position, Some(Position::Absolute));
    assert_eq!(layout.inset.top, Some(percent(10.0)));
    assert_eq!(layout.margin, Edges::default());
    let font = style.text.attributes.as_ref().unwrap();
    assert_eq!(
        font.font_family,
        Some(solor::FontFamily::Named("monospace".into()))
    );
    assert_eq!(font.font_weight, Some(650));
    assert_eq!(font.line_clamp, Some(None));
    Ok(())
}

#[test]
fn object_fit_reaches_fixed_and_signal_bound_elements() -> Result<()> {
    let mut ui = Ui::new();
    let fit = ui.context().signal(ObjectFit::Contain);
    let captured = || {
        let style = Rc::new(RefCell::new(None));
        let node = element(Capture {
            column: Column::new(px(0.0), px(0.0)),
            received: Rc::clone(&style),
        });
        (node, style)
    };
    let (fixed, fixed_style) = captured();
    let (bound, bound_style) = captured();
    ui.mount(component(move |_| {
        div()
            .child(fixed.object_fit(ObjectFit::Fill))
            .child(bound.object_fit(fit))
    }))?;
    let read_fit = |style: &RefCell<Option<Style>>| {
        style
            .borrow()
            .as_ref()
            .unwrap()
            .appearance
            .as_ref()
            .unwrap()
            .object_fit
    };
    assert_eq!(read_fit(&bound_style), Some(ObjectFit::Contain));
    ui.context().set(fit, ObjectFit::Cover)?;
    ui.flush()?;
    assert_eq!(read_fit(&bound_style), Some(ObjectFit::Cover));
    assert_eq!(read_fit(&fixed_style), Some(ObjectFit::Fill));
    Ok(())
}
