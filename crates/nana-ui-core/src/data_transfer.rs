//! Unified payload contract shared by clipboard and drag-and-drop.
//!
//! A [`DataTransfer`] carries zero or more MIME-keyed formats (eager bytes or
//! lazy providers) plus an optional process-local typed internal payload.
//! Metadata queries (`formats`, `has_format`, `entry_meta`) never trigger lazy
//! providers or read files off disk.

use std::any::Any;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::drag::DropKind;

/// Maximum length of a validated MIME type string.
pub const MAX_MIME_LEN: usize = 128;

/// Default maximum payload byte size enforced when materializing formats.
pub const DEFAULT_MAX_TRANSFER_BYTES: usize = 64 * 1024 * 1024;

const FORBIDDEN_MIME_TOKENS: [&str; 5] = ["raw-pointer", "os-handle", "hwnd", "nsview", "xwindow"];

/// Why a MIME format string was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataFormatError {
    Empty,
    TooLong,
    InvalidSyntax,
    ForbiddenFormat,
}

impl fmt::Display for DataFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("MIME format cannot be empty"),
            Self::TooLong => write!(f, "MIME format exceeds {MAX_MIME_LEN} bytes"),
            Self::InvalidSyntax => f.write_str("invalid MIME format syntax"),
            Self::ForbiddenFormat => {
                f.write_str("MIME format refers to a forbidden raw pointer or OS handle")
            }
        }
    }
}

impl std::error::Error for DataFormatError {}

/// Canonical MIME format identifier for clipboard and drag-and-drop payloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataFormat {
    TextPlain,
    TextHtml,
    TextUriList,
    ImagePng,
    ImageRgba8,
    OctetStream,
    Custom(Arc<str>),
}

impl DataFormat {
    /// Maximum MIME string length accepted by [`Self::parse`].
    pub const MAX_MIME_LEN: usize = MAX_MIME_LEN;

    /// Parse and validate a MIME type string, canonicalizing well-known formats.
    ///
    /// Rejects empty strings, lengths above [`MAX_MIME_LEN`], whitespace,
    /// non-ASCII or control characters, parameters (`;...`), invalid RFC 6838
    /// token characters, and raw-handle/pointer keywords.
    pub fn parse(mime: &str) -> Result<Self, DataFormatError> {
        if mime.is_empty() {
            return Err(DataFormatError::Empty);
        }
        if mime.len() > MAX_MIME_LEN {
            return Err(DataFormatError::TooLong);
        }
        if mime
            .bytes()
            .any(|b| !b.is_ascii() || b.is_ascii_control() || b.is_ascii_whitespace())
        {
            return Err(DataFormatError::InvalidSyntax);
        }
        if mime.contains(';') {
            return Err(DataFormatError::InvalidSyntax);
        }
        let Some((ty, sub)) = mime.split_once('/') else {
            return Err(DataFormatError::InvalidSyntax);
        };
        if ty.is_empty() || sub.is_empty() || sub.contains('/') {
            return Err(DataFormatError::InvalidSyntax);
        }
        if !is_valid_mime_token(ty) || !is_valid_mime_token(sub) {
            return Err(DataFormatError::InvalidSyntax);
        }
        let normalized = format!("{}/{}", ty.to_ascii_lowercase(), sub.to_ascii_lowercase());
        if FORBIDDEN_MIME_TOKENS
            .iter()
            .any(|forbidden| normalized.contains(forbidden))
        {
            return Err(DataFormatError::ForbiddenFormat);
        }
        Ok(match normalized.as_str() {
            "text/plain" => Self::TextPlain,
            "text/html" => Self::TextHtml,
            "text/uri-list" => Self::TextUriList,
            "image/png" => Self::ImagePng,
            "image/rgba8" => Self::ImageRgba8,
            "application/octet-stream" => Self::OctetStream,
            _ => Self::Custom(Arc::from(normalized)),
        })
    }

    /// Validate a custom MIME type string (e.g. `application/x-nana-node`).
    pub fn custom(mime: &str) -> Result<Self, DataFormatError> {
        Self::parse(mime)
    }

    /// Canonical lowercase MIME string.
    pub fn as_mime(&self) -> &str {
        match self {
            Self::TextPlain => "text/plain",
            Self::TextHtml => "text/html",
            Self::TextUriList => "text/uri-list",
            Self::ImagePng => "image/png",
            Self::ImageRgba8 => "image/rgba8",
            Self::OctetStream => "application/octet-stream",
            Self::Custom(mime) => mime.as_ref(),
        }
    }

