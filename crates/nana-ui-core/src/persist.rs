//! Physical persistence backend and independent domain capabilities.
//!
//! The default implementation is in-memory. Hosts that want a session to
//! survive process exit inject a platform store (see `nana-ui-platform`).
//! Isolated Vue windows get a private map, not this handle.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Legacy Dock layout key used as migration input.
pub const KEY_DOCK_PREFIX: &str = "nana.dock.";
/// Legacy Appearance key used as migration input.
pub const KEY_APPEARANCE_PREFIX: &str = "nana.appearance.";
/// Legacy Window geometry key used as migration input.
pub const KEY_WINDOW_PREFIX: &str = "nana.window.";

/// Reserved physical namespace for application `localStorage` entries.
pub const APP_STORAGE_PREFIX: &str = "nana.app.";
/// Reserved physical namespace for canonical framework view state.
pub const VIEW_STATE_PREFIX: &str = "nana.view.v1.";
/// Reserved physical namespace for application/user settings.
pub const SETTINGS_PREFIX: &str = "nana.settings.v1.";

/// Legacy key for a dock layout.
pub fn dock_storage_key(key: &str) -> String {
    format!("{KEY_DOCK_PREFIX}{key}")
}

/// Legacy key for appearance settings.
pub fn appearance_storage_key(key: &str) -> String {
    format!("{KEY_APPEARANCE_PREFIX}{key}")
}

/// Legacy key for window geometry.
pub fn window_storage_key(key: &str) -> String {
    format!("{KEY_WINDOW_PREFIX}{key}")
}

/// Recognizes legacy framework keys during migration.
pub fn is_framework_storage_key(key: &str) -> bool {
    key.starts_with(KEY_DOCK_PREFIX)
        || key.starts_with(KEY_APPEARANCE_PREFIX)
        || key.starts_with(KEY_WINDOW_PREFIX)
}

