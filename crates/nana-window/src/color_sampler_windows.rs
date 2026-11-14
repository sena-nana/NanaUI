use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, BeginPaint, BitBlt, CLEARTYPE_QUALITY,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, Ellipse, EndPaint, FW_MEDIUM, GetDC,
    GetDIBits, GetDeviceCaps, GetStockObject, HBRUSH, HFONT, InvalidateRect, LOGPIXELSX, LineTo,
    MoveToEx, NULL_BRUSH, PAINTSTRUCT, PS_SOLID, Rectangle, ReleaseDC, SRCCOPY, SelectObject,
    SetBkMode, SetTextColor, StretchDIBits, TRANSPARENT, TextOutW, WHITE_PEN,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HWND_TOPMOST, IDC_CROSS,
    LWA_ALPHA, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW,
    SW_SHOWNA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursor, SetCursorPos, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, UnregisterClassW, WDA_EXCLUDEFROMCAPTURE, WM_CLOSE, WM_CREATE,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NCCREATE,
    WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::color_sampler::{
    ColorSamplerError, ColorSamplerHandle, ColorSamplerResult, hex_rgb_u8, srgb_u8_to_rgb,
};
use crate::win32::wide;

const LOUPE_GRID: i32 = 11;
const LOUPE_CELL_BASE: i32 = 12;
const LOUPE_PAD_BASE: i32 = 10;
const LOUPE_FOOTER_BASE: i32 = 34;
const LOUPE_OFFSET_BASE: i32 = 20;

struct CancelState {
    cancelled: AtomicBool,
    overlay_hwnd: AtomicIsize,
}

impl CancelState {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let raw = self.overlay_hwnd.load(Ordering::Acquire);
        if raw != 0 {
            // SAFETY: `overlay_hwnd` stores a live Win32 HWND created on the sampler thread until `WM_DESTROY`.
            unsafe {
                PostMessageW(raw as HWND, WM_CLOSE, 0, 0);
            }
        }
    }
}

struct VirtualRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct ScreenBuffer {
    rect: VirtualRect,
    bgra: Vec<u8>,
}

impl ScreenBuffer {
    fn capture(rect: VirtualRect) -> Option<Self> {
        if rect.w <= 0 || rect.h <= 0 {
            return None;
        }
        let len = usize::try_from(rect.w)
            .ok()?
            .checked_mul(usize::try_from(rect.h).ok()?)?
            .checked_mul(4)?;
        let mut bgra = vec![0u8; len];
        // SAFETY: Captures the virtual desktop surface into a top-down 32bpp DIB using valid GDI handles.
        unsafe {
            let screen = GetDC(ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem_dc = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, rect.w, rect.h);
            let ok = if !mem_dc.is_null() && !bmp.is_null() {
                let prev = SelectObject(mem_dc, bmp);
                let blt = BitBlt(
                    mem_dc, 0, 0, rect.w, rect.h, screen, rect.x, rect.y, SRCCOPY,
                );
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader = top_down_bmi(rect.w, rect.h);
                let rows = if blt != 0 {
                    GetDIBits(
                        mem_dc,
                        bmp,
                        0,
                        rect.h as u32,
                        bgra.as_mut_ptr().cast(),
                        &mut bmi,
                        DIB_RGB_COLORS,
                    )
                } else {
                    0
                };
                SelectObject(mem_dc, prev);
                rows > 0
            } else {
                false
            };
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            if !mem_dc.is_null() {
                DeleteDC(mem_dc);
            }
            ReleaseDC(ptr::null_mut(), screen);
            ok.then_some(Self { rect, bgra })
        }
    }

    fn pixel_bgra(&self, sx: i32, sy: i32) -> [u8; 4] {
        let rx = sx - self.rect.x;
        let ry = sy - self.rect.y;
        if rx < 0 || ry < 0 || rx >= self.rect.w || ry >= self.rect.h {
            return [24, 24, 27, 255];
        }
        let idx = ((ry as usize) * (self.rect.w as usize) + (rx as usize)) * 4;
        self.bgra
            .get(idx..idx + 4)
            .map_or([24, 24, 27, 255], |s| [s[0], s[1], s[2], 255])
    }
}

