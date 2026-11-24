use super::Desktop;
use anyhow::{Context as _, Result};
use solor::clipboard::{ClipboardRead, PlatformClipboard};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    os::fd::AsFd,
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop, event_created_child,
    globals::GlobalList,
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_seat::WlSeat,
    },
};

mod transfers;
use transfers::Transfers;

const MIME: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

#[derive(Default)]
struct TextFormat(Mutex<Option<(usize, String)>>);

impl TextFormat {
    fn offer(&self, mime: String) {
        let rank = if let Some((base, params)) = mime.split_once(';') {
            let Some((key, value)) = params.split_once('=') else {
                return;
            };
            if !base.trim().eq_ignore_ascii_case("text/plain")
                || !key.trim().eq_ignore_ascii_case("charset")
                || !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
            {
                return;
            }
            0
        } else if mime == "UTF8_STRING" {
            1
        } else if mime.eq_ignore_ascii_case("text/plain") {
            2
        } else {
            return;
        };
        let mut best = self.0.lock().expect("clipboard offer lock");
        if best.as_ref().is_none_or(|(prior, _)| rank < *prior) {
            *best = Some((rank, mime));
        }
    }
}

struct Seat {
    device: WlDataDevice,
    selection: Option<Offer>,
    source: RefCell<Option<Source>>,
    drag: Option<Offer>,
    focus: Option<(ObjectId, Rc<()>)>,
    serial: Option<u32>,
}

impl Drop for Seat {
    fn drop(&mut self) {
        if self.device.version() >= 2 {
            self.device.release();
        }
    }
}

struct Source(WlDataSource);

