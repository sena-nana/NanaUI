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

use super::button::{IconButtonTooltip, IconVisual};
use super::layout::Stack;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ListItemAssembled {
    pub(crate) copy_slot: Option<StableNodeId>,
    pub(crate) label_line_slot: Option<StableNodeId>,
    pub(crate) label_slot: Option<StableNodeId>,
    pub(crate) note_slot: Option<StableNodeId>,
    pub(crate) detail_slot: Option<StableNodeId>,
    pub(crate) trailing_auto_margin: Option<StableNodeId>,
}

impl ListItemAssembled {
    pub(crate) fn is_empty(&self) -> bool {
        self.copy_slot.is_none()
            && self.label_line_slot.is_none()
            && self.label_slot.is_none()
            && self.note_slot.is_none()
            && self.detail_slot.is_none()
            && self.trailing_auto_margin.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub label: String,
    /// 紧跟主文本的次要文字（如译名后的原文）：非空且未挂 content 槽时，
    /// 在主文本之后渲染为与 detail 同字号、同取色的文本区，不改变行高。
    /// 放不下时先让给主文本与 detail，宽度不够就不画；无障碍名照样带上。
    pub note: String,
    /// 单行补充信息：非空且未挂 content 槽时渲染为行尾右对齐的小字号
    /// muted 文本区，不改变行高。
    pub detail: String,
    pub selected: bool,
    pub disabled: bool,
    /// The row only shows something: it takes no pointer, keyboard focus or
    /// activation, so it neither highlights on hover nor offers a click, yet
    /// it reads in the normal text colour and is not reported disabled. Use
    /// [`Self::disabled`] for a row that could act but cannot now.
    pub read_only: bool,
    pub(crate) slots: ListItemSlots,
    /// Application node placed immediately after the label (and [`Self::note`]),
    /// before [`Self::detail`] and the trailing slot. Ignored when a custom
    /// content slot replaces the label.
    pub after_label: Option<StableNodeId>,
    pub gap: f32,
    pub size: nana_ui_core::ControlSize,
    pub auto_height: bool,
    /// Where the leading and trailing slots sit across the row; `None`
    /// follows [`Self::auto_height`]. See [`TrailingAlign`].
    pub trailing_align: Option<TrailingAlign>,
    /// 行 pill 相对文本线水平外扩，语义见 [`ListItem::pill_bleed`]。
    pub pill_bleed: bool,
    /// What the row is to assistive technology; the look is the same.
    pub role: ListItemRole,
    /// Opens a menu (`aria-haspopup="menu"`): announced as a menu button,
    /// and ArrowUp / ArrowDown on it raise [`SecondaryPress`] with
    /// `keyboard` set. See [`AccessibilityState::has_popup`].
    pub has_popup: bool,
    /// The trailing slot holds row tools: shown while the pointer is on the
    /// row or keyboard focus is inside it, so Tab reaches them. Set by
    /// `El::tools`; see [`RowToolsRest`] for how they sit while put away.
    pub trailing_tools: bool,
    pub tools_rest: RowToolsRest,
    /// Row tools stay shown at rest too (a pin that is on).
    pub keep_tools: bool,
    /// Kept by the framework from the pointer and keyboard focus.
    pub tools_presence: RowToolsPresence,
    /// Shown after the pointer rests on the row (not on its slots), like an
    /// [`IconButton`]'s. A [`Self::read_only`] row with one still takes the
    /// pointer for it, without a hover look, focus or activation; a disabled
    /// row shows none. `None` or an empty label for none.
    pub tooltip: Option<IconButtonTooltip>,
    pub style: NodeStyle,
}

/// Where a [`ListItem`]'s leading and trailing slots sit across the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrailingAlign {
    /// Centred on the whole row: right for a row of one line.
    #[default]
    Center,
    /// On the row's first line: a control after a name with a description
    /// under it lines up with the name, not with the middle of both lines.
    /// The row's first line sits where a one-line row's does (a row with a
    /// leading or trailing slot pads it into its control height; one with
    /// neither keeps hugging its lines), and each slot is centred in that band,
    /// so one-line and multi-line rows of a list share the slot's place.
    /// The first line is the row's own line box: a content slot whose first
    /// line is another size should say so with its own layout.
    ///
    /// Vertical padding the row's style already gives an edge (pixels or a
    /// theme step, on that edge, through `padding`, or as
    /// `control_padding_y`) stays: the first-line inset only pads an edge
    /// left unset. The band hangs from the content edge, so the slots still
    /// centre on the first line; a padding thinner than the inset lets the
    /// band reach past the row's edge.
    ///
    /// The default for [`ListItem::auto_height`] rows. The row holds each
    /// side slot in a band node of its own, a child of the row between the
    /// row and the slot, and projects the band from its current alignment,
    /// size and theme. It puts the slots into their bands and takes them out
    /// again whenever its alignment changes, however the slots were given
    /// (`El::leading` / `trailing` / `tools`, or
    /// `AppContext::set_list_item_slots`) and however the alignment changed
    /// (set before or after the slots, or bound), so a row may switch.
    FirstLine,
}

