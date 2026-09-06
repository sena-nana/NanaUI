//! Windows host-only DirectComposition tree for native content below NanaUI.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowsCompositionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl WindowsCompositionRect {
    fn valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .into_iter()
        .all(f32::is_finite)
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

#[derive(Debug)]
pub enum WindowsCompositionError {
    WindowHandle(String),
    Native(windows::core::Error),
    InvalidGeometry,
    Removed,
}
impl fmt::Display for WindowsCompositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowHandle(message) => {
                write!(f, "DirectComposition requires a Win32 window: {message}")
            }
            Self::Native(error) => write!(f, "DirectComposition: {error}"),
            Self::InvalidGeometry => {
                f.write_str("DirectComposition geometry must be finite with nonnegative size")
            }
            Self::Removed => f.write_str("DirectComposition visual has been removed"),
        }
    }
}
impl std::error::Error for WindowsCompositionError {}
impl From<windows::core::Error> for WindowsCompositionError {
    fn from(error: windows::core::Error) -> Self {
        Self::Native(error)
    }
}

struct CompositionTree {
    device: IDCompositionDevice,
    target: IDCompositionTarget,
    _root: IDCompositionVisual,
    native_root: IDCompositionVisual,
    ui: IDCompositionVisual,
    hwnd: HWND,
    _window: Arc<dyn winit::window::Window>,
}
impl Drop for CompositionTree {
    fn drop(&mut self) {
        // The retained window remains alive until the target is detached.
        unsafe {
            let _ = self.target.SetRoot(None::<&IDCompositionVisual>);
            let _ = self.device.Commit();
        }
    }
}

