use crate::px;
mod custom_element;

use super::element::{div, text};
use super::*;
use crate::layout::Size;
use crate::text::Text;
use std::{cell::Cell, rc::Rc};

pub(super) fn test_ui() -> Ui {
    Ui::with_font(include_bytes!("../../../../fixtures/fonts/NotoSansCJKsc-fixture.otf").to_vec())
        .unwrap()
}

impl Instance {
    fn node_at(&self, path: &[usize]) -> Option<NodeId> {
        self.nodes.get(path).copied()
    }
}

fn section<E: IntoElement + 'static>(
    open: Signal<bool>,
    detail: impl Fn() -> E + 'static,
) -> impl IntoElement {
    div()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .padding(px(6.0))
                .child(text("Audio settings"))
                .on_click(move |event| event.update(open, |v| *v = !*v)),
        )
        .child_when(open, detail)
}

#[test]
fn clicks_write_immediately_and_only_final_visibility_builds_a_leaf() -> Result<()> {
    let mut host = test_ui();
    let open_handle = Cell::new(None);
    let mounts = Cell::new(0);
    let builds = Rc::new(Cell::new(0));
    let scope = host.mount(component(|mount| {
        mounts.set(mounts.get() + 1);
        let open = mount.signal(false);
        open_handle.set(Some(open));
        let builds = builds.clone();
        section(open, move || {
            builds.set(builds.get() + 1);
            text("Output follows the system device.")
        })
    }))?;
    let header = host.instances[&scope].nodes[&[0][..]];
    let open = open_handle.get().unwrap();
    host.flush()?;
    assert!(!*host.state.get_mut().read(open)?);
    assert!(host.click(scope, header)?);
    assert!(*host.state.get_mut().read(open)?);
    assert!(host.instances[&scope].node_at(&[1]).is_none());
    host.click(scope, header)?;
    host.flush()?;
    assert_eq!(builds.get(), 0);

    host.click(scope, header)?;
    host.flush()?;
    let first = host.instances[&scope].node_at(&[1]).unwrap();
    let scene = &mut host.instances.get_mut(&scope).unwrap().scene;
    for width in [320, 160, 320] {
        scene.run_assigned(
            Size {
                width: px(width as f32),
                height: px(120.0),
            },
            false,
        )?;
        let header_box = scene.geometry(header).unwrap();
        assert_eq!(
            scene.geometry(first).unwrap().origin.y,
            header_box.metrics.size.height + px(8.0)
        );
        let label = scene.children(header)?.next().unwrap();
        assert_eq!(
            scene.geometry(label).unwrap().origin,
            layout::Origin {
                x: px(6.0),
                y: px(6.0)
            }
        );
    }
    host.flush()?;
    assert_eq!(host.instances[&scope].node_at(&[1]), Some(first));
    assert_eq!(builds.get(), 1);
    assert!(*host.state.get_mut().read(open)?);

    host.click(scope, header)?;
    host.flush()?;
    assert!(
        host.instances[&scope]
            .scene
            .body::<StyledBody<Text>>(first)
            .is_err()
    );
    host.click(scope, header)?;
    host.flush()?;
    assert_ne!(host.instances[&scope].node_at(&[1]).unwrap(), first);
    assert_eq!((mounts.get(), builds.get()), (1, 2));
    Ok(())
}

#[test]
fn instance_owns_initial_value_and_unmount_cancels_pending_work() -> Result<()> {
    let open_handle = Cell::new(None);
    let make = || {
        component(|mount| {
            let open = mount.signal(false);
            open_handle.set(Some(open));
            section(open, || text("Detail"))
        })
    };
    let mut host = test_ui();
    let scope = host.mount(make())?;
    let open = open_handle.get().unwrap();
    let sibling = host.mount(make())?;
    let header = host.instances[&scope].nodes[&[0][..]];
    host.click(scope, header)?;
    host.flush()?;
    assert!(host.instances[&scope].node_at(&[1]).is_some());
    assert!(host.instances[&sibling].node_at(&[1]).is_none());
    host.click(scope, header)?;
    host.unmount(scope)?;
    assert!(host.state.get_mut().read(open).is_err());
    assert_eq!(host.state.get_mut().pending_count(), 0);
    assert_eq!(host.state.get_mut().binding_count(), 1);
    let new_scope = host.mount(make())?;
    assert!(!host.click(new_scope, header)?);
    assert!(!*host.state.get_mut().read(open_handle.get().unwrap())?);
    host.unmount(new_scope)?;
    host.unmount(sibling)?;
    assert_eq!(
        (
            host.state.get_mut().signal_count(),
            host.state.get_mut().binding_count()
        ),
        (0, 0)
    );
    Ok(())
}

