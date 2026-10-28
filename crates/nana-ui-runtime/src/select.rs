use std::sync::Arc;

use nana_ui_core::{ControlSize, LengthSpec, SemanticColorRole, SemanticPalette};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentElevation, ComponentGeometry,
    ComponentTextRegion, ComputedStyle, InteractionState, InteractionStyle, LayoutBox,
    MutationQueue, NodeKind, NodeStyle, SelectMenuGeometry, SelectMenuScroll, SelectOptionData,
    SemanticPaint, StableNodeId, StandardVisual, TextContent, TextVerticalAlignment, UiWorld,
};

/// Width of the drop-down arrow at the end of the field.
pub(crate) const HANDLE_WIDTH: f32 = nana_ui_core::type_scale::LINE;
const MENU_GAP: f32 = 0.0;
const MENU_PAD: f32 = crate::popover::MENU_SURFACE_PADDING;
/// The menu surface's stroke, inside its box: the rows scroll within it.
const MENU_BORDER: f32 = nana_ui_core::HAIRLINE;
use crate::popover::MENU_ITEM_GAP;
/// Width of the leading check lane in a drop-down menu row.
const MENU_CHECK_RESERVE: f32 = nana_ui_core::type_scale::LINE;
/// Between a label and its hint, in the field and in a menu row.
pub(crate) const HINT_GAP: f32 = nana_ui_core::space::MD;

/// Option identity stays application-owned. Disabled options remain visible.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub content: Option<Arc<crate::InlineContent>>,
    /// Quieter text that goes with the label — the original beside a
    /// translated name. See [`Self::hint`].
    pub hint: Option<Arc<str>>,
    pub disabled: bool,
}

impl SelectOption {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            content: None,
            hint: None,
            disabled: false,
        }
    }

    /// Quieter text drawn after the label, in the menu row at its end and in
    /// the field after the label of the option it shows, and said after the
    /// label. It gives way first when the width runs out. An empty hint is
    /// none.
    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }

    /// Fill-colour spans over the label; see [`crate::SelectOptionData::content`].
    pub fn content(mut self, content: impl Into<crate::InlineContent>) -> Self {
        (self.label, self.content) = crate::select::option_content(content.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Committed selection from pointer, keyboard or accessibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectChanged {
    pub value: Arc<str>,
}

/// 下拉选项开关变化；提交当前值时同样通知关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectToggled {
    pub open: bool,
}

/// Single-value field. Disabled options stay visible in the opened menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub value: Option<Arc<str>>,
    pub options: Vec<SelectOption>,
    /// What the field is for, said by assistive technology and never drawn
    /// (`aria-label`): a select shows its value, not its name. `None` leaves
    /// the name to whatever names the field (a settings row's label,
    /// [`crate::view::El::labelled_by`]), else to the option it shows.
    pub label: Option<Arc<str>>,
    pub placeholder: Option<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub invalid: bool,
    pub opened: bool,
    pub highlighted: Option<usize>,
    /// How the open menu is scrolled. See [`SelectMenuScroll`].
    pub menu_scroll: SelectMenuScroll,
    /// Size the field's content to its widest option and its arrow, as a
    /// native `<select>` sizes itself, rather than to the label it shows.
    /// See [`Self::fit_options`].
    pub fit_options: bool,
    pub style: NodeStyle,
}

impl Select {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(value: Option<impl Into<Arc<str>>>) -> Self {
        Self {
            value: value.map(Into::into),
            options: Vec::new(),
            label: None,
            placeholder: None,
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
            opened: false,
            highlighted: None,
            menu_scroll: SelectMenuScroll::default(),
            fit_options: false,
            style: field_style_for_size(ControlSize::Medium),
        }
    }

    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    /// See [`Self::label`]; an empty name is none.
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        let label = label.into();
        self.label = (!label.is_empty()).then_some(label);
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        let placeholder = placeholder.into();
        self.placeholder = (!placeholder.is_empty()).then_some(placeholder);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        apply_field_size(&mut self.style, size);
        self
    }

    /// Size the field like a native `<select>`: its content is as wide as its
    /// widest option (or, while it shows it, its placeholder) plus the arrow,
    /// whichever option it shows, so choosing another one does not move it.
    ///
    /// This is the field's own content width, which a content-sized width
    /// (`LengthSpec::Shrink`, CSS `max-content`) uses. The default width
    /// still fills the row, and an explicit width, `min_width` or `max_width`
    /// still applies over it.
    /// The open menu also fits the measured options within the viewport when
    /// an explicit width keeps the field narrower than those options.
    pub fn fit_options(mut self, fit: bool) -> Self {
        self.fit_options = fit;
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

    pub fn opened(mut self, opened: bool) -> Self {
        self.opened = opened;
        if opened {
            self.highlighted = self.selected_index().or_else(|| self.first_enabled());
        } else {
            self.highlighted = None;
        }
        self.menu_scroll = SelectMenuScroll::default();
        self
    }

    pub fn inactive(&self) -> bool {
        self.disabled || self.loading
    }

    /// Whether the menu is shown: open, and the field not disabled or loading.
    fn menu_open(&self) -> bool {
        self.opened && !self.inactive()
    }

    pub fn selected_index(&self) -> Option<usize> {
        let value = self.value.as_ref()?;
        self.options
            .iter()
            .position(|option| &option.value == value)
    }

    pub fn display_label(&self) -> (Arc<str>, bool) {
        if let Some(index) = self.selected_index() {
            return (Arc::clone(&self.options[index].label), false);
        }
        (
            self.placeholder.clone().unwrap_or_else(|| Arc::from("")),
            true,
        )
    }

    /// The hint of the option the field shows; none over a placeholder.
    pub fn display_hint(&self) -> Option<Arc<str>> {
        self.selected_index()
            .and_then(|index| self.options[index].hint.clone())
    }

    pub fn toggle_open(&mut self) -> bool {
        if self.inactive() {
            return false;
        }
        self.opened = !self.opened;
        if self.opened {
            self.highlighted = self.selected_index().or_else(|| self.first_enabled());
        } else {
            self.highlighted = None;
        }
        self.menu_scroll = SelectMenuScroll::default();
        true
    }

    pub fn close(&mut self) {
        self.opened = false;
        self.highlighted = None;
        self.menu_scroll = SelectMenuScroll::default();
    }

    pub fn highlight_delta(&mut self, delta: i32) -> bool {
        if self.inactive() || !self.opened || self.options.is_empty() {
            return false;
        }
        let enabled = self
            .options
            .iter()
            .enumerate()
            .filter(|(_, option)| !option.disabled)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return false;
        }
        let current = self
            .highlighted
            .and_then(|index| enabled.iter().position(|candidate| *candidate == index));
        let next = match current {
            Some(index) => {
                let len = enabled.len() as i32;
                enabled[((index as i32 + delta).rem_euclid(len)) as usize]
            }
            None if delta >= 0 => enabled[0],
            None => *enabled.last().expect("enabled is non-empty"),
        };
        if self.highlighted == Some(next) {
            return false;
        }
        self.highlighted = Some(next);
        true
    }

    pub fn commit_highlighted(&mut self) -> Option<SelectChanged> {
        let index = self.highlighted?;
        self.select_index(index)
    }

    pub fn select_index(&mut self, index: usize) -> Option<SelectChanged> {
        let option = self.options.get(index)?;
        if option.disabled || self.inactive() {
            return None;
        }
        let changed = self.value.as_deref() != Some(&*option.value);
        self.value = Some(Arc::clone(&option.value));
        self.close();
        changed.then(|| SelectChanged {
            value: Arc::clone(self.value.as_ref().expect("selected value")),
        })
    }

    fn first_enabled(&self) -> Option<usize> {
        self.options.iter().position(|option| !option.disabled)
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(if self.inactive() || self.display_label().1 {
            SemanticColorRole::Faint
        } else {
            SemanticColorRole::Text
        });
        style.background = Some(if self.inactive() {
            SemanticColorRole::Subtle
        } else {
            SemanticColorRole::Background
        });
        style.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else if self.opened {
            SemanticColorRole::BorderSoft
        } else {
            SemanticColorRole::Border
        });
        style.interaction.hovered.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::BorderStrong
        });
        style.interaction.focused.border = Some(if self.invalid {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::BorderStrong
        });
        style.interaction.disabled = SemanticPaint {
            foreground: Some(SemanticColorRole::Faint),
            background: Some(SemanticColorRole::Subtle),
            border: Some(SemanticColorRole::Border),
            ..SemanticPaint::default()
        };
        let has_explicit_height = style.layout.height.is_some();
        let needs_height = style.control_height.is_none() && !has_explicit_height;
        let needs_radius = style.layout.border_radius.is_none();
        {
            let layout = Arc::make_mut(&mut style.layout);
            if layout.width.is_none() {
                layout.width = Some(LengthSpec::Fill);
            }
            layout.border_width = Some(if self.invalid && self.opened {
                2.0
            } else {
                1.0
            });
            layout.white_space_nowrap = true;
        }
        if needs_radius {
            style.radius = Some(nana_ui_core::RadiusTier::Sm);
        }
        if has_explicit_height {
            // A CSS/host box is a spent number; don't let the size step
            // write over it.
            style.control_height = None;
        } else if needs_height {
            style.control_height = Some(nana_ui_core::ControlHeight::Exact(self.size));
        }
        style.control_padding_x = Some(self.size.into());
        style.text_vertical_alignment = TextVerticalAlignment::Center;
        style
    }
}

