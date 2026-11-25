use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, globals::GlobalList, protocol::wl_output,
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
