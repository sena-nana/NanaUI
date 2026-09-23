//! Early Splash (Issue #225): an embedded logo, and at most one animation
//! preset the platform compositor runs by itself, shown on the application's
//! own window before the GPU device and the Nana runtime exist.
//!
//! This is deliberately not a UI: no text, no layout, no input, no per-frame
//! callback. The host owns one [`NativeSplash`] per startup and removes it once
//! the application's first requested frame is on the compositor; every layer,
//! visual, device and hook it created goes with it.
//!
//! | Platform | Surface | Animation | Handoff |
//! |---|---|---|---|
//! | macOS | a `CALayer` above the view's Metal sublayer | `CABasicAnimation`, run by the render server | [`SplashHandoff::SameTransaction`] |
//! | Windows, plain HWND | a topmost DirectComposition visual | `IDCompositionAnimation`, run by DWM | [`SplashHandoff::AfterCompositorFlush`] |
//! | everything else | none | — | — |

use raw_window_handle::HasWindowHandle;

use crate::material::FallbackColor;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Largest encoded logo accepted, matching the packager's `early-splash` pack
/// limit: the logo is read before anything else of the application is.
pub const MAX_LOGO_ENCODED_BYTES: usize = 1024 * 1024;
/// Longest logo edge, in pixels.
pub const MAX_LOGO_EDGE: u32 = 1024;
/// Largest logo once decoded to 32-bit pixels.
pub const MAX_LOGO_DECODED_BYTES: usize = 4 * 1024 * 1024;

/// A PNG compiled into the binary. Nothing is fetched, scanned or resolved to
/// show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplashLogo {
    png: &'static [u8],
}

impl SplashLogo {
    pub const fn png(bytes: &'static [u8]) -> Self {
        Self { png: bytes }
    }

    pub const fn bytes(self) -> &'static [u8] {
        self.png
    }

    /// Checks the PNG signature and header against the limits without
    /// decoding any pixels.
    pub fn validate(self) -> Result<LogoInfo, SplashLogoError> {
        validate_png(self.png)
    }
}

/// Pixel size read from a logo's PNG header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplashLogoError {
    TooLarge { bytes: usize },
    NotPng,
    ZeroSize,
    EdgeTooLong { width: u32, height: u32 },
    DecodedTooLarge { bytes: usize },
    Decode(String),
}

impl std::fmt::Display for SplashLogoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "logo is {bytes} bytes; the limit is {MAX_LOGO_ENCODED_BYTES}"
            ),
            Self::NotPng => f.write_str("logo is not a PNG"),
            Self::ZeroSize => f.write_str("logo has no pixels"),
            Self::EdgeTooLong { width, height } => write!(
                f,
                "logo is {width}x{height}; the longest edge allowed is {MAX_LOGO_EDGE}"
            ),
            Self::DecodedTooLarge { bytes } => write!(
                f,
                "logo decodes to {bytes} bytes; the limit is {MAX_LOGO_DECODED_BYTES}"
            ),
            Self::Decode(reason) => write!(f, "logo could not be decoded: {reason}"),
        }
    }
}

impl std::error::Error for SplashLogoError {}

fn validate_png(bytes: &[u8]) -> Result<LogoInfo, SplashLogoError> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if bytes.len() > MAX_LOGO_ENCODED_BYTES {
        return Err(SplashLogoError::TooLarge { bytes: bytes.len() });
    }
    // Signature, then the IHDR chunk: length, type, width, height.
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(SplashLogoError::NotPng);
    }
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let (width, height) = (word(16), word(20));
    if width == 0 || height == 0 {
        return Err(SplashLogoError::ZeroSize);
    }
    if width > MAX_LOGO_EDGE || height > MAX_LOGO_EDGE {
        return Err(SplashLogoError::EdgeTooLong { width, height });
    }
    let decoded = width as usize * height as usize * 4;
    if decoded > MAX_LOGO_DECODED_BYTES {
        return Err(SplashLogoError::DecodedTooLarge { bytes: decoded });
    }
    Ok(LogoInfo { width, height })
}

