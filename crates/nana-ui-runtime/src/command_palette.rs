//! Modal command list. Search uses committed TextInput state.

use std::sync::Arc;

use nana_ui_core::{
    ActionId, ActionPickerNavigation, CommandPaletteEvent, CommandPaletteItem, ControlSize,
    DialogSize,
};

use crate::overlay_surfaces::{MODAL_PAD_X, modal_root_style, modal_surface_bounds};
use crate::query::query_matches;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentElevation, ComponentTextRegion, ComponentView,
    ComputedStyle, InteractionState, LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    TextHorizontalAlignment, TextInputState, UiWorld, WidgetPaintCtx, WidgetPaintOp,
    WidgetPaintOutput, WidgetQuad, WidgetTaggedOp, WidgetVisual,
};

const MAX_VISIBLE_ROWS: usize = 12;
const INPUT_GAP: f32 = nana_ui_core::space::MD;
const ROW_PAD_X: f32 = nana_ui_core::space::LG;
const SHORTCUT_LABEL_GAP: f32 = nana_ui_core::space::MD;

fn palette_row_height(metrics: nana_ui_core::ThemeMetrics) -> f32 {
    ControlSize::Large.height_in(metrics) + nana_ui_core::space::XS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRowData {
    pub action: ActionId,
    pub label: Arc<str>,
    pub category: Option<Arc<str>>,
    pub shortcut: Option<Arc<str>>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaletteRowGeometry {
    pub action: ActionId,
    pub bounds: LayoutBox,
    pub label: ComponentTextRegion,
    pub category: Option<ComponentTextRegion>,
    pub shortcut: Option<ComponentTextRegion>,
    pub selected: bool,
    pub background: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandPalette {
    pub title: Arc<str>,
    /// `None` says the framework's (`command_palette.placeholder`).
    pub placeholder: Option<Arc<str>>,
    /// `None` says the framework's (`command_palette.empty`).
    pub empty_label: Option<Arc<str>>,
    pub items: Vec<CommandPaletteItem>,
    /// Items have already been filtered and ranked by the host.
    pub filtered_items: bool,
    pub selected: usize,
    pub state: TextInputState,
    pub style: NodeStyle,
}

impl CommandPalette {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(
        title: impl Into<Arc<str>>,
        items: impl IntoIterator<Item = CommandPaletteItem>,
    ) -> Self {
        let items = items.into_iter().collect::<Vec<_>>();
        Self {
            title: title.into(),
            placeholder: None,
            empty_label: None,
            items,
            filtered_items: false,
            selected: 0,
            state: TextInputState::new(""),
            style: modal_root_style(),
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn empty_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.empty_label = Some(label.into());
        self
    }

    pub fn query(mut self, query: impl Into<String>) -> Self {
        let query = query.into();
        self.state = TextInputState::new(query);
        self.selected = 0;
        self
    }

    /// The current search text, owned by the retained text input state.
    pub fn query_text(&self) -> &str {
        self.state.value.as_str()
    }

    /// Preserve host-filtered results and their ranking without applying the
    /// palette's substring filter again. Query editing still emits Search.
    pub fn filtered_items(mut self, filtered: bool) -> Self {
        self.filtered_items = filtered;
        self
    }

    pub fn visible_items(&self) -> Vec<&CommandPaletteItem> {
        self.items
            .iter()
            .filter(|item| self.filtered_items || item_matches(item, self.query_text()))
            .collect()
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> CommandPaletteEvent {
        let query = query.into();
        self.state.replace_value(query.clone());
        self.selected = 0;
        CommandPaletteEvent::Search(query)
    }

    pub fn navigate(&mut self, navigation: ActionPickerNavigation) -> Option<CommandPaletteEvent> {
        let visible = self.visible_items().len();
        match navigation {
            ActionPickerNavigation::Dismiss => Some(CommandPaletteEvent::Dismiss),
            ActionPickerNavigation::Confirm => self.confirm(),
            ActionPickerNavigation::Previous if visible > 0 => {
                self.selected = self.selected.saturating_sub(1);
                Some(CommandPaletteEvent::Navigate(navigation))
            }
            ActionPickerNavigation::Next if visible > 0 => {
                self.selected = (self.selected + 1).min(visible.saturating_sub(1));
                Some(CommandPaletteEvent::Navigate(navigation))
            }
            ActionPickerNavigation::First if visible > 0 => {
                self.selected = 0;
                Some(CommandPaletteEvent::Navigate(navigation))
            }
            ActionPickerNavigation::Last if visible > 0 => {
                self.selected = visible.saturating_sub(1);
                Some(CommandPaletteEvent::Navigate(navigation))
            }
            _ => None,
        }
    }

    pub fn confirm(&self) -> Option<CommandPaletteEvent> {
        self.visible_items()
            .get(self.selected)
            .map(|item| CommandPaletteEvent::Select(item.action.clone()))
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        if !self.state.replace_selection(text) {
            return false;
        }
        self.selected = 0;
        true
    }

    /// Delete IME surrounding text; `composing` keeps the selection a preedit
    /// stands in for (see [`crate::TextInputState::delete_ime_surrounding`]).
    pub fn delete_surrounding(
        &mut self,
        before_bytes: usize,
        after_bytes: usize,
        composing: bool,
    ) -> bool {
        if !self
            .state
            .delete_ime_surrounding(before_bytes, after_bytes, composing)
        {
            return false;
        }
        self.selected = 0;
        true
    }

    fn windowed_rows(&self) -> Vec<PaletteRowData> {
        let visible = self.visible_items();
        if visible.is_empty() {
            return Vec::new();
        }
        let len = visible.len();
        let start = if len <= MAX_VISIBLE_ROWS {
            0
        } else {
            self.selected
                .saturating_sub(MAX_VISIBLE_ROWS / 2)
                .min(len - MAX_VISIBLE_ROWS)
        };
        visible
            .into_iter()
            .enumerate()
            .skip(start)
            .take(MAX_VISIBLE_ROWS)
            .map(|(index, item)| PaletteRowData {
                action: item.action.clone(),
                label: Arc::from(item.label.as_str()),
                category: item.category.as_deref().map(Arc::from),
                shortcut: item.shortcut.as_deref().map(Arc::from),
                selected: index == self.selected,
            })
            .collect()
    }
}

impl ComponentView for CommandPalette {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::COMMAND_PALETTE.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        activate_at: Some(crate::AppContext::activate_command_palette_at),
        capabilities: Some(|caps| {
            caps.command_palette_navigate =
                Some(crate::AppContext::navigate_focused_command_palette);
        }),
        editable: Some(&crate::framework::Editable::<Self>::HOOKS),
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
            tag: "command-palette".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let rows = self.windowed_rows();
        let strings = world.framework_strings();
        let empty = rows.is_empty().then(|| {
            self.empty_label
                .clone()
                .unwrap_or_else(|| Arc::clone(&strings.command_palette_empty))
        });
        let visual = CommandPaletteVisual {
            title: Arc::clone(&self.title),
            query: Arc::from(self.query_text()),
            placeholder: self
                .placeholder
                .clone()
                .unwrap_or_else(|| Arc::clone(&strings.command_palette_placeholder)),
            empty,
            rows: rows.into(),
        };
        if world.widget_visual_as::<CommandPaletteVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        if world
            .text_input(id)
            .is_none_or(|current| current != self.state)
        {
            mutations.set_text_input(id, Some(self.state.clone()));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                label: Some(Arc::clone(&self.title)),
                value: (!self.query_text().is_empty()).then(|| Arc::from(self.query_text())),
                modal: true,
                editable: true,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Resolved layout and surface colors for a mounted [`CommandPalette`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommandPaletteLayout {
    pub scrim: LayoutBox,
    pub surface: LayoutBox,
    pub title: ComponentTextRegion,
    /// The search field's frame: its background, border and hit area.
    pub input_box: LayoutBox,
    /// The query (or placeholder) text, inset inside `input_box` by the
    /// field's horizontal control padding.
    pub input: ComponentTextRegion,
    pub empty: Option<ComponentTextRegion>,
    pub rows: Vec<PaletteRowGeometry>,
    pub background: [f32; 4],
    pub input_background: [f32; 4],
    pub input_border: [f32; 4],
    pub elevation: ComponentElevation,
}

impl CommandPaletteLayout {
    /// Lowers this resolved layout into node-local widget paint operations.
    #[must_use]
    pub fn to_widget_paint(
        &self,
        chrome_radii: nana_ui_core::ChromeRadii,
        style: &ComputedStyle,
    ) -> WidgetPaintOutput {
        let overlay = |slot: u64, op: WidgetPaintOp| {
            WidgetTaggedOp::new(slot, op)
                .with_min_z_index(1_000)
                .with_parent_clips()
        };
        let mut out = WidgetPaintOutput {
            owns_node_text: true,
            ..WidgetPaintOutput::new()
        };
        out.push_tagged(overlay(
            10,
            WidgetPaintOp::Quad(WidgetQuad {
                rect: self.scrim,
                fill: Some([0.0, 0.0, 0.0, 0.45]),
                use_parent_clips: true,
                ..WidgetQuad::default()
            }),
        ));
        out.push_tagged(overlay(
            11,
            WidgetPaintOp::Quad(WidgetQuad {
                rect: self.surface,
                fill: Some(self.background),
                fill_paint: crate::widget_visual::matching_paint_color(
                    style.paint_colors.background,
                    self.background,
                ),
                corner_radius: [chrome_radii.md; 4],
                elevation: Some(self.elevation),
                use_parent_clips: true,
                ..WidgetQuad::default()
            }),
        ));
        out.push_tagged(overlay(
            20,
            WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                &self.title,
                style,
                TextHorizontalAlignment::Start,
                true,
            )),
        ));
        out.push_tagged(overlay(
            12,
            WidgetPaintOp::Quad(WidgetQuad {
                rect: self.input_box,
                fill: Some(self.input_background),
                fill_paint: crate::widget_visual::matching_paint_color(
                    style.paint_colors.background,
                    self.input_background,
                ),
                border: Some((self.input_border, 1.0)),
                border_paint: crate::widget_visual::matching_paint_color(
                    style.paint_colors.border,
                    self.input_border,
                ),
                corner_radius: [chrome_radii.field; 4],
                use_parent_clips: true,
                ..WidgetQuad::default()
            }),
        ));
        out.push_tagged(overlay(
            21,
            WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                &self.input,
                style,
                TextHorizontalAlignment::Start,
                true,
            )),
        ));
        if let Some(empty) = &self.empty {
            out.push_tagged(overlay(
                22,
                WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    empty,
                    style,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            ));
        }
        for (index, row) in self.rows.iter().enumerate() {
            let index = u64::try_from(index).unwrap_or(u64::MAX);
            if let Some(background) = row.background {
                out.push_tagged(overlay(
                    23u64.saturating_add(index),
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect: row.bounds,
                        fill: Some(background),
                        fill_paint: crate::widget_visual::matching_paint_color(
                            style.paint_colors.background,
                            background,
                        ),
                        corner_radius: [chrome_radii.menu_row; 4],
                        use_parent_clips: true,
                        ..WidgetQuad::default()
                    }),
                ));
            }
            out.push_tagged(overlay(
                40u64.saturating_add(index),
                WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    &row.label,
                    style,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            ));
            if let Some(category) = &row.category {
                out.push_tagged(overlay(
                    70u64.saturating_add(index),
                    WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                        category,
                        style,
                        TextHorizontalAlignment::Start,
                        true,
                    )),
                ));
            }
            if let Some(shortcut) = &row.shortcut {
                out.push_tagged(overlay(
                    100u64.saturating_add(index),
                    WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                        shortcut,
                        style,
                        TextHorizontalAlignment::End,
                        true,
                    )),
                ));
            }
        }
        out
    }
}

