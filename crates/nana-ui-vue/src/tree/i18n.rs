//! A window's localization (Issue #267): the catalog and the locales the
//! application installs, applied to this window's world. Each window has its
//! own world, so the application's catalog and locale reach every window
//! through here. A change that alters what shows moves the world's
//! generation, which is what a host redraws on.

use std::collections::HashMap;
use std::sync::Arc;

use nana_ui_runtime::{LanguageTag, Locale, MessageCatalog, MissingMessage};

use super::{MAX_COMMIT_REJECTIONS, NanaTreeDocument};
use crate::i18n::CatalogRequest;

/// The catalog installed on a window, so installing the same again does
/// nothing, and the `message-args` errors reported on it.
#[derive(Debug, Default)]
pub(super) struct DocumentI18n {
    /// Fingerprint of the catalog last installed from JavaScript; `None` for
    /// none, or for one installed from Rust, which is never taken for one
    /// installed before.
    catalog: Option<u64>,
    /// The `message-args` error last reported per element, so a lasting one
    /// is reported once.
    pub(super) errors: HashMap<u64, String>,
    pending_errors: Vec<String>,
}

impl NanaTreeDocument {
    /// Install `catalog` unless `fingerprint` says this window has it
    /// already: an isolated window runs the application script again, and
    /// installing the same catalog again would resolve every localized node
    /// again. `catalog` is built only when it is installed. One with no
    /// fingerprint, from Rust, is always installed.
    pub(crate) fn install_catalog(
        &mut self,
        fingerprint: Option<u64>,
        catalog: impl FnOnce() -> Option<Arc<dyn MessageCatalog>>,
    ) {
        if fingerprint.is_some() && self.i18n.catalog == fingerprint {
            return;
        }
        self.runtime.context_mut().set_message_catalog(catalog());
        self.i18n.catalog = fingerprint;
    }

    /// [`Self::install_catalog`] for a catalog `Nana.i18n.setCatalog` sent,
    /// with its fallback locale and missing-message policy.
    pub(crate) fn install_catalog_request(&mut self, request: &CatalogRequest) {
        self.install_catalog(Some(request.fingerprint), || Some(request.table()));
        self.set_fallback_locale(request.fallback.clone());
        self.set_missing_message(request.missing);
    }

    /// Install the catalog localized text resolves from; every localized
    /// node resolves again.
    pub fn set_message_catalog(&mut self, catalog: Option<Arc<dyn MessageCatalog>>) {
        self.install_catalog(None, || catalog);
    }

    /// The locale every fallback chain ends in.
    pub fn set_fallback_locale(&mut self, locale: Option<LanguageTag>) {
        self.runtime.set_fallback_locale(locale);
    }

    /// What a message no locale has shows.
    pub fn set_missing_message(&mut self, policy: MissingMessage) {
        self.runtime.set_missing_message(policy);
    }

    /// The application's locale, as this window's world holds it.
    pub fn set_default_locale(&mut self, locale: Option<Locale>) {
        self.runtime.context_mut().set_default_locale(locale);
    }

    pub fn default_locale(&self) -> Option<&Locale> {
        self.runtime.default_locale()
    }

    /// This window's own locale; `None` takes the application's again.
    pub fn set_document_locale(&mut self, locale: Option<Locale>) {
        let document = self.runtime.document.document();
        self.runtime
            .context_mut()
            .set_document_locale(document, locale);
    }

    pub fn document_locale(&self) -> Option<&Locale> {
        self.runtime
            .document_locale(self.runtime.document.document())
    }

    /// Queue a `message-args` error for the diagnostics sink, once per
    /// distinct error on an element. The text was written when the attribute
    /// was parsed, not here on the frame path.
    pub(super) fn note_message_args_error(&mut self, raw_id: u64, error: Option<String>) {
        let Some(error) = error else {
            self.i18n.errors.remove(&raw_id);
            return;
        };
        if self.i18n.errors.get(&raw_id) == Some(&error) {
            return;
        }
        if self.i18n.pending_errors.len() < MAX_COMMIT_REJECTIONS {
            self.i18n.pending_errors.push(error.clone());
        }
        self.i18n.errors.insert(raw_id, error);
    }

    /// `message-args` errors since the last call; drained by the host into
    /// the JS diagnostics sink.
    pub fn take_i18n_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.i18n.pending_errors)
    }
}
