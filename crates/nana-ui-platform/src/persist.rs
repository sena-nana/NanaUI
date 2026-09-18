//! File-backed [`PersistentStore`] and process data-directory lookup.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nana_ui_core::{PersistentStore, StoreError, window_storage_key};
use serde::{Deserialize, Serialize};

const STORAGE_FILE: &str = "local-storage.bin";
const STORAGE_MAGIC: &[u8; 4] = b"NANA";
const STORAGE_VERSION: u8 = 1;

/// One binary map file (`local-storage.bin`) inside a host-chosen directory.
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
    cache: Mutex<StoreCache>,
    flush: Mutex<()>,
}

#[derive(Debug, Default)]
struct StoreCache {
    entries: BTreeMap<String, String>,
    dirty: bool,
    loaded: bool,
}

impl FileStore {
    /// Create or open a directory holding `local-storage.bin`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|error| StoreError::new(error.to_string()))?;
        Ok(Self {
            dir,
            cache: Mutex::new(StoreCache::default()),
            flush: Mutex::new(()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self) -> PathBuf {
        self.dir.join(STORAGE_FILE)
    }

    fn load(&self, cache: &mut StoreCache) -> Result<(), StoreError> {
        if cache.loaded {
            return Ok(());
        }
        let path = self.path();
        let loaded = match fs::read(&path) {
            Ok(raw) => match decode_map(&raw) {
                Some(entries) => StoreCache {
                    entries,
                    dirty: false,
                    loaded: true,
                },
                None => {
                    let _ = replace_file(&path, &path.with_extension("bin.corrupt"));
                    StoreCache {
                        entries: BTreeMap::new(),
                        dirty: false,
                        loaded: true,
                    }
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => StoreCache {
                entries: BTreeMap::new(),
                dirty: false,
                loaded: true,
            },
            Err(error) => return Err(StoreError::new(error.to_string())),
        };
        let _ = fs::remove_file(tmp_path(&path));
        *cache = loaded;
        Ok(())
    }

    fn write_map(path: &Path, entries: &BTreeMap<String, String>) -> Result<(), StoreError> {
        let payload = encode_map(entries)?;
        let tmp = tmp_path(path);
        if let Err(error) = (|| -> Result<(), StoreError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|error| StoreError::new(error.to_string()))?;
            file.write_all(&payload)
                .map_err(|error| StoreError::new(error.to_string()))?;
            file.sync_all()
                .map_err(|error| StoreError::new(error.to_string()))?;
            Ok(())
        })() {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        replace_file(&tmp, path)
    }
}

impl Drop for FileStore {
    fn drop(&mut self) {
        let _ = PersistentStore::flush(self);
    }
}

impl PersistentStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        self.load(&mut cache)?;
        Ok(cache.entries.get(key).cloned())
    }

    fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        self.load(&mut cache)?;
        if cache
            .entries
            .get(key)
            .is_some_and(|current| current == &value)
        {
            return Ok(());
        }
        cache.entries.insert(key.to_string(), value);
        cache.dirty = true;
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StoreError> {
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        self.load(&mut cache)?;
        if cache.entries.remove(key).is_some() {
            cache.dirty = true;
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreError> {
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        self.load(&mut cache)?;
        if !cache.entries.is_empty() {
            cache.entries.clear();
            cache.dirty = true;
        }
        Ok(())
    }

    fn keys(&self) -> Result<Vec<String>, StoreError> {
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        self.load(&mut cache)?;
        Ok(cache.entries.keys().cloned().collect())
    }

    fn flush(&self) -> Result<(), StoreError> {
        let _flush = self.flush.lock().map_err(|_| StoreError::poisoned())?;
        let (path, snapshot) = {
            let cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
            if !cache.loaded || !cache.dirty {
                return Ok(());
            }
            (self.path(), cache.entries.clone())
        };
        Self::write_map(&path, &snapshot)?;
        let mut cache = self.cache.lock().map_err(|_| StoreError::poisoned())?;
        if cache.entries == snapshot {
            cache.dirty = false;
        }
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension("bin.tmp")
}

fn replace_file(from: &Path, to: &Path) -> Result<(), StoreError> {
    if from == to {
        return Ok(());
    }
    #[cfg(windows)]
    {
        replace_file_windows(from, to)
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to).map_err(|error| StoreError::new(error.to_string()))
    }
}

#[cfg(windows)]
fn replace_file_windows(from: &Path, to: &Path) -> Result<(), StoreError> {
    use std::os::windows::ffi::OsStrExt;

    fn wide(path: &Path) -> Result<Vec<u16>, StoreError> {
        let mut buf: Vec<u16> = path.as_os_str().encode_wide().collect();
        if buf.contains(&0) {
            return Err(StoreError::new("path contains NUL"));
        }
        buf.push(0);
        Ok(buf)
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            dw_flags: u32,
        ) -> i32;
    }

    let from_w = wide(from)?;
    let to_w = wide(to)?;
    let ok = unsafe {
        MoveFileExW(
            from_w.as_ptr(),
            to_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(StoreError::new(io::Error::last_os_error().to_string()))
    } else {
        Ok(())
    }
}

fn encode_map(entries: &BTreeMap<String, String>) -> Result<Vec<u8>, StoreError> {
    let count = u32::try_from(entries.len())
        .map_err(|_| StoreError::new("localStorage map has too many keys"))?;
    let mut buf = Vec::new();
    buf.extend_from_slice(STORAGE_MAGIC);
    buf.push(STORAGE_VERSION);
    buf.extend_from_slice(&count.to_le_bytes());
    for (key, value) in entries {
        let key_bytes = key.as_bytes();
        let value_bytes = value.as_bytes();
        let key_len = u32::try_from(key_bytes.len())
            .map_err(|_| StoreError::new("localStorage key is too large"))?;
        let value_len = u32::try_from(value_bytes.len())
            .map_err(|_| StoreError::new("localStorage value is too large"))?;
        buf.extend_from_slice(&key_len.to_le_bytes());
        buf.extend_from_slice(key_bytes);
        buf.extend_from_slice(&value_len.to_le_bytes());
        buf.extend_from_slice(value_bytes);
    }
    Ok(buf)
}

fn decode_map(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    let mut i: usize = 0;
    let magic_end = i.checked_add(4)?;
    let magic = bytes.get(i..magic_end)?;
    i = magic_end;
    if magic != STORAGE_MAGIC {
        return None;
    }
    let version = *bytes.get(i)?;
    i = i.checked_add(1)?;
    if version != STORAGE_VERSION {
        return None;
    }
    let count = u32_le(bytes, &mut i)?;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let key = utf8_len_prefixed(bytes, &mut i)?;
        let value = utf8_len_prefixed(bytes, &mut i)?;
        entries.insert(key, value);
    }
    if i != bytes.len() {
        return None;
    }
    Some(entries)
}

fn u32_le(bytes: &[u8], i: &mut usize) -> Option<u32> {
    let end = i.checked_add(4)?;
    let slice = bytes.get(*i..end)?;
    *i = end;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

fn utf8_len_prefixed(bytes: &[u8], i: &mut usize) -> Option<String> {
    let len = u32_le(bytes, i)? as usize;
    let end = i.checked_add(len)?;
    let slice = bytes.get(*i..end)?;
    *i = end;
    String::from_utf8(slice.to_vec()).ok()
}

fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && !app_id.starts_with(['/', '\\'])
        && !app_id.contains('\0')
        && app_id
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Host-chosen application data directory. The framework never writes here
/// unless the host opens a [`FileStore`] on the returned path.
///
/// - macOS: `~/Library/Application Support/{app_id}`
/// - Windows: `%APPDATA%/{app_id}`
/// - Linux: `$XDG_DATA_HOME/{app_id}` or `~/.local/share/{app_id}`
/// - Android: `Context.getFilesDir()` (the `app_id` is ignored; the process is
///   already sandboxed)
pub fn app_data_dir(app_id: &str) -> Option<PathBuf> {
    if !valid_app_id(app_id) {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(app_id),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join(app_id))
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return Some(PathBuf::from(xdg).join(app_id));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".local/share").join(app_id))
    }
    #[cfg(target_os = "android")]
    {
        let _ = app_id;
        android_files_dir()
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        let _ = app_id;
        None
    }
}