    /// Whether this is a non-well-known custom MIME format.
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom(_))
    }
}

impl fmt::Display for DataFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_mime())
    }
}

fn is_valid_mime_token(part: &str) -> bool {
    let bytes = part.as_bytes();
    let Some((&first, _)) = bytes.split_first() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'_'))
}

/// Whether a [`DataTransfer`] originated inside this process or from the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferOrigin {
    Internal,
    External,
}

/// Caching policy for a lazy [`TransferProvider`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TransferCachePolicy {
    /// Materialize once and reuse the cached bytes on subsequent reads.
    #[default]
    Cache,
    /// Invoke the provider on every read without retaining a copy.
    NeverCache,
}

/// Failure when reading, validating, or attaching a transfer payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    InvalidFormat(DataFormatError),
    FormatNotAvailable,
    PayloadTooLarge { size: usize, max_bytes: usize },
    Cancelled,
    SourceDestroyed,
    ProviderFailed(Arc<str>),
    InvalidUtf8,
    InvalidImageData,
    ExternalInternalPayloadForbidden,
}

impl From<DataFormatError> for TransferError {
    fn from(err: DataFormatError) -> Self {
        Self::InvalidFormat(err)
    }
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(err) => write!(f, "invalid format: {err}"),
            Self::FormatNotAvailable => f.write_str("requested format is not available"),
            Self::PayloadTooLarge { size, max_bytes } => {
                write!(
                    f,
                    "payload size {size} bytes exceeds limit {max_bytes} bytes"
                )
            }
            Self::Cancelled => f.write_str("transfer was cancelled"),
            Self::SourceDestroyed => f.write_str("transfer source was destroyed"),
            Self::ProviderFailed(msg) => write!(f, "transfer provider failed: {msg}"),
            Self::InvalidUtf8 => f.write_str("payload is not valid UTF-8"),
            Self::InvalidImageData => f.write_str("payload is not valid RGBA8 image data"),
            Self::ExternalInternalPayloadForbidden => {
                f.write_str("external transfers cannot carry internal typed payloads")
            }
        }
    }
}

impl std::error::Error for TransferError {}

const LIFETIME_ACTIVE: u8 = 0;
const LIFETIME_CANCELLED: u8 = 1;
const LIFETIME_SOURCE_DESTROYED: u8 = 2;

/// Shared liveness token for a [`DataTransfer`] and its lazy providers.
#[derive(Debug, Clone)]
pub struct TransferLifetime {
    state: Arc<AtomicU8>,
}

impl Default for TransferLifetime {
    fn default() -> Self {
        Self::active()
    }
}

