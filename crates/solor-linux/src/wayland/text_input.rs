use super::{Desktop, Mapping, Window};
use solor::{
    platform::{PlatformWindow, WindowEvent},
    text::{Preedit, SurroundingText, TextInputEvent, TextSelection},
};
use std::collections::HashMap;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, backend::ObjectId, delegate_noop};
pub(super) use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3 as Manager;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose, Event, ZwpTextInputV3,
};

delegate_noop!(Desktop: ignore Manager);

const MAX_SURROUNDING_BYTES: usize = 4000;

#[derive(Default)]
struct Batch {
    delete: Option<(usize, usize)>,
    commit: Option<String>,
    preedit: Option<Preedit>,
}

fn preedit(text: Option<String>, begin: i32, end: i32) -> Preedit {
    let text = text.unwrap_or_default();
    let offset = |byte: i32| text.floor_char_boundary((byte.max(0) as usize).min(text.len()));
    let selection = (begin != -1 || end != -1).then(|| TextSelection {
        head: offset(begin),
        anchor: offset(end),
    });
    Preedit { text, selection }
}

impl Batch {
    fn edits(self, composing: bool) -> impl Iterator<Item = TextInputEvent> {
        let clear = composing && self.commit.is_none() && self.preedit.is_none();
        [
            self.delete
                .map(|(before, after)| TextInputEvent::DeleteSurrounding { before, after }),
            self.commit.map(TextInputEvent::Commit),
            self.preedit
                .or_else(|| clear.then(|| preedit(None, -1, -1)))
                .map(TextInputEvent::Preedit),
        ]
        .into_iter()
        .flatten()
    }
}

pub(super) struct SeatInput {
    device: ZwpTextInputV3,
    seat: u32,
    surface: Option<ObjectId>,
    session: Option<u64>,
    serial: u32,
    enabled_serial: u32,
    surrounding: Option<SurroundingText<'static>>,
    rectangle: Option<[i32; 4]>,
    defer_publish: bool,
    batch: Batch,
}

impl SeatInput {
    pub(super) fn new(device: ZwpTextInputV3, seat: u32) -> Self {
        Self {
            device,
            seat,
            surface: None,
            session: None,
            serial: 0,
            enabled_serial: 0,
            surrounding: None,
            rectangle: None,
            defer_publish: false,
            batch: Batch::default(),
        }
    }

    fn commit_state(&mut self) {
        self.device.commit();
        self.serial = self.serial.wrapping_add(1);
    }

