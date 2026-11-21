//! Children and local drawing within a retained container.
use anyhow::Result;
use solor::{
    Children, Context, Div, Draw, IntoElement, ParentElement, Render, Styled, Ui, component, div,
    layout::{Origin, Size},
    px, rgb,
};

fn item(width: f32) -> Div {
    div().w(px(width)).h(px(1.0))
}

#[test]
fn container_paint_uses_its_content_box_between_background_and_children() -> Result<()> {
    let mut ui = Ui::new();
    let tint = ui.context().signal(rgb(0x123456));
    let view = ui.mount(component(|_| {
        div()
            .p(px(2.5))
            .bg(rgb(0x222222))
            .paint(move |cx| {
                cx.fill(cx.size, *cx.read(tint)?);
                Ok(())
            })
            .child(div().size(px(4.0)).bg(rgb(0xffffff)))
    }))?;
    let viewport = Size {
        width: px(20.0),
        height: px(12.0),
    };
    let root = ui.root(view)?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    let check_frame = |ui: &Ui, color: solor::Rgba| -> Result<()> {
        let frame = ui.record_frame(view)?;
        let mut fills = Vec::new();
        for command in frame.draws() {
            let Draw::Fill {
                origin, size, fill, ..
            } = command
            else {
                panic!("expected a rectangle");
            };
            fills.push((*origin, *size, fill.clone()));
        }
        let inset = Origin {
            x: px(2.5),
            y: px(2.5),
        };
        assert_eq!(
            fills,
            [
                (Origin::default(), viewport, rgb(0x222222).into()),
                (
                    inset,
                    Size {
                        width: px(15.0),
                        height: px(7.0)
                    },
                    color.into()
                ),
                (
                    inset,
                    Size {
                        width: px(4.0),
                        height: px(4.0)
                    },
                    rgb(0xffffff).into()
                ),
            ]
        );
        assert!(!frame.requests_animation_frame());
        Ok(())
    };
    check_frame(&ui, rgb(0x123456))?;
    ui.context().set(tint, rgb(0xabcdef))?;
    ui.flush()?;
    let mut dirty = Vec::new();
    ui.collect_repaints(view, &mut dirty)?;
    assert_eq!(dirty, [root]);
    check_frame(&ui, rgb(0xabcdef))?;
    Ok(())
}

#[test]
fn iterated_siblings_keep_order_and_identity_across_a_conditional_branch() -> Result<()> {
    let mut ui = Ui::new();
    let open = ui.context().signal(false);
    let widths = [2.5, 3.0];
    let view = ui.mount(component(|_| {
        div()
            .when(true, |list| list.flex_row().gap_x(px(0.5)))
            .when_some(Some(1.0), |list, width| list.child(item(width)))
            .children(widths.iter().copied().map(item))
            .child_when(open, || item(4.0))
            .when(false, |list| list.child(item(99.0)))
            .when_some(None, |list, width| list.child(item(width)))
            .children([item(5.0)])
    }))?;
    let root = ui.root(view)?;
    let viewport = Size {
        width: px(40.0),
        height: px(1.0),
    };
    ui.flush()?;
    ui.layout(view, viewport)?;
    let fixed: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(fixed.len(), 4);
    for (id, (width, x)) in fixed
        .iter()
        .zip([(1.0, 0.0), (2.5, 1.5), (3.0, 4.5), (5.0, 8.0)])
    {
        let geometry = ui.geometry(view, *id).unwrap();
        assert_eq!(geometry.metrics.size.width, px(width));
        assert_eq!(geometry.origin.x, px(x));
    }

    ui.context().set(open, true)?;
    ui.flush()?;
    ui.layout(view, viewport)?;
    let expanded: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(expanded.len(), 5);
    assert_eq!(expanded[..3], fixed[..3]);
    assert_eq!(expanded[4], fixed[3]);
    assert_eq!(
        ui.geometry(view, expanded[3]).unwrap().metrics.size.width,
        px(4.0)
    );
    assert_eq!(ui.geometry(view, fixed[3]).unwrap().origin.x, px(12.5));

    ui.context().set(open, false)?;
    ui.flush()?;
    assert_eq!(ui.children(view, root)?.collect::<Vec<_>>(), fixed);
    Ok(())
}

#[test]
fn text_children_keep_borrowed_content_and_update_bound_content_in_place() -> Result<()> {
    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let value = ui.context().signal(solor::SharedString::from("Headphones"));
    let row = {
        let caption = String::from("Output device");
        div()
            .flex_row()
            .text_color(rgb(0x123456))
            .child(caption.as_str())
            .child(value)
    };
    let view = ui.mount(component(move |_| row))?;
    let root = ui.root(view)?;
    let ids: Vec<_> = ui.children(view, root)?.collect();
    assert_eq!(ids.len(), 2);
    let check = |ui: &mut Ui, expected: &str| -> Result<()> {
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(360.0),
                height: px(50.0),
            },
        )?;
        let frame = ui.record_frame(view)?;
        let labels: Vec<_> = frame
            .draws()
            .iter()
            .map(|draw| match draw {
                Draw::Text {
                    paragraph,
                    color: solor::Ink::Solid(color),
                    ..
                } => (paragraph.text.as_str(), *color),
                _ => panic!("expected a text leaf"),
            })
            .collect();
        assert_eq!(
            labels,
            [("Output device", rgb(0x123456)), (expected, rgb(0x123456))]
        );
        assert_eq!(ui.children(view, root)?.collect::<Vec<_>>(), ids);
        Ok(())
    };
    check(&mut ui, "Headphones")?;
    ui.context().set(value, "Speakers".into())?;
    check(&mut ui, "Speakers")
}

#[test]
fn a_component_places_conditional_content_between_its_own_children() -> Result<()> {
    #[derive(Default)]
    struct Section(Children);

    impl ParentElement for Section {
        fn children_mut(&mut self) -> &mut Children {
            &mut self.0
        }
    }

    impl Render for Section {
        fn render(self, _: &mut Context<'_>) -> impl IntoElement {
            div().child("Audio").children(self.0).child("Done")
        }
    }

    let mut ui = Ui::with_font(
        include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec(),
    )?;
    let expanded = ui.context().signal(false);
    let section = Section::default()
        .child("Output")
        .child_when(expanded, || "Details")
        .children(["Volume", "Balance"]);
    let view = ui.mount(section)?;
    let root = ui.root(view)?;
    let record = |ui: &mut Ui, expected: &[&str]| -> Result<Vec<_>> {
        ui.flush()?;
        ui.layout(
            view,
            Size {
                width: px(200.0),
                height: px(200.0),
            },
        )?;
        let frame = ui.record_frame(view)?;
        let labels: Vec<_> = frame
            .draws()
            .iter()
            .map(|draw| match draw {
                Draw::Text { paragraph, .. } => paragraph.text.as_str(),
                _ => panic!("expected section text"),
            })
            .collect();
        assert_eq!(labels, expected);
        let children: Vec<_> = ui.children(view, root)?.collect();
        assert_eq!(children.len(), expected.len());
        Ok(children)
    };
    let collapsed = ["Audio", "Output", "Volume", "Balance", "Done"];
    let fixed = record(&mut ui, &collapsed)?;
    ui.context().set(expanded, true)?;
    let visible = record(
        &mut ui,
        &["Audio", "Output", "Details", "Volume", "Balance", "Done"],
    )?;
    assert_eq!(visible[..2], fixed[..2]);
    assert_eq!(visible[3..], fixed[2..]);
    ui.context().set(expanded, false)?;
    assert_eq!(record(&mut ui, &collapsed)?, fixed);
    Ok(())
}