impl TransferLifetime {
    /// Create a live transfer lifetime.
    pub fn active() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(LIFETIME_ACTIVE)),
        }
    }

    /// Transition the lifetime to cancelled (idempotent once terminal).
    pub fn cancel(&self) {
        let _ = self.state.compare_exchange(
            LIFETIME_ACTIVE,
            LIFETIME_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Mark the producing node/window destroyed (idempotent once terminal).
    pub fn mark_source_destroyed(&self) {
        let _ = self.state.compare_exchange(
            LIFETIME_ACTIVE,
            LIFETIME_SOURCE_DESTROYED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Whether the transfer is still active.
    pub fn is_alive(&self) -> bool {
        self.state.load(Ordering::Acquire) == LIFETIME_ACTIVE
    }

    /// Verify the lifetime is active, returning `Cancelled` or `SourceDestroyed`.
    pub fn check(&self) -> Result<(), TransferError> {
        match self.state.load(Ordering::Acquire) {
            LIFETIME_ACTIVE => Ok(()),
            LIFETIME_CANCELLED => Err(TransferError::Cancelled),
            _ => Err(TransferError::SourceDestroyed),
        }
    }
}

/// Decoded RGBA8 image payload carried in `image/rgba8`.
///
/// The wire encoding (`to_rgba8_bytes` / `from_rgba8_bytes`) uses an 8-byte
/// little-endian header `[width: u32, height: u32]` followed by tightly packed
/// RGBA8 pixels (`width * height * 4` bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl TransferImage {
    /// Construct and validate an RGBA8 image payload.
    pub fn new(width: u32, height: u32, rgba: impl Into<Arc<[u8]>>) -> Result<Self, TransferError> {
        if width == 0 || height == 0 {
            return Err(TransferError::InvalidImageData);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or(TransferError::InvalidImageData)?;
        if expected > DEFAULT_MAX_TRANSFER_BYTES {
            return Err(TransferError::PayloadTooLarge {
                size: expected,
                max_bytes: DEFAULT_MAX_TRANSFER_BYTES,
            });
        }
        let rgba: Arc<[u8]> = rgba.into();
        if rgba.len() != expected {
            return Err(TransferError::InvalidImageData);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Serialize to `[width_le: 4B][height_le: 4B][rgba pixels]`.
    pub fn to_rgba8_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.rgba.len());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.rgba);
        out
    }

    /// Parse and validate `[width_le: 4B][height_le: 4B][rgba pixels]`.
    pub fn from_rgba8_bytes(bytes: &[u8]) -> Result<Self, TransferError> {
        if bytes.len() < 8 {
            return Err(TransferError::InvalidImageData);
        }
        let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Self::new(width, height, Arc::<[u8]>::from(&bytes[8..]))
    }
}

/// On-demand payload producer invoked only when a consumer reads `format`.
pub trait TransferProvider: Send + Sync {
    fn provide(&self, format: &DataFormat) -> Result<Vec<u8>, TransferError>;
}

/// Closure adapter implementing [`TransferProvider`].
pub struct LazyFnProvider<F>(pub F);

impl<F> TransferProvider for LazyFnProvider<F>
where
    F: Fn(&DataFormat) -> Result<Vec<u8>, TransferError> + Send + Sync,
{
    fn provide(&self, format: &DataFormat) -> Result<Vec<u8>, TransferError> {
        (self.0)(format)
    }
}

/// Cheap metadata describing one format slot in a [`DataTransfer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEntryMeta {
    pub format: DataFormat,
    pub size_hint: Option<usize>,
    pub is_lazy: bool,
    pub cache_policy: TransferCachePolicy,
}

/// One stored payload entry (eager bytes or lazy provider).
#[derive(Clone)]
pub enum TransferEntry {
    Eager(Arc<[u8]>),
    Lazy {
        provider: Arc<dyn TransferProvider>,
        size_hint: Option<usize>,
        cache_policy: TransferCachePolicy,
        cached: Arc<Mutex<Option<Arc<[u8]>>>>,
    },
}

impl fmt::Debug for TransferEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eager(bytes) => f.debug_struct("Eager").field("len", &bytes.len()).finish(),
            Self::Lazy {
                size_hint,
                cache_policy,
                cached,
                ..
            } => f
                .debug_struct("Lazy")
                .field("size_hint", size_hint)
                .field("cache_policy", cache_policy)
                .field(
                    "cached_len",
                    &cached.lock().ok().and_then(|g| g.as_ref().map(|b| b.len())),
                )
                .finish(),
        }
    }
}

fn process_nonce() -> u64 {
    static NONCE: OnceLock<u64> = OnceLock::new();
    *NONCE.get_or_init(|| {
        let mut state = (std::process::id() as u64)
            ^ ((&NONCE as *const OnceLock<u64>) as usize as u64).rotate_left(17);
        if let Ok(elapsed) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            state ^= (elapsed.as_nanos() as u64).rotate_left(31);
        }
        // SplitMix64 mixing so the nonce is non-trivial.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        state ^ (state >> 31)
    })
}

static NEXT_INTERNAL_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Unforgeable process-local capability token identifying an internal object payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InternalPayloadToken {
    process_nonce: u64,
    id: u64,
}

impl InternalPayloadToken {
    fn next() -> Self {
        Self {
            process_nonce: process_nonce(),
            id: NEXT_INTERNAL_TOKEN.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Whether this token was minted in the current process.
    pub fn is_current_process(&self) -> bool {
        self.process_nonce == process_nonce()
    }
}

#[derive(Clone)]
struct InternalPayloadEntry {
    token: InternalPayloadToken,
    kind: DropKind,
    value: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for InternalPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InternalPayloadEntry")
            .field("token", &self.token)
            .field("kind", &self.kind)
            .finish()
    }
}

/// Multi-format payload container shared by clipboard and drag-and-drop.
#[derive(Debug, Clone)]
pub struct DataTransfer {
    origin: TransferOrigin,
    lifetime: TransferLifetime,
    max_bytes: usize,
    entries: Vec<(DataFormat, TransferEntry)>,
    internal: Option<InternalPayloadEntry>,
}

impl Default for DataTransfer {
    fn default() -> Self {
        Self::internal()
    }
}

impl DataTransfer {
    /// Default maximum payload byte size (`64 MiB`).
    pub const DEFAULT_MAX_BYTES: usize = DEFAULT_MAX_TRANSFER_BYTES;

