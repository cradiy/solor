//! Latest surface state requested by another thread.
mod input_region;
pub use input_region::InputRect;
pub(crate) use input_region::InputRegion;
use rustix::{
    event::{EventfdFlags, eventfd},
    io::{Errno, retry_on_intr},
};
use solor::layout::Size;
use std::{
    io,
    os::fd::OwnedFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug)]
pub(crate) struct Update {
    pub(crate) visible: Option<bool>,
    pub(crate) size: Option<Size>,
    pub(crate) region: Option<InputRegion>,
}

#[derive(Clone, Debug)]
pub(crate) struct Control(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    wake: OwnedFd,
    claimed: AtomicBool,
    update: Mutex<Update>,
}

impl Control {
    pub(crate) fn new(visible: bool) -> io::Result<Self> {
        Ok(Self(Arc::new(Shared {
            wake: eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC)?,
            claimed: AtomicBool::new(false),
            update: Mutex::new(Update {
                visible: Some(visible),
                size: None,
                region: None,
            }),
        })))
    }

    pub(crate) fn set_visible(&self, visible: bool) -> io::Result<()> {
        self.update(|state| state.visible.replace(visible) != Some(visible))
    }

    pub(crate) fn update(&self, edit: impl FnOnce(&mut Update) -> bool) -> io::Result<()> {
        let mut state = self.0.update.lock().expect("surface control lock");
        if state.visible.is_none() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "surface closed"));
        }
        let changed = edit(&mut state);
        drop(state);
        if changed {
            self.wake()?;
        }
        Ok(())
    }

    pub(crate) fn close(&self) -> io::Result<()> {
        if self
            .0
            .update
            .lock()
            .expect("surface control lock")
            .visible
            .take()
            .is_some()
        {
            self.wake()?;
        }
        Ok(())
    }

    fn wake(&self) -> io::Result<()> {
        match retry_on_intr(|| rustix::io::write(&self.0.wake, &1_u64.to_ne_bytes())) {
            Ok(_) | Err(Errno::AGAIN) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn claim(&self) -> io::Result<Binding<'_>> {
        if self.0.claimed.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "surface handle already used",
            ));
        }
        Ok(Binding(&self.0))
    }
}

pub(crate) struct Binding<'a>(&'a Shared);

impl Binding<'_> {
    pub(crate) fn take(&self) -> Update {
        let mut state = self.0.update.lock().expect("surface control lock");
        Update {
            visible: state.visible,
            size: state.size.take(),
            region: state.region.take(),
        }
    }

    pub(crate) fn fd(&self) -> &OwnedFd {
        &self.0.wake
    }

    pub(crate) fn drain(&self) -> io::Result<()> {
        retry_on_intr(|| rustix::io::read(self.fd(), &mut [0; 8]))
            .map(|_| ())
            .map_err(Into::into)
    }
}

impl Drop for Binding<'_> {
    fn drop(&mut self) {
        self.0.update.lock().expect("surface control lock").visible = None;
    }
}
