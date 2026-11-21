//! Item subscriptions, virtual row identity and collection ownership.
use anyhow::Result;
use solor::{
    Draw, NodeId, SharedString, Styled, Ui, ViewId, component, div, layout::Size, px, text,
    uniform_list, variable_list,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn submit(ui: &mut Ui, view: ViewId) -> Result<Vec<NodeId>> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(80.0),
            height: px(60.0),
        },
    )?;
    let frame = ui.record_frame(view)?;
    ui.accept_pointer_frame(frame.into_pointer_frame())?;
    Ok(ui.children(view, ui.root(view)?)?.collect())
}

#[test]
fn edits_notify_one_mounted_item_and_moves_preserve_its_identity() -> Result<()> {
    #[derive(PartialEq)]
    struct Row(usize);

    let mut ui = Ui::new();
    let rows = ui.context().signal_vec((0..100).map(Row));
    let mounts = Rc::new(Cell::new(0));
    let paints = Rc::new(RefCell::new(Vec::new()));
    let view = ui.mount(component(|_| {
        let mounts = mounts.clone();
        let paints = paints.clone();
        uniform_list(rows, px(20.0), move |item| {
            mounts.set(mounts.get() + 1);
            let paints = paints.clone();
            div().paint(move |cx| {
                paints.borrow_mut().push(cx.read(item)?.0);
                Ok(())
            })
        })
    }))?;
    let before = submit(&mut ui, view)?;
    assert_eq!(mounts.get(), 4);
    paints.borrow_mut().clear();

    rows.set(1, Row(1), &mut ui.context())?;
    rows.update_in_place(99, &mut ui.context(), |r| r.0 += 10)?;
    assert!(!ui.has_pending_bindings());
    rows.update_in_place(1, &mut ui.context(), |r| r.0 += 20)?;
    rows.update_in_place(1, &mut ui.context(), |r| r.0 += 10)?;
    assert_eq!(submit(&mut ui, view)?, before);
    assert_eq!(&*paints.borrow(), &[31]);

    let item = rows.item(1, &ui.context())?;
    rows.move_item(1, 0, &mut ui.context())?;
    assert_eq!(
        submit(&mut ui, view)?,
        [before[1], before[0], before[2], before[3]]
    );
    assert_eq!(mounts.get(), 4);
    assert_eq!(rows.item(0, &ui.context())?, item);
    paints.borrow_mut().clear();
    ui.context().set(item, Row(42))?;
    submit(&mut ui, view)?;
    assert_eq!(&*paints.borrow(), &[42]);
    Ok(())
}

#[test]
fn variable_text_rows_reconcile_edits_and_dispose_with_the_owning_scope() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let handle = Cell::new(None);
    let view = ui.mount(component(|cx| {
        let rows = cx.signal_vec([SharedString::from("A"), SharedString::from("B")]);
        handle.set(Some(rows));
        variable_list(rows, px(20.0), |item| text(item).line_height(px(20.0)))
    }))?;
    let rows = handle.get().unwrap();
    let before = submit(&mut ui, view)?;
    let removed = rows.item(1, &ui.context())?;
    let inserted = rows.insert(0, "C".into(), &mut ui.context())?;
    rows.set(1, "A\nA".into(), &mut ui.context())?;
    rows.set(2, "queued".into(), &mut ui.context())?;
    rows.remove(2, &mut ui.context())?;
    let after = submit(&mut ui, view)?;
    assert_eq!(after.len(), 2);
    assert_eq!(after[1], before[0]);
    assert_eq!(
        ui.geometry(view, after[1]).unwrap().metrics.size.height,
        px(40.0)
    );
    let frame = ui.record_frame(view)?;
    let captions: Vec<_> = frame
        .draws()
        .iter()
        .filter_map(|draw| match draw {
            Draw::Text { paragraph, .. } => Some(paragraph.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(captions, ["C", "A\nA"]);
    assert!(ui.context().read(removed).is_err());

    let other = &mut Ui::new();
    assert!(rows.push("foreign".into(), &mut other.context()).is_err());
    assert!(rows.move_item(0, 2, &mut ui.context()).is_err());
    assert_eq!(rows.item(0, &ui.context())?, inserted);
    ui.unmount(view)?;
    assert!(rows.items(&ui.context()).is_err());
    assert!(ui.context().read(inserted).is_err());
    Ok(())
}

#[test]
fn inserted_items_follow_the_collection_scope_and_last_binding() -> Result<()> {
    let mut ui = Ui::new();
    let owner = Cell::new(None);
    let view = ui.mount(component(|cx| {
        owner.set(Some(cx.signal_vec(std::iter::empty::<solor::Rgba>())));
        div()
    }))?;
    let rows = owner.get().unwrap();
    let handle = Cell::new(None);
    let writer = ui.mount(component(|cx| {
        handle.set(Some(rows.push(solor::rgb(0), cx).unwrap()));
        div()
    }))?;
    ui.unmount(writer)?;
    let item = handle.get().unwrap();
    assert_eq!(ui.context().read(item)?, &solor::rgb(0));
    // Another view may still use the old row while the owning collection is retired.
    let reader = ui.mount(component(|_| div().bg(item)))?;
    submit(&mut ui, reader)?;
    ui.unmount(view)?;
    assert!(rows.items(&ui.context()).is_err());
    assert_eq!(ui.context().read(item)?, &solor::rgb(0));
    ui.unmount(reader)?;
    assert!(ui.context().read(item).is_err());
    Ok(())
}
