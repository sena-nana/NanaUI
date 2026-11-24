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
use crate::{AppContext, Entity};

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    /// Canonical styled or mixed content. `None` uses the plain label.
    pub content: Option<Arc<crate::InlineContent>>,
    /// Spoken name when it must differ from [`Self::label`]. Empty keeps the
    /// visible label as the accessible name.
    pub accessible_name: String,
    pub icon: Option<nana_ui_core::Icon>,
    pub trailing_icon: Option<nana_ui_core::Icon>,
    pub icon_size: Option<f32>,
    pub icon_gap: f32,
    /// Default icon spacing remains a token until projection; numeric setters opt out.
    pub themed_icon_gap: Option<nana_ui_core::ThemeExpr>,
    pub(crate) icon_gap_uses_theme_default: bool,
    /// Where the glyph–label group sits; centred unless a row-like button
    /// asks for the start edge.
    pub content_align: TextHorizontalAlignment,
    pub kind: nana_ui_core::ButtonKind,
    pub size: nana_ui_core::ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub(crate) loading_phase: f32,
    pub invalid: bool,
    /// Opens a menu (`aria-haspopup="menu"`): announced as a menu button,
    /// and ArrowUp / ArrowDown on it raise [`SecondaryPress`] with
    /// `keyboard` set. See [`AccessibilityState::has_popup`].
    pub has_popup: bool,
    pub style: NodeStyle,
    pub(crate) style_override: bool,
    /// A [`Self::tool`]: its glyph is an icon tool's, drawn at the medium
    /// step, so the thin strokes keep the label's ink.
    pub(crate) tool_glyph: bool,
}

/// Font size of a `Card` title.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CARD_TITLE_SIZE: f32 = nana_ui_core::TypographyTokens::DEFAULT.body;
/// Font weight of a `Card` title.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CARD_TITLE_WEIGHT: u16 = nana_ui_core::TypographyTokens::DEFAULT.semibold;
/// Line box the title occupies.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CARD_TITLE_LINE: f32 = nana_ui_core::TypographyTokens::DEFAULT.line_tall;
/// Gap between the title band and the card's content.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CARD_TITLE_GAP: f32 = nana_ui_core::space::SM;
/// Vertical band a titled `Card` lays out above its body, inside its padding
/// (#385): the title's line and the gap below it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CARD_TITLE_BAND: f32 = CARD_TITLE_LINE + CARD_TITLE_GAP;
/// Width a loading `Card` reserves for its spinner beside the title.
pub(crate) const CARD_LOADING_RESERVE: f32 =
    nana_ui_core::space::PAGE_TIGHT + nana_ui_core::space::XXS;

static BUTTON_DEFAULT_STYLE: std::sync::LazyLock<NodeStyle> = std::sync::LazyLock::new(|| {
    let mut layout = (*control_layout()).clone();
    crate::theme_intent::weight(&mut layout, nana_ui_core::TextWeight::Medium);
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    // Declared, not solved: only a strip that solves its overflow reads
    // it (Issue #212).
    layout.adaptation = Some(nana_ui_core::dynamic_layout::AdaptationProfile::shared_control());
    NodeStyle {
        layout: Arc::new(layout),
        foreground: Some(nana_ui_core::SemanticColorRole::Text),
        background: None,
        border: None,
        ring: None,
        radius: None,
        corner_radii: None,
        shape: Some(nana_ui_core::ShapeRole::Button),
        control_height: Some(nana_ui_core::ControlHeight::Min(
            nana_ui_core::ControlSize::Medium,
        )),
        control_padding_x: Some(nana_ui_core::ControlPadding::Standard),
        control_padding_y: None,
        surface_padding: None,
        square: None,
        type_role: None,
        type_weight: None,
        gap_step: None,
        padding_step: None,
        interaction: crate::InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Active),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint::FOCUS_SURFACE,
            disabled: SemanticPaint {
                foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                background: Some(nana_ui_core::SemanticColorRole::Subtle),
                border: Some(nana_ui_core::SemanticColorRole::Border),
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        text_horizontal_alignment: TextHorizontalAlignment::Center,
        text_vertical_alignment: TextVerticalAlignment::Center,
        painter: None,
        color_overrides: None,
        theme_patch: None,
    }
});

