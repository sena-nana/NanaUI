//! What one validated event does to the retained world. The router resolves
//! the source, the pointer identity and the time; these handlers apply the
//! event: pointer and wheel through one shared hit, keys and committed text
//! through one key chain, composition into the focused editor.

use std::time::Duration;

use nana_ui_core::TableNavigation;
use nana_ui_input::{
    CompositionInput, HostServices, InputDisposition, InputModifiers, KeyInput, PointerInput,
    PointerPhase, PointerType, WheelInput, WheelUnit,
};

use crate::{
    AppContext, DocumentId, FrameworkError, OverlayKey, OverlayPointerDecision,
    OverlayPointerPhase, RangeAdjustment, RovingFocusIntent, ScrollOffset, StableNodeId,
    TextCaretIntent, TextDeleteKind, TextLineDirection, TextShaper, XYPadAdjustment,
};

/// Logical pixels one wheel line scrolls.
pub(crate) const LINE_SCROLL_EXTENT: f32 = 60.0;
/// Rows PageUp and PageDown move a focused table by.
const TABLE_PAGE_ROWS: usize = 10;

const CONSUMED: InputDisposition = InputDisposition {
    handled: true,
    prevent_default: true,
};

/// A key press or release, or committed text, as the key chain reads it.
/// Committed text is a press with an empty `key`, so it reaches every editor
/// the way the key that typed it would, without being mistaken for one.
#[derive(Debug, Clone, Copy)]
pub(super) struct KeyStroke<'a> {
    pub(super) pressed: bool,
    pub(super) key: &'a str,
    pub(super) text: Option<&'a str>,
    pub(super) repeat: bool,
    pub(super) modifiers: InputModifiers,
    /// The canonical key this stroke came from. Text-only strokes have none.
    pub(super) canonical: Option<&'a KeyInput>,
}

impl<'a> KeyStroke<'a> {
    pub(super) fn text(text: &'a str) -> Self {
        Self {
            pressed: true,
            key: "",
            text: Some(text),
            repeat: false,
            modifiers: InputModifiers::default(),
            canonical: None,
        }
    }

    fn is_text(&self) -> bool {
        self.key.is_empty()
    }
}

