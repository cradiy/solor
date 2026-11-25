use super::{Desktop, Mapping, Window};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry},
};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{Capability, Event, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

pub(super) struct Manager {
    pub(super) name: u32,
    proxy: ExtBackgroundEffectManagerV1,
    supports_blur: bool,
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.proxy.destroy();
    }
}

impl Desktop {
    pub(super) fn bind_background_effects(
        &mut self,
        registry: &WlRegistry,
        name: u32,
        interface: &str,
        qh: &QueueHandle<Self>,
    ) {
        if self.effects.is_none() && interface == ExtBackgroundEffectManagerV1::interface().name {
            self.effects = Some(Manager {
                name,
                proxy: registry.bind(name, 1, qh, ()),
                supports_blur: false,
            });
        }
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Desktop {
    fn event(
        state: &mut Self,
        proxy: &ExtBackgroundEffectManagerV1,
        event: Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Event::Capabilities { flags } = event
            && let Some(manager) = state.effects.as_mut().filter(|m| m.proxy == *proxy)
        {
            let bits = match flags {
                WEnum::Value(flags) => flags.bits(),
                WEnum::Unknown(bits) => bits,
            };
            manager.supports_blur = bits & Capability::Blur.bits() != 0;
        }
    }
}

delegate_noop!(Desktop: ignore ExtBackgroundEffectSurfaceV1);

#[derive(Default)]
pub(super) struct SurfaceBlur {
    rects: Vec<[i32; 4]>,
    effect: Option<(u32, ExtBackgroundEffectSurfaceV1)>,
    dirty: bool,
}

impl Drop for SurfaceBlur {
    fn drop(&mut self) {
        if let Some((_, effect)) = self.effect.take() {
            effect.destroy();
        }
    }
}

impl Window {
    pub(super) fn set_blur(&mut self, rects: Vec<[i32; 4]>) {
        if rects.is_empty() && self.blur.is_none() {
            return;
        }
        let blur = self.blur.get_or_insert_with(Default::default);
        if blur.rects != rects {
            blur.rects = rects;
            blur.dirty = true;
        }
    }

    pub(super) fn apply_blur(
        &mut self,
        manager: Option<&Manager>,
        compositor: &WlCompositor,
        qh: &QueueHandle<Desktop>,
    ) -> bool {
        let Some(blur) = &mut self.blur else {
            return false;
        };
        if !matches!(self.mapping, Mapping::Mapped | Mapping::Ready) {
            return false;
        }
        let content = self.floating.as_ref().map_or(&self.surface, |p| &p.content);
        let manager = manager.filter(|m| m.supports_blur && !blur.rects.is_empty());
        let mut changed = false;
        if let Some((name, effect)) = &blur.effect
            && Some(*name) != manager.map(|m| m.name)
        {
            effect.destroy();
            blur.effect = None;
            changed = true;
        }
        if let Some(manager) = manager
            && (blur.dirty || blur.effect.is_none())
        {
            let (_, effect) = blur.effect.get_or_insert_with(|| {
                (
                    manager.name,
                    manager.proxy.get_background_effect(content, qh, ()),
                )
            });
            let region = compositor.create_region(qh, ());
            for &[x, y, w, h] in &blur.rects {
                region.add(x, y, w, h);
            }
            effect.set_blur_region(Some(&region));
            region.destroy();
            blur.dirty = false;
            changed = true;
        }
        if blur.rects.is_empty() {
            self.blur = None;
        }
        if changed && self.floating.is_some() && self.mapping == Mapping::Mapped {
            content.commit();
        }
        changed
    }
}
