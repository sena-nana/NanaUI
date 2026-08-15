use std::sync::Arc;

use crate::{
    AccessibilityRole, AccessibilityState, AlignSpec, ComponentView, FlexDirection,
    InteractionState, JustifySpec, LengthSpec, MutationQueue, NodeKind, NodeStyle,
    SemanticColorRole, StableNodeId, StandardVisual, TextContent, UiWorld,
};
use nana_ui_core::Icon;

fn project_visual(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    text: &str,
    style: NodeStyle,
    visual: StandardVisual,
    accessibility: AccessibilityState,
) {
    if world.text(id) != Some(text) {
        mutations.set_text(
            id,
            TextContent {
                value: text.to_owned(),
            },
        );
    }
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
    if world.standard_visual(id) != Some(visual.clone()) {
        mutations.set_standard_visual(id, Some(visual));
    }
    let interaction = InteractionState {
        pointer_events: false,
        focusable: false,
    };
    if world.interaction(id) != Some(interaction) {
        mutations.set_interaction(id, interaction);
    }
    project_accessibility(id, world, mutations, accessibility);
}

fn project_accessibility(
    id: StableNodeId,
    world: &UiWorld,
    mutations: &mut MutationQueue,
    accessibility: AccessibilityState,
) {
    if world.accessibility(id) != Some(&accessibility) {
        mutations.set_accessibility(id, accessibility);
    }
}

fn status_foreground(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    match tone {
        nana_ui_core::StatusTone::Neutral => SemanticColorRole::Muted,
        nana_ui_core::StatusTone::Info => SemanticColorRole::Accent,
        nana_ui_core::StatusTone::Success => SemanticColorRole::Success,
        nana_ui_core::StatusTone::Warning => SemanticColorRole::Warning,
        nana_ui_core::StatusTone::Danger => SemanticColorRole::Danger,
    }
}

fn status_background(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    match tone {
        nana_ui_core::StatusTone::Neutral => SemanticColorRole::Subtle,
        nana_ui_core::StatusTone::Info => SemanticColorRole::AccentSoft,
        nana_ui_core::StatusTone::Success => SemanticColorRole::Subtle,
        nana_ui_core::StatusTone::Warning => SemanticColorRole::WarningSoft,
        nana_ui_core::StatusTone::Danger => SemanticColorRole::Subtle,
    }
}

/// Compact semantic status text. It is descriptive and never owns an action.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBadge {
    pub label: Arc<str>,
    pub tone: nana_ui_core::StatusTone,
    pub compact: bool,
    pub style: NodeStyle,
}

impl StatusBadge {
    pub fn new(label: impl Into<Arc<str>>, tone: nana_ui_core::StatusTone) -> Self {
        Self {
            label: label.into(),
            tone,
            compact: true,
            style: NodeStyle::default(),
        }
    }

    pub fn tone(mut self, tone: nana_ui_core::StatusTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(status_foreground(self.tone));
        style.background = Some(status_background(self.tone));
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        let (horizontal, vertical, indicator, gap) = if self.compact {
            (7.0, 3.0, 6.0, 5.0)
        } else {
            (8.0, 4.0, 8.0, 6.0)
        };
        layout.padding_left = Some(LengthSpec::Px(horizontal + indicator + gap));
        layout.padding_right = Some(LengthSpec::Px(horizontal));
        layout.padding_top = Some(LengthSpec::Px(vertical));
        layout.padding_bottom = Some(LengthSpec::Px(vertical));
        layout.border_width = Some(0.0);
        layout.border_radius = Some(999.0);
        layout.font_size = Some(if self.compact { 11.0 } else { 12.0 });
        layout.font_weight = Some(500);
        style
    }
}