/// UI-thread backend access; ordinary Runtime controls never receive this handle.
/// Native visuals are children of a layer permanently below the WGPU UI visual.
#[derive(Clone)]
pub struct WindowsComposition {
    tree: Rc<CompositionTree>,
}
impl WindowsComposition {
    pub(crate) fn new(
        window: Arc<dyn winit::window::Window>,
    ) -> Result<Self, WindowsCompositionError> {
        let handle = window
            .window_handle()
            .map_err(|error| WindowsCompositionError::WindowHandle(error.to_string()))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(WindowsCompositionError::WindowHandle(
                "non-Win32 handle".to_owned(),
            ));
        };
        let hwnd = HWND(handle.hwnd.get() as *mut c_void);
        // No D3D device is created here: WGPU supplies the UI swapchain.
        let (device, target, root, native_root, ui) = unsafe {
            let device: IDCompositionDevice = DCompositionCreateDevice(None::<&IDXGIDevice>)?;
            let target = device.CreateTargetForHwnd(hwnd, true)?;
            let root = device.CreateVisual()?;
            let native_root = device.CreateVisual()?;
            let ui = device.CreateVisual()?;
            root.AddVisual(&native_root, false, None::<&IDCompositionVisual>)?;
            root.AddVisual(&ui, true, Some(&native_root))?;
            target.SetRoot(&root)?;
            device.Commit()?;
            (device, target, root, native_root, ui)
        };
        Ok(Self {
            tree: Rc::new(CompositionTree {
                device,
                target,
                _root: root,
                native_root,
                ui,
                hwnd,
                _window: window,
            }),
        })
    }
    /// Borrowed HWND, valid while this composition handle remains alive.
    pub fn window_handle(&self) -> *mut c_void {
        self.tree.hwnd.0
    }
    pub(crate) fn ui_visual(&self) -> *mut c_void {
        self.tree.ui.as_raw()
    }
    /// Starts hidden. Geometry and visibility are committed explicitly with `commit`.
    pub fn create_native_visual(&self) -> Result<WindowsNativeVisual, WindowsCompositionError> {
        let (container, content) = unsafe {
            let container = self.tree.device.CreateVisual()?;
            let content = self.tree.device.CreateVisual()?;
            container.SetClip2(&D2D_RECT_F::default())?;
            container.AddVisual(&content, false, None::<&IDCompositionVisual>)?;
            self.tree
                .native_root
                .AddVisual(&container, true, None::<&IDCompositionVisual>)?;
            (container, content)
        };
        Ok(WindowsNativeVisual {
            inner: Rc::new(NativeVisual {
                tree: Rc::clone(&self.tree),
                container,
                content,
                state: RefCell::new(VisualState::default()),
            }),
        })
    }
    /// Publishes batched tree, geometry and native-engine content changes.
    pub fn commit(&self) -> Result<(), WindowsCompositionError> {
        unsafe {
            self.tree.device.Commit()?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct VisualState {
    bounds: WindowsCompositionRect,
    clip: Option<WindowsCompositionRect>,
    visible: bool,
    removed: bool,
}
struct NativeVisual {
    tree: Rc<CompositionTree>,
    container: IDCompositionVisual,
    content: IDCompositionVisual,
    state: RefCell<VisualState>,
}
impl Drop for NativeVisual {
    fn drop(&mut self) {
        if !self.state.get_mut().removed {
            unsafe {
                let _ = self.tree.native_root.RemoveVisual(&self.container);
                let _ = self.tree.device.Commit();
            }
        }
    }
}

/// Clones share one native visual; dropping the last handle removes its subtree.
/// The engine attaches to the content visual; host geometry lives on its parent.
#[derive(Clone)]
pub struct WindowsNativeVisual {
    inner: Rc<NativeVisual>,
}
impl WindowsNativeVisual {
    /// Borrowed IDCompositionVisual pointer. Do not take ownership of this COM
    /// reference; a native API retaining it must perform its own AddRef.
    pub fn as_raw(&self) -> *mut c_void {
        self.inner.content.as_raw()
    }
    pub fn window_handle(&self) -> *mut c_void {
        self.inner.tree.hwnd.0
    }
    /// Bounds and clip use physical client-area coordinates. The clip is
    /// intersected with bounds and translated into visual-local space.
    pub fn set_geometry(
        &self,
        bounds: WindowsCompositionRect,
        clip: Option<WindowsCompositionRect>,
    ) -> Result<(), WindowsCompositionError> {
        let mut state = self.inner.state.borrow_mut();
        if state.removed {
            return Err(WindowsCompositionError::Removed);
        }
        let rect = local_clip(bounds, clip, state.visible)?;
        if state.bounds == bounds && state.clip == clip {
            return Ok(());
        }
        unsafe {
            self.inner.container.SetOffsetX2(bounds.x)?;
            self.inner.container.SetOffsetY2(bounds.y)?;
            self.inner.container.SetClip2(&rect)?;
        }
        state.bounds = bounds;
        state.clip = clip;
        Ok(())
    }
    pub fn set_visible(&self, visible: bool) -> Result<(), WindowsCompositionError> {
        let mut state = self.inner.state.borrow_mut();
        if state.removed {
            return Err(WindowsCompositionError::Removed);
        }
        if state.visible == visible {
            return Ok(());
        }
        let rect = local_clip(state.bounds, state.clip, visible)?;
        unsafe {
            self.inner.container.SetClip2(&rect)?;
        }
        state.visible = visible;
        Ok(())
    }
    /// Detaches from the pending tree. Idempotent; call `commit` to publish.
    pub fn remove(&self) -> Result<(), WindowsCompositionError> {
        let mut state = self.inner.state.borrow_mut();
        if !state.removed {
            unsafe {
                self.inner
                    .tree
                    .native_root
                    .RemoveVisual(&self.inner.container)?;
            }
            state.removed = true;
        }
        Ok(())
    }
}

fn local_clip(
    bounds: WindowsCompositionRect,
    clip: Option<WindowsCompositionRect>,
    visible: bool,
) -> Result<D2D_RECT_F, WindowsCompositionError> {
    if !bounds.valid() || clip.is_some_and(|clip| !clip.valid()) {
        return Err(WindowsCompositionError::InvalidGeometry);
    }
    if !visible {
        return Ok(D2D_RECT_F::default());
    }
    let clip = clip.unwrap_or(bounds);
    let left = (clip.x - bounds.x).clamp(0.0, bounds.width);
    let top = (clip.y - bounds.y).clamp(0.0, bounds.height);
    let right = ((clip.x + clip.width) - bounds.x).clamp(left, bounds.width);
    let bottom = ((clip.y + clip.height) - bounds.y).clamp(top, bounds.height);
    Ok(D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_geometry_clips_in_client_pixels_and_never_escapes_bounds() {
        let bounds = WindowsCompositionRect {
            x: 100.0,
            y: 50.0,
            width: 80.0,
            height: 60.0,
        };
        let clip = WindowsCompositionRect {
            x: 90.0,
            y: 70.0,
            width: 50.0,
            height: 80.0,
        };
        let local = local_clip(bounds, Some(clip), true).unwrap();
        assert_eq!(
            (local.left, local.top, local.right, local.bottom),
            (0.0, 20.0, 40.0, 60.0)
        );
        let hidden = local_clip(bounds, Some(clip), false).unwrap();
        assert_eq!(
            (hidden.left, hidden.top, hidden.right, hidden.bottom),
            (0.0, 0.0, 0.0, 0.0)
        );
        let disjoint = local_clip(
            bounds,
            Some(WindowsCompositionRect { x: 200.0, ..clip }),
            true,
        )
        .unwrap();
        assert_eq!(disjoint.left, disjoint.right);
        assert!(
            local_clip(
                WindowsCompositionRect {
                    width: -1.0,
                    ..bounds
                },
                None,
                true
            )
            .is_err()
        );
        assert!(
            local_clip(
                WindowsCompositionRect {
                    x: f32::NAN,
                    ..bounds
                },
                None,
                false
            )
            .is_err()
        );
    }
}