impl crate::ComponentView for Select {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activation: Some(crate::AppContext::toggle_select),
        activate_at: Some(crate::AppContext::activate_select_at),
        close_options: Some(|context, entity| context.set_select_opened(entity, false)),
        hooks: Some(|hooks| hooks.select = Some(&SELECT_HOOKS)),
        pointer: Some(&crate::framework::SELECT_MENU_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // Value/options are application state; the open menu, its keyboard
        // highlight and its scroll belong to the current interaction while
        // those props agree.
        if next.inactive() {
            next.close();
        } else if self.value == next.value && self.options == next.options {
            next.opened = self.opened;
            next.highlighted = self.highlighted;
            next.menu_scroll = self.menu_scroll;
        }
        *self = next;
    }

    fn announce_change(&self, previous: &Self, cx: &mut crate::ViewContext<'_, Self>) {
        let open = self.menu_open();
        if open != previous.menu_open() {
            cx.emit(SelectToggled { open });
        }
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "select".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let (label, placeholder) = self.display_label();
        let hint = self.display_hint();
        let visual = StandardVisual::Select {
            label: Arc::clone(&label),
            hint: hint.clone(),
            placeholder,
            size: self.size,
            opened: self.menu_open(),
            invalid: self.invalid,
            loading: self.loading,
            options: self
                .options
                .iter()
                .map(|option| SelectOptionData {
                    key: None,
                    label: Arc::clone(&option.label),
                    content: option.content.clone(),
                    hint: option.hint.clone(),
                    disabled: option.disabled,
                    checked: false,
                    icon: None,
                    accessible_name: None,
                    mark: crate::MenuItemMark::Command,
                })
                .collect(),
            highlighted: self.highlighted,
            menu_scroll: self.menu_scroll,
            menu_room: menu_room(self.menu_open(), id, world),
            checkable: false,
            fit_options: self.fit_options,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.text(id) != Some(label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: label.to_string().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.inactive(),
                focusable: !self.inactive(),
            },
            AccessibilityState {
                role: AccessibilityRole::ComboBox,
                // Its own name first; else named by its field (a settings
                // row's label, see `MutationQueue::set_labelled_by`), or by
                // the option it shows through its text when nothing names
                // it. The option shown is its value either way, its hint
                // said after it even when the field has no room to draw it.
                label: self.label.clone(),
                value: (!placeholder).then(|| match &hint {
                    Some(hint) => Arc::from(format!("{label} {hint}")),
                    None => label,
                }),
                disabled: self.inactive(),
                busy: self.loading,
                invalid: self.invalid,
                selected: Some(self.menu_open()),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Geometry for a select / dropdown trigger and its open menu.
///
/// `padding` is the inset the node was **laid out** with, not the one it
/// authored. A control that names its inset (`NodeStyle::control_padding_x`)
/// carries no padding on its authored box — that is the point of naming a step
/// instead of spending it — so re-deriving the inset from `source.layout` here
/// reported 0 and left the trigger's label flush against its own border. The
/// caller has already resolved it through `UiWorld::used_layout_padding`.
pub(crate) fn select_geometry(
    bounds: LayoutBox,
    label: &Arc<str>,
    hint: Option<&Arc<str>>,
    placeholder: bool,
    size: ControlSize,
    opened: bool,
    options: &[SelectOptionData],
    highlighted: Option<usize>,
    menu_scroll: SelectMenuScroll,
    menu_room: Option<crate::SelectMenuRoom>,
    style: &ComputedStyle,
    source: &NodeStyle,
    model: nana_ui_core::StyleModelRef,
    scrollbars: &nana_ui_core::ScrollbarRecipe,
    viewport: Option<crate::LayoutViewport>,
    checkable: bool,
    parts: &crate::layout_engine::parts::ContentParts,
    measure: crate::text_width::ChromeTextMeasure<'_>,
    fitted_label_width: Option<f32>,
) -> ComponentGeometry {
    let palette = &model.palette;
    let border = source.layout.resolved_border_width();
    let text_color = if placeholder || matches!(style.foreground, SemanticColorRole::Faint) {
        palette.faint.as_rgba_array()
    } else {
        style.color.unwrap_or_else(|| palette.text.as_rgba_array())
    };
    let handle_color = palette.muted.as_rgba_array();
    let menu = opened.then(|| {
        select_menu_geometry(
            bounds,
            size,
            options,
            highlighted,
            menu_scroll,
            model,
            scrollbars,
            menu_room.or_else(|| window_room(viewport)),
            checkable,
            measure,
            viewport,
            fitted_label_width,
        )
    });
    // The shown option's hint follows its label, as quiet as a menu row's,
    // and gives way first: it takes only what the label leaves.
    let label_box =
        crate::layout_engine::parts::part_bounds(parts, crate::LayoutPartKind::TextContent)
            .unwrap_or_default();
    let hint = hint
        .filter(|hint| !placeholder && !hint.is_empty())
        .and_then(|hint| {
            crate::layout_engine::parts::part_bounds(parts, crate::LayoutPartKind::Hint).map(
                |bounds| ComponentTextRegion {
                    bounds,
                    content: Arc::clone(hint).into(),
                    // A field that has gone faint takes its hint with it.
                    color: Some(if text_color == palette.faint.as_rgba_array() {
                        text_color
                    } else {
                        palette.muted.as_rgba_array()
                    }),
                    font_size: size.caption_size(),
                    font_weight: None,
                },
            )
        });
    let handle =
        crate::layout_engine::parts::part_bounds(parts, crate::LayoutPartKind::TrailingContent)
            .unwrap_or_default();
    ComponentGeometry::Select {
        label: ComponentTextRegion {
            bounds: label_box,
            content: Arc::clone(label).into(),
            color: Some(text_color),
            font_size: size.text_size(),
            font_weight: None,
        },
        hint,
        handle,
        handle_color,
        background: style.background,
        border: style.border_color,
        border_width: border,
        menu,
    }
}

fn select_menu_geometry(
    field: LayoutBox,
    size: ControlSize,
    options: &[SelectOptionData],
    highlighted: Option<usize>,
    menu_scroll: SelectMenuScroll,
    model: nana_ui_core::StyleModelRef,
    scrollbars: &nana_ui_core::ScrollbarRecipe,
    room: Option<crate::SelectMenuRoom>,
    checkable: bool,
    measure: crate::text_width::ChromeTextMeasure<'_>,
    viewport: Option<crate::LayoutViewport>,
    fitted_label_width: Option<f32>,
) -> SelectMenuGeometry {
    let palette = &model.palette;
    let metrics = model.metrics;
    let item_height = size.height_in(metrics);
    let count = options.len().max(1) as f32;
    let natural_height =
        MENU_PAD * 2.0 + count * item_height + (count - 1.0).max(0.0) * MENU_ITEM_GAP;
    let is_light = palette.background.as_rgba_array()[0] > 0.5;
    let (surface_y, height) = resolve_menu_vertical(field, natural_height, room);
    // 复用文本布局的最长选项测量，字段固定宽度时仍完整展开，并保留上游滚动口。
    let check_reserve = if checkable { MENU_CHECK_RESERVE } else { 0.0 };
    let natural_width = fitted_label_width.map_or(field.width, |width| {
        field
            .width
            .max(width + MENU_PAD * 2.0 + size.padding_x_in(metrics) * 2.0 + check_reserve)
    });
    let width = viewport.map_or(natural_width, |v| natural_width.min(v.width.max(0.0)));
    let x = viewport.map_or(field.x, |v| field.x.clamp(0.0, (v.width - width).max(0.0)));
    let surface = LayoutBox {
        x,
        y: surface_y,
        width,
        height,
    };
    // A surface capped short of its rows scrolls them inside its border and
    // cuts them there, the way CSS clips a scroll container's content at its
    // padding box; drawing them on past it painted over whatever was below.
    let rows_viewport = LayoutBox {
        x: surface.x + MENU_BORDER,
        y: surface.y + MENU_BORDER,
        width: (surface.width - MENU_BORDER * 2.0).max(0.0),
        height: (surface.height - MENU_BORDER * 2.0).max(0.0),
    };
    let max_scroll = (natural_height - height).max(0.0);
    let scroll = menu_offset(
        menu_scroll.offset,
        highlighted,
        item_height,
        height,
        max_scroll,
    );
    let scrollbar = (max_scroll > 0.0)
        .then(|| {
            menu_scrollbar(
                rows_viewport,
                natural_height,
                height,
                scroll,
                menu_scroll,
                model,
                scrollbars,
            )
        })
        .flatten();
    // The rows leave the bar its lane, so its thumb never covers a label, a
    // hint or a highlight.
    let rows_x = surface.x + MENU_PAD;
    let rows_right = scrollbar.map_or(surface.x + surface.width - MENU_PAD, |bar| bar.lane.x);
    // Reserved for the whole menu when the menu *can* check, not when one
    // option currently is: deriving it from `any(checked)` shifted every label
    // 16px sideways the moment the first option became checked.
    let options = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let y = surface.y + MENU_PAD + index as f32 * (item_height + MENU_ITEM_GAP) - scroll;
            let selected = highlighted == Some(index);
            let bounds = LayoutBox {
                x: rows_x,
                y,
                width: (rows_right - rows_x).max(0.0),
                height: item_height,
            };
            let mut label_box = LayoutBox {
                x: bounds.x + size.padding_x_in(metrics) + check_reserve,
                y: bounds.y,
                width: (bounds.width - size.padding_x_in(metrics) * 2.0 - check_reserve).max(0.0),
                height: bounds.height,
            };
            // Measuring a hint shapes text; a row out of sight is not drawn.
            let hint = overlaps_vertically(bounds, rows_viewport)
                .then(|| menu_option_hint(option, &mut label_box, size, palette, measure))
                .flatten();
            crate::SelectOptionGeometry {
                bounds,
                label: ComponentTextRegion {
                    bounds: label_box,
                    content: Arc::clone(&option.label).into(),
                    color: Some(if option.disabled {
                        palette.faint.as_rgba_array()
                    } else {
                        palette.text.as_rgba_array()
                    }),
                    font_size: size.text_size(),
                    font_weight: None,
                },
                rich_content: option.content.clone(),
                hint,
                selected,
                checked: option.checked,
                disabled: option.disabled,
                background: selected.then_some(palette.selected.as_rgba_array()),
                icon: None,
            }
        })
        .collect();
    SelectMenuGeometry {
        surface,
        viewport: rows_viewport,
        border_width: MENU_BORDER,
        scroll,
        max_scroll,
        scrollbar,
        elevation: ComponentElevation {
            color: nana_ui_core::PaintColor::srgb([
                0.0,
                0.0,
                0.0,
                if is_light { 0.24 } else { 0.48 },
            ]),
            offset_x: 0.0,
            offset_y: 8.0,
            blur_radius: 16.0,
            spread_radius: 0.0,
            inset: false,
        },
        background: palette.surface.as_rgba_array(),
        border: palette.border_soft.as_rgba_array(),
        options,
    }
}

/// A menu row's hint at its end, a step smaller and quieter than the label,
/// as an action menu draws its own. The label keeps its natural width first;
/// the hint takes what is left and ellipsizes, and under the smallest
/// readable width it is not drawn. `label_box` gives up the hint's share.
fn menu_option_hint(
    option: &SelectOptionData,
    label_box: &mut LayoutBox,
    size: ControlSize,
    palette: &SemanticPalette,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> Option<ComponentTextRegion> {
    let hint = option.hint.as_ref().filter(|hint| !hint.is_empty())?;
    let hint_size = size.caption_size();
    let (new_label, hint_bounds) = crate::layout_engine::parts::menu_option_text_boxes(
        *label_box,
        &option.label,
        Some(hint),
        size.text_size(),
        hint_size,
        HINT_GAP,
        nana_ui_core::space::XXXL,
        |text, size| measure.width(text, size, None),
    );
    *label_box = new_label;
    Some(ComponentTextRegion {
        bounds: hint_bounds?,
        content: Arc::clone(hint).into(),
        color: Some(if option.disabled {
            palette.faint.as_rgba_array()
        } else {
            palette.muted.as_rgba_array()
        }),
        font_size: hint_size,
        font_weight: None,
    })
}

/// A label and its hint as one line, where only one text fits: a searchable
/// field showing its choice.
/// Splits option content into the label rows lay out and the fill-colour
/// spans they paint over it. Option rows are data, not retained paragraphs:
/// atomic children and spans that shape, decorate or mark have nowhere to
/// live there, so debug builds refuse them and release builds keep the text.
pub(crate) fn option_content(
    content: crate::InlineContent,
) -> (Arc<str>, Option<Arc<crate::InlineContent>>) {
    let content = content.normalized();
    debug_assert!(
        content.atoms().is_empty(),
        "option rows have no retained children for atomic content"
    );
    if !content.atoms().is_empty() {
        return (Arc::from(content.text().replace('\u{fffc}', "")), None);
    }
    let label = Arc::clone(content.shared_text());
    let Some(rich) = content.rich_text() else {
        return (label, None);
    };
    debug_assert!(
        rich.objects().is_empty() && rich.rubies().is_empty(),
        "option rows paint fill colour only"
    );
    let mut fills = nana_ui_core::RichText::new(Arc::clone(&label));
    for (range, span) in rich.spans().iter() {
        let fill = span
            .paint
            .color
            .map(|color| nana_ui_core::RichSpanStyle::new().color(color));
        debug_assert!(
            fill.as_ref().unwrap_or(&nana_ui_core::RichSpanStyle::new()) == span,
            "option rows paint fill colour only"
        );
        if let Some(fill) = fill {
            fills = fills.with_span(range, fill);
        }
    }
    let fills = crate::InlineContent::from(fills);
    let content = (!matches!(fills, crate::InlineContent::PlainText(_))).then(|| Arc::new(fills));
    (label, content)
}

pub(crate) fn menu_option_label(option: &SelectOptionData) -> Arc<str> {
    match option.hint.as_ref() {
        Some(hint) if !hint.is_empty() => Arc::from(format!("{}  ·  {hint}", option.label)),
        _ => Arc::clone(&option.label),
    }
}

/// The enabled row under a point, as the menu draws it: only inside the
/// viewport the rows are cut to, and never on the scrollbar's lane.
pub(crate) fn select_option_at(menu: &SelectMenuGeometry, x: f32, y: f32) -> Option<usize> {
    if !menu.viewport.contains(x, y) || menu.scrollbar.is_some_and(|bar| bar.contains(x, y)) {
        return None;
    }
    menu.options
        .iter()
        .position(|option| !option.disabled && option.bounds.contains(x, y))
}

/// How far an open menu's rows sit scrolled: `offset` clamped to what they
/// can scroll. Before anything has scrolled them (`None`) the highlighted
/// row, the chosen one as the menu opens, sits in the middle where the list
/// allows.
fn menu_offset(
    offset: Option<f32>,
    highlighted: Option<usize>,
    item_height: f32,
    height: f32,
    max_scroll: f32,
) -> f32 {
    if max_scroll <= 0.0 {
        return 0.0;
    }
    let offset = offset.unwrap_or_else(|| {
        highlighted.map_or(0.0, |row| {
            MENU_PAD + row as f32 * (item_height + MENU_ITEM_GAP) + (item_height - height) / 2.0
        })
    });
    if offset.is_finite() {
        offset.clamp(0.0, max_scroll)
    } else {
        0.0
    }
}

/// The offset that shows `row` of an open menu whole, inside the surface's
/// padding, moving the rows as little as it can from where they are.
pub(crate) fn menu_offset_revealing(menu: &SelectMenuGeometry, row: usize) -> Option<f32> {
    let bounds = menu.options.get(row)?.bounds;
    let top = menu.surface.y + MENU_PAD;
    let bottom = menu.surface.y + menu.surface.height - MENU_PAD;
    let offset = if bounds.y < top {
        menu.scroll - (top - bounds.y)
    } else if bounds.y + bounds.height > bottom {
        menu.scroll + (bounds.y + bounds.height - bottom)
    } else {
        menu.scroll
    };
    Some(offset.clamp(0.0, menu.max_scroll))
}

/// After the keyboard moves the highlight to `row`, scroll the menu, as
/// `menu` last drew it, just far enough to show that row.
pub(crate) fn reveal_menu_row(
    scroll: &mut SelectMenuScroll,
    menu: Option<&SelectMenuGeometry>,
    row: Option<usize>,
) {
    if let (Some(menu), Some(row)) = (menu, row)
        && let Some(offset) = menu_offset_revealing(menu, row)
    {
        scroll.offset = Some(offset);
    }
}

/// The bar beside an open menu's overflowing rows, built like every scroll
/// view's: the installed scrollbar metrics, the theme's recipe, the lane
/// inside the surface's border. A menu is open only while it is in use, so
/// its bar shows for as long as the rows overflow, unless the theme hides
/// scrollbars; a resident policy adds the resting track. The thumb is at its
/// active thickness while the lane is hot or the thumb is dragged.
fn menu_scrollbar(
    rows_viewport: LayoutBox,
    content: f32,
    viewport: f32,
    offset: f32,
    state: SelectMenuScroll,
    model: nana_ui_core::StyleModelRef,
    recipe: &nana_ui_core::ScrollbarRecipe,
) -> Option<crate::ScrollbarBar> {
    use nana_ui_core::{ScrollbarRect, ScrollbarVisibility};
    if recipe.visibility == ScrollbarVisibility::Hidden {
        return None;
    }
    let dragging = state.drag.is_some();
    let expand = f32::from(u8::from(state.hot || dragging));
    let layout = nana_ui_core::scrollbar_layout(
        nana_ui_core::ScrollbarAxis::Vertical,
        ScrollbarRect {
            x: rows_viewport.x,
            y: rows_viewport.y,
            width: rows_viewport.width,
            height: rows_viewport.height,
        },
        false,
        nana_ui_core::ScrollbarExtent {
            viewport,
            content,
            offset,
            min_offset: 0.0,
        },
        expand,
        model.metrics.scrollbar,
    )?;
    let paint = recipe.paint(
        model,
        None,
        nana_ui_core::ScrollbarPaintState {
            presence: 1.0,
            expand,
            dragging,
            resident: recipe.visibility == ScrollbarVisibility::Always,
        },
    );
    let layout_box = |rect: ScrollbarRect| LayoutBox {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    Some(crate::ScrollbarBar {
        lane: layout_box(layout.lane),
        track: layout_box(layout.track),
        thumb: layout_box(layout.thumb),
        track_background: paint.track,
        thumb_background: paint.thumb,
        track_radius: layout.track_radius,
        thumb_radius: layout.thumb_radius,
        min_offset: layout.axis.min_offset,
        max_offset: layout.axis.max_offset,
    })
}

fn overlaps_vertically(row: LayoutBox, viewport: LayoutBox) -> bool {
    row.y < viewport.y + viewport.height && row.y + row.height > viewport.y
}

/// Places the drop-down surface below its field, folding it back inside the
/// room it can be seen in near that room's bottom edge.
///
/// Preference order matches the menu families: open downwards; flip above the
/// field when the space below cannot hold the menu but the space above can;
/// otherwise stay on the roomier side and cap the height to it. Without a room
/// (the document has not been laid out yet) the menu keeps its natural
/// downward placement.
fn resolve_menu_vertical(
    field: LayoutBox,
    natural_height: f32,
    room: Option<crate::SelectMenuRoom>,
) -> (f32, f32) {
    let below_y = field.y + field.height + MENU_GAP;
    let Some(room) = room else {
        return (below_y, natural_height);
    };
    let space_below = (room.bottom - below_y).max(0.0);
    if natural_height <= space_below {
        return (below_y, natural_height);
    }
    let above_y = field.y - MENU_GAP;
    let space_above = (above_y - room.top).max(0.0);
    if natural_height <= space_above {
        return (above_y - natural_height, natural_height);
    }
    if space_above > space_below {
        (above_y - space_above, space_above)
    } else {
        (below_y, space_below)
    }
}

/// The menu's x and width: its field's, or as wide as its widest row when it
/// fits its options and that is wider. A menu wider than its field starts
/// where the field does and moves left as far as it must to end inside the
/// room; one wider than the room is cut to it.
fn resolve_menu_horizontal(
    field: LayoutBox,
    fitted_width: Option<f32>,
    room: Option<crate::SelectMenuRoom>,
) -> (f32, f32) {
    let Some(fitted) = fitted_width.filter(|fitted| *fitted > field.width) else {
        return (field.x, field.width);
    };
    let Some(room) = room else {
        return (field.x, fitted);
    };
    let width = fitted.min(room.right - room.left);
    if width <= field.width {
        return (field.x, field.width);
    }
    (field.x.min(room.right - width).max(room.left), width)
}

/// What an open menu projects as its room: taken now, from the boxes the
/// last layout gave the field and everything above it. A closed menu has
/// none to keep.
pub(crate) fn menu_room(
    open: bool,
    id: StableNodeId,
    world: &UiWorld,
) -> Option<crate::SelectMenuRoom> {
    open.then(|| world.select_menu_room(id)).flatten()
}

/// The room a menu opened before its field projected with a layout has: the
/// window, where the field's own layout box meets it.
fn window_room(viewport: Option<crate::LayoutViewport>) -> Option<crate::SelectMenuRoom> {
    viewport.map(|viewport| crate::SelectMenuRoom {
        top: 0.0,
        bottom: viewport.height,
        left: 0.0,
        right: viewport.width,
    })
}

/// Re-applies only the metrics `ControlSize` owns onto an existing field style.
///
/// `size()` builders use this instead of rebuilding the whole [`NodeStyle`], so
/// a style the caller supplied keeps its colors, borders and interaction paints.
pub(crate) fn apply_field_size(style: &mut NodeStyle, size: ControlSize) {
    style.control_height = Some(nana_ui_core::ControlHeight::Exact(size));
    style.control_padding_x = Some(size.into());
}

pub(crate) fn field_style_for_size(size: ControlSize) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Fill),
            border_width: Some(nana_ui_core::HAIRLINE),
            white_space_nowrap: true,
            ..nana_ui_core::LayoutStyle::default()
        }),
        radius: Some(nana_ui_core::RadiusTier::Sm),
        control_height: Some(nana_ui_core::ControlHeight::Exact(size)),
        control_padding_x: Some(size.into()),
        foreground: Some(SemanticColorRole::Text),
        background: Some(SemanticColorRole::Background),
        border: Some(SemanticColorRole::Border),
        interaction: InteractionStyle {
            hovered: SemanticPaint {
                border: Some(SemanticColorRole::BorderStrong),
                ..SemanticPaint::default()
            },
            focused: SemanticPaint::FOCUS_BORDER,
            disabled: SemanticPaint {
                foreground: Some(SemanticColorRole::Faint),
                background: Some(SemanticColorRole::Subtle),
                border: Some(SemanticColorRole::Border),
                ..SemanticPaint::default()
            },
            ..InteractionStyle::default()
        },
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..NodeStyle::default()
    }
}

