//! A caller-defined primitive, using the same mount path as a built-in element.
use crate::px;
use std::{cell::Cell, rc::Rc};

use crate::layout::{
    self, LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin,
    Query, Size,
};
use crate::state::Signal;

use super::test_ui;
use crate::element::{Draw, Element, IntoElement, PaintContext, Render, div, element, text};
use crate::{Context, Result, Styled, component};

impl crate::element::MountedNode {
    fn record(
        &self,
        scene: &layout::Scene,
        state: &crate::state::Runtime<()>,
    ) -> Result<Vec<Draw>> {
        let mut commands = Vec::new();
        self.paint(scene, state, &Default::default(), &mut commands)?;
        Ok(commands)
    }
}

struct Meter {
    active: Signal<bool>,
    percent: u8,
}

impl Render for Meter {
    fn render(self, _cx: &mut Context<'_>) -> impl IntoElement {
        element(MeterTrack {
            percent: self.percent,
        })
        .child(text("Volume"))
        .on_click(move |cx| cx.update(self.active, |v| *v = !*v))
    }
}

struct MeterTrack {
    percent: u8,
}
const TRACK: crate::Pixels = px(4.0);

impl LayoutNode for MeterTrack {
    fn measure(
        &mut self,
        cx: &mut MeasureContext<'_, '_>,
        query: Query,
    ) -> layout::Result<Metrics> {
        let label = cx.measure_child(0, query)?;
        let height = label
            .size
            .height
            .checked_add(TRACK)
            .ok_or(LayoutError::InvalidSize)?;
        Ok(Metrics {
            size: Size {
                width: query.resolve(label.size.width.max(px(80.0))),
                height,
            },
            baseline: label.baseline.map(|y| y + TRACK),
        })
    }

    fn layout(
        &mut self,
        cx: &mut LayoutContext<'_, '_>,
        size: Size,
    ) -> layout::Result<LayoutOutput> {
        let label = cx.measure_child(0, Query::AtMost(size.width))?;
        let output = cx.layout_child(0, label.size)?;
        cx.place_child(
            0,
            Origin {
                x: px(0.0),
                y: TRACK,
            },
        )?;
        Ok(LayoutOutput {
            content: Size {
                width: size.width.max(output.content.width),
                height: output
                    .content
                    .height
                    .checked_add(TRACK)
                    .ok_or(LayoutError::InvalidSize)?,
            },
            baseline: output.baseline.map(|y| y + TRACK),
        })
    }
}

impl Element for MeterTrack {
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        cx.fill(
            Size {
                width: cx.size.width * (f32::from(self.percent.min(100)) / 100.0),
                height: TRACK.min(cx.size.height),
            },
            [40, 110, 190, 255],
        );
        Ok(())
    }
}

#[test]
fn custom_element_layout_paint_and_input_follow_the_mounted_instance() -> Result<()> {
    let mut host = test_ui();
    let active = host.context().signal(true);
    let meter = move || Meter {
        active,
        percent: 75,
    };
    let root = host.mount(meter())?;
    let group = host.mount(component(|_| {
        div().child(meter()).child_when(active, meter)
    }))?;
    host.flush()?;

    let size = Size {
        width: px(200.0),
        height: px(48.0),
    };
    let i = host.instances.get_mut(&root).unwrap();
    let node = &i.bodies[&i.nodes[&[][..]]];
    assert!(node.record(&i.scene, &host.state.borrow()).is_err());
    for width in [200, 120] {
        i.scene.run_assigned(
            Size {
                width: px(width as f32),
                ..size
            },
            false,
        )?;
        assert_eq!(
            node.record(&i.scene, &host.state.borrow())?,
            [Draw::Fill {
                origin: Origin::default(),
                size: Size {
                    width: px(width as f32 * 0.75),
                    height: TRACK
                },
                fill: [40, 110, 190, 255].into(),
                corner_radii: Default::default()
            }]
        );
    }
    let label = &i.bodies[&i.nodes[&[0][..]]];
    assert_eq!(i.scene.geometry(label.id).unwrap().origin.y, TRACK);
    let held = label.record(&i.scene, &host.state.borrow())?;

    let i = host.instances.get_mut(&group).unwrap();
    i.scene.run_assigned(size, false)?;
    let fixed = i.nodes[&[0][..]];
    let conditional = i.nodes[&[1][..]];
    assert_eq!(
        i.scene.geometry(fixed).unwrap().metrics.size.width,
        px(80.0)
    );
    assert_eq!(
        i.scene.geometry(conditional).unwrap().origin.y,
        i.scene.geometry(fixed).unwrap().metrics.size.height
    );
    assert!(host.click(group, conditional)?);
    host.flush()?;
    assert!(!host.click(group, conditional)?);
    assert!(host.click(group, fixed)?);
    host.flush()?;
    assert_ne!(host.instances[&group].nodes[&[1][..]], conditional);
    host.unmount(root)?;
    host.unmount(group)?;
    assert!(matches!(&held[..], [Draw::Text {paragraph: p, ..}] if p.text == "Volume"));
    Ok(())
}

