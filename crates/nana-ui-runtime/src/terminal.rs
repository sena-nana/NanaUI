//! Retained terminal grid. Hosts supply parsed cells and own the PTY.

use std::sync::Arc;

use nana_ui_core::{
    LengthSpec, LineHeightSpec, OverflowSpec, PositionSpec, SemanticColorRole, TextDecorationLine,
};

use crate::component_registry::{RegisterableComponent, SemanticSpec};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    Text, UiWorld,
};

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalCell {
    pub text: Arc<str>,
    /// Zero is the continuation of a wide cell; otherwise one or two columns.
    pub width: u8,
    pub foreground: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub bold: bool,
    pub underline: bool,
    pub dim: bool,
    pub italic: bool,
    pub inverse: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            text: Arc::from(" "),
            width: 1,
            foreground: None,
            background: None,
            bold: false,
            underline: false,
            dim: false,
            italic: false,
            inverse: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalPosition {
    pub row: u16,
    pub column: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalCursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCursor {
    pub position: TerminalPosition,
    pub shape: TerminalCursorShape,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalScreen {
    pub columns: u16,
    pub rows: u16,
    pub cells: Arc<[TerminalCell]>,
    pub cursor: Option<TerminalCursor>,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

pub const MAX_TERMINAL_CELLS: usize = 2_000_000;

impl TerminalScreen {
    pub fn blank(columns: u16, rows: u16) -> Self {
        Self {
            columns,
            rows,
            cells: if usize::from(columns) * usize::from(rows) <= MAX_TERMINAL_CELLS {
                vec![TerminalCell::default(); usize::from(columns) * usize::from(rows)].into()
            } else {
                Arc::from([])
            },
            cursor: None,
            application_cursor: false,
            bracketed_paste: false,
        }
    }

    fn valid(&self) -> bool {
        self.columns > 0
            && self.rows > 0
            && self.cells.len() <= MAX_TERMINAL_CELLS
            && self.cells.len() == usize::from(self.columns) * usize::from(self.rows)
            && self
                .cells
                .iter()
                .all(|cell| cell.width <= 2 && cell.text.len() <= 256)
    }

    fn index(&self, position: TerminalPosition) -> usize {
        usize::from(position.row.min(self.rows.saturating_sub(1))) * usize::from(self.columns)
            + usize::from(position.column.min(self.columns.saturating_sub(1)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSelection {
    pub anchor: TerminalPosition,
    pub focus: TerminalPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalEvent {
    Input(Vec<u8>),
    Resize { columns: u16, rows: u16 },
    SelectionChanged(Option<TerminalSelection>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalView {
    pub screen: TerminalScreen,
    pub selection: Option<TerminalSelection>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub font_family: String,
    pub disabled: bool,
    pub read_only: bool,
    pub style: NodeStyle,
    preedit: String,
    dragging: Option<u64>,
    viewport: Option<(u16, u16)>,
}

impl TerminalView {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn new(screen: TerminalScreen) -> Self {
        let mut style = NodeStyle::default().surface(SemanticColorRole::Surface);
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Relative;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.flex_grow = Some(1.0);
        layout.flex_shrink = Some(1.0);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        Self {
            screen,
            selection: None,
            cell_width: 8.0,
            cell_height: 18.0,
            font_size: 14.0,
            font_family: "Cascadia Mono".to_owned(),
            disabled: false,
            read_only: false,
            style,
            preedit: String::new(),
            dragging: None,
            viewport: None,
        }
    }

    pub fn selected_text(&self) -> String {
        if !self.screen.valid() {
            return String::new();
        }
        let Some(selection) = self.selection else {
            return String::new();
        };
        let (start, end) = self.selection_indices(selection);
        let mut output = String::new();
        for index in start..=end {
            if index > start && index % usize::from(self.screen.columns) == 0 {
                while output.ends_with(' ') {
                    output.pop();
                }
                output.push('\n');
            }
            if let Some(cell) = self.screen.cells.get(index).filter(|cell| cell.width > 0) {
                output.push_str(&cell.text);
            }
        }
        output
    }

    fn selection_indices(&self, selection: TerminalSelection) -> (usize, usize) {
        let mut a = self.screen.index(selection.anchor);
        let mut b = self.screen.index(selection.focus);
        if self.screen.cells.get(a).is_some_and(|cell| cell.width == 0) {
            a = a.saturating_sub(1);
        }
        if self.screen.cells.get(b).is_some_and(|cell| cell.width == 0) {
            b = b.saturating_sub(1);
        }
        (a.min(b), a.max(b))
    }

    fn position_at(&self, bounds: LayoutBox, x: f32, y: f32) -> TerminalPosition {
        TerminalPosition {
            row: (((y - bounds.y).max(0.0) / self.cell_height.max(1.0)) as u16)
                .min(self.screen.rows.saturating_sub(1)),
            column: (((x - bounds.x).max(0.0) / self.cell_width.max(1.0)) as u16)
                .min(self.screen.columns.saturating_sub(1)),
        }
    }
}

impl ComponentView for TerminalView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "terminal".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::from("Terminal")),
                disabled: self.disabled,
                ..Default::default()
            },
        );
    }
}

#[derive(Clone)]
struct TerminalRow(NodeStyle);
impl ComponentView for TerminalRow {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "terminal-row".into(),
        }
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.0,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState::default(),
        );
    }
}

impl RegisterableComponent for TerminalView {
    const TYPE_ID: &'static str = "nana.terminal";
    const TAGS: &'static [&'static str] = &["terminal"];
    const RETAIN_SEMANTIC_STATE: bool = true;
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut view = Self::new(TerminalScreen::blank(1, 1));
        view.disabled = spec.disabled;
        view
    }
}

impl AppContext {
    pub fn sync_terminal_screen(
        &mut self,
        entity: Entity<TerminalView>,
        screen: TerminalScreen,
    ) -> Result<(), FrameworkError> {
        if !screen.valid() {
            return Err(FrameworkError::InvalidInput);
        }
        self.update_component(entity, |view, _| {
            view.screen = screen;
        })?;
        Ok(())
    }

    pub fn refresh_terminal_view(
        &mut self,
        entity: Entity<TerminalView>,
    ) -> Result<(), FrameworkError> {
        let view = self.read(entity, Clone::clone)?;
        if !view.screen.valid()
            || !view.cell_width.is_finite()
            || !view.cell_height.is_finite()
            || view.cell_width <= 0.0
            || view.cell_height <= 0.0
        {
            return Err(FrameworkError::InvalidInput);
        }
        let selection = view
            .selection
            .map(|selection| view.selection_indices(selection));
        self.mount(entity, |ui| {
            for (row, cells) in view
                .screen
                .cells
                .chunks(usize::from(view.screen.columns))
                .enumerate()
            {
                let mut style = NodeStyle::default();
                let layout = Arc::make_mut(&mut style.layout);
                layout.position = PositionSpec::Absolute;
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.offset_top = Some(LengthSpec::Px(row as f32 * view.cell_height));
                layout.width = Some(LengthSpec::Px(
                    f32::from(view.screen.columns) * view.cell_width,
                ));
                layout.height = Some(LengthSpec::Px(view.cell_height));
                ui.with_child(format!("row-{row}"), TerminalRow(style), |ui| {
                    for (column, cell) in cells.iter().enumerate() {
                        let index = row * usize::from(view.screen.columns) + column;
                        if cell.width == 0 {
                            continue;
                        }
                        let mut style = NodeStyle::default();
                        let selected = selection.is_some_and(|(a, b)| {
                            index <= b && index + usize::from(cell.width) > a
                        });
                        style.foreground = Some(if cell.inverse {
                            SemanticColorRole::Surface
                        } else if cell.dim {
                            SemanticColorRole::Muted
                        } else {
                            SemanticColorRole::Text
                        });
                        if cell.inverse {
                            style.background = Some(SemanticColorRole::Text);
                        }
                        if selected {
                            style.background = Some(SemanticColorRole::Selected);
                        }
                        let layout = Arc::make_mut(&mut style.layout);
                        layout.position = PositionSpec::Absolute;
                        layout.offset_left = Some(LengthSpec::Px(
                            (index % usize::from(view.screen.columns)) as f32 * view.cell_width,
                        ));
                        layout.offset_top = Some(LengthSpec::Px(0.0));
                        layout.width =
                            Some(LengthSpec::Px(view.cell_width * f32::from(cell.width)));
                        layout.height = Some(LengthSpec::Px(view.cell_height));
                        layout.font_size = Some(view.font_size);
                        layout.font_family = Some(view.font_family.clone());
                        layout.line_height = Some(LineHeightSpec::Absolute(view.cell_height));
                        layout.font_weight = Some(if cell.bold { 700 } else { 400 });
                        layout.font_italic = Some(cell.italic);
                        let (foreground, background) = if cell.inverse {
                            (cell.background, cell.foreground)
                        } else {
                            (cell.foreground, cell.background)
                        };
                        layout.color = foreground.map(|mut color| {
                            if cell.dim {
                                color[3] *= 0.5;
                            }
                            color
                        });
                        if !selected {
                            layout.background = background;
                        }
                        layout.text_decoration = Some(TextDecorationLine {
                            underline: cell.underline,
                            line_through: false,
                        });
                        layout.white_space_nowrap = true;
                        layout.overflow_x = OverflowSpec::Hidden;
                        layout.overflow_y = OverflowSpec::Hidden;
                        ui.child(
                            format!("cell-{index}"),
                            Text::new(cell.text.to_string()).style(style),
                        )?;
                    }
                    Ok(())
                })?;
            }
            let cursor = view.screen.cursor.unwrap_or(TerminalCursor {
                position: TerminalPosition::default(),
                shape: TerminalCursorShape::Block,
                visible: false,
            });
            if (cursor.visible || !view.preedit.is_empty())
                && cursor.position.row < view.screen.rows
                && cursor.position.column < view.screen.columns
            {
                let mut style = NodeStyle::default().outline(SemanticColorRole::Text, 1.0);
                let layout = Arc::make_mut(&mut style.layout);
                layout.position = PositionSpec::Absolute;
                layout.offset_left = Some(LengthSpec::Px(
                    f32::from(cursor.position.column) * view.cell_width,
                ));
                layout.offset_top = Some(LengthSpec::Px(
                    f32::from(cursor.position.row) * view.cell_height
                        + if cursor.shape == TerminalCursorShape::Underline {
                            view.cell_height - 2.0
                        } else {
                            0.0
                        },
                ));
                layout.width = Some(LengthSpec::Px(
                    if cursor.shape == TerminalCursorShape::Bar {
                        2.0
                    } else {
                        view.cell_width
                    },
                ));
                layout.height = Some(LengthSpec::Px(
                    if cursor.shape == TerminalCursorShape::Underline {
                        2.0
                    } else {
                        view.cell_height
                    },
                ));
                if cursor.visible {
                    ui.child("cursor", Text::new("").style(style))?;
                }
                if !view.preedit.is_empty() {
                    let mut style = NodeStyle::default().surface(SemanticColorRole::Surface);
                    let layout = Arc::make_mut(&mut style.layout);
                    layout.position = PositionSpec::Absolute;
                    layout.offset_left = Some(LengthSpec::Px(
                        f32::from(cursor.position.column) * view.cell_width,
                    ));
                    layout.offset_top = Some(LengthSpec::Px(
                        f32::from(cursor.position.row) * view.cell_height,
                    ));
                    layout.font_size = Some(view.font_size);
                    layout.font_family = Some(view.font_family.clone());
                    layout.text_decoration = Some(TextDecorationLine {
                        underline: true,
                        line_through: false,
                    });
                    ui.child("preedit", Text::new(view.preedit.clone()).style(style))?;
                }
            }
            Ok(())
        })
    }

    pub fn resize_terminal_view(
        &mut self,
        entity: Entity<TerminalView>,
        width: f32,
        height: f32,
    ) -> Result<bool, FrameworkError> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Ok(false);
        }
        let (columns, rows, previous) = self.read(entity, |view| {
            (
                (width / view.cell_width.max(1.0)).floor().max(1.0) as u16,
                (height / view.cell_height.max(1.0)).floor().max(1.0) as u16,
                view.viewport,
            )
        })?;
        if previous == Some((columns, rows)) {
            return Ok(false);
        }
        let mut changed = false;
        self.update_component(entity, |view, cx| {
            let columns = (width / view.cell_width.max(1.0)).floor().max(1.0) as u16;
            let rows = (height / view.cell_height.max(1.0)).floor().max(1.0) as u16;
            if view.viewport != Some((columns, rows)) {
                view.viewport = Some((columns, rows));
                cx.emit(TerminalEvent::Resize { columns, rows });
                changed = true;
            }
        })?;
        Ok(changed)
    }

    pub fn focused_terminal(&self, document: DocumentId) -> Option<Entity<TerminalView>> {
        let id = self.world().focused(document)?;
        let entity = Entity::from_stable_id(id);
        self.read(entity, |view: &TerminalView| !view.disabled)
            .ok()
            .filter(|value| *value)
            .map(|_| entity)
    }

    pub fn terminal_accepts_input(&self, document: DocumentId) -> bool {
        self.focused_terminal(document)
            .is_some_and(|entity| self.read(entity, |view| !view.read_only).unwrap_or(false))
    }

    pub fn terminal_caret_bounds(&self, document: DocumentId) -> Option<LayoutBox> {
        let entity = self.focused_terminal(document)?;
        let bounds = self.world().layout_box(entity.stable_id())?;
        self.read(entity, |view| {
            let position = view
                .screen
                .cursor
                .map(|cursor| cursor.position)
                .unwrap_or_default();
            LayoutBox {
                x: bounds.x + f32::from(position.column) * view.cell_width,
                y: bounds.y + f32::from(position.row) * view.cell_height,
                width: view.cell_width,
                height: view.cell_height,
            }
        })
        .ok()
    }

    pub fn set_terminal_preedit(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_terminal(document) else {
            return Ok(false);
        };
        self.update_component(entity, |view, _| {
            view.preedit = if view.read_only {
                String::new()
            } else {
                text.to_owned()
            }
        })?;
        Ok(true)
    }

    pub fn terminal_selected_text(&self, document: DocumentId) -> Option<String> {
        self.read(
            self.focused_terminal(document)?,
            TerminalView::selected_text,
        )
        .ok()
    }

    pub fn terminal_input(
        &mut self,
        document: DocumentId,
        bytes: Vec<u8>,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_terminal(document) else {
            return Ok(false);
        };
        if self.read(entity, |view| view.read_only)? {
            return Ok(true);
        }
        if !bytes.is_empty() {
            self.update_component(entity, |view, cx| {
                view.selection = None;
                view.preedit.clear();
                cx.emit(TerminalEvent::Input(bytes));
            })?;
        }
        Ok(true)
    }

    pub fn paste_terminal(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_terminal(document) else {
            return Ok(false);
        };
        let bracketed = self.read(entity, |view| view.screen.bracketed_paste)?;
        self.terminal_input(
            document,
            if bracketed {
                format!("\x1b[200~{text}\x1b[201~").into_bytes()
            } else {
                text.as_bytes().to_vec()
            },
        )
    }

    pub fn terminal_key(
        &mut self,
        document: DocumentId,
        key: &str,
        text: Option<&str>,
        control: bool,
        alt: bool,
        shift: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_terminal(document) else {
            return Ok(false);
        };
        if self.read(entity, |view| !view.preedit.is_empty())? {
            return Ok(true);
        }
        let application_cursor = self.read(entity, |view| view.screen.application_cursor)?;
        let bytes = terminal_key_bytes(key, text, control, alt, shift, application_cursor);
        self.terminal_input(document, bytes)
    }

    /// `phase`: 0 press, 1 drag, 2 release, 3 cancel; host owns pointer capture.
    pub fn terminal_pointer(
        &mut self,
        document: DocumentId,
        target: Option<StableNodeId>,
        pointer: u64,
        phase: u8,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let entity = if phase == 0 {
            target.and_then(|id| {
                let entity = Entity::from_stable_id(id);
                self.read(entity, |view: &TerminalView| !view.disabled)
                    .ok()
                    .filter(|enabled| *enabled)
                    .map(|_| entity)
            })
        } else {
            self.focused_terminal(document).filter(|entity| {
                self.read(*entity, |view| view.dragging == Some(pointer))
                    .unwrap_or(false)
            })
        };
        let Some(entity) = entity else {
            return Ok(false);
        };
        let Some(bounds) = self.world().layout_box(entity.stable_id()) else {
            return Ok(false);
        };
        if phase == 0 {
            self.focus_node(document, entity.stable_id())?;
            self.press_pointer(document, pointer, entity.stable_id())?;
        }
        if phase >= 2 {
            self.release_pointer(document, pointer);
        }
        self.update_component(entity, |view, cx| {
            let position = view.position_at(bounds, x, y);
            match phase {
                0 => {
                    view.dragging = Some(pointer);
                    view.selection = Some(TerminalSelection {
                        anchor: position,
                        focus: position,
                    });
                }
                1 | 2 => {
                    if let Some(selection) = &mut view.selection {
                        selection.focus = position;
                    }
                    if phase == 2 {
                        view.dragging = None;
                    }
                }
                _ => {
                    view.dragging = None;
                    view.selection = None;
                }
            }
            cx.emit(TerminalEvent::SelectionChanged(view.selection));
        })?;
        Ok(true)
    }
}