#[test]
fn middle_subtree_keeps_volume_identity_and_dispatches_only_the_changed_section() -> Result<()> {
    let mut host = test_ui();
    let volume_handle = Cell::new(None);
    let make = || {
        component(|mount| {
            let open = mount.signal(false);
            let volume = mount.signal(40_u8);
            volume_handle.set(Some(volume));
            div()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .child(text("Audio settings"))
                        .on_click(move |event| event.update(open, |v| *v = !*v)),
                )
                .child_when(open, move || {
                    div()
                        .flex_col()
                        .child(text("Output device"))
                        .child(text("System default"))
                        .on_click(move |event| event.update(open, |v| *v = false))
                })
                .child(
                    div()
                        .child(text("Volume"))
                        .on_click(move |event| event.update(volume, |v| *v += 1)),
                )
        })
    };
    let scope = host.mount(make())?;
    let volume = volume_handle.get().unwrap();
    let other = host.mount(make())?;
    assert_eq!(host.flush()?, 2);
    let header = host.instances[&scope].nodes[&[0][..]];
    let row = host.instances[&scope].nodes[&[2][..]];
    let layout = |host: &mut Ui| -> Result<_> {
        let i = host.instances.get_mut(&scope).unwrap();
        i.scene.run_assigned(
            Size {
                width: px(320.0),
                height: px(200.0),
            },
            false,
        )?;
        assert_eq!(i.nodes[&[2][..]], row);
        Ok((
            i.scene.geometry(header).unwrap(),
            i.scene.geometry(row).unwrap(),
        ))
    };
    let (top, closed) = layout(&mut host)?;
    assert_eq!(closed.origin.y, top.metrics.size.height + px(8.0));
    host.click(scope, row)?;
    assert_eq!(host.flush()?, 0);
    host.click(scope, header)?;
    assert_eq!(host.flush()?, 1);
    let (_, opened) = layout(&mut host)?;
    let i = &host.instances[&scope];
    let detail = i.nodes[&[1][..]];
    let bounds = i.scene.geometry(detail).unwrap();
    assert_eq!(bounds.origin.y, closed.origin.y);
    assert_eq!(
        opened.origin.y,
        bounds.origin.y + bounds.metrics.size.height + px(8.0)
    );
    assert!(i.scene.geometry(i.nodes[&[1, 1][..]]).unwrap().origin.y > px(0.0));
    assert!(!host.instances[&other].nodes.contains_key(&[1][..]));

    assert!(host.click(scope, row)?);
    assert!(host.click(scope, detail)?);
    assert_eq!(host.flush()?, 1);
    assert_eq!(layout(&mut host)?.1, closed);
    assert!(!host.click(scope, detail)?);
    assert!(host.click(scope, row)?);
    assert_eq!(*host.state.get_mut().read(volume)?, 43);
    assert_eq!(host.flush()?, 0);
    Ok(())
}

#[test]
fn nested_initial_bindings_and_simultaneous_parent_removal() -> Result<()> {
    let mut host = test_ui();
    let outer = host.state.get_mut().create_signal(true);
    let inner = host.state.get_mut().create_signal(true);
    let builds = Rc::new(Cell::new(0));
    let scope = host.mount(component(|_| {
        div()
            .flex_col()
            .child_when(outer, {
                let builds = builds.clone();
                move || {
                    let builds = builds.clone();
                    div().flex_col().child_when(inner, move || {
                        builds.set(builds.get() + 1);
                        text("Nested")
                    })
                }
            })
            .child_when(inner, || text("Sibling"))
    }))?;
    assert_eq!(host.flush()?, 3);
    let sibling = host.instances[&scope].nodes[&[1][..]];
    assert_eq!(builds.get(), 1);
    host.state.get_mut().set(inner, false)?;
    host.flush()?;
    host.state.get_mut().set(inner, true)?;
    host.state.get_mut().set(outer, false)?;
    host.flush()?;
    assert_eq!(builds.get(), 1);
    assert_eq!(host.state.get_mut().binding_count(), 2);
    assert_ne!(host.instances[&scope].nodes[&[1][..]], sibling);
    host.state.get_mut().set(outer, true)?;
    assert_eq!(host.flush()?, 2);
    assert_eq!(builds.get(), 2);
    host.unmount(scope)?;
    assert_eq!(host.state.get_mut().binding_count(), 0);
    let label = host.mount(component(|_| text("Standalone")))?;
    assert_eq!(host.flush()?, 0);
    let i = &host.instances[&label];
    assert_eq!(
        i.scene
            .body::<StyledBody<Text>>(i.scene.root())?
            .content
            .content(),
        "Standalone"
    );
    Ok(())
}