    fn window<'a>(&self, windows: &'a mut HashMap<ObjectId, Window>) -> Option<&'a mut Window> {
        windows.get_mut(self.surface.as_ref()?)
    }

    fn target(window: &Window) -> Option<u64> {
        if window.mapping == Mapping::Mapped && window.life.is_open() && !window.life.blocked() {
            window.view.text_input_session()
        } else {
            None
        }
    }

    pub(super) fn cancel(&self, windows: &mut HashMap<ObjectId, Window>) {
        if let Some(window) = self.window(windows) {
            self.cancel_window(window);
        }
    }

    fn cancel_window(&self, window: &mut Window) {
        if let Some(session) = self.session
            && window.preedit_owner == Some((self.seat, session))
        {
            window.preedit_owner = None;
            if window.view.text_input_session() == Some(session) {
                window.event(WindowEvent::TextInput(TextInputEvent::Preedit(preedit(
                    None, -1, -1,
                ))));
            }
        }
    }

    fn sync(&mut self, windows: &mut HashMap<ObjectId, Window>) {
        let next = self.window(windows).and_then(|w| Self::target(w));
        let changed = self.session != next;
        if changed {
            if self.session.is_some() {
                self.cancel(windows);
                self.device.disable();
                self.commit_state();
            }
            self.batch = Batch::default();
            self.session = next;
            self.surrounding = None;
            self.rectangle = None;
            self.defer_publish = false;
        }
        if next.is_some()
            && !self.defer_publish
            && let Some(window) = self.window(windows)
        {
            self.publish(window, changed, ChangeCause::Other);
        }
    }

    fn publish(&mut self, window: &mut Window, enable: bool, cause: ChangeCause) {
        let rectangle = window.view.caret_bounds().map(|b| {
            let [dx, dy, ..] = window
                .floating
                .as_ref()
                .map_or([0; 4], |p| p.geometry.bounds());
            let x = b.origin.x.get().floor() as i32;
            let y = b.origin.y.get().floor() as i32;
            [
                x.saturating_add(dx),
                y.saturating_add(dy),
                (((b.origin.x + b.size.width).get().ceil() as i32).saturating_sub(x)).max(1),
                (((b.origin.y + b.size.height).get().ceil() as i32).saturating_sub(y)).max(1),
            ]
        });
        let snapshot = Self::context(window);
        let context_changed = snapshot != self.surrounding;
        let position_changed = rectangle.is_some() && rectangle != self.rectangle;
        if !enable && !context_changed && !position_changed {
            return;
        }
        let reset = !enable
            && (snapshot.is_some() != self.surrounding.is_some()
                || (position_changed && self.rectangle.is_none()));
        let snapshot = if reset {
            self.cancel_window(window);
            self.device.disable();
            self.commit_state();
            Self::context(window)
        } else {
            snapshot
        };
        if enable || reset {
            self.device.enable();
            self.device
                .set_content_type(ContentHint::None, ContentPurpose::Normal);
        }
        if enable || reset || context_changed {
            self.device.set_text_change_cause(cause);
            if let Some(context) = &snapshot {
                let selection = context.selection();
                self.device.set_surrounding_text(
                    context.text().to_owned(),
                    selection.head as i32,
                    selection.anchor as i32,
                );
            }
            self.surrounding = snapshot.map(SurroundingText::into_owned);
        }
        if (enable || reset || rectangle != self.rectangle)
            && let Some([x, y, w, h]) = rectangle
        {
            self.device.set_cursor_rectangle(x, y, w, h);
        }
        if enable || reset || position_changed {
            self.rectangle = rectangle;
        }
        self.commit_state();
        if enable || reset {
            self.enabled_serial = self.serial;
        }
    }

    fn take_batch(&mut self, serial: u32, target: Option<u64>) -> Option<(u64, Batch)> {
        let batch = std::mem::take(&mut self.batch);
        let session = self.session?;
        let current_session = serial.wrapping_sub(self.enabled_serial)
            <= self.serial.wrapping_sub(self.enabled_serial);
        (current_session && target == Some(session)).then_some((session, batch))
    }

    fn context(window: &mut Window) -> Option<SurroundingText<'_>> {
        window
            .view
            .surrounding_text(MAX_SURROUNDING_BYTES)
            .filter(|snapshot| snapshot.text().len() <= MAX_SURROUNDING_BYTES)
    }

    fn accepts_deletion(
        &self,
        serial: u32,
        current: Option<SurroundingText<'_>>,
        before: usize,
        after: usize,
    ) -> bool {
        serial == self.serial
            && self.surrounding.as_ref().is_some_and(|published| {
                published.permits_deletion(before, after) && current.as_ref() == Some(published)
            })
    }

    fn done(&mut self, serial: u32, windows: &mut HashMap<ObjectId, Window>) {
        let window = self.window(windows);
        let target = window.as_deref().and_then(Self::target);
        let (Some(window), Some((session, batch))) = (window, self.take_batch(serial, target))
        else {
            return;
        };
        let owner = (self.seat, session);
        self.defer_publish = serial != self.serial;
        if let Some((before, after)) = batch.delete
            && !self.accepts_deletion(serial, Self::context(window), before, after)
        {
            return;
        }
        for edit in batch.edits(window.preedit_owner == Some(owner)) {
            if Self::target(window) != Some(session) {
                break;
            }
            if matches!(&edit, TextInputEvent::Preedit(p) if p.text.is_empty())
                && window.preedit_owner != Some(owner)
            {
                continue;
            }
            window.preedit_owner = match &edit {
                TextInputEvent::Preedit(p) if !p.text.is_empty() => Some(owner),
                _ => None,
            };
            window.event(WindowEvent::TextInput(edit));
        }
        if !self.defer_publish && Self::target(window) == Some(session) {
            self.publish(window, false, ChangeCause::InputMethod);
        }
    }
}

impl Drop for SeatInput {
    fn drop(&mut self) {
        self.device.destroy();
    }
}

impl Desktop {
    pub(super) fn sync_text_inputs(&mut self) {
        for input in self.seats.iter_mut().filter_map(|s| s.text_input.as_mut()) {
            input.sync(&mut self.windows);
        }
    }
}

impl Dispatch<ZwpTextInputV3, u32> for Desktop {
    fn event(
        desktop: &mut Self,
        device: &ZwpTextInputV3,
        event: Event,
        _: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(input) = desktop
            .seats
            .iter_mut()
            .filter_map(|s| s.text_input.as_mut())
            .find(|i| i.device == *device)
        else {
            return;
        };
        match event {
            Event::Enter { surface } => {
                input.surface = Some(surface.id());
                input.sync(&mut desktop.windows);
            }
            Event::Leave { surface } if input.surface.as_ref() == Some(&surface.id()) => {
                input.cancel(&mut desktop.windows);
                input.surface = None;
                input.session = None;
                input.surrounding = None;
                input.batch = Batch::default();
            }
            Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                input.batch.preedit = Some(preedit(text, cursor_begin, cursor_end));
            }
            Event::CommitString { text } => input.batch.commit = text,
            Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                input.batch.delete = (before_length != 0 || after_length != 0)
                    .then_some((before_length as usize, after_length as usize));
            }
            Event::Done { serial } => input.done(serial, &mut desktop.windows),
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/text_input.rs"]
mod tests;
