//! Windows Early Splash: a small topmost DirectComposition popup on its own
//! plain HWND.
//!
//! The popup is independent from the primary GPU host, so only its own client
//! rectangle participates in hit testing. DirectComposition needs
//! a rendering device for surfaces; the splash makes a short-lived D3D11 device
//! of its own for the two tiny uploads and releases it with the tree. It does
//! not touch wgpu's adapter or backend choice.
//!
//! A subclass follows `WM_SIZE` and `WM_DPICHANGED`, so the logo stays centred
//! and at its logical size without any callback into the host.

use std::cell::Cell;
use std::ffi::c_void;
use std::rc::{Rc, Weak};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectComposition::{
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR,
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, DCOMPOSITION_BORDER_MODE_HARD,
    DCompositionCreateDevice, IDCompositionAnimation, IDCompositionDevice,
    IDCompositionEffectGroup, IDCompositionRotateTransform, IDCompositionScaleTransform,
    IDCompositionSurface, IDCompositionTarget, IDCompositionTransform, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;
use windows_sys::Win32::Foundation::{POINT as SysPoint, RECT};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, IsIconic, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOSENDCHANGING, SWP_NOZORDER, SetWindowPos, ShowWindow, WS_EX_NOACTIVATE,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{LogoInfo, SplashAnimation, SplashFailure, SplashWork, fit_logo};
use crate::dcomp::{d3d_device, upload};
use crate::material::FallbackColor;
use crate::win32_hooks::{SplashHookTarget, Win32WindowHooks};

/// The D3D11 device a splash uploads its surfaces with. Making one loads
/// the GPU's user-mode driver, most of a splash's cost, so it is made on a
/// thread of its own ([`super::SplashRenderer::prepare`]).
#[derive(Clone)]
pub(super) struct RenderDevice(ID3D11Device);

// SAFETY: an ID3D11Device is free-threaded. Its immediate context is used
// only by the splash, on the one thread that shows it.
unsafe impl Send for RenderDevice {}
unsafe impl Sync for RenderDevice {}

impl RenderDevice {
    pub(super) fn create() -> Result<Self, SplashFailure> {
        d3d_device()
            .map(Self)
            .map_err(|error| SplashFailure::Native(error.to_string()))
    }
}

pub(super) struct Request<'a> {
    pub(super) png: &'a [u8],
    pub(super) info: LogoInfo,
    pub(super) logo_size: (f64, f64),
    pub(super) background: Option<FallbackColor>,
    pub(super) animation: SplashAnimation,
}

/// Everything the subclass reads. Heap-allocated so the subclass can hold a
/// stable pointer; freed only after the subclass is removed.
struct Tree {
    hwnd: HWND,
    owner: Option<HWND>,
    device: IDCompositionDevice,
    target: IDCompositionTarget,
    background: Option<(IDCompositionVisual, IDCompositionScaleTransform)>,
    logo: IDCompositionVisual,
    logo_scale: IDCompositionScaleTransform,
    logo_rotate: IDCompositionRotateTransform,
    image: (u32, u32),
    logo_box: (f64, f64),
    commits: Cell<usize>,
    // Released last: the surfaces above were created on it.
    _effects: Option<IDCompositionEffectGroup>,
    _surfaces: Vec<IDCompositionSurface>,
    _d3d: ID3D11Device,
}

impl Tree {
    /// Places the background and logo for the window's current client size
    /// and DPI, then publishes it.
    fn layout(&self) -> windows::core::Result<()> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: `hwnd` is the live window this tree targets.
        unsafe { GetClientRect(self.hwnd.0, &mut rect) };
        let client = (
            f64::from((rect.right - rect.left).max(0)),
            f64::from((rect.bottom - rect.top).max(0)),
        );
        // SAFETY: as above.
        let dpi = unsafe { GetDpiForWindow(self.hwnd.0) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
        let logo_box = (self.logo_box.0 * scale, self.logo_box.1 * scale);
        let (x, y, width, height) = fit_logo(client, logo_box, self.image);
        unsafe {
            if let Some((_, stretch)) = &self.background {
                stretch.SetScaleX2(client.0 as f32)?;
                stretch.SetScaleY2(client.1 as f32)?;
            }
            let sx = (width / f64::from(self.image.0.max(1))) as f32;
            let sy = (height / f64::from(self.image.1.max(1))) as f32;
            self.logo_scale.SetScaleX2(sx)?;
            self.logo_scale.SetScaleY2(sy)?;
            self.logo_rotate.SetCenterX2((width / 2.0) as f32)?;
            self.logo_rotate.SetCenterY2((height / 2.0) as f32)?;
            self.logo.SetOffsetX2(x as f32)?;
            self.logo.SetOffsetY2(y as f32)?;
            self.device.Commit()?;
        }
        self.commits.set(self.commits.get() + 1);
        Ok(())
    }
}

