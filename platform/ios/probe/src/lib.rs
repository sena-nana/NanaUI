//! The measurement page's iOS staticlib (`include/nana_ios_probe.h`): the
//! generic host from `nana-ios-host` around `nana-mobile-probe`, plus the
//! calls the probe shell makes for its camera layer and counters.

use nana_ios_host::{
    FrameworkError, Haptic, HostParts, IosApp, NanaIosHost, NanaRect, Shell, export_ios_app,
    with_app,
};
use nana_mobile_host::{MobileRenderer, MobileRuntime};
use nana_mobile_probe::{PreviewUploader, Probe, ProbeState};

#[derive(Default)]
struct ProbeApp {
    page: Option<Probe>,
    preview: PreviewUploader,
}

impl IosApp for ProbeApp {
    fn mount(&mut self, runtime: &mut MobileRuntime, shell: Shell) -> Result<(), FrameworkError> {
        let state = ProbeState::with_press_feedback(move || shell.haptic(Haptic::Light));
        self.page = Some(Probe::mount(runtime, state)?);
        Ok(())
    }

    fn before_frame(&mut self, _runtime: &mut MobileRuntime, renderer: &mut MobileRenderer) {
        if let Some(page) = &self.page {
            page.sync_preview(&mut self.preview, renderer.host_textures());
        }
    }
}

export_ios_app!(ProbeApp::default());

/// The mounted page and the rest of the host, for one call.
///
/// # Safety
///
/// `host` is null or a live host, used on the main thread.
unsafe fn page<R>(
    host: *mut NanaIosHost,
    f: impl FnOnce(&Probe, &mut PreviewUploader, &mut MobileRuntime, &mut MobileRenderer) -> R,
) -> Option<R> {
    // SAFETY: forwarded from the caller.
    unsafe {
        with_app::<ProbeApp, _>(
            host,
            |HostParts {
                 app,
                 runtime,
                 renderer,
             }| {
                let ProbeApp { page, preview } = app;
                page.as_ref()
                    .map(|page| f(page, preview, runtime, renderer))
            },
        )
    }
    .flatten()
}

/// Where the preview area is, logical px of the surface.
///
/// # Safety
///
/// See [`page`]; `rect` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_probe_preview_rect(
    host: *mut NanaIosHost,
    rect: *mut NanaRect,
) -> bool {
    // SAFETY: forwarded from the caller.
    let area = unsafe { page(host, |page, _, runtime, _| page.preview_rect(runtime)) }.flatten();
    let Some(area) = area else {
        return false;
    };
    if rect.is_null() {
        return false;
    }
    // SAFETY: the caller passes a writable rect.
    unsafe {
        *rect = NanaRect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        };
    }
    true
}

/// # Safety
///
/// See [`page`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_probe_cpu_preview(host: *mut NanaIosHost) -> bool {
    // SAFETY: forwarded from the caller.
    unsafe { page(host, |page, _, _, _| page.state.cpu_preview()) }.unwrap_or(false)
}

/// Upload one BGRA camera frame for the CPU preview path.
///
/// # Safety
///
/// See [`page`]; `bytes` holds `bytes_per_row * height` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_probe_push_preview_bgra(
    host: *mut NanaIosHost,
    bytes: *const u8,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) {
    if bytes.is_null() {
        return;
    }
    // SAFETY: forwarded from the caller.
    let bytes =
        unsafe { std::slice::from_raw_parts(bytes, (bytes_per_row as usize) * height as usize) };
    // SAFETY: forwarded from the caller.
    unsafe {
        page(host, |page, preview, runtime, renderer| {
            if !page.state.cpu_preview() {
                return;
            }
            let uploaded = preview.push_bgra(
                renderer.gpu(),
                renderer.host_textures(),
                bytes,
                width,
                height,
                bytes_per_row,
            );
            if uploaded.is_ok() {
                runtime.request_present();
            }
        })
    };
}

/// # Safety
///
/// See [`page`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nana_probe_presses(host: *mut NanaIosHost) -> u32 {
    // SAFETY: forwarded from the caller.
    unsafe { page(host, |page, _, _, _| page.state.presses()) }.unwrap_or(0)
}
