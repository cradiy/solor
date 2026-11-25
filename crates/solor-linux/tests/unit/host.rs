//! Socket-backed windows using the production registry, protocols and window factory.
use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
};
use wayland_client::Dispatch;

pub(in crate::wayland) struct Host {
    pub(super) events: EventLoop,
    pub(super) peer: UnixStream,
}

impl Host {
    pub(in crate::wayland) fn new() -> anyhow::Result<Self> {
        let (socket, mut peer) = UnixStream::pair()?;
        let registry = std::thread::spawn(move || -> io::Result<_> {
            // Answer wl_display.get_registry and the initial wl_display.sync.
            let mut requests = [0; 24];
            peer.read_exact(&mut requests)?;
            let registry_id = &requests[8..12];
            for (i, (interface, version)) in [
                ("wl_compositor", 4u32),
                ("wl_shm", 1),
                ("wp_viewporter", 1),
                ("wp_fractional_scale_manager_v1", 1),
                ("zwp_text_input_manager_v3", 1),
                ("wl_seat", 5),
            ]
            .into_iter()
            .enumerate()
            {
                advertise(
                    &mut peer,
                    u32::from_ne_bytes(registry_id.try_into().unwrap()),
                    i as u32 + 1,
                    interface,
                    version,
                )?;
            }
            peer.write_all(&requests[20..24])?;
            peer.write_all(&(12u32 << 16).to_ne_bytes())?;
            peer.write_all(&0u32.to_ne_bytes())?;
            Ok(peer)
        });
        let events = EventLoop::new(&Wayland::default(), Connection::from_socket(socket)?, false)?;
        Ok(Self {
            events,
            peer: registry.join().unwrap()?,
        })
    }

    pub(in crate::wayland) fn open(
        &mut self,
        view: impl WindowHandler<Rgba8> + 'static,
    ) -> anyhow::Result<ObjectId> {
        let factory = &self.events.windows;
        factory.open(Request::Popup(Box::new(|_| Ok(Box::new(())))), None, |_| {
            Ok(Box::new(view))
        })?;
        let id = factory.pending.borrow().last().unwrap().surface.id();
        for window in factory.pending.take() {
            self.events
                .desktop
                .windows
                .insert(window.surface.id(), window);
        }
        Ok(id)
    }

    pub(in crate::wayland) fn desktop(&mut self) -> &mut Desktop {
        &mut self.events.desktop
    }
    pub(in crate::wayland) fn deliver<P: Proxy>(&mut self, proxy: &P, event: P::Event)
    where
        Desktop: Dispatch<P, u32>,
    {
        Desktop::event(
            &mut self.events.desktop,
            proxy,
            event,
            proxy.data::<u32>().unwrap(),
            &self.events.connection,
            &self.events.windows.qh,
        );
    }
}

pub(super) fn advertise(
    peer: &mut UnixStream,
    registry_id: u32,
    name: u32,
    interface: &str,
    version: u32,
) -> io::Result<()> {
    let len = interface.len() + 1;
    let padded = len.next_multiple_of(4);
    let mut event = Vec::from(registry_id.to_ne_bytes());
    event.extend_from_slice(&(((20 + padded) as u32) << 16).to_ne_bytes());
    event.extend_from_slice(&name.to_ne_bytes());
    event.extend_from_slice(&(len as u32).to_ne_bytes());
    event.extend_from_slice(interface.as_bytes());
    event.resize(16 + padded, 0);
    event.extend_from_slice(&version.to_ne_bytes());
    peer.write_all(&event)
}
