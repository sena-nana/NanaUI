//! 真实 Win32 后端回归：owned 顶层窗口保留系统标题栏，Popup 保留父窗口坐标。

use dpi::{PhysicalPosition, PhysicalSize};
use rwh_06::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetWindow, GetWindowLongW, GetWindowRect, WS_CAPTION,
    WS_CHILD, WS_EX_APPWINDOW, WS_POPUP, WS_SYSMENU,
};
use winit_core::window::{Window, WindowAttributes, WindowType};

use crate::{EventLoop, PlatformSpecificEventLoopAttributes, WindowAttributesWindows};

/// 从本地后端创建的窗口读取句柄，确保断言检查实际 HWND。
fn hwnd(window: &dyn Window) -> HWND {
    let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() else {
        panic!("Win32 backend returned a non-Windows window");
    };
    handle.hwnd.get() as HWND
}

/// 对照 Win32 的窗口矩形，防止 API 往返转换掩盖屏幕坐标错误。
fn assert_screen_position(window: &dyn Window, expected: PhysicalPosition<i32>) {
    let mut rect = RECT::default();
    // SAFETY: 句柄属于本测试线程持有的存活窗口，rect 是有效输出缓冲区。
    assert_ne!(unsafe { GetWindowRect(hwnd(window), &mut rect) }, 0);
    assert_eq!(PhysicalPosition::new(rect.left, rect.top), expected);
}

/// 每次样式重写后验证标题栏、系统菜单和 ownership 均由系统维护。
fn assert_owned_caption(window: &dyn Window, owner: HWND) {
    let handle = hwnd(window);
    // SAFETY: 所有查询均针对测试持有的存活 HWND。
    let style = unsafe { GetWindowLongW(handle, GWL_STYLE) } as u32;
    let ex_style = unsafe { GetWindowLongW(handle, GWL_EXSTYLE) } as u32;
    assert_eq!(style & WS_CAPTION, WS_CAPTION);
    assert_eq!(style & WS_SYSMENU, WS_SYSMENU);
    assert_eq!(style & (WS_POPUP | WS_CHILD), 0);
    assert_eq!(ex_style & WS_EX_APPWINDOW, 0);
    assert_eq!(unsafe { GetWindow(handle, GW_OWNER) }, owner);
    assert!(window.is_decorated());

    let mut client_origin = POINT::default();
    let mut outer = RECT::default();
    // SAFETY: 输出缓冲区和窗口句柄在调用期间均有效。
    assert_ne!(unsafe { ClientToScreen(handle, &mut client_origin) }, 0);
    assert_ne!(unsafe { GetWindowRect(handle, &mut outer) }, 0);
    assert!(client_origin.y > outer.top, "system caption must reserve non-client space");
}

#[test]
fn owned_native_caption_and_popup_keep_distinct_window_contracts() {
    // 直接使用当前 crate 的 EventLoop，避免 dev-dependency facade 选择其他后端。
    // 单个事件循环覆盖所有场景，遵守 Win32 后端每进程仅创建一次的约束。
    let event_loop = EventLoop::new(&mut PlatformSpecificEventLoopAttributes {
        any_thread: true,
        ..Default::default()
    })
    .unwrap();
    let target = event_loop.window_target();
    let parent = target
        .create_window(
            WindowAttributes::default()
                .with_title("owner regression parent")
                .with_visible(false)
                .with_position(PhysicalPosition::new(140, 150))
                .with_surface_size(PhysicalSize::new(480, 360)),
        )
        .unwrap();
    let parent_handle = hwnd(&*parent);
    let position = PhysicalPosition::new(340, 260);
    let owned = target
        .create_window(
            WindowAttributes::default()
                .with_title("owned native caption regression")
                .with_visible(false)
                .with_decorations(true)
                .with_position(position)
                .with_surface_size(PhysicalSize::new(320, 240))
                .with_platform_attributes(Box::new(
                    WindowAttributesWindows::default()
                        .with_owner_window(parent_handle)
                        .with_drag_and_drop(false),
                )),
        )
        .unwrap();
    assert_owned_caption(&*owned, parent_handle);
    assert_screen_position(&*owned, position);
    assert_eq!(owned.outer_position().unwrap(), position);

    // 触发实际后端的完整样式重写，确认状态变化继续保留系统窗框。
    owned.set_resizable(false);
    assert_owned_caption(&*owned, parent_handle);
    owned.set_resizable(true);
    owned.set_decorations(false);
    owned.set_decorations(true);
    assert_owned_caption(&*owned, parent_handle);
    let moved = PhysicalPosition::new(390, 300);
    owned.set_outer_position(moved.into());
    assert_screen_position(&*owned, moved);
    assert_eq!(owned.outer_position().unwrap(), moved);

    let relative = PhysicalPosition::new(22, 34);
    // SAFETY: parent 保持存活直到 popup 在当前作用域结束时释放。
    let attributes = unsafe {
        WindowAttributes::default()
            .with_parent_window(Some(parent.window_handle().unwrap().as_raw()))
    }
    .with_window_type(WindowType::Popup)
    .with_visible(false)
    .with_active(false)
    .with_decorations(false)
    .with_position(relative)
    .with_surface_size(PhysicalSize::new(120, 80));
    let popup = target.create_window(attributes).unwrap();
    // SAFETY: popup 属于同一线程的存活窗口。
    let style = unsafe { GetWindowLongW(hwnd(&*popup), GWL_STYLE) } as u32;
    assert_eq!(style & WS_POPUP, WS_POPUP);
    assert_eq!(style & (WS_CAPTION | WS_CHILD), 0);
    let mut expected = POINT { x: relative.x, y: relative.y };
    // SAFETY: parent 和输出缓冲区均有效。
    assert_ne!(unsafe { ClientToScreen(parent_handle, &mut expected) }, 0);
    assert_screen_position(&*popup, PhysicalPosition::new(expected.x, expected.y));
    assert_eq!(popup.outer_position().unwrap(), relative);
    assert!(!popup.is_resizable());
}
