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

/// Configures an `NSSavePanel` (or its `NSOpenPanel` subclass) from a request.
///
/// Split out so the configuration can be verified without running the panel:
/// a modal dialog cannot be driven from a test.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn configure_panel(
    panel: &objc2_app_kit::NSSavePanel,
    request: &nana_ui_core::FileDialogRequest,
) {
    use objc2_foundation::{NSArray, NSString, NSURL};

    if let Some(title) = &request.title {
        panel.setTitle(Some(&NSString::from_str(title)));
    }
    if let Some(directory) = &request.directory {
        let path = NSString::from_str(&directory.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        panel.setDirectoryURL(Some(&url));
    }
    if let Some(name) = &request.file_name {
        panel.setNameFieldStringValue(&NSString::from_str(name));
    }
    if !request.filters.is_empty() {
        // AppKit filters by extension list; the group names are the
        // application's own labels and have no AppKit counterpart here.
        let extensions = request
            .filters
            .iter()
            .flat_map(|filter| filter.extensions.iter())
            .map(|extension| NSString::from_str(extension))
            .collect::<Vec<_>>();
        let refs = extensions.iter().map(|value| &**value).collect::<Vec<_>>();
        let array = NSArray::from_slice(&refs);
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&array));
    }
}

/// Opens the system file dialog as a sheet on `window`.
///
/// A sheet rather than `runModal`: a modal run would block the event loop, so
/// the window behind the dialog would stop rendering. The completion handler
/// pushes the outcome onto the queue `take_file_dialog_results` drains, which
/// is the same shape the menu uses.
pub(crate) fn open_file_dialog<W: HasWindowHandle + ?Sized>(
    window: &W,
    request: nana_ui_core::FileDialogRequest,
) {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSWindow};

    let Some(mtm) = MainThreadMarker::new() else {
        // AppKit panels are main-thread only. Report a cancel rather than
        // leaving the caller waiting for a result that will never arrive.
        crate::file_dialog::push_result(nana_ui_core::FileDialogResult::cancelled(request.id));
        return;
    };

    let id = request.id;
    let multiple = request.kind.is_multiple();
    let save = request.kind.is_save();

    // `NSOpenPanel` is an `NSSavePanel`, so both configure through the same
    // reference and only the completion differs.
    let (panel, open): (Retained<NSSavePanel>, Option<Retained<NSOpenPanel>>) = if save {
        (NSSavePanel::savePanel(mtm), None)
    } else {
        let open = NSOpenPanel::openPanel(mtm);
        let folder = matches!(request.kind, nana_ui_core::FileDialogKind::PickFolder);
        open.setCanChooseFiles(!folder);
        open.setCanChooseDirectories(folder);
        open.setAllowsMultipleSelection(multiple);
        (Retained::into_super(open.clone()), Some(open))
    };
    configure_panel(&panel, &request);

    let sheet = panel.clone();
    let completion = RcBlock::new(move |response: NSModalResponse| {
        let mut paths = Vec::new();
        if response == NSModalResponseOK {
            if let Some(open) = &open {
                for url in &*open.URLs() {
                    if let Some(path) = url.path() {
                        paths.push(std::path::PathBuf::from(path.to_string()));
                    }
                }
            } else if let Some(url) = panel.URL()
                && let Some(path) = url.path()
            {
                paths.push(std::path::PathBuf::from(path.to_string()));
            }
        }
        crate::file_dialog::push_result(nana_ui_core::FileDialogResult { id, paths });
    });

    let Some(parent) = ns_window(window, mtm) else {
        // No parent to hang a sheet on: report a cancel instead of opening a
        // detached dialog the user cannot associate with anything.
        crate::file_dialog::push_result(nana_ui_core::FileDialogResult::cancelled(id));
        return;
    };
    let _: &NSWindow = &parent;
    sheet.beginSheetModalForWindow_completionHandler(&parent, &completion);
}

/// The `NSWindow` behind a raw handle.
fn ns_window<W: HasWindowHandle + ?Sized>(
    window: &W,
    _mtm: objc2::MainThreadMarker,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // The handle is the content view; its window is the sheet's parent.
    let view: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>())? };
    view.window()
}

/// Builds and configures a panel from `request`, then reads back what AppKit
/// holds: title, starting directory and allowed extensions.
///
/// Verification entry. A file dialog is modal and cannot be driven from a
/// test, so this checks the half that is ours — that the request reached
/// AppKit intact — without presenting anything.
pub(crate) fn describe_configured_panel(
    request: &nana_ui_core::FileDialogRequest,
) -> Option<(Option<String>, Option<String>, Vec<String>)> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSOpenPanel, NSSavePanel};

    let mtm = MainThreadMarker::new()?;
    let panel: objc2::rc::Retained<NSSavePanel> = if request.kind.is_save() {
        NSSavePanel::savePanel(mtm)
    } else {
        objc2::rc::Retained::into_super(NSOpenPanel::openPanel(mtm))
    };
    configure_panel(&panel, request);

    let title = Some(panel.title().to_string()).filter(|title| !title.is_empty());
    let directory = panel
        .directoryURL()
        .and_then(|url| url.path())
        .map(|path| path.to_string());
    #[allow(deprecated)]
    let extensions = panel.allowedFileTypes().map_or_else(Vec::new, |types| {
        types.iter().map(|value| value.to_string()).collect()
    });
    Some((title, directory, extensions))
}
