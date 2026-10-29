//! The pointer over an open Select, Dropdown or SearchDropdown menu: the row
//! under it takes the highlight the arrow keys move. Over a menu whose rows
//! overflow it the wheel scrolls the rows, the scrollbar's lane heats up
//! under the pointer, and its thumb drags. A press on a row stays the
//! field's activation ([`AppContext::activate_select_at`] and its siblings).
use super::*;
use crate::{
    ComponentGeometry, Dropdown, NodePointer, NodePointerHooks, NodeWheel, SearchDropdown, Select,
    SelectMenuDrag, SelectMenuGeometry, SelectMenuScroll,
};

type HandledResult = Result<bool, FrameworkError>;

/// A field that opens a Select menu: the state these hooks drive.
pub(crate) trait MenuField: ComponentView {
    fn menu_open(&self) -> bool;
    fn menu_scroll(&self) -> SelectMenuScroll;
    fn menu_scroll_mut(&mut self) -> &mut SelectMenuScroll;
    /// The menu row the highlight is on.
    fn highlighted_row(&self) -> Option<usize>;
    /// Put the highlight on menu row `row`, an enabled one.
    fn highlight_row(&mut self, row: usize);
}

impl MenuField for Select {
    fn menu_open(&self) -> bool {
        self.opened && !self.inactive()
    }

    fn menu_scroll(&self) -> SelectMenuScroll {
        self.menu_scroll
    }

    fn menu_scroll_mut(&mut self) -> &mut SelectMenuScroll {
        &mut self.menu_scroll
    }

    fn highlighted_row(&self) -> Option<usize> {
        self.highlighted
    }

    fn highlight_row(&mut self, row: usize) {
        self.highlighted = Some(row);
    }
}

impl MenuField for Dropdown {
    fn menu_open(&self) -> bool {
        self.opened && !self.inactive()
    }

    fn menu_scroll(&self) -> SelectMenuScroll {
        self.menu_scroll
    }

    fn menu_scroll_mut(&mut self) -> &mut SelectMenuScroll {
        &mut self.menu_scroll
    }

    fn highlighted_row(&self) -> Option<usize> {
        self.highlighted
    }

    fn highlight_row(&mut self, row: usize) {
        self.highlighted = Some(row);
    }
}

impl MenuField for SearchDropdown {
    fn menu_open(&self) -> bool {
        self.opened && !self.inactive()
    }

    fn menu_scroll(&self) -> SelectMenuScroll {
        self.menu_scroll
    }

    fn menu_scroll_mut(&mut self) -> &mut SelectMenuScroll {
        &mut self.menu_scroll
    }

    fn highlighted_row(&self) -> Option<usize> {
        self.highlighted_visible()
    }

    /// The menu lists only the options the query matches.
    fn highlight_row(&mut self, row: usize) {
        if let Some(index) = self.visible_indices().get(row) {
            self.highlighted = Some(*index);
        }
    }
}

pub(crate) const SELECT_MENU_POINTER: NodePointerHooks<Select> = NodePointerHooks {
    moved: menu_moved::<Select>,
    left: menu_left::<Select>,
    pressed: menu_pressed::<Select>,
    released: menu_released::<Select>,
    wheel: menu_wheel::<Select>,
};

pub(crate) const DROPDOWN_MENU_POINTER: NodePointerHooks<Dropdown> = NodePointerHooks {
    moved: menu_moved::<Dropdown>,
    left: menu_left::<Dropdown>,
    pressed: menu_pressed::<Dropdown>,
    released: menu_released::<Dropdown>,
    wheel: menu_wheel::<Dropdown>,
};

pub(crate) const SEARCH_DROPDOWN_MENU_POINTER: NodePointerHooks<SearchDropdown> =
    NodePointerHooks {
        moved: menu_moved::<SearchDropdown>,
        left: menu_left::<SearchDropdown>,
        pressed: menu_pressed::<SearchDropdown>,
        released: menu_released::<SearchDropdown>,
        wheel: menu_wheel::<SearchDropdown>,
    };

impl AppContext {
    /// `id`'s open Select menu as it is drawn.
    pub(crate) fn open_select_menu(&self, id: StableNodeId) -> Option<SelectMenuGeometry> {
        match self.world.component_geometry(id)? {
            ComponentGeometry::Select {
                menu: Some(menu), ..
            } => Some(menu),
            _ => None,
        }
    }

    /// After a layout pass of `document`: every open menu whose room the
    /// pass moved -- a resized window, a field or clip that moved -- takes
    /// the room the published layout gives it, so it flips and caps against
    /// what can be seen now with no projection of its field. Only the room
    /// changes: the hit index rebuilds the field's entry, and paint reads the
    /// same geometry.
    pub(super) fn refresh_select_menu_rooms(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let stale = self.world.stale_select_menu_rooms(document);
        if stale.is_empty() {
            return Ok(());
        }
        let mut mutations = MutationQueue::new();
        for (id, visual) in stale {
            mutations.set_standard_visual(id, Some(visual));
        }
        self.commit_mutations(mutations)?;
        Ok(())
    }

    /// A window point in the layout px `id`'s menu is laid out in, through
    /// the scroll offsets and transforms above it.
    pub(crate) fn menu_point(&self, id: StableNodeId, x: f32, y: f32) -> (f32, f32) {
        self.world
            .pointer_layout_position(id, x, y)
            .unwrap_or((x, y))
    }
}

/// The open menu, and the hook's point in the layout px it is laid out in:
/// a hook's pointer is in the node's own px.
fn open_menu<C: MenuField>(
    cx: &AppContext,
    field: Entity<C>,
    x: f32,
    y: f32,
) -> Result<Option<(SelectMenuGeometry, f32, f32)>, FrameworkError> {
    if !cx.read(field, C::menu_open)? {
        return Ok(None);
    }
    let id = field.stable_id();
    let (Some(menu), Some(origin)) = (cx.open_select_menu(id), cx.world.component_layout_box(id))
    else {
        return Ok(None);
    };
    Ok(Some((menu, origin.x + x, origin.y + y)))
}

