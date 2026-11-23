//! Sequential focus follows the live order of each view.
use anyhow::Result;
use solor::{
    Styled, Ui, component, div,
    keyboard::{KeyEvent, KeyState, Modifiers, NamedKey},
    layout::Size,
    px, uniform_list,
};

fn tab(state: KeyState, shift: bool) -> KeyEvent {
    KeyEvent {
        state,
        key: NamedKey::Tab.into(),
        modifiers: Modifiers {
            shift,
            ..Default::default()
        },
    }
}

#[test]
fn navigation_wraps_in_tree_order_and_skips_hidden_and_removed_subtrees() -> Result<()> {
    let mut ui = Ui::new();
    let shown = ui.context().signal(true);
    let mounted = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .focusable()
            .child(div().visible_when(shown).child(div().focusable()))
            .child_when(mounted, || div().focusable())
    }))?;
    ui.flush()?;
    let root = ui.root(view)?;
    let mut children = ui.children(view, root)?;
    let group = children.next().unwrap();
    let last = children.next().unwrap();
    drop(children);
    let nested = ui.children(view, group)?.next().unwrap();
    let other = ui.mount(component(|_| div().focusable()))?;
    let other_root = ui.root(other)?;
    ui.focus(other, other_root)?;

    for (shift, expected) in [(true, last), (false, root), (false, nested), (true, root)] {
        assert!(ui.dispatch_key(view, &tab(KeyState::Down, shift))?);
        assert_eq!(ui.focused(view)?, Some(expected));
    }
    ui.focus(view, last)?;
    assert_eq!(ui.focus_previous(view)?, Some(nested));
    ui.context().set(shown, false)?;
    ui.flush()?;
    assert_eq!(ui.focus_next(view)?, Some(root));
    assert_eq!(ui.focus_next(view)?, Some(last));
    ui.context().set(mounted, false)?;
    ui.flush()?;
    assert_eq!(ui.focus_previous(view)?, Some(root));
    ui.context().set(shown, true)?;
    ui.flush()?;
    assert_eq!(ui.focus_next(view)?, Some(nested));
    assert_eq!(ui.focused(other)?, Some(other_root));
    let empty = ui.mount(component(|_| div()))?;
    assert!(!ui.dispatch_key(empty, &tab(KeyState::Down, true))?);
    assert_eq!(ui.focused(empty)?, None);
    Ok(())
}

#[test]
fn tab_indices_order_the_view_and_resume_from_focus_after_a_change() -> Result<()> {
    let mut ui = Ui::new();
    let priority = ui.context().signal(-1);
    let view = ui.mount(component(|_| {
        div()
            .focusable()
            .child(div().tab_stop(false).tab_index(-2))
            .child(div().tab_index(priority).child(div().tab_index(0)))
            .child(div().tab_index(2))
    }))?;
    let root = ui.root(view)?;
    let ranked = ui.children(view, root)?.nth(1).unwrap();
    let nested = ui.children(view, ranked)?.next().unwrap();
    let last = ui.children(view, root)?.last().unwrap();

    assert_eq!(ui.focus_next(view)?, Some(ranked));
    assert_eq!(ui.focus_next(view)?, Some(root));
    assert_eq!(ui.focus_next(view)?, Some(nested));
    assert_eq!(ui.focus_next(view)?, Some(last));
    assert_eq!(ui.focus_next(view)?, Some(ranked));
    ui.context().set(priority, 1)?;
    assert_eq!(ui.focus_previous(view)?, Some(nested));
    assert_eq!(ui.focus_previous(view)?, Some(root));
    assert_eq!(ui.focus_previous(view)?, Some(last));
    assert_eq!(ui.focus_previous(view)?, Some(ranked));
    Ok(())
}

#[test]
fn key_callbacks_can_keep_tab_and_modified_tabs_and_releases_do_not_navigate() -> Result<()> {
    let mut ui = Ui::new();
    let capture = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .focusable()
            .on_key_down(move |_, cx| {
                if *cx.read(capture)? {
                    cx.prevent_default();
                }
                Ok(())
            })
            .child(div().focusable())
    }))?;
    let root = ui.root(view)?;
    let child = ui.children(view, root)?.next().unwrap();
    assert!(ui.dispatch_key(view, &tab(KeyState::Down, false))?);
    assert!(ui.dispatch_key(view, &tab(KeyState::Down, false))?);
    assert_eq!(ui.focused(view)?, Some(root));
    ui.context().set(capture, false)?;
    for modifiers in [
        Modifiers {
            control: true,
            ..Default::default()
        },
        Modifiers {
            alt: true,
            ..Default::default()
        },
        Modifiers {
            meta: true,
            ..Default::default()
        },
    ] {
        assert!(ui.dispatch_key(
            view,
            &KeyEvent {
                modifiers,
                ..tab(KeyState::Down, false)
            }
        )?);
        assert_eq!(ui.focused(view)?, Some(root));
    }
    ui.dispatch_key(view, &tab(KeyState::Down, false))?;
    assert_eq!(ui.focused(view)?, Some(child));
    assert!(!ui.dispatch_key(view, &tab(KeyState::Up, false))?);
    assert_eq!(ui.focused(view)?, Some(child));
    Ok(())
}

