#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, replace_padding_xy,
    shared_label, text_area_footer_extent, text_field_style, tool_box, with_unit,
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
use crate::{AppContext, Entity};

use super::button::{
    CARD_LOADING_RESERVE, CARD_TITLE_BAND, CARD_TITLE_GAP, CARD_TITLE_LINE, CARD_TITLE_SIZE,
    CARD_TITLE_WEIGHT,
};

/// Non-interactive content surface. Actions belong to explicit child controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub title: Option<Arc<str>>,
    pub kind: nana_ui_core::CardKind,
    pub loading: bool,
    pub(crate) loading_phase: f32,
    pub style: NodeStyle,
}

impl Card {
    pub fn new() -> Self {
        Self {
            title: None,
            kind: nana_ui_core::CardKind::Surface,
            loading: false,
            loading_phase: 0.0,
            // 背景、边框按 kind 在 project 里填；圆角和 panel inset 在节点上
            // 命名档位，安装的主题决定数值。
            style: NodeStyle {
                radius: Some(nana_ui_core::RadiusTier::Md),
                surface_padding: Some(nana_ui_core::SurfacePadding::Panel),
                ..NodeStyle::default()
            },
        }
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.title = Some(label.into());
        self
    }
    pub fn title(self, title: impl Into<Arc<str>>) -> Self {
        self.label(title)
    }
    pub fn kind(mut self, kind: nana_ui_core::CardKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    /// Replace all four padding edges, including logical declarations.
    pub fn padding(self, padding: f32) -> Self {
        self.padding_xy(padding, padding)
    }

    pub fn padding_xy(mut self, x: f32, y: f32) -> Self {
        replace_padding_xy(Arc::make_mut(&mut self.style.layout), x, y);
        self.style.surface_padding = None;
        self
    }
    pub fn height(mut self, height: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).height =
            Some(nana_ui_core::LengthSpec::Px(height.max(0.0)));
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// 自绘这张卡片的外观，见 [`crate::Painter`]。要保留内建卡片再叠加装饰，
    /// 在 `paint` 里调用 `cx.draw_default()`。
    pub fn painter(mut self, painter: impl Into<crate::NodePainter>) -> Self {
        self.style.painter = Some(painter.into());
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for Card {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::CARD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        animation_sample: Some(|cx, target, sample, _| {
            cx.update_component(Entity::<Self>::from_stable_id(target), |card, _| {
                card.loading_phase = sample.progress;
            })
            .is_ok()
        }),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "card".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // An untitled card's node text is empty, not absent: comparing the
        // `Option` would rewrite "" on every projection.
        if world.text(id) != Some(self.title.as_deref().unwrap_or_default()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.title.as_deref().unwrap_or_default().to_owned().into(),
                },
            );
        }
        let visual = CardVisual {
            title: self.title.clone(),
            kind: self.kind,
            loading: self.loading,
            loading_phase: self.loading_phase,
        };
        if world.widget_visual_as::<CardVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut effective_style = self.style.clone();
        effective_style.text_vertical_alignment = TextVerticalAlignment::Center;
        let (kind_background, kind_border, kind_border_width) = match self.kind {
            nana_ui_core::CardKind::Surface | nana_ui_core::CardKind::Raised => {
                (Some(nana_ui_core::SemanticColorRole::Surface), None, 0.0)
            }
            nana_ui_core::CardKind::Outlined => {
                (None, Some(nana_ui_core::SemanticColorRole::Border), 1.0)
            }
            nana_ui_core::CardKind::Flat => (None, None, 0.0),
            nana_ui_core::CardKind::Selected => (
                Some(nana_ui_core::SemanticColorRole::Selected),
                Some(nana_ui_core::SemanticColorRole::BorderSoft),
                1.0,
            ),
        };
        if effective_style.background.is_none() {
            effective_style.background = kind_background;
        }
        if effective_style.border.is_none() {
            effective_style.border = kind_border;
        }
        if effective_style.radius.is_none() && effective_style.layout.border_radius.is_none() {
            effective_style.radius = Some(nana_ui_core::RadiusTier::Md);
        }
        if effective_style.surface_padding.is_none() && effective_style.layout.padding.is_none() {
            effective_style.surface_padding = Some(nana_ui_core::SurfacePadding::Panel);
        }
        {
            let layout = Arc::make_mut(&mut effective_style.layout);
            if layout.border_width.is_none() {
                layout.border_width = Some(kind_border_width);
            }
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState::default(),
            AccessibilityState {
                label: self.title.clone(),
                busy: self.loading,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Retained visual state for [`Card`].
#[derive(Debug, Clone, PartialEq)]
pub struct CardVisual {
    pub title: Option<Arc<str>>,
    pub kind: nana_ui_core::CardKind,
    pub loading: bool,
    pub loading_phase: f32,
}

impl crate::WidgetVisual for CardVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::PALETTE
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
    }

    fn uses_card_title_typography(&self) -> bool {
        true
    }

    fn tracks_text_content_part_budget(&self) -> bool {
        true
    }

    fn clearing_changes_content_layout(&self) -> bool {
        true
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        true
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        let Some(old) = prev.and_then(|any| any.downcast_ref::<Self>()) else {
            return true;
        };
        (&self.title, self.loading) != (&old.title, old.loading)
    }

    fn advance_loading_phase(&self, phase: f32) -> Option<crate::WidgetVisualHandle> {
        if self.loading_phase == phase {
            return None;
        }
        let mut next = self.clone();
        next.loading_phase = phase;
        Some(crate::WidgetVisualHandle::new(next))
    }

    fn intrinsic_chrome_content(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
    ) -> crate::WidgetVisualContent {
        crate::WidgetVisualContent {
            width: if self.loading {
                CARD_LOADING_RESERVE
            } else {
                0.0
            },
            height: 0.0,
            text: 0.0,
        }
    }

    fn leading_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> Option<crate::layout_engine::parts::LeadingParts> {
        let theme = world.node_theme(id);
        self.title
            .as_ref()
            .map(|_| crate::layout_engine::parts::LeadingParts {
                axis: nana_ui_core::FlexDirection::Column,
                extent: theme.typography().line_tall + theme.spacing().sm,
                breadth: 0.0,
                end: false,
            })
    }

    fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _layout: crate::LayoutBox,
        available_width: f32,
        _available_height: f32,
    ) -> Option<f32> {
        Some(
            world
                .content_part_box(id, crate::LayoutPartKind::Header)
                .map_or(available_width, |part| part.width.max(0.0)),
        )
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        let text = world.text_metrics(id).unwrap_or_default();
        let line_tall = world.node_theme(id).typography().line_tall;
        let x = content.x.min(bounds.x + bounds.width).max(bounds.x);
        let y = content.y.max(bounds.y).min(bounds.y + bounds.height);
        let header = crate::LayoutBox {
            x,
            y,
            width: content.width.min((bounds.x + bounds.width - x).max(0.0)),
            height: line_tall.min((bounds.y + bounds.height - y).max(0.0)),
        };
        Some(crate::layout_engine::parts::card_content_parts(
            header,
            self.title.is_some(),
            self.loading,
            crate::layout_engine::Size::new(
                if self.title.as_ref().is_some_and(|title| !title.is_empty()) {
                    text.width
                } else {
                    0.0
                },
                text.height,
            ),
            writing.direction == nana_ui_core::DirSpec::Rtl,
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let mut out = crate::WidgetPaintOutput {
            owns_node_text: true,
            child_text_suppression: match &self.title {
                Some(title) => crate::WidgetChildTextSuppression::Matching(Arc::clone(title)),
                None => crate::WidgetChildTextSuppression::Allow,
            },
            root_surface: (self.kind == nana_ui_core::CardKind::Raised).then(|| {
                crate::WidgetRootSurface {
                    background: None,
                    border_color: None,
                    border_width: None,
                    fallback_elevation: Some(crate::ComponentElevation::from_shadow(
                        ctx.theme.shadow(nana_ui_core::ElevationRole::Surface),
                    )),
                }
            }),
            ..crate::WidgetPaintOutput::new()
        };
        if let Some((title, bounds)) = self
            .title
            .as_ref()
            .zip(ctx.content_part(crate::LayoutPartKind::Header))
        {
            let text_color = ctx.text_color();
            out.push(
                2,
                crate::WidgetPaintOp::Text(crate::WidgetText {
                    bounds,
                    content: Arc::clone(title).into(),
                    color: Some(text_color),
                    paint_color: ctx.matching_text_paint(text_color),
                    font_size: ctx.theme.typography().body,
                    font_weight: Some(ctx.theme.typography().semibold),
                    horizontal_alignment: TextHorizontalAlignment::Start,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ellipsis: true,
                    wrap: false,
                    max_lines: Some(1),
                    italic: None,
                    underline: None,
                    line_through: None,
                    suppress_spans: false,
                    preserve_lines: Some(false),
                    unstyled_glyph: false,
                    line_height: Some(Some(nana_ui_core::LineHeightSpec::Absolute(bounds.height))),
                    retained_text: crate::WidgetRetainedText::AlwaysClipOverflow,
                    tabular_nums: false,
                    ..Default::default()
                }),
            );
        }
        if self.loading {
            let spinner_bounds = ctx
                .content_part(crate::LayoutPartKind::Spinner)
                .unwrap_or_else(|| {
                    let extent = 20.0_f32.min(ctx.bounds.width).min(ctx.bounds.height);
                    crate::LayoutBox {
                        x: ctx.bounds.x + (ctx.bounds.width - extent) / 2.0,
                        y: ctx.bounds.y + (ctx.bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    }
                });
            let spinner_fg = ctx
                .style_model
                .color(
                    ctx.theme
                        .recipes()
                        .foreground(nana_ui_core::ComponentRecipeId::Card, false),
                )
                .as_rgba_array();
            out.push(
                3,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds: spinner_bounds,
                    glyph: crate::WidgetGlyph::Spinner {
                        phase: (self.loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                    },
                    color: Some(spinner_fg),
                    paint_color: ctx.matching_text_paint(spinner_fg),
                    align_to_adjacent_text: false,
                }),
            );
        }
        out
    }
}

/// Hairline rule between sibling content.
///
/// A divider carries no label and no interaction: it is one themed line that
/// stretches along the cross axis of its parent. Grouping headings stay
/// ordinary `Text` siblings, so a section title never has to be a divider prop.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub orientation: crate::SelectionOrientation,
    /// Line thickness in logical pixels.
    pub thickness: f32,
    /// Inset applied at both ends along the divider's own direction.
    pub inset: f32,
    pub style: NodeStyle,
}

impl Divider {
    /// Horizontal rule: stretches across its parent's width.
    pub fn horizontal() -> Self {
        Self::new(crate::SelectionOrientation::Horizontal)
    }

    /// Vertical rule: stretches down its parent's height.
    pub fn vertical() -> Self {
        Self::new(crate::SelectionOrientation::Vertical)
    }

    fn new(orientation: crate::SelectionOrientation) -> Self {
        let mut divider = Self {
            orientation,
            thickness: 1.0,
            inset: 0.0,
            style: NodeStyle::default(),
        };
        divider.style = divider.resolved_style();
        divider
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self.style = self.resolved_style();
        self
    }

    /// Inset both ends by `inset` logical px. Replaces an
    /// [`Self::inset_step`].
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self.set_theme_inset(None);
        self.style = self.resolved_style();
        self
    }

    /// Inset both ends by a spacing step: the installed theme's spacing
    /// decides the pixels, so the rule stays in line with content padded by
    /// the same step whatever the theme. [`Self::inset`] holds the step at
    /// the default metrics; a later [`Self::inset`] replaces the step.
    pub fn inset_step(mut self, step: nana_ui_core::SpacingStep) -> Self {
        self = self.inset(nana_ui_core::SpacingTokens::DEFAULT.resolve(step));
        self.set_theme_inset(Some(crate::theme_intent::space(step)));
        self
    }

    fn set_theme_inset(&mut self, inset: Option<nana_ui_core::ThemeExpr>) {
        let theme = &self.style.layout.theme;
        let unset = match self.orientation {
            crate::SelectionOrientation::Horizontal => {
                theme.margin_left.is_none() && theme.margin_right.is_none()
            }
            crate::SelectionOrientation::Vertical => {
                theme.margin_top.is_none() && theme.margin_bottom.is_none()
            }
        };
        if inset.is_none() && unset {
            // Leave the shared default layout and theme group unshared.
            return;
        }
        let theme = &mut Arc::make_mut(&mut self.style.layout).theme;
        match self.orientation {
            crate::SelectionOrientation::Horizontal => {
                theme.margin_left = inset.clone();
                theme.margin_right = inset;
            }
            crate::SelectionOrientation::Vertical => {
                theme.margin_top = inset.clone();
                theme.margin_bottom = inset;
            }
        }
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn resolved_style(&self) -> NodeStyle {
        let thickness = if self.thickness.is_finite() && self.thickness > 0.0 {
            self.thickness
        } else {
            1.0
        };
        let inset = if self.inset.is_finite() && self.inset > 0.0 {
            self.inset
        } else {
            0.0
        };
        // Patch onto the caller's style rather than replacing it: `thickness()`
        // and `inset()` must not silently discard a style set before them.
        let mut style = self.style.clone();
        if style.background.is_none() {
            style.background = Some(nana_ui_core::SemanticColorRole::BorderSoft);
        }
        let base = (*style.layout).clone();
        let layout = Arc::new(match self.orientation {
            crate::SelectionOrientation::Horizontal => nana_ui_core::LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Fill),
                min_width: Some(nana_ui_core::LengthSpec::Px(0.0)),
                height: Some(nana_ui_core::LengthSpec::Px(thickness)),
                min_height: Some(nana_ui_core::LengthSpec::Px(thickness)),
                flex_shrink: Some(0.0),
                align_self: Some(nana_ui_core::AlignSpec::Stretch),
                margin_left: Some(nana_ui_core::LengthSpec::Px(inset)),
                margin_right: Some(nana_ui_core::LengthSpec::Px(inset)),
                ..base
            },
            crate::SelectionOrientation::Vertical => nana_ui_core::LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(thickness)),
                min_width: Some(nana_ui_core::LengthSpec::Px(thickness)),
                height: Some(nana_ui_core::LengthSpec::Fill),
                min_height: Some(nana_ui_core::LengthSpec::Px(0.0)),
                flex_shrink: Some(0.0),
                align_self: Some(nana_ui_core::AlignSpec::Stretch),
                margin_top: Some(nana_ui_core::LengthSpec::Px(inset)),
                margin_bottom: Some(nana_ui_core::LengthSpec::Px(inset)),
                ..base
            },
        });
        style.layout = layout;
        style
    }
}

impl Default for Divider {
    fn default() -> Self {
        Self::horizontal()
    }
}

impl ComponentView for Divider {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DIVIDER.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "divider".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
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
                role: AccessibilityRole::Separator,
                orientation: Some(self.orientation),
                ..AccessibilityState::default()
            },
        );
    }
}
