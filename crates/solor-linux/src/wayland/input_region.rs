use wayland_client::{
    QueueHandle, delegate_noop,
    protocol::{wl_compositor::WlCompositor, wl_region::WlRegion, wl_surface::WlSurface},
};

use super::{Mapping, Window};
use crate::layer_shell::InputRegion;

delegate_noop!(Window: ignore WlRegion);

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
        qh: &QueueHandle<Window>,
    ) {
        let mapping_with_region = mapping == Mapping::Hidden && self.current != InputRegion::Full;
        if mapping == Mapping::Unmapping || !(self.changed || mapping_with_region) {
            return;
        }
        match &self.current {
            InputRegion::Full => surface.set_input_region(None),
            InputRegion::Rects(rects) => {
                let region = compositor.create_region(qh, ());
                for &[x, y, width, height] in rects {
                    region.add(x, y, width, height);
                }
                surface.set_input_region(Some(&region));
                region.destroy();
            }
        }
        // An idle mapped surface may never submit another buffer. During the
        // initial handshake the mapping or first buffer commit applies this state.
        if mapping == Mapping::Mapped {
            surface.commit();
        }
        self.changed = false;
    }
}
