//! Wayland layer surfaces for panels, wallpapers and overlays.
use solor::{Edges, Pixels, platform::WindowBackground};

mod handle;
mod placement;
pub use handle::LayerSurfaceHandle;
pub(crate) use placement::{LayerChanges, LayerRules, extent, units};
pub use solor::platform::SurfaceRect;

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

/// Initial placement and input policy for a desktop layer surface.
#[derive(Clone, Copy, Debug)]
pub struct LayerShellOptions<'a> {
    /// Allows drag regions to move a fixed-size panel within the selected output.
    /// Movable panels cannot reserve desktop space. Anchors and margins select the initial position.
    pub movable: bool,
    /// Composition of the panel background with the desktop.
    pub background: WindowBackground,
    /// Optional handle for controlling visibility and closing this surface from any thread.
    pub handle: Option<&'a LayerSurfaceHandle>,
    /// Stable name used by compositor rules to identify the surface.
    pub namespace: &'a str,
    /// Wayland output name, such as `DP-1`. By default the compositor chooses the output.
    pub output: Option<&'a str>,
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
            movable: false,
            background: WindowBackground::Opaque,
            handle: None,
            namespace,
            output: None,
            layer,
            anchor: Anchor::Top | Anchor::Right | Anchor::Bottom | Anchor::Left,
            width: None,
            height: None,
            margin: Edges::default(),
            exclusive_zone: ExclusiveZone::Avoid,
            keyboard_interactivity: KeyboardInteractivity::None,
        }
    }

    /// Enables per-pixel transparency for the panel content.
    pub const fn transparent(mut self) -> Self {
        self.background = WindowBackground::Transparent;
        self
    }
}