/// The router reaches this list's keyboard only once one exists.
pub(crate) static SELECT_HOOKS: crate::framework::ChoiceHooks = crate::framework::ChoiceHooks {
    adjust: crate::AppContext::adjust_focused_select,
    commit: crate::AppContext::commit_focused_select,
};

#[cfg(test)]
mod tests {
    include!("../tests/fixtures/select_fitted_menu.rs");
    #[test]
    fn only_a_multi_select_dropdown_reserves_the_check_lane() {
        use crate::{Dropdown, DropdownOption, StandardVisual};

        let checkable_of = |context: &AppContext, id| match context.world().standard_visual(id) {
            Some(StandardVisual::Select { checkable, .. }) => checkable,
            other => panic!("expected a Select visual, got {other:?}"),
        };
        let options = [DropdownOption::new("a", "A"), DropdownOption::new("b", "B")];

        let mut context = AppContext::new();
        // A plain Select never draws checks, so it must not indent its labels.
        let select = context.create_component(document(), sample()).unwrap();
        assert!(!checkable_of(&context, select.stable_id()));

        // Single-select dropdown: still no check column.
        let single = context
            .create_component(
                document(),
                Dropdown::single(Some("a")).options(options.clone()),
            )
            .unwrap();
        assert!(!checkable_of(&context, single.stable_id()));

        // Multi-select reserves the lane even while nothing is checked yet, so
        // labels do not jump sideways on the first check.
        let empty_multi = context
            .create_component(
                document(),
                Dropdown::multiple(Vec::<String>::new()).options(options.clone()),
            )
            .unwrap();
        assert!(checkable_of(&context, empty_multi.stable_id()));

        let checked_multi = context
            .create_component(
                document(),
                Dropdown::multiple(vec!["a".to_string()]).options(options),
            )
            .unwrap();
        assert!(checkable_of(&context, checked_multi.stable_id()));
    }

