//! Retained property updates, per-field cascading and subscription cleanup.
use std::rc::Rc;

use anyhow::Result;
use solor::{
    Draw, FontFamily, GenericFontFamily, StyleRefinement, Styled, Ui, component, div, layout::Size,
    px, text,
};

const FONT: &[u8] = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

#[derive(Clone)]
struct Typeface(GenericFontFamily);

impl From<Typeface> for FontFamily {
    fn from(value: Typeface) -> Self {
        value.0.into()
    }
}

#[test]
fn a_font_signal_restyles_only_the_labels_that_inherit_it() -> Result<()> {
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let family = ui.context().signal(Typeface(GenericFontFamily::SansSerif));
    let size = ui.context().signal(px(14.0));
    let open = ui.context().signal(false);
    let view = ui.mount(component(|_| {
        div()
            .font_family(family)
            .text_size(size)
            .child(text("A"))
            .child(
                text("B")
                    .font_family("Local")
                    .text_size(px(12.0))
                    .line_height(px(24.0)),
            )
            .child(text("Small").text_size(px(12.0)).font_family("Local"))
            .child_when(open, || text("C"))
    }))?;
    let viewport = Size {
        width: px(200.0),
        height: px(100.0),
    };
    let record = |ui: &mut Ui| -> Result<Vec<_>> {
        ui.flush()?;
        ui.layout(view, viewport)?;
        Ok(ui
            .record_frame(view)?
            .draws()
            .iter()
            .map(|draw| match draw {
                Draw::Text { paragraph, .. } => Rc::clone(paragraph),
                _ => panic!("expected text only"),
            })
            .collect())
    };
    let original = record(&mut ui)?;
    let root = ui.root(view)?;
    let ids: Vec<_> = ui.children(view, root)?.collect();
    ui.context()
        .update_in_place(family, |f| f.0 = GenericFontFamily::Serif)?;
    let refont = record(&mut ui)?;
    assert!(!Rc::ptr_eq(&original[0], &refont[0]));
    assert!(Rc::ptr_eq(&original[1], &refont[1]));
    ui.context().set(size, px(26.0))?;
    ui.context().set(open, true)?;
    let next = record(&mut ui)?;
    assert_eq!(
        ui.children(view, root)?.take(ids.len()).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(next[0].style.size, px(26.0));
    assert_eq!(next[3].style, next[0].style);
    assert!(next[2].style.line_height < original[0].style.line_height);
    assert!(original[0].style.line_height < next[0].style.line_height);
    assert!(Rc::ptr_eq(&original[2], &next[2]));
    assert!(Rc::ptr_eq(&original[1], &next[1]));
    ui.unmount(view)?;
    ui.context().notify(family)?;
    assert!(!ui.has_pending_bindings());
    Ok(())
}

#[test]
fn box_bindings_preserve_overrides_and_retry_invalid_values() -> Result<()> {
    let mut ui = Ui::new();
    let width = ui.context().signal(px(30.0));
    let hidden_width = ui.context().signal(px(35.0));
    let inset = ui.context().signal(px(2.0));
    let card = StyleRefinement::default().w(width).p(inset);
    let view = ui.mount(component(|_| {
        div()
            .items_start()
            .child(
                div()
                    .w(hidden_width)
                    .with_style(&card)
                    .pl(px(5.0))
                    .child(div().size(px(8.0))),
            )
            .child(div().with_style(&card).w(px(40.0)))
    }))?;
    let root = ui.root(view)?;
    let siblings: Vec<_> = ui.children(view, root)?.collect();
    let content = ui.children(view, siblings[0])?.next().unwrap();
    let viewport = Size {
        width: px(200.0),
        height: px(100.0),
    };
    ui.layout(view, viewport)?;
    ui.context().set(hidden_width, px(80.0))?;
    assert!(!ui.has_pending_bindings());
    ui.context().set(inset, px(-2.0))?;
    assert!(ui.flush().is_err());
    assert!(ui.has_pending_bindings());
    ui.context().set(inset, px(7.0))?;
    ui.context().set(width, px(60.0))?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    assert_eq!(
        ui.geometry(view, siblings[0]).unwrap().metrics.size.width,
        px(60.0)
    );
    assert_eq!(
        ui.geometry(view, siblings[1]).unwrap().metrics.size.width,
        px(40.0)
    );
    let origin = ui.geometry(view, content).unwrap().origin;
    assert_eq!((origin.x, origin.y), (px(5.0), px(7.0)));
    Ok(())
}

#[test]
fn paint_changes_keep_geometry_and_follow_the_current_visibility_source() -> Result<()> {
    use solor::{rgb, style::Visibility};
    let mut ui = Ui::with_font(FONT.to_vec())?;
    let old = ui.context().signal(true);
    let next = ui.context().signal(false);
    let visible = ui.context().signal(Visibility::When(old));
    let ink = ui.context().signal(rgb(0x335577));
    let view = ui.mount(component(|_| text("A").visibility(visible).text_bg(ink)))?;
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(90.0),
            height: px(32.0),
        },
    )?;
    let frame = ui.record_frame(view)?;
    let [Draw::Fill { .. }, Draw::Text { paragraph, .. }] = frame.draws() else {
        panic!("expected a highlighted label");
    };
    ui.context().set(ink, rgb(0x203040))?;
    ui.flush()?;
    let updated = ui.record_frame(view)?;
    let [
        Draw::Fill { fill, .. },
        Draw::Text {
            paragraph: updated, ..
        },
    ] = updated.draws()
    else {
        panic!("expected a highlighted label after paint-only update");
    };
    assert_eq!(fill, &solor::Fill::from(rgb(0x203040)));
    assert!(Rc::ptr_eq(paragraph, updated));

    ui.context().set(visible, Visibility::When(next))?;
    ui.flush()?;
    assert!(ui.record_frame(view)?.draws().is_empty());
    ui.context().set(old, false)?;
    assert!(!ui.has_pending_bindings());
    ui.context().set(next, true)?;
    ui.flush()?;
    assert_eq!(ui.record_frame(view)?.draws().len(), 2);
    ui.context().set(visible, Visibility::Hidden)?;
    ui.unmount(view)?;
    ui.flush()?;
    ui.context().set(next, false)?;
    assert!(!ui.has_pending_bindings());
    Ok(())
}
