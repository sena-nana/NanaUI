//! The application's icon in the system tray, with its menu.
//!
//! Like the menu bar, the tray is desktop chrome the UI tree cannot draw: a
//! menu-bar extra on macOS, a notification-area icon on Windows, or an XDG
//! `StatusNotifierItem` (`org.kde.StatusNotifierItem` + `com.canonical.dbusmenu`)
//! on Linux. It belongs to the application, not to a window, and it is how an
//! application stays reachable while none of its windows can be clicked.
//!
//! The application describes it with the same [`MenuEntry`](crate::MenuEntry)
//! model as the menu bar and reads the chosen ids from the same queue,
//! [`take_menu_activations`](crate::take_menu_activations).
//!
//! A choice can arrive while the application draws no frames, so
//! [`install_tray`] and [`install_tray_with_outcome`] take a wake: it runs
//! right after an id is queued, and is where a host signals its event loop.
//!
//! Call these on the thread that runs the event loop. When the running desktop
//! session has no tray service (for example Linux without a
//! `StatusNotifierWatcher`), installation reports [`TraySupport::Unavailable`]
//! and records the reason in [`TrayOutcome::unavailable_reason`].

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

/// Attention / visibility state for the tray item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrayAttention {
    /// Normal active tray icon.
    #[default]
    Active,
    /// Low-priority or background status (mapped to SNI `Passive` on Linux and
    /// muted appearance on macOS).
    Passive,
    /// Requires user attention (mapped to SNI `NeedsAttention` on Linux).
    NeedsAttention,
}

impl TrayAttention {
    /// Returns the `org.kde.StatusNotifierItem` `Status` property value.
    pub const fn as_sni_status(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Passive => "Passive",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

/// What the tray shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tray {
    /// Shown when the pointer rests on the icon.
    pub tooltip: String,
    /// Primary colour icon, as an encoded PNG. Square; 32 or 36 pixels suits
    /// desktop trays.
    pub icon_png: Vec<u8>,
    /// Explicit monochrome PNG variant used on dark taskbar/panel surfaces
    /// (light foreground glyph). Falls back to auto-generated white mask when
    /// `template_icon` is true, or `icon_png` otherwise.
    pub monochrome_light_icon_png: Option<Vec<u8>>,
    /// Explicit monochrome PNG variant used on light taskbar/panel surfaces
    /// (dark foreground glyph). Falls back to auto-generated black mask when
    /// `template_icon` is true, or `icon_png` otherwise.
    pub monochrome_dark_icon_png: Option<Vec<u8>>,
    /// Treat the icon as a monochrome template mask from the PNG's alpha.
    /// Used natively on macOS and for synthesized light/dark tray variants on
    /// Windows and Linux when explicit monochrome PNGs are omitted.
    pub template_icon: bool,
    /// The menu the icon opens.
    pub menu: Vec<crate::MenuEntry>,
    /// Primary-click command id on platforms where primary click does not always
    /// open the menu (Windows notification area and Linux SNI `Activate`).
    /// `None` opens the menu on primary click as well. macOS always opens the menu.
    pub primary: Option<u32>,
    /// Windows: id reported when the tray icon is double-clicked with the
    /// primary button (`WM_LBUTTONDBLCLK`). Ignored elsewhere.
    pub double_click: Option<u32>,
    /// Initial attention state of the tray item.
    pub attention: TrayAttention,
}

/// How the running platform shows a tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraySupport {
    /// A menu-bar extra (macOS).
    MenuBarExtra,
    /// A notification-area icon (Windows).
    NotificationArea,
    /// XDG / KDE `StatusNotifierItem` + `com.canonical.dbusmenu` over D-Bus (Linux).
    StatusNotifierItem,
    /// Nothing was installed. Keep the commands reachable in a window.
    Unavailable,
}

/// Detailed result from [`install_tray_with_outcome`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayOutcome {
    pub support: TraySupport,
    /// When `support` is [`TraySupport::Unavailable`], describes why the
    /// tray could not be installed (for example, missing `StatusNotifierWatcher`).
    pub unavailable_reason: Option<String>,
}

