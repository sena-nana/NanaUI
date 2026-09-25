//! Windows Early Splash: a topmost DirectComposition tree on a plain HWND.
//!
//! The visuals sit above whatever the window presents — the redirection
//! surface before the first frame and wgpu's flip-model swap chain after it —
//! so the splash covers the window until it is removed. DirectComposition needs
//! a rendering device for surfaces; the splash makes a short-lived D3D11 device
//! of its own for the two tiny uploads and releases it with the tree. It does
//! not touch wgpu's adapter or backend choice.
//!
//! A subclass follows `WM_SIZE` and `WM_DPICHANGED`, so the logo stays centred
//! and at its logical size without any callback into the host.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectComposition::{
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR,
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, DCOMPOSITION_BORDER_MODE_HARD,
    DCompositionCreateDevice, IDCompositionAnimation, IDCompositionDevice,
    IDCompositionEffectGroup, IDCompositionRotateTransform, IDCompositionScaleTransform,
    IDCompositionSurface, IDCompositionTarget, IDCompositionTransform, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT as SysPoint, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, HWND_TOPMOST, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOSENDCHANGING, SetWindowPos, ShowWindow, WM_DPICHANGED, WM_MOVE, WM_SIZE,
    WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::{LogoInfo, SplashAnimation, SplashFailure, SplashWork, fit_logo};
use crate::material::FallbackColor;

const SUBCLASS_ID: usize = 0x4E_41_53_50;
const OWNER_SUBCLASS_ID: usize = SUBCLASS_ID + 1;

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
    commits: usize,
    // Released last: the surfaces above were created on it.
    _effects: Option<IDCompositionEffectGroup>,
    _surfaces: Vec<IDCompositionSurface>,
    _d3d: ID3D11Device,
}

impl Tree {
    /// Places the background and logo for the window's current client size
    /// and DPI, then publishes it.
    fn layout(&mut self) -> windows::core::Result<()> {
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
        self.commits += 1;
        Ok(())
    }
}

pub(super) struct Splash {
    tree: *mut Tree,
}

impl Splash {
    pub(super) fn show<W: HasWindowHandle + ?Sized>(
        window: &W,
        request: &Request<'_>,
        work: &mut SplashWork,
        separate_window: bool,
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

        let d3d = d3d_device().map_err(native)?;
        let hwnd = if separate_window {
            create_overlay_window(owner)?
        } else {
            owner
        };
        let (tree, animated) = build_tree(
            hwnd,
            separate_window.then_some(owner),
            &d3d,
            request,
            &pixels,
            work,
        )
        .map_err(|error| {
            if separate_window {
                unsafe { DestroyWindow(hwnd.0) };
            }
            native(error)
        })?;
        let tree = Box::into_raw(Box::new(tree));
        // SAFETY: `tree` is live and owned by the returned `Splash` until the
        // subclass has been removed.
        if let Err(error) = unsafe { (*tree).layout() } {
            unsafe { drop(Box::from_raw(tree)) };
            if separate_window {
                unsafe { DestroyWindow(hwnd.0) };
            }
            return Err(native(error));
        }
        // SAFETY: hwnd is the live winit window; `tree` outlives the subclass.
        let installed =
            unsafe { SetWindowSubclass(hwnd.0, Some(splash_proc), SUBCLASS_ID, tree as usize) };
        if installed == 0 {
            unsafe { drop(Box::from_raw(tree)) };
            if separate_window {
                unsafe { DestroyWindow(hwnd.0) };
            }
            return Err(SplashFailure::Native("SetWindowSubclass failed".into()));
        }
        if separate_window {
            let owner_installed = unsafe {
                SetWindowSubclass(owner.0, Some(splash_proc), OWNER_SUBCLASS_ID, tree as usize)
            };
            if owner_installed == 0 {
                unsafe { RemoveWindowSubclass(hwnd.0, Some(splash_proc), SUBCLASS_ID) };
                unsafe { drop(Box::from_raw(tree)) };
                unsafe { DestroyWindow(hwnd.0) };
                return Err(SplashFailure::Native("failed to track owner window".into()));
            }
            unsafe { ShowWindow(hwnd.0, SW_SHOWNOACTIVATE) };
        }
        work.commits += unsafe { (*tree).commits };
        unsafe { (*tree).commits = 0 };
        Ok((Self { tree }, animated))
    }

    pub(super) fn live_resources(&self) -> usize {
        // SAFETY: the tree lives as long as `self`.
        let tree = unsafe { &*self.tree };
        // D3D device, composition device, target, logo visual, the logo's two
        // transforms and its surface, plus the background's visual, transform
        // and surface, the effect group and the subclass.
        7 + usize::from(tree.background.is_some()) * 3 + usize::from(tree._effects.is_some()) + 1
    }