impl Button {
    /// Text, spans and retained children share the label's content box.
    /// A plain value returns to the allocation-light string representation.
    pub fn content(mut self, content: impl Into<crate::InlineContent>) -> Self {
        let content = content.into().normalized();
        self.label = content.text().to_owned();
        self.content = if matches!(content, crate::InlineContent::PlainText(_)) {
            None
        } else {
            Some(Arc::new(content))
        };
        self
    }
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            content: None,
            accessible_name: String::new(),
            icon: None,
            trailing_icon: None,
            icon_size: None,
            icon_gap: nana_ui_core::space::SM,
            themed_icon_gap: None,
            icon_gap_uses_theme_default: true,
            content_align: TextHorizontalAlignment::Center,
            kind: nana_ui_core::ButtonKind::Ghost,
            size: nana_ui_core::ControlSize::Medium,
            disabled: false,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
            has_popup: false,
            style: BUTTON_DEFAULT_STYLE.clone(),
            style_override: false,
            tool_glyph: false,
        }
    }

    /// A heading's labelled tool: as tall as an [`IconButton::tool`] (the
    /// theme's page-spacing square, 24px by default) with the small step's
    /// inset, type and glyph, so it stands in `heading_tools` beside icon
    /// tools without growing the heading line. For the one action of a group
    /// that should read at a glance (adding the first item, say); pick its
    /// emphasis with [`Self::kind`]. A later [`Self::size`] or
    /// [`Self::layout`] replaces the tool's box.
    pub fn tool(label: impl Into<String>) -> Self {
        let mut button = Self::new(label).size(nana_ui_core::ControlSize::Small);
        button.style = tool_box(button.style);
        button.tool_glyph = true;
        button
    }

    pub fn icon(mut self, icon: nana_ui_core::Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// A glyph after the label — the chevron that says a button opens a
    /// menu or picker. It takes the leading icon's size and gap.
    pub fn trailing_icon(mut self, icon: nana_ui_core::Icon) -> Self {
        self.trailing_icon = Some(icon);
        self
    }
    pub fn icon_size(mut self, size: f32) -> Self {
        if size.is_finite() && size >= 0.0 {
            self.icon_size = Some(size);
        }
        self
    }
    /// Place the glyph–label group at the start, centre or end of the button.
    pub fn content_align(mut self, align: TextHorizontalAlignment) -> Self {
        self.content_align = align;
        self
    }

    pub fn icon_gap(mut self, gap: f32) -> Self {
        if gap.is_finite() && gap >= 0.0 {
            self.icon_gap = gap;
            self.themed_icon_gap = None;
            self.icon_gap_uses_theme_default = false;
        }
        self
    }

    pub fn kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    /// Replace host-owned outer layout without opting out of Button semantic
    /// paint. Apply [`Self::size`] afterwards when the size contract owns
    /// padding, height and typography.
    /// Supply the whole box. Design intent that writes into the box
    /// (`control_height`, `control_padding_x`, `radius`, `shape`) is dropped: replacing
    /// the layout used to wipe those defaults when they were baked into it, and
    /// a caller who hands over a complete box owns its height, inset and corners.
    pub fn layout(mut self, layout: Arc<nana_ui_core::LayoutStyle>) -> Self {
        self.style.layout = layout;
        self.style.control_height = None;
        self.style.control_padding_x = None;
        self.style.clear_corners();
        self
    }

    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        // The step, not the pixels: the installed metrics decide the height.
        self.style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
        self.style.control_padding_x = Some(size.into());
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(size.text_size());
        layout.theme.control_typography(size);
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// The button opens a menu; see [`Self::has_popup`](field@Self::has_popup).
    pub fn has_popup(mut self, has_popup: bool) -> Self {
        self.has_popup = has_popup;
        self
    }

    /// Spoken name when several controls share one visible word, such as
    /// "设置" on different rows. Empty keeps [`Self::label`].
    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = name.into();
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self.style_override = true;
        self
    }
}

