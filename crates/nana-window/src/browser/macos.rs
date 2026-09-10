use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{
    BrowserCommand, BrowserCompletion, BrowserEvent, BrowserPolicy, BrowserRect, BrowserState,
};

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

#[derive(Default)]
struct BrowserCallbacks {
    events: RefCell<Vec<BrowserCompletion>>,
    revision: Cell<u64>,
    navigation: Cell<u64>,
    closed: Cell<bool>,
}

impl BrowserCallbacks {
    fn publish(&self, revision: u64, event: BrowserEvent) {
        if self.closed.get() {
            return;
        }
        let mut events = self.events.borrow_mut();
        if matches!(&event, BrowserEvent::State(_)) {
            events.retain(|old| !matches!(&old.event, BrowserEvent::State(_)));
        }
        events.push(BrowserCompletion { revision, event });
    }
    fn invalidate_capture(&self) {
        self.navigation.set(self.navigation.get().wrapping_add(1));
    }
    fn finish_capture(&self, revision: u64, navigation: u64, result: Result<Vec<u8>, String>) {
        let result = if self.navigation.get() == navigation {
            result
        } else {
            Err("网页已变化，请重新截图".into())
        };
        self.publish(
            revision,
            match result {
                Ok(bytes) => BrowserEvent::Captured(bytes),
                Err(error) => BrowserEvent::CaptureFailed(error),
            },
        );
    }
    fn close(&self) {
        self.closed.set(true);
        self.events.borrow_mut().clear();
    }
}

struct DelegateState {
    events: Rc<BrowserCallbacks>,
    policy: BrowserPolicy,
    wake: Rc<dyn Fn()>,
    error: RefCell<Option<String>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    struct BrowserDelegate;

    unsafe impl NSObjectProtocol for BrowserDelegate {}

    impl BrowserDelegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observed(&self, _key: &NSString, view: &AnyObject, _change: Option<&AnyObject>, _context: *mut std::ffi::c_void) {
            self.publish(view, None);
        }
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn new_window(&self, view: &AnyObject, _configuration: &AnyObject, action: &AnyObject, _features: &AnyObject) -> *mut AnyObject {
            unsafe {
                let request: Retained<AnyObject> = msg_send![action, request];
                let url: Option<Retained<AnyObject>> = msg_send![&request, URL];
                if url.is_some_and(|url| self.ivars().policy.allows(&url_string(&url))) {
                    let _: Option<Retained<AnyObject>> = msg_send![view, loadRequest: &*request];
                }
            }
            std::ptr::null_mut()
        }
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.ivars().error.borrow_mut().take();
            self.ivars().events.invalidate_capture();
            self.publish(view, None);
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.publish(view, None);
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_provisional(&self, view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            let code: isize = unsafe { msg_send![error, code] };
            if code != -999 {
                self.publish(view, Some(error_description(error)));
            }
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            let code: isize = unsafe { msg_send![error, code] };
            if code != -999 {
                self.publish(view, Some(error_description(error)));
            }
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, view: &AnyObject) {
            self.publish(view, Some("网页进程已退出，请重新加载".into()));
        }
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(&self, view: &AnyObject, action: &AnyObject, completion: &block2::Block<dyn Fn(isize)>) {
            let url = unsafe {
                let request: Retained<AnyObject> = msg_send![action, request];
                let url: Option<Retained<AnyObject>> = msg_send![&request, URL];
                url.map(|url| url_string(&url)).unwrap_or_default()
            };
            let allowed = self.ivars().policy.allows(&url);
            completion.call((if allowed { 1 } else { 0 },));
            if !allowed { self.publish(view, Some("不允许打开此地址".into())); }
        }
    }
);

impl BrowserDelegate {
    fn publish(&self, view: &AnyObject, error: Option<String>) {
        if let Some(error) = error {
            *self.ivars().error.borrow_mut() = Some(error);
        }
        let error = self.ivars().error.borrow().clone();
        if self.ivars().events.closed.get() {
            return;
        }
        let callbacks = &self.ivars().events;
        callbacks.publish(
            callbacks.revision.get(),
            BrowserEvent::State(read_state(view, error)),
        );
        (self.ivars().wake)();
    }
}

