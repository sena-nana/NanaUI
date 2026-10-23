//! 拖拽预览仅投影呈现样式；原布局、输入实体与编辑历史保持同一权威树。
use super::*;

impl UiWorld {
    /// 列表反馈更新时重抽取各行，保证开始、源行切换和取消均恢复真实呈现。
    pub(super) fn reorder_preview_extraction_ids<'a>(
        &self,
        ids: &'a [StableNodeId],
    ) -> std::borrow::Cow<'a, [StableNodeId]> {
        let enabled = |id| {
            matches!(
                self.nodes.visual(id),
                Some(StandardVisual::ReorderList {
                    preview_enabled: true,
                    ..
                })
            )
        };
        if !ids.iter().any(|id| enabled(*id)) {
            return std::borrow::Cow::Borrowed(ids);
        }
        let mut result = ids.to_vec();
        let mut seen: NodeSet = ids.iter().copied().collect();
        for id in ids {
            if !enabled(*id) {
                continue;
            }
            let mut pending = self
                .nodes
                .get(*id)
                .map(|node| node.hierarchy.children.to_vec())
                .unwrap_or_default();
            while let Some(child) = pending.pop() {
                if seen.insert(child) {
                    result.push(child);
                }
                if let Some(node) = self.nodes.get(child) {
                    pending.extend(node.hierarchy.children.iter().copied());
                }
            }
        }
        std::borrow::Cow::Owned(result)
    }

    /// 原行画面随指针移动并留出等高空槽；只修改抽取副本，不回写基础样式。
    pub(super) fn reorder_preview_layout(
        &self,
        id: StableNodeId,
        layout: &mut Arc<LayoutStyle>,
    ) -> bool {
        let Some(parent) = self.parent_id(id) else {
            return false;
        };
        let Some(StandardVisual::ReorderList {
            drag: Some(drag), ..
        }) = self.nodes.visual(parent)
        else {
            return false;
        };
        let Some([x, y]) = drag.preview_offset else {
            return false;
        };
        let Some(row) = self
            .nodes
            .get(parent)
            .and_then(|list| list.hierarchy.children.get(drag.source))
            .copied()
        else {
            return false;
        };
        if row != id {
            return false;
        }
        let layout = Arc::make_mut(layout);
        let mut transform = layout.transform.unwrap_or_default();
        transform.e += x;
        transform.f += y;
        layout.transform = Some(transform);
        layout.opacity = Some(layout.opacity.unwrap_or(1.0) * 0.65);
        layout.z_index = Some(100_000);
        true
    }
}
