//! Read-only path summaries. Values are full display spellings, never I/O paths.
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LengthSpec,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, TextContent, UiWorld,
};

/// Single-line path with width-aware middle elision and a full-path tooltip.
/// Pass `nana_ui::path::display_path(path, false)`; retain the real path for I/O.
#[derive(Debug, Clone, PartialEq)]
pub struct PathText {
    pub value: Arc<str>,
    pub style: NodeStyle,
}

impl PathText {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            style: NodeStyle::default(),
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

/// Visual state for [`PathText`] nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct PathTextVisual {
    pub value: Arc<str>,
}

impl crate::WidgetVisual for PathTextVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::PALETTE
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let shortened = shorten_path(&self.value, ctx.content.width, |text| {
            ctx.world.chrome_text_measure(ctx.id).width(
                text,
                ctx.style.font_size,
                ctx.style.font_weight,
            )
        });
        let color = ctx.style.color.unwrap_or_else(|| {
            ctx.style_model
                .color(crate::SemanticColorRole::Text)
                .as_rgba_array()
        });
        let mut out = crate::WidgetPaintOutput {
            owns_node_text: true,
            suppress_root_surface: true,
            ..crate::WidgetPaintOutput::new()
        };
        out.push(
            2,
            crate::WidgetPaintOp::Text(crate::WidgetText {
                bounds: ctx.content,
                content: Arc::<str>::from(shortened).into(),
                color: Some(color),
                paint_color: ctx.matching_text_paint(color),
                font_size: ctx.style.font_size,
                font_weight: ctx.style.font_weight,
                horizontal_alignment: crate::TextHorizontalAlignment::Start,
                vertical_alignment: crate::TextVerticalAlignment::Center,
                ellipsis: false,
                wrap: false,
                max_lines: None,
                italic: None,
                underline: None,
                line_through: None,
                suppress_spans: false,
                preserve_lines: None,
                unstyled_glyph: false,
                line_height: None,
                retained_text: crate::WidgetRetainedText::None,
                tabular_nums: false,
                ..Default::default()
            }),
        );
        out
    }
}

impl ComponentView for PathText {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::PATH_TEXT.type_id);

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "path-text".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width.get_or_insert(LengthSpec::Fill);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
        if world.text(id) != Some(self.value.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: Arc::clone(&self.value).into(),
                },
            );
        }
        crate::view_components::project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: !self.value.is_empty(),
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: Some(Arc::clone(&self.value)),
                ..AccessibilityState::default()
            },
        );
        let visual = PathTextVisual {
            value: Arc::clone(&self.value),
        };
        if world.widget_visual_as::<PathTextVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
    }
}

/// Pure display formatting; width uses the same engine/style as the painted run.
/// Drop middle directories first, then the root, and finally elide the leaf.
pub(crate) fn shorten_path(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    let fits = |s: &str| measure(s) <= width;
    if text.is_empty() || fits(text) {
        return text.to_owned();
    }
    if !width.is_finite() || width <= 0.0 || !fits("…") {
        return String::new();
    }
    let separator = |c: char| matches!(c, '/' | '\\');
    let end = text.trim_end_matches(separator).len();
    if end == 0 {
        return "…".into();
    }
    let leaf_start = text[..end].rfind(separator).map_or(0, |i| i + 1);
    // Root includes disk/relative head, or both UNC server and share.
    let mut root_end = text.find(separator).map_or(0, |i| i + 1);
    if text.starts_with("\\\\") || text.starts_with("//") {
        root_end = text
            .char_indices()
            .filter(|(i, c)| *i >= 2 && separator(*c))
            .nth(1)
            .map_or(root_end, |(i, _)| i + 1);
    }
    let sep = text[..leaf_start].chars().next_back().unwrap_or('/');
    // Preserve as many trailing directory components as fit, without partial names.
    let starts: Vec<_> = text
        .char_indices()
        .filter(|(i, c)| *i >= root_end && *i < leaf_start && separator(*c))
        .map(|(i, _)| i + 1)
        .collect();
    let first = starts
        .partition_point(|start| !fits(&format!("{}…{sep}{}", &text[..root_end], &text[*start..])));
    if let Some(start) = starts.get(first) {
        return format!("{}…{sep}{}", &text[..root_end], &text[*start..]);
    }
    if leaf_start > root_end {
        let candidate = format!("{}…{sep}{}", &text[..root_end], &text[leaf_start..]);
        if fits(&candidate) {
            return candidate;
        }
    }
    let leaf = &text[leaf_start..];
    if leaf_start > 0 {
        // Retain the nearest parent if losing the root makes room for it.
        let parent_start = text[..leaf_start.saturating_sub(1)]
            .rfind(separator)
            .map_or(0, |i| i + 1);
        for start in [parent_start, leaf_start] {
            let candidate = format!("…{sep}{}", &text[start..]);
            if fits(&candidate) {
                return candidate;
            }
        }
    }
    if fits(leaf) {
        return leaf.into();
    }
    let graphemes: Vec<&str> = leaf.graphemes(true).collect();
    let extension = leaf
        .trim_end_matches(separator)
        .rfind('.')
        .filter(|i| *i > 0)
        .map_or("", |i| &leaf[i..]);
    let suffix_count = extension.graphemes(true).count();
    // Keep the extension if possible, spending remaining space on the stem.
    if suffix_count > 0 && fits(&format!("…{extension}")) {
        return fitting_prefix(
            graphemes.len().saturating_sub(suffix_count),
            |count| format!("{}…{extension}", graphemes[..count].concat()),
            fits,
        );
    }
    fitting_prefix(
        graphemes.len(),
        |count| {
            let front = count.div_ceil(2);
            let back = count / 2;
            format!(
                "{}…{}",
                graphemes[..front].concat(),
                graphemes[graphemes.len() - back..].concat()
            )
        },
        fits,
    )
}