impl Drop for Source {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

struct Offer(WlDataOffer);
impl Drop for Offer {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

pub(super) struct Clipboard {
    manager: WlDataDeviceManager,
    queue: QueueHandle<Desktop>,
    seats: RefCell<HashMap<u32, Seat>>,
    active: Cell<Option<u32>>,
    pub(super) transfers: RefCell<Transfers>,
}

impl Clipboard {
    pub(super) fn bind(globals: &GlobalList, queue: &QueueHandle<Desktop>) -> Option<Rc<Self>> {
        Some(Rc::new(Self {
            manager: globals.bind(queue, 1..=3, ()).ok()?,
            queue: queue.clone(),
            seats: RefCell::default(),
            active: Default::default(),
            transfers: RefCell::default(),
        }))
    }
    pub(super) fn add_seat(&self, name: u32, seat: &WlSeat) {
        let device = self.manager.get_data_device(seat, &self.queue, name);
        self.seats.borrow_mut().insert(
            name,
            Seat {
                device,
                selection: None,
                source: RefCell::default(),
                drag: None,
                focus: None,
                serial: None,
            },
        );
    }
    pub(super) fn remove_seat(&self, name: u32) {
        self.seats.borrow_mut().remove(&name);
    }
    pub(super) fn focus(&self, name: u32, surface: Option<ObjectId>) {
        if let Some(seat) = self.seats.borrow_mut().get_mut(&name) {
            seat.focus = surface.map(|id| (id, Rc::new(())));
            seat.serial = None;
            if seat.focus.is_some() {
                self.active.set(Some(name));
            } else {
                seat.selection = None;
            }
        }
    }
    pub(super) fn input(&self, name: u32, surface: &ObjectId, serial: u32) {
        if let Some(seat) = self.seats.borrow_mut().get_mut(&name)
            && seat.focus.as_ref().is_some_and(|(id, _)| id == surface)
        {
            self.active.set(Some(name));
            seat.serial = Some(serial);
        }
    }
    pub(super) fn window(self: &Rc<Self>, id: ObjectId) -> Rc<dyn PlatformClipboard> {
        Rc::new(WindowClipboard(Rc::downgrade(self), id))
    }
}

struct WindowClipboard(Weak<Clipboard>, ObjectId);
impl WindowClipboard {
    fn with_seat<T>(&self, f: impl FnOnce(&Clipboard, &Seat) -> Result<T>) -> Result<T> {
        let hub = self.0.upgrade().context("clipboard closed")?;
        let seats = hub.seats.borrow();
        let focused = |s: &&Seat| s.focus.as_ref().is_some_and(|(id, _)| id == &self.1);
        let seat = hub
            .active
            .get()
            .and_then(|name| seats.get(&name))
            .filter(focused)
            .or_else(|| seats.values().find(focused))
            .context("clipboard requires keyboard focus")?;
        f(&hub, seat)
    }
}

impl PlatformClipboard for WindowClipboard {
    fn write_text(&self, text: String) -> Result<()> {
        self.with_seat(|hub, seat| {
            let serial = seat
                .serial
                .context("clipboard write requires an input serial")?;
            let source = hub
                .manager
                .create_data_source(&hub.queue, Arc::<str>::from(text));
            for mime in MIME {
                source.offer(mime.to_owned());
            }
            seat.device.set_selection(Some(&source), serial);
            seat.source.replace(Some(Source(source)));
            Ok(())
        })
    }
    fn read_text(&self, max_bytes: Option<usize>) -> Result<ClipboardRead> {
        self.with_seat(|hub, seat| -> Result<ClipboardRead> {
            let Some((offer, mime)) = seat.selection.as_ref().and_then(|offer| {
                let format = offer
                    .0
                    .data::<TextFormat>()?
                    .0
                    .lock()
                    .expect("clipboard offer lock");
                Some((&offer.0, format.as_ref()?.1.clone()))
            }) else {
                return Ok(Box::pin(async { Ok(None) }));
            };
            let focus = Rc::downgrade(&seat.focus.as_ref().unwrap().1);
            let (write, result) = hub.transfers.borrow_mut().read(focus.clone(), max_bytes)?;
            offer.receive(mime, write.as_fd());
            Ok(Box::pin(async move {
                let text = result.await;
                if focus.strong_count() == 0 {
                    return Ok(None);
                }
                Ok(Some(text.context("clipboard transfer cancelled")??))
            }))
        })
    }
}

delegate_noop!(Desktop: ignore WlDataDeviceManager);
impl Dispatch<WlDataOffer, TextFormat> for Desktop {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        formats: &TextFormat,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            formats.offer(mime_type);
        }
    }
}
impl Dispatch<WlDataDevice, u32> for Desktop {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(hub) = &state.clipboard else {
            return;
        };
        let mut seats = hub.seats.borrow_mut();
        let seat = seats.get_mut(name);
        match event {
            wl_data_device::Event::Selection { id } => {
                let offer = id.map(Offer);
                if let Some(s) = seat {
                    s.selection = offer;
                }
            }
            wl_data_device::Event::Enter { id, .. } => {
                let offer = id.map(Offer);
                if let Some(s) = seat {
                    s.drag = offer;
                }
            }
            wl_data_device::Event::Leave | wl_data_device::Event::Drop => {
                if let Some(s) = seat {
                    s.drag = None;
                }
            }
            _ => {}
        }
    }
    event_created_child!(Desktop, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, TextFormat::default())
    ]);
}
impl Dispatch<WlDataSource, Arc<str>> for Desktop {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        text: &Arc<str>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } if MIME.contains(&mime_type.as_str()) => {
                if let Some(hub) = &state.clipboard
                    && let Err(e) = hub.transfers.borrow_mut().write(fd, text.clone())
                {
                    log::warn!("clipboard transfer failed: {e}");
                }
            }
            wl_data_source::Event::Cancelled => {
                if let Some(hub) = &state.clipboard {
                    for seat in hub.seats.borrow().values() {
                        if seat
                            .source
                            .borrow()
                            .as_ref()
                            .is_some_and(|owned| &owned.0 == source)
                        {
                            seat.source.take();
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