/// Widget visual projected by [`CommandPalette`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteVisual {
    pub title: Arc<str>,
    pub query: Arc<str>,
    pub placeholder: Arc<str>,
    pub empty: Option<Arc<str>>,
    pub rows: Arc<[PaletteRowData]>,
}

impl CommandPaletteVisual {
    #[allow(clippy::too_many_arguments)]
    fn layout_at(
        &self,
        bounds: LayoutBox,
        palette: &nana_ui_core::SemanticPalette,
        metrics: nana_ui_core::ThemeMetrics,
        typography: nana_ui_core::TypographyTokens,
        measure: crate::text_width::ChromeTextMeasure<'_>,
    ) -> CommandPaletteLayout {
        command_palette_geometry(
            bounds,
            &self.title,
            &self.query,
            &self.placeholder,
            self.empty.as_ref(),
            &self.rows,
            palette,
            metrics,
            typography,
            measure,
        )
    }
}

impl WidgetVisual for CommandPaletteVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let layout = self.layout_at(
            ctx.bounds,
            &ctx.style_model.palette,
            ctx.style_model.metrics,
            ctx.theme.typography(),
            ctx.world.chrome_text_measure(ctx.id),
        );
        layout.to_widget_paint(ctx.chrome_radii, ctx.style)
    }
}