/// Decodes a logo that passed [`validate_png`] into premultiplied BGRA rows,
/// for compositors that take pixels rather than an encoded image.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn decode_premultiplied_bgra(
    bytes: &[u8],
    info: LogoInfo,
) -> Result<Vec<u8>, SplashLogoError> {
    let decode = |error: png::DecodingError| SplashLogoError::Decode(error.to_string());
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    // The header was checked; this bounds what a lying body can allocate.
    decoder.set_limits(png::Limits {
        bytes: MAX_LOGO_DECODED_BYTES * 2,
    });
    let mut reader = decoder.read_info().map_err(decode)?;
    let size = reader
        .output_buffer_size()
        .ok_or(SplashLogoError::DecodedTooLarge { bytes: usize::MAX })?;
    let mut raw = vec![0; size];
    let frame = reader.next_frame(&mut raw).map_err(decode)?;
    if (frame.width, frame.height) != (info.width, info.height) {
        return Err(SplashLogoError::Decode(
            "frame size differs from the header".into(),
        ));
    }
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(SplashLogoError::Decode("palette was not expanded".into()));
        }
    };
    let (width, height) = (frame.width as usize, frame.height as usize);
    let mut out = Vec::with_capacity(width * height * 4);
    for row in raw.chunks(frame.line_size).take(height) {
        for pixel in row.chunks_exact(channels).take(width) {
            let (r, g, b, a) = match channels {
                1 => (pixel[0], pixel[0], pixel[0], 255),
                2 => (pixel[0], pixel[0], pixel[0], pixel[1]),
                3 => (pixel[0], pixel[1], pixel[2], 255),
                _ => (pixel[0], pixel[1], pixel[2], pixel[3]),
            };
            let premultiply = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
            out.extend_from_slice(&[premultiply(b), premultiply(g), premultiply(r), a]);
        }
    }
    Ok(out)
}

/// What fills the window behind the logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplashBackground {
    /// The colour the host's default theme uses for the current system
    /// appearance, so the first real frame does not change the backdrop.
    #[default]
    System,
    Color(FallbackColor),
    /// Only the logo is drawn. Meant for transparent windows; an opaque window
    /// shows whatever the platform has under it.
    Transparent,
}

/// A preset the compositor runs on its own. None of them calls back into the
/// application, and none is advanced by a timer in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplashAnimation {
    /// A static logo.
    None,
    /// Fades the logo in once, then holds it.
    #[default]
    FadeIn,
    /// Breathes the logo's opacity until handoff.
    Pulse,
    /// Spins the logo until handoff.
    Rotate,
}

impl SplashAnimation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FadeIn => "fade-in",
            Self::Pulse => "pulse",
            Self::Rotate => "rotate",
        }
    }
}

/// The whole of what an application can say about its Early Splash.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplashSpec {
    pub logo: SplashLogo,
    /// Logo box in logical points, centred in the window. The logo keeps its
    /// aspect ratio inside it.
    pub logo_size: (f32, f32),
    pub background: SplashBackground,
    pub animation: SplashAnimation,
}

impl SplashSpec {
    pub const fn new(logo: SplashLogo) -> Self {
        Self {
            logo,
            logo_size: (128.0, 128.0),
            background: SplashBackground::System,
            animation: SplashAnimation::FadeIn,
        }
    }

    pub const fn with_logo_size(mut self, width: f32, height: f32) -> Self {
        self.logo_size = (width, height);
        self
    }

    pub const fn with_background(mut self, background: SplashBackground) -> Self {
        self.background = background;
        self
    }

    pub const fn with_animation(mut self, animation: SplashAnimation) -> Self {
        self.animation = animation;
        self
    }

    /// Logo box clamped to something a window can show.
    pub fn clamped_logo_size(&self) -> (f64, f64) {
        let clamp = |value: f32| {
            if value.is_finite() {
                f64::from(value).clamp(1.0, 4096.0)
            } else {
                128.0
            }
        };
        (clamp(self.logo_size.0), clamp(self.logo_size.1))
    }
}

