use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs,
    ToastNotification, ToastNotificationManager,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID;
use windows_core::{HSTRING, IInspectable, Interface, Ref};

use super::{
    EventHub, NotificationBackendKind, NotificationDismissReason, NotificationError,
    NotificationEvent, NotificationOutcome, NotificationPermission, NotificationRequest,
    NotificationSupport, build_windows_toast_xml, parse_windows_activation_args,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const DEFAULT_AUMID: &str = "NanaUI.DesktopApp";
const TOAST_GROUP: &str = "nanaui";

pub(super) struct WindowsNotificationBackend {
    hub: Arc<EventHub>,
    temp_images: Mutex<Vec<std::path::PathBuf>>,
}

impl WindowsNotificationBackend {
    pub(super) fn new(hub: Arc<EventHub>) -> Self {
        Self {
            hub,
            temp_images: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn support(&self) -> NotificationSupport {
        ensure_com_initialized();
        let app_id = current_aumid();
        match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&app_id)) {
            Ok(_) => NotificationSupport {
                backend: NotificationBackendKind::WindowsToast,
                permission: NotificationPermission::Granted,
                supports_actions: true,
                supports_replace: true,
                supports_images: true,
                unavailable_reason: None,
            },
            Err(error) => NotificationSupport {
                backend: NotificationBackendKind::Unavailable,
                permission: NotificationPermission::Unsupported,
                supports_actions: false,
                supports_replace: false,
                supports_images: false,
                unavailable_reason: Some(format!("Windows ToastNotifier unavailable: {error}")),
            },
        }
    }

    pub(super) fn request_permission(&self) -> Result<NotificationPermission, NotificationError> {
        let sup = self.support();
        if sup.is_available() {
            Ok(sup.permission)
        } else {
            Err(NotificationError::Unavailable(
                sup.unavailable_reason
                    .unwrap_or_else(|| "Windows ToastNotifier unavailable".into()),
            ))
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
        ensure_com_initialized();
        let app_id = HSTRING::from(&current_aumid());
        let tag = HSTRING::from(toast_tag(id));
        let group = HSTRING::from(TOAST_GROUP);
        let history = ToastNotificationManager::History().map_err(|e| {
            NotificationError::Unavailable(format!("toast history unavailable: {e}"))
        })?;
        history
            .RemoveGroupedTagWithId(&tag, &group, &app_id)
            .map_err(|e| NotificationError::Platform(e.to_string()))
    }

    pub(super) fn clear(&self) -> Result<(), NotificationError> {
        ensure_com_initialized();
        let app_id = HSTRING::from(&current_aumid());
        let history = ToastNotificationManager::History().map_err(|e| {
            NotificationError::Unavailable(format!("toast history unavailable: {e}"))
        })?;
        history
            .ClearWithId(&app_id)
            .map_err(|e| NotificationError::Platform(e.to_string()))?;
        if let Ok(mut paths) = self.temp_images.lock() {
            for path in paths.drain(..) {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    fn deliver(
        &self,
        id: &str,
        request: NotificationRequest,
        replaced: bool,
    ) -> Result<NotificationOutcome, NotificationError> {
        ensure_com_initialized();
        let app_id = current_aumid();
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&app_id))
            .map_err(|e| {
                NotificationError::Unavailable(format!("ToastNotifier unavailable: {e}"))
            })?;

        let image_uri = if let Some(png_bytes) = request.image_png.as_deref() {
            write_temp_toast_image(&self.temp_images, id, png_bytes)
        } else {
            None
        };

        let xml_str = build_windows_toast_xml(id, &request, image_uri.as_deref());
        let doc = XmlDocument::new().map_err(|e| NotificationError::Platform(e.to_string()))?;
        doc.LoadXml(&HSTRING::from(&xml_str))
            .map_err(|e| NotificationError::Platform(format!("LoadXml failed: {e}")))?;

        let toast = ToastNotification::CreateToastNotification(&doc)
            .map_err(|e| NotificationError::Platform(e.to_string()))?;
        let tag = toast_tag(id);
        let _ = toast.SetTag(&HSTRING::from(&tag));
        let _ = toast.SetGroup(&HSTRING::from(TOAST_GROUP));

        let hub_act = Arc::clone(&self.hub);
        let act_id = id.to_owned();
        let _ = toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, args: Ref<'_, IInspectable>| {
                let mut parsed_action = None;
                if let Some(insp) = args.as_ref()
                    && let Ok(toast_args) = insp.cast::<ToastActivatedEventArgs>()
                    && let Ok(arg_hstr) = toast_args.Arguments()
                {
                    let (_, action) = parse_windows_activation_args(&arg_hstr.to_string_lossy());
                    parsed_action = action;
                }
                hub_act.push(NotificationEvent::Activated {
                    id: act_id.clone(),
                    action_id: parsed_action,
                });
                Ok(())
            },
        ));

        let hub_dis = Arc::clone(&self.hub);
        let dis_id = id.to_owned();
        let _ = toast.Dismissed(&TypedEventHandler::<
            ToastNotification,
            ToastDismissedEventArgs,
        >::new(
            move |_, args: Ref<'_, ToastDismissedEventArgs>| {
                let reason = match args.as_ref().and_then(|a| a.Reason().ok()) {
                    Some(ToastDismissalReason::UserCanceled) => NotificationDismissReason::User,
                    Some(ToastDismissalReason::TimedOut) => NotificationDismissReason::Expired,
                    Some(ToastDismissalReason::ApplicationHidden) => {
                        NotificationDismissReason::Closed
                    }
                    _ => NotificationDismissReason::Unknown,
                };
                hub_dis.push(NotificationEvent::Dismissed {
                    id: dis_id.clone(),
                    reason,
                });
                Ok(())
            },
        ));

        let hub_fail = Arc::clone(&self.hub);
        let fail_id = id.to_owned();
        let _ = toast.Failed(
            &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, _| {
                hub_fail.push(NotificationEvent::Dismissed {
                    id: fail_id.clone(),
                    reason: NotificationDismissReason::Closed,
                });
                Ok(())
            }),
        );

        notifier
            .Show(&toast)
            .map_err(|e| NotificationError::Platform(format!("ToastNotifier::Show failed: {e}")))?;

        Ok(NotificationOutcome {
            id: id.to_owned(),
            replaced,
            backend: NotificationBackendKind::WindowsToast,
        })
    }
}

