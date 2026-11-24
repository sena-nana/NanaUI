#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub value: String,
    pub content: Option<Arc<crate::InlineContent>>,
    /// Shown instead of [`Self::value`]: a message resolved in the locale of
    /// the scope the text is in, and again when that locale changes.
    pub localized: Option<crate::LocalizedText>,
    /// Painted, but omitted from accessibility projection.
    pub decorative: bool,
    pub style: NodeStyle,
}

impl Text {
    pub fn content(mut self, content: impl Into<crate::InlineContent>) -> Self {
        let content = content.into().normalized();
        self.value = content.text().to_owned();
        self.localized = None;
        self.content = if matches!(content, crate::InlineContent::PlainText(_)) {
            None
        } else {
            Some(Arc::new(content))
        };
        self
    }
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            content: None,
            localized: None,
            decorative: false,
            style: NodeStyle::default(),
        }
    }

    /// Text that says `localized` in the locale of the scope it is in.
    pub fn localized(localized: crate::LocalizedText) -> Self {
        Self {
            localized: Some(localized),
            ..Self::new("")
        }
    }

    /// Keep the glyphs visible while omitting this text from the accessibility
    /// tree. Use it when an ancestor already exposes the same name, equivalent
    /// to `aria-hidden="true"`.
    pub const fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Standard typography retained until the installed theme resolves it.
    pub fn typography(mut self, role: nana_ui_core::TypeRole) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(nana_ui_core::TypographyTokens::DEFAULT.size(role));
        layout.theme.font_size = Some(nana_ui_core::ThemeToken::Type(role).into());
        let line = if matches!(
            role,
            nana_ui_core::TypeRole::Hint
                | nana_ui_core::TypeRole::Meta
                | nana_ui_core::TypeRole::Body
        ) {
            nana_ui_core::LineRole::Line
        } else {
            nana_ui_core::LineRole::Tall
        };
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
            nana_ui_core::TypographyTokens::DEFAULT.line_height(line),
        ));
        layout.theme.line_height = Some(nana_ui_core::ThemeToken::Line(line).into());
        self
    }

    pub fn text_weight(mut self, role: nana_ui_core::TextWeight) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_weight = Some(nana_ui_core::TypographyTokens::DEFAULT.weight(role));
        layout.theme.font_weight = Some(nana_ui_core::ThemeToken::Weight(role).into());
        self
    }

    /// Font size in logical px. Also sets the line box to the matching
    /// [`nana_ui_core::ControlSize`] line height, so text does not sit in a box
    /// tighter than the font.
    pub fn font_size(mut self, size: f32) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(size);
        layout.theme.font_size = None;
        layout.theme.line_height = None;
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
            nana_ui_core::ControlSize::nearest_text(size).line_height(),
        ));
        self
    }

    /// Font weight (400 regular, 500 medium, 600 semibold).
    pub fn font_weight(mut self, weight: u16) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.theme.font_weight = None;
        layout.font_weight = Some(weight);
        self
    }

    /// Semantic foreground role. Business colors do not belong here; pick the
    /// role that matches the text's job.
    /// Type step, following the installed theme's type scale. See
    /// [`NodeStyle::type_role`].
    pub fn role(mut self, role: nana_ui_core::TypeRole) -> Self {
        self.style.type_role = Some(role);
        self
    }

    /// Weight step, following the installed theme. See
    /// [`NodeStyle::type_weight`].
    pub fn weight(mut self, weight: nana_ui_core::TextWeight) -> Self {
        self.style.type_weight = Some(weight);
        self
    }

    pub fn color(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style.foreground = Some(role);
        self
    }

    /// Line box height, overriding what [`Self::font_size`] derived.
    pub fn line_height(mut self, line_height: f32) -> Self {
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.theme.line_height = None;
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(line_height));
        self
    }

    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).width = Some(width);
        self
    }

    pub fn max_width(mut self, max_width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).max_width = Some(max_width);
        self
    }

    /// Keeps the text on one line.
    pub fn nowrap(mut self, nowrap: bool) -> Self {
        Arc::make_mut(&mut self.style.layout).white_space = if nowrap {
            nana_ui_core::WhiteSpaceSpec::Nowrap
        } else {
            nana_ui_core::WhiteSpaceSpec::Normal
        };
        self
    }

    /// Truncates overflow with an ellipsis. Pair with [`Self::nowrap`] for the
    /// usual single-line label.
    pub fn ellipsis(mut self, ellipsis: bool) -> Self {
        Arc::make_mut(&mut self.style.layout).text_overflow_ellipsis = ellipsis;
        self
    }

    /// Single-line label that truncates rather than wrapping.
    pub fn truncating(self) -> Self {
        self.nowrap(true).ellipsis(true)
    }
}

impl ComponentView for Text {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TEXT.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(content) = self.content.as_deref() {
            crate::world::inline_content::project_inline_content(id, world, content, mutations);
        } else {
            match &self.localized {
                Some(localized) => {
                    if world.inline_record(id).is_some() || world.rich_text(id).is_some() {
                        crate::world::inline_content::project_inline_content(
                            id,
                            world,
                            &crate::InlineContent::default(),
                            mutations,
                        );
                    }
                    if world.localized_text(id) != Some(localized) {
                        mutations.set_localized_text(id, Some(localized.clone()));
                    }
                }
                // Written text is literal; writing it ends a localization.
                None => {
                    if !crate::world::inline_content::project_control_content(
                        id,
                        world,
                        &self.value,
                        self.content.as_deref(),
                        mutations,
                    ) && (world.text(id) != Some(self.value.as_str())
                        || world.localized_text(id).is_some())
                    {
                        mutations.set_text(
                            id,
                            TextContent {
                                value: self.value.clone().into(),
                            },
                        );
                    }
                }
            }
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                hidden: self.decorative,
                ..AccessibilityState::default()
            },
        );
    }
}