    /// Create an empty transfer with the given origin.
    pub fn new(origin: TransferOrigin) -> Self {
        Self {
            origin,
            lifetime: TransferLifetime::active(),
            max_bytes: Self::DEFAULT_MAX_BYTES,
            entries: Vec::new(),
            internal: None,
        }
    }

    /// Create an empty internal transfer.
    pub fn internal() -> Self {
        Self::new(TransferOrigin::Internal)
    }

    /// Create an empty external transfer.
    pub fn external() -> Self {
        Self::new(TransferOrigin::External)
    }

    /// Build an internal transfer carrying `text/plain`.
    pub fn from_text(text: impl Into<String>) -> Self {
        let mut transfer = Self::internal();
        let _ = transfer.set_text(text);
        transfer
    }

    /// Build a transfer carrying `text/uri-list` for `paths` without reading file contents.
    pub fn from_files(origin: TransferOrigin, paths: &[PathBuf]) -> Self {
        let mut transfer = Self::new(origin);
        let _ = transfer.set_file_paths(paths);
        transfer
    }

    /// Override the maximum byte limit enforced when storing or reading a format.
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Attach a shared [`TransferLifetime`] to this transfer.
    pub fn with_lifetime(mut self, lifetime: TransferLifetime) -> Self {
        self.lifetime = lifetime;
        self
    }

    pub fn origin(&self) -> TransferOrigin {
        self.origin
    }

    pub fn lifetime(&self) -> &TransferLifetime {
        &self.lifetime
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.internal.is_none()
    }

    /// List all advertised formats without triggering lazy providers.
    pub fn formats(&self) -> Vec<DataFormat> {
        self.entries.iter().map(|(fmt, _)| fmt.clone()).collect()
    }

    /// Whether `format` is advertised (without materializing lazy providers).
    pub fn has_format(&self, format: &DataFormat) -> bool {
        self.entries.iter().any(|(fmt, _)| fmt == format)
    }

    /// Cheap metadata for `format`, without triggering lazy providers.
    pub fn entry_meta(&self, format: &DataFormat) -> Option<TransferEntryMeta> {
        self.entries
            .iter()
            .find(|(fmt, _)| fmt == format)
            .map(|(fmt, entry)| match entry {
                TransferEntry::Eager(bytes) => TransferEntryMeta {
                    format: fmt.clone(),
                    size_hint: Some(bytes.len()),
                    is_lazy: false,
                    cache_policy: TransferCachePolicy::Cache,
                },
                TransferEntry::Lazy {
                    size_hint,
                    cache_policy,
                    ..
                } => TransferEntryMeta {
                    format: fmt.clone(),
                    size_hint: *size_hint,
                    is_lazy: true,
                    cache_policy: *cache_policy,
                },
            })
    }

    /// Return the eagerly-stored `text/plain` slice without cloning or running lazy providers.
    pub fn eager_text(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|(fmt, _)| *fmt == DataFormat::TextPlain)
            .and_then(|(_, entry)| match entry {
                TransferEntry::Eager(bytes) => std::str::from_utf8(bytes).ok(),
                TransferEntry::Lazy { .. } => None,
            })
    }

    pub fn internal_kind(&self) -> Option<&DropKind> {
        self.internal.as_ref().map(|entry| &entry.kind)
    }

    pub fn internal_token(&self) -> Option<InternalPayloadToken> {
        self.internal.as_ref().map(|entry| entry.token)
    }

    fn upsert_entry(&mut self, format: DataFormat, entry: TransferEntry) {
        if let Some((_, existing)) = self.entries.iter_mut().find(|(fmt, _)| *fmt == format) {
            *existing = entry;
        } else {
            self.entries.push((format, entry));
        }
    }

