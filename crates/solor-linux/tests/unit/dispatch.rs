use super::*;
use crate::wayland::{Unmapped, wake::EventWake};
use std::{io::Read, os::unix::net::UnixStream, task::Wake};

#[test]
fn a_blocked_wayland_writer_services_wakes_and_resumes_on_writable() -> anyhow::Result<()> {
    for popup_only in [false, true] {
        let (socket, mut peer) = UnixStream::pair()?;
        peer.set_nonblocking(true)?;
        let connection = Connection::from_socket(socket)?;
        let mut queue = connection.new_event_queue::<Desktop>();
        let mut desktop = Desktop {
            popup_only,
            ..Default::default()
        };
        let wake = EventWake::new()?;
        let display = connection.display();
        let mut sent = 0;
        let (write_interest, pending) = loop {
            let callback = display.sync(&queue.handle(), Unmapped(display.id()));
            let interest = flush_interest(&queue)?;
            if interest.contains(PollFlags::OUT) {
                break (interest, callback.id().protocol_id());
            }
            sent += 1;
        };

        let mut dispatch = || {
            dispatch_until(
                &mut queue,
                &mut desktop,
                write_interest,
                Some(Instant::now()),
                &wake,
            )
        };
        wake.wake_by_ref();
        dispatch()?;
        assert_eq!(wake.drain().unwrap_err().kind(), io::ErrorKind::WouldBlock);

        // Leave the pending wl_display.sync request for the writable turn.
        let request_len = 12;
        io::copy(&mut (&mut peer).take(sent * request_len), &mut io::sink())?;
        dispatch()?;
        let mut request = [0; 12];
        peer.read_exact(&mut request)?;
        assert_eq!(u32::from_ne_bytes(request[8..].try_into()?), pending);
        assert!(flush_interest(&queue)?.is_empty());
    }
    Ok(())
}

use solor::{
    layout::Size,
    platform::{EventResponse, WindowEvent},
    px,
    raster::{Clip, Damage, RasterFrame, RasterScale},
};
use std::io::Write;

#[test]
fn a_failed_foreground_task_leaves_sibling_input_and_submission_live() -> anyhow::Result<()> {
    use std::{cell::OnceCell, time::Duration};
    use wayland_client::{WEnum, protocol::wl_seat};

    struct Probe {
        ui: solor::Ui,
        raster: Redraw,
        handle: Rc<OnceCell<WindowHandle>>,
        moved: bool,
        submitted: Rc<Cell<bool>>,
        deadline: Instant,
    }

    impl WindowHandler<Rgba8> for Probe {
        fn poll_tasks(&mut self, waker: &std::task::Waker) -> anyhow::Result<()> {
            self.ui.poll_tasks(waker)
        }
        fn event(&mut self, event: WindowEvent) -> anyhow::Result<EventResponse> {
            self.moved |= matches!(
                event,
                WindowEvent::Pointer(solor::pointer::PointerInput::Move(_))
            );
            Ok(EventResponse {
                redraw: self.moved,
                ..Default::default()
            })
        }
        fn next_redraw(&self) -> Option<Instant> {
            Some(self.deadline)
        }
        fn frame(&mut self, size: Size, scale: RasterScale) -> anyhow::Result<RasterFrame<'_>> {
            anyhow::ensure!(
                Instant::now() < self.deadline,
                "timed out waiting for pointer"
            );
            self.raster.frame(size, scale)
        }
        fn frame_submitted(&mut self) -> anyhow::Result<()> {
            if self.moved {
                self.submitted.set(true);
                self.handle.get().unwrap().close();
            }
            Ok(())
        }
    }

    let mut host = fixture::Host::new()?;
    let make = |ui, handle, submitted| Probe {
        ui,
        handle,
        submitted,
        moved: false,
        raster: Redraw::default(),
        deadline: Instant::now() + Duration::from_secs(3),
    };
    let mut ui = solor::Ui::new();
    ui.context()
        .spawn(|_| async { anyhow::bail!("settings task failed") });
    let failed = host.open(make(ui, Rc::default(), Rc::default()))?;
    let settings = host.desktop().windows[&failed].life.handle();
    let handle = Rc::new(OnceCell::new());
    let submitted = Rc::new(Cell::new(false));
    let live = host.open(make(solor::Ui::new(), handle.clone(), submitted.clone()))?;
    handle
        .set(host.desktop().windows[&live].life.handle())
        .unwrap();

    let seat = host.desktop().seats[0].seat.clone();
    host.deliver(
        &seat,
        wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(wl_seat::Capability::Pointer),
        },
    );
    let pointer = host.desktop().seats[0]
        .pointer
        .as_ref()
        .unwrap()
        .id()
        .protocol_id();
    let peer = host.peer.try_clone()?;
    let app = WaylandApplication(Rc::downgrade(&host.events.windows));
    app.on_window_error(move |window, error| {
        assert_eq!(window, settings);
        assert!(!window.is_open());
        assert_eq!(error.root_cause().to_string(), "settings task failed");
        // wl_pointer.enter on the surviving surface after the failure notification.
        let enter = [pointer, 24 << 16, 1, live.protocol_id(), 256, 256];
        for word in enter {
            (&peer).write_all(&word.to_ne_bytes()).unwrap();
        }
    })?;
    let error = host.events.dispatch().unwrap_err();
    assert_eq!(error.root_cause().to_string(), "settings task failed");
    assert!(submitted.get());
    Ok(())
}

