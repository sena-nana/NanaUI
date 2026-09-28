//! macOS: the window's own shadow, or a child-window companion.
//!
//! AppKit derives a window's shadow from its alpha, so on a transparent
//! window it outlines whatever the client paints. For a transparent window
//! the companion is a borderless child window ordered below the primary: the
//! window server moves child windows with their parent, so a move costs
//! nothing here. It holds one layer whose `shadowPath` is the body's rounded
//! rectangle, masked (even-odd) to outside that rectangle so a translucent
//! card never shows its own shadow through itself. The render server draws
//! the shadow from the path; nothing is rasterized by the app.

use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGMutablePath, CGPath};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATransaction, kCAFillRuleEvenOdd};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{ShadowApplied, ShadowShape, ShadowStyle, ShadowWork};

fn ns_window<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Retained<NSWindow>> {
    MainThreadMarker::new()?;
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the AppKit handle's ns_view is a live NSView owned by the window.
    let view: Retained<NSView> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>())? };
    view.window()
}

pub(super) fn set_native<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> ShadowApplied {
    let Some(window) = ns_window(window) else {
        return ShadowApplied::Failed;
    };
    window.setHasShadow(enabled);
    match (enabled, window.hasShadow()) {
        (true, true) => ShadowApplied::Native,
        (false, false) => ShadowApplied::Disabled,
        _ => ShadowApplied::Failed,
    }
}

pub(super) struct Companion {
    parent: Retained<NSWindow>,
    window: Retained<NSWindow>,
    shadow: Retained<CALayer>,
    mask: Retained<CAShapeLayer>,
    /// The frame and paths last written, so an unchanged update writes
    /// nothing.
    last: Option<(NSRect, ShadowStyle, ShadowShape, bool)>,
}

impl Companion {
    /// `None`: never unsupported here. `Some(Err)`: AppKit refused.
    pub(super) fn create<W: HasWindowHandle + ?Sized>(
        window: &W,
        _style: ShadowStyle,
        _shape: ShadowShape,
        _work: &mut ShadowWork,
    ) -> Option<Result<Self, ()>> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Some(Err(()));
        };
        let Some(parent) = ns_window(window) else {
            return Some(Err(()));
        };
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        // SAFETY: plain borderless window creation on the main thread.
        let companion = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        companion.setOpaque(false);
        companion.setHasShadow(false);
        companion.setBackgroundColor(Some(&NSColor::clearColor()));
        companion.setIgnoresMouseEvents(true);
        companion.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        // SAFETY: the companion is owned by `Retained` and closed explicitly.
        unsafe { companion.setReleasedWhenClosed(false) };
        let content = NSView::initWithFrame(NSView::alloc(mtm), frame);
        content.setWantsLayer(true);
        companion.setContentView(Some(&content));
        let Some(root) = content.layer() else {
            return Some(Err(()));
        };
        let shadow = CALayer::new();
        let mask = CAShapeLayer::new();
        mask.setFillRule(unsafe { kCAFillRuleEvenOdd });
        // SAFETY: a shape layer is a CALayer.
        unsafe { shadow.setMask(Some(&mask)) };
        root.addSublayer(&shadow);
        // SAFETY: both windows are live and owned on the main thread.
        unsafe { parent.addChildWindow_ordered(&companion, NSWindowOrderingMode::Below) };
        Some(Ok(Self {
            parent,
            window: companion,
            shadow,
            mask,
            last: None,
        }))
    }

    /// Put the companion around `shape` with `style`. Returns false if the
    /// parent is gone.
    pub(super) fn update<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        style: ShadowStyle,
        shape: ShadowShape,
        visible: bool,
        work: &mut ShadowWork,
    ) -> bool {
        let Some(content) = self.parent.contentView() else {
            return false;
        };
        // The client area in screen points (bottom-up), then the body in it.
        let client = self.parent.convertRectToScreen(content.frame());
        let [x, y, width, height] = shape.rect.map(f64::from);
        let body = NSRect::new(
            NSPoint::new(
                client.origin.x + x,
                client.origin.y + client.size.height - y - height,
            ),
            NSSize::new(width, height),
        );
        let margin = style.margin();
        let frame = NSRect::new(
            NSPoint::new(body.origin.x - margin, body.origin.y - margin),
            NSSize::new(width + 2.0 * margin, height + 2.0 * margin),
        );
        if self.last == Some((frame, style, shape, visible)) {
            return true;
        }
        let resized = self.last.is_none_or(|(old, ..)| {
            old.size.width != frame.size.width || old.size.height != frame.size.height
        });
        let restyled = self.last.is_none_or(|(_, old_style, old_shape, _)| {
            old_style != style || old_shape.radii != shape.radii
        });
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.window.setFrame_display(frame, false);
        if resized || restyled {
            let bounds = CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(frame.size.width, frame.size.height),
            );
            let card = CGRect::new(CGPoint::new(margin, margin), CGSize::new(width, height));
            let radius = shape.radius() as CGFloat;
            // SAFETY: null transforms; the paths are created and consumed here.
            let body_path =
                unsafe { CGPath::with_rounded_rect(card, radius, radius, std::ptr::null()) };
            let outside = CGMutablePath::new();
            unsafe {
                CGMutablePath::add_rect(Some(&outside), std::ptr::null(), bounds);
                CGMutablePath::add_rounded_rect(
                    Some(&outside),
                    std::ptr::null(),
                    card,
                    radius,
                    radius,
                );
            }
            self.shadow.setFrame(bounds);
            self.mask.setFrame(bounds);
            self.shadow.setShadowPath(Some(&body_path));
            self.mask.setPath(Some(&outside));
            let [red, green, blue, alpha] = style.color.map(|c| CGFloat::from(c.clamp(0.0, 1.0)));
            let color = CGColor::new_srgb(red, green, blue, 1.0);
            self.shadow.setShadowColor(Some(&color));
            self.shadow.setShadowOpacity(alpha as f32);
            // A CALayer shadow radius is a Gaussian blur radius; CSS-style
            // blur is twice the standard deviation, which it approximates.
            self.shadow
                .setShadowRadius(CGFloat::from(style.blur.max(0.0)) / 2.0);
            // Layers are bottom-up: a shadow offset downward is negative y.
            self.shadow.setShadowOffset(CGSize::new(
                CGFloat::from(style.offset[0]),
                -CGFloat::from(style.offset[1]),
            ));
            work.effect_updates += 1;
            if resized {
                work.companion_resizes += 1;
            }
        } else {
            work.companion_moves += 1;
        }
        CATransaction::commit();
        let was_visible = self.last.is_some_and(|(.., shown)| shown);
        if visible && !was_visible {
            // SAFETY: both windows are live and owned on the main thread.
            unsafe {
                self.parent
                    .addChildWindow_ordered(&self.window, NSWindowOrderingMode::Below)
            };
        } else if !visible && was_visible {
            self.window.orderOut(None);
        }
        self.last = Some((frame, style, shape, visible));
        true
    }

    /// Paths are resolution independent; the window server re-renders them.
    pub(super) fn rescale<W: HasWindowHandle + ?Sized>(
        &mut self,
        _window: &W,
        _work: &mut ShadowWork,
    ) {
    }

    pub(super) fn add_work(&self, _work: &mut ShadowWork) {}
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.parent.removeChildWindow(&self.window);
        self.window.orderOut(None);
        self.window.close();
    }
}