    /// Store eager bytes for `format`, rejecting payloads exceeding `max_bytes`.
    pub fn set_bytes(
        &mut self,
        format: DataFormat,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<(), TransferError> {
        let bytes: Arc<[u8]> = bytes.into();
        if bytes.len() > self.max_bytes {
            return Err(TransferError::PayloadTooLarge {
                size: bytes.len(),
                max_bytes: self.max_bytes,
            });
        }
        self.upsert_entry(format, TransferEntry::Eager(bytes));
        Ok(())
    }

    /// Register a lazy provider for `format`.
    ///
    /// Does not invoke `provider` and accepts large `size_hint` values during
    /// registration; `max_bytes` is enforced when a consumer reads the format.
    pub fn set_lazy(
        &mut self,
        format: DataFormat,
        size_hint: Option<usize>,
        cache_policy: TransferCachePolicy,
        provider: Arc<dyn TransferProvider>,
    ) {
        self.upsert_entry(
            format,
            TransferEntry::Lazy {
                provider,
                size_hint,
                cache_policy,
                cached: Arc::new(Mutex::new(None)),
            },
        );
    }

    /// Register a closure-backed lazy provider for `format`.
    pub fn set_lazy_fn<F>(
        &mut self,
        format: DataFormat,
        size_hint: Option<usize>,
        cache_policy: TransferCachePolicy,
        f: F,
    ) where
        F: Fn(&DataFormat) -> Result<Vec<u8>, TransferError> + Send + Sync + 'static,
    {
        self.set_lazy(format, size_hint, cache_policy, Arc::new(LazyFnProvider(f)));
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> Result<(), TransferError> {
        let text = text.into();
        self.set_bytes(DataFormat::TextPlain, Arc::<[u8]>::from(text.into_bytes()))
    }

    pub fn set_html(&mut self, html: impl Into<String>) -> Result<(), TransferError> {
        let html = html.into();
        self.set_bytes(DataFormat::TextHtml, Arc::<[u8]>::from(html.into_bytes()))
    }

    pub fn set_uri_list(&mut self, uris: &[String]) -> Result<(), TransferError> {
        let encoded = format_uri_list(uris);
        self.set_bytes(
            DataFormat::TextUriList,
            Arc::<[u8]>::from(encoded.into_bytes()),
        )
    }

    /// Encode filesystem paths as `text/uri-list` without reading any file contents.
    pub fn set_file_paths(&mut self, paths: &[PathBuf]) -> Result<(), TransferError> {
        let uris: Vec<String> = paths.iter().map(|p| path_to_file_uri(p)).collect();
        self.set_uri_list(&uris)
    }

    pub fn set_png(&mut self, png_bytes: impl Into<Arc<[u8]>>) -> Result<(), TransferError> {
        self.set_bytes(DataFormat::ImagePng, png_bytes)
    }

    pub fn set_rgba_image(&mut self, image: TransferImage) -> Result<(), TransferError> {
        let bytes = image.to_rgba8_bytes();
        self.set_bytes(DataFormat::ImageRgba8, Arc::<[u8]>::from(bytes))
    }

    /// Attach a typed process-local object to an internal transfer.
    pub fn set_internal_payload<T: Send + Sync + 'static>(
        &mut self,
        kind: DropKind,
        value: T,
    ) -> Result<InternalPayloadToken, TransferError> {
        if self.origin != TransferOrigin::Internal {
            return Err(TransferError::ExternalInternalPayloadForbidden);
        }
        let token = InternalPayloadToken::next();
        self.internal = Some(InternalPayloadEntry {
            token,
            kind,
            value: Arc::new(value),
        });
        Ok(token)
    }

    /// Materialize raw bytes for `format`, checking lifetime, `max_bytes`, and cache policy.
    pub fn read_bytes(&self, format: &DataFormat) -> Result<Option<Arc<[u8]>>, TransferError> {
        self.lifetime.check()?;
        let Some((_, entry)) = self.entries.iter().find(|(fmt, _)| fmt == format) else {
            return Ok(None);
        };
        match entry {
            TransferEntry::Eager(bytes) => {
                if bytes.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: bytes.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                Ok(Some(Arc::clone(bytes)))
            }
            TransferEntry::Lazy {
                provider,
                size_hint,
                cache_policy,
                cached,
            } => {
                if let Some(hint) = *size_hint
                    && hint > self.max_bytes
                {
                    return Err(TransferError::PayloadTooLarge {
                        size: hint,
                        max_bytes: self.max_bytes,
                    });
                }
                if *cache_policy == TransferCachePolicy::Cache
                    && let Ok(guard) = cached.lock()
                    && let Some(hit) = guard.as_ref()
                {
                    return Ok(Some(Arc::clone(hit)));
                }
                let produced = provider.provide(format)?;
                self.lifetime.check()?;
                if produced.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: produced.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                let arc: Arc<[u8]> = Arc::from(produced);
                if *cache_policy == TransferCachePolicy::Cache
                    && let Ok(mut guard) = cached.lock()
                {
                    *guard = Some(Arc::clone(&arc));
                }
                Ok(Some(arc))
            }
        }
    }

    pub fn read_text(&self) -> Result<Option<String>, TransferError> {
        let Some(bytes) = self.read_bytes(&DataFormat::TextPlain)? else {
            return Ok(None);
        };
        String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(|_| TransferError::InvalidUtf8)
    }

    pub fn read_html(&self) -> Result<Option<String>, TransferError> {
        let Some(bytes) = self.read_bytes(&DataFormat::TextHtml)? else {
            return Ok(None);
        };
        String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(|_| TransferError::InvalidUtf8)
    }

    pub fn read_uri_list(&self) -> Result<Option<Vec<String>>, TransferError> {
        let Some(bytes) = self.read_bytes(&DataFormat::TextUriList)? else {
            return Ok(None);
        };
        let raw = std::str::from_utf8(&bytes).map_err(|_| TransferError::InvalidUtf8)?;
        Ok(Some(parse_uri_list(raw)))
    }

    /// Decode `file://` URIs from `text/uri-list` into local paths (no disk I/O).
    pub fn read_file_paths(&self) -> Result<Option<Vec<PathBuf>>, TransferError> {
        let Some(uris) = self.read_uri_list()? else {
            return Ok(None);
        };
        let paths = uris
            .iter()
            .filter_map(|uri| file_uri_to_path(uri))
            .collect();
        Ok(Some(paths))
    }

    pub fn read_png(&self) -> Result<Option<Arc<[u8]>>, TransferError> {
        self.read_bytes(&DataFormat::ImagePng)
    }

    pub fn read_rgba_image(&self) -> Result<Option<TransferImage>, TransferError> {
        let Some(bytes) = self.read_bytes(&DataFormat::ImageRgba8)? else {
            return Ok(None);
        };
        TransferImage::from_rgba8_bytes(&bytes).map(Some)
    }

    /// Access the process-local typed internal payload if origin, lifetime, and token match.
    pub fn read_internal<T: Send + Sync + 'static>(&self) -> Result<Option<&T>, TransferError> {
        self.lifetime.check()?;
        if self.origin != TransferOrigin::Internal {
            return Err(TransferError::ExternalInternalPayloadForbidden);
        }
        let Some(entry) = &self.internal else {
            return Ok(None);
        };
        if !entry.token.is_current_process() {
            return Err(TransferError::ExternalInternalPayloadForbidden);
        }
        Ok(entry.value.downcast_ref::<T>())
    }
}