#[test]
fn failed_subtree_attachment_releases_partial_nodes_handlers_and_bindings_then_retries()
-> Result<()> {
    let mut host = test_ui();
    let open = host.state.get_mut().create_signal(true);
    let invalid = Rc::new(Cell::new(true));
    let scope = host.mount(component(|_| {
        div()
            .flex_col()
            .child_when(open, {
                let invalid = invalid.clone();
                move || {
                    div()
                        .flex_col()
                        .on_click(|_| Ok(()))
                        .child_when(open, || text("Nested"))
                        .child(text(if invalid.get() {
                            "X".repeat(crate::text::MAX_TEXT_BYTES + 1)
                        } else {
                            "Valid".into()
                        }))
                }
            })
            .child(text("Tail"))
    }))?;
    let tail = host.instances[&scope].nodes[&[1][..]];
    assert!(host.flush().is_err());
    let i = &host.instances[&scope];
    assert_eq!(i.nodes.len(), 2);
    assert!(i.bodies.values().all(|node| node.handlers.is_none()));
    assert_eq!(host.state.get_mut().binding_count(), 1);
    assert_eq!(host.state.get_mut().pending_count(), 0);
    invalid.set(false);
    assert_eq!(host.flush()?, 2);
    assert_eq!(host.instances[&scope].nodes[&[1][..]], tail);
    assert!(host.instances[&scope].nodes.contains_key(&[0, 0][..]));
    Ok(())
}

#[test]
fn mixed_state_uses_independent_typed_handles_and_one_scope_cleanup() -> Result<()> {
    let mut host = test_ui();
    let handles = Cell::new(None);
    let scope = host.mount(component(|mount| {
        let open = mount.signal(false);
        let muted = mount.signal(false);
        let volume = mount.signal(40_u8);
        let device = mount.signal(String::from("System"));
        handles.set(Some((open, muted, volume, device)));
        div()
            .child(text("Select headphones"))
            .on_click(move |event| {
                event.update(muted, |v| *v = true)?;
                event.update(volume, |v| *v += 5)?;
                event.update(device, |v| v.push_str(" headphones"))
            })
            .child_when(open, || text("Audio settings"))
    }))?;
    let (open, muted, volume, device) = handles.get().unwrap();
    host.flush()?;
    assert!(host.click(scope, host.instances[&scope].scene.root())?);
    assert_eq!(host.state.get_mut().pending_count(), 0);
    assert!(!*host.state.get_mut().read(open)?);
    assert!(*host.state.get_mut().read(muted)?);
    assert_eq!(*host.state.get_mut().read(volume)?, 45);
    assert_eq!(host.state.get_mut().read(device)?, "System headphones");

    host.unmount(scope)?;
    assert!(host.state.get_mut().read(open).is_err());
    assert!(host.state.get_mut().read(muted).is_err());
    assert!(host.state.get_mut().read(volume).is_err());
    assert!(host.state.get_mut().read(device).is_err());
    Ok(())
}

