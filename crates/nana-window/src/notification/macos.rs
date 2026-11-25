use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{AnyThread, DeclaredClass, define_class, msg_send};
use objc2_foundation::{NSArray, NSData, NSError, NSString, NSURL};

use super::{
    EventHub, NotificationBackendKind, NotificationDismissReason, NotificationError,
    NotificationEvent, NotificationOutcome, NotificationPermission, NotificationRequest,
    NotificationSupport, NotificationTimeout,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const UN_RESPONSE_DEFAULT: &str = "com.apple.UNNotificationDefaultActionIdentifier";
const UN_RESPONSE_DISMISS: &str = "com.apple.UNNotificationDismissActionIdentifier";

// UNAuthorizationStatus constants
const UN_AUTH_NOT_DETERMINED: isize = 0;
const UN_AUTH_DENIED: isize = 1;
const UN_AUTH_AUTHORIZED: isize = 2;
const UN_AUTH_PROVISIONAL: isize = 3;

// UNAuthorizationOptions: Alert (4) | Sound (2) | Badge (1)
const UN_AUTH_OPTIONS: usize = (1 << 0) | (1 << 1) | (1 << 2);
// UNNotificationPresentationOptions: Banner (16) | List (8) | Sound (2)
const UN_PRESENT_OPTIONS: usize = (1 << 1) | (1 << 3) | (1 << 4);

struct DelegateIvars {
    hub: Arc<EventHub>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "NanaUserNotificationDelegate"]
    #[ivars = DelegateIvars]
    struct NotificationDelegate;

    impl NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        unsafe fn will_present(
            &self,
            _center: *mut AnyObject,
            _notification: *mut AnyObject,
            completion_handler: *mut AnyObject,
        ) {
            if !completion_handler.is_null() {
                // SAFETY: `completion_handler` is an Objective-C `void (^)(UNNotificationPresentationOptions)` block.
                unsafe {
                    let block: &Block<dyn Fn(usize)> = &*completion_handler.cast();
                    block.call((UN_PRESENT_OPTIONS,));
                }
            }
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        unsafe fn did_receive_response(
            &self,
            _center: *mut AnyObject,
            response: *mut AnyObject,
            completion_handler: *mut AnyObject,
        ) {
            if !response.is_null() {
                // SAFETY: `UNNotificationResponse` responds to `notification.request.identifier`
                // and `actionIdentifier`.
                unsafe {
                    let action_ns: Option<Retained<NSString>> =
                        msg_send![response, actionIdentifier];
                    let notif: *mut AnyObject = msg_send![response, notification];
                    let req: *mut AnyObject = if !notif.is_null() {
                        msg_send![notif, request]
                    } else {
                        std::ptr::null_mut()
                    };
                    let id_ns: Option<Retained<NSString>> = if !req.is_null() {
                        msg_send![req, identifier]
                    } else {
                        None
                    };
                    if let Some(id) = id_ns.map(|s| s.to_string()) {
                        let action = action_ns.map(|s| s.to_string()).unwrap_or_default();
                        let event = if action == UN_RESPONSE_DISMISS {
                            NotificationEvent::Dismissed {
                                id,
                                reason: NotificationDismissReason::User,
                            }
                        } else if action == UN_RESPONSE_DEFAULT || action.is_empty() {
                            NotificationEvent::Activated {
                                id,
                                action_id: None,
                            }
                        } else {
                            NotificationEvent::Activated {
                                id,
                                action_id: Some(action),
                            }
                        };
                        self.ivars().hub.push(event);
                    }
                }
            }
            if !completion_handler.is_null() {
                // SAFETY: `completion_handler` is an Objective-C `void (^)(void)` block.
                unsafe {
                    let block: &Block<dyn Fn()> = &*completion_handler.cast();
                    block.call(());
                }
            }
        }
    }
);

impl NotificationDelegate {
    fn new(hub: Arc<EventHub>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { hub });
        // SAFETY: `NSObject::init` on a freshly allocated delegate instance.
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) struct MacNotificationBackend {
    hub: Arc<EventHub>,
    delegate: Mutex<Option<Retained<NotificationDelegate>>>,
    temp_attachments: Mutex<Vec<std::path::PathBuf>>,
}