pub(super) struct MacBrowser {
    view: Retained<AnyObject>,
    container: Retained<NSView>,
    parent: Retained<NSView>,
    _delegate: Retained<BrowserDelegate>,
    events: Rc<BrowserCallbacks>,
    wake: Rc<dyn Fn()>,
    policy: BrowserPolicy,
    visible: bool,
    geometry: Option<(BrowserRect, BrowserRect, bool, f64, bool)>,
}

impl MacBrowser {
    pub(super) fn new<W: HasWindowHandle + ?Sized>(
        window: &W,
        policy: BrowserPolicy,
        wake: Box<dyn Fn()>,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("浏览器必须在主线程创建")?;
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("窗口不是 AppKit 窗口".into());
        };
        let parent = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
            .ok_or("窗口视图已销毁")?;
        let events = Rc::new(BrowserCallbacks::default());
        let wake: Rc<dyn Fn()> = wake.into();
        let delegate: Retained<BrowserDelegate> = unsafe {
            let allocated = BrowserDelegate::alloc(mtm).set_ivars(DelegateState {
                events: events.clone(),
                policy: policy.clone(),
                wake: wake.clone(),
                error: RefCell::new(None),
            });
            msg_send![super(allocated), init]
        };
        let container = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        let view: Retained<AnyObject> = unsafe {
            container.setWantsLayer(true);
            if let Some(layer) = container.layer() {
                layer.setMasksToBounds(true);
            }
            let configuration: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let allocated: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let view: Retained<AnyObject> =
                msg_send![allocated, initWithFrame: NSRect::ZERO, configuration: &*configuration];
            let _: () = msg_send![&view, setNavigationDelegate: &*delegate];
            let _: () = msg_send![&view, setUIDelegate: &*delegate];
            for key in ["URL", "title", "loading", "canGoBack", "canGoForward"] {
                let key = NSString::from_str(key);
                let _: () = msg_send![&view, addObserver: &*delegate, forKeyPath: &*key, options: 0usize, context: std::ptr::null_mut::<std::ffi::c_void>()];
            }
            let native_view: &NSView = &*((&*view as *const AnyObject).cast::<NSView>());
            container.addSubview(native_view);
            container.setHidden(true);
            parent.addSubview(&container);
            view
        };
        Ok(Self {
            view,
            container,
            parent,
            _delegate: delegate,
            events,
            wake,
            policy,
            visible: false,
            geometry: None,
        })
    }

    pub(super) fn set_geometry(&mut self, bounds: BrowserRect, clip: BrowserRect, visible: bool) {
        let finite = [
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            clip.x,
            clip.y,
            clip.width,
            clip.height,
        ]
        .into_iter()
        .all(f64::is_finite);
        let visible = visible
            && finite
            && bounds.width > 0.0
            && bounds.height > 0.0
            && clip.width > 0.0
            && clip.height > 0.0;
        let (bounds, clip) = if finite {
            (bounds, clip)
        } else {
            (BrowserRect::default(), BrowserRect::default())
        };
        if !visible && self.visible {
            self.events.invalidate_capture();
        }
        unsafe {
            let parent_bounds = self.parent.bounds();
            let flipped = self.parent.isFlipped();
            let geometry = (bounds, clip, visible, parent_bounds.size.height, flipped);
            if self.geometry == Some(geometry) {
                return;
            }
            self.geometry = Some(geometry);
            let y = if flipped {
                clip.y
            } else {
                parent_bounds.size.height - clip.y - clip.height
            };
            self.container.setFrame(NSRect::new(
                NSPoint::new(clip.x, y),
                NSSize::new(clip.width, clip.height),
            ));
            let frame = NSRect::new(
                NSPoint::new(
                    bounds.x - clip.x,
                    clip.height - (bounds.y - clip.y) - bounds.height,
                ),
                NSSize::new(bounds.width, bounds.height),
            );
            let _: () = msg_send![&self.view, setFrame: frame];
            if !visible
                && self.visible
                && let Some(window) = self.parent.window()
            {
                let responder = window.firstResponder();
                let browser: &NSView = &*((&*self.view as *const AnyObject).cast::<NSView>());
                if responder.as_ref().is_some_and(|value| {
                    let view: Option<&NSView> = value.downcast_ref();
                    view.is_some_and(|value| value.isDescendantOf(browser))
                }) {
                    window.makeFirstResponder(Some(&self.parent));
                }
            }
            self.container.setHidden(!visible);
        }
        if visible && !self.visible {
            self.events.publish(
                self.events.revision.get(),
                BrowserEvent::State(read_state(
                    &self.view,
                    self._delegate.ivars().error.borrow().clone(),
                )),
            );
            (self.wake)();
        }
        self.visible = visible;
    }

    pub(super) fn command(
        &mut self,
        revision: u64,
        command: Option<&BrowserCommand>,
    ) -> Result<(), String> {
        self.events.revision.set(revision);
        let Some(command) = command else {
            self.events.publish(
                revision,
                BrowserEvent::State(read_state(
                    &self.view,
                    self._delegate.ivars().error.borrow().clone(),
                )),
            );
            (self.wake)();
            return Ok(());
        };
        if matches!(
            command,
            BrowserCommand::Navigate(_)
                | BrowserCommand::Back
                | BrowserCommand::Forward
                | BrowserCommand::Reload
        ) {
            self.events.invalidate_capture();
            self._delegate.ivars().error.borrow_mut().take();
        }
        if matches!(command, BrowserCommand::Capture)
            && (!self.visible || read_state(&self.view, None).loading)
        {
            return Err("网页尚未准备好，请稍后再试".into());
        }
        unsafe {
            match command {
                BrowserCommand::Navigate(url) => {
                    if !self.policy.allows(url) {
                        return Err("不允许打开此地址".into());
                    }
                    let url_string = NSString::from_str(url);
                    let url: Option<Retained<AnyObject>> =
                        msg_send![class!(NSURL), URLWithString: &*url_string];
                    let url = url.ok_or("地址无效")?;
                    let request: Retained<AnyObject> =
                        msg_send![class!(NSURLRequest), requestWithURL: &*url];
                    let _: Option<Retained<AnyObject>> =
                        msg_send![&self.view, loadRequest: &*request];
                }
                BrowserCommand::Back => {
                    let _: Option<Retained<AnyObject>> = msg_send![&self.view, goBack];
                }
                BrowserCommand::Forward => {
                    let _: Option<Retained<AnyObject>> = msg_send![&self.view, goForward];
                }
                BrowserCommand::Reload => {
                    let _: Option<Retained<AnyObject>> = msg_send![&self.view, reload];
                }
                BrowserCommand::Stop => {
                    let _: () = msg_send![&self.view, stopLoading];
                }
                BrowserCommand::Focus => {
                    if self.visible
                        && let Some(window) = self.parent.window()
                    {
                        let _: bool = msg_send![&window, makeFirstResponder: &*self.view];
                    }
                }
                BrowserCommand::Capture => self.capture(revision),
            }
        }
        self.events.publish(
            revision,
            BrowserEvent::State(read_state(
                &self.view,
                self._delegate.ivars().error.borrow().clone(),
            )),
        );
        (self.wake)();
        Ok(())
    }

    fn capture(&self, revision: u64) {
        let navigation = self.events.navigation.get();
        let completed = Cell::new(false);
        let events = self.events.clone();
        let wake = self.wake.clone();
        let completion = RcBlock::new(move |image: *mut AnyObject, error: *mut AnyObject| {
            if completed.replace(true) || events.closed.get() {
                return;
            }
            let result = unsafe {
                if navigation != events.navigation.get() {
                    Err("网页已变化，请重新截图".into())
                } else if let Some(error) = error.as_ref() {
                    Err(error_description(error))
                } else if let Some(image) = image.as_ref() {
                    image_png(image)
                } else {
                    Err("无法截取网页".into())
                }
            };
            events.finish_capture(revision, navigation, result);
            wake();
        });
        unsafe {
            let _: () = msg_send![&self.view, takeSnapshotWithConfiguration: std::ptr::null::<AnyObject>(), completionHandler: &*completion];
        }
    }

    pub(super) fn take_events(&mut self) -> Vec<BrowserCompletion> {
        std::mem::take(&mut *self.events.events.borrow_mut())
    }
}

