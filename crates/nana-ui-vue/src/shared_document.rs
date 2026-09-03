//! A scoped view of the Runtime owned by the Vue document.
//! The DOM facade and its Runtime use the same lock, so a host callback cannot
//! mutate either side while the other is borrowed. Never hold this scope when
//! invoking JS; reentrant access returns Busy instead of blocking.

use crate::NanaTreeDocument;
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};
use std::sync::{Arc, Mutex, TryLockError};

pub struct SharedRuntimeDocument {
    document: Arc<Mutex<NanaTreeDocument>>,
}

impl SharedRuntimeDocument {
    pub(crate) fn new(document: Arc<Mutex<NanaTreeDocument>>) -> Self {
        Self { document }
    }

    pub fn with_document<R>(
        &self,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<R, DocumentAccessError> {
        let document = self.document.try_lock().map_err(access_error)?;
        Ok(f(document.runtime_document()))
    }

    pub fn with_document_mut<R>(
        &self,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<R, DocumentAccessError> {
        let mut document = self.document.try_lock().map_err(access_error)?;
        Ok(f(document.runtime_document_mut()))
    }
}

fn access_error<T>(error: TryLockError<T>) -> DocumentAccessError {
    match error {
        TryLockError::WouldBlock => DocumentAccessError::Busy,
        TryLockError::Poisoned(_) => DocumentAccessError::Poisoned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_access_is_scoped_and_reentry_returns_busy() {
        let host = crate::VueHost::new();
        let shared = host.shared_runtime_document();
        let other = host.shared_runtime_document();
        shared
            .with_document(|document| {
                assert_eq!(document.document().get(), 1);
                assert_eq!(
                    other.with_document_mut(|_| ()),
                    Err(DocumentAccessError::Busy)
                );
            })
            .unwrap();
        other
            .with_document_mut(|document| {
                assert_eq!(shared.with_document(|_| ()), Err(DocumentAccessError::Busy));
                document.context_mut().world_mut().begin_frame_counters();
            })
            .unwrap();
        assert!(shared.with_document(|_| ()).is_ok());
    }

    #[test]
    fn facade_access_and_callback_panics_are_reported() {
        let host = crate::VueHost::new();
        let shared = host.shared_runtime_document();
        let facade = host.document();
        let guard = facade.lock().unwrap();
        assert_eq!(shared.with_document(|_| ()), Err(DocumentAccessError::Busy));
        drop(guard);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = shared.with_document_mut(|_| panic!("callback failure"));
        }));
        assert_eq!(
            shared.with_document(|_| ()),
            Err(DocumentAccessError::Poisoned)
        );
    }
    #[test]
    fn windows_have_independent_scopes_and_release_on_last_owner() {
        let first = crate::VueHost::new();
        let second = crate::VueHost::new();
        let shared = first.shared_runtime_document();
        let other = second.shared_runtime_document();
        let weak = Arc::downgrade(&first.document());
        shared
            .with_document_mut(|_| {
                other
                    .with_document_mut(|_| ())
                    .expect("another window is independent");
            })
            .unwrap();
        drop(first);
        shared
            .with_document(|_| ())
            .expect("live handle owns its document");
        drop(shared);
        assert!(
            weak.upgrade().is_none(),
            "last owner releases the closed document"
        );
        assert!(other.with_document(|_| ()).is_ok());
    }
    #[test]
    fn reentrant_js_host_op_returns_error_and_can_retry_after_scope() {
        let host = crate::VueHost::new();
        let shared = host.shared_runtime_document();
        let api = host.host_api_registry();
        shared
            .with_document_mut(|_| {
                let result = api.call("createElement", &[nana_js_engine::HostValue::string("div")]);
                assert!(
                    result.is_err(),
                    "host op must fail while the document is borrowed"
                );
            })
            .unwrap();
        assert!(
            api.call("createElement", &[nana_js_engine::HostValue::string("div")])
                .is_ok()
        );
    }
}
