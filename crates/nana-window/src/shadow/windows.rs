//! Windows: the DWM frame shadow, or a DirectComposition companion.
//!
//! DWM draws a shadow only for a window with a non-client frame. A
//! transparent NanaUI window has none, and the card it paints is somewhere
//! inside its client area, so its shadow is a companion: an unowned,
//! click-through, never-activated popup kept directly below the primary in
//! z-order, whose DirectComposition tree is a nine-slice of one small shadow
//! tile. The tile — the corners of a blurred rounded rectangle — is
//! rasterized on the CPU only when the style or the scale changes; a resize
//! only rescales the edge visuals, a move only moves the popup.
//!
//! The companion follows the primary from a subclass on the primary's window
//! procedure (`WM_WINDOWPOSCHANGED`), so it moves in the same message as the
//! primary without a round trip through the host. It is hidden while the
//! primary is minimized, maximized or hidden, and destroyed with it.

use std::cell::Cell;
use std::sync::OnceLock;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND as ComHwnd;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};
use windows::Win32::Graphics::DirectComposition::{
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, DCompositionCreateDevice,
    IDCompositionDevice, IDCompositionScaleTransform, IDCompositionSurface, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_NCRENDERING_ENABLED, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, GetWindowLongPtrW, HTTRANSPARENT,
    IsIconic, IsWindowVisible, IsZoomed, LWA_ALPHA, MA_NOACTIVATE, RegisterClassW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_NCHITTEST, WM_SIZE, WM_WINDOWPOSCHANGED,
    WNDCLASSW, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP, WS_THICKFRAME,
};

use super::{ShadowApplied, ShadowShape, ShadowStyle, ShadowWork};
use crate::dcomp::{d3d_device, upload};

const OWNER_SUBCLASS_ID: usize = 0x4E_41_53_48;

fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as HWND)
}

/// Whether DWM draws this window's frame, and therefore its shadow.
fn has_dwm_frame(hwnd: HWND) -> bool {
    // SAFETY: `hwnd` is a live window; the attribute is a BOOL out-parameter.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    if style & (WS_THICKFRAME | WS_CAPTION) == 0 {
        return false;
    }
    let mut enabled: i32 = 0;
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_ENABLED as u32,
            std::ptr::from_mut(&mut enabled).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    read >= 0 && enabled != 0
}

pub(super) fn set_native<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> ShadowApplied {
    let Some(hwnd) = hwnd(window) else {
        return ShadowApplied::Failed;
    };
    match (enabled, has_dwm_frame(hwnd)) {
        (true, true) => ShadowApplied::Native,
        (true, false) => ShadowApplied::Unsupported,
        (false, true) => ShadowApplied::NativeNotRemovable,
        (false, false) => ShadowApplied::Disabled,
    }
}

/// Geometry in physical pixels, shared with the owner subclass.
struct Placement {
    companion: HWND,
    owner: HWND,
    /// The body in the owner's client area.
    body: Cell<RECT>,
    margin: Cell<i32>,
    /// The host's own visibility for the shadow (fullscreen hides it).
    wanted: Cell<bool>,
    shown: Cell<bool>,
    moves: Cell<u64>,
}

impl Placement {
    /// Put the popup where the body is now, directly below the owner, or hide
    /// it when the owner's state has no shadow.
    fn sync(&self) {
        // SAFETY: both windows are live while the subclass holds this.
        let hidden = unsafe {
            !self.wanted.get()
                || IsIconic(self.owner) != 0
                || IsZoomed(self.owner) != 0
                || IsWindowVisible(self.owner) == 0
        };
        if hidden {
            if self.shown.replace(false) {
                unsafe { ShowWindow(self.companion, SW_HIDE) };
            }
            return;
        }
        let mut origin = POINT { x: 0, y: 0 };
        unsafe { ClientToScreen(self.owner, &mut origin) };
        let body = self.body.get();
        let margin = self.margin.get();
        // Inserted right after the owner: directly below it in z-order, and
        // in the owner's band, so a topmost owner keeps a topmost companion.
        unsafe {
            SetWindowPos(
                self.companion,
                self.owner,
                origin.x + body.left - margin,
                origin.y + body.top - margin,
                body.right - body.left + 2 * margin,
                body.bottom - body.top + 2 * margin,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        self.moves.set(self.moves.get() + 1);
        if !self.shown.replace(true) {
            unsafe { ShowWindow(self.companion, SW_SHOWNOACTIVATE) };
        }
    }
}

unsafe extern "system" fn owner_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: forward first, so the owner has its new geometry.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    // SAFETY: `data` is the placement installed with this subclass; it is
    // removed before the placement is freed.
    let placement = unsafe { &*(data as *const Placement) };
    match message {
        WM_WINDOWPOSCHANGED | WM_SIZE => placement.sync(),
        WM_NCDESTROY => unsafe {
            RemoveWindowSubclass(hwnd, Some(owner_proc), OWNER_SUBCLASS_ID);
            ShowWindow(placement.companion, SW_HIDE);
        },
        _ => {}
    }
    result
}

unsafe extern "system" fn companion_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        // SAFETY: the default procedure for everything else.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn companion_class() -> Option<*const u16> {
    static CLASS: OnceLock<Option<Vec<u16>>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            let name: Vec<u16> = "NanaWindowShadow\0".encode_utf16().collect();
            let module = unsafe { GetModuleHandleW(std::ptr::null()) };
            let class = WNDCLASSW {
                lpfnWndProc: Some(companion_proc),
                hInstance: module,
                lpszClassName: name.as_ptr(),
                ..unsafe { std::mem::zeroed() }
            };
            (unsafe { RegisterClassW(&class) } != 0).then_some(name)
        })
        .as_ref()
        .map(|name| name.as_ptr())
}