impl ComponentView for StatusBadge {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "status-badge".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            self.label.as_ref(),
            self.effective_style(),
            StandardVisual::StatusBadge {
                label: Arc::clone(&self.label),
                tone: self.tone,
                compact: self.compact,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: Some(Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Inline validation feedback whose semantic intent is available to every backend.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationMessage {
    pub message: Arc<str>,
    pub intent: nana_ui_core::ValidationIntent,
    pub compact: bool,
    pub style: NodeStyle,
}

impl ValidationMessage {
    pub fn new(message: impl Into<Arc<str>>, intent: nana_ui_core::ValidationIntent) -> Self {
        Self {
            message: message.into(),
            intent,
            compact: true,
            style: NodeStyle::default(),
        }
    }

    pub fn intent(mut self, intent: nana_ui_core::ValidationIntent) -> Self {
        self.intent = intent;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(match self.intent {
            nana_ui_core::ValidationIntent::Warning => SemanticColorRole::Warning,
            nana_ui_core::ValidationIntent::Danger => SemanticColorRole::Danger,
        });
        let layout = Arc::make_mut(&mut style.layout);
        let indicator = if self.compact { 12.0 } else { 14.0 };
        let gap = if self.compact { 5.0 } else { 6.0 };
        layout.padding_left = Some(LengthSpec::Px(indicator + gap));
        layout.font_size = Some(if self.compact { 11.0 } else { 12.0 });
        layout.font_weight = None;
        style
    }
}

impl ComponentView for ValidationMessage {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "validation-message".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            self.message.as_ref(),
            self.effective_style(),
            StandardVisual::ValidationMessage {
                message: Arc::clone(&self.message),
                intent: self.intent,
                compact: self.compact,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: Some(Arc::clone(&self.message)),
                invalid: true,
                ..AccessibilityState::default()
            },
        );
    }
}

/// A flat feedback surface with intrinsic semantic content and an optional
/// application-owned action child.
#[derive(Debug, Clone, PartialEq)]
pub struct EmptyState {
    pub title: Arc<str>,
    pub message: Option<Arc<str>>,
    pub icon: Option<Icon>,
    pub compact: bool,
    pub action: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl EmptyState {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            message: None,
            icon: None,
            compact: false,
            action: None,
            style: NodeStyle::default(),
        }
    }

    pub fn message(mut self, message: impl Into<Arc<str>>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Declare an action child identity. Mount it through
    /// [`crate::AppContext::set_empty_state_action`].
    pub fn action_child(mut self, action: StableNodeId) -> Self {
        self.action = Some(action);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = None;
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = if self.compact {
            AlignSpec::Start
        } else {
            AlignSpec::Center
        };
        layout.gap = Some(LengthSpec::Px(0.0));
        let (horizontal, vertical) = if self.compact {
            (6.0, 8.0)
        } else {
            (16.0, 24.0)
        };
        layout.padding_left = Some(LengthSpec::Px(horizontal));
        layout.padding_right = Some(LengthSpec::Px(horizontal));
        // The real intrinsic leading block is written after TextShaper has
        // measured title/message for the resolved content width.
        layout.padding_top = Some(LengthSpec::Px(vertical));
        layout.padding_bottom = Some(LengthSpec::Px(vertical));
        layout.border_width = Some(0.0);
        style
    }
}

impl ComponentView for EmptyState {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "empty-state".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_visual(
            id,
            world,
            mutations,
            "",
            self.effective_style(),
            StandardVisual::EmptyState {
                title: Arc::clone(&self.title),
                message: self.message.clone(),
                icon: self.icon,
                compact: self.compact,
                action: self.action,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.title)),
                value: self.message.clone(),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueEmphasis {
    Muted,
    Normal,
    #[default]
    Strong,
}

/// A compact label/value summary with an optional application-owned action child.
/// The parent remains non-interactive; only the mounted child owns activation.
#[derive(Debug, Clone, PartialEq)]
pub struct LabeledValue {
    pub label: Arc<str>,
    pub value: Arc<str>,
    pub emphasis: ValueEmphasis,
    pub compact: bool,
    /// An existing child view. No icon or action behavior is synthesized.
    pub action: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl LabeledValue {
    pub fn new(label: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            emphasis: ValueEmphasis::Strong,
            compact: true,
            action: None,
            style: NodeStyle::default(),
        }
    }

