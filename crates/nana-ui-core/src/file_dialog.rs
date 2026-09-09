//! System file dialog request and result.
//!
//! Pure data. Opening the dialog needs the parent window handle, which only
//! the host has, so the platform work lives in `nana-window` and a control
//! never reaches it — `PathField` still just emits `BrowseRequested`, and the
//! application turns that into a request.
//!
//! This is the answering half of that contract. Before it, every consumer had
//! to pull in its own dialog crate to respond to a browse button.

use std::path::PathBuf;
use std::sync::Arc;

/// What the dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileDialogKind {
    /// One existing file.
    OpenFile,
    /// Several existing files.
    OpenFiles,
    /// A destination path, which may not exist yet.
    SaveFile,
    /// One existing directory.
    PickFolder,
}

impl FileDialogKind {
    /// Whether the dialog can return more than one path.
    pub fn is_multiple(self) -> bool {
        matches!(self, Self::OpenFiles)
    }

    /// Whether the chosen path is allowed not to exist.
    pub fn is_save(self) -> bool {
        matches!(self, Self::SaveFile)
    }
}

/// One named group of extensions in the dialog's type filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    /// Shown to the user, e.g. `图片`.
    pub name: Arc<str>,
    /// Extensions without the dot, e.g. `["png", "jpg"]`.
    pub extensions: Vec<Arc<str>>,
}

impl FileFilter {
    pub fn new(
        name: impl Into<Arc<str>>,
        extensions: impl IntoIterator<Item = impl Into<Arc<str>>>,
    ) -> Self {
        Self {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        }
    }
}

/// A dialog the application wants opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialogRequest {
    /// Echoed back in the result, so an application with several browse
    /// buttons knows which one answered.
    pub id: u32,
    pub kind: FileDialogKind,
    pub title: Option<Arc<str>>,
    /// Empty means every file type.
    pub filters: Vec<FileFilter>,
    pub directory: Option<PathBuf>,
    /// Pre-filled name for a save dialog.
    pub file_name: Option<Arc<str>>,
}

impl FileDialogRequest {
    pub fn new(id: u32, kind: FileDialogKind) -> Self {
        Self {
            id,
            kind,
            title: None,
            filters: Vec::new(),
            directory: None,
            file_name: None,
        }
    }

    pub fn title(mut self, title: impl Into<Arc<str>>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn filters(mut self, filters: impl IntoIterator<Item = FileFilter>) -> Self {
        self.filters = filters.into_iter().collect();
        self
    }

    /// Directory the dialog opens in.
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    /// Name a save dialog starts with.
    pub fn file_name(mut self, file_name: impl Into<Arc<str>>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }
}

/// What the user did.
///
/// A cancelled dialog is an empty `paths`, not an error: the user declining is
/// a normal outcome, and making callers match on an error to detect it invites
/// treating it as a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDialogResult {
    /// The `id` of the request this answers.
    pub id: u32,
    pub paths: Vec<PathBuf>,
}

impl FileDialogResult {
    pub fn cancelled(id: u32) -> Self {
        Self {
            id,
            paths: Vec::new(),
        }
    }

    /// Whether the user chose nothing.
    pub fn is_cancelled(&self) -> bool {
        self.paths.is_empty()
    }

    /// The single chosen path, for the dialogs that return at most one.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.paths.first().map(PathBuf::as_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_dialog_is_an_outcome_not_an_error() {
        let result = FileDialogResult::cancelled(3);
        assert!(result.is_cancelled());
        assert_eq!(result.path(), None);
        assert_eq!(result.id, 3);
    }

    #[test]
    fn kinds_declare_what_they_return() {
        assert!(FileDialogKind::OpenFiles.is_multiple());
        assert!(!FileDialogKind::OpenFile.is_multiple());
        assert!(FileDialogKind::SaveFile.is_save());
        assert!(!FileDialogKind::PickFolder.is_save());
    }

    #[test]
    fn a_request_carries_the_id_back_to_the_browse_button_that_asked() {
        let request = FileDialogRequest::new(7, FileDialogKind::OpenFile)
            .title("选择图片")
            .filters([FileFilter::new("图片", ["png", "jpg"])])
            .directory("/tmp");
        assert_eq!(request.id, 7);
        assert_eq!(request.filters[0].extensions.len(), 2);
        assert_eq!(FileDialogResult::cancelled(request.id).id, 7);
    }
}