/// What [`install_tray`] is designed to use on this platform.
///
/// Note that on Linux, actual runtime availability also depends on a session
/// bus and an active `org.kde.StatusNotifierWatcher`; check the return value
/// of [`install_tray`] or [`install_tray_with_outcome`] at install time.
pub const fn tray_support() -> TraySupport {
    #[cfg(target_os = "macos")]
    {
        TraySupport::MenuBarExtra
    }
    #[cfg(target_os = "windows")]
    {
        TraySupport::NotificationArea
    }
    #[cfg(target_os = "linux")]
    {
        TraySupport::StatusNotifierItem
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        TraySupport::Unavailable
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

/// Runs the installed wake. Called after a tray choice is queued.
#[cfg(any(test, target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn notify() {
    let wake = WAKE.lock().ok().and_then(|wake| wake.clone());
    if let Some(wake) = wake {
        wake();
    }
}

/// Shows `tray`, or brings the tray already shown up to date with it.
///
/// Installing again replaces the menu, tooltip, icon and attention in place:
/// the icon keeps its position. `wake` runs right after a chosen id is queued.
///
/// Answers what was installed; a failure or missing desktop watcher answers
/// [`TraySupport::Unavailable`].
pub fn install_tray(tray: &Tray, wake: impl Fn() + Send + Sync + 'static) -> TraySupport {
    install_tray_with_outcome(tray, wake).support
}

/// Shows `tray` and returns both the backend kind and any unavailability reason.
pub fn install_tray_with_outcome(
    tray: &Tray,
    wake: impl Fn() + Send + Sync + 'static,
) -> TrayOutcome {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = Some(Arc::new(wake));
    }
    #[cfg(target_os = "macos")]
    {
        match macos::install(tray) {
            Ok(()) => {
                return TrayOutcome {
                    support: TraySupport::MenuBarExtra,
                    unavailable_reason: None,
                };
            }
            Err(reason) => {
                clear_wake();
                return TrayOutcome {
                    support: TraySupport::Unavailable,
                    unavailable_reason: Some(reason),
                };
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        match windows::install(tray) {
            Ok(()) => {
                return TrayOutcome {
                    support: TraySupport::NotificationArea,
                    unavailable_reason: None,
                };
            }
            Err(reason) => {
                clear_wake();
                return TrayOutcome {
                    support: TraySupport::Unavailable,
                    unavailable_reason: Some(reason),
                };
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        match linux::install(tray) {
            Ok(()) => {
                return TrayOutcome {
                    support: TraySupport::StatusNotifierItem,
                    unavailable_reason: None,
                };
            }
            Err(reason) => {
                clear_wake();
                return TrayOutcome {
                    support: TraySupport::Unavailable,
                    unavailable_reason: Some(reason),
                };
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = tray;
        clear_wake();
        TrayOutcome {
            support: TraySupport::Unavailable,
            unavailable_reason: Some("system tray is unsupported on this target".into()),
        }
    }
}

fn clear_wake() {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = None;
    }
}

/// Updates the attention state of an already-installed tray icon in place.
///
/// Returns `false` if no tray is currently installed or the platform rejects
/// the update.
pub fn set_tray_attention(attention: TrayAttention) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::set_attention(attention)
    }
    #[cfg(target_os = "windows")]
    {
        windows::set_attention(attention)
    }
    #[cfg(target_os = "linux")]
    {
        linux::set_attention(attention)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = attention;
        false
    }
}

/// The tray the platform actually holds: its tooltip and the titles of its
/// menu's entries (`-` for a separator).
///
/// A verification entry, like `installed_menu_bar`. `None` when no tray is
/// shown or the platform cannot be asked. On Windows the shell only confirms
/// it still places the icon; the tooltip and titles are what was installed.
pub fn installed_tray() -> Option<(String, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        macos::installed()
    }
    #[cfg(target_os = "windows")]
    {
        windows::installed()
    }
    #[cfg(target_os = "linux")]
    {
        linux::installed()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Takes the tray away. Does nothing when none is shown.
pub fn remove_tray() {
    #[cfg(target_os = "macos")]
    macos::remove();
    #[cfg(target_os = "windows")]
    windows::remove();
    #[cfg(target_os = "linux")]
    linux::remove();
    clear_wake();
}

/// Maximum accepted icon edge length when decoding PNG tray icons.
const MAX_TRAY_ICON_EDGE: usize = 256;

/// Decodes an encoded PNG into `(width, height, rgba8)` bytes.
pub(crate) fn decode_png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    // A tray icon is small; this bounds what a malformed file can allocate.
    decoder.set_limits(png::Limits { bytes: 4 << 20 });
    let mut reader = decoder.read_info().ok()?;
    let mut raw = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut raw).ok()?;
    let (width, height) = (frame.width as usize, frame.height as usize);
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None,
    };
    if width == 0 || height == 0 || width > MAX_TRAY_ICON_EDGE || height > MAX_TRAY_ICON_EDGE {
        return None;
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in raw[..width * height * channels].chunks_exact(channels) {
        let (r, g, b, a) = match *pixel {
            [r, g, b, a] => (r, g, b, a),
            [r, g, b] => (r, g, b, 255),
            [l, a] => (l, l, l, a),
            [l] => (l, l, l, 255),
            _ => return None,
        };
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    Some((frame.width, frame.height, rgba))
}

/// Encodes straight RGBA8 pixels back into PNG bytes.
pub(crate) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// Produces a monochrome white (`light_foreground = true`) or black
/// (`light_foreground = false`) PNG preserving the source PNG's alpha channel.
pub(crate) fn monochrome_mask_png(bytes: &[u8], light_foreground: bool) -> Option<Vec<u8>> {
    let (width, height, mut rgba) = decode_png_rgba(bytes)?;
    let channel = if light_foreground { 255 } else { 0 };
    for px in rgba.chunks_exact_mut(4) {
        px[0] = channel;
        px[1] = channel;
        px[2] = channel;
    }
    encode_rgba_png(width, height, &rgba)
}

/// Selects the PNG bytes appropriate for a light (`light_surface = true`, dark
/// glyph) or dark (`light_surface = false`, light glyph) tray surface.
pub(crate) fn select_tray_icon_bytes<'a>(tray: &'a Tray, light_surface: bool) -> Cow<'a, [u8]> {
    if light_surface {
        if let Some(bytes) = tray.monochrome_dark_icon_png.as_deref()
            && !bytes.is_empty()
        {
            return Cow::Borrowed(bytes);
        }
        if tray.template_icon
            && let Some(masked) = monochrome_mask_png(&tray.icon_png, false)
        {
            return Cow::Owned(masked);
        }
    } else {
        if let Some(bytes) = tray.monochrome_light_icon_png.as_deref()
            && !bytes.is_empty()
        {
            return Cow::Borrowed(bytes);
        }
        if tray.template_icon
            && let Some(masked) = monochrome_mask_png(&tray.icon_png, true)
        {
            return Cow::Owned(masked);
        }
    }
    Cow::Borrowed(&tray.icon_png)
}

/// Resizes an RGBA buffer to `target_edge x target_edge` with area averaging
/// when downsizing or nearest-neighbor when upscaling.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn resize_rgba_square(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_edge: u32,
) -> (u32, u32, Vec<u8>) {
    if target_edge == 0 || (width == target_edge && height == target_edge) {
        return (width, height, rgba.to_vec());
    }
    let tw = target_edge as usize;
    let th = target_edge as usize;
    let sw = width as usize;
    let sh = height as usize;
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let y0 = ty * sh / th;
        let y1 = ((ty + 1) * sh).div_ceil(th).max(y0 + 1).min(sh);
        for tx in 0..tw {
            let x0 = tx * sw / tw;
            let x1 = ((tx + 1) * sw).div_ceil(tw).max(x0 + 1).min(sw);
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let idx = (sy * sw + sx) * 4;
                    for c in 0..4 {
                        sum[c] += u32::from(rgba[idx + c]);
                    }
                    count += 1;
                }
            }
            let dst = (ty * tw + tx) * 4;
            let count = count.max(1);
            for c in 0..4 {
                out[dst + c] = (sum[c] / count) as u8;
            }
        }
    }
    (target_edge, target_edge, out)
}