fn top_down_bmi(w: i32, h: i32) -> BITMAPINFOHEADER {
    BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    }
}

fn capture_patch(
    rect: &VirtualRect,
    fallback: Option<&ScreenBuffer>,
    cx: i32,
    cy: i32,
) -> (Vec<u8>, [u8; 3]) {
    let half = LOUPE_GRID / 2;
    let total = (LOUPE_GRID * LOUPE_GRID * 4) as usize;
    let mut patch = vec![0u8; total];

    if let Some(buf) = fallback {
        for py in 0..LOUPE_GRID {
            for px in 0..LOUPE_GRID {
                let c = buf.pixel_bgra(cx - half + px, cy - half + py);
                let idx = ((py * LOUPE_GRID + px) * 4) as usize;
                patch[idx..idx + 4].copy_from_slice(&c);
            }
        }
    } else {
        // SAFETY: Reads an 11x11 live desktop patch around `(cx, cy)` when `WDA_EXCLUDEFROMCAPTURE` hides our windows.
        unsafe {
            let screen = GetDC(ptr::null_mut());
            if !screen.is_null() {
                let mem_dc = CreateCompatibleDC(screen);
                let bmp = CreateCompatibleBitmap(screen, LOUPE_GRID, LOUPE_GRID);
                if !mem_dc.is_null() && !bmp.is_null() {
                    let prev = SelectObject(mem_dc, bmp);
                    let blt = BitBlt(
                        mem_dc,
                        0,
                        0,
                        LOUPE_GRID,
                        LOUPE_GRID,
                        screen,
                        cx - half,
                        cy - half,
                        SRCCOPY,
                    );
                    if blt != 0 {
                        let mut bmi: BITMAPINFO = std::mem::zeroed();
                        bmi.bmiHeader = top_down_bmi(LOUPE_GRID, LOUPE_GRID);
                        GetDIBits(
                            mem_dc,
                            bmp,
                            0,
                            LOUPE_GRID as u32,
                            patch.as_mut_ptr().cast(),
                            &mut bmi,
                            DIB_RGB_COLORS,
                        );
                    }
                    SelectObject(mem_dc, prev);
                }
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                if !mem_dc.is_null() {
                    DeleteDC(mem_dc);
                }
                ReleaseDC(ptr::null_mut(), screen);
            }
        }
        // Fill off-screen coordinates with a dark border color.
        for py in 0..LOUPE_GRID {
            for px in 0..LOUPE_GRID {
                let sx = cx - half + px;
                let sy = cy - half + py;
                if sx < rect.x || sy < rect.y || sx >= rect.x + rect.w || sy >= rect.y + rect.h {
                    let idx = ((py * LOUPE_GRID + px) * 4) as usize;
                    patch[idx..idx + 4].copy_from_slice(&[24, 24, 27, 255]);
                }
            }
        }
    }

    let center_idx = ((half * LOUPE_GRID + half) * 4) as usize;
    let b = patch[center_idx];
    let g = patch[center_idx + 1];
    let r = patch[center_idx + 2];
    (patch, [r, g, b])
}

struct SamplerWindowState {
    request_id: u64,
    virtual_rect: VirtualRect,
    fallback: Option<ScreenBuffer>,
    cancel_state: Arc<CancelState>,
    overlay_hwnd: HWND,
    loupe_hwnd: HWND,
    scale: i32,
    loupe_w: i32,
    loupe_h: i32,
    cursor_x: i32,
    cursor_y: i32,
    crosshair_cursor: *mut c_void,
    completion: Option<Box<dyn FnOnce(ColorSamplerResult) + Send>>,
}