impl Drop for WindowsNotificationBackend {
    fn drop(&mut self) {
        if let Ok(mut paths) = self.temp_images.lock() {
            for path in paths.drain(..) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn ensure_com_initialized() {
    // SAFETY: `CoInitializeEx` returns `RPC_E_CHANGED_MODE` harmlessly if COM is
    // already initialized on this thread with another apartment model.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn current_aumid() -> String {
    unsafe {
        if let Ok(pwstr) = GetCurrentProcessExplicitAppUserModelID() {
            let text = pwstr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(pwstr.0.cast()));
            if !text.trim().is_empty() {
                return text;
            }
        }
    }
    DEFAULT_AUMID.to_owned()
}

/// Windows `ToastNotification.Tag` has a 64-character limit (16 chars prior to
/// RS5, 64 chars since 1809); hash long ids deterministically to stay within bounds.
fn toast_tag(id: &str) -> String {
    if id.len() <= 60 {
        return id.to_owned();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("nana-{hash:016x}")
}

fn write_temp_toast_image(
    temp_paths: &Mutex<Vec<std::path::PathBuf>>,
    id: &str,
    bytes: &[u8],
) -> Option<String> {
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let sanitized: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = std::env::temp_dir().join(format!(
        "nanaui-toast-{}-{}-{seq}.png",
        std::process::id(),
        sanitized
    ));
    std::fs::write(&path, bytes).ok()?;
    let uri = format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    if let Ok(mut guard) = temp_paths.lock() {
        guard.push(path);
    }
    Some(uri)
}