#[test]
fn reordered_list_rows_keep_focus_and_change_the_next_tab_stop() -> Result<()> {
    let mut ui = Ui::new();
    let items = ui.context().signal_vec(0..3);
    let view = ui.mount(component(|_| {
        uniform_list(items, px(20.), |_| div().focusable())
    }))?;
    let size = Size {
        width: px(60.),
        height: px(80.),
    };
    ui.flush()?;
    ui.layout(view, size)?;
    let rows: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(ui.focus_next(view)?, Some(rows[0]));
    items.move_item(2, 1, &mut ui.context())?;
    ui.flush()?;
    ui.layout(view, size)?;
    assert_eq!(ui.focused(view)?, Some(rows[0]));
    assert_eq!(ui.focus_next(view)?, Some(rows[2]));
    assert_eq!(ui.focus_next(view)?, Some(rows[1]));
    Ok(())
}

#[test]
fn focus_indicator_releases_paint_dependencies_on_blur_and_hide() -> Result<()> {
    use solor::rgb;

    let mut ui = Ui::new();
    let shown = ui.context().signal(true);
    let accent = ui.context().signal(rgb(0x264ddd));
    let view = ui.mount(component(|_| {
        div()
            .visible_when(shown)
            .child(div().focus_indicator(move |cx| {
                cx.fill(cx.size, *cx.read(accent)?);
                Ok(())
            }))
            .child(div().focusable())
    }))?;
    let target = ui.children(view, ui.root(view)?)?.next().unwrap();
    let frame = |ui: &mut Ui| -> Result<()> {
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(40.),
                height: px(20.),
            },
        )?;
        ui.record_frame(view)?;
        Ok(())
    };
    frame(&mut ui)?;
    ui.focus(view, target)?;
    frame(&mut ui)?;
    ui.focus(view, target)?;
    assert!(ui.next_redraw(view).is_none());
    ui.context().set(accent, rgb(0x823011))?;
    ui.flush()?;
    let mut dirty = Vec::new();
    ui.collect_repaints(view, &mut dirty)?;
    assert_eq!(dirty, [target]);
    ui.blur(view)?;
    frame(&mut ui)?;
    ui.context().set(accent, rgb(0xffdd12))?;
    ui.flush()?;
    assert!(ui.next_redraw(view).is_none());
    ui.focus(view, target)?;
    ui.context().set(shown, false)?;
    frame(&mut ui)?;
    ui.context().set(shown, true)?;
    frame(&mut ui)?;
    assert_eq!(ui.focused(view)?, None);
    assert!(ui.record_frame(view)?.draws().is_empty());
    Ok(())
}

#[test]
fn tab_resumes_from_pointer_focus_and_reads_the_current_stop_policy() -> Result<()> {
    use solor::{
        layout::Origin,
        pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    };

    let mut ui = Ui::new();
    let enabled = ui.context().signal(true);
    let view = ui.mount(component(move |_| {
        div()
            .flex_row()
            .child(div().size(px(10.)).tab_stop(enabled))
            .child(div().size(px(10.)).tab_stop(false).focusable())
            .child(div().size(px(10.)).tab_stop(enabled))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(30.),
            height: px(10.),
        },
    )?;
    ui.accept_pointer_frame(ui.prepare_pointer_frame(view)?)?;
    let nodes: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    let mouse = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(15.),
            y: px(5.),
        },
        modifiers: Default::default(),
    };

    ui.dispatch_pointer(view, PointerInput::Down(mouse, PointerButton::Primary))?;
    ui.dispatch_pointer(view, PointerInput::Up(mouse, PointerButton::Primary))?;
    assert_eq!(ui.focused(view)?, Some(nodes[1]));
    assert_eq!(ui.focus_next(view)?, Some(nodes[2]));
    assert_eq!(ui.focus_previous(view)?, Some(nodes[0]));
    ui.focus(view, nodes[1])?;
    assert_eq!(ui.focus_previous(view)?, Some(nodes[0]));

    ui.context().set(enabled, false)?;
    assert!(!ui.dispatch_key(view, &tab(KeyState::Down, false))?);
    assert_eq!(ui.focused(view)?, Some(nodes[0]));
    ui.context().set(enabled, true)?;
    assert!(ui.dispatch_key(view, &tab(KeyState::Down, true))?);
    assert_eq!(ui.focused(view)?, Some(nodes[2]));
    Ok(())
}
