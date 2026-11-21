use wayland_client::{
    QueueHandle, delegate_noop, globals::GlobalList, protocol::wl_surface::WlSurface,
};

use super::Window;
use protocol::{
    treeland_dde_shell_manager_v2::TreelandDdeShellManagerV2,
    treeland_dde_shell_surface_v2::{Role, TreelandDdeShellSurfaceV2},
};

// Generated from linuxdeepin/treeland-protocols at 5d28a8ac78a6f78f70ed46146159c8e8a06780cf.
mod protocol {
    use wayland_client::{self, protocol::*};

    mod __interfaces {
        use wayland_client::{backend as wayland_backend, protocol::__interfaces::*};
        wayland_scanner::generate_interfaces!("protocols/treeland-dde-shell-unstable-v2.xml");
    }
    use __interfaces::*;
    wayland_scanner::generate_client_code!("protocols/treeland-dde-shell-unstable-v2.xml");
}

pub(super) struct Topmost(TreelandDdeShellSurfaceV2);

impl Topmost {
    pub fn request(
        globals: &GlobalList,
        qh: &QueueHandle<Window>,
        surface: &WlSurface,
    ) -> Option<Self> {
        let manager: TreelandDdeShellManagerV2 = globals.bind(qh, 2..=2, ()).ok()?;
        let handle = manager.get_shell_surface(surface, qh, ());
        manager.destroy();
        handle.set_role(Role::AlwaysOnTop);
        Some(Self(handle))
    }
}

impl Drop for Topmost {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

delegate_noop!(Window: ignore TreelandDdeShellManagerV2);
delegate_noop!(Window: ignore TreelandDdeShellSurfaceV2);