/// Premultiplied BGRA tile: the corners of `style`'s shadow around a
/// rounded rectangle of radius `radius`, `margin + radius` pixels each side of
/// a one-pixel centre row and column. Pixels inside the body stay clear, so a
/// translucent card does not show its own shadow through itself.
fn rasterize(style: ShadowStyle, margin: i32, radius: f64) -> (u32, Vec<u8>) {
    let corner = margin as f64 + radius;
    let size = (2.0 * corner) as u32 + 1;
    let center = corner + 0.5;
    // The body is a (2r+1)-wide rounded square centred in the tile; spread
    // grows the shadow's shape, not the body.
    let half = radius + 0.5;
    let spread = f64::from(style.spread);
    let sigma = (f64::from(style.blur) / 2.0).max(0.5);
    let alpha = f64::from(style.color[3].clamp(0.0, 1.0));
    let rgb =
        [style.color[2], style.color[1], style.color[0]].map(|c| f64::from(c.clamp(0.0, 1.0)));
    let rounded = |x: f64, y: f64, half: f64, radius: f64| {
        let qx = (x - center).abs() - (half - radius);
        let qy = (y - center).abs() - (half - radius);
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
        outside + qx.max(qy).min(0.0) - radius
    };
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            if rounded(px, py, half, radius) < 0.0 {
                continue;
            }
            let distance = rounded(px, py, half + spread, (radius + spread).max(0.0));
            // Coverage of a Gaussian-blurred half-plane at this distance.
            let coverage = 0.5 * erfc(distance / (sigma * std::f64::consts::SQRT_2));
            let a = alpha * coverage;
            let at = ((y * size + x) * 4) as usize;
            for (channel, value) in rgb.iter().enumerate() {
                pixels[at + channel] = (value * a * 255.0).round() as u8;
            }
            pixels[at + 3] = (a * 255.0).round() as u8;
        }
    }
    (size, pixels)
}

/// Complementary error function (Abramowitz–Stegun 7.1.26), enough for a
/// shadow's falloff.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let value = poly * (-x * x).exp();
    if x >= 0.0 { value } else { 2.0 - value }
}

/// One of the eight nine-slice pieces.
struct Piece {
    visual: IDCompositionVisual,
    scale: IDCompositionScaleTransform,
}

struct Tree {
    device: IDCompositionDevice,
    _target: IDCompositionTarget,
    root: IDCompositionVisual,
    pieces: Vec<Piece>,
    surface: Option<IDCompositionSurface>,
    context: ID3D11DeviceContext,
    _d3d: ID3D11Device,
}

pub(super) struct Companion {
    placement: *mut Placement,
    tree: Tree,
    /// What the tile was rasterized for: style, radius and scale.
    tile: Option<(ShadowStyle, f64, f64, u32)>,
    size: (i32, i32),
}