/// How a row's tools sit while they are put away (the pointer is elsewhere
/// and keyboard focus is outside the row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowToolsRest {
    /// Out of the layout: the label owns the whole row at rest, and revealing
    /// the tools reflows it. Tab reaches them once focus is on the row;
    /// Shift+Tab from below skips them and lands on the row.
    #[default]
    Yield,
    /// Kept in place but not drawn: revealing them never reflows the row, at
    /// the cost of the width they hold. Tab order includes them throughout.
    Reserve,
}

/// What reveals a row's tools; the framework keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowToolsPresence {
    /// The pointer is on the row.
    pub hovered: bool,
    /// Keyboard focus is on the row or inside it.
    pub focus_within: bool,
}

impl RowToolsPresence {
    pub fn revealed(self) -> bool {
        self.hovered || self.focus_within
    }
}

/// Puts a row's tools node away or back without touching the rest of its
/// style. Returns whether anything changed.
pub(crate) fn place_row_tools(
    layout: &mut nana_ui_core::LayoutStyle,
    shown: bool,
    rest: RowToolsRest,
) -> bool {
    let (hidden, opacity) = match rest {
        RowToolsRest::Yield => (!shown, layout.opacity),
        RowToolsRest::Reserve => (false, (!shown).then_some(0.0)),
    };
    let changed = layout.hidden != hidden || layout.opacity != opacity;
    layout.hidden = hidden;
    layout.opacity = opacity;
    changed
}

/// What a [`ListItem`] says it is, independent of how it looks. A row that
/// is not in a list — an entry of a dock, a capsule in a strip — is a button,
/// and one whose selected look means "on" (an open card) is a toggle button
/// that reports itself pressed rather than selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListItemRole {
    /// A row of a list; [`ListItem::selected`] is reported as selected.
    #[default]
    ListItem,
    /// A button. A selected look is still reported as selected.
    Button,
    /// A toggle button: [`ListItem::selected`] is reported as pressed.
    ToggleButton,
}

/// Retained visual state for [`ListItem`] rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListItemVisual {
    pub leading: Option<StableNodeId>,
    pub content: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
    pub note: Option<Arc<str>>,
    pub detail: Option<Arc<str>>,
}

