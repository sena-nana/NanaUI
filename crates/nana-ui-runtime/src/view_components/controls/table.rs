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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub label: Option<Arc<str>>,
    /// More than one row can be selected: assistive technology hears it
    /// (`aria-multiselectable`). The selection itself is the application's.
    pub multiselectable: bool,
    pub style: NodeStyle,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn multiselectable(mut self, multiselectable: bool) -> Self {
        self.multiselectable = multiselectable;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

static TABLE_CAPABILITIES: crate::framework::TableCapabilities =
    crate::framework::TableCapabilities {
        navigate: AppContext::navigate_focused_table,
        selection_key: AppContext::table_selection_key,
    };

static TABLE_CELL_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    column_edge_at: Some(AppContext::table_column_edge_at),
    ..crate::framework::PointerOps::EMPTY
};

impl ComponentView for Table {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TABLE.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.data_table = Some(&TABLE_CAPABILITIES);
            caps.add_edit_canceller(crate::AppContext::cancel_table_column_resize);
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
            tag: "table".into(),
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
                role: AccessibilityRole::Table,
                label: self.label.clone(),
                multiselectable: self.multiselectable,
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableRow {
    pub selected: bool,
    pub style: NodeStyle,
}

impl TableRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableRow {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TABLE_ROW.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "tr".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &selected_table_style(&self.style, self.selected),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Row,
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}

/// A selected row or cell paints the theme's selection behind its content,
/// over whatever background it set.
fn selected_table_style(style: &NodeStyle, selected: bool) -> std::borrow::Cow<'_, NodeStyle> {
    if selected && style.background != Some(nana_ui_core::SemanticColorRole::Selected) {
        let mut style = style.clone();
        style.background = Some(nana_ui_core::SemanticColorRole::Selected);
        std::borrow::Cow::Owned(style)
    } else {
        std::borrow::Cow::Borrowed(style)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub value: String,
    pub column_header: bool,
    pub selected: bool,
    /// The key of the column the cell is in. A resizable header reports
    /// drags under it ([`crate::TableColumnResized`]).
    pub column: Option<Arc<str>>,
    /// On a column header with a [`Self::column`]: dragging its trailing
    /// edge resizes the column.
    pub resizable: bool,
    /// What a header drag clamps the column's width to, px.
    pub extent_limits: (f32, f32),
    pub style: NodeStyle,
    pub(crate) resize: Option<crate::data_table::ColumnResizeDrag>,
}

impl TableCell {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            column_header: false,
            selected: false,
            column: None,
            resizable: false,
            extent_limits: (0.0, f32::INFINITY),
            style: NodeStyle {
                control_padding_x: Some(nana_ui_core::ControlPadding::ListItem),
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            },
            resize: None,
        }
    }

    pub fn column_header(mut self, column_header: bool) -> Self {
        self.column_header = column_header;
        self
    }

    /// The key of the column this cell is in.
    pub fn column(mut self, column: impl Into<Arc<str>>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// On a column header: dragging the trailing edge resizes the column.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// The header of `column`: its key, whether it resizes and the width
    /// limits a drag clamps to.
    pub fn header_for(self, column: &nana_ui_core::TableColumn) -> Self {
        let mut cell = self
            .column_header(true)
            .column(column.key.as_str())
            .resizable(column.resizable);
        cell.extent_limits = (column.min_extent, column.max_extent);
        cell
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for TableCell {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::TABLE_CELL.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.add_edit_canceller(crate::AppContext::cancel_table_column_resize);
        }),
        pointer: Some(&crate::data_table::TABLE_CELL_POINTER),
        pointer_ops: Some(&TABLE_CELL_POINTER_OPS),
        ..crate::TypeBehavior::NONE
    };

    fn reconcile(&mut self, mut next: Self) {
        // a header drag in progress is the cell's own
        next.resize = self.resize.take();
        *self = next;
    }

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: if self.column_header { "th" } else { "td" }.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.value.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.value.clone().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &selected_table_style(&self.style, self.selected),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: if self.column_header {
                    AccessibilityRole::ColumnHeader
                } else {
                    AccessibilityRole::Cell
                },
                label: Some(Arc::from(self.value.as_str())),
                selected: Some(self.selected),
                ..AccessibilityState::default()
            },
        );
    }
}