/// Why a requested animation is not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashStaticReason {
    /// The user asked the system to reduce motion.
    ReducedMotion,
    /// The compositor refused the animation; the logo is shown still.
    NativeAnimationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashAnimationOutcome {
    /// The preset is running in the platform compositor.
    Applied(SplashAnimation),
    /// The logo is shown still.
    Static {
        requested: SplashAnimation,
        reason: Option<SplashStaticReason>,
    },
}

/// Why no splash was created. Nothing was allocated for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashSkip {
    NotConfigured,
    /// The window starts hidden (for example a tray start); a splash would
    /// force it on screen.
    HiddenStart,
    /// An embedding host owns the event loop and the device.
    Embedded,
    /// This platform has no native splash path.
    PlatformUnsupported,
    /// The window presents through a platform compositor tree of NanaUI's own
    /// (Windows `WS_EX_NOREDIRECTIONBITMAP`), which has no topmost slot left.
    CompositionTarget,
}

impl SplashSkip {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotConfigured => "not configured",
            Self::HiddenStart => "window starts hidden",
            Self::Embedded => "embedded host",
            Self::PlatformUnsupported => "platform has no native splash",
            Self::CompositionTarget => "window presents through a composition tree",
        }
    }
}

/// Why a configured splash could not be shown. The application starts anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplashFailure {
    Logo(SplashLogoError),
    Native(String),
}

impl std::fmt::Display for SplashFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Logo(error) => error.fmt(f),
            Self::Native(reason) => write!(f, "native splash failed: {reason}"),
        }
    }
}

/// What the platform actually did with a splash request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplashOutcome {
    Shown { animation: SplashAnimationOutcome },
    Skipped(SplashSkip),
    Failed(SplashFailure),
}

impl SplashOutcome {
    pub const fn is_shown(&self) -> bool {
        matches!(self, Self::Shown { .. })
    }

    /// Stable code for diagnostics: 0 shown and animated, 1 shown static,
    /// 2 skipped, 3 failed.
    pub const fn code(&self) -> u64 {
        match self {
            Self::Shown {
                animation: SplashAnimationOutcome::Applied(_),
            } => 0,
            Self::Shown { .. } => 1,
            Self::Skipped(_) => 2,
            Self::Failed(_) => 3,
        }
    }
}

/// How a splash leaves the screen without uncovering an empty window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplashHandoff {
    /// Present the target frame with the window's Metal layer in transaction
    /// mode, then [`NativeSplash::remove`] before the turn ends: the drawable
    /// and the removal land in one Core Animation commit.
    SameTransaction,
    /// Present the target frame, wait until its GPU work has completed, then
    /// [`NativeSplash::remove`], which waits for one compositor pass (so the
    /// presented buffer has been latched) before removing the visual.
    AfterCompositorFlush,
}

/// Work a splash has done, for the structural gates: none of it may grow with
/// the number of animation frames, and all of it is released at handoff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplashWork {
    /// Times the logo was decoded.
    pub logo_decodes: usize,
    /// Times decoded logo pixels were handed to the compositor.
    pub logo_uploads: usize,
    /// Animations handed to the compositor. One per preset, however long it runs.
    pub animation_submissions: usize,
    /// Compositor transactions this splash published.
    pub commits: usize,
    /// Native objects (layers, visuals, surfaces, devices, hooks) still alive.
    pub live_resources: usize,
}

/// The one owner of every native object an Early Splash creates. Dropping it
/// removes and releases all of them.
pub struct NativeSplash {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    inner: Option<platform::Splash>,
    work: SplashWork,
}

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

impl NativeSplash {
    /// Whether this platform can show a splash at all.
    pub const fn platform_supported() -> bool {
        cfg!(any(target_os = "macos", target_os = "windows"))
    }