impl ComponentView for Button {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::BUTTON.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_button),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "button".into(),
        }
    }

    /// The button recipe decides which roles `project` authors onto the node,
    /// and nothing downstream can re-decide that. Opting in is what makes a
    /// recipe change reach buttons that are already on screen.
    fn wants_recipe_reproject() -> bool {
        true
    }

    fn recipe_dependency_mask() -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::RECIPE_BUTTON
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // The label goes to the text, the visual and the accessible name.
        // Compare each against the world first and copy the label once,
        // only if one of them changed.
        let label_text = self
            .content
            .as_deref()
            .map_or(self.label.as_str(), crate::InlineContent::text);
        let mut shared = self
            .content
            .as_deref()
            .map(|content| Arc::clone(content.shared_text()));
        if !crate::world::inline_content::project_control_content(
            id,
            world,
            label_text,
            self.content.as_deref(),
            mutations,
        ) && world.text(id) != Some(label_text)
        {
            let label = shared_label(&mut shared, label_text);
            mutations.set_text(
                id,
                TextContent {
                    value: label.into(),
                },
            );
        }
        // A tool's glyph matches the icon tools beside it: at the small
        // step its strokes thin below a pixel and read paler than the label.
        let glyph_step = if self.tool_glyph {
            nana_ui_core::ControlSize::Medium
        } else {
            self.size
        };
        let node_theme = world.node_theme(id);
        let icon_size = self
            .icon_size
            .filter(|size| size.is_finite() && *size >= 0.0)
            .unwrap_or(glyph_step.text_size_in(node_theme.typography()));
        let icon_gap = if let Some(gap) = &self.themed_icon_gap {
            gap.resolve(node_theme).max(0.0)
        } else if self.icon_gap_uses_theme_default || !self.icon_gap.is_finite() {
            node_theme.spacing().sm.max(0.0)
        } else {
            self.icon_gap.max(0.0)
        };
        let visual_current = matches!(
            world.widget_visual_as::<ButtonVisual>(id),
            Some(ButtonVisual {
                label,
                icon,
                trailing_icon,
                icon_size: current_size,
                icon_gap: current_gap,
                kind,
                size,
                loading,
                loading_phase,
                invalid,
                content_align,
            }) if label.as_ref() == label_text
                && *icon == self.icon
                && *trailing_icon == self.trailing_icon
                && *current_size == icon_size
                && *current_gap == icon_gap
                && *kind == self.kind
                && *size == self.size
                && *loading == self.loading
                && *loading_phase == self.loading_phase
                && *invalid == self.invalid
                && *content_align == self.content_align
        );
        if !visual_current {
            let label = shared_label(&mut shared, label_text);
            mutations.set_widget_visual(
                id,
                Some(ButtonVisual {
                    label,
                    icon: self.icon,
                    trailing_icon: self.trailing_icon,
                    icon_size,
                    icon_gap,
                    kind: self.kind,
                    size: self.size,
                    loading: self.loading,
                    loading_phase: self.loading_phase,
                    invalid: self.invalid,
                    content_align: self.content_align,
                }),
            );
        }
        let mut effective_style = self.style.clone();
        // Retained children are a flex line: they keep their own widths and
        // are aligned across it, which a block container would not do.
        if effective_style.layout.display.is_none() {
            Arc::make_mut(&mut effective_style.layout).display =
                Some(nana_ui_core::DisplaySpec::Flex);
        }
        // Retained children share the label's content box, so they sit where
        // the label does: centred vertically and placed across by
        // `content_align`. An author style that already aligns them wins;
        // the bare default would leave them in the top-left corner.
        if effective_style.layout.align_items == nana_ui_core::AlignSpec::Start
            && effective_style.layout.justify_content == nana_ui_core::JustifySpec::Start
        {
            let layout = Arc::make_mut(&mut effective_style.layout);
            let (across, along) = match self.content_align {
                TextHorizontalAlignment::Start => (
                    nana_ui_core::AlignSpec::Start,
                    nana_ui_core::JustifySpec::Start,
                ),
                TextHorizontalAlignment::Center => (
                    nana_ui_core::AlignSpec::Center,
                    nana_ui_core::JustifySpec::Center,
                ),
                TextHorizontalAlignment::End => {
                    (nana_ui_core::AlignSpec::End, nana_ui_core::JustifySpec::End)
                }
            };
            // Children stack in a column unless the author made a row.
            if layout.direction == Some(nana_ui_core::FlexDirection::Row) {
                layout.align_items = nana_ui_core::AlignSpec::Center;
                layout.justify_content = along;
            } else {
                layout.align_items = across;
                layout.justify_content = nana_ui_core::JustifySpec::Center;
            }
        }
        let recipe = world.node_theme(id).recipes().button();
        if !self.style_override {
            // These five slots used to be five inline `match self.kind` tables
            // right here, which made this function the design authority on what
            // a primary button looks like. The variant is still the selector;
            // the answer now comes from the installed theme.
            let variant = recipe.variant(self.kind);
            effective_style.foreground = Some(variant.foreground);
            effective_style.background = variant.background;
            effective_style.border = variant.border;
            effective_style.interaction.hovered.background = Some(variant.hovered_background);
            effective_style.interaction.pressed.background = Some(variant.pressed_background);
        }
        if self.invalid {
            // Invalid is an *overlay* state, not an exclusive one: the variant
            // keeps its fill and gains a stroke in every state. Issue #100 §4
            // asks recipes to say which states stack; this is that one.
            let border = Some(recipe.invalid_border);
            effective_style.border = border;
            effective_style.interaction.hovered.border = border;
            effective_style.interaction.pressed.border = border;
            effective_style.interaction.focused.border = border;
        }
        // The accessible name reuses the world's copy while it still reads
        // the same, so an unchanged button allocates nothing here. A separate
        // spoken name must not reuse the visible label's arc.
        let spoken = if self.accessible_name.is_empty() {
            label_text
        } else {
            self.accessible_name.as_str()
        };
        let label = match world.accessible_label_arc(id) {
            Some(current) if *current == *spoken => current,
            _ if self.accessible_name.is_empty() => shared_label(&mut shared, spoken),
            _ => Arc::from(spoken),
        };
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled && !self.loading,
                focusable: !self.disabled && !self.loading,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: if self.content.is_some() && self.accessible_name.is_empty() {
                    None
                } else {
                    Some(label)
                },
                disabled: self.disabled || self.loading,
                busy: self.loading,
                invalid: self.invalid,
                has_popup: self.has_popup,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Retained visual state for [`Button`] and [`crate::Chip`].
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonVisual {
    pub label: Arc<str>,
    pub icon: Option<nana_ui_core::Icon>,
    /// A glyph after the label, such as the chevron of a picker. Same
    /// size and gap as the leading one.
    pub trailing_icon: Option<nana_ui_core::Icon>,
    pub icon_size: f32,
    pub icon_gap: f32,
    pub kind: nana_ui_core::ButtonKind,
    pub size: nana_ui_core::ControlSize,
    pub loading: bool,
    pub loading_phase: f32,
    pub invalid: bool,
    /// Where the glyph–label group sits in the content box.
    pub content_align: TextHorizontalAlignment,
}