impl UiWorld {
    /// Resolves `id`'s modal surface, search field, and row layout when mounted as a [`CommandPalette`].
    #[must_use]
    pub fn command_palette_layout(&self, id: StableNodeId) -> Option<CommandPaletteLayout> {
        let visual = self.widget_visual_as::<CommandPaletteVisual>(id)?;
        let bounds = self.component_layout_box(id)?;
        let style_model = self.node_style_model(id);
        let typography = self.node_theme(id).typography();
        Some(visual.layout_at(
            bounds,
            &style_model.palette,
            style_model.metrics,
            typography,
            self.chrome_text_measure(id),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_palette_geometry(
    bounds: LayoutBox,
    title: &Arc<str>,
    query: &Arc<str>,
    placeholder: &Arc<str>,
    empty: Option<&Arc<str>>,
    rows: &[PaletteRowData],
    palette: &nana_ui_core::SemanticPalette,
    metrics: nana_ui_core::ThemeMetrics,
    typography: nana_ui_core::TypographyTokens,
    measure: crate::text_width::ChromeTextMeasure<'_>,
) -> CommandPaletteLayout {
    let input_height = ControlSize::Medium.height_in(metrics);
    let row_height = palette_row_height(metrics);
    let list_height = if rows.is_empty() {
        row_height
    } else {
        rows.len() as f32 * row_height
    };
    let intrinsic = nana_ui_core::space::PAGE * 2.0
        + input_height
        + INPUT_GAP
        + list_height
        + nana_ui_core::space::XXXL;
    // The palette borrows the dialog card's placement, not a theme's: a
    // theme that moves its dialogs does not move the palette with them.
    let surface = modal_surface_bounds(
        bounds,
        crate::ModalSurfaceKind::Dialog(DialogSize::Wide),
        Some(intrinsic),
        &nana_ui_core::DialogRecipe::DEFAULT,
    );
    let mut y = surface.y + nana_ui_core::space::XXXL;
    let title_region = ComponentTextRegion {
        bounds: LayoutBox {
            x: surface.x + MODAL_PAD_X,
            y,
            width: (surface.width - MODAL_PAD_X * 2.0).max(0.0),
            height: nana_ui_core::space::PAGE_TIGHT + nana_ui_core::space::XXS,
        },
        content: Arc::clone(title).into(),
        color: Some(palette.text.as_rgba_array()),
        font_size: typography.heading,
        font_weight: Some(typography.semibold),
    };
    y += ControlSize::Small.height_in(metrics);
    let input_box = LayoutBox {
        x: surface.x + MODAL_PAD_X,
        y,
        width: (surface.width - MODAL_PAD_X * 2.0).max(0.0),
        height: input_height,
    };
    // The text sits inside the field as a TextInput's does, not on its
    // border: inset by the same control padding.
    let input_pad = ControlSize::Medium.padding_x_in(metrics);
    let input = ComponentTextRegion {
        bounds: LayoutBox {
            x: input_box.x + input_pad,
            width: (input_box.width - input_pad * 2.0).max(0.0),
            ..input_box
        },
        content: if query.is_empty() {
            Arc::clone(placeholder).into()
        } else {
            Arc::clone(query).into()
        },
        color: Some(if query.is_empty() {
            palette.faint.as_rgba_array()
        } else {
            palette.text.as_rgba_array()
        }),
        font_size: ControlSize::Medium.text_size(),
        font_weight: None,
    };
    y += input_height + INPUT_GAP;
    let empty_label = empty.map(|label| ComponentTextRegion {
        bounds: LayoutBox {
            x: surface.x + MODAL_PAD_X,
            y,
            width: (surface.width - MODAL_PAD_X * 2.0).max(0.0),
            height: row_height,
        },
        content: Arc::clone(label).into(),
        color: Some(palette.muted.as_rgba_array()),
        font_size: typography.meta,
        font_weight: None,
    });
    let rows = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let bounds = LayoutBox {
                x: surface.x + nana_ui_core::space::MD,
                y: y + index as f32 * row_height,
                width: (surface.width - nana_ui_core::space::XXXL).max(0.0),
                height: row_height,
            };
            let shortcut = row.shortcut.as_ref().map(|shortcut| {
                let shortcut_width = measure.width(shortcut, typography.hint, None);
                ComponentTextRegion {
                    bounds: LayoutBox {
                        x: bounds.x + bounds.width - ROW_PAD_X - shortcut_width,
                        y: bounds.y + nana_ui_core::space::XL,
                        width: shortcut_width,
                        height: typography.line,
                    },
                    content: Arc::clone(shortcut).into(),
                    color: Some(palette.muted.as_rgba_array()),
                    font_size: typography.hint,
                    font_weight: None,
                }
            });
            // The label yields to the (End-aligned) shortcut instead of
            // running underneath it.
            let label_right = match &shortcut {
                Some(shortcut) => shortcut.bounds.x - SHORTCUT_LABEL_GAP,
                None => bounds.x + bounds.width - ROW_PAD_X,
            };
            let label = ComponentTextRegion {
                bounds: LayoutBox {
                    x: bounds.x + ROW_PAD_X,
                    y: bounds.y + nana_ui_core::space::SM,
                    width: (label_right - bounds.x - ROW_PAD_X).max(0.0),
                    height: typography.line,
                },
                content: Arc::clone(&row.label).into(),
                color: Some(palette.text.as_rgba_array()),
                font_size: typography.meta,
                font_weight: Some(typography.medium),
            };
            let category = row.category.as_ref().map(|category| ComponentTextRegion {
                bounds: LayoutBox {
                    x: bounds.x + ROW_PAD_X,
                    y: bounds.y + nana_ui_core::space::PAGE_TIGHT + nana_ui_core::space::XXS,
                    width: (bounds.width - ROW_PAD_X * 2.0).max(0.0),
                    height: typography.meta,
                },
                content: Arc::clone(category).into(),
                color: Some(palette.muted.as_rgba_array()),
                font_size: typography.hint,
                font_weight: None,
            });
            PaletteRowGeometry {
                action: row.action.clone(),
                bounds,
                label,
                category,
                shortcut,
                selected: row.selected,
                background: row.selected.then_some(palette.selected.as_rgba_array()),
            }
        })
        .collect();
    CommandPaletteLayout {
        scrim: bounds,
        surface,
        title: title_region,
        input_box,
        input,
        empty: empty_label,
        rows,
        background: palette.surface.as_rgba_array(),
        input_background: palette.background.as_rgba_array(),
        input_border: palette.border.as_rgba_array(),
        elevation: ComponentElevation {
            color: nana_ui_core::PaintColor::srgb([0.0, 0.0, 0.0, 0.4]),
            offset_x: 0.0,
            offset_y: nana_ui_core::space::XL,
            blur_radius: nana_ui_core::space::PAGE,
            spread_radius: 0.0,
            inset: false,
        },
    }
}

pub(crate) fn palette_row_at(rows: &[PaletteRowGeometry], x: f32, y: f32) -> Option<usize> {
    rows.iter().position(|row| row.bounds.contains(x, y))
}

pub(crate) fn activate_command_palette_at(
    palette: &mut CommandPalette,
    surface: LayoutBox,
    input: LayoutBox,
    rows: &[PaletteRowGeometry],
    x: f32,
    y: f32,
) -> Option<CommandPaletteEvent> {
    if let Some(index) = palette_row_at(rows, x, y) {
        palette.selected = palette
            .visible_items()
            .iter()
            .position(|item| item.action == rows[index].action)
            .unwrap_or(palette.selected);
        return Some(CommandPaletteEvent::Select(rows[index].action.clone()));
    }
    if input.contains(x, y) || surface.contains(x, y) {
        return None;
    }
    Some(CommandPaletteEvent::Dismiss)
}

fn item_matches(item: &CommandPaletteItem, query: &str) -> bool {
    query_matches(&item.label, query)
        || query_matches(item.action.as_str(), query)
        || item
            .category
            .as_ref()
            .is_some_and(|category| query_matches(category, query))
}

/// The router reaches this picker's keyboard only once one exists.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> CommandPalette {
        CommandPalette::new(
            "命令面板",
            [
                CommandPaletteItem::new("workspace.files", "打开文件").category("工作区"),
                CommandPaletteItem::new("workspace.settings", "设置").shortcut("Ctrl+,"),
                CommandPaletteItem::new("edit.rename", "重命名"),
            ],
        )
    }