// Search by retained graphemes, bounding shaping work for very long filenames.
fn fitting_prefix(
    limit: usize,
    candidate: impl Fn(usize) -> String,
    fits: impl Fn(&str) -> bool,
) -> String {
    let mut low = 0;
    let mut high = limit;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if fits(&candidate(middle)) {
            low = middle;
        } else {
            high = middle;
        }
    }
    candidate(low)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport};

    fn width(text: &str) -> f32 {
        text.graphemes(true).count() as f32
    }

    fn layout(cx: &mut AppContext, doc: DocumentId, width: f32, height: f32) {
        let work = cx.take_system_work();
        cx.world_mut().resolve_styles(&work.style).unwrap();
        cx.layout_document(doc, LayoutViewport::new(width, height))
            .unwrap();
        let root = cx.world().document_order(doc)[0];
        let mut queue = MutationQueue::new();
        queue.write_layout(
            root,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        );
        if let Some(lv) = cx
            .world()
            .widget_visual_as::<crate::LabeledValueVisual>(root)
            && let Some(action) = lv.action
        {
            queue.write_layout(
                action,
                crate::LayoutBox {
                    x: width - 48.0,
                    y: 4.0,
                    width: 48.0,
                    height: 28.0,
                },
            );
        }
        cx.commit_mutations(queue).unwrap();
    }

    #[test]
    fn preserves_full_path_when_it_fits_and_drops_middle_directories_first() {
        let path = r"D:\projects\characters\models\Haru.model3.json";
        assert_eq!(shorten_path(path, width(path), width), path);
        assert_eq!(
            shorten_path(path, 28.0, width),
            r"D:\…\models\Haru.model3.json"
        );
        assert_eq!(shorten_path(path, 22.0, width), r"D:\…\Haru.model3.json");
        assert_eq!(shorten_path(path, 18.0, width), r"…\Haru.model3.json");
    }

    #[test]
    fn handles_roots_relative_paths_trailing_separators_and_tiny_widths() {
        for path in [
            r"\\server\share\a\b\c\model.json",
            "/home/nana/a/b/model.json",
            "relative/a/b/model.json",
            "D:\\a\\b\\c\\模型\\",
            "/",
            r"D:\",
            "",
            "plain.txt",
        ] {
            for available in 0..=width(path) as usize + 1 {
                let shown = shorten_path(path, available as f32, width);
                assert!(width(&shown) <= available as f32, "{path}: {shown}");
                if available as f32 >= width(path) {
                    assert_eq!(shown, path);
                }
            }
        }
        assert_eq!(
            shorten_path(r"\\server\share\a\b\c\model.json", 29.0, width),
            r"\\server\share\…\c\model.json"
        );
        assert!(shorten_path("/one/two/模型/", 7.0, width).ends_with("模型/"));
    }

    #[test]
    fn long_leaf_keeps_extension_without_splitting_graphemes() {
        let path = "/模型/👨‍👩‍👧‍👦e\u{301}长长长长长长长长.json";
        let shown = shorten_path(path, 9.0, width);
        assert!(shown.starts_with("👨‍👩‍👧‍👦e\u{301}"), "{shown}");
        assert!(shown.ends_with("….json"), "{shown}");
        assert!(width(&shown) <= 9.0);
    }

    #[test]
    fn geometry_restores_full_path_after_resize_and_accessibility_stays_full() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let full = "/characters/very-long-directory/Haru.model3.json";
        let path = cx.create_component(doc, PathText::new(full)).unwrap();
        layout(&mut cx, doc, 150.0, 32.0);
        let paint = cx
            .world()
            .widget_paint(path.stable_id())
            .expect("path paint");
        let text = paint
            .ops
            .iter()
            .find_map(|tagged| match &tagged.op {
                crate::WidgetPaintOp::Text(text) if tagged.slot == 2 => Some(text),
                _ => None,
            })
            .expect("path text");
        assert_ne!(text.content.as_str(), full);
        assert!(text.content.as_str().ends_with(".json"));
        let accessibility = cx.world().accessibility(path.stable_id()).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some(full));
        layout(&mut cx, doc, 1200.0, 32.0);
        let paint = cx
            .world()
            .widget_paint(path.stable_id())
            .expect("path paint");
        let text = paint
            .ops
            .iter()
            .find_map(|tagged| match &tagged.op {
                crate::WidgetPaintOp::Text(text) if tagged.slot == 2 => Some(text),
                _ => None,
            })
            .expect("path text");
        assert_eq!(text.content.as_str(), full);
    }

    #[test]
    fn full_tooltip_obeys_delay_updates_and_closes_on_leave_park_and_removal() {
        use crate::OverlayHostState;
        use std::time::Duration;
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let path = cx
            .create_component(doc, PathText::new("/first/model.json"))
            .unwrap();
        layout(&mut cx, doc, 200.0, 150.0);
        let tooltip = cx.component_tooltip(path).unwrap().unwrap();
        cx.set_pointer_hover_at(doc, 1, Some(path.stable_id()), Duration::ZERO)
            .unwrap();
        cx.advance_animations(Duration::from_millis(349));
        assert_eq!(
            cx.world().overlay_host(path.stable_id()).unwrap().active,
            None
        );
        cx.advance_animations(Duration::from_millis(350));
        assert_eq!(
            cx.world().overlay_host(path.stable_id()).unwrap().active,
            Some(tooltip.stable_id())
        );
        cx.update_component(path, |path, _| path.value = "/second/模型.json".into())
            .unwrap();
        assert_eq!(
            &*cx.read(tooltip, |tip| tip.label.clone()).unwrap(),
            "/second/模型.json"
        );
        cx.set_pointer_hover_at(doc, 1, None, Duration::from_millis(360))
            .unwrap();
        assert_eq!(
            cx.world().overlay_host(path.stable_id()),
            Some(OverlayHostState::default())
        );
        cx.set_pointer_hover_at(doc, 1, Some(path.stable_id()), Duration::from_millis(400))
            .unwrap();
        cx.advance_animations(Duration::from_millis(750));
        let mut park = MutationQueue::new();
        park.park_subtree(path.stable_id());
        cx.commit_mutations(park).unwrap();
        assert_eq!(
            cx.world().overlay_host(path.stable_id()),
            Some(OverlayHostState::default())
        );
        cx.remove_view(path).unwrap();
        assert!(!cx.world().contains(tooltip.stable_id()));
    }

    #[test]
    fn labeled_path_keeps_full_value_and_its_action_child() {
        use crate::{Button, LabeledValue};
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let full = r"D:\characters\models\Haru.model3.json";
        let row = cx
            .create_component(doc, LabeledValue::new("路径", "").path_value(full))
            .unwrap();
        let action = cx
            .create_detached_component(doc, Button::new("打开"))
            .unwrap();
        cx.set_labeled_value_action(row, Some(action.stable_id()))
            .unwrap();
        layout(&mut cx, doc, 240.0, 40.0);
        let paint = cx.world().widget_paint(row.stable_id()).unwrap();
        let Some(crate::WidgetPaintOp::Text(value)) = paint
            .ops
            .iter()
            .find(|tagged| tagged.slot == 3)
            .map(|tagged| &tagged.op)
        else {
            panic!("path row value");
        };
        let bounds = cx.world().layout_box(action.stable_id());
        assert!(value.content.as_str().contains('…'), "{value:?}");
        assert!(bounds.is_some());
        assert_eq!(
            cx.world()
                .accessibility(row.stable_id())
                .unwrap()
                .value
                .as_deref(),
            Some(full)
        );
        assert_eq!(
            &*cx.read(cx.component_tooltip(row).unwrap().unwrap(), |tip| tip
                .label
                .clone())
                .unwrap(),
            full
        );
    }
}
