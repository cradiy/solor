//! A mounted window routes against the last successfully submitted allocation.
use std::time::Duration;
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
        window
            .event(WindowEvent::Pointer(PointerInput::Up(
                p,
                PointerButton::Primary,
            )))
            .map(|response| response.redraw)
    };
    window.frame(small, RasterScale::ONE)?;
    window.frame_submitted()?;
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );

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
    assert!(
        !window
            .event(WindowEvent::FrameReady(Duration::ZERO))?
            .redraw
    );
    Ok(())
}

#[test]
fn changing_only_an_ancestor_clip_repairs_old_pixels_and_publishes_hits_on_submission() -> Result<()>
{
    use solor::raster::Clip;
    use solor::style::PointerEvents;
    let clicks = Rc::new(Cell::new(0));
    let seen = clicks.clone();
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let narrow = cx.signal(false);
            div()
                .items_stretch()
                .child(
                    div()
                        .flex_row()
                        .child(
                            div().flex_1().h(px(10.0)).overflow_hidden().child(
                                div()
                                    .min_w(px(80.0))
                                    .h(px(10.0))
                                    .bg(rgb(0xaa2211))
                                    .on_click(move |_| {
                                        seen.set(seen.get() + 1);
                                        Ok(())
                                    }),
                            ),
                        )
                        .child_when(narrow, || {
                            div().w(px(32.0)).pointer_events(PointerEvents::None)
                        }),
                )
                .child(
                    div()
                        .h(px(10.0))
                        .on_click(move |cx| cx.update(narrow, |n| *n = !*n)),
                )
        }),
    )?;
    let click = |window: &mut CpuWindow, x, y| -> Result<()> {
        let p = PointerSample {
            id: PointerId(1),
            kind: PointerKind::Mouse,
            position: Origin { x: px(x), y: px(y) },
            modifiers: Default::default(),
        };
        window.event(WindowEvent::Pointer(PointerInput::Down(
            p,
            PointerButton::Primary,
        )))?;
        window.event(WindowEvent::Pointer(PointerInput::Up(
            p,
            PointerButton::Primary,
        )))?;
        Ok(())
    };
    let size = Size {
        width: px(64.0),
        height: px(24.0),
    };
    let sample = 4 * (5 * 64 + 48);
    let wide = window.frame(size, RasterScale::ONE)?;
    assert_eq!(&wide.rgba[sample..sample + 4], [0xaa, 0x22, 0x11, 255]);
    window.frame_submitted()?;
    click(&mut window, 4.0, 15.0)?;
    let narrow = window.frame(size, RasterScale::ONE)?;
    assert_eq!(
        narrow.damage.bounds(),
        Some(Clip {
            left: 0,
            top: 0,
            right: 64,
            bottom: 10
        })
    );
    assert_eq!(&narrow.rgba[sample..sample + 4], [255; 4]);
    click(&mut window, 48.0, 5.0)?;
    assert_eq!(clicks.get(), 1);
    // Retry a discarded submission and compare the full repair with its pending image.
    let retry = window.frame(size, RasterScale::ONE)?;
    assert_eq!(&retry.rgba[sample..sample + 4], [255; 4]);
    window.frame_submitted()?;
    click(&mut window, 48.0, 5.0)?;
    click(&mut window, 16.0, 5.0)?;
    assert_eq!(clicks.get(), 2);
    assert!(window.frame(size, RasterScale::ONE)?.damage.is_empty());
    window.frame_submitted()?;

    let resized = window.frame(
        Size {
            width: px(48.0),
            ..size
        },
        RasterScale::ONE,
    )?;
    assert_eq!(&resized.rgba[8 * 4..9 * 4], [0xaa, 0x22, 0x11, 255]);
    assert_eq!(&resized.rgba[24 * 4..25 * 4], [255; 4]);
    click(&mut window, 24.0, 5.0)?;
    assert_eq!(clicks.get(), 3);
    window.frame_submitted()?;
    click(&mut window, 24.0, 5.0)?;
    assert_eq!(clicks.get(), 3);
    Ok(())
}

#[test]
fn window_keys_follow_focus_and_retire_with_their_controls() -> Result<()> {
    use solor::keyboard::{KeyEvent, KeyState, Modifiers, NamedKey};

    let transitions = Rc::new(Cell::new(0));
    let observed = transitions.clone();
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let shown = cx.signal(true);
            let attached = cx.signal(true);
            div()
                .flex_row()
                .child(
                    div()
                        .focusable()
                        .visible_when(shown)
                        .size(px(20.))
                        .on_key_down(move |event, cx| {
                            assert_eq!(event.key, NamedKey::Enter.into());
                            assert!(event.modifiers.shift);
                            cx.set(shown, false)
                        }),
                )
                .child_when(attached, move || {
                    let observed = observed.clone();
                    div().focusable().size(px(20.)).on_key_up(move |_, cx| {
                        observed.set(observed.get() + 1);
                        cx.set(attached, false)
                    })
                })
        }),
    )?;
    let submit = |window: &mut CpuWindow| -> Result<()> {
        window.frame(
            Size {
                width: px(60.),
                height: px(24.),
            },
            RasterScale::ONE,
        )?;
        window.frame_submitted()
    };
    let press = |window: &mut CpuWindow, x| -> Result<()> {
        let pointer = PointerSample {
            id: PointerId(1),
            kind: PointerKind::Mouse,
            position: Origin {
                x: px(x),
                y: px(2.),
            },
            modifiers: Modifiers::default(),
        };
        window.event(WindowEvent::Pointer(PointerInput::Down(
            pointer,
            PointerButton::Primary,
        )))?;
        window.event(WindowEvent::Pointer(PointerInput::Up(
            pointer,
            PointerButton::Primary,
        )))?;
        Ok(())
    };
    let key = |window: &mut CpuWindow, state| -> Result<bool> {
        Ok(window
            .event(WindowEvent::Key(KeyEvent {
                key: NamedKey::Enter.into(),
                state,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            }))?
            .redraw)
    };

    submit(&mut window)?;
    assert!(!key(&mut window, KeyState::Down)?);
    press(&mut window, 2.)?;
    press(&mut window, 22.)?;
    assert!(!key(&mut window, KeyState::Down)?);
    assert!(key(&mut window, KeyState::Up)?);
    submit(&mut window)?;
    assert!(!key(&mut window, KeyState::Up)?);
    assert_eq!(transitions.get(), 1);
    window.event(WindowEvent::Key(KeyEvent {
        key: NamedKey::Tab.into(),
        state: KeyState::Down,
        modifiers: Modifiers::default(),
    }))?;
    assert!(key(&mut window, KeyState::Down)?);
    submit(&mut window)?;
    assert!(!key(&mut window, KeyState::Down)?);
    Ok(())
}