impl ButtonVisual {
    /// Effective button kind and loading state inside `world`, folding any ancestor
    /// [`crate::ConfirmDialog`] action state override.
    pub fn effective_in(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> (nana_ui_core::ButtonKind, bool) {
        if let Some((busy, danger, true)) = world.confirm_action_effect(id) {
            (
                if danger {
                    nana_ui_core::ButtonKind::Danger
                } else {
                    nana_ui_core::ButtonKind::Primary
                },
                busy,
            )
        } else {
            (self.kind, self.loading)
        }
    }
}

impl crate::WidgetVisual for ButtonVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_BUTTON)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn offsets_overflowing_inline_text(&self) -> bool {
        true
    }

    fn tracks_text_content_part_budget(&self) -> bool {
        true
    }

    fn caps_control_text(&self) -> bool {
        true
    }

    fn button_label_state(&self) -> Option<bool> {
        Some(!self.label.is_empty())
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
        (
            self.icon.is_some(),
            self.trailing_icon.is_some(),
            self.icon_size,
            self.icon_gap,
            self.loading,
            self.content_align,
        ) != (
            old.icon.is_some(),
            old.trailing_icon.is_some(),
            old.icon_size,
            old.icon_gap,
            old.loading,
            old.content_align,
        )
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
        let parts =
            crate::layout_engine::parts::button_parts(self, crate::layout_engine::Size::default());
        let chrome = crate::layout_engine::parts::chrome_extent(&parts);
        crate::WidgetVisualContent {
            width: chrome.width,
            height: chrome.height,
            text: 0.0,
        }
    }

    fn node_text_max_width(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
        _layout: crate::LayoutBox,
        available_width: f32,
        available_height: f32,
    ) -> Option<f32> {
        Some(crate::layout_engine::parts::button_text_budget(
            self,
            crate::layout_engine::Size::new(available_width, available_height),
        ))
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        let text = world.text_metrics(id).unwrap_or_default();
        Some(crate::layout_engine::parts::button_content_parts(
            self,
            content,
            crate::layout_engine::Size::new(text.width, text.height),
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let leading = ctx.content_part(crate::LayoutPartKind::LeadingContent);
        let trailing = ctx.content_part(crate::LayoutPartKind::TrailingContent);
        let spinner = ctx.content_part(crate::LayoutPartKind::Spinner);
        let label_bounds = ctx
            .content_part(crate::LayoutPartKind::TextContent)
            .unwrap_or(crate::LayoutBox {
                width: 0.0,
                ..ctx.content
            });
        let (_, effective_loading) = self.effective_in(ctx.id, ctx.world);
        let fg = ctx.text_color();
        let fg_paint = ctx.matching_text_paint(fg);
        let background = ctx
            .style
            .paint_colors
            .background
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(ctx.style.background);
        let border_color = ctx
            .style
            .paint_colors
            .border
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(ctx.style.border_color);
        let border_width = if ctx.style.border_color.is_some() {
            ctx.source_style.layout.resolved_border_width()
        } else {
            0.0
        };
        let mut out = crate::WidgetPaintOutput {
            owns_node_text: true,
            root_surface: Some(crate::WidgetRootSurface {
                background: Some(background),
                border_color: Some(border_color),
                border_width: Some(border_width),
                fallback_elevation: None,
            }),
            ..crate::WidgetPaintOutput::new()
        };
        if !effective_loading && let Some((glyph, bounds)) = self.icon.zip(leading) {
            out.push(
                4,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds,
                    glyph: crate::WidgetGlyph::Icon(glyph),
                    color: Some(fg),
                    paint_color: fg_paint,
                    align_to_adjacent_text: false,
                }),
            );
        }
        if let Some((glyph, bounds)) = self.trailing_icon.zip(trailing) {
            out.push(
                5,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds,
                    glyph: crate::WidgetGlyph::Icon(glyph),
                    color: Some(fg),
                    paint_color: fg_paint,
                    align_to_adjacent_text: false,
                }),
            );
        }
        out.push(
            2,
            crate::WidgetPaintOp::Text(crate::WidgetText {
                bounds: label_bounds,
                content: Arc::clone(&self.label).into(),
                color: Some(fg),
                paint_color: fg_paint,
                font_size: ctx.style.font_size,
                font_weight: ctx.style.font_weight,
                horizontal_alignment: TextHorizontalAlignment::Center,
                vertical_alignment: TextVerticalAlignment::Center,
                ellipsis: true,
                wrap: false,
                max_lines: None,
                italic: None,
                underline: None,
                line_through: None,
                suppress_spans: false,
                preserve_lines: None,
                unstyled_glyph: false,
                line_height: None,
                retained_text: crate::WidgetRetainedText::NodeTextIfRich,
                tabular_nums: false,
                ..Default::default()
            }),
        );
        if let Some(bounds) = spinner {
            let spinner_fg = ctx
                .style
                .paint_colors
                .color
                .map(nana_ui_core::PaintColor::to_srgb)
                .or(ctx.style.color)
                .unwrap_or_else(|| {
                    ctx.style_model
                        .color(
                            ctx.theme
                                .recipes()
                                .foreground(nana_ui_core::ComponentRecipeId::Button, false),
                        )
                        .as_rgba_array()
                });
            out.push(
                3,
                crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                    bounds,
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

/// Retained visual state for an icon host or [`IconButton`]/[`IconGlyph`].
#[derive(Debug, Clone, PartialEq)]
pub struct IconVisual {
    pub icon: nana_ui_core::Icon,
    pub size: f32,
    pub tooltip: Option<crate::TooltipVisual>,
}

impl crate::WidgetVisual for IconVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_FAMILY)
    }

    fn is_icon_visual(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let extent = self
            .size
            .max(0.0)
            .min(ctx.bounds.width)
            .min(ctx.bounds.height);
        let x = ctx.bounds.x + (ctx.bounds.width - extent) / 2.0;
        let y = ctx.bounds.y + (ctx.bounds.height - extent) / 2.0;
        let fg = ctx
            .style
            .paint_colors
            .color
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(ctx.style.color)
            .unwrap_or_else(|| {
                ctx.style_model
                    .color(
                        ctx.theme
                            .recipes()
                            .foreground(nana_ui_core::ComponentRecipeId::Icon, false),
                    )
                    .as_rgba_array()
            });
        let mut out = crate::WidgetPaintOutput {
            suppresses_in_flow_descendants: true,
            ..crate::WidgetPaintOutput::new()
        };
        out.push(
            3,
            crate::WidgetPaintOp::Icon(crate::WidgetIcon {
                bounds: crate::LayoutBox {
                    x,
                    y,
                    width: extent,
                    height: extent,
                },
                glyph: crate::WidgetGlyph::Icon(self.icon),
                color: Some(fg),
                paint_color: ctx.matching_text_paint(fg),
                align_to_adjacent_text: true,
            }),
        );
        out
    }
}

