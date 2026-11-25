//! Platform custom cursor handle creation, bounded LRU caching, and window application.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::OwnedHcursor;

use std::collections::HashMap;
use std::sync::Arc;

use raw_window_handle::HasWindowHandle;

/// Maximum number of native custom cursor handles kept in a [`PlatformCursorCache`].
pub const MAX_CACHED_CURSORS: usize = 64;

/// Capabilities of the active platform custom-cursor backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomCursorCapabilities {
    pub supported: bool,
    pub multi_dpi: bool,
    pub max_edge: u16,
}

/// Outcome of applying a custom cursor to a native window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomCursorOutcome {
    Applied { scale_milli: u32 },
    Unsupported(String),
    Fallback(String),
}

/// Validated RGBA8 cursor bitmap prepared for a target display scale.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorBitmap {
    /// Content identity hash (derived from `CustomCursorSpec`).
    pub cache_key: u64,
    /// Quantized scale factor in thousandths (`1000` = 1x, `2000` = 2x).
    pub scale_milli: u32,
    /// Logical cursor width in points / DIPs.
    pub logical_width: u16,
    /// Logical cursor height in points / DIPs.
    pub logical_height: u16,
    /// Logical hotspot `(x, y)` inside `[0..logical_width, 0..logical_height)`.
    pub logical_hotspot: [f64; 2],
    /// Physical bitmap width in pixels.
    pub width: u16,
    /// Physical bitmap height in pixels.
    pub height: u16,
    /// Hotspot in physical bitmap pixels `[x, y]`.
    pub hotspot: [u16; 2],
    /// Straight-alpha RGBA8 pixels (`width * height * 4` bytes).
    pub rgba: Arc<[u8]>,
}

/// Quantize a floating-point display scale factor into 25-milli steps (`>= 250`).
pub fn quantize_scale_milli(scale: f32) -> u32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 1000;
    }
    (((scale * 40.0).round() as u32).max(10)) * 25
}

/// Platform-owned native cursor handle (AppKit `NSCursor`, Win32 `HCURSOR`, or fallback).
#[derive(Clone)]
pub struct NativeCursorHandle {
    #[cfg(target_os = "macos")]
    inner: macos::MacosCursorHandle,
    #[cfg(target_os = "windows")]
    inner: windows::WindowsCursorHandle,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    inner: fallback::FallbackCursorHandle,
}

impl std::fmt::Debug for NativeCursorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeCursorHandle").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    spec_hash: u64,
    scale_milli: u32,
}

struct CacheEntry {
    handle: NativeCursorHandle,
    last_used: u64,
}

/// Bounded LRU cache of native platform cursor handles keyed by `(spec_hash, scale_milli)`.
pub struct PlatformCursorCache {
    capacity: usize,
    tick: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: HashMap<CacheKey, CacheEntry>,
}

impl Default for PlatformCursorCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_CURSORS)
    }
}

impl PlatformCursorCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            tick: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Look up or build a cached native cursor handle for `bitmap`.
    pub fn get_or_create(&mut self, bitmap: &CursorBitmap) -> Result<NativeCursorHandle, String> {
        let key = CacheKey {
            spec_hash: bitmap.cache_key,
            scale_milli: bitmap.scale_milli,
        };
        self.tick = self.tick.wrapping_add(1);
        let now = self.tick;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = now;
            self.hits += 1;
            return Ok(entry.handle.clone());
        }
        self.misses += 1;
        let handle = build_native_cursor(bitmap)?;
        if self.entries.len() >= self.capacity
            && let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
        {
            self.entries.remove(&oldest_key);
            self.evictions += 1;
        }
        self.entries.insert(
            key,
            CacheEntry {
                handle: handle.clone(),
                last_used: now,
            },
        );
        Ok(handle)
    }
}

pub fn custom_cursor_capabilities() -> CustomCursorCapabilities {
    #[cfg(target_os = "macos")]
    {
        macos::capabilities()
    }
    #[cfg(target_os = "windows")]
    {
        windows::capabilities()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        fallback::capabilities()
    }
}

