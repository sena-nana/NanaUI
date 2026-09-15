//! NanaUI storage: one string map, same contract as `localStorage`.
//!
//! The default implementation is in-memory. Hosts that want a session to
//! survive process exit inject a platform store (see `nana-ui-platform`).
//! Isolated Vue windows get a private map, not this handle.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Dock layout JSON under `{KEY_DOCK_PREFIX}{persist_key}`.
pub const KEY_DOCK_PREFIX: &str = "nana.dock.";
/// Appearance JSON under `{KEY_APPEARANCE_PREFIX}{persist_key}`.
pub const KEY_APPEARANCE_PREFIX: &str = "nana.appearance.";
/// Window geometry JSON under `{KEY_WINDOW_PREFIX}{persist_key}`.
pub const KEY_WINDOW_PREFIX: &str = "nana.window.";

/// `localStorage` key for a dock layout.
pub fn dock_storage_key(key: &str) -> String {
    format!("{KEY_DOCK_PREFIX}{key}")
}

/// `localStorage` key for appearance settings.
pub fn appearance_storage_key(key: &str) -> String {
    format!("{KEY_APPEARANCE_PREFIX}{key}")
}

/// `localStorage` key for window geometry.
pub fn window_storage_key(key: &str) -> String {
    format!("{KEY_WINDOW_PREFIX}{key}")
}

/// Framework chrome keys. `Nana.storage` `get` / `set` / `clear` / `remove` /
/// `keys` leave these; `localStorage` can still read, write, and wipe them.
pub fn is_framework_storage_key(key: &str) -> bool {
    key.starts_with(KEY_DOCK_PREFIX)
        || key.starts_with(KEY_APPEARANCE_PREFIX)
        || key.starts_with(KEY_WINDOW_PREFIX)
}

/// Failure from a [`PersistentStore`] operation.
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

/// UTF-8 string map. This *is* `localStorage` (plus `Nana.storage` JSON helpers
/// and framework keys with [`KEY_DOCK_PREFIX`] / [`KEY_APPEARANCE_PREFIX`] /
/// [`KEY_WINDOW_PREFIX`]).
pub trait PersistentStore: Send + Sync + fmt::Debug {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError>;
    fn set(&self, key: &str, value: String) -> Result<(), StoreError>;
    fn remove(&self, key: &str) -> Result<(), StoreError>;
    fn clear(&self) -> Result<(), StoreError>;
    fn keys(&self) -> Result<Vec<String>, StoreError>;
    fn flush(&self) -> Result<(), StoreError>;
}

/// Shared store handle installed into host APIs and `RuntimeProgramContext`.
pub type SharedStore = Arc<dyn PersistentStore>;

/// Wrap any [`PersistentStore`] for injection.
pub fn shared_store<S: PersistentStore + 'static>(store: S) -> SharedStore {
    Arc::new(store)
}

/// Default memory-only store. Values vanish when the last handle is dropped.
pub fn memory_store() -> SharedStore {
    shared_store(MemoryStore::new())
}

/// In-process [`PersistentStore`]. Default when the host does not inject one.
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: Mutex<BTreeMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PersistentStore for MemoryStore {
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