/// Stable scope component; serialized as an opaque string, never an Entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct RestorationScopeId(String);
impl RestorationScopeId {
    pub fn new(value: impl Into<String>) -> Result<Self, StoreError> {
        let value = value.into();
        if value.is_empty() || value.contains('\0') {
            return Err(StoreError::new("invalid restoration scope"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for RestorationScopeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
pub type RestorationKey = RestorationScopeId;
pub type ViewStateSchemaVersion = u32;

/// Structural path encoding keeps arbitrary user keys and sibling scopes distinct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RestorationPath(Vec<RestorationScopeId>);
impl RestorationPath {
    pub fn new(parts: impl IntoIterator<Item = RestorationScopeId>) -> Self {
        Self(parts.into_iter().collect())
    }
    pub fn root() -> Self {
        Self::default()
    }
    pub fn push(&self, part: RestorationScopeId) -> Self {
        let mut next = self.0.clone();
        next.push(part);
        Self(next)
    }
    pub fn key(&self) -> String {
        self.0
            .iter()
            .map(|p| format!("{}:{}", p.0.len(), p.0))
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewStateEnvelope {
    pub schema_version: ViewStateSchemaVersion,
    pub payload: String,
}

/// Frontend capability: no access to raw physical keys or backend flush.
#[derive(Debug, Clone)]
pub struct LocalStorageAdapter {
    backend: SharedStore,
}
impl LocalStorageAdapter {
    pub fn new(backend: SharedStore) -> Self {
        Self { backend }
    }
    fn physical(key: &str) -> String {
        format!("{APP_STORAGE_PREFIX}{key}")
    }
    pub fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.backend.get(&Self::physical(key))
    }
    pub fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
        self.backend.set(&Self::physical(key), value)
    }
    pub fn remove(&self, key: &str) -> Result<(), StoreError> {
        self.backend.remove(&Self::physical(key))
    }
    pub fn clear(&self) -> Result<(), StoreError> {
        for key in self.keys()? {
            self.remove(&key)?;
        }
        Ok(())
    }
    pub fn keys(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .backend
            .keys()?
            .into_iter()
            .filter_map(|k| k.strip_prefix(APP_STORAGE_PREFIX).map(ToOwned::to_owned))
            .collect())
    }
}

/// Canonical view-state capability, independently scoped from settings and app KV.
#[derive(Debug, Clone)]
pub struct ViewStateStore {
    backend: SharedStore,
    scope: RestorationPath,
}
impl ViewStateStore {
    pub fn new(backend: SharedStore, scope: RestorationPath) -> Self {
        Self { backend, scope }
    }
    pub fn scope(&self) -> &RestorationPath {
        &self.scope
    }
    fn physical(&self, path: &RestorationPath) -> String {
        format!("{VIEW_STATE_PREFIX}{}/{}", self.scope.key(), path.key())
    }
    pub fn child(&self, part: RestorationScopeId) -> Self {
        Self::new(Arc::clone(&self.backend), self.scope.push(part))
    }
    pub fn read(&self, path: &RestorationPath, version: u32) -> Result<Option<String>, StoreError> {
        let key = self.physical(path);
        let Some(raw) = self.backend.get(&key)? else {
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_MISSES);
            return Ok(None);
        };
        match serde_json::from_str::<ViewStateEnvelope>(&raw) {
            Ok(v) if v.schema_version == version => {
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_HITS);
                Ok(Some(v.payload))
            }
            Ok(_) => {
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::persistence::SCHEMA_MISMATCHES
                );
                self.backend.remove(&key)?;
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_MISSES);
                Ok(None)
            }
            _ => {
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::CORRUPTIONS);
                self.backend.remove(&key)?;
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_MISSES);
                Ok(None)
            }
        }
    }
    pub fn write(
        &self,
        path: &RestorationPath,
        schema_version: u32,
        payload: String,
    ) -> Result<(), StoreError> {
        let value = serde_json::to_string(&ViewStateEnvelope {
            schema_version,
            payload,
        })
        .map_err(|e| StoreError::new(e.to_string()))?;
        self.backend.set(&self.physical(path), value)
    }
    pub fn remove(&self, path: &RestorationPath) -> Result<(), StoreError> {
        self.backend.remove(&self.physical(path))
    }
    pub fn reset_scope(&self) -> Result<(), StoreError> {
        let prefix = if self.scope.key().is_empty() {
            VIEW_STATE_PREFIX.to_string()
        } else {
            format!("{VIEW_STATE_PREFIX}{}/", self.scope.key())
        };
        for key in self
            .backend
            .keys()?
            .into_iter()
            .filter(|key| key.starts_with(&prefix))
        {
            self.backend.remove(&key)?;
        }
        Ok(())
    }
    /// Import a legacy value only after its domain codec accepts it. Canonical
    /// data wins, including a damaged canonical entry (never resurrect legacy).
    pub fn restore<T>(
        &self,
        kind: &str,
        key: &str,
        version: u32,
        decode: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<T>, StoreError> {
        let path =
            RestorationPath::new([RestorationScopeId::new(kind)?, RestorationKey::new(key)?]);
        let legacy = format!("nana.{kind}.{key}");
        if self.backend.get(&self.physical(&path))?.is_some() {
            let result = self.read(&path, version)?.and_then(|raw| decode(&raw));
            if result.is_none() {
                self.remove(&path)?;
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::CORRUPTIONS);
            }
            self.backend.remove(&legacy)?;
            return Ok(result);
        }
        let Some(raw) = self.backend.get(&legacy)? else {
            return Ok(None);
        };
        let result = decode(&raw);
        if result.is_some() {
            self.write(&path, version, raw)?;
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::MIGRATIONS);
        } else {
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::CORRUPTIONS);
        }
        self.backend.remove(&legacy)?;
        Ok(result)
    }
    pub fn save(
        &self,
        kind: &str,
        key: &str,
        version: u32,
        payload: String,
    ) -> Result<(), StoreError> {
        let path =
            RestorationPath::new([RestorationScopeId::new(kind)?, RestorationKey::new(key)?]);
        self.write(&path, version, payload)?;
        self.backend.remove(&format!("nana.{kind}.{key}"))
    }
}

/// User/application settings are not restoration data.
#[derive(Debug, Clone)]
pub struct AppSettings {
    backend: SharedStore,
}
impl AppSettings {
    pub fn new(backend: SharedStore) -> Self {
        Self { backend }
    }
    pub fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.backend.get(&format!("{SETTINGS_PREFIX}{key}"))
    }
    pub fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
        self.backend.set(&format!("{SETTINGS_PREFIX}{key}"), value)
    }
    pub fn restore_appearance<T>(
        &self,
        key: &str,
        decode: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<T>, StoreError> {
        let legacy = appearance_storage_key(key);
        if let Some(raw) = self.get(key)? {
            self.backend.remove(&legacy)?;
            let result = decode(&raw);
            if result.is_some() {
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_HITS);
            } else {
                self.backend.remove(&format!("{SETTINGS_PREFIX}{key}"))?;
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::CORRUPTIONS);
                nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_MISSES);
            }
            return Ok(result);
        }
        let Some(raw) = self.backend.get(&legacy)? else {
            return Ok(None);
        };
        let result = decode(&raw);
        if result.is_some() {
            self.set(key, raw)?;
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::MIGRATIONS);
        }
        if result.is_some() {
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_HITS);
        } else {
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::CORRUPTIONS);
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::RESTORE_MISSES);
        }
        self.backend.remove(&legacy)?;
        Ok(result)
    }
}

