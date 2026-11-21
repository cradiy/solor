use std::{
    io,
    os::fd::OwnedFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use super::{InputRect, InputRegion};

use rustix::{
    event::{EventfdFlags, eventfd},
    io::{Errno, retry_on_intr},
};

/// Thread-safe control of a single layer surface, including while its event loop is asleep.
#[derive(Clone, Debug)]
pub struct LayerSurfaceHandle(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    wake: OwnedFd,
    claimed: AtomicBool,
    desired: AtomicU8,
    pending_region: Mutex<Option<InputRegion>>,
}

const HIDDEN: u8 = 0;
const VISIBLE: u8 = 1;
const CLOSED: u8 = 2;

impl LayerSurfaceHandle {
    /// Creates a handle for one run and selects whether the surface should initially appear.
    pub fn new(visible: bool) -> io::Result<Self> {
        Ok(Self(Arc::new(Shared {
            wake: eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC)?,
            claimed: AtomicBool::new(false),
            desired: AtomicU8::new(u8::from(visible)),
            pending_region: Mutex::default(),
        })))
    }

    /// Requests the latest visibility. A hidden surface keeps its mounted component and signals.
    pub fn set_visible(&self, visible: bool) -> io::Result<()> {
        self.request(u8::from(visible))
    }

    /// Sets the surface-local pointer and touch input region. Use `None` for the whole surface
    /// or `Some(&[])` to pass all input through. Keyboard focus is controlled separately.
    /// Nonempty rectangles round outward to whole logical pixels. Invalid coordinates or
    /// extents return an error and preserve the previous request.
    pub fn set_input_region(&self, rects: Option<&[InputRect]>) -> io::Result<()> {
        let region = InputRegion::from_rects(rects)?;
        if self.0.desired.load(Ordering::Acquire) == CLOSED {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "layer surface closed",
            ));
        }
        *self
            .0
            .pending_region
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(region);
        self.wake()
    }

    /// Requests termination, including while hidden. Closing the surface is final.
    pub fn close(&self) -> io::Result<()> {
        self.request(CLOSED)
    }

    fn request(&self, desired: u8) -> io::Result<()> {
        let previous = self
            .0
            .desired
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state != CLOSED).then_some(desired)
            });
        match previous {
            Err(_) if desired == CLOSED => return Ok(()),
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "layer surface closed",
                ));
            }
            Ok(old) if old == desired => return Ok(()),
            _ => {}
        }
        self.wake()
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
                "layer surface handle already used",
            ));
        }
        Ok(Binding(&self.0))
    }
}

pub(crate) struct Binding<'a>(&'a Shared);

impl Binding<'_> {
    pub(crate) fn visibility(&self) -> Option<bool> {
        match self.0.desired.load(Ordering::Acquire) {
            HIDDEN => Some(false),
            VISIBLE => Some(true),
            _ => None,
        }
    }

    pub(crate) fn take_input_region(&self) -> Option<InputRegion> {
        self.0
            .pending_region
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
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
        self.0.desired.store(CLOSED, Ordering::Release);
    }
}
