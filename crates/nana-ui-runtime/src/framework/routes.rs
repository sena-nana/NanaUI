//! Which routed calls ([`RoutedCalls`](super::hooks::RoutedCalls)) each
//! built-in type brings with its first node.
//!
//! The router names none of these methods itself: it asks the table
//! ([`via`]), and a type fills its entries from its `BEHAVIOR`. A program
//! that never creates a terminal links no terminal input handling.

use std::time::Duration;

use super::route;
use crate::{AppContext, Entity, StableNodeId, TypeHooks};

/// Steps the frame loop makes for one type's nodes, each a method so that
/// the loop reaches it through [`via`] and not by naming the type.
impl AppContext {
    pub(crate) fn set_switch_loading_phase(&mut self, target: StableNodeId, phase: f32) -> bool {
        self.update_component(
            Entity::<crate::Switch>::from_stable_id(target),
            |switch, _| {
                switch.loading_phase = phase;
            },
        )
        .is_ok()
    }

    pub(crate) fn set_card_loading_phase(&mut self, target: StableNodeId, phase: f32) -> bool {
        self.update_component(Entity::<crate::Card>::from_stable_id(target), |card, _| {
            card.loading_phase = phase;
        })
        .is_ok()
    }

    pub(crate) fn set_sidebar_section_progress(
        &mut self,
        target: StableNodeId,
        expansion: f32,
    ) -> bool {
        self.update_component(
            Entity::<crate::SidebarSection>::from_stable_id(target),
            |section, _| {
                section.animation_progress = expansion.clamp(0.0, 1.0);
            },
        )
        .is_ok()
    }

    pub(crate) fn set_spinner_phase(&mut self, target: StableNodeId, phase: f32) -> bool {
        self.update_component(
            Entity::<crate::Spinner>::from_stable_id(target),
            |spinner, _| {
                spinner.phase = phase;
            },
        )
        .is_ok()
    }

    /// A toast's timer ran out: it says so once its pause, if any, is over.
    pub(crate) fn expire_toast(&mut self, target: StableNodeId, now: Duration) {
        if self.advance_toast_timer(target, now).unwrap_or(false) {
            let _ = self
                .update_component(Entity::<crate::Toast>::from_stable_id(target), |_, cx| {
                    cx.emit(crate::ToastDismissed)
                });
        }
    }