impl Companion {
    pub(super) fn create<W: HasWindowHandle + ?Sized>(
        window: &W,
        _style: ShadowStyle,
        _shape: ShadowShape,
        _work: &mut ShadowWork,
    ) -> Option<Result<Self, ()>> {
        let owner = hwnd(window)?;
        let Some(class) = companion_class() else {
            return Some(Err(()));
        };
        // SAFETY: a plain popup; no owner, so it does not sit above its
        // primary the way an owned window would.
        let companion = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                class,
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if companion.is_null() {
            return Some(Err(()));
        }
        // Layered + transparent: hit tests pass through to whatever is below,
        // in any process. Fully opaque layer alpha; the content's own alpha
        // comes from DirectComposition.
        unsafe { SetLayeredWindowAttributes(companion, 0, 255, LWA_ALPHA) };
        let tree = match build_tree(companion) {
            Ok(tree) => tree,
            Err(_) => {
                unsafe { DestroyWindow(companion) };
                return Some(Err(()));
            }
        };
        let placement = Box::into_raw(Box::new(Placement {
            companion,
            owner,
            body: Cell::new(RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            }),
            margin: Cell::new(0),
            wanted: Cell::new(false),
            shown: Cell::new(false),
            moves: Cell::new(0),
        }));
        // SAFETY: `placement` outlives the subclass; Drop removes it first.
        let installed = unsafe {
            SetWindowSubclass(
                owner,
                Some(owner_proc),
                OWNER_SUBCLASS_ID,
                placement as usize,
            )
        };
        if installed == 0 {
            unsafe {
                drop(Box::from_raw(placement));
                DestroyWindow(companion);
            }
            return Some(Err(()));
        }
        Some(Ok(Self {
            placement,
            tree,
            tile: None,
            size: (0, 0),
        }))
    }

    pub(super) fn update<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        style: ShadowStyle,
        shape: ShadowShape,
        visible: bool,
        work: &mut ShadowWork,
    ) -> bool {
        // SAFETY: the placement lives as long as `self`.
        let placement = unsafe { &*self.placement };
        let dpi = unsafe { GetDpiForWindow(placement.owner) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
        let px = |v: f32| (f64::from(v) * scale).round() as i32;
        let [x, y, width, height] = shape.rect;
        let body = RECT {
            left: px(x),
            top: px(y),
            right: px(x + width),
            bottom: px(y + height),
        };
        let margin = (style.margin() * scale).ceil() as i32;
        let radius = (shape.radius() * scale).round();
        let restyled = self.tile.is_none_or(|(old, old_radius, old_scale, _)| {
            old != style || old_radius != radius || old_scale != scale
        });
        if restyled && self.retile(style, margin, radius, scale, work).is_err() {
            return false;
        }
        let size = (
            body.right - body.left + 2 * margin,
            body.bottom - body.top + 2 * margin,
        );
        if restyled || size != self.size {
            if self.layout(size, style, scale).is_err() {
                return false;
            }
            if size != self.size {
                work.companion_resizes += 1;
            }
            work.effect_updates += 1;
            self.size = size;
        }
        placement.body.set(body);
        placement.margin.set(margin);
        placement.wanted.set(visible);
        placement.sync();
        true
    }

    fn retile(
        &mut self,
        style: ShadowStyle,
        margin: i32,
        radius: f64,
        scale: f64,
        work: &mut ShadowWork,
    ) -> windows::core::Result<()> {
        let (size, pixels) = rasterize(style, margin, radius);
        let surface = upload(&self.tree.device, &self.tree.context, size, size, &pixels)?;
        for piece in &self.tree.pieces {
            unsafe { piece.visual.SetContent(&surface)? };
        }
        self.tree.surface = Some(surface);
        self.tile = Some((style, radius, scale, size));
        work.rasterizations += 1;
        Ok(())
    }

    /// Place the eight pieces for a popup of `size` pixels. The tile's
    /// corners go to the corners; its one-pixel centre row and column are
    /// stretched along the edges. The shadow offset moves the whole tree.
    fn layout(
        &self,
        size: (i32, i32),
        style: ShadowStyle,
        scale: f64,
    ) -> windows::core::Result<()> {
        let Some((_, _, _, tile)) = self.tile else {
            return Ok(());
        };
        let tile = tile as f32;
        let corner = (tile - 1.0) / 2.0;
        let (width, height) = (size.0 as f32, size.1 as f32);
        let stretch_x = (width - 2.0 * corner).max(0.0);
        let stretch_y = (height - 2.0 * corner).max(0.0);
        let (right, bottom) = (width - tile, height - tile);
        // (clip in tile space, offset, stretch along x, stretch along y)
        let pieces = [
            ([0.0, 0.0, corner, corner], (0.0, 0.0), None, None),
            ([tile - corner, 0.0, tile, corner], (right, 0.0), None, None),
            (
                [0.0, tile - corner, corner, tile],
                (0.0, bottom),
                None,
                None,
            ),
            (
                [tile - corner, tile - corner, tile, tile],
                (right, bottom),
                None,
                None,
            ),
            (
                [corner, 0.0, corner + 1.0, corner],
                (0.0, 0.0),
                Some(stretch_x),
                None,
            ),
            (
                [corner, tile - corner, corner + 1.0, tile],
                (0.0, bottom),
                Some(stretch_x),
                None,
            ),
            (
                [0.0, corner, corner, corner + 1.0],
                (0.0, 0.0),
                None,
                Some(stretch_y),
            ),
            (
                [tile - corner, corner, tile, corner + 1.0],
                (right, 0.0),
                None,
                Some(stretch_y),
            ),
        ];
        unsafe {
            for (piece, (clip, offset, sx, sy)) in self.tree.pieces.iter().zip(pieces) {
                piece.visual.SetClip2(&D2D_RECT_F {
                    left: clip[0],
                    top: clip[1],
                    right: clip[2],
                    bottom: clip[3],
                })?;
                piece.visual.SetOffsetX2(offset.0)?;
                piece.visual.SetOffsetY2(offset.1)?;
                piece.scale.SetCenterX2(corner)?;
                piece.scale.SetCenterY2(corner)?;
                piece.scale.SetScaleX2(sx.unwrap_or(1.0))?;
                piece.scale.SetScaleY2(sy.unwrap_or(1.0))?;
            }
            self.tree
                .root
                .SetOffsetX2((f64::from(style.offset[0]) * scale) as f32)?;
            self.tree
                .root
                .SetOffsetY2((f64::from(style.offset[1]) * scale) as f32)?;
            self.tree.device.Commit()
        }
    }

    /// The tile is rasterized per scale; the next update sees the new DPI.
    pub(super) fn rescale<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        _work: &mut ShadowWork,
    ) {
        self.tile = None;
    }

    pub(super) fn add_work(&self, work: &mut ShadowWork) {
        // SAFETY: the placement lives as long as `self`.
        work.companion_moves += unsafe { &*self.placement }.moves.get();
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        // SAFETY: the subclass is removed before the placement it reads.
        unsafe {
            let placement = &*self.placement;
            let removed =
                RemoveWindowSubclass(placement.owner, Some(owner_proc), OWNER_SUBCLASS_ID) != 0;
            DestroyWindow(placement.companion);
            // A subclass that could not be removed still holds the pointer;
            // leak the placement rather than free it under it.
            if removed {
                drop(Box::from_raw(self.placement));
            }
        }
    }
}