    fn menu_field(y: f32) -> LayoutBox {
        LayoutBox {
            x: 10.0,
            y,
            width: 120.0,
            height: 32.0,
        }
    }

    fn window(height: f32) -> Option<crate::SelectMenuRoom> {
        window_room(Some(crate::LayoutViewport::new(600.0, height)))
    }

    #[test]
    fn menu_opens_downwards_when_the_viewport_has_room() {
        let field = menu_field(100.0);
        let (y, height) = resolve_menu_vertical(field, 90.0, window(600.0));
        assert_eq!(y, field.y + field.height + MENU_GAP);
        assert_eq!(height, 90.0);
    }

    #[test]
    fn menu_flips_above_the_field_when_it_would_fall_off_the_bottom() {
        // Field near the bottom edge: 600 - (500 + 32 + gap) leaves < 90px below,
        // while 500px sit above it.
        let field = menu_field(500.0);
        let (y, height) = resolve_menu_vertical(field, 90.0, window(600.0));
        assert_eq!(height, 90.0, "a flipped menu keeps its natural height");
        assert_eq!(y, field.y - MENU_GAP - 90.0);
        assert!(y >= 0.0, "flipped menu stays inside the viewport");
    }

    #[test]
    fn a_menu_taller_than_both_sides_is_capped_to_the_roomier_one() {
        let field = menu_field(220.0);
        let (y, height) = resolve_menu_vertical(field, 5_000.0, window(300.0));
        // 220px above vs 300-(220+32+gap) below: it opens upwards, capped.
        assert_eq!(y, 0.0);
        assert!(height <= 220.0, "capped to the space above, got {height}");
        assert!(height > 0.0);
    }