#[test]
fn external_playback_logic_updates_both_views_and_outlives_them() -> Result<()> {
    struct Playback {
        playing: Signal<bool>,
    }
    impl Playback {
        fn toggle(&self, cx: &mut Context) -> Result<()> {
            let next = !*cx.read(self.playing)?;
            cx.set(self.playing, next)
        }
    }

    let mut host = test_ui();
    let playback = Playback {
        playing: host.context().signal(false),
    };
    let view = || {
        component(|_| {
            div()
                .flex_col()
                .child_when(playback.playing, || text("Playing"))
        })
    };
    let player = host.mount(view())?;
    let mini_player = host.mount(view())?;
    host.flush()?;

    playback.toggle(&mut host.context())?;
    assert!(*host.context().read(playback.playing)?);
    assert!(host.instances[&player].node_at(&[0]).is_none());
    assert_eq!(host.flush()?, 2);
    assert!(host.instances[&player].node_at(&[0]).is_some());
    assert!(host.instances[&mini_player].node_at(&[0]).is_some());

    host.unmount(player)?;
    playback.toggle(&mut host.context())?;
    assert_eq!(host.flush()?, 1);
    assert!(host.instances[&mini_player].node_at(&[0]).is_none());
    host.unmount(mini_player)?;
    playback.toggle(&mut host.context())?;
    assert!(*host.context().read(playback.playing)?);
    assert_eq!(host.flush()?, 0);
    Ok(())
}

#[test]
fn borrowed_state_survives_remount_and_stale_input_rolls_back_mount() -> Result<()> {
    let mut host = test_ui();
    let open = host.state.get_mut().create_signal(true);
    let make = || component(|_| section(open, || text("Detail")));
    let scope = host.mount(make())?;
    host.flush()?;
    assert!(host.instances[&scope].node_at(&[1]).is_some());
    host.click(scope, host.instances[&scope].nodes[&[0][..]])?;
    host.unmount(scope)?;
    let scope = host.mount(make())?;
    host.flush()?;
    assert!(!*host.state.get_mut().read(open)?);
    assert!(host.instances[&scope].node_at(&[1]).is_none());
    host.unmount(scope)?;
    host.state.get_mut().remove_signal(open)?;
    assert!(
        host.mount(component(|mount| {
            mount.signal(false);
            mount.signal(String::from("System"));
            section(open, || text("Detail"))
        }))
        .is_err()
    );
    assert_eq!(
        (
            host.state.get_mut().signal_count(),
            host.state.get_mut().binding_count()
        ),
        (0, 0)
    );
    Ok(())
}

#[test]
fn intrinsic_flex_basis_and_direct_probes_keep_separate_measurements() -> Result<()> {
    use crate::layout::Query;

    let mut ui = Ui::new();
    let view = ui.mount(component(|_| {
        div().child(div().h(px(70.0)).flex_basis(rem(2.0))).child(
            div()
                .h(px(99.0))
                .flex_basis(percent(50.0))
                .child(div().size(px(8.0))),
        )
    }))?;
    let instance = ui.instances.get_mut(&view).unwrap();
    let root = instance.scene.root();
    let absolute = instance.node_at(&[0]).unwrap();
    let relative = instance.node_at(&[1]).unwrap();
    assert_eq!(
        instance.scene.probe(absolute, Query::Natural)?.size.height,
        px(70.0)
    );
    assert_eq!(
        instance.scene.probe(root, Query::Natural)?.size.height,
        px(40.0)
    );
    assert_eq!(
        instance.scene.probe(relative, Query::Natural)?.size.height,
        px(99.0)
    );
    Ok(())
}

#[cfg(feature = "layout-audit")]
#[test]
fn audit_replays_the_basis_and_grown_width_used_to_measure_a_label() -> Result<()> {
    use crate::layout::{
        Query,
        validation::{CheckOutcome, CheckTarget},
    };

    let mut ui = test_ui();
    let view = ui.mount(component(|_| {
        div().flex_row().child(
            text("Audio settings")
                .w(px(160.0))
                .flex_basis(px(40.0))
                .max_w(px(100.0))
                .flex_grow(1.0),
        )
    }))?;
    let scene = &mut ui.instances.get_mut(&view).unwrap().scene;
    let query = Query::Exact(px(180.0));
    scene.run(query, None, false)?;
    let target = CheckTarget::Measure(query);
    let checkpoint = scene.capture_dependencies(scene.root(), target)?.unwrap();
    let report = scene.check_dependencies(&checkpoint, target)?;
    assert!(report.checked_reads >= 2);
    assert_eq!(report.outcome, CheckOutcome::Unchanged);
    Ok(())
}