    /// Puts the splash over `window`'s client area. Call before the window is
    /// first shown, on the thread that owns it.
    ///
    /// `system_background` is what [`SplashBackground::System`] resolves to;
    /// `reduced_motion` turns every preset into a still logo.
    pub fn show<W: HasWindowHandle + ?Sized>(
        window: &W,
        spec: &SplashSpec,
        system_background: FallbackColor,
        reduced_motion: bool,
    ) -> (Option<Self>, SplashOutcome) {
        let info = match spec.logo.validate() {
            Ok(info) => info,
            Err(error) => return (None, SplashOutcome::Failed(SplashFailure::Logo(error))),
        };
        let background = match spec.background {
            SplashBackground::System => Some(system_background),
            SplashBackground::Color(color) => Some(color),
            SplashBackground::Transparent => None,
        };
        let animation = if reduced_motion {
            SplashAnimation::None
        } else {
            spec.animation
        };
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let request = platform::Request {
                png: spec.logo.bytes(),
                info,
                logo_size: spec.clamped_logo_size(),
                background,
                animation,
            };
            let mut work = SplashWork::default();
            match platform::Splash::show(window, &request, &mut work) {
                Ok((splash, animated)) => {
                    let outcome = animation_outcome(spec.animation, reduced_motion, animated);
                    work.live_resources = splash.live_resources();
                    (
                        Some(Self {
                            inner: Some(splash),
                            work,
                        }),
                        SplashOutcome::Shown { animation: outcome },
                    )
                }
                Err(failure) => (None, SplashOutcome::Failed(failure)),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (window, info, background, animation);
            (
                None,
                SplashOutcome::Skipped(SplashSkip::PlatformUnsupported),
            )
        }
    }

    /// The window moved to a display with another backing scale. macOS
    /// re-renders the logo's layer contents for it; Windows follows
    /// `WM_DPICHANGED` by itself.
    pub fn set_scale_factor(&mut self, scale: f64) {
        #[cfg(target_os = "macos")]
        if let Some(splash) = self.inner.as_ref()
            && scale.is_finite()
            && scale > 0.0
        {
            splash.set_scale_factor(scale, &mut self.work);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = scale;
    }

    pub const fn handoff(&self) -> SplashHandoff {
        if cfg!(target_os = "windows") {
            SplashHandoff::AfterCompositorFlush
        } else {
            SplashHandoff::SameTransaction
        }
    }

    /// Takes the splash off the window as the handoff and releases everything
    /// it created. See [`SplashHandoff`] for when this is safe to call.
    pub fn remove(mut self) -> SplashWork {
        self.release(true);
        self.work
    }

    /// Takes the splash off without waiting for the compositor — for a window
    /// that is closing, where there is no frame to wait for. Dropping a
    /// splash does the same.
    pub fn discard(mut self) -> SplashWork {
        self.release(false);
        self.work
    }

    pub const fn work(&self) -> SplashWork {
        self.work
    }

    fn release(&mut self, handoff: bool) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(splash) = self.inner.take() {
            splash.remove(&mut self.work, handoff);
            self.work.live_resources = 0;
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = handoff;
    }
}

impl Drop for NativeSplash {
    fn drop(&mut self) {
        self.release(false);
    }
}

impl std::fmt::Debug for NativeSplash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSplash")
            .field("work", &self.work)
            .finish_non_exhaustive()
    }
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn animation_outcome(
    requested: SplashAnimation,
    reduced_motion: bool,
    animated: bool,
) -> SplashAnimationOutcome {
    if requested == SplashAnimation::None {
        return SplashAnimationOutcome::Static {
            requested,
            reason: None,
        };
    }
    if reduced_motion {
        return SplashAnimationOutcome::Static {
            requested,
            reason: Some(SplashStaticReason::ReducedMotion),
        };
    }
    if animated {
        SplashAnimationOutcome::Applied(requested)
    } else {
        SplashAnimationOutcome::Static {
            requested,
            reason: Some(SplashStaticReason::NativeAnimationFailed),
        }
    }
}

