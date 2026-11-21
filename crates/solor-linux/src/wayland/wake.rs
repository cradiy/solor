//! Wakes the event thread when a foreground task becomes runnable.
use rustix::{
    event::{EventfdFlags, eventfd},
    io::{Errno, retry_on_intr},
};
use std::{io, os::fd::OwnedFd, sync::Arc, task::Wake};

pub(super) struct EventWake(pub(super) OwnedFd);

impl EventWake {
    pub(super) fn new() -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self(eventfd(
            0,
            EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC,
        )?)))
    }

    pub(super) fn drain(&self) -> io::Result<()> {
        retry_on_intr(|| rustix::io::read(&self.0, &mut [0; 8]))?;
        Ok(())
    }
}

impl Wake for EventWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        // A saturated counter is already readable.
        let result = retry_on_intr(|| rustix::io::write(&self.0, &1_u64.to_ne_bytes()));
        debug_assert!(result.is_ok() || result == Err(Errno::AGAIN));
    }
}
