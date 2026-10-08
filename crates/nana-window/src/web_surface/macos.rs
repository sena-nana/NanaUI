//! WKWebView in a borderless window parked far outside every screen.
//!
//! WebKit stops rendering a page whose window is occluded, so the window stays
//! ordered in and the view opts out of occlusion detection; captures use
//! `takeSnapshotWithConfiguration`, and the CGImage is converted to RGBA on a
//! worker thread.
//!
//! Every snapshot costs WebKit a full render of the page, so the page renders
//! at the requested device scale rather than the screen's (no 2× image to scale
//! back down), unchanged captures are dropped, and a page that stopped
//! changing is captured only a few times a second until it changes again.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;

use block2::RcBlock;
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use super::{
    SurfaceEvents, WebFrame, WebFrameSink, WebSurfaceCommand, WebSurfaceCompletion, WebSurfaceDesc,
    WebSurfaceEvent, WebSurfaceWake, capture_interval,
};
use crate::BrowserPolicy;
use crate::browser::platform::{
    NOT_ALLOWED, action_url, error_description, follow_new_window, is_cancelled, load_url,
    read_state,
};

#[repr(C)]
struct CGImage {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGImage {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextRelease(context: *mut c_void);
    fn CGContextSetInterpolationQuality(context: *mut c_void, quality: i32);
    fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *mut CGImage);
    fn CGImageRetain(image: *mut CGImage) -> *mut CGImage;
    fn CGImageGetWidth(image: *mut CGImage) -> usize;
    fn CGImageGetHeight(image: *mut CGImage) -> usize;
    fn CGImageRelease(image: *mut CGImage);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSRunLoopCommonModes: &'static NSString;
}

/// `kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big`: RGBA in memory.
const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);
const CG_INTERPOLATION_NONE: i32 = 1;
const CG_INTERPOLATION_HIGH: i32 = 3;
/// Identical captures in a row after which the page counts as still.
const STILL_AFTER: u32 = 10;
/// Capture rate of a still page; the first change restores the full rate.
const STILL_FPS: u32 = 4;
/// Far outside any arrangement of displays, so the parked window is never seen.
const PARKED_ORIGIN: f64 = -32_000.0;

/// A retained CGImage moved to the conversion thread. CGImage is immutable and
/// thread-safe.
struct CapturedImage(*mut CGImage);

unsafe impl Send for CapturedImage {}

impl Drop for CapturedImage {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.0) };
    }
}

struct ConvertJob {
    image: CapturedImage,
    size: [u32; 2],
}