/// Compact action whose visible glyph is independent from its accessible name.
#[derive(Debug, Clone, PartialEq)]
pub struct IconButton {
    pub icon: nana_ui_core::Icon,
    pub label: Arc<str>,
    pub kind: nana_ui_core::ButtonKind,
    pub size: nana_ui_core::ControlSize,
    /// Use the theme's page-spacing square for inline heading tools (default 24px).
    pub compact: bool,
    pub selected: bool,
    pub disabled: bool,
    /// The glyph's size in logical pixels, over the one [`Self::size`] (or
    /// [`Self::compact`]) picks from the type scale. See
    /// [`IconButton::icon_size`].
    pub icon_size: Option<f32>,
    pub tooltip: Option<IconButtonTooltip>,
    pub(crate) tooltip_open: bool,
    /// Take every colour from [`Self::style`] as written instead of from
    /// [`Self::kind`], the disabled look included. Off by default; see
    /// [`IconButton::colors_from_style`].
    pub colors_from_style: bool,
    /// Opens a menu (`aria-haspopup="menu"`): announced as a menu button,
    /// and ArrowUp / ArrowDown on it raise [`SecondaryPress`] with
    /// `keyboard` set. See [`AccessibilityState::has_popup`].
    pub has_popup: bool,
    /// Shows and hides something in place (`aria-expanded`): `Some(open)`
    /// tells assistive technology whether it is shown now. See
    /// [`AccessibilityState::expanded`].
    pub expanded: Option<bool>,
    pub style: NodeStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IconButtonTooltip {
    pub label: Arc<str>,
    pub config: nana_ui_core::TooltipConfig,
}

impl IconButton {
    pub fn new(icon: nana_ui_core::Icon, label: impl Into<Arc<str>>) -> Self {
        let mut style = NodeStyle {
            foreground: Some(nana_ui_core::SemanticColorRole::Muted),
            interaction: crate::InteractionStyle {
                selected: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoft),
                    ..SemanticPaint::default()
                },
                selected_hovered: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoftHover),
                    ..SemanticPaint::default()
                },
                selected_pressed: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::AccentOnSoft),
                    background: Some(nana_ui_core::SemanticColorRole::AccentSoftPressed),
                    ..SemanticPaint::default()
                },
                hovered: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Text),
                    background: Some(nana_ui_core::SemanticColorRole::Hover),
                    ..SemanticPaint::default()
                },
                pressed: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Text),
                    background: Some(nana_ui_core::SemanticColorRole::Active),
                    ..SemanticPaint::default()
                },
                focused: SemanticPaint::FOCUS_SURFACE,
                disabled: SemanticPaint {
                    foreground: Some(nana_ui_core::SemanticColorRole::Faint),
                    ..SemanticPaint::default()
                },
                ..crate::InteractionStyle::default()
            }
            .into(),
            shape: Some(nana_ui_core::ShapeRole::IconButton),
            control_padding_x: Some(nana_ui_core::ControlPadding::Compact),
            square: Some(nana_ui_core::SquareSize::IconButton),
            text_horizontal_alignment: TextHorizontalAlignment::Center,
            text_vertical_alignment: TextVerticalAlignment::Center,
            ..NodeStyle::default()
        };
        style.layout = control_layout();
        Self {
            icon,
            label: label.into(),
            kind: nana_ui_core::ButtonKind::Ghost,
            size: nana_ui_core::ControlSize::Medium,
            compact: false,
            selected: false,
            disabled: false,
            icon_size: None,
            tooltip: None,
            tooltip_open: false,
            colors_from_style: false,
            has_popup: false,
            expanded: None,
            style,
        }
    }

    pub fn kind(mut self, kind: nana_ui_core::ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        self.style.square = Some(nana_ui_core::SquareSize::Control(size));
        self
    }
    /// A heading tool with the theme's page-spacing target and Body glyph.
    /// Independent of `size`; disabling restores the declared control size.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Draw the glyph `size` logical pixels square instead of at the type
    /// scale step its control size picks: a dense theme's small buttons get
    /// a glyph that reads at a glance without growing the button. The
    /// button's box is unchanged. A size that is not finite or below zero
    /// is ignored, as on [`Button::icon_size`].
    pub fn icon_size(mut self, size: f32) -> Self {
        if size.is_finite() && size >= 0.0 {
            self.icon_size = Some(size);
        }
        self
    }

    pub fn tooltip(
        mut self,
        label: impl Into<Arc<str>>,
        config: nana_ui_core::TooltipConfig,
    ) -> Self {
        self.tooltip = Some(IconButtonTooltip {
            label: label.into(),
            config,
        });
        self
    }

    /// Tooltip with [`nana_ui_core::TooltipConfig::default`].
    pub fn with_tooltip(self, label: impl Into<Arc<str>>) -> Self {
        self.tooltip(label, nana_ui_core::TooltipConfig::default())
    }

    /// A heading's tool: [`Self::compact`], its label also its tooltip.
    pub fn tool(icon: nana_ui_core::Icon, label: impl Into<Arc<str>>) -> Self {
        let label = label.into();
        Self::new(icon, Arc::clone(&label))
            .compact(true)
            .with_tooltip(label)
    }

    /// Paint this button with the colours in its own [`Self::style`] instead
    /// of the ones [`Self::kind`] picks.
    ///
    /// By default `kind` decides the resting fill, the glyph and the hover /
    /// press / selected washes, and overwrites whatever the style says — which
    /// is what keeps every icon button in a product consistent. A button that
    /// stands on a surface no kind was drawn for (a coloured plate, an image)
    /// needs combinations no kind has, such as no resting fill with an
    /// on-accent glyph. This switch hands that one button's colours to its
    /// style, verbatim: `background: None` there means no fill, not "use the
    /// kind's". It only affects the button it is set on.
    ///
    /// The disabled look goes with the colours. A button in its kind's
    /// colours also fades to half opacity when disabled, because the kind's
    /// resting and disabled glyph inks sit close together; this one paints
    /// `style.interaction.disabled` as written and does not fade, so an ink
    /// already mixed for the disabled state is not dimmed again. A
    /// [`Painter`](crate::Painter) that draws the button from
    /// [`PaintState::disabled`](crate::PaintState) sets this for the same
    /// reason.
    pub fn colors_from_style(mut self) -> Self {
        self.colors_from_style = true;
        self
    }

    /// The button opens a menu; see [`Self::has_popup`](field@Self::has_popup).
    pub fn has_popup(mut self, has_popup: bool) -> Self {
        self.has_popup = has_popup;
        self
    }

    /// The button shows and hides something in place, shown now when
    /// `expanded`; see [`Self::expanded`](field@Self::expanded).
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for IconButton {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::ICON_BUTTON.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::activate_icon_button),
        overlay: Some(&crate::framework::OverlayBehavior {
            note_tooltip: Some(AppContext::note_icon_button_tooltip),
            note_modal_open: None,
            sync_modal_open: None,
            announce_close_request: None,
            announce_confirm_intent: None,
            note_anchored_menu_dismissed: None,
            dismiss_context_menu: None,
            open_hover_card: None,
            close_hover_card: None,
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
        NodeKind::Element {
            tag: "icon-button".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        if self.tooltip.is_some() && world.overlay_host(id).is_none() {
            mutations.set_overlay_host(id, OverlayHostState::default());
        }
        let node_theme = world.node_theme(id);
        let visual = IconVisual {
            icon: self.icon,
            size: if let Some(size) = self.icon_size {
                size
            } else if self.compact {
                nana_ui_core::ControlSize::Medium.text_size_in(node_theme.typography())
            } else {
                self.size.text_size_in(node_theme.typography())
            },
            tooltip: self.tooltip.as_ref().map(|tooltip| crate::TooltipVisual {
                label: Arc::clone(&tooltip.label),
                config: tooltip.config,
                open: self.tooltip_open,
            }),
        };
        if world.widget_visual_as::<IconVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let selected = self.selected || self.kind == nana_ui_core::ButtonKind::Selected;
        let mut effective_style = if self.compact {
            compact_icon_style(self.style.clone())
        } else {
            self.style.clone()
        };
        if !self.colors_from_style {
            kind_colors(self.kind, selected, &mut effective_style);
            if self.disabled {
                // A quiet icon's resting ink and the faint disabled ink sit
                // close together, closest on dark themes: a disabled icon also
                // fades, so a row of tools shows at a glance which can act.
                let disabled_opacity = world.node_theme(id).opacity().disabled;
                let layout = Arc::make_mut(&mut effective_style.layout);
                layout.opacity = Some(layout.opacity.unwrap_or(1.0) * disabled_opacity);
            }
        }
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::clone(&self.label)),
                disabled: self.disabled,
                selected: Some(selected),
                has_popup: self.has_popup,
                expanded: self.expanded,
                ..AccessibilityState::default()
            },
        );
    }
}