    #[test]
    fn a_menu_opens_into_the_room_it_can_be_seen_in() {
        // A card scrolled 400px, its viewport 150..550 of the window: in the
        // field's layout y that is 550..950. The field at 600 has 318px
        // below it in the card and 50 above.
        let room = Some(crate::SelectMenuRoom {
            top: 550.0,
            bottom: 950.0,
            left: 0.0,
            right: 600.0,
        });
        let field = menu_field(600.0);
        let (y, height) = resolve_menu_vertical(field, 200.0, room);
        assert_eq!((y, height), (field.y + field.height + MENU_GAP, 200.0));
        let (y, height) = resolve_menu_vertical(field, 5_000.0, room);
        assert_eq!((y, height), (632.0, 318.0), "capped to the card's bottom");
        // Near the card's bottom it flips up and stops at the card's top.
        let field = menu_field(900.0);
        let (y, height) = resolve_menu_vertical(field, 5_000.0, room);
        assert_eq!((y, height), (550.0, 350.0));
    }

    #[test]
    fn menu_without_a_laid_out_viewport_keeps_its_downward_placement() {
        let field = menu_field(500.0);
        let (y, height) = resolve_menu_vertical(field, 90.0, None);
        assert_eq!(y, field.y + field.height + MENU_GAP);
        assert_eq!(height, 90.0);
    }

    #[test]
    fn size_keeps_caller_supplied_style_and_only_rewrites_size_metrics() {
        let mut custom = field_style_for_size(ControlSize::Medium);
        custom.background = Some(SemanticColorRole::Selected);
        Arc::make_mut(&mut custom.layout).border_radius = Some(17.0);

        let select = Select::new(Some("a"))
            .style(custom)
            .size(ControlSize::Large);

        // Caller-owned paint and geometry survive `size()`.
        assert_eq!(select.style.background, Some(SemanticColorRole::Selected));
        assert_eq!(select.style.layout.border_radius, Some(17.0));
        // Metrics that `ControlSize` owns are refreshed as named steps.
        assert_eq!(
            select.style.control_height,
            Some(nana_ui_core::ControlHeight::Exact(ControlSize::Large))
        );
        assert_eq!(
            select.style.control_padding_x,
            Some(ControlSize::Large.into())
        );
    }

    #[test]
    fn background_props_refresh_preserves_open_select_and_keyboard_highlight() {
        let options = [SelectOption::new("a", "A"), SelectOption::new("b", "B")];
        let mut select = Select::new(Some("a")).options(options.clone()).opened(true);
        select.highlight_delta(1);
        crate::ComponentView::reconcile(
            &mut select,
            Select::new(Some("a")).options(options).invalid(true),
        );
        assert!(select.opened);
        assert_eq!(select.highlighted, Some(1));
        assert!(select.invalid);
    }

    #[test]
    fn application_selection_or_options_change_replaces_menu_state() {
        let options = [SelectOption::new("a", "A"), SelectOption::new("b", "B")];
        let mut select = Select::new(Some("a")).options(options.clone()).opened(true);
        crate::ComponentView::reconcile(
            &mut select,
            Select::new(Some("b")).options(options.clone()),
        );
        assert!(!select.opened);
        assert_eq!(select.highlighted, None);
        assert_eq!(select.value.as_deref(), Some("b"));
        select.toggle_open();
        crate::ComponentView::reconcile(
            &mut select,
            Select::new(Some("b")).options([options[1].clone()]),
        );
        assert!(!select.opened);
        assert_eq!(select.highlighted, None);
    }

    #[test]
    fn disabling_or_loading_select_closes_its_open_menu() {
        for loading in [false, true] {
            let mut select = Select::new(Some("a"))
                .options([SelectOption::new("a", "A")])
                .opened(true);
            let next = select.clone().disabled(!loading).loading(loading);
            crate::ComponentView::reconcile(&mut select, next);
            assert!(!select.opened);
            assert_eq!(select.highlighted, None);
        }
    }

    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> Select {
        Select::new(Some("code"))
            .placeholder("Choose view")
            .options([
                SelectOption::new("code", "Code"),
                SelectOption::new("split", "Split").disabled(true),
                SelectOption::new("preview", "Preview"),
            ])
    }