fn spawn_converter(
    sink: WebFrameSink,
    unchanged: Arc<AtomicU32>,
) -> Result<SyncSender<ConvertJob>, String> {
    // One job in flight: a capture that finds the worker busy is dropped, so a
    // slow sink lowers the rate instead of growing a queue.
    let (sender, receiver) = std::sync::mpsc::sync_channel::<ConvertJob>(1);
    std::thread::Builder::new()
        .name("nana-web-surface".into())
        .spawn(move || {
            let mut last: Option<Arc<[u8]>> = None;
            let mut sequence = 0;
            for job in receiver {
                let Some(rgba) = convert(&job.image, job.size) else {
                    continue;
                };
                // A capture equal to the last delivered frame is not a frame.
                if last.as_deref() == Some(&*rgba) {
                    unchanged.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                unchanged.store(0, Ordering::Relaxed);
                last = Some(rgba.clone());
                sequence += 1;
                sink(WebFrame {
                    width: job.size[0],
                    height: job.size[1],
                    rgba,
                    sequence,
                });
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(sender)
}

fn convert(image: &CapturedImage, [width, height]: [u32; 2]) -> Option<Arc<[u8]>> {
    let (width, height) = (width as usize, height as usize);
    // Same size: a straight copy; otherwise (an engine without a device scale
    // override) a resample.
    let exact = unsafe { [CGImageGetWidth(image.0), CGImageGetHeight(image.0)] } == [width, height];
    let mut pixels = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
    unsafe {
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if space.is_null() {
            return None;
        }
        let context = CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space,
            RGBA_PREMULTIPLIED,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        CGContextSetInterpolationQuality(
            context,
            if exact {
                CG_INTERPOLATION_NONE
            } else {
                CG_INTERPOLATION_HIGH
            },
        );
        CGContextDrawImage(
            context,
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width as f64, height as f64),
            ),
            image.0,
        );
        CGContextRelease(context);
    }
    Some(pixels.into())
}

struct Shared {
    events: RefCell<SurfaceEvents>,
    revision: Cell<u64>,
    error: RefCell<Option<String>>,
    policy: BrowserPolicy,
    wake: WebSurfaceWake,
    window: RefCell<Option<Retained<NSWindow>>>,
    view: RefCell<Option<Retained<AnyObject>>>,
    desc: Cell<WebSurfaceDesc>,
    interactive: Cell<bool>,
    capturing: Cell<bool>,
    /// Capture ticks so far, for the still-page rate.
    ticks: Cell<u64>,
    /// Identical captures in a row, counted by the converter.
    unchanged: Arc<AtomicU32>,
    converter: SyncSender<ConvertJob>,
}

impl Shared {
    fn publish_state(&self) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        let error = self.error.borrow().clone();
        self.events.borrow_mut().publish(
            self.revision.get(),
            WebSurfaceEvent::State(read_state(&view, error)),
        );
        (self.wake)();
    }

    fn capture(self: &Rc<Self>) {
        if self.capturing.get() {
            return;
        }
        let ticks = self.ticks.get() + 1;
        self.ticks.set(ticks);
        if self.unchanged.load(Ordering::Relaxed) >= STILL_AFTER {
            let every = u64::from((self.desc.get().max_fps / STILL_FPS).max(1));
            if !ticks.is_multiple_of(every) {
                return;
            }
        }
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        self.capturing.set(true);
        let shared = Rc::downgrade(self);
        let completion = RcBlock::new(move |image: *mut AnyObject, _error: *mut AnyObject| {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            shared.capturing.set(false);
            let Some(image) = (unsafe { image.as_ref() }) else {
                return;
            };
            let cg: *mut CGImage = unsafe {
                msg_send![image, CGImageForProposedRect: std::ptr::null_mut::<NSRect>(), context: std::ptr::null::<AnyObject>(), hints: std::ptr::null::<AnyObject>()]
            };
            if cg.is_null() {
                return;
            }
            let job = ConvertJob {
                image: CapturedImage(unsafe { CGImageRetain(cg) }),
                size: shared.desc.get().frame_size(),
            };
            // A busy converter drops this capture; the next tick takes another.
            let _ = shared.converter.try_send(job);
        });
        unsafe {
            let configuration: Retained<AnyObject> =
                msg_send![class!(WKSnapshotConfiguration), new];
            // Waiting for a screen update would stall: the window is never on a screen.
            let _: () = msg_send![&configuration, setAfterScreenUpdates: false];
            let _: () = msg_send![&view, takeSnapshotWithConfiguration: &*configuration, completionHandler: &*completion];
        }
    }

    fn park(&self) {
        let Some(window) = self.window.borrow().clone() else {
            return;
        };
        let [width, height] = self.desc.get().size.map(f64::from);
        window.setStyleMask(NSWindowStyleMask::Borderless);
        window.setIgnoresMouseEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.setFrame_display(
            NSRect::new(
                NSPoint::new(PARKED_ORIGIN, PARKED_ORIGIN),
                NSSize::new(width, height),
            ),
            false,
        );
        // An ordered-out window counts as hidden and the page would stop
        // rendering; ordering in a window this far away shows nothing.
        window.orderFrontRegardless();
    }

    /// Return the page from its interaction window to its parking place.
    fn hide(&self) {
        if self.interactive.replace(false) {
            self.park();
            self.events
                .borrow_mut()
                .publish(self.revision.get(), WebSurfaceEvent::WindowClosed);
            (self.wake)();
        }
    }

    fn show(&self, title: &str, mtm: MainThreadMarker) {
        let Some(window) = self.window.borrow().clone() else {
            return;
        };
        let [width, height] = self.desc.get().size.map(f64::from);
        window.setStyleMask(
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable,
        );
        window.setTitle(&NSString::from_str(title));
        window.setIgnoresMouseEvents(false);
        window.setCollectionBehavior(NSWindowCollectionBehavior::Default);
        window.setContentSize(NSSize::new(width, height));
        window.center();
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Shared>]
    struct SurfaceDelegate;

    unsafe impl NSObjectProtocol for SurfaceDelegate {}

    unsafe impl NSWindowDelegate for SurfaceDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self, _sender: &NSWindow) -> bool {
            self.ivars().hide();
            false
        }
    }

    impl SurfaceDelegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observed(&self, _key: &NSString, _view: &AnyObject, _change: Option<&AnyObject>, _context: *mut c_void) {
            self.ivars().publish_state();
        }
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn new_window(&self, view: &AnyObject, _configuration: &AnyObject, action: &AnyObject, _features: &AnyObject) -> *mut AnyObject {
            follow_new_window(view, action, &self.ivars().policy);
            std::ptr::null_mut()
        }
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, _view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.ivars().error.borrow_mut().take();
            self.ivars().publish_state();
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, _view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.ivars().publish_state();
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_provisional(&self, _view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            self.fail(error);
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, _view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            self.fail(error);
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, view: &AnyObject) {
            *self.ivars().error.borrow_mut() = Some("网页进程已退出，正在重新加载".into());
            self.ivars().publish_state();
            let _: Option<Retained<AnyObject>> = unsafe { msg_send![view, reload] };
        }
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(&self, _view: &AnyObject, action: &AnyObject, completion: &block2::Block<dyn Fn(isize)>) {
            let allowed = self.ivars().policy.allows(&action_url(action));
            completion.call((if allowed { 1 } else { 0 },));
            if !allowed {
                *self.ivars().error.borrow_mut() = Some(NOT_ALLOWED.into());
                self.ivars().publish_state();
            }
        }
    }
);