fn build_tree(companion: HWND) -> windows::core::Result<Tree> {
    let d3d = d3d_device()?;
    // SAFETY: COM calls on objects created here, on the window's thread.
    unsafe {
        let dxgi: IDXGIDevice = d3d.cast()?;
        let device: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let context = d3d.GetImmediateContext()?;
        let target = device.CreateTargetForHwnd(ComHwnd(companion), true)?;
        let root = device.CreateVisual()?;
        let mut pieces = Vec::with_capacity(8);
        for _ in 0..8 {
            let visual = device.CreateVisual()?;
            visual.SetBitmapInterpolationMode(
                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            )?;
            let scale = device.CreateScaleTransform()?;
            visual.SetTransform(&scale)?;
            root.AddVisual(&visual, true, None::<&IDCompositionVisual>)?;
            pieces.push(Piece { visual, scale });
        }
        target.SetRoot(&root)?;
        Ok(Tree {
            device,
            _target: target,
            root,
            pieces,
            surface: None,
            context,
            _d3d: d3d,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{erfc, rasterize};
    use crate::shadow::ShadowStyle;

    #[test]
    fn the_tile_is_clear_inside_the_body_and_fades_outward() {
        let style = ShadowStyle {
            color: [0.0, 0.0, 0.0, 0.5],
            offset: [0.0, 0.0],
            blur: 8.0,
            spread: 0.0,
        };
        let (size, pixels) = rasterize(style, 10, 4.0);
        assert_eq!(size, 29);
        let alpha = |x: u32, y: u32| pixels[((y * size + x) * 4 + 3) as usize];
        let center = size / 2;
        assert_eq!(alpha(center, center), 0, "inside the body stays clear");
        assert!(alpha(center, 9) > alpha(center, 2), "darker near the body");
        assert!(alpha(0, 0) < alpha(center, 9));
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
    }
}