impl Drop for MacBrowser {
    fn drop(&mut self) {
        self.events.close();
        self.set_geometry(BrowserRect::default(), BrowserRect::default(), false);
        unsafe {
            for key in ["URL", "title", "loading", "canGoBack", "canGoForward"] {
                let key = NSString::from_str(key);
                let _: () =
                    msg_send![&self.view, removeObserver: &*self._delegate, forKeyPath: &*key];
            }
            let _: () = msg_send![&self.view, setNavigationDelegate: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![&self.view, setUIDelegate: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![&self.view, stopLoading];
            self.container.removeFromSuperview();
        }
    }
}

fn url_string(url: &AnyObject) -> String {
    unsafe {
        let value: Option<Retained<NSString>> = msg_send![url, absoluteString];
        value.map(|value| value.to_string()).unwrap_or_default()
    }
}

fn error_description(error: &AnyObject) -> String {
    unsafe {
        let value: Retained<NSString> = msg_send![error, localizedDescription];
        value.to_string()
    }
}

fn read_state(view: &AnyObject, error: Option<String>) -> BrowserState {
    unsafe {
        let url: Option<Retained<AnyObject>> = msg_send![view, URL];
        let title: Option<Retained<NSString>> = msg_send![view, title];
        BrowserState {
            attached: true,
            url: url.map(|value| url_string(&value)).unwrap_or_default(),
            title: title.map(|value| value.to_string()).unwrap_or_default(),
            loading: msg_send![view, isLoading],
            can_go_back: msg_send![view, canGoBack],
            can_go_forward: msg_send![view, canGoForward],
            error,
        }
    }
}

unsafe fn image_png(image: &AnyObject) -> Result<Vec<u8>, String> {
    unsafe {
        let tiff: Option<Retained<AnyObject>> = msg_send![image, TIFFRepresentation];
        let tiff = tiff.ok_or("截图没有像素数据")?;
        let bitmap: Option<Retained<AnyObject>> =
            msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
        let bitmap = bitmap.ok_or("无法解码截图")?;
        let properties: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
        let data: Option<Retained<AnyObject>> =
            msg_send![&bitmap, representationUsingType: 4usize, properties: &*properties];
        let data = data.ok_or("无法编码截图")?;
        let length: usize = msg_send![&data, length];
        let bytes: *const std::ffi::c_void = msg_send![&data, bytes];
        if bytes.is_null() || length == 0 {
            return Err("截图为空".into());
        }
        Ok(std::slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asynchronous_capture_keeps_its_origin_and_rejects_a_changed_page() {
        let callbacks = BrowserCallbacks::default();
        callbacks.revision.set(7);
        let capture_navigation = callbacks.navigation.get();
        callbacks.revision.set(8);
        callbacks.invalidate_capture();
        callbacks.finish_capture(7, capture_navigation, Ok(vec![1, 2, 3]));
        let events = callbacks.events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].revision, 7);
        assert!(matches!(events[0].event, BrowserEvent::CaptureFailed(_)));
    }

    #[test]
    fn closing_releases_queued_results_and_ignores_late_native_callbacks() {
        let callbacks = BrowserCallbacks::default();
        callbacks.finish_capture(2, 0, Ok(vec![1, 2, 3]));
        assert_eq!(callbacks.events.borrow().len(), 1);
        callbacks.close();
        callbacks.finish_capture(3, 0, Ok(vec![4]));
        callbacks.publish(3, BrowserEvent::State(BrowserState::default()));
        assert!(callbacks.events.borrow().is_empty());
    }

    #[test]
    fn remounting_with_the_same_revision_cannot_receive_the_destroyed_views_capture() {
        let old = Rc::new(BrowserCallbacks::default());
        let late_callback = Rc::clone(&old);
        old.close();
        let replacement = BrowserCallbacks::default();
        // Same application node/id/revision can be reused after park; the native
        // callback queue still belongs to the destroyed instance.
        late_callback.finish_capture(7, 0, Ok(vec![1]));
        replacement.finish_capture(7, 0, Ok(vec![2]));
        assert!(old.events.borrow().is_empty());
        assert_eq!(
            replacement.events.borrow()[0].event,
            BrowserEvent::Captured(vec![2])
        );
    }

    #[test]
    fn observed_state_coalesces_without_losing_capture_completions() {
        let callbacks = BrowserCallbacks::default();
        callbacks.publish(4, BrowserEvent::State(BrowserState::default()));
        callbacks.finish_capture(4, 0, Ok(vec![1]));
        callbacks.publish(
            5,
            BrowserEvent::State(BrowserState {
                title: "Page two".into(),
                ..Default::default()
            }),
        );
        let events = callbacks.events.borrow();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].event, BrowserEvent::Captured(_)));
        assert_eq!(events[0].revision, 4);
        assert_eq!(events[1].revision, 5);
    }
}