impl SamplerWindowState {
    fn finish(&mut self, result: ColorSamplerResult) {
        if let Some(done) = self.completion.take() {
            done(result);
        }
        // SAFETY: Runs on the owning Win32 thread to release mouse capture and close both sampler windows.
        unsafe {
            ReleaseCapture();
            if !self.loupe_hwnd.is_null() {
                let loupe = self.loupe_hwnd;
                self.loupe_hwnd = ptr::null_mut();
                DestroyWindow(loupe);
            }
            if !self.overlay_hwnd.is_null() {
                let overlay = self.overlay_hwnd;
                self.overlay_hwnd = ptr::null_mut();
                DestroyWindow(overlay);
            }
        }
    }

    fn pick_at(&mut self, x: i32, y: i32) {
        let (_, rgb) = capture_patch(&self.virtual_rect, self.fallback.as_ref(), x, y);
        let result =
            ColorSamplerResult::picked(self.request_id, srgb_u8_to_rgb(rgb[0], rgb[1], rgb[2]));
        self.finish(result);
    }

    fn cancel(&mut self) {
        self.finish(ColorSamplerResult::cancelled(self.request_id));
    }

    fn move_cursor_to(&mut self, x: i32, y: i32) {
        self.cursor_x = x.clamp(
            self.virtual_rect.x,
            self.virtual_rect.x + self.virtual_rect.w - 1,
        );
        self.cursor_y = y.clamp(
            self.virtual_rect.y,
            self.virtual_rect.y + self.virtual_rect.h - 1,
        );
        if self.loupe_hwnd.is_null() {
            return;
        }
        let offset = LOUPE_OFFSET_BASE * self.scale;
        let max_x = self.virtual_rect.x + self.virtual_rect.w - self.loupe_w - offset;
        let max_y = self.virtual_rect.y + self.virtual_rect.h - self.loupe_h - offset;
        let lx =
            if self.cursor_x + offset + self.loupe_w > self.virtual_rect.x + self.virtual_rect.w {
                (self.cursor_x - offset - self.loupe_w).max(self.virtual_rect.x)
            } else {
                self.cursor_x + offset
            }
            .min(max_x.max(self.virtual_rect.x));
        let ly =
            if self.cursor_y + offset + self.loupe_h > self.virtual_rect.y + self.virtual_rect.h {
                (self.cursor_y - offset - self.loupe_h).max(self.virtual_rect.y)
            } else {
                self.cursor_y + offset
            }
            .min(max_y.max(self.virtual_rect.y));

        // SAFETY: Moves and triggers repaint of `loupe_hwnd` on the owning UI thread.
        unsafe {
            SetWindowPos(
                self.loupe_hwnd,
                HWND_TOPMOST,
                lx,
                ly,
                self.loupe_w,
                self.loupe_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.loupe_hwnd, ptr::null(), 0);
        }
    }