    #[test]
    fn select_projects_closed_field_and_keeps_disabled_options() {
        let mut context = AppContext::new();
        let select = context.create_component(document(), sample()).unwrap();
        let id = select.stable_id();
        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "select"
        ));
        assert!(matches!(
            context.world().standard_visual(id),
            Some(StandardVisual::Select {
                placeholder: false,
                opened: false,
                ..
            })
        ));
        assert_eq!(context.world().text(id), Some("Code"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Background));
        assert_eq!(style.border, Some(SemanticColorRole::Border));
        // The field names the radius step; the number is produced at extract
        // against the installed metrics (Issue #101 F1).
        assert_eq!(style.radius, Some(nana_ui_core::RadiusTier::Sm));
        assert_eq!(
            context.world().extract_nodes(&[id])[0]
                .source_style
                .layout
                .border_radius,
            Some(nana_ui_core::UI_METRICS.radius_sm)
        );
        let visual = match context.world().standard_visual(id) {
            Some(StandardVisual::Select { options, .. }) => options,
            _ => panic!("select visual"),
        };
        assert_eq!(visual.len(), 3);
        assert!(visual[1].disabled);
    }

    #[test]
    fn a_hint_goes_with_its_option_into_the_visual_and_the_value() {
        let mut context = AppContext::new();
        let select = context
            .create_component(
                document(),
                Select::new(Some("x")).options([
                    SelectOption::new("x", "头部左右转").hint("FaceAngleX"),
                    SelectOption::new("none", "不绑定").hint(""),
                ]),
            )
            .unwrap();
        let id = select.stable_id();
        let Some(StandardVisual::Select { hint, options, .. }) =
            context.world().standard_visual(id)
        else {
            panic!("select visual");
        };
        assert_eq!(hint.as_deref(), Some("FaceAngleX"), "the shown option's");
        assert_eq!(options[0].hint.as_deref(), Some("FaceAngleX"));
        assert_eq!(options[1].hint, None, "an empty hint is none");
        // Said after the label, whether or not the field has room to draw it.
        assert_eq!(
            context.world().accessibility(id).unwrap().value.as_deref(),
            Some("头部左右转 FaceAngleX")
        );
        context
            .update_component(select, |select, _| select.value = Some("none".into()))
            .unwrap();
        assert_eq!(
            context.world().accessibility(id).unwrap().value.as_deref(),
            Some("不绑定")
        );
    }

    #[test]
    fn select_toggle_and_commit_skip_disabled_options() {
        let mut select = sample();
        assert!(select.toggle_open());
        assert!(select.opened);
        assert_eq!(select.highlighted, Some(0));
        assert!(select.highlight_delta(1));
        assert_eq!(select.highlighted, Some(2));
        let changed = select.commit_highlighted().expect("commit preview");
        assert_eq!(changed.value.as_ref(), "preview");
        assert!(!select.opened);
        assert_eq!(select.value.as_deref(), Some("preview"));
        assert!(select.select_index(1).is_none());
    }

    #[test]
    fn inactive_select_does_not_open() {
        let mut select = sample().disabled(true);
        assert!(!select.toggle_open());
        let mut loading = sample().loading(true);
        assert!(!loading.toggle_open());
    }

    #[test]
    fn opened_invalid_select_uses_a_thicker_danger_border() {
        let mut context = AppContext::new();
        let select = context
            .create_component(document(), sample().invalid(true).opened(true))
            .unwrap();
        let style = context.world().node_style(select.stable_id()).unwrap();
        assert_eq!(style.border, Some(SemanticColorRole::Danger));
        assert_eq!(style.layout.border_width, Some(2.0));
        assert!(matches!(
            context.world().standard_visual(select.stable_id()),
            Some(StandardVisual::Select {
                opened: true,
                invalid: true,
                ..
            })
        ));
    }

    #[test]
    fn select_projects_overlay_css_size_into_world_style() {
        let mut select = sample();
        Arc::make_mut(&mut select.style.layout).overlay_css_size_overrides(
            &nana_ui_core::LayoutStyle {
                height: Some(LengthSpec::Px(48.0)),
                width: Some(LengthSpec::Px(120.0)),
                ..nana_ui_core::LayoutStyle::default()
            },
        );
        let mut context = AppContext::new();
        let select = context.create_component(document(), select).unwrap();
        let layout = context
            .world()
            .node_style(select.stable_id())
            .unwrap()
            .layout
            .as_ref();
        assert_eq!(layout.height, Some(LengthSpec::Px(48.0)));
        assert_eq!(layout.width, Some(LengthSpec::Px(120.0)));
        assert_ne!(
            layout.height,
            Some(LengthSpec::Px(
                ControlSize::Medium.height_in(nana_ui_core::UI_METRICS)
            ))
        );
    }

    #[test]
    fn select_projects_without_css_keeps_control_size() {
        let mut context = AppContext::new();
        let select = context.create_component(document(), sample()).unwrap();
        let style = context.world().node_style(select.stable_id()).unwrap();
        assert_eq!(
            style.control_height,
            Some(nana_ui_core::ControlHeight::Exact(ControlSize::Medium))
        );
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(
            context.world().extract_nodes(&[select.stable_id()])[0]
                .source_style
                .layout
                .height,
            Some(LengthSpec::Px(
                ControlSize::Medium.height_in(nana_ui_core::UI_METRICS)
            ))
        );
    }

    /// One frame the way `RuntimeDocument::flush` runs it: styles, the
    /// scheduled text, layout of what moved, then text again at the boxes
    /// that layout gave it.
    fn frame(context: &mut AppContext, shaper: &mut crate::MeasureTextShaper) {
        let viewport = crate::LayoutViewport::new(600.0, 400.0);
        context.compat_world_mut().observe_text_shaper(shaper);
        for _ in 0..8 {
            let work = context.take_system_work();
            let mut seeds = work.layout_frontier_seeds.clone();
            if !work.is_empty() {
                context.resolve_styles(&work.style).unwrap();
                context.shape_text(&work.text, shaper).unwrap();
            }
            seeds.extend(context.take_layout_frontier_seeds(document()));
            if work.is_empty() && seeds.is_empty() {
                return;
            }
            if !seeds.is_empty() {
                context
                    .layout_document_for_viewport(document(), viewport, &seeds)
                    .unwrap();
                let scope = context.take_last_layout_scope();
                context
                    .shape_text_for_layout_scoped(&scope, shaper)
                    .unwrap();
            }
        }
        panic!("the frame did not settle");
    }

    fn sized_options() -> Vec<SelectOption> {
        vec![
            SelectOption::new("a", "A"),
            SelectOption::new("long", "A much longer option"),
            SelectOption::new("b", "B"),
        ]
    }

    fn shrink(mut select: Select) -> Select {
        Arc::make_mut(&mut select.style.layout).width = Some(LengthSpec::Shrink);
        select
    }

    /// `selects` stacked in a 400px column that does not stretch them.
    fn stacked(
        selects: Vec<Select>,
    ) -> (
        AppContext,
        Vec<crate::Entity<Select>>,
        crate::MeasureTextShaper,
    ) {
        use crate::Stack;

        let mut context = AppContext::new();
        let page = context
            .create_component(
                document(),
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(400.0));
                    layout.align_items = nana_ui_core::AlignSpec::Start;
                }),
            )
            .unwrap();
        let ids = selects
            .into_iter()
            .map(|select| {
                let select = context.create_component(document(), select).unwrap();
                context.append_child(page, select).unwrap();
                select
            })
            .collect();
        let mut shaper = crate::MeasureTextShaper;
        frame(&mut context, &mut shaper);
        (context, ids, shaper)
    }

    fn width(context: &AppContext, select: crate::Entity<Select>) -> f32 {
        context
            .world()
            .layout_box(select.stable_id())
            .unwrap()
            .width
    }

    #[test]
    fn a_select_that_fits_its_options_holds_the_widest_and_its_arrow() {
        let (context, selects, _) = stacked(vec![
            shrink(
                Select::new(Some("a"))
                    .options(sized_options())
                    .fit_options(true),
            ),
            // Showing the widest option: the width the fitted one sizes to.
            shrink(Select::new(Some("long")).options(sized_options())),
            shrink(Select::new(Some("a")).options(sized_options())),
            Select::new(Some("a"))
                .options(sized_options())
                .fit_options(true),
        ]);
        let widest = width(&context, selects[1]);
        assert_eq!(width(&context, selects[0]), widest + HANDLE_WIDTH);
        // Without it a content-sized select is as wide as what it shows.
        assert!(width(&context, selects[2]) < widest);
        // An explicit width still decides: the default one fills the row.
        assert_eq!(width(&context, selects[3]), 400.0);
        // The label holds the widest option; the arrow is beside it.
        let widest_label = context
            .world()
            .text_metrics(selects[1].stable_id())
            .unwrap()
            .width;
        let Some(ComponentGeometry::Select { label, handle, .. }) =
            context.world().component_geometry(selects[0].stable_id())
        else {
            panic!("select geometry");
        };
        assert_eq!(label.bounds.width, widest_label);
        assert_eq!(handle.width, HANDLE_WIDTH);
        assert_eq!(handle.x, label.bounds.x + label.bounds.width);
    }

    #[test]
    fn a_hint_follows_the_field_label_and_ends_its_menu_row_quieter() {
        let mut narrow = Select::new(Some("x")).options([
            SelectOption::new("x", "Head").hint("FaceAngleX"),
            SelectOption::new("y", "Tilt"),
        ]);
        Arc::make_mut(&mut narrow.style.layout).width = Some(LengthSpec::Px(72.0));
        let (context, selects, _) = stacked(vec![
            Select::new(Some("x"))
                .options([
                    SelectOption::new("x", "Head").hint("FaceAngleX"),
                    SelectOption::new("y", "Tilt"),
                ])
                .opened(true),
            narrow,
        ]);
        let muted = Some(nana_ui_core::SemanticPalette::dark().muted.as_rgba_array());
        let Some(ComponentGeometry::Select {
            label, hint, menu, ..
        }) = context.world().component_geometry(selects[0].stable_id())
        else {
            panic!("select geometry");
        };
        let hint = hint
            .as_ref()
            .expect("the field draws the shown option's hint");
        assert!(
            label.bounds.x + label.bounds.width < hint.bounds.x,
            "after its label"
        );
        assert!(hint.font_size < label.font_size, "a step smaller");
        assert_eq!(hint.color, muted);
        let menu = menu.as_ref().expect("an open select draws its menu");
        let row = &menu.options[0];
        let row_hint = row.hint.as_ref().expect("a row with room draws its hint");
        assert!(row.label.bounds.x + row.label.bounds.width < row_hint.bounds.x);
        assert!(row_hint.bounds.x + row_hint.bounds.width <= row.bounds.x + row.bounds.width);
        assert_eq!(row_hint.color, muted);
        assert_eq!(
            &*row.label.content, "Head",
            "the label no longer carries it"
        );
        assert!(menu.options[1].hint.is_none());
        // A field with no room left for the hint leaves it out.
        let Some(ComponentGeometry::Select { hint, .. }) =
            context.world().component_geometry(selects[1].stable_id())
        else {
            panic!("select geometry");
        };
        assert!(hint.is_none());
    }

    #[test]
    fn a_select_that_fits_its_options_follows_them_and_not_its_value() {
        let (mut context, selects, mut shaper) = stacked(vec![shrink(
            Select::new(Some("a"))
                .options(sized_options())
                .fit_options(true),
        )]);
        let select = selects[0];
        let fitted = width(&context, select);
        // One character of the shaper's text is one em: what "A" measures.
        let em = context
            .world()
            .text_metrics(select.stable_id())
            .unwrap()
            .width;
        for value in ["b", "long", "a"] {
            context
                .update_component(select, |select, _| select.value = Some(value.into()))
                .unwrap();
            frame(&mut context, &mut shaper);
            assert_eq!(width(&context, select), fitted, "showing {value}");
        }
        // Six characters longer than the widest option so far.
        let mut longer = sized_options();
        longer.push(SelectOption::new("longer", "An even much longer option"));
        context
            .update_component(select, |select, _| select.options = longer)
            .unwrap();
        frame(&mut context, &mut shaper);
        assert_eq!(width(&context, select), fitted + 6.0 * em);
        context
            .update_component(select, |select, _| select.options = sized_options())
            .unwrap();
        frame(&mut context, &mut shaper);
        assert_eq!(width(&context, select), fitted);
        // Off again: as wide as "A", the arrow no longer reserved.
        context
            .update_component(select, |select, _| select.fit_options = false)
            .unwrap();
        frame(&mut context, &mut shaper);
        let plain = width(&context, select);
        assert_eq!(plain, fitted - 19.0 * em - HANDLE_WIDTH);
        context
            .update_component(select, |select, _| select.fit_options = true)
            .unwrap();
        frame(&mut context, &mut shaper);
        assert_eq!(width(&context, select), fitted);
        // Showing the widest option, only the arrow is the difference.
        context
            .update_component(select, |select, _| {
                select.value = Some("long".into());
                select.fit_options = false;
            })
            .unwrap();
        frame(&mut context, &mut shaper);
        assert_eq!(width(&context, select), fitted - HANDLE_WIDTH);
        context
            .update_component(select, |select, _| select.fit_options = true)
            .unwrap();
        frame(&mut context, &mut shaper);
        assert_eq!(width(&context, select), fitted);
    }

    #[test]
    fn placeholder_is_used_when_no_value_matches() {
        let select = Select::new(None::<Arc<str>>)
            .placeholder("Choose view")
            .options([SelectOption::new("code", "Code")]);
        let (label, placeholder) = select.display_label();
        assert_eq!(label.as_ref(), "Choose view");
        assert!(placeholder);
    }

    /// `count` options, `v{n}` labelled `Option {n}`.
    /// `select` 128px wide at the right end of a `width` column, which clips
    /// what it holds when `clip`, opened once laid out.
    fn at_the_right_end(
        select: Select,
        width: f32,
        clip: bool,
    ) -> (AppContext, crate::Entity<Select>) {
        use crate::Stack;

        let mut select = select;
        Arc::make_mut(&mut select.style.layout).width = Some(LengthSpec::Px(128.0));
        let mut context = AppContext::new();
        let page = context
            .create_component(
                document(),
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(width));
                    layout.align_items = nana_ui_core::AlignSpec::End;
                    if clip {
                        layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
                        layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;
                    }
                }),
            )
            .unwrap();
        let select = context.create_component(document(), select).unwrap();
        context.append_child(page, select).unwrap();
        frame(&mut context, &mut crate::MeasureTextShaper);
        assert!(context.toggle_select(select).unwrap());
        frame(&mut context, &mut crate::MeasureTextShaper);
        (context, select)
    }

    fn fitted(label: &str) -> Select {
        Select::new(Some("a"))
            .options([
                SelectOption::new("a", "A"),
                SelectOption::new("long", label),
            ])
            .fit_options(true)
    }

    #[test]
    fn a_fitted_menu_holds_its_widest_option_inside_the_window() {
        let label = "A much longer option than the field";
        let (context, select) = at_the_right_end(fitted(label), 600.0, false);
        let field = context.world().layout_box(select.stable_id()).unwrap();
        assert_eq!((field.x, field.width), (472.0, 128.0));
        let menu = open_menu(&context, select);
        assert!(menu.surface.width > field.width, "wider than its field");
        // It would run past the window's right edge from the field's x, so it
        // moves left until it ends there.
        assert_eq!(menu.surface.x + menu.surface.width, 600.0);
        assert!(menu.surface.x < field.x);
        let widest = context
            .world()
            .text_natural_width(select.stable_id())
            .unwrap();
        let row = &menu.options[1];
        assert_eq!(&*row.label.content, label);
        assert!(row.label.bounds.width >= widest, "the label is not cut");
        assert_eq!(row.label.bounds.width, menu.options[0].label.bounds.width);
        // The pointer meets the rows where they are drawn, left of the field.
        let (x, y) = centre(row.bounds);
        assert!(x < field.x);
        assert_eq!(select_option_at(&menu, x, y), Some(1));

        // An option wider than the window: the menu is the window's width.
        let (context, select) = at_the_right_end(fitted(&"Long option ".repeat(100)), 600.0, false);
        let menu = open_menu(&context, select);
        assert_eq!((menu.surface.x, menu.surface.width), (0.0, 600.0));

        // Without `fit_options` the menu is its field's width.
        let mut plain = fitted(label);
        plain.fit_options = false;
        let (context, select) = at_the_right_end(plain, 600.0, false);
        let menu = open_menu(&context, select);
        assert_eq!((menu.surface.x, menu.surface.width), (472.0, 128.0));
    }

    #[test]
    fn a_fitted_menu_stays_inside_the_box_that_clips_its_field() {
        let label = "A much longer option than the field";
        let (context, select) = at_the_right_end(fitted(label), 300.0, true);
        let field = context.world().layout_box(select.stable_id()).unwrap();
        assert_eq!(field.x + field.width, 300.0);
        let menu = open_menu(&context, select);
        assert!(menu.surface.width > field.width);
        assert_eq!(menu.surface.x + menu.surface.width, 300.0);

        let (context, select) = at_the_right_end(fitted(&"Long option ".repeat(100)), 300.0, true);
        let menu = open_menu(&context, select);
        assert_eq!((menu.surface.x, menu.surface.width), (0.0, 300.0));
    }

    #[test]
    fn a_menu_wider_than_its_field_moves_left_only_as_far_as_its_room_needs() {
        let bounds = crate::SelectMenuRoom {
            top: 0.0,
            bottom: 400.0,
            left: 100.0,
            right: 400.0,
        };
        let room = Some(bounds);
        let field = LayoutBox {
            x: 300.0,
            y: 0.0,
            width: 80.0,
            height: 32.0,
        };
        // Its field's own box unless its options ask for more.
        assert_eq!(resolve_menu_horizontal(field, None, room), (300.0, 80.0));
        assert_eq!(
            resolve_menu_horizontal(field, Some(60.0), room),
            (300.0, 80.0)
        );
        // From the field's x while it fits, else ending at the room's right.
        let left = LayoutBox { x: 120.0, ..field };
        assert_eq!(
            resolve_menu_horizontal(left, Some(200.0), room),
            (120.0, 200.0)
        );
        assert_eq!(
            resolve_menu_horizontal(field, Some(250.0), room),
            (150.0, 250.0)
        );
        // No wider than the room, and never narrower than the field.
        assert_eq!(
            resolve_menu_horizontal(field, Some(500.0), room),
            (100.0, 300.0)
        );
        let narrow = Some(crate::SelectMenuRoom {
            left: 320.0,
            right: 380.0,
            ..bounds
        });
        assert_eq!(
            resolve_menu_horizontal(field, Some(500.0), narrow),
            (300.0, 80.0)
        );
        // Before a layout gives it a room it keeps the field's x.
        assert_eq!(
            resolve_menu_horizontal(field, Some(250.0), None),
            (300.0, 250.0)
        );
    }

    /// Every [`SelectToggled`] `select` sends from now on, as `open`.
    fn heard_toggles(
        context: &mut AppContext,
        select: crate::Entity<Select>,
    ) -> Arc<std::sync::Mutex<Vec<bool>>> {
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        context
            .on(select, move |_, event: &SelectToggled, _| {
                sink.lock().unwrap().push(event.open);
            })
            .unwrap();
        heard
    }

    fn key(context: &mut AppContext, key: &'static str) {
        use nana_ui_input::{KeyInput, KeyState};
        let mut input = crate::HeadlessInput::bind(context, document());
        input
            .press(
                context,
                KeyInput::named(key, key, KeyState::Pressed, Default::default()),
                None,
                None,
            )
            .unwrap();
    }

    fn click(context: &mut AppContext, (x, y): (f32, f32)) {
        use nana_ui_input::{InputPayload, PointerInput, PointerPhase};
        let mut input = crate::HeadlessInput::bind(context, document());
        for phase in [PointerPhase::Down, PointerPhase::Up] {
            input
                .route(
                    context,
                    InputPayload::Pointer(PointerInput::mouse(phase, x, y)),
                )
                .unwrap();
        }
    }

    #[test]
    fn the_menu_reports_each_open_and_close_once_whichever_path_closes_it() {
        let options = || {
            [
                SelectOption::new("a", "Alpha"),
                SelectOption::new("b", "Beta"),
                SelectOption::new("c", "Gamma"),
            ]
        };
        let (mut context, selects, mut shaper) = stacked(vec![
            Select::new(Some("a")).options(options()),
            Select::new(Some("a")).options(options()),
        ]);
        let (select, id) = (selects[0], selects[0].stable_id());
        let heard = heard_toggles(&mut context, select);
        let take = || std::mem::take(&mut *heard.lock().unwrap());
        let open = |context: &mut AppContext, shaper: &mut crate::MeasureTextShaper| {
            context.focus_node(document(), id).unwrap();
            assert!(context.toggle_select(select).unwrap());
            frame(context, shaper);
        };
        let opened = |context: &AppContext| context.read(select, |select| select.opened).unwrap();

        // The field opens and closes it; asking again for what it is sends
        // nothing.
        open(&mut context, &mut shaper);
        assert!(!context.set_select_opened(select, true).unwrap());
        assert!(context.toggle_select(select).unwrap());
        assert!(!context.set_select_opened(select, false).unwrap());
        assert_eq!(take(), [true, false]);

        // Choosing an option, then the one the field already shows: both close
        // it and are the press handled.
        for _ in 0..2 {
            open(&mut context, &mut shaper);
            let (x, y) = centre(open_menu(&context, select).options[1].bounds);
            assert!(context.activate_select_at(select, x, y).unwrap());
            assert_eq!(take(), [true, false]);
        }
        // Enter commits the highlighted option, the one shown.
        open(&mut context, &mut shaper);
        assert!(context.commit_focused_select(document()).unwrap());
        assert_eq!(take(), [true, false]);

        // Escape, from the keyboard; once closed it has nothing to close.
        open(&mut context, &mut shaper);
        key(&mut context, "Escape");
        assert!(!opened(&context));
        assert!(!context.dismiss_focused_field_options(document()).unwrap());
        assert_eq!(take(), [true, false]);

        // A press elsewhere in the window.
        open(&mut context, &mut shaper);
        click(&mut context, (580.0, 380.0));
        assert!(!opened(&context));
        assert_eq!(take(), [true, false]);

        // Focus leaving the field, by Tab or by the application.
        open(&mut context, &mut shaper);
        key(&mut context, "Tab");
        assert_eq!(
            context.world().focused(document()),
            Some(selects[1].stable_id())
        );
        assert!(!opened(&context));
        open(&mut context, &mut shaper);
        context
            .focus_node(document(), selects[1].stable_id())
            .unwrap();
        assert!(!opened(&context));
        assert_eq!(take(), [true, false, true, false]);

        // A refresh with the same props keeps it open and sends nothing; one
        // that removes an option, or disables the field, closes it. The
        // application builds its props afresh, closed.
        open(&mut context, &mut shaper);
        let value = context.read(select, |select| select.value.clone()).unwrap();
        let props = Select::new(value).options(options());
        context.set_component(select, props.clone()).unwrap();
        assert!(opened(&context));
        assert_eq!(take(), [true]);
        let mut fewer = props.clone();
        fewer.options.pop();
        context.set_component(select, fewer.clone()).unwrap();
        assert!(!opened(&context));
        assert_eq!(take(), [false]);
        open(&mut context, &mut shaper);
        context
            .set_component(select, fewer.clone().disabled(true))
            .unwrap();
        context.set_component(select, fewer.clone()).unwrap();
        assert_eq!(take(), [true, false], "re-enabled, it stays closed");

        // An application write that disables it hides the menu; enabling it
        // again shows the menu it left open.
        open(&mut context, &mut shaper);
        context
            .update_component(select, |select, _| select.loading = true)
            .unwrap();
        context
            .update_component(select, |select, _| select.loading = false)
            .unwrap();
        assert_eq!(take(), [true, false, true]);
    }

    fn long_options(count: usize) -> Vec<SelectOption> {
        (0..count)
            .map(|n| SelectOption::new(format!("v{n}"), format!("Option {n}")))
            .collect()
    }

    fn open_menu(context: &AppContext, select: crate::Entity<Select>) -> SelectMenuGeometry {
        match context.world().component_geometry(select.stable_id()) {
            Some(ComponentGeometry::Select {
                menu: Some(menu), ..
            }) => menu,
            other => panic!("expected an open select menu, got {other:?}"),
        }
    }

    fn centre(bounds: LayoutBox) -> (f32, f32) {
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }

    /// Whether the menu draws `row` whole, clear of its top and bottom
    /// padding.
    fn shows_whole(menu: &SelectMenuGeometry, row: usize) -> bool {
        let bounds = menu.options[row].bounds;
        bounds.y >= menu.surface.y + MENU_PAD
            && bounds.y + bounds.height <= menu.surface.y + menu.surface.height - MENU_PAD
    }

    #[test]
    fn a_menu_taller_than_its_room_scrolls_its_rows_inside_the_surface() {
        // The field sits at the top of a 400px window; 40 rows need 1366px.
        let (context, selects, _) = stacked(vec![
            Select::new(Some("v30"))
                .options(long_options(40))
                .opened(true),
        ]);
        let menu = open_menu(&context, selects[0]);
        let surface = menu.surface;
        assert_eq!(
            surface.y + surface.height,
            400.0,
            "capped to the room below"
        );
        assert_eq!(
            menu.viewport,
            LayoutBox {
                x: surface.x + MENU_BORDER,
                y: surface.y + MENU_BORDER,
                width: surface.width - MENU_BORDER * 2.0,
                height: surface.height - MENU_BORDER * 2.0,
            }
        );
        let pitch = ControlSize::Medium.height_in(nana_ui_core::UI_METRICS) + MENU_ITEM_GAP;
        assert_eq!(
            menu.max_scroll,
            MENU_PAD * 2.0 + 40.0 * pitch - MENU_ITEM_GAP - surface.height
        );
        // Rows keep their places in the list and move together by the offset.
        for (row, option) in menu.options.iter().enumerate() {
            assert_eq!(
                option.bounds.y,
                surface.y + MENU_PAD + row as f32 * pitch - menu.scroll
            );
        }
        // It opens on the option the field shows, in the middle of the rows.
        assert!(shows_whole(&menu, 30));
        let middle = menu.viewport.y + menu.viewport.height / 2.0;
        assert!((centre(menu.options[30].bounds).1 - middle).abs() < 1.0);
        // A bar stands beside the rows, which end where its lane starts.
        let bar = menu.scrollbar.expect("rows that overflow get a bar");
        assert!(bar.lane.x + bar.lane.width <= menu.viewport.x + menu.viewport.width);
        assert!(bar.lane.y >= menu.viewport.y);
        assert!(bar.lane.y + bar.lane.height <= menu.viewport.y + menu.viewport.height);
        assert!(
            menu.options
                .iter()
                .all(|option| option.bounds.x + option.bounds.width <= bar.lane.x)
        );
        assert_eq!(bar.min_offset, 0.0);
        assert_eq!(bar.max_offset, menu.max_scroll);
        // The pointer meets what is drawn: the shown row, not one scrolled
        // out of sight, not the part of a row the edge cuts off, not the lane.
        let (x, y) = centre(menu.options[30].bounds);
        assert_eq!(select_option_at(&menu, x, y), Some(30));
        let below = menu.options[39].bounds;
        assert!(below.y >= menu.viewport.y + menu.viewport.height);
        assert_eq!(select_option_at(&menu, x, centre(below).1), None);
        let edge = menu.viewport.y + menu.viewport.height;
        let cut = menu
            .options
            .iter()
            .position(|option| {
                option.bounds.y < edge && option.bounds.y + option.bounds.height > edge
            })
            .expect("a row runs past the bottom edge");
        assert_eq!(select_option_at(&menu, x, edge - 1.0), Some(cut));
        assert_eq!(select_option_at(&menu, x, edge + 0.5), None);
        let (lane_x, lane_y) = centre(bar.lane);
        assert_eq!(select_option_at(&menu, lane_x, lane_y), None);
    }

    #[test]
    fn a_menu_that_holds_its_rows_does_not_scroll_or_show_a_bar() {
        let (context, selects, _) = stacked(vec![
            Select::new(Some("v2"))
                .options(long_options(4))
                .opened(true),
        ]);
        let menu = open_menu(&context, selects[0]);
        assert_eq!((menu.scroll, menu.max_scroll), (0.0, 0.0));
        assert!(menu.scrollbar.is_none());
        let first = menu.options[0].bounds;
        assert_eq!(first.y, menu.surface.y + MENU_PAD);
        assert_eq!(first.width, menu.surface.width - MENU_PAD * 2.0);
    }

    #[test]
    fn arrow_keys_scroll_the_highlighted_row_into_view() {
        let (mut context, selects, _) = stacked(vec![
            Select::new(Some("v0"))
                .options(long_options(40))
                .opened(true),
        ]);
        let select = selects[0];
        assert!(context.focus_node(document(), select.stable_id()).unwrap());
        let top = open_menu(&context, select);
        assert_eq!(top.scroll, 0.0);
        // Down past the bottom edge and on round to the first option again.
        for step in 1..=40 {
            assert!(context.adjust_focused_select(document(), 1).unwrap());
            let row = step % 40;
            assert_eq!(
                context.read(select, |select| select.highlighted).unwrap(),
                Some(row)
            );
            let menu = open_menu(&context, select);
            assert!(
                shows_whole(&menu, row),
                "row {row} at {:?}",
                menu.options[row].bounds
            );
            if row > 0 && menu.scroll > 0.0 {
                // Only as far as it has to: the row comes in at the bottom.
                let bounds = menu.options[row].bounds;
                let bottom = menu.surface.y + menu.surface.height - MENU_PAD;
                assert!(
                    (bounds.y + bounds.height - bottom).abs() < 0.01,
                    "row {row} ends at {} not {bottom}",
                    bounds.y + bounds.height
                );
            }
        }
        assert_eq!(open_menu(&context, select).scroll, 0.0);
        // Up from the first wraps to the last, at the end of the list.
        assert!(context.adjust_focused_select(document(), -1).unwrap());
        let menu = open_menu(&context, select);
        assert_eq!(menu.scroll, menu.max_scroll);
        assert!(shows_whole(&menu, 39));
        // Back up a row it already shows does not move the rows.
        assert!(context.adjust_focused_select(document(), -1).unwrap());
        assert_eq!(open_menu(&context, select).scroll, menu.max_scroll);
    }

    #[test]
    fn reopening_or_refreshing_keeps_the_scroll_only_while_the_menu_stays_open() {
        let mut select = Select::new(Some("v0"))
            .options(long_options(40))
            .opened(true);
        select.menu_scroll.offset = Some(120.0);
        // A refresh of unrelated props keeps the interaction's scroll.
        crate::ComponentView::reconcile(
            &mut select,
            Select::new(Some("v0"))
                .options(long_options(40))
                .invalid(true),
        );
        assert_eq!(select.menu_scroll.offset, Some(120.0));
        // Closing forgets it, so the next opening shows the chosen option.
        select.close();
        assert_eq!(select.menu_scroll, SelectMenuScroll::default());
        assert!(select.toggle_open());
        assert_eq!(select.menu_scroll, SelectMenuScroll::default());
    }
}