    pub fn emphasis(mut self, emphasis: ValueEmphasis) -> Self {
        self.emphasis = emphasis;
        self
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Declare an action child identity. Use
    /// [`crate::AppContext::set_labeled_value_action`] to validate and mount it.
    pub fn action_child(mut self, action: StableNodeId) -> Self {
        self.action = Some(action);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.direction = Some(FlexDirection::Row);
        layout.justify_content = JustifySpec::End;
        layout.align_items = AlignSpec::Center;
        layout.min_height = Some(LengthSpec::Px(if self.compact { 28.6 } else { 31.0 }));
        layout.gap = Some(LengthSpec::Px(if self.compact { 4.0 } else { 8.0 }));
        style
    }
}

impl ComponentView for LabeledValue {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "labeled-value".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let value_role = match self.emphasis {
            ValueEmphasis::Muted => SemanticColorRole::Muted,
            ValueEmphasis::Normal | ValueEmphasis::Strong => SemanticColorRole::Text,
        };
        let value_weight = match self.emphasis {
            ValueEmphasis::Muted => 400,
            ValueEmphasis::Normal => 500,
            ValueEmphasis::Strong => 600,
        };
        project_visual(
            id,
            world,
            mutations,
            "",
            self.effective_style(),
            StandardVisual::LabeledValue {
                label: Arc::clone(&self.label),
                value: Arc::clone(&self.value),
                value_role,
                value_weight,
                compact: self.compact,
                action: self.action,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.label)),
                value: Some(Arc::clone(&self.value)),
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, Button, DocumentId, FrameworkError, StandardVisual};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use unicode_segmentation::UnicodeSegmentation;

    #[derive(Default)]
    struct WrappingShaper;

    impl crate::TextShaper for WrappingShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            text: &crate::TextContent,
            style: &crate::ComputedStyle,
            constraints: crate::TextShapeConstraints,
        ) -> crate::TextMetrics {
            let count = text.value.graphemes(true).count();
            if count == 0 {
                return crate::TextMetrics::default();
            }
            let advance = style.font_size;
            let natural_width = count as f32 * advance;
            let columns = constraints
                .max_width
                .filter(|_| constraints.wrap)
                .map(|width| (width / advance).floor().max(1.0) as usize)
                .unwrap_or(count);
            let lines = count.div_ceil(columns);
            crate::TextMetrics {
                width: constraints
                    .max_width
                    .map_or(natural_width, |width| natural_width.min(width)),
                height: lines as f32 * style.font_size * 1.2,
            }
        }
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn layout(context: &mut AppContext, id: StableNodeId, width: f32, height: f32) {
        let mut mutations = MutationQueue::new();
        mutations.write_layout(
            id,
            crate::LayoutBox {
                x: 10.0,
                y: 20.0,
                width,
                height,
            },
        );
        context.commit_mutations(mutations).unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
    }

