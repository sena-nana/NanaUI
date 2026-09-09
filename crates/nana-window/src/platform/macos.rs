use raw_window_handle::HasWindowHandle;
use window_vibrancy::{NSVisualEffectMaterial, apply_vibrancy, clear_vibrancy};

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    _appearance: Appearance,
    _fallback: FallbackColor,
) -> MaterialOutcome {
    match requested {
        MaterialEffect::Solid => {
            clear(window);
            MaterialOutcome::chosen_solid()
        }
        MaterialEffect::Transparent => {
            clear(window);
            MaterialOutcome::transparent()
        }
        MaterialEffect::Vibrancy => match apply_vibrancy(
            window,
            NSVisualEffectMaterial::UnderWindowBackground,
            None,
            Some(16.0),
        ) {
            Ok(()) => MaterialOutcome::native(MaterialEffect::Vibrancy),
            Err(_) => MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable),
        },
        MaterialEffect::Mica | MaterialEffect::Acrylic => {
            clear(window);
            MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
        }
    }
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(window: &W) {
    let _ = clear_vibrancy(window);
}

pub(crate) fn set_application_icon_png(png: &[u8]) {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    // AppKit requires the main thread; `mtm` is taken above.
    unsafe {
        NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
    }
}

// One ObjC object receives every menu selection and turns it back into an id.
//
// A menu item's target must be an ObjC object, so there has to be a class; it
// holds no state, because the id travels on the item's `tag`.
objc2::define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "NanaMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(nanaMenuAction:))]
        fn nana_menu_action(&self, sender: &objc2_app_kit::NSMenuItem) {
            let tag = sender.tag();
            if let Ok(id) = u32::try_from(tag) {
                crate::menu::push_activation(id);
            }
        }
    }
);

/// The shared target. Menu items borrow it; it lives for the process.
fn menu_target(mtm: objc2::MainThreadMarker) -> objc2::rc::Retained<MenuTarget> {
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use std::sync::OnceLock;

    // `Retained` is not `Sync`, so the singleton is kept as a raw pointer and
    // retained again on each read. AppKit only reaches it on the main thread,
    // which `mtm` witnesses.
    static TARGET: OnceLock<usize> = OnceLock::new();
    let _ = mtm;
    let pointer = *TARGET.get_or_init(|| {
        let target: Retained<MenuTarget> = unsafe { objc2::msg_send![MenuTarget::alloc(), init] };
        Retained::into_raw(target) as usize
    });
    unsafe { Retained::retain(pointer as *mut MenuTarget).expect("menu target is alive") }
}

/// Builds `NSMenu` from the model and makes it the application's main menu.
///
/// The first menu is the application menu: AppKit gives that slot the process
/// name and puts Quit in it, so the caller's first menu is expected to be the
/// app menu and its entries are appended after a standard Quit item.
pub(crate) fn install_menu_bar(bar: &crate::MenuBar) {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        // AppKit menus are main-thread only. Silently doing nothing beats
        // corrupting the menu bar from a worker.
        return;
    };

    fn build_menu(mtm: MainThreadMarker, menu: &crate::Menu) -> Retained<NSMenu> {
        let title = NSString::from_str(&menu.title);
        let built = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
        // A menu that autoenables its items would grey out everything we do
        // not wire to a responder, so the model owns enablement instead.
        built.setAutoenablesItems(false);
        for entry in &menu.entries {
            match entry {
                crate::MenuEntry::Separator => {
                    built.addItem(&NSMenuItem::separatorItem(mtm));
                }
                crate::MenuEntry::Submenu(child) => {
                    let child_title = NSString::from_str(&child.title);
                    // AppKit init/target setters are `unsafe` in objc2; the
                    // arguments here are all owned by this function.
                    let item = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &child_title,
                            None,
                            &NSString::new(),
                        )
                    };
                    let submenu = build_menu(mtm, child);
                    built.setSubmenu_forItem(Some(&submenu), &item);
                    built.addItem(&item);
                }
                crate::MenuEntry::Item {
                    id,
                    label,
                    shortcut,
                    enabled,
                    checked,
                } => {
                    let title = NSString::from_str(label);
                    let key = NSString::from_str(
                        shortcut
                            .as_ref()
                            .map(|shortcut| shortcut.key.as_str())
                            .unwrap_or(""),
                    );
                    let item = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &title,
                            Some(objc2::sel!(nanaMenuAction:)),
                            &key,
                        )
                    };
                    if let Some(shortcut) = shortcut {
                        let mut flags = NSEventModifierFlags::empty();
                        if shortcut.primary {
                            flags |= NSEventModifierFlags::Command;
                        }
                        if shortcut.shift {
                            flags |= NSEventModifierFlags::Shift;
                        }
                        if shortcut.alt {
                            flags |= NSEventModifierFlags::Option;
                        }
                        item.setKeyEquivalentModifierMask(flags);
                    }
                    // The id rides on the item's tag, which is what the action
                    // reads back; no per-item Rust state to keep alive.
                    item.setTag(*id as isize);
                    item.setEnabled(*enabled);
                    item.setState(if *checked {
                        objc2_app_kit::NSControlStateValueOn
                    } else {
                        objc2_app_kit::NSControlStateValueOff
                    });
                    unsafe { item.setTarget(Some(&menu_target(mtm))) };
                    built.addItem(&item);
                }
            }
        }
        built
    }

    let root = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::new());
    root.setAutoenablesItems(false);
    for menu in &bar.menus {
        let title = NSString::from_str(&menu.title);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                None,
                &NSString::new(),
            )
        };
        let submenu = build_menu(mtm, menu);
        root.setSubmenu_forItem(Some(&submenu), &item);
        root.addItem(&item);
    }
    let app = NSApplication::sharedApplication(mtm);
    app.setMainMenu(Some(&root));
}

/// Reads back the installed main menu: each top-level title with its item
/// titles. Verification entry — the host asks AppKit what it actually holds.
pub(crate) fn installed_menu_bar() -> Option<Vec<(String, Vec<String>)>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let root = app.mainMenu()?;
    let mut bar = Vec::new();
    for index in 0..root.numberOfItems() {
        let Some(item) = root.itemAtIndex(index) else {
            continue;
        };
        let title = item.title().to_string();
        let mut entries = Vec::new();
        if let Some(submenu) = item.submenu() {
            for child in 0..submenu.numberOfItems() {
                if let Some(child) = submenu.itemAtIndex(child) {
                    entries.push(if child.isSeparatorItem() {
                        "-".to_owned()
                    } else {
                        child.title().to_string()
                    });
                }
            }
        }
        bar.push((title, entries));
    }
    Some(bar)
}