/// Change the menu's scroll; answers whether it changed. Most pointer
/// moves change nothing, and those leave the field alone.
fn update_scroll<C: MenuField>(
    cx: &mut AppContext,
    field: Entity<C>,
    change: impl FnOnce(&mut SelectMenuScroll),
) -> HandledResult {
    let current = cx.read(field, C::menu_scroll)?;
    let mut next = current;
    change(&mut next);
    if next == current {
        return Ok(false);
    }
    cx.update_component(field, |field, _| *field.menu_scroll_mut() = next)?;
    Ok(true)
}

/// The row under the pointer takes the highlight. Between rows, on the
/// scrollbar or over the field it stays where it was, and a thumb being
/// dragged moves only the rows.
fn menu_moved<C: MenuField>(
    cx: &mut AppContext,
    field: Entity<C>,
    pointer: NodePointer,
) -> HandledResult {
    let Some((menu, x, y)) = open_menu(cx, field, pointer.x, pointer.y)? else {
        return Ok(false);
    };
    if let Some(bar) = menu.scrollbar
        && pointer.captured
        && let Some(drag) = cx.read(field, |field| field.menu_scroll().drag)?
    {
        let offset = bar
            .track_geometry(nana_ui_core::ScrollbarAxis::Vertical)
            .offset_for_thumb_origin(y - drag.grab);
        return update_scroll(cx, field, |scroll| scroll.offset = Some(offset));
    }
    let hot = menu.scrollbar.is_some_and(|bar| bar.contains(x, y));
    let highlighted = cx.read(field, C::highlighted_row)?;
    let Some(row) =
        crate::select::select_option_at(&menu, x, y).filter(|row| highlighted != Some(*row))
    else {
        return update_scroll(cx, field, |scroll| scroll.hot = hot);
    };
    cx.update_component(field, |field, _| {
        let scroll = field.menu_scroll_mut();
        scroll.hot = hot;
        // Rows nothing has scrolled sit around the highlight; they stay where
        // they are drawn while it follows the pointer.
        scroll.offset.get_or_insert(menu.scroll);
        field.highlight_row(row);
    })?;
    Ok(true)
}

/// The pointer left the field and its menu, or stopped reaching them. A
/// drag still held here never saw its release: the press was cancelled, and
/// the rows go back to where it started.
fn menu_left<C: MenuField>(cx: &mut AppContext, field: Entity<C>) -> Result<(), FrameworkError> {
    update_scroll(cx, field, |scroll| {
        if let Some(drag) = scroll.drag.take() {
            scroll.offset = Some(drag.from);
        }
        scroll.hot = false;
    })?;
    Ok(())
}

/// A press on the scrollbar's lane takes the thumb: where it was taken on
/// the thumb, or, on bare track, with the thumb centred on the press first,
/// as a scroll view's bar does.
fn menu_pressed<C: MenuField>(
    cx: &mut AppContext,
    field: Entity<C>,
    pointer: NodePointer,
) -> HandledResult {
    if pointer.button != 0 {
        return Ok(false);
    }
    let Some((menu, x, y)) = open_menu(cx, field, pointer.x, pointer.y)? else {
        return Ok(false);
    };
    let Some(bar) = menu.scrollbar.filter(|bar| bar.contains(x, y)) else {
        return Ok(false);
    };
    let track = bar.track_geometry(nana_ui_core::ScrollbarAxis::Vertical);
    let (offset, grab) = if track.thumb_contains(y) {
        (menu.scroll, y - track.thumb_origin)
    } else {
        (track.offset_for_position(y), track.thumb_length / 2.0)
    };
    update_scroll(cx, field, |scroll| {
        scroll.offset = Some(offset);
        scroll.hot = true;
        scroll.drag = Some(SelectMenuDrag {
            grab,
            from: menu.scroll,
        });
    })?;
    // A searchable field's editor saw the press first and began selecting
    // its query; the press is the bar's.
    cx.text_edit.text_pointer_drag = None;
    Ok(true)
}

fn menu_released<C: MenuField>(
    cx: &mut AppContext,
    field: Entity<C>,
    pointer: NodePointer,
) -> HandledResult {
    let hot = open_menu(cx, field, pointer.x, pointer.y)?
        .and_then(|(menu, x, y)| menu.scrollbar.map(|bar| bar.contains(x, y)))
        .unwrap_or(false);
    update_scroll(cx, field, |scroll| {
        scroll.drag = None;
        scroll.hot = hot;
    })?;
    Ok(true)
}

/// The wheel over a menu whose rows overflow scrolls them, and stays there
/// at either end too: the page under an open menu does not move it away.
/// Over a menu that holds its rows, or over the field, the wheel goes on to
/// whatever scrolls under it.
fn menu_wheel<C: MenuField>(
    cx: &mut AppContext,
    field: Entity<C>,
    wheel: NodeWheel,
) -> HandledResult {
    if wheel.delta_y == 0.0 || wheel.ctrl {
        return Ok(false);
    }
    let Some((menu, x, y)) = open_menu(cx, field, wheel.x, wheel.y)? else {
        return Ok(false);
    };
    if menu.max_scroll <= 0.0 || !menu.surface.contains(x, y) {
        return Ok(false);
    }
    let offset = (menu.scroll - wheel.delta_y).clamp(0.0, menu.max_scroll);
    update_scroll(cx, field, |scroll| scroll.offset = Some(offset))?;
    Ok(true)
}
