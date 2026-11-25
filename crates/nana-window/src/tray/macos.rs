use std::cell::RefCell;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSColor, NSImage, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::{NSData, NSSize, NSString};

use super::{Tray, TrayAttention, select_tray_icon_bytes};

/// The menu bar's icon height in points.
const ICON_POINTS: f64 = 18.0;

struct MacTrayState {
    item: Retained<NSStatusItem>,
    attention: TrayAttention,
}

thread_local! {
    static ITEM: RefCell<Option<MacTrayState>> = const { RefCell::new(None) };
}

pub(super) fn install(tray: &Tray) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("macOS tray must be installed on the main thread".into());
    };
    let item = ITEM.with(|slot| {
        let mut slot = slot.borrow_mut();
        let entry = slot.get_or_insert_with(|| MacTrayState {
            item: NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength),
            attention: tray.attention,
        });
        entry.attention = tray.attention;
        entry.item.clone()
    });
    if let Some(button) = item.button(mtm) {
        let light_menu_bar = !menu_bar_is_dark(&button);
        let has_explicit_variant = if light_menu_bar {
            tray.monochrome_dark_icon_png
                .as_ref()
                .is_some_and(|bytes| !bytes.is_empty())
        } else {
            tray.monochrome_light_icon_png
                .as_ref()
                .is_some_and(|bytes| !bytes.is_empty())
        };
        let bytes = if has_explicit_variant {
            select_tray_icon_bytes(tray, light_menu_bar)
        } else {
            std::borrow::Cow::Borrowed(tray.icon_png.as_slice())
        };
        let data = NSData::with_bytes(bytes.as_ref());
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            return Err("invalid tray PNG icon bytes".into());
        };
        image.setSize(NSSize::new(ICON_POINTS, ICON_POINTS));
        image.setTemplate(tray.template_icon && !has_explicit_variant);
        button.setImage(Some(&image));
        button.setToolTip(Some(&NSString::from_str(&tray.tooltip)));
        apply_button_attention(&button, tray.attention);
    }
    let menu = crate::platform::build_menu(
        mtm,
        &crate::Menu {
            title: String::new(),
            entries: tray.menu.clone(),
        },
    );
    item.setMenu(Some(&menu));
    Ok(())
}

pub(super) fn set_attention(attention: TrayAttention) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let item = ITEM.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().map(|state| {
            state.attention = attention;
            state.item.clone()
        })
    });
    let Some(item) = item else {
        return false;
    };
    if let Some(button) = item.button(mtm) {
        apply_button_attention(&button, attention);
    }
    true
}

fn apply_button_attention(button: &objc2_app_kit::NSStatusBarButton, attention: TrayAttention) {
    // SAFETY: `NSStatusBarButton` inherits `NSButton`/`NSControl` selectors
    // `setAppearsDisabled:` and `setContentTintColor:`.
    unsafe {
        let disabled = matches!(attention, TrayAttention::Passive);
        let _: () = msg_send![button, setAppearsDisabled: disabled];
        let tint: Option<&NSColor> = match attention {
            TrayAttention::NeedsAttention => Some(&NSColor::systemRedColor()),
            TrayAttention::Active | TrayAttention::Passive => None,
        };
        let _: () = msg_send![button, setContentTintColor: tint];
    }
}

fn menu_bar_is_dark(button: &objc2_app_kit::NSStatusBarButton) -> bool {
    // SAFETY: `effectiveAppearance` returns an `NSAppearance`; `name` returns an
    // `NSAppearanceName` string containing `"Dark"` for dark menu bars.
    unsafe {
        let appearance: Option<Retained<AnyObject>> = msg_send![button, effectiveAppearance];
        let Some(appearance) = appearance else {
            return false;
        };
        let name: Option<Retained<NSString>> = msg_send![&*appearance, name];
        name.is_some_and(|n| n.to_string().contains("Dark"))
    }
}

pub(super) fn installed() -> Option<(String, Vec<String>)> {
    let mtm = MainThreadMarker::new()?;
    let item = ITEM.with(|slot| slot.borrow().as_ref().map(|state| state.item.clone()))?;
    let tooltip = item
        .button(mtm)
        .and_then(|button| button.toolTip())
        .map(|tip| tip.to_string())
        .unwrap_or_default();
    let menu = item.menu(mtm)?;
    let mut entries = Vec::new();
    for index in 0..menu.numberOfItems() {
        if let Some(entry) = menu.itemAtIndex(index) {
            entries.push(if entry.isSeparatorItem() {
                "-".to_owned()
            } else {
                entry.title().to_string()
            });
        }
    }
    Some((tooltip, entries))
}

pub(super) fn remove() {
    if let Some(state) = ITEM.with(|slot| slot.borrow_mut().take()) {
        NSStatusBar::systemStatusBar().removeStatusItem(&state.item);
    }
}