/// The colours an [`IconButton`] of `kind` paints with, written over `style`.
/// Skipped for a button that opted into [`IconButton::colors_from_style`].
fn kind_colors(kind: nana_ui_core::ButtonKind, selected: bool, style: &mut NodeStyle) {
    style.background = match kind {
        nana_ui_core::ButtonKind::Primary => Some(nana_ui_core::SemanticColorRole::Accent),
        nana_ui_core::ButtonKind::Subtle => Some(nana_ui_core::SemanticColorRole::Subtle),
        nana_ui_core::ButtonKind::Selected => Some(nana_ui_core::SemanticColorRole::Selected),
        _ => None,
    };
    style.foreground = Some(match kind {
        nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentText,
        nana_ui_core::ButtonKind::Warning => nana_ui_core::SemanticColorRole::Warning,
        nana_ui_core::ButtonKind::Danger => nana_ui_core::SemanticColorRole::Danger,
        _ => nana_ui_core::SemanticColorRole::Muted,
    });
    style.interaction.hovered.background = Some(match kind {
        nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentStrong,
        nana_ui_core::ButtonKind::Subtle
        | nana_ui_core::ButtonKind::Selected
        | nana_ui_core::ButtonKind::Ghost
        | nana_ui_core::ButtonKind::Warning
        | nana_ui_core::ButtonKind::Danger
        | nana_ui_core::ButtonKind::Text
        | nana_ui_core::ButtonKind::Menu => nana_ui_core::SemanticColorRole::Hover,
    });
    style.interaction.pressed.background = Some(match kind {
        nana_ui_core::ButtonKind::Primary => nana_ui_core::SemanticColorRole::AccentStrong,
        _ => nana_ui_core::SemanticColorRole::Active,
    });
    style.interaction.selected.foreground = Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
    style.interaction.selected.background = Some(nana_ui_core::SemanticColorRole::AccentSoft);
    style.interaction.selected_hovered.foreground =
        Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
    style.interaction.selected_hovered.background =
        Some(nana_ui_core::SemanticColorRole::AccentSoftHover);
    style.interaction.selected_pressed.foreground =
        Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
    style.interaction.selected_pressed.background =
        Some(nana_ui_core::SemanticColorRole::AccentSoftPressed);
    if selected {
        style.background = Some(nana_ui_core::SemanticColorRole::AccentSoft);
        style.foreground = Some(nana_ui_core::SemanticColorRole::AccentOnSoft);
    }
}

