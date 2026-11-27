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
        repeat: false,
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
fn nested_tab_groups_stay_together_when_the_outer_order_changes() -> Result<()> {
    let mut ui = Ui::new();
    let position = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .child(
                div()
                    .tab_group(position)
                    .tab_index(2)
                    .child(div().tab_index(1))
                    .child(div().tab_group(-1).child(div().tab_index(9))),
            )
            .child(div().tab_group(0).child(div().tab_index(-10)))
            .child(div().focusable())
    }))?;
    let root = ui.root(view)?;
    let children: Vec<_> = ui.children(view, root)?.collect();
    let [first_group, second_group, tail] = children[..] else {
        panic!("expected two groups and a trailing stop");
    };
    let first = ui.children(view, first_group)?.next().unwrap();
    let subgroup = ui.children(view, first_group)?.nth(1).unwrap();
    let nested = ui.children(view, subgroup)?.next().unwrap();
    let second = ui.children(view, second_group)?.next().unwrap();

    for expected in [nested, first, first_group, second, tail, nested] {
        assert_eq!(ui.focus_next(view)?, Some(expected));
    }
    ui.context().set(position, 1)?;
    assert_eq!(ui.focused(view)?, Some(nested));
    for expected in [tail, second, first_group, first, nested] {
        assert_eq!(ui.focus_previous(view)?, Some(expected));
    }
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

#[test]
fn search_shortcuts_switch_the_active_input_and_end_its_composition() -> Result<()> {
    use solor::{
        FocusHandle,
        keyboard::{Key, KeyBinding, Modifier},
        text::{Preedit, TextInputEvent, TextInputState},
    };

    struct Find;
    let mut ui = Ui::new();
    let editor = FocusHandle::new();
    let search = FocusHandle::new();
    let document = ui.context().signal(TextInputState::new("draft"));
    let query = ui.context().signal(TextInputState::new(""));
    let view = ui.mount(component(|_| {
        div()
            .on_action(move |_: &Find, cx| cx.focus(search))
            .child(div().text_input(document).track_focus(editor).auto_focus())
            .child(
                div()
                    .text_input(query)
                    .track_focus(search)
                    .on_key_down(move |event, cx| {
                        if event.key == Key::Escape {
                            cx.prevent_default();
                            cx.focus(editor)?;
                        }
                        Ok(())
                    }),
            )
    }))?;
    ui.bind_keys([KeyBinding::new(Key::F.with(Modifier::Ctrl), Find)]);
    let mut key = tab(KeyState::Down, false);
    key.key = Key::F;
    key.modifiers.control = true;
    assert!(ui.dispatch_key(view, &key)?);
    assert!(ui.dispatch_text_input(view, TextInputEvent::Commit("needle".into()))?);
    assert!(ui.dispatch_text_input(
        view,
        TextInputEvent::Preedit(Preedit {
            text: "x".into(),
            selection: None,
        })
    )?);
    let session = ui.text_input_session(view)?;
    key.key = Key::Escape;
    key.modifiers.control = false;
    assert!(ui.dispatch_key(view, &key)?);
    assert_ne!(ui.text_input_session(view)?, session);
    assert_eq!(ui.focused_text_input(view)?.unwrap().text(), "draft");
    assert_eq!(ui.context().read(query)?.text(), "needle");
    assert!(ui.context().read(query)?.preedit().is_none());
    Ok(())
}

#[test]
fn focus_requests_resolve_only_visible_live_targets_in_the_callback_view() -> Result<()> {
    use solor::FocusHandle;

    let mut ui = Ui::new();
    let target = FocusHandle::new();
    let shown = ui.context().signal(false);
    let mounted = ui.context().signal(true);
    let view = ui.mount(component(|_| {
        div()
            .auto_focus()
            .on_click(move |cx| cx.focus(target))
            .child_when(mounted, move || {
                div().track_focus(target).visible_when(shown)
            })
    }))?;
    let foreign = ui.mount(component(|_| {
        div().auto_focus().on_click(move |cx| cx.focus(target))
    }))?;
    ui.flush()?;
    let root = ui.root(view)?;
    ui.click(view, root)?;
    assert_eq!(ui.focused(view)?, Some(root));
    ui.context().set(shown, true)?;
    ui.flush()?;
    ui.click(foreign, ui.root(foreign)?)?;
    assert_eq!(ui.focused(view)?, Some(root));
    assert_eq!(ui.focused(foreign)?, Some(ui.root(foreign)?));
    ui.click(view, root)?;
    assert_eq!(ui.focused(view)?, ui.children(view, root)?.next());
    ui.context().set(mounted, false)?;
    ui.flush()?;
    ui.focus(view, root)?;
    ui.click(view, root)?;
    assert_eq!(ui.focused(view)?, Some(root));
    Ok(())
}

#[test]
fn a_menu_action_on_the_list_anchors_to_the_presented_focused_row() -> Result<()> {
    use solor::{
        keyboard::{Key, KeyBinding},
        layout::Origin,
        platform::SurfaceRect,
    };
    use std::{cell::Cell, rc::Rc};

    struct OpenMenu;
    let mut ui = Ui::new();
    let anchor = Rc::new(Cell::new(None));
    let view = ui.mount(component({
        let anchor = anchor.clone();
        move |_| {
            div()
                .overflow_y_scroll()
                .children((0..4).map(|i| {
                    div()
                        .w_full()
                        .min_h(px(10.5))
                        .focusable()
                        .z_index(if i == 2 { -1 } else { 0 })
                }))
                .on_action(move |_: &OpenMenu, cx| {
                    anchor.set(cx.focused_bounds());
                    Ok(())
                })
        }
    }))?;
    let key = Key::Named(NamedKey::ContextMenu);
    ui.bind_keys([KeyBinding::new(key.clone(), OpenMenu)]);
    let press = KeyEvent {
        key,
        state: KeyState::Down,
        repeat: false,
        modifiers: Modifiers::default(),
    };
    let root = ui.root(view)?;
    let row = ui.children(view, root)?.nth(2).unwrap();
    ui.focus(view, row)?;
    let query = |ui: &mut Ui| -> Result<_> {
        assert!(ui.dispatch_key(view, &press)?);
        Ok(anchor.take())
    };
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(60.),
            height: px(30.),
        },
    )?;
    let initial = ui.record_frame(view)?.into_pointer_frame();
    assert_eq!(query(&mut ui)?, None);
    ui.accept_pointer_frame(initial)?;
    let rect = |y| Some(SurfaceRect::new(px(0.), px(y), px(60.), px(10.5)));
    assert_eq!(query(&mut ui)?, rect(21.));

    ui.set_scroll_offset(
        view,
        root,
        Origin {
            x: px(0.),
            y: px(5.25),
        },
    )?;
    let scrolled = ui.record_frame(view)?.into_pointer_frame();
    assert_eq!(query(&mut ui)?, rect(21.));
    ui.accept_pointer_frame(scrolled)?;
    assert_eq!(query(&mut ui)?, rect(15.75));
    Ok(())
}
