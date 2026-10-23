//! 富文本剪贴板事务；键盘与上下文菜单共用宿主服务、来源校验和编辑通知。
use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// 可由工具栏或上下文菜单发起的富文本剪贴板操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichClipboardCommand {
    Cut,
    Copy,
    Paste,
}

/// 私有宿主可持有各自的剪贴板；有限缓存按原生来源标记区分同正文内容。
static RICH_CLIPBOARD: Mutex<VecDeque<(String, RichText)>> = Mutex::new(VecDeque::new());
static CLIPBOARD_TOKEN: AtomicU64 = AtomicU64::new(1);

impl AppContext {
    /// 对指定编辑器执行剪贴板事务，保留选区、富文本、撤销和 Changed 通知。
    ///
    /// 宿主沿用输入路由的剪贴板后端。只读编辑器允许复制；预编辑期间等待输入法提交。
    /// 剪切先确认写入成功再删除；无文本的粘贴发出 PasteRequested 供图片等格式使用。
    /// 空选区、不可编辑状态和剪贴板拒绝写入返回 false；组件访问错误返回 FrameworkError。
    pub fn rich_clipboard(
        &mut self,
        entity: Entity<RichTextEditor>,
        command: RichClipboardCommand,
        services: &mut dyn HostServices,
    ) -> Result<bool, FrameworkError> {
        let allowed = self.read(entity, |view| {
            !view.disabled
                && view.state.preedit.is_none()
                && (command == RichClipboardCommand::Copy || view.editable())
        })?;
        if !allowed {
            return Ok(false);
        }
        match command {
            RichClipboardCommand::Copy | RichClipboardCommand::Cut => {
                let piece = self.read(entity, |view| {
                    let range = view.range();
                    (!range.is_empty()).then(|| view.value.slice(range))
                })?;
                let Some(piece) = piece else {
                    return Ok(false);
                };
                let token = format!(
                    "{:x}-{:x}",
                    std::process::id(),
                    CLIPBOARD_TOKEN.fetch_add(1, Ordering::Relaxed)
                );
                if services
                    .write_clipboard_with_token(piece.text(), &token)
                    .is_err()
                {
                    return Ok(false);
                }
                if let Ok(mut clipboard) = RICH_CLIPBOARD.lock() {
                    clipboard.push_back((token, piece));
                    if clipboard.len() > 16 {
                        clipboard.pop_front();
                    }
                }
                if command == RichClipboardCommand::Cut {
                    self.edit_with(entity, |view, _| {
                        let range = view.range();
                        view.delete(range)
                    })?;
                }
                Ok(true)
            }
            RichClipboardCommand::Paste => {
                let token = services.read_clipboard_token().ok().flatten();
                let clipboard = services.read_clipboard();
                let Ok(Some(text)) = clipboard
                    .as_ref()
                    .map(|text| text.as_ref().filter(|text| !text.is_empty()).cloned())
                else {
                    self.update_component(entity, |_, cx| {
                        cx.emit(RichTextEditorEvent::PasteRequested)
                    })?;
                    return Ok(true);
                };
                // 前后来源标记一致，才复用与本次纯文本相对应的富文本缓存。
                let token = token.filter(|token| {
                    services.read_clipboard_token().ok().flatten().as_ref() == Some(token)
                });
                let piece = RICH_CLIPBOARD.lock().ok().and_then(|clipboard| {
                    clipboard
                        .iter()
                        .find(|(key, piece)| Some(key) == token.as_ref() && piece.text() == text)
                        .map(|(_, piece)| piece.clone())
                });
                self.edit_with(entity, |view, _| match &piece {
                    Some(piece) => view.replace_selection(piece, false),
                    None => {
                        // 纯文本粘贴沿用输入样式，同时独立于前后连续打字的撤销步骤。
                        view.state.coalesce = false;
                        let changed = view.insert_text(&text.replace("\r\n", "\n"));
                        view.state.coalesce = false;
                        changed
                    }
                })
            }
        }
    }
}