/// Converts RGBA8 pixels into the network-byte-order ARGB32 payload expected
/// by `org.kde.StatusNotifierItem.IconPixmap`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn rgba_to_sni_argb32(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    out
}

/// Formats readable summaries of [`crate::MenuEntry`] items (`-` for separators)
/// for [`installed_tray`] verification on backends that store a structured model.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(dead_code))]
pub(crate) fn summarize_menu_entries(entries: &[crate::MenuEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            crate::MenuEntry::Separator => "-".to_owned(),
            crate::MenuEntry::Item { label, .. } => label.clone(),
            crate::MenuEntry::Submenu(menu) => menu.title.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn sample_png_bytes() -> Vec<u8> {
        let rgba = [
            12, 34, 56, 255, // opaque
            90, 10, 20, 128, // half
            200, 100, 50, 0, // transparent
            1, 2, 3, 255, // opaque
        ];
        encode_rgba_png(2, 2, &rgba).expect("sample PNG encodes cleanly")
    }

    #[test]
    fn a_tray_choice_runs_the_installed_wake_and_removal_drops_it() {
        let _serial = crate::test_serial_lock();
        static WOKEN: AtomicUsize = AtomicUsize::new(0);
        if let Ok(mut slot) = WAKE.lock() {
            *slot = Some(Arc::new(|| {
                WOKEN.fetch_add(1, Ordering::SeqCst);
            }));
        }
        notify();
        assert_eq!(WOKEN.load(Ordering::SeqCst), 1);
        remove_tray();
        notify();
        assert_eq!(WOKEN.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn monochrome_variants_select_explicit_or_synthesize_from_template() {
        let base = sample_png_bytes();
        let explicit_dark = encode_rgba_png(1, 1, &[5, 6, 7, 255]).unwrap();
        let explicit_light = encode_rgba_png(1, 1, &[245, 246, 247, 255]).unwrap();

        let tray = Tray {
            tooltip: "Nana".into(),
            icon_png: base.clone(),
            monochrome_light_icon_png: Some(explicit_light.clone()),
            monochrome_dark_icon_png: Some(explicit_dark.clone()),
            template_icon: false,
            ..Default::default()
        };
        assert_eq!(
            select_tray_icon_bytes(&tray, true).as_ref(),
            explicit_dark.as_slice()
        );
        assert_eq!(
            select_tray_icon_bytes(&tray, false).as_ref(),
            explicit_light.as_slice()
        );

        let templated = Tray {
            tooltip: "Template".into(),
            icon_png: base.clone(),
            template_icon: true,
            ..Default::default()
        };
        let dark_bytes = select_tray_icon_bytes(&templated, true);
        let (_, _, dark_rgba) = decode_png_rgba(dark_bytes.as_ref()).unwrap();
        assert_eq!(&dark_rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&dark_rgba[4..8], &[0, 0, 0, 128]);
        assert_eq!(&dark_rgba[8..12], &[0, 0, 0, 0]);

        let light_bytes = select_tray_icon_bytes(&templated, false);
        let (_, _, light_rgba) = decode_png_rgba(light_bytes.as_ref()).unwrap();
        assert_eq!(&light_rgba[0..4], &[255, 255, 255, 255]);
        assert_eq!(&light_rgba[4..8], &[255, 255, 255, 128]);
    }

    #[test]
    fn argb32_and_resize_helpers_preserve_pixel_layout() {
        let argb = rgba_to_sni_argb32(&[10, 20, 30, 40, 1, 2, 3, 255]);
        assert_eq!(argb, vec![40, 10, 20, 30, 255, 1, 2, 3]);

        let src = vec![200u8; 4 * 4 * 4];
        let (w, h, resized) = resize_rgba_square(4, 4, &src, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(resized.len(), 16);
        assert!(resized.iter().all(|&b| b == 200));
    }

    #[test]
    fn attention_maps_to_expected_sni_strings() {
        assert_eq!(TrayAttention::Active.as_sni_status(), "Active");
        assert_eq!(TrayAttention::Passive.as_sni_status(), "Passive");
        assert_eq!(
            TrayAttention::NeedsAttention.as_sni_status(),
            "NeedsAttention"
        );
    }

    #[test]
    fn set_tray_attention_returns_false_when_not_installed() {
        let _serial = crate::test_serial_lock();
        remove_tray();
        assert!(!set_tray_attention(TrayAttention::NeedsAttention));
    }
}
