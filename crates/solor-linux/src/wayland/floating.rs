//! A stationary layer surface receives input in output-local coordinates. Its
//! synchronized subsurface carries only the panel pixels and moves on parent commit.
use std::{fs::File, os::fd::AsFd};

use anyhow::Result;
use rustix::fs::{MemfdFlags, memfd_create};
use solor::{
    layout::{Origin, Size},
    pointer::{PointerButton, PointerId, PointerInput},
    px,
};
use wayland_client::{
    QueueHandle, delegate_noop,
    globals::GlobalList,
    protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_shm,
        wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface, wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use super::Desktop;
use crate::layer_shell::Anchor;

delegate_noop!(Desktop: ignore WlSubcompositor);
delegate_noop!(Desktop: ignore WlSubsurface);
delegate_noop!(Desktop: ignore WlBuffer);

pub(super) struct FloatingPanel {
    pub content: WlSurface,
    child: WlSubsurface,
    viewport: WpViewport,
    pixel: WlBuffer,
    pub geometry: Geometry,
    placement_changed: bool,
}

impl FloatingPanel {
    pub fn new(
        globals: &GlobalList,
        compositor: &WlCompositor,
        shm: &wl_shm::WlShm,
        viewporter: &WpViewporter,
        parent: &WlSurface,
        geometry: Geometry,
        qh: &QueueHandle<Desktop>,
    ) -> Result<Self> {
        let manager: WlSubcompositor = globals.bind(qh, 1..=1, ())?;
        let file = File::from(memfd_create("solor-transparent", MemfdFlags::CLOEXEC)?);
        file.set_len(4)?;
        let pool = shm.create_pool(file.as_fd(), 4, qh, ());
        let pixel = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, qh, ());
        pool.destroy();
        let content = compositor.create_surface(qh, ());
        let child = manager.get_subsurface(&content, parent, qh, ());
        manager.destroy();
        let empty = compositor.create_region(qh, ());
        content.set_input_region(Some(&empty));
        empty.destroy();
        Ok(Self {
            content,
            child,
            viewport: viewporter.get_viewport(parent, qh, ()),
            pixel,
            geometry,
            placement_changed: true,
        })
    }

    pub fn configure(&mut self, width: u32, height: u32) -> Size {
        self.geometry.configure(width, height);
        let [w, h] = self.geometry.available;
        self.viewport.set_destination(w, h);
        self.placement_changed = true;
        let [_, _, w, h] = self.geometry.bounds();
        Size {
            width: px(w as f32),
            height: px(h as f32),
        }
    }

    pub fn pointer(&mut self, input: &mut PointerInput) {
        let before = self.geometry.position;
        self.geometry.pointer(input);
        self.placement_changed |= before != self.geometry.position;
    }

    pub fn apply_placement(&mut self) -> bool {
        if !std::mem::take(&mut self.placement_changed) {
            return false;
        }
        let [x, y] = self.geometry.position;
        self.child.set_position(x, y);
        true
    }

    pub fn map(&self, parent: &WlSurface) {
        parent.attach(Some(&self.pixel), 0, 0);
        parent.damage_buffer(0, 0, 1, 1);
    }
}

impl Drop for FloatingPanel {
    fn drop(&mut self) {
        self.child.destroy();
        self.content.destroy();
        self.viewport.destroy();
        self.pixel.destroy();
    }
}

pub(super) struct Geometry {
    requested: [i32; 2],
    available: [i32; 2],
    position: [i32; 2],
    anchor: Anchor,
    margin: [i32; 4],
    grab: Option<(PointerId, Origin)>,
}

impl Geometry {
    pub fn new(size: Size, anchor: Anchor, margin: [i32; 4]) -> Self {
        Self {
            requested: [size.width.get() as i32, size.height.get() as i32],
            available: [0, 0],
            position: [0, 0],
            anchor,
            margin,
            grab: None,
        }
    }

    fn configure(&mut self, width: u32, height: u32) {
        let first = self.available == [0, 0];
        let [top, right, bottom, left] = self.margin;
        for (i, (extent, near, far, start, end)) in [
            (width, Anchor::Left, Anchor::Right, left, right),
            (height, Anchor::Top, Anchor::Bottom, top, bottom),
        ]
        .into_iter()
        .enumerate()
        {
            if extent > 0 {
                self.available[i] = extent.min(i32::MAX as u32) as i32;
            } else if first {
                self.available[i] = self.requested[i];
            }
            let limit = (self.available[i] - self.requested[i]).max(0);
            if first {
                self.position[i] = match (self.anchor.contains(near), self.anchor.contains(far)) {
                    (true, false) => start,
                    (false, true) => limit.saturating_sub(end),
                    (true, true) => ((i64::from(limit) + i64::from(start) - i64::from(end)) / 2)
                        .clamp(0, i64::from(limit)) as i32,
                    (false, false) => limit / 2,
                };
            }
            self.position[i] = self.position[i].clamp(0, limit);
        }
    }

    pub fn bounds(&self) -> [i32; 4] {
        let [x, y] = self.position;
        [
            x,
            y,
            self.requested[0].min(self.available[0]),
            self.requested[1].min(self.available[1]),
        ]
    }

    pub fn begin(&mut self, id: PointerId, local: Origin) {
        self.grab.get_or_insert((id, local));
    }

    fn pointer(&mut self, input: &mut PointerInput) {
        use PointerInput::*;
        match *input {
            Move(p)
                if let Some((owner, grab)) = self.grab
                    && owner == p.id =>
            {
                for (i, delta) in [p.position.x - grab.x, p.position.y - grab.y]
                    .into_iter()
                    .enumerate()
                {
                    let limit = (self.available[i] - self.requested[i]).max(0);
                    self.position[i] = (delta.get().round() as i32).clamp(0, limit);
                }
            }
            Cancel(id)
            | Leave(id)
            | Up(solor::pointer::PointerSample { id, .. }, PointerButton::Primary)
                if self.grab.is_some_and(|(owner, _)| id == owner) =>
            {
                self.grab = None
            }
            _ => {}
        }
        if let Move(p) | Down(p, _) | Up(p, _) | Scroll(p, _) = input {
            p.position.x = p.position.x - px(self.position[0] as f32);
            p.position.y = p.position.y - px(self.position[1] as f32);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/floating.rs"]
mod tests;