unsafe impl Send for MacNotificationBackend {}
unsafe impl Sync for MacNotificationBackend {}

impl MacNotificationBackend {
    pub(super) fn new(hub: Arc<EventHub>) -> Self {
        Self {
            hub,
            delegate: Mutex::new(None),
            temp_attachments: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn support(&self) -> NotificationSupport {
        let Some(center) = ensure_center() else {
            return NotificationSupport {
                backend: NotificationBackendKind::Unavailable,
                permission: NotificationPermission::Unsupported,
                supports_actions: false,
                supports_replace: false,
                supports_images: false,
                unavailable_reason: Some(
                    "macOS UNUserNotificationCenter requires an application bundle identifier"
                        .into(),
                ),
            };
        };
        let permission = query_permission(center);
        NotificationSupport {
            backend: NotificationBackendKind::UserNotifications,
            permission,
            supports_actions: true,
            supports_replace: true,
            supports_images: true,
            unavailable_reason: None,
        }
    }

    pub(super) fn request_permission(&self) -> Result<NotificationPermission, NotificationError> {
        let center = self.ready_center()?;
        let (tx, rx) = mpsc::channel::<Result<bool, String>>();
        let handler = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if !error.is_null() {
                // SAFETY: `error` is a valid `NSError` pointer passed by UserNotifications.
                let msg = unsafe { (&*error).localizedDescription().to_string() };
                let _ = tx.send(Err(msg));
            } else {
                let _ = tx.send(Ok(granted.as_bool()));
            }
        });
        unsafe {
            let _: () = msg_send![
                center,
                requestAuthorizationWithOptions: UN_AUTH_OPTIONS,
                completionHandler: &*handler
            ];
        }
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(true)) => Ok(NotificationPermission::Granted),
            Ok(Ok(false)) => Ok(NotificationPermission::Denied),
            Ok(Err(msg)) => Err(NotificationError::Platform(msg)),
            Err(_) => Ok(query_permission(center)),
        }
    }

    pub(super) fn present(
        &self,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        let replaced = request.tag.is_some();
        let id = request
            .tag
            .clone()
            .unwrap_or_else(|| format!("nana-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
        self.deliver(&id, request, replaced)
    }

    pub(super) fn update(
        &self,
        id: &str,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        self.deliver(id, request, true)
    }

    pub(super) fn withdraw(&self, id: &str) -> Result<(), NotificationError> {
        let center = self.ready_center()?;
        let ns_id = NSString::from_str(id);
        let arr: Retained<NSArray<NSString>> = NSArray::from_slice(&[&*ns_id]);
        unsafe {
            let _: () = msg_send![center, removeDeliveredNotificationsWithIdentifiers: &*arr];
            let _: () = msg_send![center, removePendingNotificationRequestsWithIdentifiers: &*arr];
        }
        Ok(())
    }

    pub(super) fn clear(&self) -> Result<(), NotificationError> {
        let center = self.ready_center()?;
        unsafe {
            let _: () = msg_send![center, removeAllDeliveredNotifications];
            let _: () = msg_send![center, removeAllPendingNotificationRequests];
        }
        if let Ok(mut paths) = self.temp_attachments.lock() {
            for path in paths.drain(..) {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    fn ready_center(&self) -> Result<*mut AnyObject, NotificationError> {
        let Some(center) = ensure_center() else {
            return Err(NotificationError::Unavailable(
                "macOS UNUserNotificationCenter requires an application bundle identifier".into(),
            ));
        };
        if let Ok(mut guard) = self.delegate.lock()
            && guard.is_none()
        {
            let delegate = NotificationDelegate::new(Arc::clone(&self.hub));
            unsafe {
                let _: () = msg_send![center, setDelegate: &*delegate];
            }
            *guard = Some(delegate);
        }
        Ok(center)
    }

    fn deliver(
        &self,
        id: &str,
        request: NotificationRequest,
        replaced: bool,
    ) -> Result<NotificationOutcome, NotificationError> {
        let center = self.ready_center()?;
        let content_cls = AnyClass::get(c"UNMutableNotificationContent").ok_or_else(|| {
            NotificationError::Unavailable("UNMutableNotificationContent missing".into())
        })?;
        let req_cls = AnyClass::get(c"UNNotificationRequest").ok_or_else(|| {
            NotificationError::Unavailable("UNNotificationRequest missing".into())
        })?;

        // SAFETY: Classes loaded from `UserNotifications.framework` above.
        unsafe {
            let content: Retained<AnyObject> = msg_send![content_cls, new];
            let _: () = msg_send![&*content, setTitle: &*NSString::from_str(&request.title)];
            let _: () = msg_send![&*content, setBody: &*NSString::from_str(&request.body)];
            if let Some(tag) = request.tag.as_deref() {
                let _: () = msg_send![&*content, setThreadIdentifier: &*NSString::from_str(tag)];
            }
            if !request.silent
                && let Some(sound_cls) = AnyClass::get(c"UNNotificationSound")
            {
                let sound: Option<Retained<AnyObject>> = msg_send![sound_cls, defaultSound];
                if let Some(sound) = sound {
                    let _: () = msg_send![&*content, setSound: &*sound];
                }
            }
            if !request.actions.is_empty()
                && let Some(cat_id) = register_category(center, id, &request.actions)
            {
                let _: () = msg_send![&*content, setCategoryIdentifier: &*cat_id];
            }
            if let Some(png_bytes) = request.image_png.as_deref()
                && let Some(att) = attach_png(&self.temp_attachments, id, png_bytes)
            {
                let arr: Retained<NSArray<AnyObject>> = NSArray::from_slice(&[&*att]);
                let _: () = msg_send![&*content, setAttachments: &*arr];
            }

            let trigger: *mut AnyObject = match request.timeout {
                NotificationTimeout::Default | NotificationTimeout::Persistent => {
                    std::ptr::null_mut()
                }
                NotificationTimeout::Millis(_) => std::ptr::null_mut(),
            };
            let un_req: Retained<AnyObject> = msg_send![
                req_cls,
                requestWithIdentifier: &*NSString::from_str(id),
                content: &*content,
                trigger: trigger
            ];
            let (tx, rx) = mpsc::channel::<Option<String>>();
            let completion = RcBlock::new(move |error: *mut NSError| {
                if error.is_null() {
                    let _ = tx.send(None);
                } else {
                    let msg = (&*error).localizedDescription().to_string();
                    let _ = tx.send(Some(msg));
                }
            });
            let _: () = msg_send![
                center,
                addNotificationRequest: &*un_req,
                withCompletionHandler: &*completion
            ];
            if let Ok(Some(msg)) = rx.recv_timeout(Duration::from_secs(3)) {
                return Err(NotificationError::Platform(msg));
            }
        }

        Ok(NotificationOutcome {
            id: id.to_owned(),
            replaced,
            backend: NotificationBackendKind::UserNotifications,
        })
    }
}

impl Drop for MacNotificationBackend {
    fn drop(&mut self) {
        if let Ok(mut paths) = self.temp_attachments.lock() {
            for path in paths.drain(..) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Checks `NSBundle.mainBundle.bundleIdentifier` FIRST so unbundled binaries
/// (tests, CLI probes) never trigger `+[UNUserNotificationCenter currentNotificationCenter]`'s
/// fatal `NSInternalInconsistencyException`.
fn has_bundle_identifier() -> bool {
    let Some(bundle_cls) = AnyClass::get(c"NSBundle") else {
        return false;
    };
    unsafe {
        let main: *mut AnyObject = msg_send![bundle_cls, mainBundle];
        if main.is_null() {
            return false;
        }
        let bid: Option<Retained<NSString>> = msg_send![main, bundleIdentifier];
        bid.is_some_and(|s| !s.to_string().is_empty())
    }
}

fn ensure_center() -> Option<*mut AnyObject> {
    if !has_bundle_identifier() {
        return None;
    }
    static FRAMEWORK_LOADED: OnceLock<bool> = OnceLock::new();
    let loaded = *FRAMEWORK_LOADED.get_or_init(|| {
        let Some(bundle_cls) = AnyClass::get(c"NSBundle") else {
            return false;
        };
        let path = NSString::from_str("/System/Library/Frameworks/UserNotifications.framework");
        unsafe {
            let bundle: Option<Retained<AnyObject>> = msg_send![bundle_cls, bundleWithPath: &*path];
            bundle.is_some_and(|b| msg_send![&*b, load])
        }
    });
    if !loaded {
        return None;
    }
    let center_cls = AnyClass::get(c"UNUserNotificationCenter")?;
    unsafe {
        let center: *mut AnyObject = msg_send![center_cls, currentNotificationCenter];
        (!center.is_null()).then_some(center)
    }
}

fn query_permission(center: *mut AnyObject) -> NotificationPermission {
    let (tx, rx) = mpsc::channel::<isize>();
    let block = RcBlock::new(move |settings: *mut AnyObject| {
        if settings.is_null() {
            let _ = tx.send(-1);
        } else {
            let status: isize = unsafe { msg_send![settings, authorizationStatus] };
            let _ = tx.send(status);
        }
    });
    unsafe {
        let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*block];
    }
    match rx.recv_timeout(Duration::from_millis(500)) {
        Ok(UN_AUTH_AUTHORIZED | UN_AUTH_PROVISIONAL) => NotificationPermission::Granted,
        Ok(UN_AUTH_DENIED) => NotificationPermission::Denied,
        Ok(UN_AUTH_NOT_DETERMINED) => NotificationPermission::Prompt,
        _ => NotificationPermission::Prompt,
    }
}

unsafe fn register_category(
    center: *mut AnyObject,
    id: &str,
    actions: &[super::NotificationAction],
) -> Option<Retained<NSString>> {
    let action_cls = AnyClass::get(c"UNNotificationAction")?;
    let cat_cls = AnyClass::get(c"UNNotificationCategory")?;
    let set_cls = AnyClass::get(c"NSSet")?;
    let mut built_actions: Vec<Retained<AnyObject>> = Vec::with_capacity(actions.len());
    for action in actions {
        let obj: Option<Retained<AnyObject>> = msg_send![
            action_cls,
            actionWithIdentifier: &*NSString::from_str(&action.id),
            title: &*NSString::from_str(&action.label),
            options: 1usize // UNNotificationActionOptionForeground
        ];
        if let Some(obj) = obj {
            built_actions.push(obj);
        }
    }
    let refs: Vec<&AnyObject> = built_actions.iter().map(|o| &**o).collect();
    let actions_arr = NSArray::from_slice(&refs);
    let empty_arr = NSArray::<AnyObject>::new();
    let cat_id = NSString::from_str(&format!("nana.cat.{id}"));
    let cat: Option<Retained<AnyObject>> = msg_send![
        cat_cls,
        categoryWithIdentifier: &*cat_id,
        actions: &*actions_arr,
        intentIdentifiers: &*empty_arr,
        options: 0usize
    ];
    let cat = cat?;
    let cats_set: Retained<AnyObject> = msg_send![set_cls, setWithObject: &*cat];
    let _: () = msg_send![center, setNotificationCategories: &*cats_set];
    Some(cat_id)
}

unsafe fn attach_png(
    temp_paths: &Mutex<Vec<std::path::PathBuf>>,
    id: &str,
    bytes: &[u8],
) -> Option<Retained<AnyObject>> {
    let att_cls = AnyClass::get(c"UNNotificationAttachment")?;
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let sanitized: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = std::env::temp_dir().join(format!(
        "nanaui-notif-{}-{}-{seq}.png",
        std::process::id(),
        sanitized
    ));
    std::fs::write(&path, bytes).ok()?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let mut err: *mut NSError = std::ptr::null_mut();
    let att: Option<Retained<AnyObject>> = msg_send![
        att_cls,
        attachmentWithIdentifier: &*NSString::from_str("image"),
        URL: &*url,
        options: std::ptr::null::<c_void>(),
        error: &mut err
    ];
    if att.is_some()
        && let Ok(mut guard) = temp_paths.lock()
    {
        guard.push(path);
    } else {
        let _ = std::fs::remove_file(&path);
    }
    let _ = NSData::new();
    att
}
