//! macOS Early Splash: two `CALayer`s on the content view's root layer.
//!
//! wgpu (through raw-window-metal) adds its `CAMetalLayer` as a *sublayer* of
//! that root layer, after this splash exists, so the container carries a
//! z-position no Metal sublayer has: whatever order the siblings end up in,
//! the splash stays on top until it is removed.

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSImage, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::{NSData, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    CAAutoresizingMask, CABasicAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction,
    CATransaction, kCAGravityResizeAspect, kCAMediaTimingFunctionEaseInEaseOut,
    kCAMediaTimingFunctionLinear,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{LogoInfo, SplashAnimation, SplashFailure, SplashLogoError, SplashWork};
use crate::material::FallbackColor;

/// Above any sibling the view's root layer will be given.
const SPLASH_Z: f64 = 1.0e6;

pub(super) struct Request<'a> {
    pub(super) png: &'a [u8],
    #[allow(dead_code)]
    pub(super) info: LogoInfo,
    pub(super) logo_size: (f64, f64),
    pub(super) background: Option<FallbackColor>,
    pub(super) animation: SplashAnimation,
}

pub(super) struct Splash {
    window: Retained<NSWindow>,
    owner: Retained<NSWindow>,
    container: Retained<CALayer>,
    logo: Retained<CALayer>,
    /// Kept to re-render the layer contents when the backing scale changes.
    image: Retained<NSImage>,
}

impl Splash {
    pub(super) fn show<W: HasWindowHandle + ?Sized>(
        window: &W,
        request: &Request<'_>,
        work: &mut SplashWork,
    ) -> Result<(Self, bool), SplashFailure> {
        let native = |reason: &str| SplashFailure::Native(reason.to_owned());
        let mtm = MainThreadMarker::new().ok_or_else(|| native("not on the main thread"))?;
        let handle = window
            .window_handle()
            .map_err(|error| SplashFailure::Native(error.to_string()))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(native("not an AppKit window"));
        };
        // SAFETY: the AppKit handle's ns_view is a live NSView owned by this
        // window, and `mtm` witnesses the main thread.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let owner = view
            .window()
            .ok_or_else(|| native("owner window unavailable"))?;
        let scale = owner.backingScaleFactor();
        let size = request.logo_size;
        let owner_frame = owner.frame();
        let frame = NSRect::new(
            NSPoint::new(
                owner_frame.origin.x + (owner_frame.size.width - size.0) / 2.0,
                owner_frame.origin.y + (owner_frame.size.height - size.1) / 2.0,
            ),
            NSSize::new(size.0, size.1),
        );
        let splash_window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(),
                frame,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        splash_window.setOpaque(false);
        splash_window.setHasShadow(false);
        splash_window.setBackgroundColor(Some(&NSColor::clearColor()));
        splash_window.setIgnoresMouseEvents(false);
        splash_window.setLevel(NSFloatingWindowLevel);
        splash_window.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        unsafe { splash_window.setReleasedWhenClosed(false) };
        let content = unsafe {
            NSView::initWithFrame(
                NSView::alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.0, size.1)),
            )
        };
        content.setWantsLayer(true);
        splash_window.setContentView(Some(&content));
        let root = content
            .layer()
            .ok_or_else(|| native("splash content has no layer"))?;
        let _ = mtm;

        // One decode, by ImageIO; the header was checked against the limits
        // before this.
        let data = NSData::with_bytes(request.png);
        work.logo_decodes += 1;
        let image = NSImage::initWithData(NSImage::alloc(), &data).ok_or_else(|| {
            SplashFailure::Logo(SplashLogoError::Decode("ImageIO rejected the PNG".into()))
        })?;

        CATransaction::begin();
        // A new sublayer would otherwise fade in with Core Animation's implicit
        // action; the splash is either there or it is not.
        CATransaction::setDisableActions(true);
        let bounds = root.bounds();
        let container = CALayer::new();
        container.setFrame(bounds);
        container.setAutoresizingMask(
            CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable,
        );
        container.setZPosition(SPLASH_Z);
        container.setContentsScale(scale);
        if let Some(color) = request.background {
            let color = CGColor::new_srgb(
                f64::from(color.red) / 255.0,
                f64::from(color.green) / 255.0,
                f64::from(color.blue) / 255.0,
                f64::from(color.alpha) / 255.0,
            );
            container.setBackgroundColor(Some(&color));
        }

        let logo = CALayer::new();
        let (width, height) = request.logo_size;
        logo.setBounds(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width, height),
        ));
        logo.setPosition(CGPoint::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        ));
        // Flexible margins on every side keep the logo centred as the window
        // resizes, with no callback.
        logo.setAutoresizingMask(
            CAAutoresizingMask::LayerMinXMargin
                | CAAutoresizingMask::LayerMaxXMargin
                | CAAutoresizingMask::LayerMinYMargin
                | CAAutoresizingMask::LayerMaxYMargin,
        );
        // SAFETY: kCAGravityResizeAspect is a constant owned by Core Animation.
        logo.setContentsGravity(unsafe { kCAGravityResizeAspect });
        set_logo_contents(&logo, &image, scale);
        work.logo_uploads += 1;

        let animated = match animation(request.animation) {
            Some(animation) => {
                logo.addAnimation_forKey(&animation, Some(&NSString::from_str("nana.splash")));
                work.animation_submissions += 1;
                true
            }
            None => false,
        };
        container.addSublayer(&logo);
        root.addSublayer(&container);
        CATransaction::commit();
        splash_window.orderFrontRegardless();
        work.commits += 1;
        Ok((
            Self {
                window: splash_window,
                owner,
                container,
                logo,
                image,
            },
            animated,
        ))
    }

    pub(super) const fn live_resources(&self) -> usize {
        2
    }

    pub(super) fn set_scale_factor(&self, scale: f64, work: &mut SplashWork) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.container.setContentsScale(scale);
        set_logo_contents(&self.logo, &self.image, scale);
        CATransaction::commit();
        let owner_frame = self.owner.frame();
        let size = self.window.frame().size;
        self.window.setFrame_display(
            NSRect::new(
                NSPoint::new(
                    owner_frame.origin.x + (owner_frame.size.width - size.width) / 2.0,
                    owner_frame.origin.y + (owner_frame.size.height - size.height) / 2.0,
                ),
                size,
            ),
            false,
        );
        work.logo_uploads += 1;
        work.commits += 1;
    }

    /// Detaches both layers. Inside an event-loop turn this nests into the
    /// turn's implicit transaction, which is the one a transaction-mode
    /// drawable presented earlier in the same turn is published with.
    pub(super) fn remove(self, work: &mut SplashWork, _handoff: bool) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.logo.removeAllAnimations();
        self.container.removeFromSuperlayer();
        CATransaction::commit();
        self.window.orderOut(None);
        work.commits += 1;
    }
}