    fn paint_loupe(&self, hwnd: HWND) {
        // SAFETY: Paints double-buffered zoomed pixel grid + swatch + hex label into `loupe_hwnd`.
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            if hdc.is_null() {
                return;
            }
            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, self.loupe_w, self.loupe_h);
            if !mem_dc.is_null() && !mem_bmp.is_null() {
                let prev_bmp = SelectObject(mem_dc, mem_bmp);
                let scale = self.scale;
                let pad = LOUPE_PAD_BASE * scale;
                let cell = LOUPE_CELL_BASE * scale;
                let grid_px = LOUPE_GRID * cell;

                let bg_brush = CreateSolidBrush(rgb_ref(24, 24, 27));
                let border_pen = CreatePen(PS_SOLID as i32, scale, rgb_ref(63, 63, 70));
                let old_brush = SelectObject(mem_dc, bg_brush as *mut c_void);
                let old_pen = SelectObject(mem_dc, border_pen as *mut c_void);
                Rectangle(mem_dc, 0, 0, self.loupe_w, self.loupe_h);

                let (patch, rgb) = capture_patch(
                    &self.virtual_rect,
                    self.fallback.as_ref(),
                    self.cursor_x,
                    self.cursor_y,
                );
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader = top_down_bmi(LOUPE_GRID, LOUPE_GRID);
                StretchDIBits(
                    mem_dc,
                    pad,
                    pad,
                    grid_px,
                    grid_px,
                    0,
                    0,
                    LOUPE_GRID,
                    LOUPE_GRID,
                    patch.as_ptr().cast(),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );

                // subtle grid lines
                let grid_pen = CreatePen(PS_SOLID as i32, 1, rgb_ref(46, 46, 52));
                SelectObject(mem_dc, grid_pen as *mut c_void);
                for i in 1..LOUPE_GRID {
                    let gx = pad + i * cell;
                    MoveToEx(mem_dc, gx, pad, ptr::null_mut());
                    LineTo(mem_dc, gx, pad + grid_px);
                    let gy = pad + i * cell;
                    MoveToEx(mem_dc, pad, gy, ptr::null_mut());
                    LineTo(mem_dc, pad + grid_px, gy);
                }
                DeleteObject(grid_pen as *mut c_void);

                // center cell highlight
                let center = LOUPE_GRID / 2;
                let cx0 = pad + center * cell;
                let cy0 = pad + center * cell;
                let null_brush = GetStockObject(NULL_BRUSH as i32);
                SelectObject(mem_dc, null_brush);
                let white_pen = GetStockObject(WHITE_PEN as i32);
                SelectObject(mem_dc, white_pen);
                Rectangle(mem_dc, cx0, cy0, cx0 + cell, cy0 + cell);
                let black_pen = CreatePen(PS_SOLID as i32, 1, rgb_ref(0, 0, 0));
                SelectObject(mem_dc, black_pen as *mut c_void);
                Rectangle(mem_dc, cx0 + 1, cy0 + 1, cx0 + cell - 1, cy0 + cell - 1);
                DeleteObject(black_pen as *mut c_void);

                // footer: swatch + hex label
                let footer_top = pad + grid_px + 8 * scale;
                let swatch_size = 18 * scale;
                let swatch_brush = CreateSolidBrush(rgb_ref(rgb[0], rgb[1], rgb[2]));
                let swatch_pen = CreatePen(PS_SOLID as i32, 1, rgb_ref(82, 82, 91));
                SelectObject(mem_dc, swatch_brush as *mut c_void);
                SelectObject(mem_dc, swatch_pen as *mut c_void);
                Ellipse(
                    mem_dc,
                    pad,
                    footer_top,
                    pad + swatch_size,
                    footer_top + swatch_size,
                );
                DeleteObject(swatch_brush as *mut c_void);
                DeleteObject(swatch_pen as *mut c_void);

                let face = wide("Consolas");
                let font: HFONT = CreateFontW(
                    -(13 * scale),
                    0,
                    0,
                    0,
                    FW_MEDIUM as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    0,
                    0,
                    CLEARTYPE_QUALITY as u32,
                    0,
                    face.as_ptr(),
                );
                let old_font = if !font.is_null() {
                    SelectObject(mem_dc, font as *mut c_void)
                } else {
                    ptr::null_mut()
                };
                SetBkMode(mem_dc, TRANSPARENT as i32);
                SetTextColor(mem_dc, rgb_ref(244, 244, 245));
                let hex = hex_rgb_u8(rgb[0], rgb[1], rgb[2]);
                let hex_w: Vec<u16> = hex.encode_utf16().collect();
                TextOutW(
                    mem_dc,
                    pad + swatch_size + 8 * scale,
                    footer_top + 2 * scale,
                    hex_w.as_ptr(),
                    hex_w.len() as i32,
                );
                if !font.is_null() {
                    SelectObject(mem_dc, old_font);
                    DeleteObject(font as *mut c_void);
                }

                SelectObject(mem_dc, old_pen);
                SelectObject(mem_dc, old_brush);
                DeleteObject(border_pen as *mut c_void);
                DeleteObject(bg_brush as *mut c_void);

                BitBlt(hdc, 0, 0, self.loupe_w, self.loupe_h, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, prev_bmp);
            }
            if !mem_bmp.is_null() {
                DeleteObject(mem_bmp as *mut c_void);
            }
            if !mem_dc.is_null() {
                DeleteDC(mem_dc);
            }
            EndPaint(hwnd, &ps);
        }
    }
}

