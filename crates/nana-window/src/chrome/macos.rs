use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn native_live_resize_active<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    appkit_window(window).is_some_and(|window| window.inLiveResize())
}

/// Reads AppKit's zoom state without changing the window's style or frame.
/// Borderless and fixed-size windows cannot be zoomed. Unlike winit's
/// `is_maximized`, this never temporarily adds title-bar or resize styles,
/// so it is safe while handling native move and resize notifications.
/// Returns `None` when the handle does not refer to an AppKit window.
pub fn native_window_maximized<W: HasWindowHandle + ?Sized>(window: &W) -> Option<bool> {
    use objc2_app_kit::NSWindowStyleMask;

    let window = appkit_window(window)?;
    let zoomable = NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable;
    Some(window.styleMask().contains(zoomable) && window.isZoomed())
}

pub(super) fn set_metal_present_transaction<W: HasWindowHandle + ?Sized>(
    window: &W,
    enabled: bool,
) -> bool {
    use objc2_app_kit::NSView;
    use objc2_quartz_core::CAMetalLayer;

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: the AppKit handle's ns_view is a live NSView owned by this window.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(layer) = view.layer() else {
        return false;
    };
    // raw-window-metal, which wgpu creates its surface through, leaves the
    // view's own layer in place and adds the CAMetalLayer as a sublayer of it;
    // only a view that already had a Metal layer is used as is.
    let metal = match layer.downcast::<CAMetalLayer>() {
        Ok(metal) => metal,
        Err(layer) => {
            // SAFETY: `sublayers` is read on the main thread that owns the view.
            let Some(sublayers) = (unsafe { layer.sublayers() }) else {
                return false;
            };
            let Some(metal) = sublayers
                .iter()
                .find_map(|sublayer| sublayer.downcast::<CAMetalLayer>().ok())
            else {
                return false;
            };
            metal
        }
    };
    metal.setPresentsWithTransaction(enabled);
    true
}

/// Moves the traffic-light cluster so its center lands on `center_x` (kept
/// when `None`) and `center_y`, measured from the window's top-left corner.
pub(super) fn move_traffic_lights<W: HasWindowHandle + ?Sized>(
    window: &W,
    center_x: Option<f64>,
    center_y: f64,
) -> bool {
    use objc2_app_kit::NSWindowButton;
    use objc2_foundation::NSPoint;

    let Some(window) = appkit_window(window) else {
        return false;
    };
    // All three buttons must exist and share one container view, so the
    // buttons keep their spacing while the titlebar around them moves.
    let (Some(close), Some(miniaturize), Some(zoom)) = (
        window.standardWindowButton(NSWindowButton::CloseButton),
        window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return false;
    };
    // SAFETY: plain property reads on live views owned by this window.
    let container = unsafe { close.superview() };
    // The titlebar view is what moves, not the button group inside it: a
    // group shifted past its parent's bounds keeps drawing but stops being
    // hit, and the buttons would no longer answer clicks.
    // SAFETY: plain property reads on live views owned by this window.
    let titlebar = container
        .as_ref()
        .and_then(|view| unsafe { view.superview() });
    // SAFETY: plain property reads on live views owned by this window.
    let titlebar_parent = titlebar
        .as_ref()
        .and_then(|view| unsafe { view.superview() });
    let (Some(container), Some(titlebar), Some(titlebar_parent)) =
        (container, titlebar, titlebar_parent)
    else {
        return false;
    };
    // SAFETY: plain property reads on live views owned by this window.
    if unsafe { miniaturize.superview() }.as_ref() != Some(&container)
        || unsafe { zoom.superview() }.as_ref() != Some(&container)
    {
        return false;
    }

    // Buttons that are fading or hidden are mid-flight: AppKit is moving
    // them itself, and a position read now sends the cluster somewhere else.
    // Leaving them be is safe — a reveal restores them at full alpha, and
    // the window places them again then.
    if [&close, &miniaturize, &zoom]
        .into_iter()
        .any(|button| button.isHidden() || button.alphaValue() < 1.0)
    {
        return true;
    }

    // Current cluster center in window space. Window base coordinates grow
    // upward from the bottom-left corner.
    let close_in_window = close.convertRect_toView(close.bounds(), None);
    let zoom_in_window = zoom.convertRect_toView(zoom.bounds(), None);
    let width = zoom_in_window.origin.x + zoom_in_window.size.width - close_in_window.origin.x;
    if width <= 0.0 || close_in_window.size.height <= 0.0 {
        return false;
    }
    let current_x = close_in_window.origin.x + width / 2.0;
    let current_from_top =
        window.frame().size.height - (close_in_window.origin.y + close_in_window.size.height / 2.0);
    let delta_x = center_x.map_or(0.0, |x| x - current_x);
    let delta_y = center_y - current_from_top;
    if delta_x.abs() < 0.5 && delta_y.abs() < 0.5 {
        return true;
    }

    // Learn how the titlebar's parent maps its axes onto window space
    // (titlebar views are flipped, the window is not) instead of assuming.
    let origin = titlebar_parent.convertPoint_toView(NSPoint::new(0.0, 0.0), None);
    let unit_x = titlebar_parent.convertPoint_toView(NSPoint::new(1.0, 0.0), None);
    let unit_y = titlebar_parent.convertPoint_toView(NSPoint::new(0.0, 1.0), None);
    let x_per_unit = unit_x.x - origin.x;
    let y_up_per_unit = unit_y.y - origin.y;
    if x_per_unit == 0.0 || y_up_per_unit == 0.0 {
        return false;
    }
    let mut frame = titlebar.frame();
    frame.origin.x += delta_x / x_per_unit;
    frame.origin.y -= delta_y / y_up_per_unit;
    titlebar.setFrameOrigin(frame.origin);
    true
}