/// Format URIs per RFC 2483 (`\r\n`-separated).
pub fn format_uri_list(uris: &[String]) -> String {
    let mut out = String::new();
    for (idx, uri) in uris.iter().enumerate() {
        if idx > 0 {
            out.push_str("\r\n");
        }
        out.push_str(uri);
    }
    out
}

/// Parse an RFC 2483 `text/uri-list` payload, skipping `#` comments and blank lines.
pub fn parse_uri_list(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Convert a local filesystem path into a percent-encoded `file://` URI without touching disk.
pub fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let mut out = String::from("file://");
    if !normalized.starts_with('/') {
        out.push('/');
    }
    for byte in normalized.as_bytes() {
        let keep =
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~' | b':');
        if keep {
            out.push(*byte as char);
        } else {
            out.push('%');
            let hi = (byte >> 4) & 0xF;
            let lo = byte & 0xF;
            out.push(hex_upper(hi));
            out.push(hex_upper(lo));
        }
    }
    out
}

/// Parse a `file://` URI into a [`PathBuf`] without touching disk.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path_part = if let Some(after_local) = rest.strip_prefix("localhost/") {
        format!("/{after_local}")
    } else if rest.starts_with('/') {
        rest.to_owned()
    } else {
        return None;
    };
    let decoded = percent_decode_utf8(&path_part)?;
    #[cfg(windows)]
    let decoded = {
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            decoded[1..].replace('/', "\\")
        } else {
            decoded
        }
    };
    Some(PathBuf::from(decoded))
}

fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + (nibble - 10)) as char,
        _ => '0',
    }
}