#[inline]
fn rgb_ref(r: u8, g: u8, b: u8) -> COLORREF {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

fn window_state_mut<'a>(hwnd: HWND) -> Option<&'a mut SamplerWindowState> {
    // SAFETY: `GWLP_USERDATA` points to `SamplerWindowState` allocated on the sampler thread until `WM_DESTROY`.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut SamplerWindowState;
    (!ptr.is_null()).then(|| unsafe { &mut *ptr })
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
        return 1;
    }

    match msg {
        WM_CREATE => {
            if let Some(state) = window_state_mut(hwnd) {
                state
                    .cancel_state
                    .overlay_hwnd
                    .store(hwnd as isize, Ordering::Release);
            }
            0
        }
        WM_SETCURSOR => {
            if let Some(state) = window_state_mut(hwnd)
                && !state.crosshair_cursor.is_null()
            {
                unsafe { SetCursor(state.crosshair_cursor) };
                return 1;
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_MOUSEMOVE => {
            if let Some(state) = window_state_mut(hwnd) {
                let mut pt = POINT { x: 0, y: 0 };
                if unsafe { GetCursorPos(&mut pt) } != 0 {
                    state.move_cursor_to(pt.x, pt.y);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = window_state_mut(hwnd) {
                let mut pt = POINT {
                    x: state.cursor_x,
                    y: state.cursor_y,
                };
                unsafe { GetCursorPos(&mut pt) };
                state.pick_at(pt.x, pt.y);
            }
            0
        }
        WM_RBUTTONDOWN | WM_KILLFOCUS | WM_CLOSE => {
            if let Some(state) = window_state_mut(hwnd) {
                state.cancel();
            } else {
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_KEYDOWN => {
            if let Some(state) = window_state_mut(hwnd) {
                let key = wparam as u16;
                let step = if unsafe { GetKeyState(VK_SHIFT as i32) } < 0 {
                    10
                } else {
                    1
                };
                match key {
                    VK_ESCAPE => state.cancel(),
                    VK_RETURN | VK_SPACE => state.pick_at(state.cursor_x, state.cursor_y),
                    VK_LEFT => {
                        let nx = state.cursor_x - step;
                        unsafe { SetCursorPos(nx, state.cursor_y) };
                        state.move_cursor_to(nx, state.cursor_y);
                    }
                    VK_RIGHT => {
                        let nx = state.cursor_x + step;
                        unsafe { SetCursorPos(nx, state.cursor_y) };
                        state.move_cursor_to(nx, state.cursor_y);
                    }
                    VK_UP => {
                        let ny = state.cursor_y - step;
                        unsafe { SetCursorPos(state.cursor_x, ny) };
                        state.move_cursor_to(state.cursor_x, ny);
                    }
                    VK_DOWN => {
                        let ny = state.cursor_y + step;
                        unsafe { SetCursorPos(state.cursor_x, ny) };
                        state.move_cursor_to(state.cursor_x, ny);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_DESTROY => {
            if let Some(state) = window_state_mut(hwnd) {
                state.cancel_state.overlay_hwnd.store(0, Ordering::Release);
                if let Some(done) = state.completion.take() {
                    done(ColorSamplerResult::cancelled(state.request_id));
                }
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe extern "system" fn loupe_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
        return 1;
    }

    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(state) = window_state_mut(hwnd) {
                state.paint_loupe(hwnd);
                return 0;
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn validate_owner_window(window: Option<&dyn HasWindowHandle>) -> Result<(), ColorSamplerError> {
    let Some(window) = window else {
        return Ok(());
    };
    let handle = window
        .window_handle()
        .map_err(|_| ColorSamplerError::InvalidWindow)?;
    match handle.as_raw() {
        RawWindowHandle::Win32(raw) if raw.hwnd.get() != 0 => Ok(()),
        _ => Err(ColorSamplerError::InvalidWindow),
    }
}

fn run_sampler(
    request_id: u64,
    cancel_state: Arc<CancelState>,
    completion: Box<dyn FnOnce(ColorSamplerResult) + Send>,
) {
    if cancel_state.cancelled.load(Ordering::Acquire) {
        completion(ColorSamplerResult::cancelled(request_id));
        return;
    }

    // SAFETY: Configures per-monitor V2 DPI awareness for this worker thread before reading metrics or creating HWNDs.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if vw <= 0 || vh <= 0 {
        completion(ColorSamplerResult::failed(
            request_id,
            ColorSamplerError::Platform("empty virtual screen bounds".into()),
        ));
        return;
    }
    let virtual_rect = VirtualRect {
        x: vx,
        y: vy,
        w: vw,
        h: vh,
    };

    let mut pt = POINT {
        x: vx + vw / 2,
        y: vy + vh / 2,
    };
    unsafe { GetCursorPos(&mut pt) };

    let dpi = unsafe {
        let dc = GetDC(ptr::null_mut());
        let d = if dc.is_null() {
            96
        } else {
            let v = GetDeviceCaps(dc, LOGPIXELSX as i32);
            ReleaseDC(ptr::null_mut(), dc);
            v
        };
        d.max(96)
    };
    let scale = ((dpi + 48) / 96).clamp(1, 3);
    let loupe_w = (LOUPE_PAD_BASE * 2 + LOUPE_GRID * LOUPE_CELL_BASE) * scale;
    let loupe_h = (LOUPE_PAD_BASE * 2 + LOUPE_GRID * LOUPE_CELL_BASE + LOUPE_FOOTER_BASE) * scale;

    // Pre-capture the virtual screen before showing any sampler window as a fallback when
    // `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` is unsupported by the session/driver.
    let pre_snapshot = ScreenBuffer::capture(VirtualRect {
        x: vx,
        y: vy,
        w: vw,
        h: vh,
    });

    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(ptr::null()) };
    let overlay_class = wide(&format!("NanaColorSamplerOverlay_{request_id}"));
    let loupe_class = wide(&format!("NanaColorSamplerLoupe_{request_id}"));
    let crosshair = unsafe { LoadCursorW(ptr::null_mut(), IDC_CROSS) };

    let mut state = Box::new(SamplerWindowState {
        request_id,
        virtual_rect,
        fallback: pre_snapshot,
        cancel_state: Arc::clone(&cancel_state),
        overlay_hwnd: ptr::null_mut(),
        loupe_hwnd: ptr::null_mut(),
        scale,
        loupe_w,
        loupe_h,
        cursor_x: pt.x,
        cursor_y: pt.y,
        crosshair_cursor: crosshair,
        completion: Some(completion),
    });

    // SAFETY: Registers temporary window classes on this worker thread, runs a modal loop, and unregisters before returning.
    unsafe {
        let mut wc_overlay: WNDCLASSEXW = std::mem::zeroed();
        wc_overlay.cbSize = size_of::<WNDCLASSEXW>() as u32;
        wc_overlay.lpfnWndProc = Some(overlay_wnd_proc);
        wc_overlay.hInstance = hinstance;
        wc_overlay.hCursor = crosshair;
        wc_overlay.hbrBackground = GetStockObject(BLACK_BRUSH as i32) as HBRUSH;
        wc_overlay.lpszClassName = overlay_class.as_ptr();
        if RegisterClassExW(&wc_overlay) == 0 {
            if let Some(done) = state.completion.take() {
                done(ColorSamplerResult::failed(
                    request_id,
                    ColorSamplerError::Platform("failed to register overlay class".into()),
                ));
            }
            return;
        }

        let mut wc_loupe: WNDCLASSEXW = std::mem::zeroed();
        wc_loupe.cbSize = size_of::<WNDCLASSEXW>() as u32;
        wc_loupe.lpfnWndProc = Some(loupe_wnd_proc);
        wc_loupe.hInstance = hinstance;
        wc_loupe.lpszClassName = loupe_class.as_ptr();
        let loupe_registered = RegisterClassExW(&wc_loupe) != 0;

        let state_ptr: *mut SamplerWindowState = &mut *state;
        let overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            overlay_class.as_ptr(),
            ptr::null(),
            WS_POPUP,
            state.virtual_rect.x,
            state.virtual_rect.y,
            state.virtual_rect.w,
            state.virtual_rect.h,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            state_ptr.cast(),
        );
        if overlay.is_null() {
            UnregisterClassW(overlay_class.as_ptr(), hinstance);
            if loupe_registered {
                UnregisterClassW(loupe_class.as_ptr(), hinstance);
            }
            if let Some(done) = state.completion.take() {
                done(ColorSamplerResult::failed(
                    request_id,
                    ColorSamplerError::Platform("failed to create overlay window".into()),
                ));
            }
            return;
        }
        state.overlay_hwnd = overlay;
        SetLayeredWindowAttributes(overlay, 0, 1, LWA_ALPHA);

        let excluded_overlay = SetWindowDisplayAffinity(overlay, WDA_EXCLUDEFROMCAPTURE) != 0;
        if loupe_registered {
            let loupe = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                loupe_class.as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                state.loupe_w,
                state.loupe_h,
                overlay,
                ptr::null_mut(),
                hinstance,
                state_ptr.cast(),
            );
            if !loupe.is_null() {
                let excluded_loupe = SetWindowDisplayAffinity(loupe, WDA_EXCLUDEFROMCAPTURE) != 0;
                if excluded_overlay && excluded_loupe {
                    state.fallback = None;
                }
                state.loupe_hwnd = loupe;
            }
        } else if excluded_overlay {
            state.fallback = None;
        }

        ShowWindow(overlay, SW_SHOW);
        SetForegroundWindow(overlay);
        SetCapture(overlay);
        if !state.loupe_hwnd.is_null() {
            state.move_cursor_to(pt.x, pt.y);
            ShowWindow(state.loupe_hwnd, SW_SHOWNA);
        }

        if cancel_state.cancelled.load(Ordering::Acquire) {
            state.cancel();
        } else {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if !state.loupe_hwnd.is_null() {
            DestroyWindow(state.loupe_hwnd);
            state.loupe_hwnd = ptr::null_mut();
        }
        if !state.overlay_hwnd.is_null() {
            DestroyWindow(state.overlay_hwnd);
            state.overlay_hwnd = ptr::null_mut();
        }
        SetWindowLongPtrW(overlay, GWLP_USERDATA, 0);
        if loupe_registered {
            UnregisterClassW(loupe_class.as_ptr(), hinstance);
        }
        UnregisterClassW(overlay_class.as_ptr(), hinstance);
    }
}

pub(crate) fn pick_screen_color<F>(
    window: Option<&dyn HasWindowHandle>,
    request_id: u64,
    completion: F,
) -> Result<ColorSamplerHandle, ColorSamplerError>
where
    F: FnOnce(ColorSamplerResult) + Send + 'static,
{
    validate_owner_window(window)?;
    let cancel_state = Arc::new(CancelState {
        cancelled: AtomicBool::new(false),
        overlay_hwnd: AtomicIsize::new(0),
    });
    let thread_cancel = Arc::clone(&cancel_state);
    let handle_cancel = Arc::clone(&cancel_state);

    std::thread::Builder::new()
        .name("nana-color-sampler".into())
        .spawn(move || run_sampler(request_id, thread_cancel, Box::new(completion)))
        .map(|_| ColorSamplerHandle::new(move || handle_cancel.cancel()))
        .map_err(|error| ColorSamplerError::Platform(error.to_string()))
}