#[derive(Default)]
struct Redraw(Vec<u8>);
impl WindowHandler<Rgba8> for Redraw {
    fn event(&mut self, _: WindowEvent) -> anyhow::Result<EventResponse> {
        Ok(Default::default())
    }
    fn next_redraw(&self) -> Option<Instant> {
        Some(Instant::now())
    }
    fn frame(&mut self, size: Size, scale: RasterScale) -> anyhow::Result<RasterFrame<'_>> {
        let size = scale.buffer_size(size)?;
        self.0
            .resize(size.width as usize * size.height as usize * 4, 255);
        Ok(RasterFrame {
            size,
            rgba: &self.0,
            damage: Damage::from_rect(Clip::canvas(size)),
            request_callback: false,
        })
    }
}

#[test]
fn oversized_surface_waits_while_a_second_window_keeps_submitting() -> anyhow::Result<()> {
    let mut host = fixture::Host::new()?;
    let large = host.open(Redraw::default())?;
    let small = host.open(Redraw::default())?;
    let events = &mut host.events;
    for (id, w, h) in [(&large, 1920., 1080.), (&small, 32., 16.)] {
        let window = events.desktop.windows.get_mut(id).unwrap();
        window.resize(Size {
            width: px(w),
            height: px(h),
        });
        window.set_visible(true, &events.connection, &events.windows.qh);
    }
    let fractional = events.desktop.windows[&large]
        .fractional
        .as_ref()
        .unwrap()
        .id()
        .protocol_id();
    for (scale, expected_large, expected_small) in [(150u32, 1, 1), (180, 1, 2), (150, 2, 3)] {
        host.peer.write_all(&fractional.to_ne_bytes())?;
        host.peer.write_all(&(12u32 << 16).to_ne_bytes())?;
        host.peer.write_all(&scale.to_ne_bytes())?;
        events.queue.blocking_dispatch(&mut events.desktop)?;
        for (id, window) in &mut events.desktop.windows {
            let next = window.present_if_due(&events.windows.compositor, &events.windows.qh)?;
            if id == &large && scale == 180 {
                assert!(next.is_none());
                assert!(
                    window
                        .present_if_due(&events.windows.compositor, &events.windows.qh)?
                        .is_none()
                );
                assert!(window.full_damage);
            } else {
                assert!(next.is_some());
                let slot = window.slots[1 - window.next_slot].as_ref().unwrap();
                host.peer
                    .write_all(&slot.handle.id().protocol_id().to_ne_bytes())?;
                host.peer.write_all(&(8u32 << 16).to_ne_bytes())?;
            }
        }
        assert_eq!(events.desktop.windows[&large].commits, expected_large);
        assert_eq!(events.desktop.windows[&small].commits, expected_small);
    }
    Ok(())
}