    /// Takes the tree off the window and releases it. As a handoff it first
    /// waits one DWM composition pass, so a frame the caller presented and saw
    /// complete has been latched and the window is never left uncovered.
    pub(super) fn remove(self, work: &mut SplashWork, handoff: bool) {
        // SAFETY: the tree lives until the end of this function.
        let hwnd = unsafe { (*self.tree).hwnd };
        let owner = unsafe { (*self.tree).owner };
        // SAFETY: the subclass was installed with this id and pointer.
        let removed = unsafe { RemoveWindowSubclass(hwnd.0, Some(splash_proc), SUBCLASS_ID) };
        if let Some(owner) = owner {
            unsafe { RemoveWindowSubclass(owner.0, Some(splash_proc), OWNER_SUBCLASS_ID) };
        }
        if handoff {
            // SAFETY: blocks until the next composition pass; no arguments.
            unsafe { DwmFlush() };
        }
        // SAFETY: after removal nothing else can reach the tree.
        let tree = unsafe { &mut *self.tree };
        let cleared = unsafe { tree.target.SetRoot(None::<&IDCompositionVisual>) }
            .and_then(|()| unsafe { tree.device.Commit() });
        if cleared.is_ok() {
            work.commits += tree.commits + 1;
        }
        if removed != 0 {
            unsafe { drop(Box::from_raw(self.tree)) };
        }
        if owner.is_some() {
            unsafe { DestroyWindow(hwnd.0) };
        }
        // A subclass that could not be removed keeps its pointer valid; the
        // tree is then leaked rather than freed under it.
    }
}

fn d3d_device() -> windows::core::Result<ID3D11Device> {
    let create = |driver: D3D_DRIVER_TYPE| {
        let mut device = None;
        // SAFETY: out-pointer to a local; no adapter, default feature levels.
        unsafe {
            D3D11CreateDevice(
                None::<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .map(|()| device)
    };
    match create(D3D_DRIVER_TYPE_HARDWARE) {
        Ok(Some(device)) => Ok(device),
        _ => create(D3D_DRIVER_TYPE_WARP)?
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL)),
    }
}

fn upload(
    device: &IDCompositionDevice,
    context: &ID3D11DeviceContext,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> windows::core::Result<IDCompositionSurface> {
    // SAFETY: plain COM calls on live objects; `bgra` holds width*height rows.
    unsafe {
        let surface = device.CreateSurface(
            width,
            height,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_ALPHA_MODE_PREMULTIPLIED,
        )?;
        let mut offset = POINT::default();
        let texture: ID3D11Texture2D = surface.BeginDraw(None, &mut offset)?;
        let region = D3D11_BOX {
            left: offset.x as u32,
            top: offset.y as u32,
            front: 0,
            right: offset.x as u32 + width,
            bottom: offset.y as u32 + height,
            back: 1,
        };
        context.UpdateSubresource(
            &texture,
            0,
            Some(&raw const region),
            bgra.as_ptr().cast(),
            width * 4,
            0,
        );
        surface.EndDraw()?;
        Ok(surface)
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
                commits: 0,
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

unsafe extern "system" fn splash_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> LRESULT {
    // SAFETY: forwards to winit's procedure first, so the window has its new
    // size before the splash follows it.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    let tree = unsafe { &*(ref_data as *mut Tree) };
    let is_splash = tree.hwnd.0 == hwnd;
    let is_owner = tree.owner.is_some_and(|owner| owner.0 == hwnd);
    if is_owner && matches!(message, WM_MOVE | WM_SIZE | WM_DPICHANGED) {
        // SetWindowPos may synchronously notify the splash window. Do not
        // hold a mutable Tree borrow across that call.
        sync_overlay_geometry(tree);
    } else if is_splash && matches!(message, WM_SIZE | WM_DPICHANGED) {
        // SAFETY: ref_data is the live Tree installed with this subclass.
        let tree = unsafe { &mut *(ref_data as *mut Tree) };
        let _ = tree.layout();
    }
    result
}

fn sync_overlay_geometry(tree: &Tree) {
    let Some(owner) = tree.owner else { return };
    let (position, width, height) = owner_client_geometry(owner);
    unsafe {
        SetWindowPos(
            tree.hwnd.0,
            HWND_TOPMOST,
            position.x,
            position.y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOSENDCHANGING,
        );
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

fn create_overlay_window(owner: HWND) -> Result<HWND, SplashFailure> {
    let (position, width, height) = owner_client_geometry(owner);
    let class: [u16; 7] = [83, 84, 65, 84, 73, 67, 0];
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        return Err(SplashFailure::Native("GetModuleHandleW failed".into()));
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT,
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
