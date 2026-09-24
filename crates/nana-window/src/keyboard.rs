//! Global modifier-key sampling.
//!
//! During an OS drag the source application keeps keyboard focus, so the
//! target window never sees key or modifier events. The host samples the
//! system state instead when it maps drag events.

/// Modifier keys held down right now, as seen by the whole system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyboardModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

/// Current system modifier state. `None` where the platform cannot sample it
/// without focus (Linux); callers keep their last tracked state instead.
pub fn keyboard_modifiers() -> Option<KeyboardModifiers> {
    #[cfg(target_os = "windows")]
    {
        Some(windows_modifiers())
    }
    #[cfg(target_os = "macos")]
    {
        Some(macos_modifiers())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_modifiers() -> KeyboardModifiers {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    // The high bit is set while the key is down.
    let down = |key: u16| unsafe { GetAsyncKeyState(i32::from(key)) } < 0;
    KeyboardModifiers {
        alt: down(VK_MENU),
        control: down(VK_CONTROL),
        meta: down(VK_LWIN) || down(VK_RWIN),
        shift: down(VK_SHIFT),
    }
}

#[cfg(target_os = "macos")]
fn macos_modifiers() -> KeyboardModifiers {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};

    let flags = NSEvent::modifierFlags_class();
    KeyboardModifiers {
        alt: flags.contains(NSEventModifierFlags::Option),
        control: flags.contains(NSEventModifierFlags::Control),
        meta: flags.contains(NSEventModifierFlags::Command),
        shift: flags.contains(NSEventModifierFlags::Shift),
    }
}