#[test]
fn windowless_application_drains_messages_from_a_disconnected_worker() -> anyhow::Result<()> {
    let mut host = fixture::Host::new()?;
    let app = WaylandApplication(Rc::downgrade(&host.events.windows));
    let received = Rc::new(RefCell::new(Vec::new()));
    let ui_thread = std::thread::current().id();
    let sender = app.channel(128, {
        let received = received.clone();
        move |app, message| {
            assert_eq!(std::thread::current().id(), ui_thread);
            assert!(app.factory().is_ok());
            received.borrow_mut().push(message);
        }
    })?;
    std::thread::spawn(move || {
        for i in 0..128 {
            sender.try_send(i).unwrap();
        }
    })
    .join()
    .unwrap();
    host.events.dispatch()?;
    assert_eq!(*received.borrow(), (0..128).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn lock_surfaces_follow_output_hotplug_and_fit_large_outputs() -> anyhow::Result<()> {
    let mut host = fixture::Host::new()?;
    let events = &mut host.events;
    let globals = events.windows.globals.as_ref().unwrap();
    let registry = globals.registry().id().protocol_id();
    fixture::advertise(
        &mut host.peer,
        registry,
        20,
        "ext_session_lock_manager_v1",
        1,
    )?;
    fixture::advertise(&mut host.peer, registry, 21, "wl_output", 3)?;
    events.queue.blocking_dispatch(&mut events.desktop)?;
    let app = WaylandApplication(Rc::downgrade(&events.windows));
    let built = Rc::new(RefCell::new(Vec::new()));
    app.lock_session(
        {
            let built = built.clone();
            move |id, _| {
                built.borrow_mut().push(id);
                Ok(Box::new(Redraw::default()))
            }
        },
        |_| {},
    )?;
    events.windows.locks.sync_outputs(globals);
    let mut first = events.windows.pending.borrow_mut().pop().unwrap();
    first.resize(Size {
        width: px(3840.),
        height: px(2160.),
    });
    let Role::Lock { raster_scale, .. } = first.role else {
        unreachable!()
    };
    let buffer = first.buffer_size.unwrap();
    assert!(u64::from(buffer.width) * u64::from(buffer.height) <= 4 * 1024 * 1024);
    assert!(
        first
            .view
            .buffer_size(raster_scale + 1, first.size)?
            .is_none()
    );

    fixture::advertise(&mut host.peer, registry, 22, "wl_output", 3)?;
    events.queue.blocking_dispatch(&mut events.desktop)?;
    events.windows.locks.sync_outputs(globals);
    let second = events.windows.pending.borrow_mut().pop().unwrap();
    host.peer.write_all(&registry.to_ne_bytes())?;
    host.peer.write_all(&((12u32 << 16) | 1).to_ne_bytes())?;
    host.peer.write_all(&21u32.to_ne_bytes())?;
    events.queue.blocking_dispatch(&mut events.desktop)?;
    events.windows.locks.sync_outputs(globals);
    assert_eq!(*built.borrow(), [21, 22]);
    assert!(!first.life.is_open() && second.life.is_open());
    first.destroy();
    second.destroy();
    Ok(())
}

#[test]
fn toplevel_follows_integer_outputs_when_scaling_extensions_are_incomplete() -> anyhow::Result<()> {
    use wayland_client::{
        Dispatch,
        protocol::{wl_output, wl_surface},
    };
    for omitted in [
        &["wp_viewporter", "wp_fractional_scale_manager_v1"][..],
        &["wp_fractional_scale_manager_v1"],
        &["wp_viewporter"],
    ] {
        let mut host = fixture::Host::without(omitted)?;
        let factory = &host.events.windows;
        factory.open(
            Request::Toplevel(WindowOptions::new("Scale").size(px(80.), px(40.))),
            None,
            |_| Ok(Box::new(Redraw::default())),
        )?;
        let window = factory.pending.take().pop().unwrap();
        let surface = window.surface.clone();
        let id = surface.id();
        host.events.desktop.windows.insert(id.clone(), window);
        let output = host.desktop().outputs.as_ref().unwrap().0[&8].0.clone();
        host.deliver(&output, wl_output::Event::Scale { factor: 2 });
        for (event, expected_width) in [
            (
                wl_surface::Event::Enter {
                    output: output.clone(),
                },
                160,
            ),
            (
                wl_surface::Event::Leave {
                    output: output.clone(),
                },
                80,
            ),
        ] {
            Desktop::event(
                &mut host.events.desktop,
                &surface,
                event,
                &(),
                &host.events.connection,
                &host.events.windows.qh,
            );
            let window = &host.events.desktop.windows[&id];
            assert_eq!(window.size.width, px(80.));
            assert_eq!(window.buffer_size.unwrap().width, expected_width);
        }
    }
    Ok(())
}

#[test]
fn window_state_is_published_once_per_confirmed_change() -> anyhow::Result<()> {
    use solor::platform::WindowState;
    use wayland_client::Dispatch;
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel};

    let mut host = fixture::Host::new()?;
    let events = &mut host.events;
    let factory = &events.windows;
    let handle = factory.open(Request::Toplevel(WindowOptions::new("State")), None, |_| {
        Ok(Box::new(Redraw::default()))
    })?;
    let window = factory.pending.take().pop().unwrap();
    let id = window.surface.id();
    let Role::Toplevel { top, surface, .. } = &window.role else {
        unreachable!()
    };
    let (top, xdg) = (top.clone(), surface.clone());
    events.desktop.windows.insert(id.clone(), window);
    let updates = Rc::new(RefCell::new(Vec::new()));
    handle.on_state_changed({
        let updates = updates.clone();
        let handle = handle.clone();
        move |state| {
            assert_eq!(handle.state(), Some(state));
            updates.borrow_mut().push(state);
        }
    });
    let active = xdg_toplevel::State::Activated as u32;
    let max = xdg_toplevel::State::Maximized as u32;
    let full = xdg_toplevel::State::Fullscreen as u32;
    for (serial, flags) in [
        vec![active],
        vec![active, max],
        vec![active, max],
        vec![full],
    ]
    .into_iter()
    .enumerate()
    {
        let old = handle.state();
        Desktop::event(
            &mut events.desktop,
            &top,
            xdg_toplevel::Event::Configure {
                width: 80,
                height: 40,
                states: flags.into_iter().flat_map(u32::to_ne_bytes).collect(),
            },
            &id,
            &events.connection,
            &factory.qh,
        );
        assert_eq!(handle.state(), old);
        Desktop::event(
            &mut events.desktop,
            &xdg,
            xdg_surface::Event::Configure {
                serial: serial as u32 + 1,
            },
            &id,
            &events.connection,
            &factory.qh,
        );
    }
    assert_eq!(
        &*updates.borrow(),
        &[
            WindowState {
                active: true,
                ..Default::default()
            },
            WindowState {
                active: true,
                maximized: true,
                fullscreen: false
            },
            WindowState {
                fullscreen: true,
                ..Default::default()
            },
        ]
    );
    Ok(())
}

#[test]
fn native_and_title_bar_close_requests_share_the_modal_confirmation() -> anyhow::Result<()> {
    use solor::platform::WindowControl;
    use wayland_client::Dispatch;
    use wayland_protocols::xdg::shell::client::xdg_toplevel;

    let mut host = fixture::Host::new()?;
    let events = &mut host.events;
    let app = WaylandApplication(Rc::downgrade(&events.windows));
    let parent = app.open_window(WindowOptions::new("Workspace"), Box::new(Redraw::default()))?;
    let window = events.windows.pending.take().pop().unwrap();
    let id = window.surface.id();
    let Role::Toplevel { top, .. } = &window.role else {
        unreachable!()
    };
    let top = top.clone();
    events.desktop.windows.insert(id.clone(), window);
    let dialog = Rc::new(RefCell::new(None));
    parent.on_should_close({
        let parent = parent.clone();
        let dialog = dialog.clone();
        move || {
            let modal = app
                .open_window(
                    WindowOptions::new("Confirm").modal_for(&parent),
                    Box::new(Redraw::default()),
                )
                .unwrap();
            assert!(dialog.replace(Some(modal)).is_none());
            false
        }
    });

    for _ in 0..2 {
        Desktop::event(
            &mut events.desktop,
            &top,
            xdg_toplevel::Event::Close,
            &id,
            &events.connection,
            &events.windows.qh,
        );
    }
    dialog.take().unwrap().close();
    assert!(parent.is_open());
    let window = events.desktop.windows.get_mut(&id).unwrap();
    window.window_control(WindowControl::Close, None);
    let confirmation = dialog.take().unwrap();
    assert!(confirmation.is_open() && parent.is_open());
    parent.close();
    assert!(!confirmation.is_open() && !parent.is_open());
    Ok(())
}
