use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, WaylandWindowHandle, WindowHandle,
};
use std::ptr::NonNull;
use wayland_backend::client::Backend;
use wayland_client::{Proxy as _, protocol::wl_surface::WlSurface};

/// Owns a Wayland surface and keeps its display alive while Vulkan presents to it.
pub struct WaylandWindow {
    backend: Backend,
    pub wl: WlSurface,
}

impl WaylandWindow {
    /// Takes ownership of a surface created on a live Wayland connection.
    pub fn new(wl: WlSurface) -> Self {
        let backend = wl.backend().upgrade().expect("the Wayland surface needs a live connection");
        Self { backend, wl }
    }
}

impl HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}

impl HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer = NonNull::new(self.wl.id().as_ptr().cast()).ok_or(HandleError::Unavailable)?;
        // SAFETY: the owned surface and display outlive the borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer))) })
    }
}

impl Drop for WaylandWindow {
    fn drop(&mut self) {
        self.wl.destroy();
    }
}