#[test]
fn nested_render_state_retires_on_failure_close_and_remount() -> Result<()> {
    let mut host = test_ui();
    let visible = host.context().signal(true);
    let handle = Rc::new(Cell::new(None));
    let fail = Rc::new(Cell::new(true));
    let view = host.mount(component(|_| {
        let handle = handle.clone();
        let fail = fail.clone();
        div().flex_col().child_when(visible, move || {
            let handle = handle.clone();
            let fail = fail.clone();
            component(move |cx| {
                let local = cx.signal(false);
                handle.set(Some(local));
                let title = if fail.get() {
                    "x".repeat(crate::text::MAX_TEXT_BYTES + 1)
                } else {
                    "Details".to_owned()
                };
                div()
                    .child(text(title))
                    .on_click(move |cx| cx.set(local, true))
            })
        })
    }))?;
    assert!(host.flush().is_err());
    assert!(host.context().read(handle.get().unwrap()).is_err());
    fail.set(false);
    host.flush()?;
    let first = handle.get().unwrap();
    let target = host.instances[&view].nodes[&[0][..]];
    host.click(view, target)?;
    assert!(*host.context().read(first)?);
    host.context().set(visible, false)?;
    host.flush()?;
    assert!(host.context().read(first).is_err());
    host.context().set(visible, true)?;
    host.flush()?;
    let second = handle.get().unwrap();
    assert!(!*host.context().read(second)?);
    host.unmount(view)?;
    assert!(host.context().read(second).is_err());
    Ok(())
}

#[test]
fn failed_paint_keeps_prior_commands_and_rejects_a_retired_signal() -> Result<()> {
    struct FillTwice(Signal<bool>);

    impl LayoutNode for FillTwice {
        fn measure(
            &mut self,
            _: &mut MeasureContext<'_, '_>,
            query: Query,
        ) -> layout::Result<Metrics> {
            Ok(Metrics {
                size: Size {
                    width: query.resolve(px(1.0)),
                    height: px(1.0),
                },
                baseline: None,
            })
        }

        fn layout(
            &mut self,
            _: &mut LayoutContext<'_, '_>,
            size: Size,
        ) -> layout::Result<LayoutOutput> {
            Ok(LayoutOutput {
                content: size,
                baseline: None,
            })
        }
    }

    impl Element for FillTwice {
        fn paint(&self, cx: &mut PaintContext) -> Result<()> {
            cx.fill(cx.size, [0; 4]);
            if *cx.read(self.0)? {
                cx.fill(cx.size, [255; 4]);
            }
            Ok(())
        }
    }

    let mut ui = crate::Ui::new();
    let handle = Cell::new(None);
    let owner = ui.mount(component(|cx| {
        handle.set(Some(cx.signal(true)));
        div()
    }))?;
    let view = ui.mount(component(|_| element(FillTwice(handle.get().unwrap()))))?;
    let root = ui.root(view)?;
    let size = Size {
        width: px(10.0),
        height: px(2.0),
    };
    ui.layout(view, size)?;
    let mut commands = Vec::new();
    ui.paint(view, root, &mut commands)?;
    ui.unmount(owner)?;
    assert!(ui.paint(view, root, &mut commands).is_err());
    assert_eq!(
        commands,
        [
            Draw::Fill {
                origin: Origin::default(),
                size,
                fill: [0; 4].into(),
                corner_radii: Default::default()
            },
            Draw::Fill {
                origin: Origin::default(),
                size,
                fill: [255; 4].into(),
                corner_radii: Default::default()
            }
        ]
    );
    Ok(())
}