    #[test]
    fn query_filters_and_confirm_selects_the_highlighted_action() {
        let mut palette = sample();
        palette.set_query("设");
        assert_eq!(palette.visible_items().len(), 1);
        assert_eq!(
            palette.confirm(),
            Some(CommandPaletteEvent::Select(ActionId::new(
                "workspace.settings"
            )))
        );
    }

    #[test]
    fn host_filtered_results_keep_ranking_for_navigation_and_selection() {
        let mut palette = sample().filtered_items(true);
        assert_eq!(
            palette.set_query("wfs"),
            CommandPaletteEvent::Search("wfs".into())
        );
        assert_eq!(palette.visible_items().len(), 3);
        assert_eq!(
            palette.confirm(),
            Some(CommandPaletteEvent::Select(ActionId::new(
                "workspace.files"
            )))
        );
        palette.navigate(ActionPickerNavigation::Next);
        assert_eq!(
            palette.confirm(),
            Some(CommandPaletteEvent::Select(ActionId::new(
                "workspace.settings"
            )))
        );
        palette.items.clear();
        assert_eq!(palette.confirm(), None);
    }

    #[test]
    fn palette_projects_modal_search_and_rows() {
        let mut context = AppContext::new();
        let palette = context.create_component(document(), sample()).unwrap();
        let id = palette.stable_id();
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<CommandPaletteVisual>(id),
            Some(CommandPaletteVisual { rows, .. }) if rows.len() == 3
        ));
        assert!(context.world().text_input(id).is_some());
    }

    #[test]
    fn palette_commits_ime_into_the_query() {
        let mut context = AppContext::new();
        let palette = context.create_component(document(), sample()).unwrap();
        context.focus_node(document(), palette.stable_id()).unwrap();
        assert!(context.commit_ime(document(), "设").unwrap());
        context
            .read(palette, |palette| {
                assert_eq!(palette.query_text(), "设");
                assert_eq!(palette.visible_items().len(), 1);
            })
            .unwrap();
    }

    #[test]
    fn text_input_state_is_the_query_authority() {
        let mut palette = sample().query("files");
        palette.state.replace_value("设");

        assert_eq!(palette.query_text(), "设");
        assert_eq!(palette.visible_items().len(), 1);
    }

    #[test]
    fn long_shortcut_reserves_its_estimate_and_stays_clear_of_the_label() {
        let rows = [PaletteRowData {
            action: ActionId::new("workspace.settings"),
            label: Arc::from("打开设置"),
            category: None,
            shortcut: Some(Arc::from("Ctrl+Alt+Delete")),
            selected: false,
        }];
        let geometry = command_palette_geometry(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 720.0,
                height: 480.0,
            },
            &Arc::from("命令面板"),
            &Arc::from(""),
            &Arc::from("搜索命令"),
            None,
            &rows,
            &nana_ui_core::SemanticPalette::dark(),
            nana_ui_core::UI_METRICS,
            nana_ui_core::TypographyTokens::DEFAULT,
            crate::text_width::ChromeTextMeasure::ESTIMATE,
        );
        let CommandPaletteLayout { rows, .. } = geometry;
        let row = &rows[0];
        let shortcut = row.shortcut.as_ref().expect("shortcut region");
        // "Ctrl+Alt+Delete" at 10px ≈ 93px, far past the old fixed 70px box.
        let estimated = crate::text_width::estimated_text_width(
            shortcut.content.as_ref(),
            nana_ui_core::TypographyTokens::DEFAULT.hint,
        );
        assert!(
            estimated > 80.0,
            "estimate {estimated} should exceed the old box"
        );
        assert!(shortcut.bounds.width + 0.01 >= estimated);
        assert!((shortcut.bounds.width - estimated).abs() < 0.5);
        // End edge stays pinned to the row's trailing padding edge.
        let shortcut_end = shortcut.bounds.x + shortcut.bounds.width;
        assert!((shortcut_end - (row.bounds.x + row.bounds.width - ROW_PAD_X)).abs() < 0.01);
        // The label region ends before the shortcut region begins.
        assert!(row.label.bounds.width > 0.0);
        assert!(row.label.bounds.x + row.label.bounds.width <= shortcut.bounds.x);
    }

    #[test]
    fn query_text_is_inset_inside_the_search_field() {
        let geometry = command_palette_geometry(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 720.0,
                height: 480.0,
            },
            &Arc::from("命令"),
            &Arc::from("工作区"),
            &Arc::from("搜索命令"),
            None,
            &[],
            &nana_ui_core::SemanticPalette::dark(),
            nana_ui_core::UI_METRICS,
            nana_ui_core::TypographyTokens::DEFAULT,
            crate::text_width::ChromeTextMeasure::ESTIMATE,
        );
        let CommandPaletteLayout {
            input_box, input, ..
        } = geometry;
        let pad = ControlSize::Medium.padding_x_in(nana_ui_core::UI_METRICS);
        assert!(pad > 0.0);
        assert_eq!(input.bounds.x, input_box.x + pad);
        assert_eq!(
            input.bounds.x + input.bounds.width,
            input_box.x + input_box.width - pad
        );
        assert_eq!(input.bounds.y, input_box.y);
        assert_eq!(input.bounds.height, input_box.height);
    }
}