impl AppContext {
    /// One pointer sample, its id already the source-local one. `landed`
    /// receives where it landed: the capture owner, else the topmost
    /// reachable node. It is resolved once and everything the event does
    /// reads it, so an uncaptured sample costs one hit query and a captured
    /// one none.
    pub(super) fn dispatch_pointer(
        &mut self,
        document: DocumentId,
        pointer: &PointerInput,
        now: Duration,
        mut text_shaper: Option<&mut dyn TextShaper>,
        landed: &mut Option<StableNodeId>,
    ) -> Result<InputDisposition, FrameworkError> {
        // Before anything routes, so the focus this press causes is recorded
        // against the device that caused it.
        self.world
            .note_input_modality(document, crate::InputModality::Pointer);
        let PointerInput {
            phase,
            x,
            y,
            button,
            is_primary,
            activation_click,
            modifiers,
            ..
        } = pointer;
        let pointer_id = &pointer.pointer_id.0;
        let held = (document, *pointer_id);
        self.note_pointer_detail(document, pointer, now);
        // A finger past the slop scrolls; until then the press is ordinary.
        let touch = pointer.pointer_type == PointerType::Touch;
        if touch
            && let Some(disposition) = self.touch_scroll_pointer(document, pointer, now, landed)?
        {
            return Ok(disposition);
        }
        match phase {
            PointerPhase::Down if (*is_primary && *button == 0) || *button == 1 => {
                if self
                    .input
                    .pressed_buttons
                    .get(&held)
                    .is_some_and(|pressed| pressed != button)
                {
                    return Ok(InputDisposition::default());
                }
                self.input.pressed_buttons.insert(held, *button);
            }
            PointerPhase::Up if (*is_primary && *button == 0) || *button == 1 => {
                if self
                    .input
                    .pressed_buttons
                    .get(&held)
                    .is_some_and(|pressed| pressed != button)
                {
                    return Ok(InputDisposition::default());
                }
                self.input.pressed_buttons.remove(&held);
            }
            PointerPhase::Cancel => {
                self.input.pressed_buttons.remove(&held);
            }
            _ => {}
        }
        let overlay_phase = match phase {
            PointerPhase::Move => OverlayPointerPhase::Move,
            PointerPhase::Down if *is_primary && *button == 0 => OverlayPointerPhase::PrimaryDown,
            PointerPhase::Up if *is_primary && *button == 0 => OverlayPointerPhase::PrimaryUp,
            PointerPhase::Cancel => OverlayPointerPhase::Cancel,
            PointerPhase::Down | PointerPhase::Up => OverlayPointerPhase::Move,
        };
        let overlay = if matches!(
            phase,
            PointerPhase::Move | PointerPhase::Up | PointerPhase::Cancel
        ) {
            self.world()
                .pointer_capture(document, *pointer_id)
                .map_or_else(
                    || self.route_overlay_pointer(document, *pointer_id, overlay_phase, *x, *y),
                    |target| {
                        Ok(OverlayPointerDecision {
                            target: Some(target),
                            prevent_default: false,
                            dismissed: false,
                        })
                    },
                )?
        } else {
            self.route_overlay_pointer(document, *pointer_id, overlay_phase, *x, *y)?
        };
        // A shown scrollbar lane takes the pointer from the content under it:
        // the row there loses its hover and its tooltip, the lane heats up,
        // and a press there grabs the bar. A captured pointer stays with its
        // capture, a drag included. A finger scrolls the content instead.
        let captured = self
            .world()
            .pointer_capture(document, *pointer_id)
            .is_some();
        let lane = if captured || touch || matches!(phase, PointerPhase::Cancel) {
            None
        } else {
            self.capabilities
                .scrollbar
                .and_then(|s| (s.near_hit)(self, *x, *y, overlay.target))
        };
        let target = lane.map_or(overlay.target, |(view, _)| Some(view));
        *landed = target;
        if touch && *phase == PointerPhase::Down {
            self.note_touch_target(document, *pointer_id, target);
        }
        self.set_pointer_location(document, *pointer_id, Some((*x, *y)));
        self.set_pointer_hover_at(document, *pointer_id, target, now)?;
        if !captured && let Some(scrollbar) = self.capabilities.scrollbar {
            (scrollbar.hot_sync)(self, document, *pointer_id, lane)?;
        }
        self.update_text_diagnostic_hover_hit(document, *x, *y, target)?;
        if *phase == PointerPhase::Move {
            self.emit_pointer_moved(target, *x, *y)?;
        }
        let terminal_phase = match phase {
            PointerPhase::Down if *is_primary && *button == 0 => Some(0),
            PointerPhase::Move => Some(1),
            PointerPhase::Up if *is_primary && *button == 0 => Some(2),
            PointerPhase::Cancel => Some(3),
            _ => None,
        };
        if !overlay.prevent_default
            && let Some(phase) = terminal_phase
            && let Some(term) = self.capabilities.terminal
            && (term.pointer)(self, document, target, *pointer_id, phase, *x, *y)?
        {
            return Ok(CONSUMED);
        }
        if !overlay.prevent_default
            && !activation_click
            && let Some(phase) = terminal_phase
            && let Some(rich_editor) = self.capabilities.rich_editor
            && (rich_editor.pointer)(
                self,
                document,
                target,
                *pointer_id,
                phase,
                *x,
                *y,
                modifiers.shift,
                now,
            )?
        {
            return Ok(CONSUMED);
        }

        let mut number_selected_all = false;
        let component_handled = match phase {
            PointerPhase::Move => {
                if self.move_view_drag(document, *pointer_id, *x, *y, *modifiers, now)? {
                    return Ok(CONSUMED);
                }
                if self.update_active_drag(document, *pointer_id, true, *x, *y, now)? {
                    return Ok(CONSUMED);
                }
                if let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                    && self.text_editor_pointer_drag(document, *pointer_id, *x, *y, shaper)?
                {
                    true
                } else if let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                    && self.document_text_pointer_drag(document, *pointer_id, *x, *y, shaper)?
                {
                    true
                } else if self.update_active_drag(document, *pointer_id, false, *x, *y, now)? {
                    true
                } else {
                    let mut hovered = false;
                    for i in 0..self.capabilities.hover_moves.len() {
                        hovered |= (self.capabilities.hover_moves[i])(
                            self, document, target, *x, *y, now,
                        )?;
                    }
                    hovered || target.is_some()
                }
            }
            PointerPhase::Down if *button == 2 => {
                // A secondary press outside an open popover dismisses
                // it and goes no further, matching the primary press.
                if let Some(popover) = self.capabilities.popover
                    && (popover.dismiss_outside)(self, target)?
                {
                    *landed = None;
                    return Ok(CONSUMED);
                }
                if let Some(menu_bar) = self.capabilities.menu_bar {
                    (menu_bar.cancel_alt)(self);
                }
                self.dismiss_detached_menus(target)?;
                if let Some(target) = target
                    && self.press_pointer_hook(
                        *pointer_id,
                        target,
                        *x,
                        *y,
                        modifiers.shift,
                        *button,
                    )?
                {
                    return Ok(CONSUMED);
                }
                if let Some(target) = target {
                    if let Some(prep) = self
                        .behavior(target)
                        .and_then(|b| b.pointer_ops)
                        .and_then(|p| p.prepare_context_menu)
                        && self
                            .world()
                            .accessibility(target)
                            .is_some_and(|a| !a.disabled)
                    {
                        self.focus_node(document, target)?;
                        prep(self, target, *x, *y)?;
                    } else if self.world().text_input(target).is_some()
                        && self
                            .world()
                            .accessibility(target)
                            .is_some_and(|a| !a.disabled)
                    {
                        let unfocused = self.world().focused(document) != Some(target);
                        self.focus_node(document, target)?;
                        if unfocused
                            && let Some(select_entry) = self
                                .behavior(target)
                                .and_then(|b| b.pointer_ops)
                                .and_then(|p| p.select_on_focus_entry)
                            && select_entry(self, document, target, Some((*x, *y)))?
                        {
                            self.text_edit.text_pointer_drag = None;
                            self.text_edit.text_pointer_click = None;
                        } else if self
                            .world()
                            .text_input(target)
                            .is_some_and(|s| s.selection.range().is_empty())
                            && let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                        {
                            self.text_edit.unfocused_press = unfocused.then_some(target);
                            let pressed = self.text_editor_pointer_press(
                                document,
                                target,
                                *pointer_id,
                                *x,
                                *y,
                                false,
                                false,
                                now,
                                shaper,
                            );
                            self.text_edit.unfocused_press = None;
                            pressed?;
                            self.text_editor_pointer_release(*pointer_id);
                        }
                    }
                }
                self.secondary_press_at(document, *x, *y)?.is_some()
            }
            PointerPhase::Up if *button == 2 => {
                self.release_pointer_hook(document, *pointer_id, *x, *y, modifiers.shift, 2)?
            }
            PointerPhase::Down if (*is_primary && *button == 0) || *button == 1 => {
                if let Some(popover) = self.capabilities.popover
                    && (popover.dismiss_outside)(self, target)?
                {
                    // Consume the press that dismissed the popover.
                    // Activation needs a press recorded here to match
                    // on release, so skipping it also stops this click
                    // from reaching the control underneath.
                    *landed = None;
                    return Ok(CONSUMED);
                }
                if let Some(menu_bar) = self.capabilities.menu_bar {
                    (menu_bar.cancel_alt)(self);
                }
                self.dismiss_detached_menus(target)?;
                // Scrollbars overlay content, so they claim the press
                // before the node underneath sees it.
                if *button == 0
                    && !activation_click
                    && let Some(target) = target
                    && let Some(corner_press) = self
                        .behavior(target)
                        .and_then(|b| b.pointer_ops)
                        .and_then(|p| p.corner_press)
                    && corner_press(self, *pointer_id, target, *x, *y)?
                {
                    return Ok(CONSUMED);
                }
                if *button == 0
                    && let Some((view, axis)) = lane
                    && let Some(scrollbar) = self.capabilities.scrollbar
                    && (scrollbar.begin_drag)(self, *pointer_id, view, axis, *x, *y)?
                {
                    return Ok(CONSUMED);
                }
                // A press a blocking overlay swallowed reaches nothing
                // underneath, not even a handle within slop of it.
                let reachable = !(overlay.prevent_default && target.is_none());
                let [split_handle, dock_handle, workspace_handle] = if reachable {
                    self.reachable_handle_near(document, *x, *y, target)
                } else {
                    [None; 3]
                };
                let dock_source = self.capabilities.dock.and_then(|dock| {
                    target
                        .filter(|id| (dock.is_item_source)(self, *id))
                        .or_else(|| {
                            reachable
                                .then(|| (dock.tab_strip_near_hit)(self, document, *x, *y, target))
                                .flatten()
                        })
                });
                let hit = dock_handle.or(split_handle).or(workspace_handle).or(target);
                let focus_target = hit.and_then(|id| nearest_focusable(self, id));
                let unfocused =
                    focus_target.filter(|id| self.world().focused(document) != Some(*id));
                // A toast above an open modal takes the press; focus stays in
                // the modal, as Tab does.
                if !hit.is_some_and(|id| {
                    self.preserves_hover_card_editor_focus(id)
                        || self.press_keeps_modal_focus(document, id)
                        || self.is_menu_bar_node(id)
                }) {
                    if let Some(focus) = focus_target {
                        // The press already sits on its target. Revealing it
                        // would scroll the content out from under the release.
                        self.focus_node_in_place(document, focus)?;
                    } else {
                        self.clear_focus(document)?;
                    }
                }
                if *button == 0 && !activation_click {
                    if let Some(focus) = focus_target
                        && unfocused == Some(focus)
                        && !modifiers.shift
                        && !modifiers.alt
                        && let Some(select_entry) = self
                            .behavior(focus)
                            .and_then(|b| b.pointer_ops)
                            .and_then(|p| p.select_on_focus_entry)
                        && select_entry(self, document, focus, Some((*x, *y)))?
                    {
                        self.text_edit.text_pointer_drag = None;
                        self.text_edit.text_pointer_click = None;
                        number_selected_all = true;
                        self.clear_document_text_selection(document);
                    } else if let Some(shaper) = reborrow_text_shaper(&mut text_shaper) {
                        let editor = if let Some(focus) = focus_target {
                            self.text_edit.unfocused_press = unfocused;
                            let pressed = self.text_editor_pointer_press(
                                document,
                                focus,
                                *pointer_id,
                                *x,
                                *y,
                                modifiers.shift,
                                modifiers.alt,
                                now,
                                shaper,
                            );
                            self.text_edit.unfocused_press = None;
                            pressed?
                        } else {
                            false
                        };
                        if editor {
                            self.clear_document_text_selection(document);
                        } else {
                            self.document_text_pointer_press(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?;
                        }
                    }
                }
                if let Some(target) = hit {
                    if !activation_click
                        && self.press_pointer_hook(
                            *pointer_id,
                            target,
                            *x,
                            *y,
                            modifiers.shift,
                            *button,
                        )?
                    {
                        return Ok(CONSUMED);
                    }
                    if *button == 0
                        && !activation_click
                        && let Some(rich_text_pointer) = self.capabilities.rich_text_pointer
                        && rich_text_pointer(self, document, *pointer_id, target, *x, *y)?
                    {
                        return Ok(CONSUMED);
                    }
                    let pointer_ops = self.behavior(target).and_then(|b| b.pointer_ops);
                    if let Some(ops) = pointer_ops.filter(|ops| ops.early_press)
                        && (!*activation_click || !ops.skip_on_activation_click)
                        && let Some(press) = ops.press
                    {
                        let press_button = *button;
                        let handled =
                            press(self, document, target, *pointer_id, press_button, *x, *y)?;
                        if !ops.skips_press_pointer && handled {
                            return Ok(CONSUMED);
                        }
                    } else if *button == 0
                        && let Some(reorder) = self.capabilities.reorder_list
                        && (reorder.begin_pointer)(self, document, *pointer_id, target, *x, *y)?
                    {
                    } else if *button == 0
                        && let Some(dock) = self.capabilities.dock
                        && (dock.is_handle)(self, target)
                    {
                        (dock.begin_split)(self, document, *pointer_id, target, *x, *y)?;
                    } else if *button == 0
                        && let Some(split) = self.capabilities.split_pane
                        && (split.is_handle)(self, target)
                    {
                        (split.begin_resize)(self, document, *pointer_id, target, *x, *y)?;
                    } else if *button == 0
                        && let Some(ws) = self.capabilities.workspace
                        && (ws.is_resize_handle)(self, target)
                    {
                        (ws.begin_resize)(self, document, *pointer_id, target, *x, *y, now)?;
                    } else if *button == 0 {
                        if let Some(source) = dock_source
                            && let Some(dock) = self.capabilities.dock
                        {
                            (dock.begin_item)(self, document, *pointer_id, source, *x, *y)?;
                        } else {
                            self.press_pointer(document, *pointer_id, target)?;
                            if let Some(ops) = pointer_ops.filter(|ops| !ops.early_press)
                                && (!*activation_click || !ops.skip_on_activation_click)
                                && let Some(press) = ops.press
                            {
                                if press(self, document, target, *pointer_id, *button, *x, *y)?
                                    && ops.releases_pointer_on_press
                                {
                                    self.release_pointer(document, *pointer_id);
                                }
                            } else if let Some(range_span) = self.capabilities.range_span
                                && (range_span.target)(self, target).is_some()
                            {
                                (range_span.begin_drag)(
                                    self,
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if let Some(curve) = self.capabilities.curve_field
                                && (curve.target)(self, target).is_some()
                            {
                                (curve.begin_drag)(self, document, *pointer_id, target, *x, *y)?;
                            }
                        }
                    }
                    true
                } else {
                    false
                }
            }
            PointerPhase::Up if (*is_primary && *button == 0) || *button == 1 => {
                if self.end_view_drag(document, *pointer_id, *x, *y, *modifiers, false)? {
                    return Ok(CONSUMED);
                }
                if self.end_active_drag(document, *pointer_id, true, *x, *y, now, false)? {
                    return Ok(CONSUMED);
                }
                if self.release_pointer_hook(
                    document,
                    *pointer_id,
                    *x,
                    *y,
                    modifiers.shift,
                    *button,
                )? {
                    return Ok(CONSUMED);
                }
                // 拖拽移动选中的落点执行先于通用释放清理：active 态
                // 落文本、pending 态回落为点击。
                let mut drop_handled = false;
                if let Some(shaper) = reborrow_text_shaper(&mut text_shaper) {
                    drop_handled =
                        self.text_editor_selection_drop(document, *pointer_id, *x, *y, shaper)?;
                }
                self.text_editor_pointer_release(*pointer_id);
                self.document_text_pointer_release(*pointer_id);
                if drop_handled {
                    self.release_pointer(document, *pointer_id);
                    return Ok(CONSUMED);
                }
                if self.end_active_drag(document, *pointer_id, false, *x, *y, now, false)? {
                    self.release_pointer(document, *pointer_id);
                    return Ok(CONSUMED);
                }
                let pressed = self.release_pointer(document, *pointer_id);
                if let Some(pressed) = pressed {
                    if Some(pressed) == target && !*activation_click {
                        self.activate_node_at(pressed, *x, *y)?;
                    }
                    true
                } else {
                    false
                }
            }
            PointerPhase::Cancel => {
                let pre_gesture_drag =
                    self.end_active_drag(document, *pointer_id, true, *x, *y, now, true)?;
                self.text_editor_pointer_release(*pointer_id);
                self.document_text_pointer_release(*pointer_id);
                let drag = self.end_active_drag(document, *pointer_id, false, *x, *y, now, true)?;
                let view_drag =
                    self.end_view_drag(document, *pointer_id, *x, *y, *modifiers, true)?;
                let pressed = self.release_pointer(document, *pointer_id).is_some();
                // Before the hover clears, so the hooked node hears it leave.
                let hooked = self.cancel_pointer_hook(document, *pointer_id)?;
                self.set_pointer_hover_at(document, *pointer_id, None, now)?;
                let mut hover_cleared = false;
                for i in 0..self.capabilities.hover_clears.len() {
                    hover_cleared |= (self.capabilities.hover_clears[i])(self, document)?;
                }
                pre_gesture_drag || drag || hover_cleared || pressed || hooked || view_drag
            }
            _ => false,
        };
        // Reached only by a press no control took for itself.
        if *phase == PointerPhase::Down
            && *is_primary
            && *button == 0
            && !activation_click
            && !number_selected_all
        {
            self.arm_view_drag(document, *pointer_id, pointer.pointer_type, *x, *y);
        }
        let handled = overlay.prevent_default || component_handled;
        Ok(InputDisposition {
            handled,
            prevent_default: handled,
        })
    }

    /// One wheel delta. `landed` receives the node under the pointer.
    pub(super) fn dispatch_wheel(
        &mut self,
        document: DocumentId,
        wheel: &WheelInput,
        landed: &mut Option<StableNodeId>,
    ) -> Result<InputDisposition, FrameworkError> {
        let WheelInput {
            x,
            y,
            delta_x,
            delta_y,
            ..
        } = wheel;
        let line_delta = &(wheel.unit == WheelUnit::Lines);
        // Deltas arrive oriented: a host whose platform leaves Shift+wheel
        // vertical turns it horizontal before it routes.
        let (dx, dy) = (*delta_x, *delta_y);
        let scale = if *line_delta { LINE_SCROLL_EXTENT } else { 1.0 };
        let delta = ScrollOffset {
            x: -dx * scale,
            y: -dy * scale,
        };
        let overlay = self.route_overlay_pointer(
            document,
            wheel.pointer_id.0,
            OverlayPointerPhase::Wheel,
            *x,
            *y,
        )?;
        *landed = overlay.target;
        if let Some(target) = overlay.target
            && let Some(hooks) = self.pointer_hooks(target)
            && let Some(pointer) = self.node_pointer(
                target,
                wheel.pointer_id.0,
                *x,
                *y,
                wheel.modifiers.shift,
                -1,
            )
            && hooks.wheel(
                self,
                target,
                crate::NodeWheel {
                    pointer_id: wheel.pointer_id.0,
                    x: pointer.x,
                    y: pointer.y,
                    delta_x: dx * scale,
                    delta_y: dy * scale,
                    ctrl: wheel.modifiers.control || wheel.modifiers.meta,
                    modifiers: wheel.modifiers,
                },
            )?
        {
            return Ok(CONSUMED);
        }
        if let Some(target) = overlay.target
            && let Some(wheel) = self
                .behavior(target)
                .and_then(|b| b.pointer_ops)
                .and_then(|p| p.wheel_pre_overlay)
            && wheel(self, target, *x, *y, dy)?
        {
            return Ok(CONSUMED);
        }
        // 上时滚轮滚动浮层自身（按行，方向跟随滚轮），不再落到
        // 编辑器或文档滚动。
        let overlay_rows = if *delta_y > 0.0 {
            1isize
        } else if *delta_y < 0.0 {
            -1
        } else {
            0
        };
        if overlay_rows != 0 && self.scroll_text_overlay_at(document, *x, *y, overlay_rows)? {
            return Ok(CONSUMED);
        }
        let scrolled = if overlay.prevent_default {
            overlay
                .target
                .map(|target| {
                    self.wheel_scroll_from(document, Some(target), delta, *line_delta, true)
                })
                .transpose()?
                .flatten()
                .is_some()
        } else if let Some(target) = overlay.target
            && let Some(wheel) = self
                .behavior(target)
                .and_then(|b| b.pointer_ops)
                .and_then(|p| p.wheel_post_overlay)
        {
            wheel(self, document, target, *x, *y, dy, *line_delta)?
        } else {
            self.wheel_scroll_from(document, overlay.target, delta, *line_delta, false)?
                .is_some()
        };
        let handled = overlay.prevent_default || scrolled;
        Ok(InputDisposition {
            handled,
            prevent_default: handled,
        })
    }
    /// One key transition or committed text through the key chain: blocking
    /// overlays, the application's key policy, a focused terminal, the
    /// focused editor, focus traversal, then the focused control. Clipboard
    /// shortcuts run at their place in that chain, after anything above them
    /// had its chance at the key.
    pub(super) fn dispatch_keystroke(
        &mut self,
        document: DocumentId,
        stroke: KeyStroke<'_>,
        services: &mut dyn HostServices,
        mut text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputDisposition, FrameworkError> {
        self.world
            .note_input_modality(document, crate::InputModality::Keyboard);
        let KeyStroke {
            pressed,
            key,
            text,
            repeat,
            modifiers,
            canonical,
            ..
        } = stroke;
        // Composition owns key transitions, including Enter and Escape,
        // before application shortcuts and overlay dismissal see them.
        if !stroke.is_text() && self.editor_composing(document) {
            return Ok(CONSUMED);
        }
        // A list drag holding the pointer takes the first Escape press: the
        // gesture is the innermost thing the key can cancel.
        if pressed
            && key == "Escape"
            && !repeat
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
            && let Some(rl) = self.capabilities.reorder_list
            && (rl.cancel_drag)(self, document)?
        {
            return Ok(CONSUMED);
        }
        let keyboard_barrier = self.has_blocking_runtime_overlay(document);
        // An open menu bar, or one the keyboard holds, takes its keys first.
        if !keyboard_barrier
            && !stroke.is_text()
            && let Some(menu_bar) = self.capabilities.menu_bar
            && (menu_bar.route_key)(self, document, key, pressed, repeat, modifiers)?
        {
            return Ok(CONSUMED);
        }
        if pressed && !modifiers.alt && !modifiers.control && !modifiers.meta {
            let overlay_key = match key {
                "Escape" if !repeat => Some(OverlayKey::Escape),
                "Tab" => Some(OverlayKey::Tab {
                    reverse: modifiers.shift,
                }),
                _ => None,
            };
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !modifiers.shift
                && self.dismiss_focused_field_options(document)?
            {
                return Ok(CONSUMED);
            }
            if let Some(key) = overlay_key
                && self.route_overlay_key(document, key)?
            {
                return Ok(CONSUMED);
            }
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && let Some(popover) = self.capabilities.popover
                && (popover.dismiss_on_escape)(self)?
            {
                return Ok(CONSUMED);
            }
            // overlay 未消费的 Esc：先取消拖拽移动选中，再结束 snippet 会
            // 话，签名帮助在场时消费但不关补全（两段式：宿主随后撤签名），
            // 再关闭补全弹层，最后塌缩多光标到主光标。都只在聚焦多行
            // 编辑器且状态存在时消费事件，否则穿透给宿主（首次按下才生
            // 效，repeat 不消费）。
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !repeat
                && (self.cancel_focused_text_selection_drag(document)
                    || self.cancel_focused_text_snippet(document)?
                    || self.focused_text_signature_showing(document)
                    || self.dismiss_focused_text_completion(document)?
                    || self.collapse_focused_text_selections(document)?)
            {
                return Ok(CONSUMED);
            }
            // Open menus (context menu, action menu, anchored menu) claim
            // arrows and confirmation before a text field or focused control.
            for i in 0..self.capabilities.open_menu_navigators.len() {
                let nav = self.capabilities.open_menu_navigators[i];
                if nav(self, document, key, repeat)? {
                    return Ok(CONSUMED);
                }
            }
        }
        // The application's key policy sees keys, not text.
        if !stroke.is_text()
            && !keyboard_barrier
            && self.dispatch_focused_key(
                document,
                canonical.expect("a key transition keeps its canonical key"),
            )
        {
            return Ok(CONSUMED);
        }
        // Past the application's policy, only presses act.
        if !pressed {
            return Ok(InputDisposition {
                handled: false,
                prevent_default: keyboard_barrier,
            });
        }
        if !keyboard_barrier
            && self
                .capabilities
                .terminal
                .is_some_and(|term| (term.focused_node)(self, document).is_some())
        {
            return self.terminal_keystroke(document, stroke, services);
        }
        if !keyboard_barrier
            && let Some(rich_editor) = self.capabilities.rich_editor
            && let Some(handled) =
                (rich_editor.key)(self, document, key, text, modifiers, services)?
            && handled
        {
            return Ok(CONSUMED);
        }
        // A tree's in-place rename ends with Enter (committed) or Escape
        // before the input sees either.
        if matches!(key, "Enter" | "Escape")
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
            && let Some(tree) = self.capabilities.tree
            && (tree.end_rename)(self, document, key == "Enter")?
        {
            return Ok(CONSUMED);
        }
        // Focused plain text editors own their editing keys (caret moves,
        // selection, deletion, indent, pairing) before any generic routing.
        if self.text_editor_key(
            document,
            key,
            text,
            modifiers,
            reborrow_text_shaper(&mut text_shaper),
        )? {
            return Ok(CONSUMED);
        }
        if key == "Tab"
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
            && self.navigate_sequential_focus(document, modifiers.shift)?
        {
            return Ok(CONSUMED);
        }
        if !modifiers.alt && !modifiers.control && !modifiers.meta && !modifiers.shift {
            let segmented_navigation = match key {
                "ArrowLeft" => Some(RovingFocusIntent::Previous),
                "ArrowRight" => Some(RovingFocusIntent::Next),
                "Home" => Some(RovingFocusIntent::First),
                "End" => Some(RovingFocusIntent::Last),
                _ => None,
            };
            if let Some(intent) = segmented_navigation
                && let Some(nav) = self.capabilities.segmented_navigate
                && nav(self, document, intent)?
            {
                return Ok(CONSUMED);
            }
            if matches!(key, " " | "Space" | "Enter")
                && let Some(target) = self.world().focused(document)
                && self
                    .behavior(target)
                    .and_then(|b| b.pointer_ops)
                    .is_some_and(|p| p.is_segmented_option)
            {
                if !repeat {
                    self.activate_node(target)?;
                }
                return Ok(CONSUMED);
            }
        }
        let handled = if !modifiers.alt
            && (modifiers.control || modifiers.meta)
            && key.eq_ignore_ascii_case("z")
        {
            // Undo/redo is the one editing shortcut that carries Shift, so
            // it is matched before the clipboard arm excludes it.
            if modifiers.shift {
                self.redo_focused_text(document)?
            } else {
                self.undo_focused_text(document)?
            }
        } else if !modifiers.alt && !modifiers.shift {
            let primary = modifiers.control || modifiers.meta;
            if primary && self.dispatch_clipboard_shortcut(document, key, services)? {
                return Ok(CONSUMED);
            }
            if self.focused_control_key(document, key, primary)? {
                return Ok(CONSUMED);
            }
            let navigation = match (key, primary) {
                ("ArrowUp", false) => Some(TableNavigation::PreviousRow),
                ("ArrowDown", false) => Some(TableNavigation::NextRow),
                ("ArrowLeft", false) => Some(TableNavigation::PreviousColumn),
                ("ArrowRight", false) => Some(TableNavigation::NextColumn),
                ("Home", false) => Some(TableNavigation::RowStart),
                ("End", false) => Some(TableNavigation::RowEnd),
                ("Home", true) => Some(TableNavigation::FirstRow),
                ("End", true) => Some(TableNavigation::LastRow),
                ("PageUp", false) => Some(TableNavigation::PageUp),
                ("PageDown", false) => Some(TableNavigation::PageDown),
                _ => None,
            };
            if let Some(navigation) = navigation {
                match self.capabilities.data_table {
                    Some(t) => (t.navigate)(self, document, navigation, TABLE_PAGE_ROWS)?,
                    None => false,
                }
            } else if primary {
                // Ctrl with ArrowUp / ArrowDown, Space or A in a tree or a table
                let tree_handled = match self.capabilities.tree {
                    Some(tree) => (tree.focused_key)(
                        self,
                        document,
                        key,
                        nana_ui_core::SelectionModifiers::TOGGLE,
                    )?,
                    None => false,
                };
                tree_handled
                    || match self.capabilities.data_table {
                        Some(t) => (t.selection_key)(
                            self,
                            document,
                            key,
                            nana_ui_core::SelectionModifiers::TOGGLE,
                        )?,
                        None => false,
                    }
            } else {
                match key {
                    "Backspace" => self.delete_focused_text_backward(document)?,
                    _ => match text {
                        Some(text) => self.replace_focused_text(document, text)?,
                        None => false,
                    },
                }
            }
        } else if modifiers.shift && !modifiers.alt && !modifiers.control && !modifiers.meta {
            // Shift with an arrow extends a tree's or a table's selection
            let tree_handled = match self.capabilities.tree {
                Some(tree) => (tree.focused_key)(
                    self,
                    document,
                    key,
                    nana_ui_core::SelectionModifiers::EXTEND,
                )?,
                None => false,
            };
            tree_handled
                || match self.capabilities.data_table {
                    Some(t) => (t.selection_key)(
                        self,
                        document,
                        key,
                        nana_ui_core::SelectionModifiers::EXTEND,
                    )?,
                    None => false,
                }
        } else if modifiers.alt && !modifiers.shift && !modifiers.control && !modifiers.meta {
            // Alt with an arrow moves a reorder list's row
            match self.capabilities.reorder_list {
                Some(rl) => (rl.focused_key)(self, document, key)?,
                None => false,
            }
        } else {
            false
        };
        // Last, once the focused control has passed on the key: the arrows
        // of the roving group the focused item is in, then the keyboard's
        // context-menu request. A group's own axis wins over a popup
        // trigger's arrows, as in a vertical menu whose item opens a submenu.
        let handled = handled
            || (!modifiers.alt
                && !modifiers.control
                && !modifiers.meta
                && !modifiers.shift
                && self.roving_focus_key(document, key)?)
            || self.keyboard_menu_key(document, key, repeat, modifiers)?;
        Ok(InputDisposition {
            handled,
            prevent_default: handled || keyboard_barrier,
        })
    }

    /// Keys a focused range, pad, graph, split, number field, palette,
    /// select, dropdown, tree, timeline or activatable control acts on. `primary` is
    /// Ctrl or Cmd held; none of these take it.
    fn focused_control_key(
        &mut self,
        document: DocumentId,
        key: &str,
        primary: bool,
    ) -> Result<bool, FrameworkError> {
        if primary {
            return Ok(false);
        }
        if key == "F2"
            && let Some(tree) = self.capabilities.tree
            && (tree.focused_key)(self, document, key, nana_ui_core::SelectionModifiers::NONE)?
        {
            return Ok(true);
        }
        if matches!(key, "Delete" | "Backspace")
            && let Some(delete_clip) = self.capabilities.timeline_clip_delete
            && delete_clip(self, document)?
        {
            return Ok(true);
        }
        // Across a curve the side arrows walk its points; the others step
        // the focused one as on a vertical slider.
        if matches!(key, "ArrowLeft" | "ArrowRight")
            && let Some(curve) = self.capabilities.curve_field
            && (curve.focus_neighbour)(self, document, key == "ArrowRight")?
        {
            return Ok(true);
        }
        let range_adjustment = match key {
            "ArrowLeft" | "ArrowDown" => Some(RangeAdjustment::Decrement),
            "ArrowRight" | "ArrowUp" => Some(RangeAdjustment::Increment),
            "PageDown" => Some(RangeAdjustment::PageDecrement),
            "PageUp" => Some(RangeAdjustment::PageIncrement),
            "Home" => Some(RangeAdjustment::Minimum),
            "End" => Some(RangeAdjustment::Maximum),
            _ => None,
        };
        if let Some(adjustment) = range_adjustment {
            for i in 0..self.capabilities.step_adjusters.len() {
                let adjust = self.capabilities.step_adjusters[i];
                if adjust(self, document, adjustment)? {
                    return Ok(true);
                }
            }
        }
        let xy_adjustment = match key {
            "ArrowLeft" => Some(XYPadAdjustment::Left),
            "ArrowRight" => Some(XYPadAdjustment::Right),
            "ArrowUp" => Some(XYPadAdjustment::Up),
            "ArrowDown" => Some(XYPadAdjustment::Down),
            _ => None,
        };
        if let Some(adjustment) = xy_adjustment
            && let Some(adjust) = self.capabilities.adjust_xy_pad
            && adjust(self, document, adjustment)?
        {
            return Ok(true);
        }
        if let Some(step_viewer) = self.capabilities.image_viewer_step {
            let forward = match key {
                "ArrowLeft" => Some(false),
                "ArrowRight" => Some(true),
                _ => None,
            };
            if let Some(forward) = forward
                && step_viewer(self, document, forward)?
            {
                return Ok(true);
            }
        }
        if let Some(graph_key) = self.capabilities.graph_canvas_key
            && graph_key(self, document, key)?
        {
            return Ok(true);
        }
        let split_direction = match key {
            "ArrowLeft" | "ArrowUp" => Some(-1.0),
            "ArrowRight" | "ArrowDown" => Some(1.0),
            _ => None,
        };
        if let Some(direction) = split_direction {
            if let Some(sp) = self.capabilities.split_pane
                && (sp.adjust_focused)(self, document, direction)?
            {
                return Ok(true);
            }
            if let Some(dock) = self.capabilities.dock
                && (dock.adjust_focused_split)(self, document, direction)?
            {
                return Ok(true);
            }
        }
        // A numeric field's step and commit keys are routed with its other
        // editing keys in `text_editor_key`; Escape is not an editing key and
        // reverts the draft here.
        if key == "Escape" {
            let mut cancelled = false;
            for i in 0..self.capabilities.edit_cancellers.len() {
                let cancel = self.capabilities.edit_cancellers[i];
                if cancel(self, document)? {
                    cancelled = true;
                    break;
                }
            }
            let mut ended = false;
            for i in 0..self.capabilities.edit_enders.len() {
                let end = self.capabilities.edit_enders[i];
                ended |= end(self, document)?;
            }
            if cancelled || ended {
                return Ok(true);
            }
        }
        let palette_nav = match key {
            "ArrowUp" => Some(crate::ActionPickerNavigation::Previous),
            "ArrowDown" => Some(crate::ActionPickerNavigation::Next),
            "Home" => Some(crate::ActionPickerNavigation::First),
            "End" => Some(crate::ActionPickerNavigation::Last),
            "Enter" => Some(crate::ActionPickerNavigation::Confirm),
            "Escape" => Some(crate::ActionPickerNavigation::Dismiss),
            _ => None,
        };
        if let Some(navigation) = palette_nav
            && let Some(nav) = self.capabilities.command_palette_navigate
            && nav(self, document, navigation)?
        {
            return Ok(true);
        }
        let select_delta = match key {
            "ArrowUp" => Some(-1),
            "ArrowDown" => Some(1),
            _ => None,
        };
        if let Some(delta) = select_delta {
            for i in 0..self.capabilities.choice_hooks.len() {
                let hook = self.capabilities.choice_hooks[i];
                if (hook.adjust)(self, document, delta)? {
                    return Ok(true);
                }
            }
        }
        let is_enter = key == "Enter";
        let is_space = matches!(key, " " | "Space");
        if is_enter || is_space {
            for i in 0..self.capabilities.choice_hooks.len() {
                let hook = self.capabilities.choice_hooks[i];
                if (is_enter || hook.commit_on_space) && (hook.commit)(self, document)? {
                    return Ok(true);
                }
            }
        }
        let tree_nav = match key {
            "ArrowUp" => Some(crate::TreeNavigation::Previous),
            "ArrowDown" => Some(crate::TreeNavigation::Next),
            "Home" => Some(crate::TreeNavigation::First),
            "End" => Some(crate::TreeNavigation::Last),
            "ArrowLeft" => Some(crate::TreeNavigation::Parent),
            "ArrowRight" => Some(crate::TreeNavigation::Child),
            "Enter" => Some(crate::TreeNavigation::Activate),
            " " | "Space" => Some(crate::TreeNavigation::Toggle),
            _ => None,
        };
        if let Some(navigation) = tree_nav
            && let Some(tree) = self.capabilities.tree
            && (tree.navigate)(self, document, navigation)?
        {
            return Ok(true);
        }
        if matches!(key, " " | "Space" | "Enter")
            && let Some(target) = self.world().focused(document)
            && self.activate_node(target)?
        {
            return Ok(true);
        }
        Ok(false)
    }

    /// A key or text for the focused terminal. Copy and paste go through
    /// the host clipboard; everything else becomes terminal input. The event
    /// is consumed either way, but counts as handled only when it sent
    /// something, so a printable key leaves its text to arrive on its own.
    fn terminal_keystroke(
        &mut self,
        document: DocumentId,
        stroke: KeyStroke<'_>,
        services: &mut dyn HostServices,
    ) -> Result<InputDisposition, FrameworkError> {
        let Some(term) = self.capabilities.terminal else {
            return Ok(InputDisposition {
                handled: false,
                prevent_default: true,
            });
        };
        let KeyStroke {
            key,
            text,
            modifiers,
            ..
        } = stroke;
        let clipboard_chord = modifiers.control && modifiers.shift || modifiers.meta;
        let handled = if clipboard_chord && key.eq_ignore_ascii_case("c") {
            let copied = (term.selected_text)(self, document).filter(|text| !text.is_empty());
            copied.is_some_and(|text| services.write_clipboard(&text).is_ok())
        } else if clipboard_chord && key.eq_ignore_ascii_case("v") {
            match services.read_clipboard() {
                Ok(Some(text)) if !text.is_empty() => (term.paste)(self, document, &text)?,
                _ => false,
            }
        } else {
            (term.key)(
                self,
                document,
                key,
                text,
                modifiers.control,
                modifiers.alt,
                modifiers.shift,
            )?
        };
        Ok(InputDisposition {
            handled,
            prevent_default: true,
        })
    }

    /// Ctrl/Cmd + A / C / X / V on the focused editor, or a copy of the
    /// document selection.
    ///
    /// The Runtime owns what is selected and what an edit does; this only
    /// moves text between that selection and the host clipboard. A copy with
    /// nothing selected leaves the clipboard alone rather than clearing it,
    /// a cut deletes only once the clipboard took the text, and a paste of
    /// an empty clipboard is not an edit.
    pub(super) fn dispatch_clipboard_shortcut(
        &mut self,
        document: DocumentId,
        key: &str,
        services: &mut dyn HostServices,
    ) -> Result<bool, FrameworkError> {
        if key.eq_ignore_ascii_case("a") {
            return self.select_all_focused_text(document);
        }
        if key.eq_ignore_ascii_case("c") {
            let text = self
                .focused_selected_text(document)
                .or_else(|| self.document_selected_text(document));
            return Ok(text
                .is_some_and(|text| !text.is_empty() && services.write_clipboard(&text).is_ok()));
        }
        if key.eq_ignore_ascii_case("x") {
            let Some(text) = self.focused_selected_text(document) else {
                return Ok(false);
            };
            // A read-only field copies but never loses its text.
            if services.write_clipboard(&text).is_err() {
                return Ok(false);
            }
            self.cut_focused_text(document)?;
            return Ok(true);
        }
        if key.eq_ignore_ascii_case("v") {
            return match services.read_clipboard() {
                Ok(Some(text)) if !text.is_empty() => self.paste_focused_text(document, &text),
                _ => Ok(false),
            };
        }
        Ok(false)
    }

    /// Composition into the focused editor or terminal.
    ///
    /// Retained TextInput/TextArea/SearchDropdown/CommandPalette state is the
    /// only editing authority. A focused editable field, or a blocking
    /// overlay, consumes the event so no second IME path can also apply it.
    ///
    /// Multi-cursor restriction: composition is anchored to the primary
    /// cursor only. While preedit is active the editor paints a single caret
    /// and, on commit, only the primary selection's text is replaced; the
    /// additional cursors survive through offset remapping.
    pub(super) fn dispatch_composition(
        &mut self,
        document: DocumentId,
        composition: &CompositionInput,
    ) -> Result<InputDisposition, FrameworkError> {
        let overlay_blocks = self.has_blocking_runtime_overlay(document);
        if !overlay_blocks
            && let Some(term) = self.capabilities.terminal
            && (term.focused_node)(self, document).is_some()
        {
            match composition {
                CompositionInput::Start => {
                    (term.set_preedit)(self, document, "")?;
                }
                CompositionInput::Update { text, .. } => {
                    (term.set_preedit)(self, document, text)?;
                }
                CompositionInput::Commit(text) => {
                    (term.set_preedit)(self, document, "")?;
                    (term.input_bytes)(self, document, text.as_bytes().to_vec())?;
                }
                CompositionInput::Disabled | CompositionInput::End => {
                    (term.set_preedit)(self, document, "")?;
                }
                CompositionInput::Enabled | CompositionInput::DeleteSurrounding { .. } => {}
            }
            return Ok(CONSUMED);
        }
        if !overlay_blocks
            && let Some(rich_editor) = self.capabilities.rich_editor
            && let Some(handled) = (rich_editor.composition)(self, document, composition)?
        {
            return Ok(InputDisposition {
                handled,
                prevent_default: true,
            });
        }
        let owns_ime = self.editable_focused_text_input(document).is_some();
        let handled = match composition {
            CompositionInput::Enabled => false,
            CompositionInput::Start => self.set_ime_preedit(document, String::new(), None)?,
            CompositionInput::Disabled => {
                let leftover = self
                    .world()
                    .focused_text_input(document)
                    .and_then(|(id, _)| self.world().ime(id).map(|ime| ime.text.to_owned()))
                    .filter(|text| !text.is_empty());
                match leftover {
                    Some(text) => self.commit_ime(document, &text)?,
                    None => self.clear_ime(document)?,
                }
            }
            CompositionInput::End => self.clear_ime(document)?,
            CompositionInput::Update { text, selection } => {
                self.set_ime_preedit(document, text.clone(), *selection)?
            }
            CompositionInput::Commit(text) => self.commit_ime(document, text)?,
            CompositionInput::DeleteSurrounding {
                before_bytes,
                after_bytes,
            } => self.delete_ime_surrounding(document, *before_bytes, *after_bytes)?,
        };
        Ok(InputDisposition {
            handled,
            prevent_default: handled || owns_ime || overlay_blocks,
        })
    }
}

impl AppContext {
    /// Keyboard editing for the focused plain text editor.
    ///
    /// Returns `false` when no plain editor is focused or the key is not an
    /// editing key, so composite-surface navigation and generic activation
    /// keep working unchanged.
    fn text_editor_key(
        &mut self,
        document: DocumentId,
        key: &str,
        text: Option<&str>,
        modifiers: InputModifiers,
        mut shaper: Option<&mut dyn TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.focused_text_editor(document) else {
            // An IME composition hides the editor, but its navigation keys
            // still belong to it: the composition owns them, and they must
            // not reach an enclosing table, tree or select and carry focus
            // out of the field.
            return Ok(caret_intent(key, modifiers).is_some()
                && self.focused_text_editor_composing(document));
        };
        // A numeric field steps on plain ArrowUp/ArrowDown and commits its
        // draft on Enter. Shift+ArrowUp/Down select like any single-line
        // field. The arrows are the field's even when nothing moves (a bound,
        // read-only), so they never fall through to an enclosing table or
        // tree and carry focus out of the field. An Enter that commits
        // nothing falls through, so a dialog or form can still confirm.
        // Alt+ArrowUp/Down move the caret like any single-line field's; Enter
        // commits whatever Shift or Alt accompany it.
        if focused.is_numeric() && !modifiers.control && !modifiers.meta {
            match key {
                "ArrowUp" | "ArrowDown" if !modifiers.shift && !modifiers.alt => {
                    let steps = if key == "ArrowUp" { 1 } else { -1 };
                    if let Some(step) = self.capabilities.number_step {
                        step(self, document, steps)?;
                    }
                    return Ok(true);
                }
                "Enter" => {
                    if self.world().ime(focused.node).is_some() {
                        return Ok(true);
                    }
                    let submitted = match self.capabilities.number_submit {
                        Some(submit) => submit(self, document)?,
                        None => false,
                    };
                    // a scrub field's typing ends with Enter, committed or not
                    let mut ended = false;
                    for i in 0..self.capabilities.edit_enders.len() {
                        let end = self.capabilities.edit_enders[i];
                        ended |= end(self, document)?;
                    }
                    return Ok(submitted || ended);
                }
                _ => {}
            }
        }
        // Arrow keys in the editor's line space (#59): a vertical editor's
        // Up/Down walk its column and Left/Right cross columns, with every
        // modifier below carried along by translating the key itself.
        let key = self.focused_text_line_space_key(document, key);
        if key == "Tab"
            && focused.multiline
            && !modifiers.control
            && !modifiers.meta
            && !modifiers.alt
            && self.advance_focused_text_snippet(document, modifiers.shift)?
        {
            return Ok(true);
        } // 补全弹层激活时，无修饰的 Up/Down/Enter/Tab 由弹层消费：Up/Down
        // 移动候选选中项（编辑器选区不动），Enter/Tab 接受选中项。其余键
        // 穿透正常编辑（打字触发宿主重喂过滤列表）；任何修饰键组合
        // （Cmd+D、Alt+Up、Shift+Up 等）一律穿透。
        if !modifiers.control
            && !modifiers.meta
            && !modifiers.alt
            && !modifiers.shift
            && matches!(key, "ArrowUp" | "ArrowDown" | "Enter" | "Tab")
            && self.focused_text_completion_active(document)
        {
            if matches!(key, "ArrowUp" | "ArrowDown") {
                self.move_focused_text_completion(document, key == "ArrowDown")?;
            } else {
                self.accept_focused_text_completion(document, None)?;
            }
            // 弹层激活期间整键消费（边界上导航无可做也不移动选区）。
            return Ok(true);
        }
        let control = modifiers.control;
        let meta = modifiers.meta;
        // Alt+Cmd/Ctrl+Up/Down adds cursors above/below the selection(s)
        // (Zed-style multi-cursor). Multiline editors own the gesture even
        // when every target already holds a cursor; single-line fields
        // reject multi-cursor entirely and keep plain movement.
        if modifiers.alt
            && (control || meta)
            && focused.multiline
            && matches!(key, "ArrowUp" | "ArrowDown")
        {
            self.add_focused_text_cursor(
                document,
                key == "ArrowUp",
                reborrow_text_shaper(&mut shaper),
            )?;
            return Ok(true);
        }
        // Alt+Up/Down moves the caret's line block; Alt+Shift+Up/Down
        // duplicates it. Multiline editors own the gesture even at the
        // document edge; single-line fields keep plain caret movement.
        if modifiers.alt
            && !control
            && !meta
            && focused.multiline
            && matches!(key, "ArrowUp" | "ArrowDown")
        {
            let direction = if key == "ArrowUp" {
                TextLineDirection::Up
            } else {
                TextLineDirection::Down
            };
            if modifiers.shift {
                self.duplicate_focused_text_lines(document)?;
            } else {
                self.move_focused_text_lines(document, direction)?;
            }
            return Ok(true);
        }
        if let Some(intent) = caret_intent(key, modifiers) {
            return self.move_focused_text_caret(document, intent, modifiers.shift, shaper);
        }
        let delete = match (key, meta, modifiers.alt || control) {
            ("Backspace", false, false) => Some(TextDeleteKind::Backward),
            ("Backspace", _, true) => Some(TextDeleteKind::WordBackward),
            ("Backspace", true, false) => Some(TextDeleteKind::LineStart),
            ("Delete", false, false) => Some(TextDeleteKind::Forward),
            ("Delete", _, true) => Some(TextDeleteKind::WordForward),
            ("Delete", true, false) => Some(TextDeleteKind::LineEnd),
            _ => None,
        };
        if let Some(kind) = delete {
            return self.delete_focused_text(document, kind);
        }
        if control || meta {
            // Cmd/Ctrl+D selects the next occurrence of the primary
            // selection's word (Zed-style multi-cursor). Multiline only.
            // Cmd/Ctrl+Shift+D is deliberately unbound for now; hosts can
            // call `select_focused_text_occurrence(document, true)` for the
            // reverse direction.
            if key.eq_ignore_ascii_case("d") && focused.multiline && !modifiers.shift {
                return self.select_focused_text_occurrence(document, false);
            }
            // Comment toggle is the only code-editing modified key.
            if key == "/" && focused.code_editing.is_some() {
                return self.code_edit_toggle_comment(document);
            }
            // Cmd/Ctrl+Shift+K deletes the caret line.
            if modifiers.shift && key.eq_ignore_ascii_case("k") {
                return self.delete_focused_text_lines(document);
            }
            // Ctrl/Cmd+J joins the touched selection lines.
            if !modifiers.shift && key.eq_ignore_ascii_case("j") {
                return self.join_focused_text_lines(document);
            }
            // Ctrl/Cmd+Shift+U uppercases, Ctrl/Cmd+U lowercases.
            if key.eq_ignore_ascii_case("u") {
                return self.transform_focused_text_case(document, modifiers.shift);
            }
            return Ok(false);
        }
        if key == "Enter" {
            // An IME session can be active between updates with empty preedit.
            // Its confirmation must neither submit nor insert an ordinary newline.
            if self.world().ime(focused.node).is_some() {
                return Ok(true);
            }
            if focused.multiline && !modifiers.shift && self.submit_focused_text_input(document)? {
                return Ok(true);
            }
            if focused.multiline {
                return self.insert_focused_text_newline(document);
            }
            // Single-line fields submit without inserting a newline. IME
            // confirmation remains exclusively owned by the composition path.
            self.submit_focused_text_input(document)?;
            return Ok(true);
        }
        if key == "Tab" && !meta {
            // snippet 会话内 Tab 跳位优先于缩进；无会话时 `Ok(false)`，
            // 代码编辑器的缩进行为接手。
            if focused.multiline && self.advance_focused_text_snippet(document, modifiers.shift)? {
                return Ok(true);
            }
            if focused.code_editing.is_some() {
                return self.code_edit_indent(document, modifiers.shift);
            }
            return Ok(false);
        }
        let Some(text) = text else {
            return Ok(false);
        };
        let mut typed = text.chars();
        if let (Some(single), None) = (typed.next(), typed.next())
            && focused.code_editing.is_some()
            && self.code_edit_typed(document, single)?
        {
            return Ok(true);
        }
        self.replace_focused_text(document, text)
    }
}

/// The caret move an editor's navigation key asks for, in line space.
/// `None` for keys that are not caret navigation.
fn caret_intent(key: &str, modifiers: InputModifiers) -> Option<TextCaretIntent> {
    let (control, meta) = (modifiers.control, modifiers.meta);
    let word_modifier = control || modifiers.alt;
    match key {
        "ArrowLeft" => Some(match (meta, word_modifier) {
            (true, _) => TextCaretIntent::LineStart,
            (_, true) => TextCaretIntent::WordLeft,
            (false, false) => TextCaretIntent::Left,
        }),
        "ArrowRight" => Some(match (meta, word_modifier) {
            (true, _) => TextCaretIntent::LineEnd,
            (_, true) => TextCaretIntent::WordRight,
            _ => TextCaretIntent::Right,
        }),
        "ArrowUp" => Some(if meta {
            TextCaretIntent::DocStart
        } else {
            TextCaretIntent::Up
        }),
        "ArrowDown" => Some(if meta {
            TextCaretIntent::DocEnd
        } else {
            TextCaretIntent::Down
        }),
        "Home" => Some(if control || meta {
            TextCaretIntent::DocStart
        } else {
            TextCaretIntent::LineStart
        }),
        "End" => Some(if control || meta {
            TextCaretIntent::DocEnd
        } else {
            TextCaretIntent::LineEnd
        }),
        "PageUp" if !control && !meta && !modifiers.alt => Some(TextCaretIntent::PageUp),
        "PageDown" if !control && !meta && !modifiers.alt => Some(TextCaretIntent::PageDown),
        _ => None,
    }
}

/// The text a key carries, or `None` when it carries none. Which characters
/// may type is the runtime's call: its typing path refuses the control
/// characters command keys carry.
/// Reborrow the per-dispatch shaper so sequential uses never alias.
pub(super) fn reborrow_text_shaper<'s>(
    shaper: &'s mut Option<&mut dyn TextShaper>,
) -> Option<&'s mut dyn TextShaper> {
    match shaper.as_mut() {
        Some(shaper) => Some(&mut **shaper),
        None => None,
    }
}

fn nearest_focusable(context: &AppContext, mut target: StableNodeId) -> Option<StableNodeId> {
    loop {
        if context
            .world()
            .interaction(target)
            .is_some_and(|interaction| interaction.focusable)
        {
            return Some(target);
        }
        target = context.world().parent_id(target)?;
    }
}
