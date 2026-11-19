//! A mounted window routes against the last successfully submitted allocation.
use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
    rgb,
};
use solor_render_cpu::CpuWindow;

#[test]
fn rejected_resize_keeps_old_hit_geometry_until_retry_is_submitted() -> Result<()> {
    let clicks = Rc::new(Cell::new(0));
    let observed = clicks.clone();
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let visible = cx.signal(false);
            div()
                .flex_col()
                .child(div().w_full().h(px(10.0)).on_click(move |cx| {
                    observed.set(observed.get() + 1);
                    cx.update(visible, |open| *open = !*open)
                }))
                .child_when(visible, || div().w_full().h(px(10.0)).bg(rgb(0xff0000)))
        }),
    )?;
    let small = Size {
        width: px(32.0),
        height: px(20.0),
    };
    let large = Size {
        width: px(64.0),
        ..small
    };
    let click = |window: &mut CpuWindow, x| -> Result<bool> {
        let p = PointerSample {
            id: PointerId(1),
            kind: PointerKind::Mouse,
            position: Origin {
                x: px(x),
                y: px(5.0),
            },
            modifiers: Default::default(),
        };
        window.event(WindowEvent::Pointer(PointerInput::Down(
            p,
            PointerButton::Primary,
        )))?;
        window.event(WindowEvent::Pointer(PointerInput::Up(
            p,
            PointerButton::Primary,
        )))
    };
    window.frame(small, RasterScale::ONE)?;
    window.frame_submitted()?;
    assert!(!window.event(WindowEvent::FrameReady)?);

    window.frame(large, RasterScale::ONE)?;
    // The platform rejects this buffer. A failed preparation supersedes it, too.
    assert!(window.frame(large, RasterScale(0.0)).is_err());
    assert!(!click(&mut window, 48.0)?);
    assert_eq!(clicks.get(), 0);
    assert!(click(&mut window, 12.0)?);
    let retry = window.frame(large, RasterScale::ONE)?;
    let sample = (15 * retry.size.width as usize + 48) * 4;
    assert_eq!(&retry.rgba[sample..sample + 4], [255, 0, 0, 255]);
    assert!(!click(&mut window, 48.0)?);
    window.frame_submitted()?;
    assert!(click(&mut window, 48.0)?);
    assert_eq!(clicks.get(), 2);
    let closed = window.frame(large, RasterScale::ONE)?;
    assert_eq!(&closed.rgba[sample..sample + 4], [255; 4]);
    window.frame_submitted()?;
    assert!(!window.event(WindowEvent::FrameReady)?);
    Ok(())
}