/// Failure from a [`KvBackend`] operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    pub message: String,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn poisoned() -> Self {
        Self::new("persistent store poisoned")
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

/// UTF-8 physical string map. Frontend `localStorage` access is restricted to
/// [`APP_STORAGE_PREFIX`] by [`LocalStorageAdapter`]; framework capabilities
/// use [`ViewStateStore`] and [`AppSettings`] instead.
pub trait KvBackend: Send + Sync + fmt::Debug {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError>;
    fn set(&self, key: &str, value: String) -> Result<(), StoreError>;
    fn remove(&self, key: &str) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
    fn keys(&self) -> Result<Vec<String>, StoreError>;
    fn flush(&self) -> Result<(), StoreError>;
}

/// Shared store handle installed into host APIs and `RuntimeProgramContext`.
pub type SharedStore = Arc<dyn KvBackend>;

/// Wrap any [`KvBackend`] for injection.
pub fn shared_store<S: KvBackend + 'static>(store: S) -> SharedStore {
    Arc::new(store)
}

/// Default memory-only store. Values vanish when the last handle is dropped.
pub fn memory_store() -> SharedStore {
    shared_store(MemoryStore::new())
}

/// In-process [`KvBackend`]. Default when the host does not inject one.
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: Mutex<BTreeMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KvBackend for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        let entries = self.entries.lock().map_err(|_| StoreError::poisoned())?;
        Ok(entries.get(key).cloned())
    }

    fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
        let mut entries = self.entries.lock().map_err(|_| StoreError::poisoned())?;
        if entries.get(key).is_some_and(|current| current == &value) {
            return Ok(());
        }
        entries.insert(key.to_string(), value);
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StoreError> {
        let mut entries = self.entries.lock().map_err(|_| StoreError::poisoned())?;
        entries.remove(key);
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreError> {
        let mut entries = self.entries.lock().map_err(|_| StoreError::poisoned())?;
        entries.clear();
        Ok(())
    }

    fn keys(&self) -> Result<Vec<String>, StoreError> {
        let entries = self.entries.lock().map_err(|_| StoreError::poisoned())?;
        Ok(entries.keys().cloned().collect())
    }

    fn flush(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip_is_one_map() {
        let store = MemoryStore::new();
        store.set("who", "nana".into()).unwrap();
        store.set("doc", "{\"n\":1}".into()).unwrap();
        assert_eq!(store.get("who").unwrap().as_deref(), Some("nana"));
        assert_eq!(store.get("doc").unwrap().as_deref(), Some("{\"n\":1}"));
        assert_eq!(
            store.keys().unwrap(),
            vec!["doc".to_string(), "who".to_string()]
        );
        store.remove("who").unwrap();
        assert_eq!(store.get("who").unwrap(), None);
        store.set("a", "1".into()).unwrap();
        store
            .set(&dock_storage_key("gallery"), "layout".into())
            .unwrap();
        store.clear().unwrap();
        assert!(store.keys().unwrap().is_empty());
    }

    #[test]
    fn last_write_wins_on_the_same_key() {
        let store = MemoryStore::new();
        store.set("session", "plain".into()).unwrap();
        store.set("session", "{\"user\":\"nana\"}".into()).unwrap();
        assert_eq!(
            store.get("session").unwrap().as_deref(),
            Some("{\"user\":\"nana\"}")
        );
    }

    #[test]
    fn framework_storage_keys_are_prefixed() {
        assert!(is_framework_storage_key(&dock_storage_key("gallery")));
        assert!(is_framework_storage_key(&appearance_storage_key("gallery")));
        assert!(is_framework_storage_key(&window_storage_key("main")));
        assert!(!is_framework_storage_key("session"));
        assert!(!is_framework_storage_key("nana"));
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    fn path(s: &str) -> RestorationPath {
        RestorationPath::new([RestorationScopeId::new(s).unwrap()])
    }
    #[test]
    fn paths_and_reset_are_structural() {
        let backend = memory_store();
        let root = ViewStateStore::new(backend, RestorationPath::root());
        let a = root.child(RestorationScopeId::new("profile/a").unwrap());
        let b = root
            .child(RestorationScopeId::new("profile").unwrap())
            .child(RestorationScopeId::new("a").unwrap());
        a.write(&path("main"), 1, "a".into()).unwrap();
        b.write(&path("main"), 1, "b".into()).unwrap();
        a.reset_scope().unwrap();
        assert_eq!(b.read(&path("main"), 1).unwrap().as_deref(), Some("b"));
        root.reset_scope().unwrap();
        assert_eq!(b.read(&path("main"), 1).unwrap(), None);
    }
    #[test]
    fn migration_is_once_and_bad_entry_is_local() {
        let backend = memory_store();
        backend.set("nana.window.main", "42".into()).unwrap();
        backend.set("nana.window.bad", "bad".into()).unwrap();
        let store = ViewStateStore::new(backend.clone(), RestorationPath::root());
        assert_eq!(
            store
                .restore("window", "main", 1, |s| s.parse::<u32>().ok())
                .unwrap(),
            Some(42)
        );
        assert_eq!(
            store
                .restore("window", "bad", 1, |s| s.parse::<u32>().ok())
                .unwrap(),
            None
        );
        assert_eq!(backend.get("nana.window.main").unwrap(), None);
        backend.set("nana.window.main", "99".into()).unwrap();
        assert_eq!(
            store
                .restore("window", "main", 1, |s| s.parse::<u32>().ok())
                .unwrap(),
            Some(42)
        );
        assert_eq!(
            store
                .restore("window", "main", 2, |s| s.parse::<u32>().ok())
                .unwrap(),
            None
        );
    }
    #[test]
    fn canonical_schema_or_payload_failure_resets_only_that_scope() {
        let backend = memory_store();
        let root = ViewStateStore::new(backend.clone(), RestorationPath::root());
        let first = root.child(RestorationScopeId::new("profile/one").unwrap());
        let second = root.child(RestorationScopeId::new("profile/two").unwrap());
        let state = path("window");
        first.write(&state, 1, "one".into()).unwrap();
        second.write(&state, 1, "two".into()).unwrap();
        backend
            .set(
                &first.physical(&state),
                r#"{"schema_version":99,"payload":"old"}"#.into(),
            )
            .unwrap();
        assert_eq!(first.read(&state, 1).unwrap(), None);
        assert_eq!(second.read(&state, 1).unwrap().as_deref(), Some("two"));
        first.write(&state, 1, "one".into()).unwrap();
        backend
            .set(&first.physical(&state), "malformed".into())
            .unwrap();
        assert_eq!(first.read(&state, 1).unwrap(), None);
        assert_eq!(second.read(&state, 1).unwrap().as_deref(), Some("two"));
    }
    #[test]
    fn app_clear_cannot_reset_views_or_settings() {
        let backend = memory_store();
        let views = ViewStateStore::new(backend.clone(), RestorationPath::root());
        let app = LocalStorageAdapter::new(backend.clone());
        let settings = AppSettings::new(backend);
        views.write(&path("dock"), 1, "layout".into()).unwrap();
        settings.set("appearance", "dark".into()).unwrap();
        app.set("nana.view.v1.4:dock", "forged".into()).unwrap();
        app.clear().unwrap();
        assert_eq!(
            views.read(&path("dock"), 1).unwrap().as_deref(),
            Some("layout")
        );
        assert_eq!(settings.get("appearance").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn malformed_canonical_appearance_is_removed_locally() {
        let backend = memory_store();
        let settings = AppSettings::new(backend.clone());
        settings.set("appearance", "not-json".into()).unwrap();
        assert!(
            settings
                .restore_appearance("appearance", |raw| Some(raw.to_owned()))
                .unwrap()
                .is_some()
        );
        settings.set("appearance", "not-json".into()).unwrap();
        assert!(
            settings
                .restore_appearance("appearance", |raw| raw.parse::<u64>().ok())
                .unwrap()
                .is_none()
        );
        assert_eq!(settings.get("appearance").unwrap(), None);
    }
}
