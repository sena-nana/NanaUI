//! Applies a custom cursor to a real window and reads back what the platform shows.
//!
//! On Windows the probe opens a small topmost window, applies two custom
//! cursors (1x and 2x) through the public cursor API, moves the pointer over
//! the window so `WM_SETCURSOR` runs, and reads the system cursor back with
//! `GetCursorInfo` / `GetIconInfo` (size and hotspot). Clearing the override
//! must hand the window back to its class cursor. The pointer is put back where
//! it was. Other platforms print support and exit. Exits non-zero on a mismatch.

fn main() -> std::process::ExitCode {
    println!(
        "capabilities: {:?}",
        nana_window::cursor::custom_cursor_capabilities()
    );
    #[cfg(target_os = "windows")]
    {
        match windows_live::run() {
            Ok(()) => {
                println!(
                    "OK: the window shows each custom cursor at its size and hotspot, then lets it go"
                );
                std::process::ExitCode::SUCCESS
            }
            Err(failure) => {
                eprintln!("FAIL: {failure}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        println!("no live read-back on this platform; nothing to verify");
        std::process::ExitCode::SUCCESS
    }
}

#[cfg(target_os = "windows")]
mod windows_live {
    use std::num::NonZeroIsize;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nana_window::cursor::{
        CursorBitmap, CustomCursorOutcome, PlatformCursorCache, apply_custom_cursor,
        clear_custom_cursor,
    };
    use raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    };
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CURSORINFO, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        GetCursorInfo, GetCursorPos, GetIconInfo, ICONINFO, IDC_ARROW, LoadCursorW, MSG, PM_REMOVE,
        PeekMessageW, RegisterClassW, SetCursorPos, TranslateMessage, WNDCLASSW, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };

    struct Window(HWND);

    impl HasWindowHandle for Window {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let hwnd = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
            // SAFETY: the window outlives every borrow the probe takes.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: created by `open` on this thread.
            unsafe { DestroyWindow(self.0) };
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    const ORIGIN: (i32, i32) = (120, 120);
    const EDGE: i32 = 240;

    fn open() -> Result<Window, String> {
        let class = wide("NanaCursorProbe");
        // SAFETY: plain Win32 window creation with a static class name.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let mut definition: WNDCLASSW = std::mem::zeroed();
            definition.lpfnWndProc = Some(DefWindowProcW);
            definition.hInstance = instance;
            definition.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
            definition.lpszClassName = class.as_ptr();
            RegisterClassW(&definition);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                ORIGIN.0,
                ORIGIN.1,
                EDGE,
                EDGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW failed".into());
            }
            Ok(Window(hwnd))
        }
    }

    fn pump(duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            // SAFETY: a zeroed MSG is a valid out-parameter.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Moves the pointer within the window so it asks for its cursor again.
    fn hover(step: i32) {
        let (x, y) = (ORIGIN.0 + EDGE / 2 + step, ORIGIN.1 + EDGE / 2);
        unsafe { SetCursorPos(x, y) };
        pump(Duration::from_millis(250));
    }

    /// The shown cursor's edge in pixels and its hotspot.
    fn shown() -> Result<(i32, (u32, u32)), String> {
        // SAFETY: plain data out-parameters; the bitmaps GetIconInfo hands
        // back are deleted here.
        unsafe {
            let mut info: CURSORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut info) == 0 || info.hCursor.is_null() {
                return Err("GetCursorInfo failed".into());
            }
            let mut icon: ICONINFO = std::mem::zeroed();
            if GetIconInfo(info.hCursor, &mut icon) == 0 {
                return Err("GetIconInfo failed".into());
            }
            let source = if icon.hbmColor.is_null() {
                icon.hbmMask
            } else {
                icon.hbmColor
            };
            let mut bitmap: BITMAP = std::mem::zeroed();
            GetObjectW(
                source,
                std::mem::size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            );
            for handle in [icon.hbmColor, icon.hbmMask] {
                if !handle.is_null() {
                    DeleteObject(handle);
                }
            }
            Ok((bitmap.bmWidth, (icon.xHotspot, icon.yHotspot)))
        }
    }

    fn bitmap(edge: u16, hotspot: [u16; 2], scale_milli: u32, key: u64) -> CursorBitmap {
        let mut rgba = Vec::with_capacity(usize::from(edge) * usize::from(edge) * 4);
        for _ in 0..usize::from(edge) * usize::from(edge) {
            rgba.extend_from_slice(&[230, 40, 60, 255]);
        }
        let logical = edge * 1000 / scale_milli as u16;
        CursorBitmap {
            cache_key: key,
            scale_milli,
            logical_width: logical,
            logical_height: logical,
            logical_hotspot: [
                f64::from(hotspot[0]) * 1000.0 / f64::from(scale_milli),
                f64::from(hotspot[1]) * 1000.0 / f64::from(scale_milli),
            ],
            width: edge,
            height: edge,
            hotspot,
            rgba: Arc::from(rgba),
        }
    }

    fn expect_custom(
        window: &Window,
        cache: &mut PlatformCursorCache,
        bitmap: &CursorBitmap,
        step: i32,
    ) -> Result<(), String> {
        let handle = cache.get_or_create(bitmap)?;
        let outcome = apply_custom_cursor(window, &handle, bitmap.scale_milli);
        println!("apply {}x: {outcome:?}", bitmap.scale_milli / 1000);
        if !matches!(outcome, CustomCursorOutcome::Applied { .. }) {
            return Err(format!("apply was not applied: {outcome:?}"));
        }
        hover(step);
        let seen = shown()?;
        println!("shown: edge {} hotspot {:?}", seen.0, seen.1);
        let want = (
            i32::from(bitmap.width),
            (u32::from(bitmap.hotspot[0]), u32::from(bitmap.hotspot[1])),
        );
        if seen != want {
            return Err(format!("expected edge/hotspot {want:?}, saw {seen:?}"));
        }
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        let mut saved = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut saved) };
        let result = (|| {
            let window = open()?;
            pump(Duration::from_millis(150));
            hover(0);
            let arrow = shown()?;
            println!("class cursor: edge {} hotspot {:?}", arrow.0, arrow.1);
            let mut cache = PlatformCursorCache::default();
            expect_custom(&window, &mut cache, &bitmap(32, [5, 7], 1000, 1), 3)?;
            expect_custom(&window, &mut cache, &bitmap(64, [10, 14], 2000, 1), -3)?;
            clear_custom_cursor(&window);
            hover(5);
            let cleared = shown()?;
            println!("after clear: edge {} hotspot {:?}", cleared.0, cleared.1);
            if cleared != arrow {
                return Err(format!(
                    "clearing did not restore the class cursor {arrow:?}, saw {cleared:?}"
                ));
            }
            Ok(())
        })();
        unsafe { SetCursorPos(saved.x, saved.y) };
        result
    }
}
