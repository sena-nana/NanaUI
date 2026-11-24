//! Component geometry derived from committed Runtime data.

use super::*;

mod text_input;

/// Block extent of a progress bar.
pub(crate) const PROGRESS_GIRTH: f32 = nana_ui_core::space::SM;

impl UiWorld {
    /// An open menu's room in its field's own coordinates: the room the
    /// field projected (or the window, before it has one) moved by the
    /// field's page origin. `None` for a closed field, and for an open one
    /// with neither.
    ///
    /// This reads where the field stands, as fitting a menu to what can be
    /// seen must; what it decides is only where the menu's surface goes
    /// ([`crate::select::select_menu_surface`]), and a field that only moved
    /// keeps its record while that box is the same
    /// ([`Self::local_select_menu_surface`]).
    pub(crate) fn local_select_menu_room(
        &self,
        id: StableNodeId,
        opened: bool,
        room: Option<crate::SelectMenuRoom>,
    ) -> Option<crate::SelectMenuRoom> {
        if !opened {
            return None;
        }
        let room = room.or_else(|| {
            let document = self.node(id)?.document;
            crate::select::window_room(self.invalidation.document_viewports.get(&document).copied())
        })?;
        let [x, y] = self.layout_origin_in(id, None)?;
        Some(crate::SelectMenuRoom {
            top: room.top - y,
            bottom: room.bottom - y,
            left: room.left - x,
            right: room.right - x,
        })
    }

    /// Where `id`'s open menu surface stands in the field's own coordinates
    /// now, as [`Self::local_widget_paint`] would place it: the one
    /// part of an open field's record that depends on where the field
    /// stands. `None` for anything but an open Select, Dropdown or
    /// SearchDropdown.
    pub(crate) fn local_select_menu_surface(&self, id: StableNodeId) -> Option<LayoutBox> {
        let Some(crate::SelectVisual {
            opened: true,
            size,
            options,
            menu_room,
            checkable,
            fit_options,
            ..
        }) = self.widget_visual_as::<crate::SelectVisual>(id)
        else {
            return None;
        };
        let previous = self.invalidation.geometry_frame.replace(Some(id));
        let bounds = self.component_layout_box(id);
        let room = self.local_select_menu_room(id, true, *menu_room);
        let fitted = self.select_fitted_options_width(id, *fit_options);
        self.invalidation.geometry_frame.set(previous);
        let node_theme = self.node_theme(id);
        let style_model = self.node_style_model(id);
        Some(crate::select::select_menu_surface(
            bounds?,
            *size,
            options,
            style_model,
            node_theme,
            room,
            *checkable,
            fitted,
        ))
    }

    /// The open Select, Dropdown and SearchDropdown menus of `document`
    /// whose room is no longer the one [`Self::select_menu_room`] gives
    /// now, each with its visual carrying the room it has now.
    ///
    /// A field projects its menu's room from the boxes the last layout gave
    /// it; a layout pass that moves those boxes or the viewport, but not the
    /// field's own state, leaves that room behind. Fed back after every
    /// layout pass, the menu that is painted, hit and read is placed against
    /// the layout just published.
    pub(crate) fn stale_select_menu_rooms(
        &self,
        document: DocumentId,
    ) -> Vec<(StableNodeId, crate::SelectVisual)> {
        let mut stale = self
            .nodes
            .open_select_menus()
            .filter(|id| self.document_of(*id) == Some(document) && self.is_mounted(*id))
            .filter_map(|id| {
                let visual = self.widget_visual_as::<crate::SelectVisual>(id)?;
                let room = self.select_menu_room(id);
                (visual.menu_room != room).then(|| {
                    let mut updated = visual.clone();
                    updated.menu_room = room;
                    (id, updated)
                })
            })
            .collect::<Vec<_>>();
        stale.compact_sort_by_key(|(id, _)| *id);
        stale
    }

    /// Where a menu hanging off `id` can be seen, in `id`'s own layout
    /// coordinates: the window and every clip on `id`'s clip chain, each moved
    /// through the scroll offsets between that box and `id`. The chain is the
    /// one paint and hit testing follow ([`Self::clip_ancestors`], #201): an
    /// escaped box passes over the containers it escapes, and a `fixed` box,
    /// the field or one above it, ends it. Transforms are left out: they bend
    /// where the menu is placed, not whether what it draws matches what it
    /// hits.
    ///
    /// `None` until the document is laid out, or when nothing is left to see.
    pub(crate) fn select_menu_room(&self, id: StableNodeId) -> Option<crate::SelectMenuRoom> {
        let viewport = self.document_viewport_of(id)?;
        let mut room = crate::SelectMenuRoom {
            top: f32::NEG_INFINITY,
            bottom: f32::INFINITY,
            left: f32::NEG_INFINITY,
            right: f32::INFINITY,
        };
        let mut clip = |bounds: LayoutBox, shift: crate::ScrollOffset| {
            room.top = room.top.max(bounds.y + shift.y);
            room.bottom = room.bottom.min(bounds.y + bounds.height + shift.y);
            room.left = room.left.max(bounds.x + shift.x);
            room.right = room.right.min(bounds.x + bounds.width + shift.x);
        };
        // Scroll offsets between the box being looked at and `id`.
        let mut shift = crate::ScrollOffset::default();
        // The link of the chain below `ancestor`.
        let mut below = id;
        for ancestor in self.clip_ancestors(id) {
            let offset = self.scroll_offset(ancestor).unwrap_or_default();
            shift.x += offset.x;
            shift.y += offset.y;
            let style = self.hit_motion_layout(ancestor);
            let layout = self.component_layout_box(ancestor)?;
            if let Some((x, y, width, height)) =
                style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                clip(
                    LayoutBox {
                        x,
                        y,
                        width,
                        height,
                    },
                    shift,
                );
            }
            if self.invalidation.clip_visuals != 0 {
                if self
                    .widget_visual_ref(ancestor)
                    .is_some_and(crate::WidgetVisualHandle::clips_descendants)
                {
                    clip(layout, shift);
                }
                if let Some(modal) = self.widget_visual_as::<crate::ModalFrameVisual>(ancestor)
                    && let Some(crate::ModalFrameGeometry { surface, body, .. }) =
                        self.modal_frame_geometry(ancestor)
                {
                    clip(surface, shift);
                    if modal.slots.body == Some(below) {
                        clip(body, shift);
                    }
                }
            }
            below = ancestor;
        }
        clip(
            LayoutBox {
                width: viewport.width,
                height: viewport.height,
                ..LayoutBox::default()
            },
            shift,
        );
        (room.top < room.bottom && room.left < room.right).then_some(room)
    }
}

/// A value thumb's height: the control's text line with a small step of
/// room above and below it, a little under the control itself.
pub(crate) fn range_value_thumb_height(
    size: nana_ui_core::ControlSize,
    typography: nana_ui_core::TypographyTokens,
    spacing: nana_ui_core::SpacingTokens,
) -> f32 {
    let line = match size {
        nana_ui_core::ControlSize::Large => typography.line_tall,
        _ => typography.line,
    };
    line + spacing.sm
}
