//! System file dialog request and result.
//!
//! Pure data. Opening the dialog needs the parent window handle, which only
//! the host has, so the platform work lives in `nana-window` and a control
//! never reaches it — `PathField` still just emits `BrowseRequested`, and the
//! application turns that into a request.
//!
//! This is the answering half of that contract. Before it, every consumer had
//! to pull in its own dialog crate to respond to a browse button.

use std::path::{Path, PathBuf};
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
    /// Several existing directories.
    PickFolders,
}

impl FileDialogKind {
    /// Whether the dialog can return more than one path.
    pub fn is_multiple(self) -> bool {
        matches!(self, Self::OpenFiles | Self::PickFolders)
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
///
/// Built with [`FileDialogRequest::new`] and the builder methods, so a new
/// option does not break callers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileDialogRequest {
    /// Echoed back in the result, so an application with several browse
    /// buttons knows which one answered.
    pub id: u64,
    pub kind: FileDialogKind,
    pub title: Option<Arc<str>>,
    /// Empty means every file type.
    pub filters: Vec<FileFilter>,
    pub directory: Option<PathBuf>,
    /// Pre-filled name for a save dialog.
    pub file_name: Option<Arc<str>>,
    /// Save dialogs only: when the chosen name does not end with one of the
    /// selected filter's extensions, append its first one. Off by default.
    /// See [`FileDialogRequest::apply_extension_policy`].
    pub enforce_extension: bool,
}

impl FileDialogRequest {
    pub fn new(id: u64, kind: FileDialogKind) -> Self {
        Self {
            id,
            kind,
            title: None,
            filters: Vec::new(),
            directory: None,
            file_name: None,
            enforce_extension: false,
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

    /// Make a save dialog's result carry the selected type's extension.
    pub fn enforce_extension(mut self, enforce: bool) -> Self {
        self.enforce_extension = enforce;
        self
    }

    /// Applies [`FileDialogRequest::enforce_extension`] to a completed save.
    ///
    /// Only the end of the file name is compared, ignoring case, so `v1.2`
    /// becomes `v1.2.json` instead of losing `.2`. The extensions are those of
    /// `result.selected_filter`; when the platform did not report one, a name
    /// ending with any requested extension is kept and the first one is
    /// appended otherwise. A wildcard filter accepts every name.
    pub fn apply_extension_policy(&self, mut result: FileDialogResult) -> FileDialogResult {
        if !self.enforce_extension || !self.kind.is_save() {
            return result;
        }
        let filters = match result
            .selected_filter
            .and_then(|index| self.filters.get(index))
        {
            Some(filter) => std::slice::from_ref(filter),
            None => self.filters.as_slice(),
        };
        for path in &mut result.paths {
            if let Some(extension) = missing_extension(path, filters) {
                let name = path.as_mut_os_string();
                name.push(".");
                name.push(extension);
            }
        }
        result
    }
}

/// The extension to append to `path`, or `None` when its name already ends
/// with an accepted one (or any name is accepted).
fn missing_extension<'a>(path: &Path, filters: &'a [FileFilter]) -> Option<&'a str> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let mut first = None;
    for extension in filters.iter().flat_map(|filter| &filter.extensions) {
        if extension.is_empty() || extension.as_ref() == "*" {
            return None;
        }
        let suffix = extension.to_lowercase();
        if name
            .strip_suffix(suffix.as_str())
            .is_some_and(|stem| stem.ends_with('.'))
        {
            return None;
        }
        first.get_or_insert(extension.as_ref());
    }
    first
}

/// A host or platform failure, distinct from the user choosing Cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDialogError {
    /// This window already owns an active request.
    Busy,
    /// The active request already uses this identity; it remains active.
    DuplicateRequest,
    /// The requested window closed or does not exist.
    WindowClosed,
    /// No backend is available on this target.
    Unavailable,
    /// An observable backend or worker failure.
    Platform(String),
}

/// What the user did.
///
/// A cancelled dialog is an empty `paths`, not an error: the user declining is
/// a normal outcome, and making callers match on an error to detect it invites
/// treating it as a failure.
///
/// Built with the constructors below, so a new field does not break hosts
/// that inject results.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileDialogResult {
    /// The `id` of the request this answers.
    pub id: u64,
    pub paths: Vec<PathBuf>,
    pub error: Option<FileDialogError>,
    /// 用户选择的过滤器索引（从零开始）；平台未提供时为 None。
    pub selected_filter: Option<usize>,
}

impl FileDialogResult {
    pub fn selected(id: u64, paths: Vec<PathBuf>) -> Self {
        Self {
            id,
            paths,
            error: None,
            selected_filter: None,
        }
    }

    pub fn failed(id: u64, error: FileDialogError) -> Self {
        Self {
            id,
            paths: Vec::new(),
            error: Some(error),
            selected_filter: None,
        }
    }

