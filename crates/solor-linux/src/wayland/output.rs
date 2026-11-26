use super::{Desktop, Window};
use anyhow::{Context, Result};
use std::collections::HashMap;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::GlobalList,
    protocol::{wl_output, wl_registry::WlRegistry, wl_surface},
};

pub(super) struct NamedOutput(pub wl_output::WlOutput);

impl Drop for NamedOutput {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl NamedOutput {
    pub fn find(globals: &GlobalList, name: &str) -> Result<Self> {
        let registry = globals.registry();
        let connection = Connection::from_backend(
            registry
                .backend()
                .upgrade()
                .context("display disconnected")?,
        );
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        let outputs = globals
            .contents()
            .clone_list()
            .into_iter()
            .filter(|g| g.interface == "wl_output" && g.version >= 4)
            .map(|g| Self(registry.bind(g.name, 4, &qh, ())))
            .collect::<Vec<_>>();
        let mut search = Search {
            name: name.to_owned(),
            found: None,
        };
        queue.roundtrip(&mut search)?;
        outputs
            .into_iter()
            .find(|output| Some(&output.0) == search.found.as_ref())
            .with_context(|| {
                format!(
                    "Wayland output {name:?} is unavailable (named outputs require wl_output v4)"
                )
            })
    }
}

struct Search {
    name: String,
    found: Option<wl_output::WlOutput>,
}

impl Dispatch<wl_output::WlOutput, ()> for Search {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && name == state.name
        {
            state.found = Some(output.clone());
        }
    }
}

#[derive(Default)]
pub(super) struct OutputScales(pub(super) HashMap<u32, (wl_output::WlOutput, u32)>);

impl OutputScales {
    pub fn bind(
        &mut self,
        registry: &WlRegistry,
        name: u32,
        version: u32,
        qh: &QueueHandle<Desktop>,
    ) {
        self.0
            .entry(name)
            .or_insert_with(|| (registry.bind(name, version.min(3), qh, name), 120));
    }

    pub fn remove<'a>(&mut self, name: u32, windows: impl Iterator<Item = &'a mut Window>) {
        if let Some((output, _)) = self.0.remove(&name) {
            if output.version() >= 3 {
                output.release();
            }
            for window in windows.filter(|w| w.entered_outputs.contains(&name)) {
                window.entered_outputs.retain(|id| *id != name);
                self.rescale(window);
            }
        }
    }

    fn rescale(&self, window: &mut Window) {
        let scale = window
            .entered_outputs
            .iter()
            .filter_map(|id| self.0.get(id).map(|(_, scale)| *scale))
            .max()
            .unwrap_or(120);
        window.configure_raster(window.size, scale);
    }
}

impl Drop for OutputScales {
    fn drop(&mut self) {
        for (output, _) in self.0.values() {
            if output.version() >= 3 {
                output.release();
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for Desktop {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event
            && let Some(scale) = u32::try_from(factor)
                .ok()
                .and_then(|n| n.checked_mul(120))
                .filter(|n| *n > 0)
            && let Some(outputs) = &mut state.outputs
            && let Some((_, old)) = outputs.0.get_mut(name)
            && std::mem::replace(old, scale) != scale
        {
            for window in state
                .windows
                .values_mut()
                .filter(|w| w.entered_outputs.contains(name))
            {
                outputs.rescale(window);
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Desktop {
    fn event(
        state: &mut Self,
        surface: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(outputs) = &state.outputs else {
            return;
        };
        let Some(window) = state.windows.get_mut(&surface.id()) else {
            return;
        };
        let (entered, output) = match event {
            wl_surface::Event::Enter { output } => (true, output),
            wl_surface::Event::Leave { output } => (false, output),
            _ => return,
        };
        let Some(&name) = output.data::<u32>() else {
            return;
        };
        if outputs
            .0
            .get(&name)
            .is_none_or(|(bound, _)| bound != &output)
        {
            return;
        }
        if entered {
            if !window.entered_outputs.contains(&name) {
                window.entered_outputs.push(name);
            }
        } else {
            window.entered_outputs.retain(|id| *id != name);
        }
        outputs.rescale(window);
    }
}