impl SplashHookTarget for Tree {
    fn sync_from_owner(&self) {
        sync_overlay_geometry(self);
    }

    fn layout_splash(&self) {
        let _ = self.layout();
    }
}

pub(super) struct Splash {
    tree: Rc<Tree>,
    splash_hooks: Weak<Win32WindowHooks>,
    owner_hooks: Weak<Win32WindowHooks>,
}

impl Splash {
    pub(super) fn show<W: HasWindowHandle + ?Sized>(
        window: &W,
        request: &Request<'_>,
        device: Option<&RenderDevice>,
        work: &mut SplashWork,
    ) -> Result<(Self, bool), SplashFailure> {
        let native = |error: windows::core::Error| SplashFailure::Native(error.to_string());
        let handle = window
            .window_handle()
            .map_err(|error| SplashFailure::Native(error.to_string()))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(SplashFailure::Native("not a Win32 window".into()));
        };
        let owner = HWND(handle.hwnd.get() as *mut c_void);
        work.logo_decodes += 1;
        let pixels = super::decode_premultiplied_bgra(request.png, request.info)
            .map_err(SplashFailure::Logo)?;

        let d3d = match device {
            Some(device) => device.0.clone(),
            None => d3d_device().map_err(native)?,
        };
        // Keep the startup surface independent from the primary window even
        // on the plain HWND path. The primary window is a GPU host and may be
        // much larger than the logo; it must not become the splash hit area.
        let hwnd = create_overlay_window(owner, request.logo_size)?;
        let (tree, animated) = build_tree(hwnd, Some(owner), &d3d, request, &pixels, work)
            .map_err(|error| {
                unsafe { DestroyWindow(hwnd.0) };
                native(error)
            })?;
        let tree = Rc::new(tree);
        if let Err(error) = tree.layout() {
            drop(tree);
            unsafe { DestroyWindow(hwnd.0) };
            return Err(native(error));
        }
        let splash_hooks = Win32WindowHooks::install_or_get(hwnd.0).map_err(|error| {
            unsafe { DestroyWindow(hwnd.0) };
            SplashFailure::Native(error)
        })?;
        let owner_hooks = Win32WindowHooks::install_or_get(owner.0).map_err(|error| {
            unsafe { DestroyWindow(hwnd.0) };
            SplashFailure::Native(error)
        })?;
        let weak = Rc::downgrade(&tree) as Weak<dyn SplashHookTarget>;
        *splash_hooks.splash_self.borrow_mut() = Some(weak.clone());
        *owner_hooks.splash_owner.borrow_mut() = Some(weak);
        unsafe { ShowWindow(hwnd.0, SW_SHOWNOACTIVATE) };
        work.commits += tree.commits.replace(0);
        Ok((
            Self {
                tree,
                splash_hooks: Rc::downgrade(&splash_hooks),
                owner_hooks: Rc::downgrade(&owner_hooks),
            },
            animated,
        ))
    }

    pub(super) fn live_resources(&self) -> usize {
        // D3D device, composition device, target, logo visual, the logo's two
        // transforms and its surface, plus the background's visual, transform
        // and surface, the effect group and the hook.
        7 + usize::from(self.tree.background.is_some()) * 3
            + usize::from(self.tree._effects.is_some())
            + 1
    }

    /// Takes the tree off the window and releases it. As a handoff it first
    /// waits one DWM composition pass, so a frame the caller presented and saw
    /// complete has been latched and the window is never left uncovered.
    pub(super) fn remove(self, work: &mut SplashWork, handoff: bool) {
        let hwnd = self.tree.hwnd;
        let owner = self.tree.owner;
        let target = Rc::downgrade(&self.tree) as Weak<dyn SplashHookTarget>;
        if let Some(hooks) = self.splash_hooks.upgrade() {
            let mut slot = hooks.splash_self.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|weak| Weak::ptr_eq(weak, &target))
            {
                *slot = None;
            }
        }
        if let Some(hooks) = self.owner_hooks.upgrade() {
            let mut slot = hooks.splash_owner.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|weak| Weak::ptr_eq(weak, &target))
            {
                *slot = None;
            }
        }
        if handoff {
            // SAFETY: blocks until the next composition pass; no arguments.
            unsafe { DwmFlush() };
        }
        let cleared = unsafe { self.tree.target.SetRoot(None::<&IDCompositionVisual>) }
            .and_then(|()| unsafe { self.tree.device.Commit() });
        if cleared.is_ok() {
            work.commits += self.tree.commits.get() + 1;
        }
        if owner.is_some() {
            unsafe { DestroyWindow(hwnd.0) };
        }
    }
}