impl crate::WidgetVisual for ListItemVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn tracks_text_content_part_budget(&self) -> bool {
        self.content.is_none()
    }

    fn caps_control_text(&self) -> bool {
        true
    }

    fn lone_list_trailing_slots(&self) -> Option<(StableNodeId, Option<StableNodeId>)> {
        self.leading.map(|leading| (leading, self.content))
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        prev.and_then(|p| p.downcast_ref::<Self>()) != Some(self)
    }

    fn clearing_changes_content_layout(&self) -> bool {
        true
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        true
    }

    fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _layout: crate::LayoutBox,
        available_width: f32,
        _available_height: f32,
    ) -> Option<f32> {
        self.content.is_none().then(|| {
            world
                .content_part_box(id, crate::LayoutPartKind::TextContent)
                .map_or(available_width, |part| part.width.max(0.0))
        })
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        if self.content.is_some() {
            return Some(crate::layout_engine::parts::NO_CONTENT_PARTS);
        }
        let style = world.computed_style(id)?;
        let reverse = style.direction == nana_ui_core::DirSpec::Rtl;
        let gap = world.flex_row_main_gap(id, content.width, content.height);
        let slot_box = |slot: StableNodeId| {
            if own_frame {
                world.layout_box_from(id, slot)
            } else {
                world.component_layout_box(slot)
            }
        };
        let mut left = content.x;
        let mut right = content.x + content.width;
        for (slot, start) in [(self.leading, true), (self.trailing, false)] {
            let Some(slot) = slot.filter(|slot| {
                world
                    .node_style(*slot)
                    .is_none_or(|style| !style.layout.omits_box())
            }) else {
                continue;
            };
            let Some(bounds) = slot_box(slot) else {
                continue;
            };
            if start != reverse {
                left = left.max(bounds.x + bounds.width + gap);
            } else {
                right = right.min(bounds.x - gap);
            }
        }
        let bounds = crate::LayoutBox {
            x: left,
            width: (right - left).max(0.0),
            ..content
        };
        if self.note.as_ref().is_none_or(|text| text.is_empty())
            && self.detail.as_ref().is_none_or(|text| text.is_empty())
        {
            return Some(crate::layout_engine::parts::content_parts([Some(
                crate::LayoutPart::new(crate::LayoutPartKind::TextContent, bounds),
            )]));
        }
        let measure = world.chrome_text_measure(id);
        let node_theme = world.node_theme(id);
        let typography = node_theme.typography();
        let spacing = node_theme.spacing();
        let size = typography.meta;
        let height = (size * typography.line / typography.body).min(bounds.height.max(size));
        let secondary = |text: &Option<Arc<str>>| {
            text.as_ref().filter(|text| !text.is_empty()).map(|text| {
                crate::layout_engine::Size::new(
                    measure.width(text, size, Some(typography.regular)),
                    height,
                )
            })
        };
        Some(crate::layout_engine::parts::list_text_parts(
            bounds,
            crate::layout_engine::parts::RowTextMetrics {
                label_width: measure.width(
                    world.text(id).unwrap_or_default(),
                    style.font_size,
                    style.font_weight,
                ),
                note: secondary(&self.note),
                detail: secondary(&self.detail),
                note_gap: spacing.sm,
                detail_gap: spacing.md,
                min_secondary_width: spacing.xxxl,
            },
            reverse,
        ))
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let parts = ctx.content_parts();
        let part = |kind| crate::layout_engine::parts::part_bounds(&parts, kind);
        let primary = ctx.style.color.unwrap_or_else(|| {
            ctx.style_model
                .color(nana_ui_core::SemanticColorRole::Text)
                .as_rgba_array()
        });
        let secondary = ctx.world.row_secondary_text_color(ctx.id, primary);
        let node_text_bounds = self
            .content
            .and_then(|slot| ctx.slot_box(slot))
            .or_else(|| part(crate::LayoutPartKind::TextContent))
            .or(Some(ctx.content));
        let mut out = crate::WidgetPaintOutput {
            aligns_child_icon_to_text: true,
            node_text_bounds,
            ..crate::WidgetPaintOutput::default()
        };
        let meta_size = ctx.theme.typography().meta;
        let regular_weight = Some(ctx.theme.typography().regular);
        let vertical_alignment = ctx.source_style.text_vertical_alignment;
        let push_secondary =
            |out: &mut crate::WidgetPaintOutput,
             slot: u64,
             text: &Option<Arc<str>>,
             kind: crate::LayoutPartKind,
             horizontal_alignment: TextHorizontalAlignment| {
                let Some((text, bounds)) = text.as_ref().zip(part(kind)) else {
                    return;
                };
                out.push(
                    slot,
                    crate::WidgetPaintOp::Text(crate::WidgetText {
                        bounds,
                        content: Arc::clone(text).into(),
                        color: Some(secondary),
                        paint_color: ctx.matching_text_paint(secondary),
                        font_size: meta_size,
                        font_weight: regular_weight,
                        horizontal_alignment,
                        vertical_alignment,
                        ellipsis: true,
                        wrap: false,
                        max_lines: None,
                        italic: None,
                        underline: None,
                        line_through: None,
                        suppress_spans: false,
                        preserve_lines: None,
                        unstyled_glyph: false,
                        line_height: Some(Some(nana_ui_core::LineHeightSpec::Absolute(
                            bounds.height,
                        ))),
                        retained_text: crate::WidgetRetainedText::None,
                        tabular_nums: false,
                        ..Default::default()
                    }),
                );
            };
        push_secondary(
            &mut out,
            4,
            &self.note,
            crate::LayoutPartKind::Note,
            TextHorizontalAlignment::Start,
        );
        push_secondary(
            &mut out,
            3,
            &self.detail,
            crate::LayoutPartKind::Detail,
            TextHorizontalAlignment::End,
        );
        if ctx.is_focus_visible() {
            let ring_color = ctx
                .style_model
                .color(
                    ctx.theme
                        .recipes()
                        .foreground(nana_ui_core::ComponentRecipeId::ListItem, false),
                )
                .as_rgba_array();
            out.push(
                7,
                crate::WidgetPaintOp::FocusRing {
                    bounds: ctx.bounds,
                    color: ring_color,
                    extra_outset: 0.0,
                },
            );
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListItemSlots {
    pub leading: Option<StableNodeId>,
    pub content: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
}

static LIST_ITEM_DEFAULT_STYLE: std::sync::LazyLock<NodeStyle> = std::sync::LazyLock::new(|| {
    let mut layout = (*control_layout()).clone();
    layout.direction = Some(nana_ui_core::FlexDirection::Row);
    layout.align_items = nana_ui_core::AlignSpec::Center;
    layout.gap = Some(nana_ui_core::LengthSpec::Px(nana_ui_core::space::MD));
    layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
    layout.font_size = Some(nana_ui_core::ControlSize::Small.text_size());
    layout
        .theme
        .control_typography(nana_ui_core::ControlSize::Small);
    layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(
        nana_ui_core::ControlSize::Small.line_height(),
    ));
    // 列表行是单行原语：过长文本截断省略，不折行撑破固定行高。
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    NodeStyle {
        layout: Arc::new(layout),
        background: None,
        shape: Some(nana_ui_core::ShapeRole::ListItem),
        control_height: Some(nana_ui_core::ControlHeight::Min(
            nana_ui_core::ControlSize::Large,
        )),
        control_padding_x: Some(nana_ui_core::ControlPadding::ListItem),
        interaction: crate::InteractionStyle {
            selected: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Selected),
                ..SemanticPaint::default()
            },
            selected_hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::SelectedHover),
                border: Some(nana_ui_core::SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            selected_pressed: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::SelectedPressed),
                border: Some(nana_ui_core::SemanticColorRole::Accent),
                ..SemanticPaint::default()
            },
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
                ..SemanticPaint::default()
            },
            ..crate::InteractionStyle::default()
        }
        .into(),
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
});

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            note: String::new(),
            detail: String::new(),
            selected: false,
            disabled: false,
            read_only: false,
            slots: ListItemSlots::default(),
            after_label: None,
            gap: nana_ui_core::space::MD,
            size: nana_ui_core::ControlSize::Small,
            auto_height: false,
            trailing_align: None,
            pill_bleed: false,
            role: ListItemRole::ListItem,
            has_popup: false,
            trailing_tools: false,
            tools_rest: RowToolsRest::Yield,
            keep_tools: false,
            tools_presence: RowToolsPresence::default(),
            tooltip: None,
            style: LIST_ITEM_DEFAULT_STYLE.clone(),
        }
    }

    /// What the row is to assistive technology; see [`ListItemRole`].
    pub fn role(mut self, role: ListItemRole) -> Self {
        self.role = role;
        self
    }

    /// The row opens a menu; see [`Self::has_popup`](field@Self::has_popup).
    pub fn has_popup(mut self, has_popup: bool) -> Self {
        self.has_popup = has_popup;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 设置紧跟主文本的次要文字；空串清除。仅在未挂 content 槽时生效。
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    /// 设置单行补充信息；空串清除。仅在未挂 content 槽时生效。
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The row only shows something; see [`Self::read_only`](field@Self::read_only).
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Takes the pointer, focus and activation: neither disabled nor
    /// read-only.
    pub(crate) fn interactive(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// See [`Self::tooltip`](field@Self::tooltip).
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

    /// The tooltip the row shows now: none while disabled or empty.
    pub(crate) fn shown_tooltip(&self) -> Option<&IconButtonTooltip> {
        self.tooltip
            .as_ref()
            .filter(|tooltip| !self.disabled && !tooltip.label.is_empty())
    }

    /// Takes the pointer: interactive, or read-only only for its tooltip.
    fn takes_pointer(&self) -> bool {
        self.interactive() || self.shown_tooltip().is_some()
    }
    /// Declarative projection hook for adapters that already own and validate
    /// the retained hierarchy. Direct Runtime consumers should prefer
    /// `AppContext::set_list_item_slots`, which validates and orders children
    /// atomically.
    pub fn slots(mut self, slots: ListItemSlots) -> Self {
        self.slots = slots;
        self
    }

    /// Put `node` immediately after the label (and [`Self::note`]), before
    /// [`Self::detail`] and the trailing slot. Assembled with the row's slots.
    pub fn after_label(mut self, node: StableNodeId) -> Self {
        self.after_label = Some(node);
        self
    }
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        Arc::make_mut(&mut self.style.layout).gap = Some(nana_ui_core::LengthSpec::Px(self.gap));
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        // The step, not the pixels: the installed metrics decide the box.
        self.style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
        self.style.control_padding_x = Some(size.into());
        let layout = Arc::make_mut(&mut self.style.layout);
        layout.font_size = Some(size.text_size());
        layout.theme.control_typography(size);
        layout.line_height = Some(nana_ui_core::LineHeightSpec::Absolute(size.line_height()));
        self
    }
    pub fn auto_height(mut self, auto_height: bool) -> Self {
        self.auto_height = auto_height;
        if auto_height {
            Arc::make_mut(&mut self.style.layout).min_height = None;
        }
        self
    }

    /// Where the leading and trailing slots sit across the row; see
    /// [`TrailingAlign`]. Without this, an [`Self::auto_height`] row puts
    /// them on its first line and any other row centres them.
    pub fn trailing_align(mut self, align: TrailingAlign) -> Self {
        self.trailing_align = Some(align);
        self
    }

    /// [`Self::trailing_align`] as given, else as `auto_height` implies.
    pub fn resolved_trailing_align(&self) -> TrailingAlign {
        self.trailing_align.unwrap_or(if self.auto_height {
            TrailingAlign::FirstLine
        } else {
            TrailingAlign::Center
        })
    }

    /// The band a [`TrailingAlign::FirstLine`] row's slots are centred in:
    /// the row's control height, the height of a one-line row.
    pub(crate) fn first_line_band(&self) -> nana_ui_core::ThemeExpr {
        let size = self
            .style
            .control_height
            .map_or(self.size, nana_ui_core::ControlHeight::size);
        nana_ui_core::ThemeToken::Control(size).into()
    }

    /// How far the row pads its first line to centre it in
    /// [`Self::first_line_band`]; the band counts the row's border, as a
    /// one-line row's height does.
    pub(crate) fn first_line_inset(&self) -> nana_ui_core::ThemeExpr {
        use nana_ui_core::{ThemeExpr, ThemeToken};
        let line = match self.size {
            nana_ui_core::ControlSize::Large => nana_ui_core::LineRole::Tall,
            _ => nana_ui_core::LineRole::Line,
        };
        self.first_line_band()
            .plus(ThemeExpr::from(ThemeToken::Line(line)).scaled(-1.0))
            .scaled(0.5)
            .plus(
                ThemeExpr::from(ThemeToken::Border(nana_ui_core::BorderWidth::Hairline))
                    .scaled(-1.0),
            )
    }

    /// How far a side slot's band reaches up from the row's content edge to
    /// the row's border edge, where a one-line row's band starts.
    pub(crate) fn first_line_reach(&self) -> nana_ui_core::ThemeExpr {
        self.first_line_inset().plus(nana_ui_core::ThemeExpr::from(
            nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline),
        ))
    }

    /// Whether the side slots sit in first-line bands.
    pub(crate) fn bands_slots(&self) -> bool {
        self.resolved_trailing_align() == TrailingAlign::FirstLine
    }

    /// A side slot's first-line band as the row has it now: as tall as a
    /// one-line row and reaching up over the row's first-line inset, so the
    /// slot centres on the first line whatever its own height. It reaches as
    /// far down, so it takes only the first line's height in the row and a
    /// one-line row is no taller than a centred one. A trailing band of row
    /// tools that are put away takes no room either.
    fn band_style(&self, trailing: bool, push_end: bool) -> NodeStyle {
        let band = self.first_line_band();
        let reach = self.first_line_reach().scaled(-1.0);
        let away = trailing
            && self.trailing_tools
            && self.tools_rest == RowToolsRest::Yield
            && !self.tools_shown();
        Stack::row(0.0)
            .shrink(0.0)
            .align(nana_ui_core::AlignSpec::Center)
            .with_layout(|layout| {
                layout.theme.min_height = Some(band);
                layout.theme.margin_top = Some(reach.clone());
                layout.theme.margin_bottom = Some(reach);
                layout.hidden = away;
                if trailing && push_end {
                    layout.logical_edges.margin.inline_start = Some(nana_ui_core::LengthSpec::Auto);
                    layout.resolve_logical_box_edges();
                }
            })
            .node_style()
    }

    /// Projects a side slot's band, which this row owns, from the row's
    /// current state; see [`list_item_band`].
    pub(crate) fn project_band(
        &self,
        band: StableNodeId,
        trailing: bool,
        push_end: bool,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        project_common(
            band,
            world,
            mutations,
            &self.band_style(trailing, push_end),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }

    /// 行 pill 相对文本线水平外扩一个内边距：margin 取自身水平 padding
    /// 的负值，文本内缩不变。用于宿主面板把行文本对齐到内容线、同时
    /// pill 保留文本呼吸区的场景。
    pub fn pill_bleed(mut self, pill_bleed: bool) -> Self {
        self.pill_bleed = pill_bleed;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// How the row tools sit while put away; see [`RowToolsRest`].
    pub fn tools_rest(mut self, rest: RowToolsRest) -> Self {
        self.tools_rest = rest;
        self
    }

    /// Keep the row tools shown at rest too.
    pub fn keep_tools(mut self, keep: bool) -> Self {
        self.keep_tools = keep;
        self
    }

    fn tools_shown(&self) -> bool {
        self.keep_tools || self.tools_presence.revealed()
    }

    /// The trailing slot as laid out: row tools that yield their width are
    /// out of the row while put away.
    fn visible_trailing(&self) -> Option<StableNodeId> {
        self.slots.trailing.filter(|_| {
            !self.trailing_tools || self.tools_rest == RowToolsRest::Reserve || self.tools_shown()
        })
    }

    /// Patches the trailing slot's visibility (for row tools) and end push
    /// (when middle body children pack from the start).
    fn project_trailing(
        &self,
        has_body: bool,
        trailing_auto_margin: Option<StableNodeId>,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        let Some(trailing) = self.slots.trailing else {
            return;
        };
        if world.node(trailing).is_none() {
            return;
        }
        let mut style = world.node_style(trailing).cloned().unwrap_or_default();
        let layout = Arc::make_mut(&mut style.layout);
        let mut changed = if self.trailing_tools {
            place_row_tools(layout, self.tools_shown(), self.tools_rest)
        } else {
            false
        };
        let wants_auto = has_body && !self.bands_slots();
        if wants_auto {
            if layout.logical_edges.margin.inline_start.is_none() && layout.margin_left.is_none() {
                layout.logical_edges.margin.inline_start = Some(nana_ui_core::LengthSpec::Auto);
                layout.resolve_logical_box_edges();
                changed = true;
            }
        } else if trailing_auto_margin == Some(trailing)
            && layout.logical_edges.margin.inline_start == Some(nana_ui_core::LengthSpec::Auto)
        {
            layout.logical_edges.margin.inline_start = None;
            layout.margin_left = None;
            layout.margin_right = None;
            layout.resolve_logical_box_edges();
            changed = true;
        }
        if changed {
            mutations.set_style(trailing, style);
        }
    }

    /// Whether `id` carries middle child nodes (a content slot, an assembled
    /// `after_label` copy row, or custom `.children(..)`).
    pub(crate) fn has_body_children_with(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        copy_slot: Option<StableNodeId>,
    ) -> bool {
        if self.slots.content.is_some() || copy_slot.is_some() {
            return true;
        }
        let Some(node) = world.node(id) else {
            return false;
        };
        node.children.iter().copied().any(|child| {
            Some(child) != self.slots.leading
                && Some(child) != self.slots.trailing
                && !is_list_item_internal(world, child)
        })
    }

    /// 行内子节点的对齐语义：无中间子节点时，有 trailing 槽则贴行尾（兼有
    /// leading 时两端对齐），否则按 start 排；有中间子节点时一律从行首起排，
    /// 由 trailing 自身的 inline-start auto margin 推向行尾。
    #[cfg(test)]
    pub(in crate::view_components) fn effective_style(&self) -> NodeStyle {
        self.effective_style_in(nana_ui_core::UI_METRICS, self.slots.content.is_some())
    }

    fn effective_style_in(&self, metrics: nana_ui_core::ThemeMetrics, has_body: bool) -> NodeStyle {
        let mut style = self.style.clone();
        if self.read_only {
            // Taking the pointer only for a tooltip, it still reads as text.
            let interaction = &mut style.interaction;
            interaction.hovered = SemanticPaint::default();
            interaction.pressed = SemanticPaint::default();
            interaction.selected_hovered = interaction.selected;
            interaction.selected_pressed = interaction.selected;
        }
        let inset = style
            .control_padding_x
            .map(|padding| padding.resolve(metrics));
        let padded_y = style.control_padding_y.is_some();
        let layout = Arc::make_mut(&mut style.layout);
        layout.justify_content = if self.visible_trailing().is_some() && !has_body {
            if self.slots.leading.is_some() {
                nana_ui_core::JustifySpec::SpaceBetween
            } else {
                nana_ui_core::JustifySpec::End
            }
        } else {
            nana_ui_core::JustifySpec::Start
        };
        // A row without side slots has nothing to line up; it keeps hugging
        // its lines. Tools put away still count, so revealing them never
        // moves the row's lines.
        let has_side = self.slots.leading.is_some() || self.slots.trailing.is_some();
        if has_side && self.resolved_trailing_align() == TrailingAlign::FirstLine {
            layout.align_items = nana_ui_core::AlignSpec::Start;
            // Padding the author gave an edge (pixels or a theme step) stays;
            // the first-line inset only pads an edge left unset. The band
            // hangs from the content edge, so it centres on the first line
            // either way.
            let padded_y = padded_y || layout.padding.is_some() || layout.theme.padding.is_some();
            let inset = self.first_line_inset();
            if !padded_y && layout.padding_top.is_none() && layout.theme.padding_top.is_none() {
                layout.theme.padding_top = Some(inset.clone());
            }
            if !padded_y && layout.padding_bottom.is_none() && layout.theme.padding_bottom.is_none()
            {
                layout.theme.padding_bottom = Some(inset);
            }
        }
        if self.pill_bleed {
            // 对称外扩与书写方向无关。Intent 还没写成像素时按安装值算；
            // 已经写成 `Px` 的显式 padding 仍按它外扩。
            let bleed = |named: Option<f32>, edge: &Option<nana_ui_core::LengthSpec>| {
                let px = named.unwrap_or(match edge {
                    Some(nana_ui_core::LengthSpec::Px(px)) => *px,
                    _ => 0.0,
                });
                nana_ui_core::LengthSpec::Px(-px)
            };
            layout.margin_left = Some(bleed(inset, &layout.padding_left));
            layout.margin_right = Some(bleed(inset, &layout.padding_right));
        }
        style
    }
}

/// The tag of the node a [`ListItem`] holds a side slot in while the row puts
/// its slots on its first line.
pub(crate) const LIST_ITEM_BAND_TAG: &str = "list-item-band";

/// The tag of the middle row a [`ListItem`] assembles for [`ListItem::after_label`].
pub(crate) const LIST_ITEM_COPY_TAG: &str = "list-item-copy";

/// Whether `id` is a list row's band node.
pub(crate) fn is_list_item_band(world: &UiWorld, id: StableNodeId) -> bool {
    world.element_tag(id) == Some(LIST_ITEM_BAND_TAG)
}

/// Whether `id` is an internal child node owned by a [`ListItem`] (a side
/// band, an assembled `after_label` copy row, or a tooltip anchor).
pub(crate) fn is_list_item_internal(world: &UiWorld, id: StableNodeId) -> bool {
    matches!(
        world.element_tag(id),
        Some(LIST_ITEM_BAND_TAG | LIST_ITEM_COPY_TAG | crate::framework::TOOLTIP_ANCHOR_TAG)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListItemCopy;

impl ComponentView for ListItemCopy {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: LIST_ITEM_COPY_TAG.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let style = Stack::row(0.0)
            .grow(1.0)
            .shrink(1.0)
            .align(nana_ui_core::AlignSpec::Center)
            .justify(nana_ui_core::JustifySpec::SpaceBetween)
            .with_layout(|layout| {
                layout.allow_shrink = true;
                layout.min_width = Some(nana_ui_core::LengthSpec::Px(0.0));
                layout.gap = Some(nana_ui_core::LengthSpec::Px(nana_ui_core::space::MD));
                layout.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
            })
            .node_style();
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }
}

/// The band of `item` that holds `slot`: the row's own child between the
/// row and the slot, while the row puts its slots on its first line.
pub(crate) fn list_item_band(
    world: &UiWorld,
    item: StableNodeId,
    slot: StableNodeId,
) -> Option<StableNodeId> {
    let band = world.parent_id(slot)?;
    (band != item && world.parent_id(band) == Some(item) && is_list_item_band(world, band))
        .then_some(band)
}

/// Assembles a [`ListItem`]'s `after_label` copy row, first-line bands, child
/// order and trailing tools.
fn assemble_list_item(
    cx: &mut crate::AppContext,
    item: crate::Entity<ListItem>,
) -> Result<bool, crate::FrameworkError> {
    cx.assemble_list_item(item)
}

impl ComponentView for ListItem {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::LIST_ITEM.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.sync_row_tools = Some(AppContext::sync_row_tools);
        }),
        activation: Some(crate::AppContext::activate_list_item),
        pointer: Some(&crate::list_selection::LIST_ITEM_POINTER),
        slot_assembler: Some(assemble_list_item),
        assembler: Some(assemble_list_item),
        slot_projector: Some(Self::project_with_slots),
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
            tag: "list-item".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_with_assembled(id, world, mutations, None);
    }
}