impl SurfaceDelegate {
    fn fail(&self, error: &AnyObject) {
        if !is_cancelled(error) {
            *self.ivars().error.borrow_mut() = Some(error_description(error));
            self.ivars().publish_state();
        }
    }
}

const OBSERVED_KEYS: [&str; 3] = ["URL", "title", "loading"];

pub(super) struct PlatformSurface {
    shared: Rc<Shared>,
    delegate: Retained<SurfaceDelegate>,
    timer: Option<Retained<AnyObject>>,
    mtm: MainThreadMarker,
}

impl PlatformSurface {
    pub(super) fn new(
        policy: BrowserPolicy,
        desc: WebSurfaceDesc,
        frames: WebFrameSink,
        wake: WebSurfaceWake,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("网页画面必须在主线程创建")?;
        let unchanged = Arc::new(AtomicU32::new(0));
        let shared = Rc::new(Shared {
            events: RefCell::default(),
            revision: Cell::new(0),
            error: RefCell::new(None),
            policy,
            wake,
            window: RefCell::new(None),
            view: RefCell::new(None),
            desc: Cell::new(desc),
            interactive: Cell::new(false),
            capturing: Cell::new(false),
            ticks: Cell::new(0),
            converter: spawn_converter(frames, unchanged.clone())?,
            unchanged,
        });
        let delegate: Retained<SurfaceDelegate> = unsafe {
            let allocated = SurfaceDelegate::alloc(mtm).set_ivars(shared.clone());
            msg_send![super(allocated), init]
        };
        let [width, height] = desc.size.map(f64::from);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setExcludedFromWindowsMenu(true);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let view: Retained<AnyObject> = unsafe {
            let configuration: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let preferences: Retained<AnyObject> = msg_send![&configuration, preferences];
            let responds: bool =
                msg_send![&preferences, respondsToSelector: sel!(setInactiveSchedulingPolicy:)];
            if responds {
                // WKInactiveSchedulingPolicyNone: timers and animations keep
                // running although the page is never on screen (macOS 14+).
                let _: () = msg_send![&preferences, setInactiveSchedulingPolicy: 2isize];
            }
            let allocated: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let view: Retained<AnyObject> =
                msg_send![allocated, initWithFrame: frame, configuration: &*configuration];
            let responds: bool =
                msg_send![&view, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)];
            if !responds {
                return Err("此系统版本无法在后台渲染网页".into());
            }
            let _: () = msg_send![&view, _setWindowOcclusionDetectionEnabled: false];
            let _: () = msg_send![&view, setNavigationDelegate: &*delegate];
            let _: () = msg_send![&view, setUIDelegate: &*delegate];
            for key in OBSERVED_KEYS {
                let key = NSString::from_str(key);
                let _: () = msg_send![&view, addObserver: &*delegate, forKeyPath: &*key, options: 0usize, context: std::ptr::null_mut::<c_void>()];
            }
            view
        };
        let native_view: &NSView = unsafe { &*((&*view as *const AnyObject).cast::<NSView>()) };
        window.setContentView(Some(native_view));
        *shared.window.borrow_mut() = Some(window);
        *shared.view.borrow_mut() = Some(view);
        let mut surface = Self {
            shared,
            delegate,
            timer: None,
            mtm,
        };
        surface.configure(desc);
        surface.shared.park();
        Ok(surface)
    }

    /// Apply size, scale, background and capture rate; the page stays loaded.
    pub(super) fn configure(&mut self, desc: WebSurfaceDesc) {
        let previous = self.shared.desc.replace(desc);
        if let Some(view) = self.shared.view.borrow().as_ref() {
            unsafe {
                // Render at the frame's own scale rather than the screen's, so
                // a snapshot needs no resampling and WebKit draws 1/4 of the
                // pixels on a 2× display.
                let responds: bool =
                    msg_send![view, respondsToSelector: sel!(_setOverrideDeviceScaleFactor:)];
                if responds {
                    let _: () =
                        msg_send![view, _setOverrideDeviceScaleFactor: f64::from(desc.scale)];
                }
                let draws: Retained<AnyObject> =
                    msg_send![class!(NSNumber), numberWithBool: !desc.transparent];
                let key = NSString::from_str("drawsBackground");
                let _: () = msg_send![view, setValue: &*draws, forKey: &*key];
            }
        }
        if previous.size != desc.size
            && let Some(window) = self.shared.window.borrow().as_ref()
        {
            window.setContentSize(NSSize::new(desc.size[0] as f64, desc.size[1] as f64));
        }
        if self.timer.is_some() && previous.max_fps == desc.max_fps {
            return;
        }
        self.stop_timer();
        let Some(interval) = capture_interval(desc.max_fps) else {
            return;
        };
        let shared: Weak<Shared> = Rc::downgrade(&self.shared);
        let tick = RcBlock::new(move |_timer: *mut AnyObject| {
            if let Some(shared) = shared.upgrade() {
                shared.capture();
            }
        });
        unsafe {
            let timer: Retained<AnyObject> = msg_send![class!(NSTimer), timerWithTimeInterval: interval.as_secs_f64(), repeats: true, block: &*tick];
            let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
            // Common modes keep frames coming during live resize and menu tracking.
            let _: () = msg_send![&run_loop, addTimer: &*timer, forMode: NSRunLoopCommonModes];
            self.timer = Some(timer);
        }
    }

    fn stop_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            let _: () = unsafe { msg_send![&timer, invalidate] };
        }
    }

    pub(super) fn command(
        &mut self,
        revision: u64,
        command: Option<&WebSurfaceCommand>,
    ) -> Result<(), String> {
        self.shared.revision.set(revision);
        let view = self.shared.view.borrow().clone().ok_or("网页画面已关闭")?;
        match command {
            None => {}
            Some(WebSurfaceCommand::Navigate(url)) => {
                load_url(&view, url, &self.shared.policy)?;
                self.shared.error.borrow_mut().take();
            }
            Some(WebSurfaceCommand::Reload) => {
                self.shared.error.borrow_mut().take();
                let _: Option<Retained<AnyObject>> = unsafe { msg_send![&view, reload] };
            }
            Some(WebSurfaceCommand::ShowWindow { title }) => {
                self.shared.interactive.set(true);
                self.shared.show(title, self.mtm);
            }
            Some(WebSurfaceCommand::HideWindow) => self.shared.hide(),
        }
        self.shared.publish_state();
        Ok(())
    }

    pub(super) fn take_events(&mut self) -> Vec<WebSurfaceCompletion> {
        self.shared.events.borrow_mut().take()
    }
}

impl Drop for PlatformSurface {
    fn drop(&mut self) {
        self.stop_timer();
        self.shared.events.borrow_mut().close();
        let view = self.shared.view.borrow_mut().take();
        let window = self.shared.window.borrow_mut().take();
        unsafe {
            if let Some(view) = view {
                for key in OBSERVED_KEYS {
                    let key = NSString::from_str(key);
                    let _: () =
                        msg_send![&view, removeObserver: &*self.delegate, forKeyPath: &*key];
                }
                let _: () = msg_send![&view, setNavigationDelegate: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![&view, setUIDelegate: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![&view, stopLoading];
            }
            if let Some(window) = window {
                window.setDelegate(None);
                window.setContentView(None);
                window.orderOut(None);
                window.close();
            }
        }
    }
}
