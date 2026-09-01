//! LiliaCode 侧边栏结构复刻：`SidebarSection.body` -> `ReorderList`(items) ->
//! `SidebarRow` 行子节点。live 行子节点负责绘制标签，行标签必须只画一份。

use nana_ui_core::ControlSize;
use std::sync::Arc;

use nana_ui_runtime::{
    ComponentView, DocumentId, LayoutViewport, ReorderItem, ReorderList, SidebarRow,
    SidebarRowState, SidebarSection, StableNodeId, TextContent, TextMetrics, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

struct TestShaper;

impl TextShaper for TestShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        _style: &nana_ui_runtime::ComputedStyle,
        constraints: nana_ui_runtime::TextShapeConstraints,
    ) -> TextMetrics {
        let intrinsic = text.value.len() as f32 * 8.0;
        let width = constraints.max_width.unwrap_or(intrinsic).min(intrinsic);
        TextMetrics {
            width,
            height: 18.0,
            ascent: None,
        }
    }
}

fn row(label: &'static str, depth: u16, state: SidebarRowState) -> SidebarRow {
    SidebarRow::new(label).state(state).depth(depth)
}

#[test]
fn live_sidebar_rows_paint_their_label_exactly_once() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let list = runtime
        .context_mut()
        .build(document, |ui| {
            ui.child("projects-body", SidebarSection::body_port());
            let list = ui.child(
                "projects-reorder",
                ReorderList::new([
                    ReorderItem::new("p-lilia", "LiliaCode")
                        .draggable(false)
                        .drop_target(true)
                        .selected(true),
                    ReorderItem::new("p-empty", "还没有对话"),
                ])
                .size(ControlSize::Medium)
                .spacing(1.0)
                .tree_drop(true)
                .label("项目"),
            );
            ui.nest(list, |ui| {
                ui.child("row-lilia", row("LiliaCode", 0, SidebarRowState::Active));
                ui.child("row-empty", row("还没有对话", 1, SidebarRowState::Idle));
            });
            list
        })
        .unwrap();

    runtime
        .context_mut()
        .update_component(list, |_, _| {})
        .unwrap();
    runtime
        .flush(LayoutViewport::new(240.0, 400.0), &mut TestShaper)
        .unwrap();

    let mut counts: Vec<(String, usize)> = Vec::new();
    for primitive in runtime.scene().primitives() {
        if let ScenePrimitiveKind::Text { content, .. } = &primitive.kind {
            if let Some(entry) = counts.iter_mut().find(|(label, _)| label == content) {
                entry.1 += 1;
            } else {
                counts.push((content.clone(), 1));
            }
        }
    }
    for (label, count) in &counts {
        assert_eq!(*count, 1, "label {label:?} painted {count} times");
    }
    assert!(
        counts.iter().any(|(label, _)| label == "LiliaCode"),
        "project row label missing from the scene: {counts:?}"
    );
    assert!(
        counts.iter().any(|(label, _)| label == "还没有对话"),
        "empty-row label missing from the scene: {counts:?}"
    );
}

#[test]
fn stale_rows_projected_before_children_attach_never_paint() {
    // 真实时序：列表组件在行子节点挂载之前就已投影（items 非空写入 visual），
    // 之后行挂上若数据不再变化就不会重投影——绘制端必须按实际 children 兜底。
    let document = DocumentId::new(3).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    runtime
        .context_mut()
        .build(document, |ui| {
            ui.child("projects-body", SidebarSection::body_port());
            let list = ui.child(
                "projects-reorder",
                ReorderList::new([ReorderItem::new("p-lilia", "LiliaCode")
                    .draggable(false)
                    .drop_target(true)
                    .selected(true)])
                .size(ControlSize::Medium)
                .spacing(1.0)
                .tree_drop(true)
                .label("项目"),
            );
            ui.nest(list, |ui| {
                ui.child("row-lilia", row("LiliaCode", 0, SidebarRowState::Active));
            });
            list
        })
        .unwrap();

    let mut shaper = TestShaper;
    runtime
        .flush(LayoutViewport::new(240.0, 400.0), &mut shaper)
        .unwrap();

    let count = runtime
        .scene()
        .primitives()
        .filter(|primitive| match &primitive.kind {
            ScenePrimitiveKind::Text { content, .. } => content == "LiliaCode",
            _ => false,
        })
        .count();
    assert_eq!(
        count, 1,
        "row label painted {count} times without reproject"
    );
}

// 保持 ComponentView 在编译期可用（child 需要其 bound），避免未使用告警。
const _: fn() = || {
    fn assert_view<V: ComponentView>() {}
    assert_view::<SidebarRow>();
};

#[test]
fn relabeled_and_reflowed_rows_never_keep_stale_label_primitives() {
    let document = DocumentId::new(2).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let (list, rows) = runtime
        .context_mut()
        .build(document, |ui| {
            ui.child("projects-body", SidebarSection::body_port());
            let list = ui.child(
                "projects-reorder",
                ReorderList::new([
                    ReorderItem::new("p-lilia", "占位项目名")
                        .draggable(false)
                        .drop_target(true)
                        .selected(true),
                    ReorderItem::new("p-empty", "占位空行"),
                ])
                .size(ControlSize::Medium)
                .spacing(1.0)
                .tree_drop(true)
                .label("项目"),
            );
            let rows = ui.nest(list, |ui| {
                let row_a = ui.child("row-lilia", row("占位项目名", 0, SidebarRowState::Active));
                let row_b = ui.child("row-empty", row("占位空行", 1, SidebarRowState::Idle));
                (row_a, row_b)
            });
            (list, rows)
        })
        .unwrap();

    let count_labels = |runtime: &RuntimeDocument| {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for primitive in runtime.scene().primitives() {
            if let ScenePrimitiveKind::Text { content, .. } = &primitive.kind {
                if let Some(entry) = counts.iter_mut().find(|(label, _)| label == content) {
                    entry.1 += 1;
                } else {
                    counts.push((content.clone(), 1));
                }
            }
        }
        counts
    };

    let mut shaper = TestShaper;
    runtime
        .flush(LayoutViewport::new(240.0, 400.0), &mut shaper)
        .unwrap();
    let initial = count_labels(&runtime);
    for (label, count) in &initial {
        assert_eq!(*count, 1, "initial label {label:?} painted {count} times");
    }

    // 模拟 LiliaCode sync：复用已存在的行组件更新 label，再重投影列表并多帧刷新。
    runtime
        .context_mut()
        .update_component(rows.0, |row_view, _| {
            row_view.label = Arc::from("LiliaCode");
        })
        .unwrap();
    runtime
        .context_mut()
        .update_component(rows.1, |row_view, _| {
            row_view.label = Arc::from("还没有对话");
        })
        .unwrap();
    runtime
        .context_mut()
        .update_component(list, |_, _| {})
        .unwrap();
    runtime
        .flush(LayoutViewport::new(240.0, 400.0), &mut shaper)
        .unwrap();
    runtime
        .flush(LayoutViewport::new(240.0, 400.0), &mut shaper)
        .unwrap();

    let relabeled = count_labels(&runtime);
    for (label, count) in &relabeled {
        assert_eq!(
            *count, 1,
            "label {label:?} painted {count} times after relabel"
        );
    }
    assert!(
        relabeled
            .iter()
            .all(|(label, _)| label != "占位项目名" && label != "占位空行"),
        "stale placeholder labels still in the scene: {relabeled:?}"
    );
    assert!(relabeled.iter().any(|(label, _)| label == "LiliaCode"));
}