fn terminal_key_bytes(
    key: &str,
    text: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    application_cursor: bool,
) -> Vec<u8> {
    let arrow = match key {
        "ArrowUp" => Some("A"),
        "ArrowDown" => Some("B"),
        "ArrowRight" => Some("C"),
        "ArrowLeft" => Some("D"),
        "Home" => Some("H"),
        "End" => Some("F"),
        _ => None,
    };
    let function = match key {
        "F1" => Some(11),
        "F2" => Some(12),
        "F3" => Some(13),
        "F4" => Some(14),
        "F5" => Some(15),
        "F6" => Some(17),
        "F7" => Some(18),
        "F8" => Some(19),
        "F9" => Some(20),
        "F10" => Some(21),
        "F11" => Some(23),
        "F12" => Some(24),
        _ => None,
    };
    let mut bytes = if let Some(code) = function {
        let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else if code <= 14 {
            vec![0x1b, b'O', b'P' + (code - 11)]
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    } else if control && key == "Space" {
        vec![0]
    } else if let Some(arrow) = arrow {
        let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
        if modifier > 1 {
            format!("\x1b[1;{modifier}{arrow}").into_bytes()
        } else {
            format!("\x1b{}{arrow}", if application_cursor { "O" } else { "[" }).into_bytes()
        }
    } else if control
        && key.len() == 1
        && (key.as_bytes()[0].is_ascii_alphabetic() || b"@[\\]^_ ".contains(&key.as_bytes()[0]))
    {
        vec![key.as_bytes()[0].to_ascii_uppercase() & 0x1f]
    } else {
        match key {
            "Enter" => b"\r".to_vec(),
            "Backspace" => vec![0x7f],
            "Tab" if shift => b"\x1b[Z".to_vec(),
            "Tab" => b"\t".to_vec(),
            "Escape" => vec![0x1b],
            "Delete" => b"\x1b[3~".to_vec(),
            "Insert" => b"\x1b[2~".to_vec(),
            "PageUp" => b"\x1b[5~".to_vec(),
            "PageDown" => b"\x1b[6~".to_vec(),
            _ => text.unwrap_or_default().as_bytes().to_vec(),
        }
    };
    if alt && arrow.is_none() && function.is_none() && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn layout_emits_size_and_rebinding_republishes_it() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let terminal = context
            .create_component(document, TerminalView::new(TerminalScreen::blank(3, 2)))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        context
            .on(terminal, move |_, event: &TerminalEvent, _| {
                observed.lock().unwrap().push(event.clone())
            })
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(80.0, 36.0))
            .unwrap();
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[TerminalEvent::Resize {
                columns: 10,
                rows: 2
            }]
        );
        let rows = context.world().node(terminal.stable_id()).unwrap().children;
        let second_cell = context.world().node(rows[1]).unwrap().children[1];
        assert_eq!(
            context.world().layout_box(second_cell),
            Some(LayoutBox {
                x: 8.0,
                y: 18.0,
                width: 8.0,
                height: 18.0
            })
        );
        context
            .layout_document(document, crate::LayoutViewport::new(81.0, 36.0))
            .unwrap();
        assert_eq!(events.lock().unwrap().len(), 1);
        context
            .update_component(terminal, |view, _| {
                *view = TerminalView::new(TerminalScreen::blank(3, 2))
            })
            .unwrap();
        assert_eq!(events.lock().unwrap().len(), 2);
        let oversized = TerminalScreen::blank(u16::MAX, u16::MAX);
        assert!(oversized.cells.is_empty());
        assert!(context.sync_terminal_screen(terminal, oversized).is_err());
        assert_eq!(
            context.read(terminal, |view| view.screen.columns).unwrap(),
            3
        );
    }

    #[test]
    fn grid_reuses_cells_and_removes_stale_nodes_on_resize() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let terminal = context
            .create_component(document, TerminalView::new(TerminalScreen::blank(3, 2)))
            .unwrap();
        let initial = context
            .world()
            .node(terminal.stable_id())
            .unwrap()
            .children
            .clone();
        assert_eq!(initial.len(), 2);
        let mut screen = TerminalScreen::blank(3, 2);
        Arc::make_mut(&mut screen.cells)[0].text = Arc::from("a");
        context.sync_terminal_screen(terminal, screen).unwrap();
        assert_eq!(
            context.world().node(terminal.stable_id()).unwrap().children,
            initial
        );
        context
            .sync_terminal_screen(terminal, TerminalScreen::blank(2, 1))
            .unwrap();
        assert_eq!(
            context
                .world()
                .node(terminal.stable_id())
                .unwrap()
                .children
                .len(),
            1
        );
        assert!(
            initial[1..]
                .iter()
                .all(|id| context.world().node(*id).is_none())
        );
    }

    #[test]
    fn wide_continuation_selection_copies_the_whole_grapheme() {
        let mut screen = TerminalScreen::blank(3, 2);
        let cells = Arc::make_mut(&mut screen.cells);
        cells[0].text = Arc::from("你");
        cells[0].width = 2;
        cells[1].text = Arc::from("");
        cells[1].width = 0;
        cells[3].text = Arc::from("e\u{301}");
        let mut view = TerminalView::new(screen);
        view.selection = Some(TerminalSelection {
            anchor: TerminalPosition { row: 0, column: 1 },
            focus: TerminalPosition { row: 1, column: 0 },
        });
        assert_eq!(view.selected_text(), "你\ne\u{301}");
    }

    #[test]
    fn terminal_input_is_scoped_and_resize_is_coalesced() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, TerminalView::new(TerminalScreen::blank(3, 2)))
            .unwrap();
        let mut screen = TerminalScreen::blank(3, 2);
        screen.bracketed_paste = true;
        let second = context
            .create_component(document, TerminalView::new(screen))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        context
            .on(second, move |_, event: &TerminalEvent, _| {
                observed.lock().unwrap().push(event.clone())
            })
            .unwrap();
        context.focus_node(document, second.stable_id()).unwrap();
        context
            .terminal_key(document, "c", None, true, false, false)
            .unwrap();
        context.paste_terminal(document, "中文").unwrap();
        assert!(context.resize_terminal_view(second, 80.0, 36.0).unwrap());
        assert!(!context.resize_terminal_view(second, 81.0, 36.0).unwrap());
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                TerminalEvent::Input(vec![3]),
                TerminalEvent::Input("\x1b[200~中文\x1b[201~".as_bytes().to_vec()),
                TerminalEvent::Resize {
                    columns: 10,
                    rows: 2
                }
            ]
        );
        assert!(
            context
                .read(first, |view| view.selection.is_none())
                .unwrap()
        );
        context
            .update_component(second, |view, _| {
                view.read_only = true;
                view.selection = Some(TerminalSelection {
                    anchor: TerminalPosition::default(),
                    focus: TerminalPosition::default(),
                });
            })
            .unwrap();
        context.terminal_input(document, vec![b'a']).unwrap();
        assert_eq!(events.lock().unwrap().len(), 3);
        assert_eq!(
            context.terminal_selected_text(document).as_deref(),
            Some(" ")
        );
        assert!(!context.terminal_accepts_input(document));
        assert_eq!(
            terminal_key_bytes("ArrowUp", None, false, false, false, true),
            b"\x1bOA"
        );
        assert_eq!(
            terminal_key_bytes("ArrowLeft", None, true, false, false, false),
            b"\x1b[1;5D"
        );
    }
}
