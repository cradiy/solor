//! Selection and composition in an application-owned text buffer.
use anyhow::Result;
use solor::{
    Ui, component, div,
    text::{Preedit, TextInputEvent, TextInputState, TextSelection},
};

#[test]
fn surrounding_deletion_preserves_the_selected_text_across_composition() -> Result<()> {
    let mut input = TextInputState::new("前\u{1f600}选中\u{1f30d}后");
    input.select(TextSelection {
        anchor: "前\u{1f600}选中".len(),
        head: "前\u{1f600}".len(),
    })?;
    let excerpt = input.surrounding_text(14).unwrap();
    assert_eq!(excerpt.text(), "\u{1f600}选中\u{1f30d}");
    assert_eq!(
        excerpt.selection(),
        TextSelection {
            anchor: 10,
            head: 4
        }
    );
    assert!(input.surrounding_text(5).is_none());
    input.set_preedit("pin", None)?;
    let excerpt = input.surrounding_text(8).unwrap();
    assert_eq!(excerpt.text(), "\u{1f600}\u{1f30d}");
    assert_eq!(excerpt.selection(), TextSelection::caret(4));
    assert!(!excerpt.permits_deletion(1, 4));
    assert!(input.delete_surrounding(4, 3).is_err());
    assert_eq!(
        input.display_segments(),
        ["前\u{1f600}", "pin", "\u{1f30d}后"]
    );
    input.delete_surrounding(4, 4)?;
    assert_eq!(input.text(), "前选中后");
    assert_eq!(input.selection(), TextSelection { anchor: 9, head: 3 });
    input.commit("拼");
    assert_eq!(input.text(), "前拼后");
    Ok(())
}

#[test]
fn composition_replaces_a_reversed_selection_only_on_commit() -> Result<()> {
    let mut input = TextInputState::new("A中b\u{1f600}Z");
    let selected = TextSelection {
        anchor: "A中b\u{1f600}".len(),
        head: 1,
    };
    input.select(selected)?;
    input.set_preedit("ni", None)?;
    assert_eq!(input.display_segments(), ["A", "ni", "Z"]);
    assert_eq!(input.preedit().unwrap().selection, None);

    let local = TextSelection {
        anchor: "你\u{1f600}".len(),
        head: "你".len(),
    };
    input.set_preedit("你\u{1f600}", Some(local))?;
    assert_eq!(input.preedit().unwrap().selection, Some(local));
    assert_eq!(input.display_segments(), ["A", "你\u{1f600}", "Z"]);
    assert_eq!(input.text(), "A中b\u{1f600}Z");

    input.cancel_preedit();
    assert_eq!(input.selection(), selected);
    assert_eq!(input.display_segments().concat(), "A中b\u{1f600}Z");
    input.set_preedit("ni", Some(TextSelection::caret(2)))?;
    input.commit("你");
    assert_eq!(input.text(), "A你Z");
    assert_eq!(input.selection(), TextSelection::caret("A你".len()));
    input.commit("好");
    assert_eq!(input.display_segments().concat(), "A你好Z");
    Ok(())
}

#[test]
fn invalid_offsets_leave_an_active_composition_intact() -> Result<()> {
    let mut input = TextInputState::new("中");
    input.set_preedit("拼", None)?;
    assert!(input.select(TextSelection::caret(1)).is_err());
    assert!(
        input
            .set_preedit("\u{1f600}", Some(TextSelection::caret(2)))
            .is_err()
    );
    assert_eq!(input.display_segments(), ["中", "拼", ""]);
    assert_eq!(input.selection(), TextSelection::caret("中".len()));
    Ok(())
}

#[test]
fn host_edits_follow_focus_and_cancel_composition_on_detach() -> Result<()> {
    let mut ui = Ui::new();
    let first = ui.context().signal(TextInputState::new("中"));
    let second = ui.context().signal(TextInputState::default());
    let show = ui.context().signal(true);
    let view = ui.mount(component(move |_| {
        div()
            .child(div().text_input(first))
            .child_when(show, move || div().text_input(second))
    }))?;
    ui.flush()?;
    let children: Vec<_> = ui.children(view, ui.root(view)?)?.collect();
    let composing = || {
        TextInputEvent::Preedit(Preedit {
            text: "ni".into(),
            selection: None,
        })
    };
    assert!(!ui.dispatch_text_input(view, composing())?);
    ui.focus(view, children[0])?;
    assert!(ui.dispatch_text_input(view, composing())?);
    assert_eq!(
        ui.focused_text_input(view)?.unwrap().display_segments(),
        ["中", "ni", ""]
    );
    assert_eq!(ui.focus_next(view)?, Some(children[1]));
    assert!(ui.context().read(first)?.preedit().is_none());
    assert!(ui.dispatch_text_input(view, TextInputEvent::Commit("你".into()))?);
    assert_eq!(
        ui.focused_text_input(view)?.unwrap().selection(),
        TextSelection::caret(3)
    );
    assert_eq!(ui.context().read(first)?.text(), "中");
    ui.dispatch_text_input(view, composing())?;
    ui.context().set(show, false)?;
    ui.flush()?;
    assert!(ui.focused_text_input(view)?.is_none());
    assert!(ui.context().read(second)?.preedit().is_none());
    assert_eq!(ui.context().read(second)?.text(), "你");
    ui.focus(view, children[0])?;
    ui.dispatch_text_input(view, composing())?;
    ui.unmount(view)?;
    assert!(ui.context().read(first)?.preedit().is_none());
    Ok(())
}