/// Non-interactive glyph for inline decoration. Hosts that need a hit target or
/// hover paint use [`IconButton`] instead.
#[derive(Debug, Clone, PartialEq)]
pub struct IconGlyph {
    pub icon: nana_ui_core::Icon,
    pub size: f32,
    /// Optional size intent shared by its box and painted glyph.
    pub themed_size: Option<nana_ui_core::ThemeExpr>,
    pub decorative: bool,
    pub style: NodeStyle,
}

impl IconGlyph {
    pub fn new(icon: nana_ui_core::Icon) -> Self {
        Self {
            icon,
            size: nana_ui_core::ControlSize::Small.icon_size(),
            themed_size: Some(nana_ui_core::ThemeToken::Type(nana_ui_core::TypeRole::Meta).into()),
            decorative: false,
            style: NodeStyle {
                foreground: Some(nana_ui_core::SemanticColorRole::Muted),
                ..NodeStyle::default()
            },
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(0.0);
        self.themed_size = None;
        self
    }

    /// Use one of the standard type-scale extents for inline decoration.
    pub fn size_role(self, role: nana_ui_core::TypeRole) -> Self {
        self.size_theme(nana_ui_core::ThemeToken::Type(role).into())
    }

    /// Keep layout and paint on the same live theme expression.
    pub fn size_theme(mut self, size: nana_ui_core::ThemeExpr) -> Self {
        self.size = size
            .resolve(nana_ui_core::builtin_theme(
                nana_ui_core::ThemeAppearance::Dark,
            ))
            .max(0.0);
        self.themed_size = Some(size);
        self
    }

    /// Keep the glyph painted and drop it from the accessibility tree.
    pub const fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    pub fn role(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style.foreground = Some(role);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        let edge = nana_ui_core::LengthSpec::Px(self.size);
        layout.width = Some(edge);
        layout.height = Some(edge);
        layout.min_width = Some(edge);
        layout.min_height = Some(edge);
        layout.theme.width = self.themed_size.clone();
        layout.theme.height = self.themed_size.clone();
        layout.theme.min_width = self.themed_size.clone();
        layout.theme.min_height = self.themed_size.clone();
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        style
    }
}

impl ComponentView for IconGlyph {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::ICON_GLYPH.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "icon-glyph".into(),
        }
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        let visual = IconVisual {
            icon: self.icon,
            size: self.themed_size.as_ref().map_or(self.size, |size| {
                size.resolve(world.node_theme(id)).max(0.0)
            }),
            tooltip: None,
        };
        if world.widget_visual_as::<IconVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                hidden: self.decorative,
                ..AccessibilityState::default()
            },
        );
    }
}