/// Renders `image` as `layer`'s contents for a backing `scale`.
fn set_logo_contents(layer: &CALayer, image: &NSImage, scale: f64) {
    let contents_scale = image.recommendedLayerContentsScale(scale);
    let contents = image.layerContentsForContentsScale(contents_scale);
    // SAFETY: `layerContentsForContentsScale:` returns an object CALayer
    // accepts as contents.
    unsafe { layer.setContents(Some(&contents)) };
    layer.setContentsScale(contents_scale);
}

/// The preset as one `CABasicAnimation`, or `None` for a still logo.
fn animation(preset: SplashAnimation) -> Option<Retained<CABasicAnimation>> {
    let (key_path, from, to, duration, repeat, autoreverses, timing) = match preset {
        SplashAnimation::None => return None,
        SplashAnimation::FadeIn => ("opacity", 0.0, 1.0, 0.35, 0.0, false, false),
        SplashAnimation::Pulse => ("opacity", 1.0, 0.45, 0.9, f32::INFINITY, true, false),
        SplashAnimation::Rotate => (
            "transform.rotation.z",
            0.0,
            -std::f64::consts::TAU,
            1.2,
            f32::INFINITY,
            false,
            true,
        ),
    };
    let animation = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str(key_path)));
    let from = NSNumber::numberWithDouble(from);
    let to = NSNumber::numberWithDouble(to);
    // SAFETY: NSNumber is a valid from/to value for scalar key paths.
    unsafe {
        animation.setFromValue(Some(&from));
        animation.setToValue(Some(&to));
    }
    animation.setDuration(duration);
    animation.setRepeatCount(repeat);
    animation.setAutoreverses(autoreverses);
    // SAFETY: the timing-function names are constants owned by Core Animation.
    let name = unsafe {
        if timing {
            kCAMediaTimingFunctionLinear
        } else {
            kCAMediaTimingFunctionEaseInEaseOut
        }
    };
    animation.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(name)));
    Some(animation)
}
