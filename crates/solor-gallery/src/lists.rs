use crate::{INK, LINE, MUTED, label};
use solor::{
    Context, Div, IntoElement, ListScrollHandle, ListScrollPosition, Signal, Styled, component,
    div,
    layout::{Origin, Size},
    pointer::PointerEvent,
    px, rgb, text, uniform_list,
};

const COLORS: usize = 1 << 24;

pub(super) fn palette() -> impl IntoElement {
    component(|cx| {
        let (list, position) = ListScrollHandle::with_position_signal(cx);
        div()
            .flex_1()
            .items_stretch()
            .p(px(22.0))
            .gap(px(8.0))
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(label("RGB atlas", 33.0, 44.0).letter_spacing(px(-1.3)))
                    .child(label("16,777,216 colors", 13.0, 20.0).text_color(MUTED)),
            )
            .child(seek_bar(cx, list.clone(), position))
            .child(
                uniform_list(COLORS, px(40.0), color_row)
                    .scroll_handle(&list)
                    .flex_1(),
            )
    })
}

fn color_row(index: usize) -> Div {
    // Interleave the channel bits so neighboring swatches span the RGB cube.
    let channel = |channel: usize| -> u8 {
        (0..8).fold(0, |value, bit| {
            (value << 1) | ((index >> (bit * 3 + channel)) & 1) as u8
        })
    };
    let (r, g, b) = (channel(0), channel(1), channel(2));
    let color = u32::from_be_bytes([0, r, g, b]);
    div()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .w(px(56.0))
                .h(px(26.0))
                .rounded(px(6.0))
                .bg(rgb(color)),
        )
        .child(text(format!("#{color:06X}")).text_size(px(15.0)))
        .child(div().flex_1())
        .child(
            text(format!("rgb({r}, {g}, {b})"))
                .text_size(px(12.0))
                .text_color(MUTED),
        )
}

fn seek_bar(
    cx: &mut Context<'_>,
    list: ListScrollHandle,
    position: Signal<Option<ListScrollPosition>>,
) -> Div {
    let owner = cx.signal(None);
    let seek = move |event: &PointerEvent| -> anyhow::Result<()> {
        let width = f64::from(event.content_size().width);
        if width > 0.0 {
            list.scroll_to_percent(100.0 * f64::from(event.content_position().x) / width)?;
        }
        Ok(())
    };
    let press = seek.clone();
    let drag = seek.clone();
    div()
        .h(px(28.0))
        .px(px(8.0))
        .cursor_pointer()
        .paint(move |cx| {
            let Some(position) = *cx.read(position)? else {
                return Ok(());
            };
            let x = cx.size.width * (position.percent() / 100.0) as f32;
            let y = cx.size.height / 2.0;
            let origin = Origin {
                x: px(0.0),
                y: y - px(2.0),
            };
            let rail = Size {
                width: cx.size.width,
                height: px(4.0),
            };
            cx.fill_rect(origin, rail, LINE)?;
            cx.fill_rect(origin, Size { width: x, ..rail }, INK)?;
            cx.fill_circle(Origin { x, y }, px(6.0), INK)
        })
        .on_pointer_down(move |event, cx| {
            if event.is_primary_action() && cx.read(owner)?.is_none() {
                press(event)?;
                cx.capture_pointer();
                cx.set(owner, Some(event.pointer_id()))?;
            }
            Ok(())
        })
        .on_pointer_move(move |event, cx| {
            if *cx.read(owner)? == Some(event.pointer_id()) {
                drag(event)?;
            }
            Ok(())
        })
        .on_pointer_up(move |event, cx| {
            if event.is_primary_action() && *cx.read(owner)? == Some(event.pointer_id()) {
                seek(event)?;
                cx.set(owner, None)?;
            }
            Ok(())
        })
        .on_pointer_cancel(move |id, cx| {
            if *cx.read(owner)? == Some(id) {
                cx.set(owner, None)?;
            }
            Ok(())
        })
}