fn build_native_cursor(bitmap: &CursorBitmap) -> Result<NativeCursorHandle, String> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return Err("custom cursor bitmap dimensions must be non-zero".into());
    }
    if bitmap.hotspot[0] >= bitmap.width || bitmap.hotspot[1] >= bitmap.height {
        return Err("custom cursor hotspot out of bounds".into());
    }
    let expected = usize::from(bitmap.width) * usize::from(bitmap.height) * 4;
    if bitmap.rgba.len() != expected {
        return Err("custom cursor RGBA byte length mismatch".into());
    }
    #[cfg(target_os = "macos")]
    {
        macos::create_cursor(bitmap).map(|inner| NativeCursorHandle { inner })
    }
    #[cfg(target_os = "windows")]
    {
        windows::create_cursor(bitmap).map(|inner| NativeCursorHandle { inner })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        fallback::create_cursor(bitmap).map(|inner| NativeCursorHandle { inner })
    }
}

/// Apply a cached native custom cursor handle to `window`.
pub fn apply_custom_cursor<W: HasWindowHandle + ?Sized>(
    window: &W,
    handle: &NativeCursorHandle,
    scale_milli: u32,
) -> CustomCursorOutcome {
    #[cfg(target_os = "macos")]
    {
        macos::apply_custom_cursor(window, &handle.inner, scale_milli)
    }
    #[cfg(target_os = "windows")]
    {
        windows::apply_custom_cursor(window, &handle.inner, scale_milli)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        fallback::apply_custom_cursor(window, &handle.inner, scale_milli)
    }
}

/// Clear any active platform custom cursor override so the standard/system cursor applies.
pub fn clear_custom_cursor<W: HasWindowHandle + ?Sized>(window: &W) {
    #[cfg(target_os = "macos")]
    {
        macos::clear_custom_cursor(window);
    }
    #[cfg(target_os = "windows")]
    {
        windows::clear_custom_cursor(window);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bitmap(key: u64, scale_milli: u32) -> CursorBitmap {
        CursorBitmap {
            cache_key: key,
            scale_milli,
            logical_width: 4,
            logical_height: 4,
            logical_hotspot: [1.0, 1.0],
            width: 4,
            height: 4,
            hotspot: [1, 1],
            rgba: Arc::from(vec![255u8; 4 * 4 * 4]),
        }
    }

    #[test]
    fn scale_quantization_preserves_standard_dpi_steps() {
        assert_eq!(quantize_scale_milli(1.0), 1000);
        assert_eq!(quantize_scale_milli(1.25), 1250);
        assert_eq!(quantize_scale_milli(1.5), 1500);
        assert_eq!(quantize_scale_milli(2.0), 2000);
        assert_eq!(quantize_scale_milli(f32::NAN), 1000);
    }

    #[test]
    fn lru_cache_evicts_oldest_and_tracks_hits_misses() {
        let mut cache = PlatformCursorCache::new(2);
        let a = test_bitmap(1, 1000);
        let b = test_bitmap(2, 2000);
        let c = test_bitmap(3, 1000);

        assert!(cache.get_or_create(&a).is_ok());
        assert!(cache.get_or_create(&b).is_ok());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.misses(), 2);

        // Touch `a` so `b` becomes least recently used.
        assert!(cache.get_or_create(&a).is_ok());
        assert_eq!(cache.hits(), 1);

        assert!(cache.get_or_create(&c).is_ok());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.evictions(), 1);

        // `a` is still cached; `b` was evicted.
        assert!(cache.get_or_create(&a).is_ok());
        assert_eq!(cache.hits(), 2);
        assert!(cache.get_or_create(&b).is_ok());
        assert_eq!(cache.misses(), 4);
        assert_eq!(cache.evictions(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn invalid_bitmap_is_rejected_without_entering_cache() {
        let mut cache = PlatformCursorCache::new(4);
        let bad = CursorBitmap {
            hotspot: [4, 0],
            ..test_bitmap(9, 1000)
        };
        assert!(cache.get_or_create(&bad).is_err());
        assert!(cache.is_empty());
    }
}