#[cfg(target_os = "android")]
fn android_files_dir() -> Option<PathBuf> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(android_files_dir_inner)).ok()?
}

#[cfg(target_os = "android")]
fn android_files_dir_inner() -> Option<PathBuf> {
    use std::mem::ManuallyDrop;

    use jni::JavaVM;
    use jni::objects::{JObject, JString};

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let context =
        ManuallyDrop::new(unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) });
    let files = env
        .call_method(&*context, "getFilesDir", "()Ljava/io/File;", &[])
        .ok()?
        .l()
        .ok()?;
    if files.is_null() {
        return None;
    }
    let path = env
        .call_method(files, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path = JString::from(path);
    let path = env.get_string(&path).ok()?;
    Some(PathBuf::from(path.to_str().ok()?))
}

/// Logical window frame recorded under [`window_storage_key`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedWindowGeometry {
    pub version: u8,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub maximized: bool,
}

impl Default for PersistedWindowGeometry {
    fn default() -> Self {
        Self {
            version: 1,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            maximized: false,
        }
    }
}

impl PersistedWindowGeometry {
    pub fn from_descriptor(settings: &crate::WindowDescriptor) -> Self {
        let (x, y) = settings.initial_position.unwrap_or((0.0, 0.0));
        Self {
            version: 1,
            x,
            y,
            width: settings.initial_size.0,
            height: settings.initial_size.1,
            maximized: settings.maximized,
        }
    }