    pub fn cancelled(id: u64) -> Self {
        Self {
            id,
            paths: Vec::new(),
            error: None,
            selected_filter: None,
        }
    }

    /// 保留平台选中的文件类型，同时兼容仅返回路径的后端。
    pub fn with_selected_filter(mut self, index: Option<usize>) -> Self {
        self.selected_filter = index;
        self
    }

    /// Whether the user chose nothing.
    pub fn is_cancelled(&self) -> bool {
        self.error.is_none() && self.paths.is_empty()
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
        assert_eq!(result.selected_filter, None);
    }

    /// 类型索引独立于路径后缀保存，缺少类型回执的平台仍可只提供路径。
    #[test]
    fn selected_filter_roundtrips_without_reinterpreting_the_path() {
        let path = PathBuf::from("剧本.json");
        let selected =
            FileDialogResult::selected(7, vec![path.clone()]).with_selected_filter(Some(2));
        assert_eq!(selected.selected_filter, Some(2));
        assert_eq!(selected.path(), Some(path.as_path()));
        assert_eq!(
            FileDialogResult::selected(8, vec![path]).selected_filter,
            None
        );
        assert_eq!(
            FileDialogResult::failed(9, FileDialogError::Busy).selected_filter,
            None
        );
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
        assert!(!request.enforce_extension);
    }

    fn save(name: &str, filter: Option<usize>) -> PathBuf {
        let request = FileDialogRequest::new(1, FileDialogKind::SaveFile)
            .filters([
                FileFilter::new("JSON", ["json"]),
                FileFilter::new("文本", ["txt", "md"]),
            ])
            .enforce_extension(true);
        let result = FileDialogResult::selected(1, vec![PathBuf::from("out").join(name)])
            .with_selected_filter(filter);
        let result = request.apply_extension_policy(result);
        assert_eq!(result.selected_filter, filter);
        result.paths.into_iter().next().unwrap()
    }

    #[test]
    fn a_dot_inside_the_name_is_not_the_selected_extension() {
        assert_eq!(save("v1.2", Some(0)), PathBuf::from("out/v1.2.json"));
        assert_eq!(save("剧本", Some(1)), PathBuf::from("out/剧本.txt"));
    }

    #[test]
    fn a_name_with_the_selected_extension_is_kept() {
        assert_eq!(save("v1.2.json", Some(0)), PathBuf::from("out/v1.2.json"));
        assert_eq!(save("notes.md", Some(1)), PathBuf::from("out/notes.md"));
        // Another type's extension is not the selected one.
        assert_eq!(
            save("notes.md", Some(0)),
            PathBuf::from("out/notes.md.json")
        );
        // A bare suffix without its dot is part of the name.
        assert_eq!(save("ajson", Some(0)), PathBuf::from("out/ajson.json"));
    }

    #[test]
    fn extension_case_is_ignored() {
        assert_eq!(
            save("REPORT.JSON", Some(0)),
            PathBuf::from("out/REPORT.JSON")
        );
        assert_eq!(save("Notes.Txt", Some(1)), PathBuf::from("out/Notes.Txt"));
    }

    #[test]
    fn without_a_reported_type_any_requested_extension_is_kept() {
        assert_eq!(save("notes.md", None), PathBuf::from("out/notes.md"));
        assert_eq!(save("v1.2", None), PathBuf::from("out/v1.2.json"));
        // An index the request does not have is treated as unreported.
        assert_eq!(save("notes.txt", Some(9)), PathBuf::from("out/notes.txt"));
    }

    #[test]
    fn the_policy_is_opt_in_and_save_only() {
        let filters = [FileFilter::new("JSON", ["json"])];
        let picked = || FileDialogResult::selected(2, vec![PathBuf::from("v1.2")]);
        let off = FileDialogRequest::new(2, FileDialogKind::SaveFile).filters(filters.clone());
        assert_eq!(off.apply_extension_policy(picked()), picked());
        let open = FileDialogRequest::new(2, FileDialogKind::OpenFile)
            .filters(filters)
            .enforce_extension(true);
        assert_eq!(open.apply_extension_policy(picked()), picked());
        let any = FileDialogRequest::new(2, FileDialogKind::SaveFile)
            .filters([FileFilter::new("全部", ["*"])])
            .enforce_extension(true);
        assert_eq!(any.apply_extension_policy(picked()), picked());
        let cancelled = FileDialogResult::cancelled(2);
        assert_eq!(any.apply_extension_policy(cancelled.clone()), cancelled);
    }

    #[test]
    fn constructors_report_no_selected_filter() {
        assert_eq!(FileDialogResult::cancelled(1).selected_filter, None);
        assert_eq!(
            FileDialogResult::failed(1, FileDialogError::Busy).selected_filter,
            None
        );
        let result = FileDialogResult::selected(1, Vec::new()).with_selected_filter(Some(2));
        assert_eq!(result.selected_filter, Some(2));
    }
}