fn build_tree(
    hwnd: HWND,
    owner: Option<HWND>,
    d3d: &ID3D11Device,
    request: &Request<'_>,
    pixels: &[u8],
    work: &mut SplashWork,
) -> windows::core::Result<(Tree, bool)> {
    // SAFETY: COM calls on objects created here, on the window's thread.
    unsafe {
        let dxgi: IDXGIDevice = d3d.cast()?;
        let device: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let context = d3d.GetImmediateContext()?;
        let target = device.CreateTargetForHwnd(hwnd, true)?;
        let root = device.CreateVisual()?;
        let mut surfaces = Vec::new();

        let background = match request.background {
            Some(color) => {
                let premultiply =
                    |c: u8| ((u16::from(c) * u16::from(color.alpha) + 127) / 255) as u8;
                let pixel = [
                    premultiply(color.blue),
                    premultiply(color.green),
                    premultiply(color.red),
                    color.alpha,
                ];
                let surface = upload(&device, &context, 1, 1, &pixel)?;
                let visual = device.CreateVisual()?;
                visual.SetContent(&surface)?;
                visual.SetBitmapInterpolationMode(
                    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                )?;
                visual.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
                let stretch = device.CreateScaleTransform()?;
                visual.SetTransform(&stretch)?;
                root.AddVisual(&visual, true, None::<&IDCompositionVisual>)?;
                surfaces.push(surface);
                Some((visual, stretch))
            }
            None => None,
        };

        let (width, height) = (request.info.width, request.info.height);
        let logo_surface = upload(&device, &context, width, height, pixels)?;
        work.logo_uploads += 1;
        let logo = device.CreateVisual()?;
        logo.SetContent(&logo_surface)?;
        logo.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
        let logo_scale = device.CreateScaleTransform()?;
        let logo_rotate = device.CreateRotateTransform()?;
        let group = device.CreateTransformGroup(&[
            Some(logo_scale.cast::<IDCompositionTransform>()?),
            Some(logo_rotate.cast::<IDCompositionTransform>()?),
        ])?;
        logo.SetTransform(&group)?;
        root.AddVisual(&logo, true, None::<&IDCompositionVisual>)?;
        surfaces.push(logo_surface);

        // An animation the compositor refuses leaves the logo still; the
        // splash itself is not given up for it.
        let (effects, animated) = match animate(&device, &logo, &logo_rotate, request.animation) {
            Ok(effects) => (effects, request.animation != SplashAnimation::None),
            Err(_) => (None, false),
        };
        if animated {
            work.animation_submissions += 1;
        }
        target.SetRoot(&root)?;

        Ok((
            Tree {
                hwnd,
                owner,
                device,
                target,
                background,
                logo,
                logo_scale,
                logo_rotate,
                image: (width, height),
                logo_box: request.logo_size,
                commits: Cell::new(0),
                _effects: effects,
                _surfaces: surfaces,
                _d3d: d3d.clone(),
            },
            animated,
        ))
    }
}

/// Hands the preset to DWM as one animation function. Nothing in this process
/// advances it.
fn animate(
    device: &IDCompositionDevice,
    logo: &IDCompositionVisual,
    rotate: &IDCompositionRotateTransform,
    preset: SplashAnimation,
) -> windows::core::Result<Option<IDCompositionEffectGroup>> {
    // SAFETY: COM calls on live objects.
    unsafe {
        let opacity = |build: &dyn Fn(&IDCompositionAnimation) -> windows::core::Result<()>| {
            let animation = device.CreateAnimation()?;
            build(&animation)?;
            let effects = device.CreateEffectGroup()?;
            effects.SetOpacity(&animation)?;
            logo.SetEffect(&effects)?;
            Ok::<_, windows::core::Error>(Some(effects))
        };
        match preset {
            SplashAnimation::None => Ok(None),
            // 0 → 1 over 0.35 s, then held.
            SplashAnimation::FadeIn => opacity(&|animation| {
                animation.AddCubic(0.0, 0.0, 1.0 / 0.35, 0.0, 0.0)?;
                animation.End(0.35, 1.0)
            }),
            // 0.45 ↔ 1.0, one breath every 1.8 s, until removed.
            SplashAnimation::Pulse => {
                opacity(&|animation| animation.AddSinusoidal(0.0, 0.725, 0.275, 1.0 / 1.8, 90.0))
            }
            // One counter-clockwise turn every 1.2 s, repeated until removed.
            SplashAnimation::Rotate => {
                let animation = device.CreateAnimation()?;
                animation.AddCubic(0.0, 0.0, -360.0 / 1.2, 0.0, 0.0)?;
                animation.AddRepeat(1.2, 1.2)?;
                rotate.SetAngle(&animation)?;
                Ok(None)
            }
        }
    }
}