    pub fn apply(&self, settings: &mut crate::WindowDescriptor) {
        if self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
        {
            settings.initial_size = (self.width, self.height);
        }
        if self.x.is_finite() && self.y.is_finite() {
            settings.initial_position = Some((self.x, self.y));
        }
        settings.maximized = self.maximized;
    }

    pub fn load(store: &dyn PersistentStore, key: &str) -> Result<Option<Self>, StoreError> {
        let Some(json) = store.get(&window_storage_key(key))? else {
            return Ok(None);
        };
        Ok(serde_json::from_str(&json).ok())
    }

    pub fn save(&self, store: &dyn PersistentStore, key: &str) -> Result<(), StoreError> {
        let json =
            serde_json::to_string(self).map_err(|error| StoreError::new(error.to_string()))?;
        store.set(&window_storage_key(key), json)
    }
}

/// Overlay stored geometry onto a descriptor when `persist_key` is set.
pub fn restore_window_geometry(
    settings: &mut crate::WindowDescriptor,
    store: &dyn PersistentStore,
) {
    let Some(key) = settings.persist_key.as_deref() else {
        return;
    };
    let Ok(Some(persisted)) = PersistedWindowGeometry::load(store, key) else {
        return;
    };
    persisted.apply(settings);
}

/// Win32 iconic windows often report outer origin near `-32000` before
/// `is_minimized` is true. Real multi-monitor origins never go this far.
fn looks_iconic(geometry: &crate::WindowGeometry) -> bool {
    const ICONIC_ORIGIN: f32 = -16_000.0;
    match geometry.logical_position {
        Some((x, y)) => {
            !x.is_finite() || !y.is_finite() || x <= ICONIC_ORIGIN || y <= ICONIC_ORIGIN
        }
        None => false,
    }
}