    /// Align every title bar of `document` with the window's maximized
    /// state, touching only bars that differ.
    pub(crate) fn set_title_bars_maximized(
        &mut self,
        document: crate::DocumentId,
        maximized: bool,
    ) {
        let stale = self
            .world()
            .nodes_of_component(
                document,
                crate::component_descriptors::APP_TITLE_BAR.type_id,
            )
            .filter(|&node| {
                self.read(Entity::<crate::AppTitleBar>::from_stable_id(node), |bar| {
                    bar.maximized != maximized
                })
                .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        for bar in stale {
            let _ = self.update_component(
                Entity::<crate::AppTitleBar>::from_stable_id(bar),
                |bar, _| {
                    bar.maximized = maximized;
                },
            );
        }
    }

    /// What a host calls when its window is maximized or restored: the
    /// title bars of `document` show the matching control. A document
    /// without one has nothing to do.
    pub fn sync_title_bars_maximized(&mut self, document: crate::DocumentId, maximized: bool) {
        super::via!(self.set_title_bars_maximized(document, maximized))
    }

    pub(crate) fn advance_workspace_animation(&mut self, id: StableNodeId, now: Duration) -> bool {
        self.update_component(
            Entity::<crate::Workspace>::from_stable_id(id),
            |workspace, _| workspace.apply(nana_ui_core::WorkspaceMutation::AdvanceAnimations, now),
        )
        .unwrap_or(false)
    }
}

pub(crate) fn terminal(hooks: &mut TypeHooks) {
    route!(hooks;
        set_terminal_preedit, focused_terminal, terminal_pointer, terminal_key,
        terminal_input, paste_terminal, resize_terminal_view,
    );
}

pub(crate) fn xy_pad(hooks: &mut TypeHooks) {
    route!(hooks;
        end_xy_pad_drag, update_xy_pad_drag, begin_xy_pad_drag, is_xy_pad,
        adjust_focused_xy_pad, end_xy_pad_drag_on,
    );
}

pub(crate) fn range_field(hooks: &mut TypeHooks) {
    route!(hooks;
        end_range_drag, update_range_drag_at, begin_range_drag_at, adjust_focused_range, is_range_field, end_focused_range_edit, end_range_drag_on, adjust_range, set_range_value,
    );
}

pub(crate) fn number_input(hooks: &mut TypeHooks) {
    route!(hooks;
        step_focused_number_input, revert_focused_number_input, press_number_stepper, commit_focused_number_input, submit_focused_number_input, cancel_focused_number_input, select_focused_number_on_entry, settle_focused_number_input, step_number_input, set_number_text,
    );
}

pub(crate) fn split_pane(hooks: &mut TypeHooks) {
    route!(hooks;
        end_split_resize, update_split_resize, sync_split_handle_hover_near_hit, sync_split_handle_hover, is_split_handle, begin_split_resize, adjust_focused_split,
    );
}

pub(crate) fn range_span(hooks: &mut TypeHooks) {
    route!(hooks;
        end_range_span_drag, end_focused_range_span_edit, update_range_span_drag_at, range_span_target, begin_range_span_drag_at, adjust_focused_range_span, end_range_span_drag_on, adjust_range_span, set_range_span_thumb,
    );
}

pub(crate) fn curve_field(hooks: &mut TypeHooks) {
    route!(hooks;
        end_curve_drag, update_curve_drag_at, curve_target, begin_curve_drag_at, focus_curve_neighbour, adjust_focused_curve, end_curve_drag_on, adjust_curve_point, set_curve_point,
    );
}

pub(crate) fn scrub_field(hooks: &mut TypeHooks) {
    route!(hooks; end_focused_scrub_edit, adjust_focused_scrub_field, step_scrub_field);
}

pub(crate) fn tree_view(hooks: &mut TypeHooks) {
    route!(hooks;
        focused_tree_key, end_focused_tree_rename, apply_tree_item_action, apply_tree_toggle_action,
    );
}

pub(crate) fn timeline(hooks: &mut TypeHooks) {
    route!(hooks; request_focused_timeline_clip_delete);
}

pub(crate) fn rich_text_editor(hooks: &mut TypeHooks) {
    route!(hooks;
        rich_editor_pointer, rich_editor_key, rich_editor_composition,
        prepare_rich_editor_context_menu, advance_rich_drag, note_rich_editor_scroll,
    );
}

pub(crate) fn segmented(hooks: &mut TypeHooks) {
    route!(hooks;
        navigate_focused_segmented, is_segmented_option_node, focus_segmented_option, segmented_option_is_tab_stop,
    );
}

pub(crate) fn table(hooks: &mut TypeHooks) {
    route!(hooks; navigate_focused_table, table_selection_key, table_column_edge_at);
}

pub(crate) fn text_area(hooks: &mut TypeHooks) {
    route!(hooks;
        end_text_area_resize, update_text_area_resize, begin_text_area_resize, refresh_text_area_compact_layout, note_text_area_scroll, text_editor_minimap_navigate, close_focused_text_atom,
    );
}

pub(crate) fn modal(hooks: &mut TypeHooks) {
    route!(hooks;
        note_modal_open, sync_modal_open_at, announce_close_request, announce_confirm_intent,
    );
}

pub(crate) fn anchored_menu(hooks: &mut TypeHooks) {
    route!(hooks;
        note_anchored_menu_dismissed, navigate_open_anchored_menu,
    );
}

pub(crate) fn sidebar_section(hooks: &mut TypeHooks) {
    route!(hooks; sync_sidebar_section_hover, set_sidebar_section_progress);
}

pub(crate) fn option_list(hooks: &mut TypeHooks) {
    route!(hooks; activate_select_option);
}

pub(crate) fn dropdown(hooks: &mut TypeHooks) {
    route!(hooks; set_dropdown_opened);
}

pub(crate) fn hover_card(hooks: &mut TypeHooks) {
    route!(hooks;
        close_hover_card, sweep_hover_cards, open_hover_card, dismiss_popovers_on_escape,
    );
}

pub(crate) fn popover(hooks: &mut TypeHooks) {
    route!(hooks;
        close_open_popovers_within, dismiss_popovers_outside, dismiss_popovers_on_escape, navigate_open_action_menu,
    );
}

pub(crate) fn toast_stack(hooks: &mut TypeHooks) {
    route!(hooks; reconcile_toast_stacks);
}

pub(crate) fn settings_row(hooks: &mut TypeHooks) {
    route!(hooks; refresh_settings_row_layout, advance_settings_row_flash);
    // A row looks up the placement its containers give it once a flush is
    // done, with or without a group in the program.
    route!(hooks; mark_deferred_settings_groups);
}

pub(crate) fn switch(hooks: &mut TypeHooks) {
    route!(hooks; set_switch_loading_phase);
}

pub(crate) fn card(hooks: &mut TypeHooks) {
    route!(hooks; set_card_loading_phase);
}

pub(crate) fn spinner(hooks: &mut TypeHooks) {
    route!(hooks; set_spinner_phase);
}

pub(crate) fn toast(hooks: &mut TypeHooks) {
    route!(hooks; expire_toast);
}

pub(crate) fn workspace(hooks: &mut TypeHooks) {
    route!(hooks; advance_workspace_animation);
}

pub(crate) fn media_transport(hooks: &mut TypeHooks) {
    route!(hooks; drive_due_auto_overlays, drive_auto_overlay, sync_media_bound_bars);
}

pub(crate) fn scroll_view(hooks: &mut TypeHooks) {
    route!(hooks;
        release_scrollbar_holds, hold_scrollbars, apply_scroll_view_to, sync_scrollbar_hot, scrollbar_target_near_hit, begin_scrollbar_drag, update_scrollbar_drag, end_scrollbar_drag, set_scroll_view_hover,
    );
}

pub(crate) fn menu_bar(hooks: &mut TypeHooks) {
    route!(hooks;
        settle_menu_aims, toggle_menu_bar_keyboard, route_menu_bar_key, cancel_menu_bar_alt,
    );
}

pub(crate) fn tools_row(hooks: &mut TypeHooks) {
    route!(hooks; sync_row_tools);
}

pub(crate) fn select(hooks: &mut TypeHooks) {
    route!(hooks; set_select_opened);
}

pub(crate) fn context_menu(hooks: &mut TypeHooks) {
    route!(hooks;
        activate_context_menu_index, focus_context_menu_index, navigate_open_context_menu, dismiss_context_menu,
    );
}

pub(crate) fn search_dropdown(hooks: &mut TypeHooks) {
    route!(hooks; set_search_dropdown_opened);
}

pub(crate) fn settings_group(hooks: &mut TypeHooks) {
    route!(hooks; mark_deferred_settings_groups);
}

pub(crate) fn icon_button(hooks: &mut TypeHooks) {
    route!(hooks; note_icon_button_tooltip);
}

pub(crate) fn title_bar(hooks: &mut TypeHooks) {
    route!(hooks; set_title_bars_maximized);
}

#[cfg(test)]
mod tests {
    use crate::framework::hooks::RoutedCalls;
    use crate::{AppContext, Button, DocumentId, RangeField, Stack, Text, TextInput};

    /// A call no type brings is a call that never happens: the feature
    /// behind it would be dead for every program.
    #[test]
    fn every_routed_call_is_brought_by_a_type() {
        let routes = include_str!("routes.rs");
        let brought: std::collections::HashSet<&str> = routes
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .collect();
        for name in RoutedCalls::NAMES {
            assert!(brought.contains(name), "no type routes `{name}`");
        }
    }

    /// The controls of a small form bring none of the routed calls, so a
    /// program made of them links none of the handling behind those calls;
    /// a range field brings its own and a number input's, and nothing of a
    /// terminal or a tree.
    #[test]
    fn a_type_brings_its_routed_calls_with_its_first_node() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx.create_component(document, Stack::column(0.0)).unwrap();
        let label = cx
            .create_detached_component(document, Text::new("Name"))
            .unwrap();
        cx.append_child(root, label).unwrap();
        let field = cx
            .create_detached_component(document, TextInput::new(""))
            .unwrap();
        cx.append_child(root, field).unwrap();
        let button = cx
            .create_detached_component(document, Button::new("Save"))
            .unwrap();
        cx.append_child(root, button).unwrap();
        assert_eq!(cx.type_hooks.calls.installed(), Vec::<&str>::new());

        let range = cx
            .create_detached_component(document, RangeField::new(0.5, 0.0, 1.0, 0.1))
            .unwrap();
        cx.append_child(root, range).unwrap();
        let installed = cx.type_hooks.calls.installed();
        for name in [
            "adjust_focused_range",
            "end_range_drag",
            "settle_focused_number_input",
        ] {
            assert!(installed.contains(&name), "a range field brings `{name}`");
        }
        for name in ["terminal_pointer", "focused_tree_key", "end_split_resize"] {
            assert!(!installed.contains(&name), "a range field brought `{name}`");
        }
    }
}