fn sync_overlay_geometry(tree: &Tree) {
    let Some(owner) = tree.owner else { return };
    // A minimized owner has no client area to centre on (its origin is
    // parked near -32000); the splash goes away with it and comes back on
    // restore.
    if unsafe { IsIconic(owner.0) } != 0 {
        unsafe { ShowWindow(tree.hwnd.0, SW_HIDE) };
        return;
    }
    let (position, width, height) = splash_geometry(owner, tree.logo_box);
    // Position only: the splash keeps the z-order it was shown with rather
    // than floating above every other application.
    unsafe {
        SetWindowPos(
            tree.hwnd.0,
            std::ptr::null_mut(),
            position.x,
            position.y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOZORDER,
        );
        ShowWindow(tree.hwnd.0, SW_SHOWNOACTIVATE);
    }
}

fn owner_client_geometry(owner: HWND) -> (SysPoint, i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    };
    unsafe { GetClientRect(owner.0, &mut rect) };
    let mut position = SysPoint { x: 0, y: 0 };
    unsafe { ClientToScreen(owner.0, &mut position) };
    (
        position,
        (rect.right - rect.left).max(1),
        (rect.bottom - rect.top).max(1),
    )
}

fn splash_geometry(owner: HWND, logical_size: (f64, f64)) -> (SysPoint, i32, i32) {
    let (owner_position, owner_width, owner_height) = owner_client_geometry(owner);
    let dpi = unsafe { GetDpiForWindow(owner.0) }.max(1);
    let scale = f64::from(dpi) / 96.0;
    let (width, height) = splash_physical_size(logical_size, scale);
    let center_x = owner_position.x + owner_width / 2;
    let center_y = owner_position.y + owner_height / 2;
    let mut x = center_x - width / 2;
    let mut y = center_y - height / 2;
    let monitor = unsafe { MonitorFromWindow(owner.0, MONITOR_DEFAULTTONEAREST) };
    if !monitor.is_null() {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            x = x.clamp(info.rcWork.left, info.rcWork.right.saturating_sub(width));
            y = y.clamp(info.rcWork.top, info.rcWork.bottom.saturating_sub(height));
        }
    }
    (SysPoint { x, y }, width, height)
}

fn splash_physical_size(logical_size: (f64, f64), scale: f64) -> (i32, i32) {
    (
        (logical_size.0.max(1.0) * scale).round().max(1.0) as i32,
        (logical_size.1.max(1.0) * scale).round().max(1.0) as i32,
    )
}

fn create_overlay_window(owner: HWND, logical_size: (f64, f64)) -> Result<HWND, SplashFailure> {
    let (position, width, height) = splash_geometry(owner, logical_size);
    let class: [u16; 7] = [83, 84, 65, 84, 73, 67, 0];
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err(SplashFailure::Native("GetModuleHandleW failed".into()));
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            position.x,
            position.y,
            width,
            height,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            module,
            std::ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        Err(SplashFailure::Native("CreateWindowExW failed".into()))
    } else {
        Ok(HWND(hwnd))
    }
}

#[cfg(test)]
mod tests {
    use super::splash_physical_size;

    #[test]
    fn logo_size_is_the_native_client_size_at_each_dpi() {
        assert_eq!(splash_physical_size((128.0, 96.0), 1.0), (128, 96));
        assert_eq!(splash_physical_size((128.0, 96.0), 1.25), (160, 120));
        assert_eq!(splash_physical_size((128.0, 96.0), 2.0), (256, 192));
    }

    #[test]
    fn size_is_independent_of_primary_window_geometry() {
        assert_eq!(splash_physical_size((240.0, 180.0), 1.5), (360, 270));
    }
}