/// Record live geometry. Fullscreen and minimized frames are skipped so a
/// later restore does not reopen covering a display or at a minimized origin.
pub fn persist_live_window_geometry(
    store: &dyn PersistentStore,
    settings: &crate::WindowDescriptor,
    geometry: &crate::WindowGeometry,
    fullscreen: bool,
    minimized: bool,
) -> Result<(), StoreError> {
    let Some(key) = settings.persist_key.as_deref() else {
        return Ok(());
    };
    if fullscreen || minimized || looks_iconic(geometry) {
        return Ok(());
    }
    let mut persisted = PersistedWindowGeometry::load(store, key)?
        .unwrap_or_else(|| PersistedWindowGeometry::from_descriptor(settings));
    if !geometry.maximized {
        if let Some((x, y)) = geometry.logical_position {
            persisted.x = f64::from(x);
            persisted.y = f64::from(y);
        }
        persisted.width = f64::from(geometry.logical_size.0);
        persisted.height = f64::from(geometry.logical_size.1);
    }
    persisted.maximized = geometry.maximized;
    persisted.save(store, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::PersistentStore;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A directory no other test in this run can be handed.
    ///
    /// The timestamp alone is not enough: `SystemTime::now` is not
    /// nanosecond-resolution on every platform, so two tests starting in the
    /// same tick got the same directory and one would then read the other's
    /// deliberately corrupted file. The counter is what makes it unique; the
    /// timestamp only keeps two *runs* apart.
    fn unique_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nana-file-store-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_store_reopens_after_drop() {
        let dir = unique_dir();
        {
            let store = FileStore::open(&dir).unwrap();
            store.set("who", "nana".into()).unwrap();
            store.flush().unwrap();
        }
        let store = FileStore::open(&dir).unwrap();
        assert_eq!(store.get("who").unwrap().as_deref(), Some("nana"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_store_writes_atomically() {
        let dir = unique_dir();
        let store = FileStore::open(&dir).unwrap();
        store.set("k", "v".into()).unwrap();
        store.flush().unwrap();
        let bin = dir.join(STORAGE_FILE);
        assert!(bin.is_file());
        assert!(!tmp_path(&bin).exists());
        let entries = decode_map(&fs::read(&bin).unwrap()).expect("valid map");
        assert_eq!(entries.get("k").map(String::as_str), Some("v"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_store_recovers_from_corrupt_file() {
        let dir = unique_dir();
        let path = dir.join(STORAGE_FILE);
        fs::write(&path, "not-a-map").unwrap();
        let store = FileStore::open(&dir).unwrap();
        assert_eq!(store.get("k").unwrap(), None);
        store.set("k", "ok".into()).unwrap();
        store.flush().unwrap();
        assert!(dir.join("local-storage.bin.corrupt").is_file());
        assert_eq!(store.get("k").unwrap().as_deref(), Some("ok"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persisted_window_geometry_roundtrip() {
        let store = nana_ui_core::MemoryStore::new();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        settings.initial_size = (800.0, 600.0);
        settings.initial_position = Some((40.0, 50.0));
        PersistedWindowGeometry::from_descriptor(&settings)
            .save(&store, "main")
            .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (800.0, 600.0));
        assert_eq!(restored.initial_position, Some((40.0, 50.0)));
    }

    #[test]
    fn file_store_overwrites_existing_file() {
        let dir = unique_dir();
        {
            let store = FileStore::open(&dir).unwrap();
            store.set("k", "first".into()).unwrap();
            store.flush().unwrap();
            store.set("k", "second".into()).unwrap();
            store.flush().unwrap();
        }
        let store = FileStore::open(&dir).unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("second"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn decode_map_rejects_truncated_payload() {
        assert!(decode_map(b"NANA").is_none());
        let mut truncated =
            encode_map(&BTreeMap::from([("k".to_string(), "v".to_string())])).unwrap();
        truncated.pop();
        assert!(decode_map(&truncated).is_none());
    }

    fn geometry(x: f32, y: f32, width: f32, height: f32, maximized: bool) -> crate::WindowGeometry {
        crate::WindowGeometry {
            physical_position: Some((x as i32, y as i32)),
            physical_size: (width as u32, height as u32),
            logical_position: Some((x, y)),
            logical_size: (width, height),
            scale_factor: 1.0,
            maximized,
        }
    }

    #[test]
    fn invalid_window_json_does_not_block_later_saves() {
        let store = nana_ui_core::MemoryStore::new();
        store
            .set(&window_storage_key("main"), "not-geometry".into())
            .unwrap();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        restore_window_geometry(&mut settings, &store);
        assert_eq!(
            settings.initial_size,
            crate::WindowDescriptor::new("nana").initial_size
        );
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(40.0, 50.0, 640.0, 480.0, false),
            false,
            false,
        )
        .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (640.0, 480.0));
        assert_eq!(restored.initial_position, Some((40.0, 50.0)));
    }

    #[test]
    fn fullscreen_does_not_replace_saved_geometry() {
        let store = nana_ui_core::MemoryStore::new();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(10.0, 20.0, 800.0, 600.0, false),
            false,
            false,
        )
        .unwrap();
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(0.0, 0.0, 1920.0, 1080.0, false),
            true,
            false,
        )
        .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (800.0, 600.0));
        assert_eq!(restored.initial_position, Some((10.0, 20.0)));
    }

    #[test]
    fn maximized_keeps_last_normal_size() {
        let store = nana_ui_core::MemoryStore::new();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(10.0, 20.0, 800.0, 600.0, false),
            false,
            false,
        )
        .unwrap();
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(0.0, 0.0, 1920.0, 1080.0, true),
            false,
            false,
        )
        .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (800.0, 600.0));
        assert_eq!(restored.initial_position, Some((10.0, 20.0)));
        assert!(restored.maximized);
    }

    #[test]
    fn minimized_does_not_replace_saved_geometry() {
        let store = nana_ui_core::MemoryStore::new();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(10.0, 20.0, 800.0, 600.0, false),
            false,
            false,
        )
        .unwrap();
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(-32000.0, -32000.0, 160.0, 28.0, false),
            false,
            true,
        )
        .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (800.0, 600.0));
        assert_eq!(restored.initial_position, Some((10.0, 20.0)));
    }

    #[test]
    fn iconic_origin_is_skipped_without_minimized_flag() {
        let store = nana_ui_core::MemoryStore::new();
        let mut settings = crate::WindowDescriptor::new("nana");
        settings.persist_key = Some("main".into());
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(10.0, 20.0, 800.0, 600.0, false),
            false,
            false,
        )
        .unwrap();
        persist_live_window_geometry(
            &store,
            &settings,
            &geometry(-32000.0, -32000.0, 160.0, 28.0, false),
            false,
            false,
        )
        .unwrap();
        let mut restored = crate::WindowDescriptor::new("nana");
        restored.persist_key = Some("main".into());
        restore_window_geometry(&mut restored, &store);
        assert_eq!(restored.initial_size, (800.0, 600.0));
        assert_eq!(restored.initial_position, Some((10.0, 20.0)));
    }

    #[test]
    fn file_store_drops_leftover_tmp() {
        let dir = unique_dir();
        let dest = dir.join(STORAGE_FILE);
        let tmp = tmp_path(&dest);
        fs::write(
            &dest,
            encode_map(&BTreeMap::from([("k".to_string(), "old".to_string())])).unwrap(),
        )
        .unwrap();
        fs::write(&tmp, b"partial").unwrap();
        let store = FileStore::open(&dir).unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("old"));
        assert!(!tmp.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_file_overwrites_existing_dest() {
        let dir = unique_dir();
        let dest = dir.join("local-storage.bin");
        let tmp = tmp_path(&dest);
        fs::write(&dest, b"old").unwrap();
        fs::write(&tmp, b"new").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!tmp.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
