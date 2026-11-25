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
        io::copy(&mut peer.by_ref().take(sent * request_len), &mut io::sink())?;
        dispatch()?;
        let mut request = [0; 12];
        peer.read_exact(&mut request)?;
        assert_eq!(u32::from_ne_bytes(request[8..].try_into()?), pending);
        assert!(flush_interest(&queue)?.is_empty());
    }
    Ok(())
}

#[test]
fn oversized_surface_waits_while_a_second_window_keeps_submitting() -> anyhow::Result<()> {
    use solor::{
        layout::Size,
        platform::{EventResponse, WindowEvent},
        px,
        raster::{Clip, Damage, RasterFrame, RasterScale},
    };
    use std::io::Write;

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
    let fractional = events.desktop.windows[&large].fractional.id().protocol_id();
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
