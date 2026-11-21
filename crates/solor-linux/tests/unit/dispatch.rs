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
                None,
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
