//! Platform-neutral desktop notification contracts shared across
//! `nana-window` (native hosts) and `nana-ui` (Scene runtime integration).

use std::fmt;

/// Native backend providing desktop notifications on the current platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationBackendKind {
    /// macOS `UNUserNotificationCenter` (requires app bundle identifier).
    UserNotifications,
    /// Windows `Windows.UI.Notifications.ToastNotificationManager`.
    WindowsToast,
    /// Linux XDG / Freedesktop `org.freedesktop.Notifications` over session D-Bus.
    FreedesktopNotifications,
    /// Notifications are unavailable or unbundled in the current environment.
    Unavailable,
}

/// Current notification authorization state reported by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPermission {
    /// Allowed to present notifications.
    Granted,
    /// User or policy explicitly denied notifications.
    Denied,
    /// Authorization has not been granted yet; call `request_permission`.
    Prompt,
    /// Platform notification service is unavailable.
    Unsupported,
}

/// Individual capabilities supported by a desktop notification daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationCapability {
    Actions,
    ReplaceByTag,
    Image,
    Silent,
    Persistent,
}

/// Lifetime preference for a presented notification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NotificationTimeout {
    /// Follow the desktop environment's default expiration policy.
    #[default]
    Default,
    /// Request that the notification stay open until dismissed or activated.
    Persistent,
    /// Request expiration after `u32` milliseconds where supported.
    Millis(u32),
}

/// Action button offered inside a desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

impl NotificationAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// Complete host capability summary returned by `NotificationHost::support()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSupport {
    pub backend: NotificationBackendKind,
    pub permission: NotificationPermission,
    pub supports_actions: bool,
    pub supports_replace: bool,
    pub supports_images: bool,
    pub unavailable_reason: Option<String>,
}

impl NotificationSupport {
    pub const fn is_available(&self) -> bool {
        !matches!(self.backend, NotificationBackendKind::Unavailable)
    }

    pub fn supports(&self, capability: NotificationCapability) -> bool {
        if !self.is_available() {
            return false;
        }
        match capability {
            NotificationCapability::Actions => self.supports_actions,
            NotificationCapability::ReplaceByTag => self.supports_replace,
            NotificationCapability::Image => self.supports_images,
            NotificationCapability::Silent | NotificationCapability::Persistent => true,
        }
    }
}

/// Pure-data description of a desktop notification to present or update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationRequest {
    /// Optional replacement / coalescing tag. Updating or presenting with the
    /// same tag replaces the prior notification where supported.
    pub tag: Option<String>,
    /// Application display name override (used primarily on Freedesktop).
    pub app_name: Option<String>,
    pub title: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
    /// Optional PNG image attached to the notification (e.g. hero / icon).
    pub image_png: Option<Vec<u8>>,
    pub timeout: NotificationTimeout,
    /// Suppress default sound / banner chime where supported.
    pub silent: bool,
}

impl NotificationRequest {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    #[must_use]
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.actions.push(NotificationAction::new(id, label));
        self
    }

    #[must_use]
    pub fn image_png(mut self, bytes: Vec<u8>) -> Self {
        self.image_png = Some(bytes);
        self
    }

    #[must_use]
    pub fn timeout(mut self, timeout: NotificationTimeout) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
}

/// Outcome returned when presenting or updating a notification succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationOutcome {
    pub id: String,
    pub replaced: bool,
    pub backend: NotificationBackendKind,
}

/// Why a desktop notification was closed without invoking an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationDismissReason {
    User,
    Expired,
    Closed,
    Replaced,
    Unknown,
}

/// Asynchronous user interaction or dismissal emitted by the native host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    Activated {
        id: String,
        action_id: Option<String>,
    },
    Dismissed {
        id: String,
        reason: NotificationDismissReason,
    },
}

/// Failure returned when presenting, updating, or withdrawing a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationError {
    Unavailable(String),
    PermissionDenied,
    InvalidRequest(String),
    Platform(String),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "notification host unavailable: {reason}"),
            Self::PermissionDenied => f.write_str("notification permission denied"),
            Self::InvalidRequest(reason) => write!(f, "invalid notification request: {reason}"),
            Self::Platform(reason) => write!(f, "notification platform error: {reason}"),
        }
    }
}

impl std::error::Error for NotificationError {}
