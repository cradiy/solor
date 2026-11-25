//! Latest surface state requested by another thread.
mod input_region;
use crate::layer_shell::{LayerChanges, LayerRules};
pub(crate) use input_region::InputRegion;
pub(crate) use input_region::region_rectangles;
use rustix::{
    event::{EventfdFlags, eventfd},
    io::{Errno, retry_on_intr},
};
use solor::layout::Size;
pub use solor::platform::{SurfaceRect, SurfaceRect as InputRect};
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
    pub(crate) layer: LayerChanges,
    pub(crate) region: Option<InputRegion>,
    pub(crate) blur: Option<Vec<[i32; 4]>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Control(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    wake: OwnedFd,
    claimed: AtomicBool,
    update: Mutex<State>,
}

#[derive(Debug)]
struct State {
    pending: Update,
    rules: Option<LayerRules>,
}

impl Control {
    pub(crate) fn new(visible: bool) -> io::Result<Self> {
        Ok(Self(Arc::new(Shared {
            wake: eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC)?,
            claimed: AtomicBool::new(false),
            update: Mutex::new(State {
                rules: None,
                pending: Update {
                    layer: LayerChanges::default(),
                    visible: Some(visible),
                    size: None,
                    region: None,
                    blur: None,
                },
            }),
        })))
    }

    pub(crate) fn set_blur(&self, rects: &[SurfaceRect]) -> io::Result<()> {
        let rects = region_rectangles(rects)?;
        self.update(|state| {
            state.blur = Some(rects);
            true
        })
    }

    pub(crate) fn set_visible(&self, visible: bool) -> io::Result<()> {
        self.update(|state| state.visible.replace(visible) != Some(visible))
    }

    pub(crate) fn update(&self, edit: impl FnOnce(&mut Update) -> bool) -> io::Result<()> {
        self.edit(|state| Ok(edit(&mut state.pending)))
    }

    pub(crate) fn update_layer(&self, edit: impl FnOnce(&mut LayerChanges)) -> io::Result<()> {
        self.edit(|state| {
            let mut next = state.pending.layer;
            edit(&mut next);
            if let Some(rules) = state.rules {
                rules.validate(next)?;
            }
            let changed = state.pending.layer != next;
            state.pending.layer = next;
            Ok(changed)
        })
    }

    fn edit(&self, edit: impl FnOnce(&mut State) -> io::Result<bool>) -> io::Result<()> {
        let mut state = self.0.update.lock().expect("surface control lock");
        if state.pending.visible.is_none() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "surface closed"));
        }
        let changed = edit(&mut state)?;
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
            .pending
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

    pub(crate) fn claim(&self) -> io::Result<Binding> {
        if self.0.claimed.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "surface handle already used",
            ));
        }
        Ok(Binding(self.0.clone()))
    }
}

pub(crate) struct Binding(Arc<Shared>);

impl Binding {
    pub(crate) fn control(&self) -> Control {
        Control(self.0.clone())
    }

    pub(crate) fn set_layer_rules(&self, rules: LayerRules) -> io::Result<()> {
        let mut state = self.0.update.lock().expect("surface control lock");
        rules.validate(state.pending.layer)?;
        state.rules = Some(rules);
        Ok(())
    }

    pub(crate) fn take(&self) -> Update {
        let mut state = self.0.update.lock().expect("surface control lock");
        let state = &mut state.pending;
        Update {
            layer: std::mem::take(&mut state.layer),
            visible: state.visible,
            size: state.size.take(),
            region: state.region.take(),
            blur: state.blur.take(),
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

impl Drop for Binding {
    fn drop(&mut self) {
        self.0
            .update
            .lock()
            .expect("surface control lock")
            .pending
            .visible = None;
    }
}
