//! Typed shortcuts dispatch application actions through the focused view.
use anyhow::Result;
use solor::{
    Ui, component, div,
    keyboard::{Key, KeyBinding, KeyEvent, KeyState, Modifier, Modifiers},
};

#[derive(PartialEq)]
struct Select(u8);
struct Save;

#[test]
fn direct_actions_follow_current_focus_and_deliver_the_supplied_value() -> Result<()> {
    let mut ui = Ui::new();
    let received = ui.context().signal(None);
    let view = ui.mount(component(|_| {
        div()
            .on_action(move |choice: &Select, cx| cx.set(received, Some(("root", choice.0))))
            .child(
                div()
                    .on_action(move |_: &Save, cx| cx.set(received, Some(("save", 0))))
                    .on_action(move |choice: &Select, cx| {
                        cx.set(received, Some(("editor", choice.0)))
                    })
                    .child(div().focusable()),
            )
            .child(div().focusable())
    }))?;
    assert!(!ui.is_action_available(view, &Select(1))?);
    assert!(!ui.dispatch_action(view, &Select(1))?);
    ui.focus_next(view)?;
    assert!(ui.is_action_available(view, &Save)?);
    assert_eq!(*ui.context().read(received)?, None);
    assert!(ui.dispatch_action(view, &Select(2))?);
    assert_eq!(*ui.context().read(received)?, Some(("editor", 2)));
    ui.focus_next(view)?;
    assert!(!ui.is_action_available(view, &Save)?);
    assert!(ui.is_action_available(view, &Select(3))?);
    assert!(ui.dispatch_action(view, &Select(3))?);
    assert_eq!(*ui.context().read(received)?, Some(("root", 3)));
    assert!(!ui.dispatch_action(view, &Save)?);
    Ok(())
}

#[test]
fn shortcut_resolves_against_focus_and_uses_the_latest_binding_value() -> Result<()> {
    let mut ui = Ui::new();
    let selected = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .on_action(move |choice: &Select, cx| cx.set(selected, choice.0))
            .child(div().focusable())
            .child(
                div()
                    .focusable()
                    .on_action(move |_: &Save, cx| cx.set(selected, 99)),
            )
    }))?;
    let shortcut = Key::S.with(Modifier::Ctrl).with(Modifier::Shift);
    let alternate = Key::Space.with(Modifier::Alt);
    ui.bind_keys([
        KeyBinding::new(alternate.clone(), Select(1)),
        KeyBinding::new(shortcut.clone(), Save),
        KeyBinding::new(shortcut.clone(), Select(1)),
    ]);
    let mut event = KeyEvent {
        key: 'S'.into(),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        state: KeyState::Down,
        repeat: false,
    };
    ui.focus_next(view)?;
    assert_eq!(ui.shortcut_for_action(view, &Select(1))?, Some(&shortcut));
    assert_eq!(shortcut.to_string(), "Ctrl+Shift+S");
    assert!(!ui.dispatch_key(view, &event)?);
    event.modifiers.shift = true;
    assert!(ui.dispatch_key(view, &event)?);
    assert_eq!(*ui.context().read(selected)?, 1);

    ui.bind_keys([KeyBinding::new(shortcut.clone(), Select(2))]);
    assert_eq!(ui.shortcut_for_action(view, &Select(1))?, Some(&alternate));
    assert_eq!(ui.shortcut_for_action(view, &Select(2))?, Some(&shortcut));
    assert!(ui.dispatch_key(view, &event)?);
    assert_eq!(*ui.context().read(selected)?, 2);
    ui.focus_next(view)?;
    assert_eq!(ui.shortcut_for_action(view, &Select(2))?, None);
    assert!(ui.dispatch_key(view, &event)?);
    assert_eq!(*ui.context().read(selected)?, 99);
    Ok(())
}

#[test]
fn reloading_bindings_moves_the_shortcut_and_releases_tab_for_navigation() -> Result<()> {
    let mut ui = Ui::new();
    let calls = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .key_context("Editor")
            .on_action(move |_: &Select, cx| cx.update(calls, |n| *n += 1))
            .child(div().focusable())
            .child(div().focusable())
    }))?;
    let first = ui.focus_next(view)?;
    let tab = KeyEvent {
        key: Key::Tab,
        modifiers: Modifiers::default(),
        state: KeyState::Down,
        repeat: false,
    };
    ui.bind_keys([KeyBinding::new(Key::Tab, Select(1)).in_context("Editor")]);
    ui.dispatch_key(view, &tab)?;
    assert_eq!(*ui.context().read(calls)?, 1);
    assert_eq!(ui.focused(view)?, first);

    ui.set_key_bindings([KeyBinding::new(Key::F2, Select(1)).in_context("Editor")]);
    assert_eq!(
        ui.shortcut_for_action(view, &Select(1))?,
        Some(&Key::F2.into())
    );
    ui.dispatch_key(view, &tab)?;
    let second = ui.children(view, ui.root(view)?)?.nth(1).unwrap();
    assert_eq!(ui.focused(view)?, Some(second));
    ui.dispatch_key(
        view,
        &KeyEvent {
            key: Key::F2,
            ..tab
        },
    )?;
    assert_eq!(*ui.context().read(calls)?, 2);
    Ok(())
}

#[test]
fn focused_key_handler_can_consume_a_bound_press_before_action_dispatch() -> Result<()> {
    let mut ui = Ui::new();
    let value = ui.context().signal(0);
    ui.bind_keys([KeyBinding::new(Key::Enter, Select(7))]);
    let view = ui.mount(component(|_| {
        div()
            .on_action(move |choice: &Select, cx| cx.set(value, choice.0))
            .child(div().focusable().on_key_down(move |event, cx| {
                if matches!(event.key, Key::Space | Key::Enter) {
                    cx.prevent_default();
                    cx.set(value, 3)?;
                }
                Ok(())
            }))
    }))?;
    ui.focus_next(view)?;
    assert!(ui.dispatch_key(
        view,
        &KeyEvent {
            key: Key::Enter,
            modifiers: Default::default(),
            state: KeyState::Down,
            repeat: false,
        }
    )?);
    assert_eq!(*ui.context().read(value)?, 3);
    Ok(())
}

#[test]
fn one_handler_resolves_the_same_shortcut_across_nested_and_sibling_regions() -> Result<()> {
    let mut ui = Ui::new();
    let selected = ui.context().signal(0);
    let view = ui.mount(component(|_| {
        div()
            .on_action(move |choice: &Select, cx| cx.set(selected, choice.0))
            .child(
                div()
                    .key_context("Atlas")
                    .child(div().key_context("Palette").child(div().focusable()))
                    .child(div().focusable()),
            )
            .child(div().focusable())
    }))?;
    ui.bind_keys([
        KeyBinding::new(Key::Home, Select(1)).in_context("Palette"),
        KeyBinding::new(Key::Home, Select(2)).in_context("Atlas"),
        KeyBinding::new(Key::Home, Select(3)),
        KeyBinding::new(Key::Home, Select(4)).in_context("Atlas"),
    ]);
    let home = KeyEvent {
        key: Key::Home,
        modifiers: Default::default(),
        state: KeyState::Down,
        repeat: false,
    };
    for expected in [1, 4, 3, 1] {
        ui.focus_next(view)?;
        for value in [1, 2, 3, 4] {
            assert_eq!(
                ui.shortcut_for_action(view, &Select(value))?.is_some(),
                value == expected
            );
        }
        assert!(ui.dispatch_key(view, &home)?);
        assert_eq!(*ui.context().read(selected)?, expected);
    }
    Ok(())
}