impl ListItem {
    fn project_with_slots(
        cx: &crate::AppContext,
        id: StableNodeId,
        view: &Self,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        view.project_with_assembled(
            id,
            world,
            mutations,
            cx.assembled_slot::<ListItemAssembled>(id),
        );
    }

    fn project_with_assembled(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
        assembled: Option<&ListItemAssembled>,
    ) {
        let copy_slot = assembled.and_then(|a| a.copy_slot);
        let trailing_auto_margin = assembled.and_then(|a| a.trailing_auto_margin);
        // 行文本只承载 label；Foundation 发布 note 与 detail 的布局部件
        // （note 紧跟 label，detail 位于行尾），与 label 同行但不挤占
        // 同一文本。挂了 content、after_label 或自定义子节点时由子节点自绘。
        let has_body = self.has_body_children_with(id, world, copy_slot);
        let draws_parts = !has_body;
        let has_note_part = draws_parts && !self.note.is_empty();
        let has_detail_part = draws_parts && !self.detail.is_empty();
        let visible_label = if draws_parts { self.label.as_str() } else { "" };
        let exposes_secondary = draws_parts || copy_slot.is_some();
        let shares_plain_label = draws_parts && self.note.is_empty() && self.detail.is_empty();
        let mut shared = None;
        if world.text(id) != Some(visible_label) {
            let value = if shares_plain_label {
                shared_label(&mut shared, visible_label).into()
            } else {
                visible_label.to_owned().into()
            };
            mutations.set_text(id, TextContent { value });
        }
        let trailing = self.visible_trailing();
        if self.slots.leading.is_none()
            && self.slots.content.is_none()
            && copy_slot.is_none()
            && trailing.is_none()
            && !has_note_part
            && !has_detail_part
        {
            if world.widget_visual_as::<ListItemVisual>(id) != Some(&ListItemVisual::default()) {
                mutations.set_widget_visual_handle(
                    id,
                    Some(crate::store::EMPTY_LIST_ITEM_VISUAL.clone()),
                );
            }
        } else {
            let visual = ListItemVisual {
                leading: self.slots.leading,
                content: self.slots.content.or(copy_slot),
                trailing,
                note: has_note_part.then(|| Arc::from(self.note.as_str())),
                detail: has_detail_part.then(|| Arc::from(self.detail.as_str())),
            };
            if world.widget_visual_as::<ListItemVisual>(id) != Some(&visual) {
                mutations.set_widget_visual(id, Some(visual));
            }
        }
        // A note that does not fit is not drawn, but it is still said.
        let label_arc = if shares_plain_label {
            match world.accessible_label_arc(id) {
                Some(current) if *current == *self.label => current,
                _ => shared_label(&mut shared, self.label.as_str()),
            }
        } else {
            let mut accessible = self.label.clone();
            if exposes_secondary && !self.note.is_empty() {
                accessible.push(' ');
                accessible.push_str(&self.note);
            }
            if exposes_secondary && !self.detail.is_empty() {
                accessible.push_str("  ");
                accessible.push_str(&self.detail);
            }
            Arc::from(accessible.as_str())
        };
        project_common(
            id,
            world,
            mutations,
            &self.effective_style_in(world.theme_metrics(), has_body),
            InteractionState {
                pointer_events: self.takes_pointer(),
                focusable: self.interactive(),
            },
            AccessibilityState {
                role: match self.role {
                    ListItemRole::ListItem => AccessibilityRole::ListItem,
                    ListItemRole::Button | ListItemRole::ToggleButton => AccessibilityRole::Button,
                },
                label: Some(label_arc),
                disabled: self.disabled,
                display_only: self.read_only && !self.disabled,
                // A pressed toggle paints as selected, so the look holds.
                checked: (self.role == ListItemRole::ToggleButton).then_some(self.selected),
                has_popup: self.has_popup,
                selected: match self.role {
                    ListItemRole::ListItem => Some(self.selected),
                    ListItemRole::Button => self.selected.then_some(true),
                    ListItemRole::ToggleButton => None,
                },
                ..AccessibilityState::default()
            },
        );
        self.project_trailing(has_body, trailing_auto_margin, world, mutations);
        if self.bands_slots() {
            for (slot, trailing) in [(self.slots.leading, false), (self.slots.trailing, true)] {
                if let Some(band) = slot.and_then(|slot| list_item_band(world, id, slot)) {
                    self.project_band(band, trailing, has_body, world, mutations);
                }
            }
        }
    }
}
