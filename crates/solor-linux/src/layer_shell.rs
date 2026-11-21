//! Wayland layer surfaces for panels, wallpapers and overlays.
use solor::{Edges, Pixels};

mod handle;
mod input_region;
pub(crate) use handle::Binding;
pub use handle::LayerSurfaceHandle;
pub use input_region::InputRect;
pub(crate) use input_region::InputRegion;

pub use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

/// How a surface shares space with other desktop surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ExclusiveZone {
    /// Respect space reserved by other surfaces.
    #[default]
    Avoid,
    /// Extend to the anchored edges, including space reserved by other surfaces.
    Ignore,
    /// Ask the compositor to reserve this distance from the anchored edge, including its margin.
    Reserve(Pixels),
}

/// Initial placement and input policy on a compositor-selected output.
#[derive(Clone, Copy, Debug)]
pub struct LayerShellOptions<'a> {
    /// Optional handle for controlling visibility and closing this surface from any thread.
    pub handle: Option<&'a LayerSurfaceHandle>,
    /// Stable name used by compositor rules to identify the surface.
    pub namespace: &'a str,
    /// Stacking layer relative to other desktop surfaces.
    pub layer: Layer,
    /// Output edges used for placement.
    pub anchor: Anchor,
    /// Preferred logical width. `None` stretches between left and right anchors.
    pub width: Option<Pixels>,
    /// Preferred logical height. `None` stretches between top and bottom anchors.
    pub height: Option<Pixels>,
    /// Signed logical offsets from the anchored edges.
    pub margin: Edges,
    /// Space reserved at the edge or placement relative to existing reservations.
    pub exclusive_zone: ExclusiveZone,
    /// Keyboard focus policy. On-demand focus requires layer-shell version 4.
    pub keyboard_interactivity: KeyboardInteractivity,
}

impl<'a> LayerShellOptions<'a> {
    /// Creates a surface spanning all four edges with no keyboard focus or reserved space.
    pub fn new(namespace: &'a str, layer: Layer) -> Self {
        Self {
            handle: None,
            namespace,
            layer,
            anchor: Anchor::Top | Anchor::Right | Anchor::Bottom | Anchor::Left,
            width: None,
            height: None,
            margin: Edges::default(),
            exclusive_zone: ExclusiveZone::Avoid,
            keyboard_interactivity: KeyboardInteractivity::None,
        }
    }
}
