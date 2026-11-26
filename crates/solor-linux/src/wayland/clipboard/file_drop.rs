use super::{Clipboard, Desktop, Offer, OfferFormats};
use crate::wayland::{Mapping, Window};
use anyhow::Result;
use futures_channel::oneshot;
use solor::{NodeId, layout::Origin, platform::PlatformWindow, px};
use std::{
    os::{fd::AsFd, unix::ffi::OsStringExt},
    path::PathBuf,
    sync::atomic::Ordering,
};
use wayland_client::{Proxy, backend::ObjectId, protocol::wl_data_device_manager::DndAction};

pub(super) const MIME: &str = "text/uri-list";
pub(super) const COPY: DndAction = DndAction::Copy;

pub(in crate::wayland) struct Drag {
    offer: Offer,
    serial: u32,
    pub(super) surface: ObjectId,
    position: Origin,
    accepted: bool,
}

impl Drag {
    pub(super) fn enter(
        offer: Offer,
        serial: u32,
        surface: ObjectId,
        x: f64,
        y: f64,
        window: Option<&Window>,
    ) -> Option<Self> {
        if !offer
            .0
            .data::<OfferFormats>()?
            .files
            .load(Ordering::Relaxed)
        {
            offer.0.accept(serial, None);
            return None;
        }
        if offer.0.version() >= 3 {
            offer.0.set_actions(COPY, COPY);
        }
        let mut drag = Self {
            offer,
            serial,
            surface,
            position: Origin {
                x: px(x as f32),
                y: px(y as f32),
            },
            accepted: false,
        };
        drag.accept(drag.target(window).is_some());
        Some(drag)
    }

    fn target(&self, window: Option<&Window>) -> Option<NodeId> {
        let window = window.filter(|w| available(w))?;
        let mut local = self.position;
        if let Some(panel) = &window.floating {
            let [x, y, _, _] = panel.geometry.bounds();
            local.x = local.x - px(x as f32);
            local.y = local.y - px(y as f32);
        }
        window.view.file_drop_target(local)
    }

    fn accept(&mut self, accepted: bool) {
        self.offer
            .0
            .accept(self.serial, accepted.then(|| MIME.to_owned()));
        self.accepted = accepted;
    }

    pub(super) fn motion(&mut self, x: f64, y: f64, window: Option<&Window>) {
        self.position = Origin {
            x: px(x as f32),
            y: px(y as f32),
        };
        self.refresh(window);
    }

    fn refresh(&mut self, window: Option<&Window>) -> Option<NodeId> {
        let target = self.target(window);
        if target.is_some() != self.accepted {
            self.accept(target.is_some());
        }
        target
    }

    pub(super) fn receive(
        self,
        window: Option<&Window>,
        clipboard: &Clipboard,
    ) -> Result<Option<PendingDrop>> {
        let Some(target) = self.target(window).filter(|_| self.accepted) else {
            return Ok(None);
        };
        let (fd, result) = clipboard
            .transfers
            .borrow_mut()
            .read(None, Some(4 * 1024 * 1024))?;
        self.offer.0.receive(MIME.to_owned(), fd.as_fd());
        Ok(Some(PendingDrop {
            offer: self.offer,
            surface: self.surface,
            target,
            result,
        }))
    }
}

fn available(window: &Window) -> bool {
    window.failure.is_none()
        && window.life.is_open()
        && !window.life.blocked()
        && window.mapping == Mapping::Mapped
}

pub(in crate::wayland) struct PendingDrop {
    offer: Offer,
    surface: ObjectId,
    target: NodeId,
    result: oneshot::Receiver<Result<String>>,
}

impl Desktop {
    pub(in crate::wayland) fn sync_file_drop_hover(&mut self) -> bool {
        if self.file_drags.is_empty() && self.file_drop_hover.is_empty() {
            return false;
        }
        let mut current = smallvec::SmallVec::<[(ObjectId, NodeId); 1]>::new();
        for drag in self.file_drags.values_mut() {
            let target = drag.refresh(self.windows.get(&drag.surface));
            if let Some(target) = target
                && (drag.offer.0.version() < 3
                    || drag
                        .offer
                        .0
                        .data::<OfferFormats>()
                        .is_some_and(|f| f.copy.load(Ordering::Relaxed)))
            {
                let entry = (drag.surface.clone(), target);
                if !current.contains(&entry) {
                    current.push(entry);
                }
            }
        }
        let mut changed = false;
        for (from, to, hovered) in [
            (&self.file_drop_hover, &current, false),
            (&current, &self.file_drop_hover, true),
        ] {
            for entry in from {
                if !to.contains(entry)
                    && let Some(window) = self.windows.get_mut(&entry.0)
                {
                    window.event(solor::platform::WindowEvent::FileDropHover {
                        target: entry.1,
                        hovered,
                    });
                    changed = true;
                }
            }
        }
        self.file_drop_hover = current;
        changed
    }

    pub(in crate::wayland) fn deliver_file_drops(&mut self) {
        self.file_drops.retain_mut(|drop| {
            let Some(window) = self.windows.get_mut(&drop.surface).filter(|w| available(w)) else {
                return false;
            };
            match drop.result.try_recv() {
                Ok(None) => return true,
                Ok(Some(Ok(text))) => {
                    let paths = local_paths(&text);
                    if !paths.is_empty() {
                        match window.view.drop_files(drop.target, &paths) {
                            Ok(true) => {
                                window.dirty = true;
                                let proxy = &drop.offer.0;
                                if proxy.version() >= 3
                                    && proxy
                                        .data::<OfferFormats>()
                                        .is_some_and(|f| f.copy.load(Ordering::Relaxed))
                                {
                                    proxy.finish();
                                }
                            }
                            Ok(false) => {}
                            Err(error) => window.record_failure(error),
                        }
                    }
                }
                Ok(Some(Err(error))) => log::warn!("file drop transfer failed: {error}"),
                Err(_) => {}
            }
            false
        });
    }
}

fn local_paths(list: &str) -> Vec<PathBuf> {
    list.lines()
        .filter_map(|line| {
            let (scheme, rest) = line.split_once(':')?;
            if !scheme.eq_ignore_ascii_case("file") || rest.contains(['?', '#']) {
                return None;
            }
            let path = if let Some(authority) = rest.strip_prefix("//") {
                let (host, path) = authority.split_at(authority.find('/')?);
                if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                    return None;
                }
                path
            } else {
                rest
            };
            if !path.starts_with('/')
                || path.match_indices('%').any(|(i, _)| {
                    !path
                        .as_bytes()
                        .get(i + 1..i + 3)
                        .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
                })
            {
                return None;
            }
            let decoded = percent_encoding::percent_decode_str(path).collect::<Vec<_>>();
            if decoded.contains(&0) {
                return None;
            }
            Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/file_drop.rs"]
mod tests;