fn percent_decode_utf8(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = from_hex(bytes[i + 1])?;
            let lo = from_hex(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[test]
    fn mime_parsing_canonicalizes_well_known_and_rejects_invalid_or_handles() {
        assert_eq!(DataFormat::parse("TEXT/Plain"), Ok(DataFormat::TextPlain));
        assert_eq!(DataFormat::parse("text/html"), Ok(DataFormat::TextHtml));
        assert_eq!(
            DataFormat::parse("text/uri-list"),
            Ok(DataFormat::TextUriList)
        );
        assert_eq!(DataFormat::parse("image/png"), Ok(DataFormat::ImagePng));
        assert_eq!(DataFormat::parse("image/rgba8"), Ok(DataFormat::ImageRgba8));
        assert_eq!(
            DataFormat::parse("application/octet-stream"),
            Ok(DataFormat::OctetStream)
        );

        let custom = DataFormat::custom("application/x-Nana-Node").unwrap();
        assert_eq!(custom.as_mime(), "application/x-nana-node");
        assert!(custom.is_custom());

        assert_eq!(DataFormat::parse(""), Err(DataFormatError::Empty));
        assert_eq!(
            DataFormat::parse(&format!("application/{}", "a".repeat(120))),
            Err(DataFormatError::TooLong)
        );
        assert_eq!(
            DataFormat::parse("text/plain;charset=utf-8"),
            Err(DataFormatError::InvalidSyntax)
        );
        assert_eq!(
            DataFormat::parse("text"),
            Err(DataFormatError::InvalidSyntax)
        );
        assert_eq!(
            DataFormat::parse("text/plain/extra"),
            Err(DataFormatError::InvalidSyntax)
        );
        assert_eq!(
            DataFormat::parse(" text/plain"),
            Err(DataFormatError::InvalidSyntax)
        );
        assert_eq!(
            DataFormat::parse("application/x-raw-pointer"),
            Err(DataFormatError::ForbiddenFormat)
        );
        assert_eq!(
            DataFormat::parse("application/x-nana-hwnd"),
            Err(DataFormatError::ForbiddenFormat)
        );
    }

    #[test]
    fn multiple_formats_coexist_in_one_transfer() {
        let mut transfer = DataTransfer::internal();
        transfer.set_text("hello").unwrap();
        transfer.set_html("<b>hello</b>").unwrap();
        let img = TransferImage::new(1, 1, [255, 0, 128, 255].as_slice()).unwrap();
        transfer.set_rgba_image(img.clone()).unwrap();
        transfer
            .set_bytes(
                DataFormat::custom("application/x-nana-snippet").unwrap(),
                b"{}".as_slice(),
            )
            .unwrap();

        assert_eq!(transfer.formats().len(), 4);
        assert_eq!(transfer.read_text().unwrap().as_deref(), Some("hello"));
        assert_eq!(
            transfer.read_html().unwrap().as_deref(),
            Some("<b>hello</b>")
        );
        assert_eq!(transfer.read_rgba_image().unwrap(), Some(img));
        assert_eq!(
            transfer
                .read_bytes(&DataFormat::custom("application/x-nana-snippet").unwrap())
                .unwrap()
                .as_deref(),
            Some(b"{}".as_slice())
        );
    }

    #[test]
    fn lazy_provider_defers_materialization_respects_cache_and_cancels_cleanly() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = Arc::clone(&calls);
        let mut cached = DataTransfer::internal();
        cached.set_lazy_fn(
            DataFormat::TextPlain,
            Some(5),
            TransferCachePolicy::Cache,
            move |_| {
                calls_ref.fetch_add(1, Ordering::Relaxed);
                Ok(b"lazy!".to_vec())
            },
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(cached.has_format(&DataFormat::TextPlain));
        assert_eq!(cached.eager_text(), None);
        assert_eq!(
            cached.entry_meta(&DataFormat::TextPlain),
            Some(TransferEntryMeta {
                format: DataFormat::TextPlain,
                size_hint: Some(5),
                is_lazy: true,
                cache_policy: TransferCachePolicy::Cache,
            })
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        assert_eq!(cached.read_text().unwrap().as_deref(), Some("lazy!"));
        assert_eq!(cached.read_text().unwrap().as_deref(), Some("lazy!"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        let uncached_calls = Arc::new(AtomicUsize::new(0));
        let uncached_ref = Arc::clone(&uncached_calls);
        let mut uncached = DataTransfer::internal();
        uncached.set_lazy_fn(
            DataFormat::TextPlain,
            None,
            TransferCachePolicy::NeverCache,
            move |_| {
                uncached_ref.fetch_add(1, Ordering::Relaxed);
                Ok(b"fresh".to_vec())
            },
        );
        assert_eq!(uncached.read_text().unwrap().as_deref(), Some("fresh"));
        assert_eq!(uncached.read_text().unwrap().as_deref(), Some("fresh"));
        assert_eq!(uncached_calls.load(Ordering::Relaxed), 2);

        uncached.lifetime().cancel();
        assert_eq!(uncached.read_text(), Err(TransferError::Cancelled));

        let destroyed = DataTransfer::internal();
        destroyed.lifetime().mark_source_destroyed();
        assert_eq!(destroyed.read_text(), Err(TransferError::SourceDestroyed));
    }

    #[test]
    fn large_lazy_payload_is_not_allocated_on_registration_and_enforces_limit_on_read() {
        let invoked = Arc::new(AtomicUsize::new(0));
        let invoked_ref = Arc::clone(&invoked);
        let mut transfer = DataTransfer::internal();
        let two_gib = 2usize * 1024 * 1024 * 1024;
        transfer.set_lazy_fn(
            DataFormat::OctetStream,
            Some(two_gib),
            TransferCachePolicy::Cache,
            move |_| {
                invoked_ref.fetch_add(1, Ordering::Relaxed);
                Ok(vec![1, 2, 3])
            },
        );
        let meta = transfer.entry_meta(&DataFormat::OctetStream).unwrap();
        assert_eq!(meta.size_hint, Some(two_gib));
        assert_eq!(invoked.load(Ordering::Relaxed), 0);
        assert_eq!(
            transfer.read_bytes(&DataFormat::OctetStream),
            Err(TransferError::PayloadTooLarge {
                size: two_gib,
                max_bytes: DataTransfer::DEFAULT_MAX_BYTES,
            })
        );
        assert_eq!(invoked.load(Ordering::Relaxed), 0);

        let mut capped = DataTransfer::internal().with_max_bytes(4);
        assert_eq!(
            capped.set_bytes(DataFormat::OctetStream, [0u8; 5].as_slice()),
            Err(TransferError::PayloadTooLarge {
                size: 5,
                max_bytes: 4
            })
        );
        capped.set_lazy_fn(
            DataFormat::OctetStream,
            None,
            TransferCachePolicy::Cache,
            |_| Ok(vec![0u8; 5]),
        );
        assert_eq!(
            capped.read_bytes(&DataFormat::OctetStream),
            Err(TransferError::PayloadTooLarge {
                size: 5,
                max_bytes: 4
            })
        );
    }

    #[test]
    fn internal_typed_payload_is_isolated_from_external_transfers() {
        #[derive(Debug, PartialEq)]
        struct NodeRef(u64);

        let mut internal = DataTransfer::internal();
        let token = internal
            .set_internal_payload(DropKind::custom("node-ref"), NodeRef(42))
            .unwrap();
        assert!(token.is_current_process());
        assert_eq!(
            internal.read_internal::<NodeRef>().unwrap(),
            Some(&NodeRef(42))
        );
        assert_eq!(internal.read_internal::<String>().unwrap(), None);

        let mut external = DataTransfer::external();
        assert_eq!(
            external.set_internal_payload(DropKind::custom("node-ref"), NodeRef(1)),
            Err(TransferError::ExternalInternalPayloadForbidden)
        );
        assert_eq!(
            external.read_internal::<NodeRef>(),
            Err(TransferError::ExternalInternalPayloadForbidden)
        );
    }

    #[test]
    fn uri_list_and_rgba8_round_trip_without_disk_io() {
        let missing = PathBuf::from("/nonexistent-dir-120/hello world.png");
        let transfer = DataTransfer::from_files(TransferOrigin::External, &[missing.clone()]);
        let uris = transfer.read_uri_list().unwrap().unwrap();
        assert_eq!(uris, vec!["file:///nonexistent-dir-120/hello%20world.png"]);
        assert_eq!(transfer.read_file_paths().unwrap(), Some(vec![missing]));

        let image = TransferImage::new(2, 1, [10, 20, 30, 40, 50, 60, 70, 80].as_slice()).unwrap();
        let round = TransferImage::from_rgba8_bytes(&image.to_rgba8_bytes()).unwrap();
        assert_eq!(round, image);
        assert_eq!(
            TransferImage::from_rgba8_bytes(&[1, 0, 0, 0, 1, 0, 0, 0, 1, 2]),
            Err(TransferError::InvalidImageData)
        );
    }
}
