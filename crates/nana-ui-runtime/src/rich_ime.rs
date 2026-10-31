//! 富文本编辑器向原生输入法提供的显示镜像与周边删除事务。
use super::RichTextEditor;
use std::ops::Range;
#[cfg(test)]
#[path = "rich_ime_tests.rs"]
mod tests;

/// 原生输入法的完整显示文本；所有偏移均为 UTF-8 字节边界。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichImeSnapshot {
    pub text: String,
    pub selection: (usize, usize),
    pub composition: Option<Range<usize>>,
}

impl RichTextEditor {
    /// 读取同一编辑权威的预编辑显示，宿主仅维护平台编码镜像。
    pub fn ime_snapshot(&self) -> Option<RichImeSnapshot> {
        if !self.editable() {
            return None;
        }
        let Some((preedit, cursor)) = &self.state.preedit else {
            return Some(RichImeSnapshot {
                text: self.value.text().to_owned(),
                selection: self.selection(),
                composition: None,
            });
        };
        let range = self.range();
        let mut text = self.value.text().to_owned();
        text.replace_range(range.clone(), preedit);
        let end = range.start + preedit.len();
        let caret = range.start + super::floor_char(preedit, (*cursor).min(preedit.len()));
        Some(RichImeSnapshot {
            text,
            selection: (caret, caret),
            composition: Some(range.start..end),
        })
    }

    /// 沿用文本引擎的周边删除合同，组合期间保留预编辑替换的原始选区。
    pub(super) fn delete_ime_surrounding(&mut self, before: usize, after: usize) -> bool {
        let composing = self.state.preedit.is_some();
        let Some(deletion) = nana_text::editable::ime::surrounding_deletion(
            self.value.text(),
            self.range(),
            before,
            after,
            composing,
        ) else {
            return false;
        };
        if !composing {
            return self.delete(deletion.before);
        }
        self.remember(false);
        self.value.replace_range(deletion.after.clone(), "");
        self.value.replace_range(deletion.before.clone(), "");
        self.state.anchor = deletion.map_offset(self.state.anchor);
        self.state.focus = deletion.map_offset(self.state.focus);
        true
    }
}
