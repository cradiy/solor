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
fn capacity_reserves_handle_space_without_creating_rows_or_limiting_growth() -> Result<()> {
    let mut ui = Ui::new();
    let cx = &mut ui.context();
    let rows = cx.signal_vec_with_capacity([3, 4], 4)?;
    let original = rows.item(0, cx)?;
    let storage = rows.items(cx)?.as_ptr();
    assert_eq!(rows.items(cx)?.len(), 2);
    rows.splice(0..0, [1, 2], cx)?;
    assert_eq!(rows.items(cx)?.as_ptr(), storage);
    rows.push(5, cx)?;
    assert_eq!(rows.item(2, cx)?, original);
    assert_eq!(cx.read(original)?, &3);

    let short_estimate = cx.signal_vec_with_capacity([1, 2, 3], 1)?;
    assert_eq!(cx.read(short_estimate.item(2, cx)?)?, &3);
    let unread = std::iter::from_fn(|| -> Option<u32> { panic!("polled before reserving") });
    assert!(cx.signal_vec_with_capacity(unread, usize::MAX).is_err());
    Ok(())
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

#[test]
fn splice_reuses_mounted_rows_outside_the_replacement() -> Result<()> {
    let mut ui = Ui::new();
    let a = solor::rgb(0x123456);
    let b = solor::rgb(0x654321);
    let rows = ui.context().signal_vec([a, b, b, a]);
    let view = ui.mount(component(|_| {
        uniform_list(rows, px(20.0), |color| div().bg(color))
    }))?;
    let before = submit(&mut ui, view)?;
    let handles = rows.items(&ui.context())?.to_vec();
    rows.splice(1..3, [b], &mut ui.context())?;
    let new = rows.item(1, &ui.context())?;
    assert!(!handles.contains(&new));
    assert_eq!(rows.items(&ui.context())?, &[handles[0], new, handles[3]]);
    assert_eq!(ui.context().read(handles[1])?, &b);
    let after = submit(&mut ui, view)?;
    assert_eq!([after[0], after[2]], [before[0], before[3]]);
    assert!(!before.contains(&after[1]));
    for old in &handles[1..3] {
        assert!(ui.context().read(*old).is_err());
    }

    ui.context().spawn(async move |cx| cx.extend(rows, [a, b]));
    ui.poll_tasks(std::task::Waker::noop())?;
    assert_eq!(&submit(&mut ui, view)?[..3], &after);
    let cx = ui.context();
    assert_eq!(cx.read(rows.item(4, &cx)?)?, &b);
    let all = rows.items(&ui.context())?.to_vec();
    rows.splice(0..all.len(), [], &mut ui.context())?;
    assert!(submit(&mut ui, view)?.is_empty());
    assert!(all.iter().all(|item| ui.context().read(*item).is_err()));
    Ok(())
}

#[test]
fn empty_invalid_and_interrupted_splices_publish_nothing() -> Result<()> {
    let mut ui = Ui::new();
    let rows = ui.context().signal_vec([7]);
    let view = ui.mount(component(|_| uniform_list(rows, px(20.0), |_| div())))?;
    submit(&mut ui, view)?;
    let original = rows.item(0, &ui.context())?;
    rows.splice(1..1, [], &mut ui.context())?;
    let unread = || std::iter::from_fn(|| panic!("invalid splice polled its input"));
    let start = 1;
    assert!(rows.splice(start..0, unread(), &mut ui.context()).is_err());
    assert!(rows.splice(1..2, unread(), &mut ui.context()).is_err());
    assert!(
        rows.splice(0..1, unread(), &mut Ui::new().context())
            .is_err()
    );

    let failing = std::iter::once(8).chain(std::iter::from_fn(|| panic!("interrupted input")));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rows.splice(0..1, failing, &mut ui.context())
        }))
        .is_err()
    );
    assert_eq!(rows.items(&ui.context())?, &[original]);
    assert_eq!(*ui.context().read(original)?, 7);
    assert!(!ui.has_pending_bindings());
    Ok(())
}

#[test]
fn a_panicking_item_destructor_leaves_the_new_order_usable() -> Result<()> {
    struct Row(bool);
    impl Drop for Row {
        fn drop(&mut self) {
            if self.0 {
                panic!("interrupted disposal");
            }
        }
    }
    let mut ui = Ui::new();
    let rows = ui.context().signal_vec([Row(false), Row(true), Row(false)]);
    let old = rows.items(&ui.context())?.to_vec();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rows.splice(1..3, [Row(false)], &mut ui.context())
        }))
        .is_err()
    );
    assert_eq!(rows.items(&ui.context())?.len(), 2);
    assert_eq!(rows.item(0, &ui.context())?, old[0]);
    assert!(
        old[1..]
            .iter()
            .all(|item| ui.context().read(*item).is_err())
    );
    rows.remove(1, &mut ui.context())?;
    Ok(())
}