pub(super) fn fade_traffic_lights<W: HasWindowHandle + ?Sized>(
    window: &W,
    visible: bool,
    duration: std::time::Duration,
) -> bool {
    use block2::RcBlock;
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext, NSWindowButton};

    let Some(window) = appkit_window(window) else {
        return false;
    };
    let buttons: Vec<_> = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .filter_map(|kind| window.standardWindowButton(kind))
    .collect();
    if buttons.is_empty() {
        return false;
    }
    let alpha = if visible { 1.0 } else { 0.0 };
    if duration.is_zero() {
        for button in &buttons {
            button.setAlphaValue(alpha);
            button.setHidden(!visible);
        }
        return true;
    }
    if visible {
        for button in &buttons {
            button.setHidden(false);
        }
    }
    let seconds = duration.as_secs_f64();
    let changes = RcBlock::new(|context: std::ptr::NonNull<NSAnimationContext>| {
        // SAFETY: AppKit passes the live context of this animation group.
        unsafe { context.as_ref() }.setDuration(seconds);
        for button in &buttons {
            button.animator().setAlphaValue(alpha);
        }
    });
    // The model alpha is the latest target, so a reveal issued during this
    // fade keeps the buttons.
    let concealed = buttons.clone();
    let completion = RcBlock::new(move || {
        for button in &concealed {
            if button.alphaValue() <= 0.0 {
                button.setHidden(true);
            }
        }
    });
    NSAnimationContext::runAnimationGroup_completionHandler(
        &changes,
        (!visible).then_some(&*completion),
    );
    true
}

pub(super) fn set_drag_enabled<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    let Some(window) = appkit_window(window) else {
        return false;
    };
    window.setMovable(enabled);
    true
}

pub(super) fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventType};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = appkit_movable_window(window) else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return false;
    };
    // AppKit silently ignores a drag started from any other event, e.g. when
    // the press was queued and handled outside its own mouseDown dispatch.
    let kind = event.r#type();
    if kind != NSEventType::LeftMouseDown && kind != NSEventType::LeftMouseDragged {
        return false;
    }
    // `currentEvent` can still be the press that triggered this command after
    // a short click has already released the mouse. AppKit then reports the
    // synthetic completion without a matching move begin. Only enter its
    // native move loop while the primary button is still physically held.
    if NSEvent::pressedMouseButtons() & 1 == 0 {
        return false;
    }

    window.setMovable(true);
    window.performWindowDragWithEvent(&event);
    window.setMovable(false);
    true
}

pub(crate) fn appkit_window<W: HasWindowHandle + ?Sized>(
    window: &W,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;

    let Ok(handle) = window.window_handle() else {
        return None;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

pub(crate) fn appkit_movable_window<W: HasWindowHandle + ?Sized>(
    window: &W,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    let window = appkit_window(window)?;
    (!window
        .styleMask()
        .contains(objc2_app_kit::NSWindowStyleMask::FullScreen))
    .then_some(window)
}