/// Logo box inside a window's client area, in the same units as `client`:
/// centred, aspect-fit into `logo_box`, never larger than the client itself.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn fit_logo(
    client: (f64, f64),
    logo_box: (f64, f64),
    image: (u32, u32),
) -> (f64, f64, f64, f64) {
    let (image_w, image_h) = (f64::from(image.0.max(1)), f64::from(image.1.max(1)));
    let box_w = logo_box.0.min(client.0.max(0.0));
    let box_h = logo_box.1.min(client.1.max(0.0));
    let scale = (box_w / image_w).min(box_h / image_h).max(0.0);
    let (width, height) = (image_w * scale, image_h * scale);
    (
        (client.0 - width) / 2.0,
        (client.1 - height) / 2.0,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn header_is_read_without_decoding() {
        assert_eq!(
            validate_png(&png_header(256, 128)),
            Ok(LogoInfo {
                width: 256,
                height: 128
            })
        );
    }

    #[test]
    fn limits_reject_before_any_pixel_is_touched() {
        assert_eq!(validate_png(b"GIF89a"), Err(SplashLogoError::NotPng));
        assert_eq!(
            validate_png(&png_header(0, 4)),
            Err(SplashLogoError::ZeroSize)
        );
        assert_eq!(
            validate_png(&png_header(MAX_LOGO_EDGE + 1, 4)),
            Err(SplashLogoError::EdgeTooLong {
                width: MAX_LOGO_EDGE + 1,
                height: 4
            })
        );
        let mut huge = png_header(4, 4);
        huge.resize(MAX_LOGO_ENCODED_BYTES + 1, 0);
        assert_eq!(
            validate_png(&huge),
            Err(SplashLogoError::TooLarge {
                bytes: MAX_LOGO_ENCODED_BYTES + 1
            })
        );
    }

    #[test]
    fn the_edge_limit_keeps_decoded_pixels_inside_the_byte_limit() {
        // 1024² RGBA is exactly 4 MiB, so the edge check is what binds.
        assert_eq!(
            MAX_LOGO_EDGE as usize * MAX_LOGO_EDGE as usize * 4,
            MAX_LOGO_DECODED_BYTES
        );
        assert!(validate_png(&png_header(MAX_LOGO_EDGE, MAX_LOGO_EDGE)).is_ok());
    }

    #[test]
    fn reduced_motion_and_failures_report_a_still_logo_with_the_reason() {
        use SplashAnimation::*;
        assert_eq!(
            animation_outcome(Rotate, false, true),
            SplashAnimationOutcome::Applied(Rotate)
        );
        assert_eq!(
            animation_outcome(Rotate, true, false),
            SplashAnimationOutcome::Static {
                requested: Rotate,
                reason: Some(SplashStaticReason::ReducedMotion)
            }
        );
        assert_eq!(
            animation_outcome(Pulse, false, false),
            SplashAnimationOutcome::Static {
                requested: Pulse,
                reason: Some(SplashStaticReason::NativeAnimationFailed)
            }
        );
        assert_eq!(
            animation_outcome(None, false, false),
            SplashAnimationOutcome::Static {
                requested: None,
                reason: Option::None
            }
        );
    }

    #[test]
    fn decoding_premultiplies_into_bgra() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 128])
                .unwrap();
        }
        let info = validate_png(&encoded).unwrap();
        assert_eq!(
            decode_premultiplied_bgra(&encoded, info).unwrap(),
            vec![0, 0, 255, 255, 0, 128, 0, 128]
        );
    }

    #[test]
    fn logo_is_centred_and_aspect_fit_into_its_box_and_the_client() {
        assert_eq!(
            fit_logo((400.0, 300.0), (100.0, 100.0), (200, 100)),
            (150.0, 125.0, 100.0, 50.0)
        );
        // A client smaller than the box shrinks the logo instead of clipping it.
        assert_eq!(
            fit_logo((50.0, 50.0), (100.0, 100.0), (10, 10)),
            (0.0, 0.0, 50.0, 50.0)
        );
    }
}
