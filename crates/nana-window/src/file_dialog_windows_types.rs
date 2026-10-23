//! Windows 保存类型联动：在原生对话框中确定后缀，保留最终路径的覆盖确认。
use super::{FileDialogRequest, dialog_file_name, shell_item_path, wide};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::UI::Shell::{
    FDE_OVERWRITE_RESPONSE, FDE_SHAREVIOLATION_RESPONSE, FDEOR_DEFAULT, FDESVR_DEFAULT,
    IFileDialog, IFileDialogEvents, IFileDialogEvents_Impl, IShellItem,
};
use windows::core::{Error, HRESULT, PCWSTR, Ref, Result, implement};

/// 原生过滤器从 1 开始；应用契约从 0 开始，并检查平台返回的范围。
pub(super) fn selected_filter(dialog: &IFileDialog, count: usize) -> Option<usize> {
    let index = unsafe { dialog.GetFileTypeIndex() }.ok()?.checked_sub(1)? as usize;
    (index < count).then_some(index)
}

pub(super) struct Subscription {
    dialog: IFileDialog,
    cookie: u32,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // 对话框仍存活，先解除事件订阅再释放 COM 对象。
        let _ = unsafe { self.dialog.Unadvise(self.cookie) };
    }
}

/// 仅保存对话框需要后缀联动；打开对话框继续使用平台原有过滤行为。
pub(super) fn subscribe(
    dialog: &IFileDialog,
    request: &FileDialogRequest,
) -> Result<Option<Subscription>> {
    if !request.kind.is_save() || request.filters.is_empty() {
        return Ok(None);
    }
    let events: IFileDialogEvents = TypeEvents {
        extensions: request
            .filters
            .iter()
            .map(|filter| filter.extensions.iter().map(ToString::to_string).collect())
            .collect(),
    }
    .into();
    let cookie = unsafe { dialog.Advise(&events) }?;
    Ok(Some(Subscription {
        dialog: dialog.clone(),
        cookie,
    }))
}

#[implement(IFileDialogEvents)]
struct TypeEvents {
    extensions: Vec<Vec<String>>,
}

impl TypeEvents {
    fn current(&self, dialog: &IFileDialog) -> Option<&[String]> {
        self.extensions
            .get(selected_filter(dialog, self.extensions.len())?)
            .map(Vec::as_slice)
    }

    /// 切换类型时同步默认后缀，并替换上一个类型留下的可见后缀。
    fn change_type(&self, dialog: &IFileDialog) -> Result<()> {
        let Some(extensions) = self.current(dialog) else {
            return Ok(());
        };
        let Some(extension) = extensions.first().filter(|value| value.as_str() != "*") else {
            return Ok(());
        };
        let value = wide(extension);
        unsafe { dialog.SetDefaultExtension(PCWSTR(value.as_ptr())) }?;
        if let Some(name) = dialog_file_name(dialog) {
            let path = Path::new(&name);
            let previous = path.extension().and_then(|v| v.to_str());
            if previous.is_some_and(|old| {
                self.extensions
                    .iter()
                    .flatten()
                    .any(|v| v.eq_ignore_ascii_case(old))
            }) && !accepts(path, extensions)
            {
                set_name(dialog, &path.with_extension(extension))?;
            }
        }
        Ok(())
    }
}

#[allow(non_snake_case)]
impl IFileDialogEvents_Impl for TypeEvents_Impl {
    fn OnTypeChange(&self, dialog: Ref<IFileDialog>) -> Result<()> {
        if let Some(dialog) = dialog.as_ref() {
            self.change_type(dialog)?;
        }
        Ok(())
    }

    fn OnFileOk(&self, dialog: Ref<IFileDialog>) -> Result<()> {
        let Some(dialog) = dialog.as_ref() else {
            return Ok(());
        };
        let Some(extensions) = self.current(dialog) else {
            return Ok(());
        };
        if extensions.iter().any(|ext| ext == "*") {
            return Ok(());
        }
        let item = unsafe { dialog.GetResult() }?;
        let path = shell_item_path(&item)
            .map_err(|error| Error::new(HRESULT(0x80004005u32 as i32), format!("{error:?}")))?;
        if !accepts(&path, extensions)
            && let Some(extension) = extensions.first()
        {
            set_name(dialog, &path.with_extension(extension))?;
            // S_FALSE 保持窗口打开，让原生保存流程对最终文件再次执行覆盖检查。
            return Err(Error::from_hresult(HRESULT(1)));
        }
        Ok(())
    }

    fn OnFolderChanging(&self, _: Ref<IFileDialog>, _: Ref<IShellItem>) -> Result<()> {
        Ok(())
    }
    fn OnFolderChange(&self, _: Ref<IFileDialog>) -> Result<()> {
        Ok(())
    }
    fn OnSelectionChange(&self, _: Ref<IFileDialog>) -> Result<()> {
        Ok(())
    }
    fn OnShareViolation(
        &self,
        _: Ref<IFileDialog>,
        _: Ref<IShellItem>,
    ) -> Result<FDE_SHAREVIOLATION_RESPONSE> {
        Ok(FDESVR_DEFAULT)
    }
    fn OnOverwrite(
        &self,
        _: Ref<IFileDialog>,
        _: Ref<IShellItem>,
    ) -> Result<FDE_OVERWRITE_RESPONSE> {
        Ok(FDEOR_DEFAULT)
    }
}

fn accepts(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|value| value.eq_ignore_ascii_case(extension))
        })
}

fn set_name(dialog: &IFileDialog, path: &Path) -> Result<()> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { dialog.SetFileName(PCWSTR(name.as_ptr())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_dialog::{FileDialogKind, FileFilter};

    #[test]
    fn changing_type_updates_the_visible_suffix_and_result_index() {
        let _com = super::super::ComScope::enter().unwrap();
        let request = FileDialogRequest::new(71, FileDialogKind::SaveFile)
            .file_name("剧本.json")
            .filters([
                FileFilter::new("JSON", ["json"]),
                FileFilter::new("TXT", ["txt"]),
                FileFilter::new("SRT", ["srt"]),
            ]);
        let native = super::super::NativeDialog::create(request.kind).unwrap();
        super::super::configure(&native, &request).unwrap();
        let dialog = native.file_dialog();
        let _subscription = subscribe(dialog, &request).unwrap();
        let types = TypeEvents {
            extensions: vec![vec!["json".into()], vec!["txt".into()], vec!["srt".into()]],
        };
        for (index, name) in [(2, "剧本.txt"), (3, "剧本.srt"), (1, "剧本.json")] {
            unsafe { dialog.SetFileTypeIndex(index) }.unwrap();
            types.change_type(dialog).unwrap();
            assert_eq!(dialog_file_name(dialog).as_deref(), Some(name));
            assert_eq!(selected_filter(dialog, 3), Some(index as usize - 1));
        }
        assert_eq!(selected_filter(dialog, 0), None);
    }
}