    #[test]
    fn status_badge_projects_tone_density_and_accessible_label() {
        let mut context = AppContext::new();
        let badge = context
            .create_component(
                document(),
                StatusBadge::new("Offline", nana_ui_core::StatusTone::Danger).compact(false),
            )
            .unwrap();
        let id = badge.stable_id();

        assert_eq!(context.world().text(id), Some("Offline"));
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Danger)
        );
        assert_eq!(
            context.world().node_style(id).unwrap().layout.font_size,
            Some(12.0)
        );
        assert_eq!(
            context.world().accessibility(id).unwrap().label.as_deref(),
            Some("Offline")
        );
        layout(&mut context, id, 96.0, 24.0);
        let crate::ComponentGeometry::StatusBadge {
            indicator,
            label,
            background,
            foreground,
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("status badge geometry")
        };
        assert!(indicator.width > 0.0);
        assert_eq!(label.font_size, 12.0);
        assert_eq!(label.font_weight, Some(500));
        assert_eq!(background[..3], foreground[..3]);
        assert!(background[3] < foreground[3]);
        let dark_foreground = foreground;
        let mut theme = MutationQueue::new();
        theme.set_theme(crate::ThemeMode::Light);
        context.commit_mutations(theme).unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let crate::ComponentGeometry::StatusBadge { foreground, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("status badge geometry")
        };
        assert_ne!(foreground, dark_foreground);
    }

    #[test]
    fn validation_message_projects_intent_and_invalid_semantics() {
        let mut context = AppContext::new();
        let message = context
            .create_component(
                document(),
                ValidationMessage::new(
                    "A project is required",
                    nana_ui_core::ValidationIntent::Warning,
                ),
            )
            .unwrap();
        let id = message.stable_id();

        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Warning)
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert!(accessibility.invalid);
        assert_eq!(
            accessibility.label.as_deref(),
            Some("A project is required")
        );
        layout(&mut context, id, 180.0, 18.0);
        let crate::ComponentGeometry::ValidationMessage {
            indicator,
            label,
            foreground,
        } = context.world().component_geometry(id).unwrap()
        else {
            panic!("validation geometry")
        };
        assert!(indicator.width > 0.0);
        assert_eq!(label.font_size, 11.0);
        assert_eq!(label.color, Some(foreground));
        assert_eq!(label.font_weight, None);
        assert_eq!(
            context.world().node_style(id).unwrap().layout.font_weight,
            None
        );
    }

    #[test]
    fn empty_state_wrapped_intrinsics_drive_action_layout_and_settle() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("继续"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("没有可显示的项目👩‍💻请稍后再试")
                    .message("较长的说明文字会根据最终内容宽度自动换行🙂")
                    .icon(Icon::Folder),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 400.0))
            .unwrap();
        let first_action_y = context.world().layout_box(action.stable_id()).unwrap().y;

        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 400.0))
            .unwrap();
        assert!(
            !context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );

        let crate::ComponentGeometry::EmptyState {
            title,
            message: Some(message),
            action: Some(action_bounds),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("wrapped empty state geometry")
        };
        assert!(title.bounds.height > 13.0 * 1.2);
        assert!(message.bounds.height > 12.0 * 1.2);
        assert!(action_bounds.y > first_action_y);
        assert!((action_bounds.y - (message.bounds.y + message.bounds.height) - 10.0).abs() < 0.01);

        let metrics = (title.bounds.height, message.bounds.height, action_bounds.y);
        context.set_theme(crate::ThemeMode::Light).unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        assert!(
            !context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        let crate::ComponentGeometry::EmptyState {
            title,
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("light empty state geometry")
        };
        assert_eq!(
            metrics,
            (title.bounds.height, message.bounds.height, action.y)
        );
    }

    #[test]
    fn empty_state_title_only_has_no_synthetic_spacing() {
        let mut context = AppContext::new();
        let empty = context
            .create_component(document(), EmptyState::new("空").compact(true))
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(100.0, 100.0))
            .unwrap();
        context
            .shape_text_for_layout(document(), &mut shaper)
            .unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(100.0, 100.0))
            .unwrap();
        let crate::ComponentGeometry::EmptyState {
            icon: None,
            message: None,
            action: None,
            title,
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("title-only empty state geometry")
        };
        assert_eq!(title.bounds.height, 12.0 * 1.2);
        assert_eq!(
            context
                .world()
                .node_style(empty.stable_id())
                .unwrap()
                .layout
                .padding_top,
            Some(LengthSpec::Px(8.0 + title.bounds.height))
        );
    }

    #[test]
    fn normal_empty_state_centers_shaped_regions_while_preserving_start_aligned_lines() {
        let mut context = AppContext::new();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("ABCDE").message("这是用于验证窄宽换行起点的较长消息文本"),
            )
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 200.0))
            .unwrap();
        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(120.0, 200.0))
            .unwrap();

        let crate::ComponentGeometry::EmptyState {
            content_clip: clip,
            title,
            message: Some(message),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("normal empty state geometry")
        };
        assert!(message.bounds.height > 12.0 * 1.2);
        assert!(title.bounds.width < message.bounds.width);
        assert!(title.bounds.x > message.bounds.x);
        assert!((title.bounds.x - (clip.x + (clip.width - title.bounds.width) / 2.0)).abs() < 0.01);
        assert!(
            (message.bounds.x - (clip.x + (clip.width - message.bounds.width) / 2.0)).abs() < 0.01
        );
    }

    #[test]
    fn extremely_narrow_empty_state_keeps_intrinsics_ordered_inside_content_width() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("标题")
                    .icon(Icon::Folder)
                    .message("包含 emoji 🎉 的窄宽消息"),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(40.0, 400.0))
            .unwrap();
        assert!(
            context
                .shape_text_for_layout(document(), &mut shaper)
                .unwrap()
        );
        context
            .layout_document(document(), crate::LayoutViewport::new(40.0, 400.0))
            .unwrap();

        let crate::ComponentGeometry::EmptyState {
            content_clip: clip,
            icon: Some((_, icon, _)),
            title,
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("narrow empty state geometry")
        };
        for bounds in [icon, title.bounds, message.bounds] {
            assert!(bounds.x >= clip.x);
            assert!(bounds.x + bounds.width <= clip.x + clip.width + 0.01);
        }
        assert!(icon.y + icon.height <= title.bounds.y + 0.01);
        assert!(title.bounds.y + title.bounds.height <= message.bounds.y + 0.01);
        assert!(message.bounds.y + message.bounds.height <= action.y + 0.01);
    }

    #[test]
    fn empty_state_root_clip_bounds_action_hit_testing_and_accessibility() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(document(), EmptyState::new("Empty"))
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let mut layouts = MutationQueue::new();
        layouts.write_layout(
            empty.stable_id(),
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        layouts.write_layout(
            action.stable_id(),
            crate::LayoutBox {
                x: 80.0,
                y: 40.0,
                width: 40.0,
                height: 20.0,
            },
        );
        context.commit_mutations(layouts).unwrap();
        context.world_mut().rebuild_hit_test(document());

        assert_eq!(
            context.world().hit_test(document(), 95.0, 45.0),
            Some(action.stable_id())
        );
        assert_ne!(
            context.world().hit_test(document(), 110.0, 45.0),
            Some(action.stable_id())
        );
        let action_accessibility = context
            .world()
            .project_accessibility(document())
            .into_iter()
            .find(|node| node.id == action.stable_id())
            .unwrap();
        assert_eq!(
            action_accessibility.bounds,
            crate::LayoutBox {
                x: 80.0,
                y: 40.0,
                width: 20.0,
                height: 10.0,
            }
        );

        let mut outside = MutationQueue::new();
        outside.write_layout(
            action.stable_id(),
            crate::LayoutBox {
                x: 110.0,
                y: 40.0,
                width: 20.0,
                height: 10.0,
            },
        );
        context.commit_mutations(outside).unwrap();
        context.world_mut().rebuild_hit_test(document());
        assert_ne!(
            context.world().hit_test(document(), 115.0, 45.0),
            Some(action.stable_id())
        );
        let accessibility = context.world().project_accessibility(document());
        assert!(
            accessibility
                .iter()
                .all(|node| node.id != action.stable_id())
        );
        assert!(
            accessibility
                .iter()
                .find(|node| node.id == empty.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
    }

    #[test]
    fn compact_empty_state_action_gap_combines_column_and_action_padding() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Action"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("Title").message("Message").compact(true),
            )
            .unwrap();
        context
            .set_empty_state_action(empty, Some(action.stable_id()))
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        let mut shaper = WrappingShaper;
        context.shape_text(&work.text, &mut shaper).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(160.0, 120.0))
            .unwrap();
        context
            .shape_text_for_layout(document(), &mut shaper)
            .unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(160.0, 120.0))
            .unwrap();
        let crate::ComponentGeometry::EmptyState {
            message: Some(message),
            action: Some(action),
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("compact action geometry")
        };
        assert!((action.y - (message.bounds.y + message.bounds.height) - 6.0).abs() < 0.01);
    }

    #[test]
    fn empty_state_owns_semantic_content_and_action_mount_is_atomic() {
        let mut context = AppContext::new();
        let action = context
            .create_detached_component(document(), Button::new("Create project").loading(true))
            .unwrap();
        let replacement = context
            .create_detached_component(document(), Button::new("Import project"))
            .unwrap();
        let empty = context
            .create_component(
                document(),
                EmptyState::new("No projects")
                    .icon(Icon::Folder)
                    .message("Create the first project"),
            )
            .unwrap();
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert!(
            !context
                .world()
                .document_order(document())
                .contains(&action.stable_id())
        );
        assert_eq!(context.next_animation_deadline(), None);
        assert_eq!(
            context.world().interaction(empty.stable_id()),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert!(matches!(
            context.world().standard_visual(empty.stable_id()),
            Some(StandardVisual::EmptyState {
                icon: Some(Icon::Folder),
                message: Some(message),
                action: None,
                ..
            }) if message.as_ref() == "Create the first project"
        ));
        assert!(
            context
                .set_empty_state_action(empty, Some(action.stable_id()))
                .unwrap()
        );
        assert!(
            !context
                .set_empty_state_action(empty, Some(action.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![action.stable_id()]
        );
        assert_eq!(context.next_animation_deadline(), Some(Duration::ZERO));

        let missing = StableNodeId::new(999).unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, Some(missing)),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == empty.stable_id() && slot == missing
        ));
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![action.stable_id()]
        );
        assert!(
            context
                .set_empty_state_action(empty, Some(replacement.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(empty.stable_id()).unwrap().children,
            vec![replacement.stable_id()]
        );
        assert!(
            context
                .world()
                .node(action.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert_eq!(context.next_animation_deadline(), None);
        assert!(context.set_empty_state_action(empty, None).unwrap());
        assert!(
            context
                .world()
                .node(empty.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
        assert!(
            context
                .world()
                .node(replacement.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(replacement.stable_id()),
            Some(crate::MountState::Parked)
        );
        layout(&mut context, empty.stable_id(), 240.0, 120.0);
        let crate::ComponentGeometry::EmptyState {
            icon,
            title,
            message,
            action,
            ..
        } = context
            .world()
            .component_geometry(empty.stable_id())
            .unwrap()
        else {
            panic!("empty state geometry")
        };
        assert!(matches!(icon, Some((Icon::Folder, _, _))));
        assert_eq!(title.font_size, 13.0);
        assert_eq!(message.unwrap().font_size, 12.0);
        assert!(action.is_none());
    }

    #[test]
    fn labeled_value_action_is_validated_and_remains_application_owned() {
        let mut context = AppContext::new();
        let foreign_action = context
            .create_detached_component(DocumentId::new(2).unwrap(), Button::new("Foreign"))
            .unwrap();
        let action = context
            .create_detached_component(document(), Button::new("Copy"))
            .unwrap();
        let replacement = context
            .create_detached_component(document(), Button::new("Reveal"))
            .unwrap();
        let owner = context
            .create_component(document(), Button::new("Owner"))
            .unwrap();
        let owned_action = context
            .create_detached_component(document(), Button::new("Owned"))
            .unwrap();
        context.append_child(owner, owned_action).unwrap();
        let summary = context
            .create_component(
                document(),
                LabeledValue::new("Revision", "42").emphasis(ValueEmphasis::Normal),
            )
            .unwrap();
        let id = summary.stable_id();

        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        let missing = StableNodeId::new(999).unwrap();
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(missing)),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == missing
        ));
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(foreign_action.stable_id())),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == foreign_action.stable_id()
        ));
        assert!(matches!(
            context.set_labeled_value_action(summary, Some(owned_action.stable_id())),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot)
            }) if parent == id && slot == owned_action.stable_id()
        ));
        assert_eq!(context.read(summary, |view| view.action).unwrap(), None);
        assert!(context.world().node(id).unwrap().children.is_empty());
        assert_eq!(
            context
                .world()
                .node(owned_action.stable_id())
                .unwrap()
                .parent,
            Some(owner.stable_id())
        );

        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(action, move |_button, _event: &crate::Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        assert!(
            context
                .set_labeled_value_action(summary, Some(action.stable_id()))
                .unwrap()
        );

        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::LabeledValue {
                label: Arc::from("Revision"),
                value: Arc::from("42"),
                value_role: SemanticColorRole::Text,
                value_weight: 500,
                compact: true,
                action: Some(action.stable_id()),
            })
        );
        assert_eq!(context.world().text(id), Some(""));
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
        assert_eq!(
            context.world().node(id).unwrap().children,
            vec![action.stable_id()]
        );
        assert!(context.activate_button(action).unwrap());
        assert_eq!(*activations.lock().unwrap(), 1);
        assert!(
            context
                .set_labeled_value_action(summary, Some(replacement.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(id).unwrap().children,
            vec![replacement.stable_id()]
        );
        assert!(
            context
                .world()
                .node(action.stable_id())
                .unwrap()
                .parent
                .is_none()
        );
        assert_eq!(
            context.world().mount_state(action.stable_id()),
            Some(crate::MountState::Parked)
        );
        assert!(
            context
                .set_labeled_value_action(summary, Some(action.stable_id()))
                .unwrap()
        );
        assert!(context.activate_button(action).unwrap());
        assert_eq!(*activations.lock().unwrap(), 2);
        assert!(context.set_labeled_value_action(summary, None).unwrap());
        assert!(context.world().node(id).unwrap().children.is_empty());
        let removed = context.remove_view(action).unwrap();
        assert_eq!(removed.label, "Copy");
        assert!(!context.world().contains(action.stable_id()));
        layout(&mut context, id, 180.0, 32.0);
        let crate::ComponentGeometry::LabeledValue { label, value, .. } =
            context.world().component_geometry(id).unwrap()
        else {
            panic!("labeled value geometry")
        };
        assert_eq!(label.font_size, 11.0);
        assert_eq!(value.font_size, 12.0);
        assert_eq!(value.font_weight, Some(500));
        assert_ne!(label.color, value.color);
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.label.as_deref(), Some("Revision"));
        assert_eq!(accessibility.value.as_deref(), Some("42"));
    }

    #[test]
    fn feedback_helpers_never_park_unowned_field_only_actions() {
        let mut context = AppContext::new();
        let foreign = context
            .create_detached_component(DocumentId::new(2).unwrap(), Button::new("Foreign"))
            .unwrap();
        let empty = context
            .create_component(document(), EmptyState::new("Empty"))
            .unwrap();
        context
            .update_component(empty, |empty, _cx| {
                empty.action = Some(foreign.stable_id());
            })
            .unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot),
            }) if parent == empty.stable_id() && slot == foreign.stable_id()
        ));
        assert_eq!(
            context.read(empty, |empty| empty.action).unwrap(),
            Some(foreign.stable_id())
        );
        assert_eq!(
            context.world().mount_state(foreign.stable_id()),
            Some(crate::MountState::Parked)
        );

        let missing = StableNodeId::new(999).unwrap();
        context
            .update_component(empty, |empty, _cx| empty.action = Some(missing))
            .unwrap();
        assert!(matches!(
            context.set_empty_state_action(empty, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                slot: Some(slot),
                ..
            }) if slot == missing
        ));

        let stale = context
            .create_detached_component(document(), Button::new("Stale"))
            .unwrap();
        context
            .update_component(empty, |empty, _cx| {
                empty.action = Some(stale.stable_id());
            })
            .unwrap();
        assert!(context.set_empty_state_action(empty, None).is_err());
        assert_eq!(
            context.world().mount_state(stale.stable_id()),
            Some(crate::MountState::Parked)
        );

        let declared = context
            .create_detached_component(document(), Button::new("Declared"))
            .unwrap();
        let declared_empty = context
            .create_component(
                document(),
                EmptyState::new("Declared empty").action_child(declared.stable_id()),
            )
            .unwrap();
        assert!(
            context
                .set_empty_state_action(declared_empty, Some(declared.stable_id()))
                .unwrap()
        );
        assert_eq!(
            context.world().node(declared.stable_id()).unwrap().parent,
            Some(declared_empty.stable_id())
        );

        let owner = context
            .create_component(document(), Button::new("Owner"))
            .unwrap();
        let owned = context
            .create_detached_component(document(), Button::new("Owned"))
            .unwrap();
        context.append_child(owner, owned).unwrap();
        let summary = context
            .create_component(document(), LabeledValue::new("Key", "Value"))
            .unwrap();
        context
            .update_component(summary, |summary, _cx| {
                summary.action = Some(owned.stable_id());
            })
            .unwrap();
        assert!(matches!(
            context.set_labeled_value_action(summary, None),
            Err(FrameworkError::InvalidFeedbackSlots {
                parent,
                slot: Some(slot),
            }) if parent == summary.stable_id() && slot == owned.stable_id()
        ));
        assert_eq!(
            context.world().node(owned.stable_id()).unwrap().parent,
            Some(owner.stable_id())
        );
        assert_eq!(
            context.world().mount_state(owned.stable_id()),
            Some(crate::MountState::Mounted)
        );
        assert!(
            context
                .world()
                .node(summary.stable_id())
                .unwrap()
                .children
                .is_empty()
        );
    }
}
