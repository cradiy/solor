use wayland_client::{
    QueueHandle, delegate_noop,
    protocol::{wl_compositor::WlCompositor, wl_region::WlRegion, wl_surface::WlSurface},
};

use super::{Desktop, Mapping};
use crate::layer_shell::InputRegion;

delegate_noop!(Desktop: ignore WlRegion);

#[derive(Default)]
pub(super) struct SurfaceInputRegion {
    current: InputRegion,
    changed: bool,
}

impl SurfaceInputRegion {
    pub(super) fn update(&mut self, region: Option<InputRegion>) {
        if let Some(region) = region
            && region != self.current
        {
            self.current = region;
            self.changed = true;
        }
    }

    pub(super) fn apply(
        &mut self,
        mapping: Mapping,
        compositor: &WlCompositor,
        surface: &WlSurface,
        qh: &QueueHandle<Desktop>,
        panel: Option<[i32; 4]>,
        placement_changed: bool,
    ) {
        let mapping_with_region = mapping == Mapping::Hidden && self.current != InputRegion::Full;
        if mapping == Mapping::Unmapping
            || !(self.changed || mapping_with_region || placement_changed)
        {
            return;
        }
        if self.current == InputRegion::Full && panel.is_none() {
            surface.set_input_region(None);
        } else {
            let region = compositor.create_region(qh, ());
            match &self.current {
                InputRegion::Full => {
                    if let Some([x, y, w, h]) = panel {
                        region.add(x, y, w, h);
                    }
                }
                InputRegion::Rects(rects) => {
                    for &rect in rects {
                        if let Some([x, y, w, h]) = translate_rect(rect, panel) {
                            region.add(x, y, w, h);
                        }
                    }
                }
            }
            surface.set_input_region(Some(&region));
            region.destroy();
        }
        // An idle mapped surface may never submit another buffer. During the
        // initial handshake the mapping or first buffer commit applies this state.
        if mapping == Mapping::Mapped {
            surface.commit();
        }
        self.changed = false;
    }
}

fn translate_rect(rect: [i32; 4], panel: Option<[i32; 4]>) -> Option<[i32; 4]> {
    let Some([ox, oy, pw, ph]) = panel else {
        return Some(rect);
    };
    let [x, y, w, h] = rect;
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + w).min(pw);
    let bottom = (y + h).min(ph);
    (left < right && top < bottom).then(|| [ox + left, oy + top, right - left, bottom - top])
}
