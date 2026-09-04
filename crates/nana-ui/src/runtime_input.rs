//! Stable platform-input routing for Nana-native Runtime components.

use nana_ui_core::TableNavigation;
use nana_ui_platform::{
    ImeEvent, InputDisposition, InputEvent, PointerPhase, SharedClipboardHost,
    default_shared_clipboard,
};
use nana_ui_runtime::{
    AppContext, DocumentId, FrameworkError, RangeAdjustment, RovingFocusIntent, ScrollOffset,
    StableNodeId, TextCaretIntent, TextDeleteKind, TextLineDirection, TextShaper, XYPadAdjustment,
};
#[cfg(feature = "graph-canvas")]
use nana_ui_runtime::{GraphCanvasAdjustment, GraphPointerButton, GraphScrollDelta};
use nana_ui_runtime::{OverlayKey, OverlayPointerPhase};
use std::sync::OnceLock;
use std::time::Duration;

macro_rules! optional_input {
    ($feature:literal, $call:expr, $absent:expr) => {{
        #[cfg(feature = $feature)]
        {
            $call
        }
        #[cfg(not(feature = $feature))]
        {
            $absent
        }
    }};
}

const DEFAULT_LINE_SCROLL_EXTENT: f32 = 60.0;

/// Pasteboard shared by every adapter that does not carry its own.
///
/// The OS pasteboard is one system resource, and adapters are built per event,
/// so the backend is opened once per process instead of once per keystroke.
fn process_clipboard() -> &'static SharedClipboardHost {
    static CLIPBOARD: OnceLock<SharedClipboardHost> = OnceLock::new();
    CLIPBOARD.get_or_init(default_shared_clipboard)
}

/// Converts renderer-neutral platform input into typed Runtime component
/// actions. It owns no input or component state.
#[derive(Clone)]
pub struct RuntimeInputAdapter {
    pub line_scroll_extent: f32,
    pub table_page_rows: usize,
    clipboard: Option<SharedClipboardHost>,
}

impl std::fmt::Debug for RuntimeInputAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInputAdapter")
            .field("line_scroll_extent", &self.line_scroll_extent)
            .field("table_page_rows", &self.table_page_rows)
            .field("own_clipboard", &self.clipboard.is_some())
            .finish()
    }
}

impl Default for RuntimeInputAdapter {
    fn default() -> Self {
        Self {
            line_scroll_extent: DEFAULT_LINE_SCROLL_EXTENT,
            table_page_rows: 10,
            clipboard: None,
        }
    }
}

impl RuntimeInputAdapter {
    /// Route clipboard shortcuts through `clipboard` instead of the process
    /// pasteboard. Hosts with their own backend, and tests, install one here.
    #[must_use]
    pub fn with_clipboard(mut self, clipboard: SharedClipboardHost) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    fn clipboard(&self) -> &SharedClipboardHost {
        match &self.clipboard {
            Some(clipboard) => clipboard,
            None => process_clipboard(),
        }
    }

    fn read_clipboard(&self) -> Option<String> {
        self.clipboard()
            .lock()
            .ok()?
            .read_text()
            .filter(|text| !text.is_empty())
    }

    fn write_clipboard(&self, text: &str) -> bool {
        self.clipboard()
            .lock()
            .is_ok_and(|mut clipboard| clipboard.write_text(text))
    }

    pub fn dispatch(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        self.dispatch_with_shaper(context, document, event, Duration::ZERO, None)
    }

    /// Dispatch input at the host's monotonic Runtime timestamp. Timed
    /// component behavior such as tooltip delay uses this clock; no component
    /// owns a timer or requests frames while idle.
    pub fn dispatch_at(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
        now: Duration,
    ) -> Result<InputDisposition, FrameworkError> {
        self.dispatch_with_shaper(context, document, event, now, None)
    }

    /// Dispatch input with the host text shaper so caret movement,
    /// click-to-caret, and drag selection follow real text geometry. Hosts
    /// without a shaper fall back to logical-line caret movement.
    pub fn dispatch_with_shaper(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
        now: Duration,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputDisposition, FrameworkError> {
        let mut text_shaper = text_shaper;
        let keyboard_barrier = matches!(event, InputEvent::Keyboard { .. })
            && context.has_blocking_runtime_overlay(document);
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
        {
            let overlay_key = match key.as_str() {
                "Escape" if !repeat => Some(OverlayKey::Escape),
                "Tab" => Some(OverlayKey::Tab {
                    reverse: modifiers.shift,
                }),
                _ => None,
            };
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !modifiers.shift
                && context.dismiss_focused_field_options(document)?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
            if let Some(key) = overlay_key
                && context.route_overlay_key(document, key)?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && context.dismiss_popovers_on_escape()?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
            // overlay 未消费的 Esc：先取消拖拽移动选中，再结束 snippet 会
            // 话，再关闭补全弹层，最后塌缩多光标到主光标。都只在聚焦多行
            // 编辑器且状态存在时消费事件，否则穿透给宿主（首次按下才生
            // 效，repeat 不消费）。
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !repeat
                && (context.cancel_focused_text_selection_drag(document)
                    || context.cancel_focused_text_snippet(document)?
                    || context.dismiss_focused_text_completion(document)?
                    || context.collapse_focused_text_selections(document)?)
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
        }
        // Focused plain text editors own their editing keys (caret moves,
        // selection, deletion, indent, pairing) before any generic routing.
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            text,
            repeat: _,
            modifiers,
            ..
        } = event
            && Self::text_editor_key(
                context,
                document,
                key,
                text.as_deref(),
                *modifiers,
                reborrow_text_shaper(&mut text_shaper),
            )?
        {
            return Ok(InputDisposition {
                prevent_default: true,
            });
        }
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
        {
            if key == "Tab"
                && !modifiers.alt
                && !modifiers.control
                && !modifiers.meta
                && context.navigate_sequential_focus(document, modifiers.shift)?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
            if !modifiers.alt && !modifiers.control && !modifiers.meta && !modifiers.shift {
                let segmented_navigation = match key.as_str() {
                    "ArrowLeft" => Some(RovingFocusIntent::Previous),
                    "ArrowRight" => Some(RovingFocusIntent::Next),
                    "Home" => Some(RovingFocusIntent::First),
                    "End" => Some(RovingFocusIntent::Last),
                    _ => None,
                };
                if let Some(intent) = segmented_navigation
                    && context.navigate_focused_segmented(document, intent)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                if matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.is_segmented_option_node(target)
                {
                    if !repeat {
                        context.activate_node(target)?;
                    }
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
            }
        }
        let handled = match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                x,
                y,
                button,
                is_primary,
                activation_click,
                modifiers,
                ..
            } => {
                let overlay_phase = match phase {
                    PointerPhase::Move => OverlayPointerPhase::Move,
                    PointerPhase::Down if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryDown
                    }
                    PointerPhase::Up if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryUp
                    }
                    PointerPhase::Cancel => OverlayPointerPhase::Cancel,
                    PointerPhase::Down | PointerPhase::Up => OverlayPointerPhase::Move,
                };
                let overlay =
                    context.route_overlay_pointer(document, *pointer_id, overlay_phase, *x, *y)?;
                let target = overlay.target;
                context.set_pointer_location(document, *pointer_id, Some((*x, *y)));
                context.set_pointer_hover_at(document, *pointer_id, target, now)?;
                #[cfg(feature = "graph-canvas")]
                let graph_button = match *button {
                    1 => GraphPointerButton::Middle,
                    _ => GraphPointerButton::Primary,
                };
                let component_handled = match phase {
                    PointerPhase::Move => {
                        if let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                            && context.text_editor_pointer_drag(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?
                        {
                            true
                        } else {
                            context.update_scrollbar_drag(document, *pointer_id, *x, *y)?
                                || context.update_range_drag(document, *pointer_id, *x)?
                                || context.update_xy_pad_drag(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    modifiers.shift,
                                )?
                                || optional_input!(
                                    "graph-canvas",
                                    context.update_graph_canvas_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || optional_input!(
                                    "graph-canvas",
                                    context.update_graph_minimap_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || optional_input!(
                                    "controls",
                                    context.update_reorder_list_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || context.update_split_resize(document, *pointer_id, *x, *y)?
                                || context.update_dock_split_resize(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                )?
                                || context.update_workspace_resize(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    now,
                                )?
                                || context.update_dock_item_drag(document, *pointer_id, *x, *y)?
                                || target
                                    .map(|_target| {
                                        optional_input!(
                                            "graph-canvas",
                                            context.hover_graph_canvas(_target, *x, *y),
                                            Ok::<bool, FrameworkError>(false)
                                        )
                                    })
                                    .transpose()?
                                    .unwrap_or(false)
                                || target
                                    .map(|_target| {
                                        optional_input!(
                                            "calendar",
                                            context.hover_calendar_heatmap(_target, *x, *y),
                                            Ok::<bool, FrameworkError>(false)
                                        )
                                    })
                                    .transpose()?
                                    .unwrap_or(false)
                                || optional_input!(
                                    "calendar",
                                    context.clear_calendar_heatmap_hover(document),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || context.sync_split_handle_hover_near(document, *x, *y, now)?
                                || target.is_some()
                        }
                    }
                    PointerPhase::Down if *button == 2 => {
                        // A secondary press outside an open popover dismisses
                        // it and goes no further, matching the primary press.
                        if context.dismiss_popovers_outside(target)? {
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        context.dismiss_detached_menus(target)?;
                        context.secondary_press_at(document, *x, *y)?.is_some()
                    }
                    PointerPhase::Down if (*is_primary && *button == 0) || *button == 1 => {
                        if context.dismiss_popovers_outside(target)? {
                            // Consume the press that dismissed the popover.
                            // Activation needs a press recorded here to match
                            // on release, so skipping it also stops this click
                            // from reaching the control underneath.
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        context.dismiss_detached_menus(target)?;
                        // Scrollbars overlay content, so they claim the press
                        // before the node underneath sees it.
                        if *button == 0
                            && let Some((view, axis)) =
                                context.scrollbar_target_near(document, *x, *y)
                            && context.begin_scrollbar_drag(*pointer_id, view, axis, *x, *y)?
                        {
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        let split_handle = context.split_handle_near(document, *x, *y);
                        let dock_handle = context.dock_handle_near(document, *x, *y);
                        let workspace_handle = context.workspace_handle_near(document, *x, *y);
                        let dock_source = target
                            .filter(|id| context.is_dock_item_source(*id))
                            .or_else(|| context.dock_tab_strip_near(document, *x, *y));
                        let hit = dock_handle.or(split_handle).or(workspace_handle).or(target);
                        let focus_target = hit.and_then(|id| nearest_focusable(context, id));
                        if let Some(focus) = focus_target {
                            context.focus_node(document, focus)?;
                        } else {
                            context.clear_focus(document)?;
                        }
                        if *button == 0
                            && !activation_click
                            && let Some(focus) = focus_target
                            && let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                        {
                            context.text_editor_pointer_press(
                                document,
                                focus,
                                *pointer_id,
                                *x,
                                *y,
                                modifiers.shift,
                                modifiers.alt,
                                now,
                                shaper,
                            )?;
                        }
                        if let Some(target) = hit {
                            if optional_input!(
                                "graph-canvas",
                                context.is_graph_canvas(target),
                                false
                            ) {
                                optional_input!(
                                    "graph-canvas",
                                    context.begin_graph_canvas_pointer(
                                        document,
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y,
                                        graph_button,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?;
                            } else if optional_input!(
                                "graph-canvas",
                                context.is_graph_minimap(target),
                                false
                            ) {
                                optional_input!(
                                    "graph-canvas",
                                    context.begin_graph_minimap_pointer(
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?;
                            } else if *button == 0
                                && optional_input!(
                                    "controls",
                                    context.begin_reorder_list_pointer(
                                        document,
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                            {
                            } else if context.is_dock_handle(target) && *button == 0 {
                                context.begin_dock_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if context.is_split_handle(target) && *button == 0 {
                                context.begin_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if context.is_workspace_resize_handle(target) && *button == 0 {
                                context.begin_workspace_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                    now,
                                )?;
                            } else if *button == 0 {
                                if let Some(source) = dock_source {
                                    context.begin_dock_item_drag(
                                        document,
                                        *pointer_id,
                                        source,
                                        *x,
                                        *y,
                                    )?;
                                } else {
                                    context.press_pointer(document, *pointer_id, target)?;
                                    if !*activation_click
                                        && context.press_number_stepper(target, *x, *y)?
                                    {
                                        context.release_pointer(document, *pointer_id);
                                    } else if context.is_range_field(target) {
                                        context.begin_range_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                        )?;
                                    } else if context.is_xy_pad(target) {
                                        context.begin_xy_pad_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                            *y,
                                        )?;
                                    }
                                }
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Up if (*is_primary && *button == 0) || *button == 1 => {
                        // 拖拽移动选中的落点执行先于通用释放清理：active 态
                        // 落文本、pending 态回落为点击。
                        let mut drop_handled = false;
                        if let Some(shaper) = reborrow_text_shaper(&mut text_shaper) {
                            drop_handled = context.text_editor_selection_drop(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?;
                        }
                        context.text_editor_pointer_release(*pointer_id);
                        if drop_handled {
                            context.release_pointer(document, *pointer_id);
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        if context.end_scrollbar_drag(document, *pointer_id, false)?
                            || context.end_range_drag(document, *pointer_id, false)?
                            || context.end_xy_pad_drag(document, *pointer_id, false)?
                            || optional_input!(
                                "graph-canvas",
                                context.end_graph_canvas_pointer(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    false,
                                ),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || optional_input!(
                                "graph-canvas",
                                context.end_graph_minimap_pointer(document, *pointer_id, false),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || optional_input!(
                                "controls",
                                context.end_reorder_list_pointer(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    false,
                                ),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || context.end_split_resize(document, *pointer_id, false)?
                            || context.end_dock_split_resize(document, *pointer_id, false)?
                            || context.end_workspace_resize(document, *pointer_id, now)?
                            || context.end_dock_item_drag(document, *pointer_id, *x, *y, false)?
                        {
                            context.release_pointer(document, *pointer_id);
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        let pressed = context.release_pointer(document, *pointer_id);
                        if let Some(pressed) = pressed {
                            if Some(pressed) == target && !*activation_click {
                                context.activate_node_at(pressed, *x, *y)?;
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Cancel => {
                        context.text_editor_pointer_release(*pointer_id);
                        let scrollbar = context.end_scrollbar_drag(document, *pointer_id, true)?;
                        let range = context.end_range_drag(document, *pointer_id, true)?;
                        let xy_pad = context.end_xy_pad_drag(document, *pointer_id, true)?;
                        let graph = optional_input!(
                            "graph-canvas",
                            context.end_graph_canvas_pointer(document, *pointer_id, *x, *y, true,),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let minimap = optional_input!(
                            "graph-canvas",
                            context.end_graph_minimap_pointer(document, *pointer_id, true),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let reorder = optional_input!(
                            "controls",
                            context.end_reorder_list_pointer(document, *pointer_id, *x, *y, true,),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let split = context.end_split_resize(document, *pointer_id, true)?;
                        let dock_split =
                            context.end_dock_split_resize(document, *pointer_id, true)?;
                        let workspace = context.end_workspace_resize(document, *pointer_id, now)?;
                        let dock_item =
                            context.end_dock_item_drag(document, *pointer_id, *x, *y, true)?;
                        let pressed = context.release_pointer(document, *pointer_id).is_some();
                        context.set_pointer_hover_at(document, *pointer_id, None, now)?;
                        let calendar = optional_input!(
                            "calendar",
                            context.clear_calendar_heatmap_hover(document),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let split_hover = context.sync_split_handle_hover(document, None)?;
                        scrollbar
                            || range
                            || xy_pad
                            || graph
                            || minimap
                            || reorder
                            || split
                            || dock_split
                            || workspace
                            || dock_item
                            || calendar
                            || split_hover
                            || pressed
                    }
                    _ => false,
                };
                overlay.prevent_default || component_handled
            }
            InputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => {
                let (dx, dy) = if modifiers.shift && !cfg!(target_os = "macos") {
                    (*delta_y, *delta_x)
                } else {
                    (*delta_x, *delta_y)
                };
                let scale = if *line_delta {
                    self.line_scroll_extent
                } else {
                    1.0
                };
                let delta = ScrollOffset {
                    x: -dx * scale,
                    y: -dy * scale,
                };
                let overlay = context.route_overlay_pointer(
                    document,
                    0,
                    OverlayPointerPhase::Wheel,
                    *x,
                    *y,
                )?;
                // 锚定浮层（补全弹层 / hover 浮窗）优先：指针落在浮层面板
                // 上时滚轮滚动浮层自身（按行，方向跟随滚轮），不再落到
                // 编辑器或文档滚动。
                let overlay_rows = if *delta_y > 0.0 {
                    1isize
                } else if *delta_y < 0.0 {
                    -1
                } else {
                    0
                };
                if overlay_rows != 0
                    && context.scroll_text_overlay_at(document, *x, *y, overlay_rows)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                #[cfg(feature = "graph-canvas")]
                let graph_delta = if *line_delta {
                    GraphScrollDelta::Lines { y: -dy }
                } else {
                    GraphScrollDelta::Pixels { y: -dy }
                };
                let graph_target = context.pointer_target(document, *x, *y);
                let scrolled = if overlay.prevent_default {
                    overlay
                        .target
                        .map(|target| context.scroll_overlay_from(document, target, delta))
                        .transpose()?
                        .flatten()
                        .is_some()
                } else if graph_target.is_some_and(|_target| {
                    optional_input!("graph-canvas", context.is_graph_canvas(_target), false)
                }) {
                    optional_input!(
                        "graph-canvas",
                        context.scroll_graph_canvas(
                            document,
                            graph_target.expect("graph target"),
                            *x,
                            *y,
                            graph_delta,
                        ),
                        Ok::<bool, FrameworkError>(false)
                    )?
                } else {
                    context.scroll_at(document, *x, *y, delta)?.is_some()
                };
                overlay.prevent_default || scrolled
            }
            InputEvent::Keyboard {
                pressed,
                key,
                text,
                repeat: _,
                modifiers,
                ..
            } if *pressed && !modifiers.alt && !modifiers.shift => {
                let primary = modifiers.control || modifiers.meta;
                if primary && self.dispatch_clipboard_shortcut(context, document, key)? {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let range_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowDown" => Some(RangeAdjustment::Decrement),
                        "ArrowRight" | "ArrowUp" => Some(RangeAdjustment::Increment),
                        "PageDown" => Some(RangeAdjustment::PageDecrement),
                        "PageUp" => Some(RangeAdjustment::PageIncrement),
                        "Home" => Some(RangeAdjustment::Minimum),
                        "End" => Some(RangeAdjustment::Maximum),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = range_adjustment
                    && context.adjust_focused_range(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let xy_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(XYPadAdjustment::Left),
                        "ArrowRight" => Some(XYPadAdjustment::Right),
                        "ArrowUp" => Some(XYPadAdjustment::Up),
                        "ArrowDown" => Some(XYPadAdjustment::Down),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = xy_adjustment
                    && context.adjust_focused_xy_pad(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                #[cfg(feature = "graph-canvas")]
                let graph_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(GraphCanvasAdjustment::PanLeft),
                        "ArrowRight" => Some(GraphCanvasAdjustment::PanRight),
                        "ArrowUp" => Some(GraphCanvasAdjustment::PanUp),
                        "ArrowDown" => Some(GraphCanvasAdjustment::PanDown),
                        "Home" | "0" => Some(GraphCanvasAdjustment::Fit),
                        "+" | "=" => Some(GraphCanvasAdjustment::ZoomIn),
                        "-" => Some(GraphCanvasAdjustment::ZoomOut),
                        "Escape" => Some(GraphCanvasAdjustment::ClearSelection),
                        _ => None,
                    })
                    .flatten();
                #[cfg(feature = "graph-canvas")]
                if let Some(adjustment) = graph_adjustment
                    && optional_input!(
                        "graph-canvas",
                        context.adjust_focused_graph_canvas(document, adjustment),
                        Ok::<bool, FrameworkError>(false)
                    )?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let split_direction = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowUp" => Some(-1.0),
                        "ArrowRight" | "ArrowDown" => Some(1.0),
                        _ => None,
                    })
                    .flatten();
                if let Some(direction) = split_direction
                    && (context.adjust_focused_split(document, direction)?
                        || context.adjust_focused_dock_split(document, direction)?)
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                if !primary {
                    let number_steps = match key.as_str() {
                        "ArrowUp" => Some(1),
                        "ArrowDown" => Some(-1),
                        _ => None,
                    };
                    if let Some(steps) = number_steps
                        && context.step_focused_number_input(document, steps)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), "Enter")
                        && context.commit_focused_number_input(document)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), "Escape")
                        && context.revert_focused_number_input(document)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    let palette_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::ActionPickerNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::ActionPickerNavigation::Next),
                        "Home" => Some(nana_ui_runtime::ActionPickerNavigation::First),
                        "End" => Some(nana_ui_runtime::ActionPickerNavigation::Last),
                        "Enter" => Some(nana_ui_runtime::ActionPickerNavigation::Confirm),
                        "Escape" => Some(nana_ui_runtime::ActionPickerNavigation::Dismiss),
                        _ => None,
                    };
                    if let Some(navigation) = palette_nav
                        && context.navigate_focused_command_palette(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    let select_delta = match key.as_str() {
                        "ArrowUp" => Some(-1),
                        "ArrowDown" => Some(1),
                        _ => None,
                    };
                    if let Some(delta) = select_delta
                        && (context.adjust_focused_select(document, delta)?
                            || context.adjust_focused_dropdown(document, delta)?
                            || context.adjust_focused_search_dropdown(document, delta)?)
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), " " | "Space" | "Enter")
                        && (context.commit_focused_select(document)?
                            || context.commit_focused_dropdown(document)?)
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), "Enter")
                        && context.commit_focused_search_dropdown(document)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    let tree_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::TreeNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::TreeNavigation::Next),
                        "Home" => Some(nana_ui_runtime::TreeNavigation::First),
                        "End" => Some(nana_ui_runtime::TreeNavigation::Last),
                        "ArrowLeft" => Some(nana_ui_runtime::TreeNavigation::Parent),
                        "ArrowRight" => Some(nana_ui_runtime::TreeNavigation::Child),
                        "Enter" => Some(nana_ui_runtime::TreeNavigation::Activate),
                        " " | "Space" => Some(nana_ui_runtime::TreeNavigation::Toggle),
                        _ => None,
                    };
                    if let Some(navigation) = tree_nav
                        && context.navigate_focused_tree(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                }
                if !primary
                    && matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.activate_node(target)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let navigation = match (key.as_str(), primary) {
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
                    context.navigate_focused_table(document, navigation, self.table_page_rows)?
                } else if !primary && !modifiers.alt {
                    match key.as_str() {
                        "Backspace" => context.delete_focused_text_backward(document)?,
                        _ if text.as_ref().is_some_and(|text| !text.is_empty()) => context
                            .replace_focused_text(document, text.as_deref().unwrap_or_default())?,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            InputEvent::Keyboard {
                pressed,
                text,
                modifiers,
                ..
            } if *pressed
                && !modifiers.alt
                && !modifiers.control
                && !modifiers.meta
                && text.as_ref().is_some_and(|value| !value.is_empty()) =>
            {
                context.replace_focused_text(document, text.as_deref().unwrap_or_default())?
            }
            _ => false,
        };
        Ok(InputDisposition {
            prevent_default: handled || keyboard_barrier,
        })
    }

    /// Apply Ctrl/Cmd + C / X / V / A to the focused Runtime editor.
    ///
    /// The Runtime owns what is selected and what an edit does; this adapter
    /// only moves text between that selection and the host pasteboard. A copy
    /// with nothing selected leaves the pasteboard alone rather than clearing
    /// it, and a paste with an empty pasteboard is not an edit.
    fn dispatch_clipboard_shortcut(
        &self,
        context: &mut AppContext,
        document: DocumentId,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        if key.eq_ignore_ascii_case("a") {
            return context.select_all_focused_text(document);
        }
        if key.eq_ignore_ascii_case("c") {
            return Ok(context
                .focused_selected_text(document)
                .is_some_and(|text| self.write_clipboard(&text)));
        }
        if key.eq_ignore_ascii_case("x") {
            let Some(text) = context.focused_selected_text(document) else {
                return Ok(false);
            };
            // Never delete text the pasteboard refused to take.
            if !self.write_clipboard(&text) {
                return Ok(false);
            }
            context.cut_focused_text(document)?;
            return Ok(true);
        }
        if key.eq_ignore_ascii_case("v") {
            let Some(text) = self.read_clipboard() else {
                return Ok(false);
            };
            return context.replace_focused_text(document, &text);
        }
        Ok(false)
    }

    /// Route platform IME into the focused Runtime editor.
    ///
    /// Retained TextInput/TextArea/SearchDropdown/CommandPalette state is the
    /// only editing authority. A
    /// focused editable field, or a blocking overlay, consumes the event so a
    /// second host IME path cannot also mutate it.
    ///
    /// Multi-cursor restriction: composition is anchored to the primary
    /// cursor only. While preedit is active the editor paints a single caret
    /// and, on commit, only the primary selection's text is replaced; the
    /// additional cursors survive through offset remapping.
    pub fn dispatch_ime(
        &self,
        context: &mut AppContext,
        document: DocumentId,
        event: &ImeEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        let overlay_blocks = context.has_blocking_runtime_overlay(document);
        let owns_ime = context
            .focused_text_input(document)
            .is_some_and(|(target, _)| {
                context
                    .world()
                    .accessibility(target)
                    .is_some_and(|state| state.editable)
            });
        let handled = match event {
            ImeEvent::Enabled => false,
            ImeEvent::Disabled => {
                let leftover = context
                    .world()
                    .focused_text_input(document)
                    .and_then(|(id, _)| context.world().ime(id).map(|ime| ime.text.clone()))
                    .filter(|text| !text.is_empty());
                match leftover {
                    Some(text) => context.commit_ime(document, &text)?,
                    None => context.clear_ime(document)?,
                }
            }
            ImeEvent::Preedit { text, selection } => {
                context.set_ime_preedit(document, text.clone(), *selection)?
            }
            ImeEvent::Commit(text) => context.commit_ime(document, text)?,
            ImeEvent::DeleteSurrounding {
                before_bytes,
                after_bytes,
            } => context.delete_ime_surrounding(document, *before_bytes, *after_bytes)?,
        };
        Ok(InputDisposition {
            prevent_default: handled || owns_ime || overlay_blocks,
        })
    }
}

/// 补全弹层在激活期间消费的编辑键。
enum CompletionKey {
    Up,
    Down,
    Accept,
}

impl RuntimeInputAdapter {
    /// Keyboard editing for the focused plain text editor.
    ///
    /// Returns `false` when no plain editor is focused or the key is not an
    /// editing key, so composite-surface navigation and generic activation
    /// keep working unchanged.
    fn text_editor_key(
        context: &mut AppContext,
        document: DocumentId,
        key: &str,
        text: Option<&str>,
        modifiers: nana_ui_platform::InputModifiers,
        mut shaper: Option<&mut dyn TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = context.focused_text_editor(document) else {
            return Ok(false);
        };
        // 补全弹层激活时，无修饰的 Up/Down/Enter/Tab 由弹层消费：Up/Down
        // 移动候选选中项（编辑器选区不动），Enter/Tab 接受选中项。其余键
        // 穿透正常编辑（打字触发宿主重喂过滤列表）；任何修饰键组合
        // （Cmd+D、Alt+Up、Shift+Up 等）一律穿透。
        if !modifiers.control && !modifiers.meta && !modifiers.alt {
            let completion_key = match key {
                "ArrowUp" if !modifiers.shift => Some(CompletionKey::Up),
                "ArrowDown" if !modifiers.shift => Some(CompletionKey::Down),
                "Enter" if !modifiers.shift => Some(CompletionKey::Accept),
                "Tab" if !modifiers.shift => Some(CompletionKey::Accept),
                _ => None,
            };
            if let Some(completion_key) = completion_key
                && context.focused_text_completion_active(document)
            {
                match completion_key {
                    CompletionKey::Up => {
                        context.move_focused_text_completion(document, false)?;
                    }
                    CompletionKey::Down => {
                        context.move_focused_text_completion(document, true)?;
                    }
                    CompletionKey::Accept => {
                        context.accept_focused_text_completion(document, None)?;
                    }
                }
                // 弹层激活期间整键消费（边界上导航无可做也不移动选区）。
                return Ok(true);
            }
        }
        let control = modifiers.control;
        let meta = modifiers.meta;
        let word_modifier = control || modifiers.alt;
        // Alt+Cmd/Ctrl+Up/Down adds cursors above/below the selection(s)
        // (Zed-style multi-cursor). Multiline editors own the gesture even
        // when every target already holds a cursor; single-line fields
        // reject multi-cursor entirely and keep plain movement.
        if modifiers.alt
            && (control || meta)
            && focused.multiline
            && matches!(key, "ArrowUp" | "ArrowDown")
        {
            context.add_focused_text_cursor(
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
                context.duplicate_focused_text_lines(document)?;
            } else {
                context.move_focused_text_lines(document, direction)?;
            }
            return Ok(true);
        }
        let intent = match key {
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
        };
        if let Some(intent) = intent {
            return context.move_focused_text_caret(document, intent, modifiers.shift, shaper);
        }
        let delete = match (key, control || meta, modifiers.alt) {
            ("Backspace", false, false) => Some(TextDeleteKind::Backward),
            ("Backspace", _, true) => Some(TextDeleteKind::WordBackward),
            ("Backspace", true, false) => Some(TextDeleteKind::LineStart),
            ("Delete", false, false) => Some(TextDeleteKind::Forward),
            ("Delete", _, true) => Some(TextDeleteKind::WordForward),
            ("Delete", true, false) => Some(TextDeleteKind::LineEnd),
            _ => None,
        };
        if let Some(kind) = delete {
            return context.delete_focused_text(document, kind);
        }
        if control || meta {
            // Cmd/Ctrl+D selects the next occurrence of the primary
            // selection's word (Zed-style multi-cursor). Multiline only.
            // Cmd/Ctrl+Shift+D is deliberately unbound for now; hosts can
            // call `select_focused_text_occurrence(document, true)` for the
            // reverse direction.
            if key.eq_ignore_ascii_case("d") && focused.multiline && !modifiers.shift {
                return context.select_focused_text_occurrence(document, false);
            }
            // Comment toggle is the only code-editing modified key.
            if key == "/" && focused.code_editing.is_some() {
                return context.code_edit_toggle_comment(document);
            }
            // Cmd/Ctrl+Shift+K deletes the caret line.
            if modifiers.shift && key.eq_ignore_ascii_case("k") {
                return context.delete_focused_text_lines(document);
            }
            // Ctrl/Cmd+J joins the touched selection lines.
            if !modifiers.shift && key.eq_ignore_ascii_case("j") {
                return context.join_focused_text_lines(document);
            }
            // Ctrl/Cmd+Shift+U uppercases, Ctrl/Cmd+U lowercases.
            if key.eq_ignore_ascii_case("u") {
                return context.transform_focused_text_case(document, modifiers.shift);
            }
            return Ok(false);
        }
        if key == "Enter" {
            if focused.multiline {
                return context.insert_focused_text_newline(document);
            }
            // Single-line fields never accept a newline character.
            return Ok(true);
        }
        if key == "Tab" && !meta {
            // snippet 会话内 Tab 跳位优先于缩进；无会话时 `Ok(false)`，
            // 代码编辑器的缩进行为接手。
            if focused.multiline
                && context.advance_focused_text_snippet(document, modifiers.shift)?
            {
                return Ok(true);
            }
            if focused.code_editing.is_some() {
                return context.code_edit_indent(document, modifiers.shift);
            }
            return Ok(false);
        }
        let Some(text) = text.filter(|text| !text.is_empty() && key != "Escape") else {
            return Ok(false);
        };
        let mut typed = text.chars();
        if let (Some(single), None) = (typed.next(), typed.next())
            && focused.code_editing.is_some()
            && context.code_edit_typed(document, single)?
        {
            return Ok(true);
        }
        context.replace_focused_text(document, text)
    }
}

/// Reborrow the per-dispatch shaper so sequential uses never alias.
fn reborrow_text_shaper<'s>(
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
        target = context.world().node(target).and_then(|node| node.parent)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::{LayoutStyle, OverflowSpec};
    use nana_ui_platform::{
        ImeEvent, InputModifiers, MemoryClipboard, PointerType, shared_clipboard,
    };
    use nana_ui_runtime::{
        ActionMenu, ActionMenuItem, Activate, Button, Card, ComponentGeometry, Dialog, Dock,
        DockAxis, DockNode, Entity, GraphModel, GraphNode, GraphPoint, GraphSize, GraphViewport,
        LayoutBox, MeasureTextShaper, ModalSlots, MutationQueue, NodeKind, NodeStyle, OverlayHost,
        OverlayHostState, RangeField, ScrollAxes, ScrollMetrics, ScrollView, SegmentedControl,
        SegmentedOption, SegmentedSelectionRequested, Table, TableCell, TableRow, Text, TextArea,
        TextChanged, TextFindScope, TextInput, TextSearchOptions, TextSelection,
    };
    #[cfg(feature = "calendar")]
    use nana_ui_runtime::{CalendarHeatmap, CalendarHeatmapDatum};
    #[cfg(feature = "graph-canvas")]
    use nana_ui_runtime::{GraphMinimap, GraphMinimapEvent};
    use std::sync::{Arc, Mutex};

    fn wheel(x: f32, y: f32, delta_y: f32) -> InputEvent {
        InputEvent::Wheel {
            x,
            y,
            delta_x: 0.0,
            delta_y,
            line_delta: true,
            modifiers: InputModifiers::default(),
        }
    }

    fn focused_untyped_text_input(
        context: &mut AppContext,
        value: &str,
    ) -> (DocumentId, nana_ui_runtime::StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let id = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let mut create = MutationQueue::new();
        create.create(
            id,
            document,
            nana_ui_runtime::NodeKind::Element {
                tag: "input".into(),
            },
        );
        create.set_interaction(
            id,
            nana_ui_runtime::InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_text_input(id, Some(nana_ui_runtime::TextInputState::new(value)));
        create.set_accessibility(
            id,
            nana_ui_runtime::AccessibilityState {
                role: nana_ui_runtime::AccessibilityRole::TextInput,
                editable: true,
                ..nana_ui_runtime::AccessibilityState::default()
            },
        );
        create.request_focus(document, Some(id));
        context.commit_mutations(create).unwrap();
        (document, id)
    }

    fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
        pointer_with(phase, x, y, false)
    }

    fn pointer_with(phase: PointerPhase, x: f32, y: f32, activation_click: bool) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click,
            modifiers: InputModifiers::default(),
        }
    }

    #[test]
    fn pointer_release_activates_the_retained_hit_target() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(button.stable_id()), Some("Running"));
    }

    #[test]
    fn macos_activation_click_does_not_activate_the_hit_target() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer_with(PointerPhase::Down, 30.0, 30.0, true)
                )
                .unwrap()
                .prevent_default
        );
        let _ = adapter.dispatch(
            &mut context,
            document,
            &pointer_with(PointerPhase::Up, 30.0, 30.0, true),
        );
        assert_eq!(context.world().text(button.stable_id()), Some("Build"));
    }

    #[test]
    fn a_right_button_press_dispatches_a_secondary_press_without_activating() {
        use nana_ui_runtime::SecondaryPress;
        use std::sync::{Arc, Mutex};

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        let presses = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&presses);
        context
            .on(button, move |_button, press: &SecondaryPress, _cx| {
                observed.lock().unwrap().push(*press);
            })
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        let mut secondary = pointer(PointerPhase::Down, 30.0, 30.0);
        if let InputEvent::Pointer { button, .. } = &mut secondary {
            *button = 2;
        }
        assert!(
            adapter
                .dispatch(&mut context, document, &secondary)
                .unwrap()
                .prevent_default
        );
        let press = *presses
            .lock()
            .unwrap()
            .first()
            .expect("one secondary press");
        assert_eq!(press.target, button.stable_id());
        assert_eq!((press.x, press.y), (30.0, 30.0));

        // The release must not activate: no press was recorded for button 2.
        let mut release = pointer(PointerPhase::Up, 30.0, 30.0);
        if let InputEvent::Pointer { button, .. } = &mut release {
            *button = 2;
        }
        adapter.dispatch(&mut context, document, &release).unwrap();
        assert_eq!(context.world().text(button.stable_id()), Some("Build"));
    }

    #[test]
    #[cfg(feature = "graph-canvas")]
    fn pointer_drag_on_a_graph_minimap_requests_viewport_navigation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let model = GraphModel::new(
            vec![GraphNode::new(
                "node",
                "Node",
                GraphPoint::ZERO,
                GraphSize::new(200.0, 100.0),
            )],
            Vec::new(),
        )
        .expect("valid graph");
        let minimap = context
            .create_component(
                document,
                GraphMinimap::new(model).canvas_size(GraphSize::new(400.0, 200.0)),
            )
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(minimap, move |_minimap, event: &GraphMinimapEvent, _cx| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            minimap.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        for (phase, x, y) in [
            (PointerPhase::Down, 50.0, 25.0),
            (PointerPhase::Move, 60.0, 30.0),
            (PointerPhase::Up, 60.0, 30.0),
        ] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &pointer(phase, x, y))
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(
            *events.lock().unwrap(),
            [
                GraphMinimapEvent::ViewportRequested(GraphViewport::new(
                    GraphPoint::new(100.0, 50.0),
                    1.0
                )),
                GraphMinimapEvent::ViewportRequested(GraphViewport::new(
                    GraphPoint::new(80.0, 40.0),
                    1.0
                )),
            ]
        );
        assert!(context.world().pointer_capture(document, 1).is_none());
    }

    #[test]
    fn pointer_down_moves_focus_to_the_hit_text_input() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Other"))
            .unwrap();
        let input = context
            .create_component(document, TextInput::new("NanaUI"))
            .unwrap();
        assert!(context.focus_node(document, button.stable_id()).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            input.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 40.0,
                width: 160.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 24.0, 52.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(input.stable_id()));
    }

    #[test]
    fn focused_textarea_caret_uses_text_color_and_clears_on_outside_press() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let editor = context
            .create_component(document, TextArea::new("draft"))
            .unwrap();
        let surface = context.create_component(document, Card::new()).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            editor.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 72.0,
            },
        );
        layout.write_layout(
            surface.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 90.0,
                width: 200.0,
                height: 80.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        assert!(context.focus_node(document, editor.stable_id()).unwrap());
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);

        let Some(ComponentGeometry::TextInput {
            caret, caret_color, ..
        }) = context.world().component_geometry(editor.stable_id())
        else {
            panic!("expected text input geometry");
        };
        let palette = nana_ui_core::SemanticPalette::dark();
        assert!(caret.is_some());
        assert_eq!(caret_color, palette.text.as_rgba_array());
        assert_ne!(caret_color, palette.accent.as_rgba_array());

        RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 24.0, 120.0),
            )
            .unwrap();
        assert_eq!(context.world().focused(document), None);
        assert!(matches!(
            context.world().component_geometry(editor.stable_id()),
            Some(ComponentGeometry::TextInput { caret: None, .. })
        ));

        assert!(context.focus_node(document, editor.stable_id()).unwrap());
        RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 400.0, 400.0),
            )
            .unwrap();
        assert_eq!(context.world().focused(document), None);
    }

    #[test]
    fn segmented_pointer_lease_consumes_release_and_only_requests_on_inside_up() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, _event: &SegmentedSelectionRequested, _cx| {
                    *observed.lock().unwrap() += 1;
                },
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            control.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            first.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        layout.write_layout(
            second.stable_id(),
            LayoutBox {
                x: 76.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, SegmentedControl::focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 140.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
        assert!(context.read(first, SegmentedOption::selected).unwrap());
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
    }

    #[test]
    fn document_tab_order_uses_one_roving_entry_and_wraps_both_directions() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context
            .create_component(document, Button::new("Before"))
            .unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let after = context
            .create_component(document, Button::new("After"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        context.focus_node(document, before.stable_id()).unwrap();
        let tab = |shift| InputEvent::Keyboard {
            pressed: true,
            key: "Tab".into(),
            text: None,
            code: "Tab".into(),
            repeat: false,
            modifiers: InputModifiers {
                shift,
                ..InputModifiers::default()
            },
        };
        let mut adapter = RuntimeInputAdapter::default();
        for expected in [first.stable_id(), after.stable_id(), before.stable_id()] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &tab(false))
                    .unwrap()
                    .prevent_default
            );
            assert_eq!(context.world().focused(document), Some(expected));
        }
        assert!(
            adapter
                .dispatch(&mut context, document, &tab(true))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(after.stable_id()));
        assert!(context.focus_node(document, second.stable_id()).unwrap());
    }

    #[test]
    fn focused_runtime_text_uses_keyboard_and_ime_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: (key.chars().count() == 1).then(|| key.into()),
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("U"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "你".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .ime(input.stable_id())
                .map(|ime| ime.text.as_str()),
            Some("你")
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("你".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU你"));
        assert_eq!(context.world().ime(input.stable_id()), None);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Backspace"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
    }

    #[test]
    fn clipboard_shortcuts_move_text_between_the_editor_and_the_pasteboard() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());

        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let primary = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        };

        // Nothing is selected yet, so a copy must not clear the pasteboard.
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(clipboard.lock().unwrap().read_text(), None);

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some(""));
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Nana")
        );

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaNana"));

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaNana"));
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("NanaNana")
        );
    }

    #[test]
    fn a_read_only_field_copies_but_never_loses_text_to_a_cut() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana").read_only(true))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());

        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let primary = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Nana")
        );
        assert_eq!(context.world().text(input.stable_id()), Some("Nana"));

        assert!(clipboard.lock().unwrap().write_text("pasted"));
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("Nana"));
    }

    #[test]
    fn focused_runtime_text_inserts_shifted_printable_characters() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("inspect "))
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());
        let event = InputEvent::Keyboard {
            pressed: true,
            key: "2".into(),
            text: Some("@".into()),
            code: "Digit2".into(),
            repeat: false,
            modifiers: InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(area.stable_id()), Some("inspect @"));
    }

    #[test]
    fn focused_runtime_textarea_ime_updates_multiline_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("第一行\n"))
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "第二".into(),
                        selection: Some((0, "第".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        let composition = context
            .world()
            .ime(area.stable_id())
            .expect("focused textarea keeps preedit on retained state");
        assert_eq!(composition.text, "第二");
        assert_eq!(composition.selection, Some((0, "第".len())));
        assert_eq!(context.world().text(area.stable_id()), Some("第一行\n"));

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("第二行".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);

        context
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        assert!(
            !adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "三".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);
    }

    #[test]
    fn dispatch_ime_commits_a_focused_text_input_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().ime(id).map(|ime| ime.text.as_str()),
            Some("世")
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana")
        );

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("世界".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana世界")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_disabled_commits_leftover_preedit_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Disabled)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana世")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_deletes_surrounding_committed_text_and_skips_invalid_spans() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "你好");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::DeleteSurrounding {
                        before_bytes: "好".len(),
                        after_bytes: 0,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("你")
        );
        assert_eq!(
            context.world().ime(id).map(|ime| ime.text.as_str()),
            Some("世"),
            "delete surrounding must not clear preedit"
        );

        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::DeleteSurrounding {
                        before_bytes: 1,
                        after_bytes: 0,
                    },
                )
                .unwrap()
                .prevent_default,
            "focused editable still consumes an un-applicable span"
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("你"),
            "invalid byte span must leave committed text unchanged"
        );
    }

    #[test]
    fn wheel_routes_to_nearest_scrollview_and_bubbles_at_edge() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let outer = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let inner = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let cell = context
            .create_component(document, TableCell::new("row"))
            .unwrap();
        context.append_child(outer, inner).unwrap();
        context.append_child(inner, cell).unwrap();
        context
            .set_scroll_metrics(
                outer,
                ScrollMetrics {
                    viewport_width: 200.0,
                    viewport_height: 100.0,
                    content_width: 200.0,
                    content_height: 300.0,
                },
            )
            .unwrap();
        context
            .set_scroll_metrics(
                inner,
                ScrollMetrics {
                    viewport_width: 180.0,
                    viewport_height: 80.0,
                    content_width: 180.0,
                    content_height: 140.0,
                },
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            outer.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        layout.write_layout(
            inner.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 80.0,
            },
        );
        layout.write_layout(
            cell.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 30.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            0.0
        );
        context.take_system_work();
        context.rebuild_hit_test(document);

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            60.0
        );
    }

    #[test]
    fn wheel_on_overflow_auto_updates_scroll_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroller = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let child = nana_ui_runtime::StableNodeId::new(2).unwrap();
        let mut create = MutationQueue::new();
        create.create(scroller, document, NodeKind::Element { tag: "div".into() });
        create.create(child, document, NodeKind::Element { tag: "item".into() });
        create.insert(scroller, child, None);
        create.set_style(
            scroller,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_y: OverflowSpec::Auto,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        create.write_layout(
            scroller,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        create.write_layout(
            child,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 300.0,
            },
        );
        context.commit_mutations(create).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().scroll_offset(scroller).unwrap().y, 60.0);
        assert_eq!(context.world().scroll_offset(child).unwrap().y, 0.0);
        assert!(
            context
                .world()
                .node_style(scroller)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !context.is_scroll_view(scroller),
            "L1 overflow must not stamp a ScrollView"
        );
    }

    #[test]
    fn keyboard_routes_navigation_from_focused_table_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context.create_component(document, Table::new()).unwrap();
        let first_row = context.create_component(document, TableRow::new()).unwrap();
        let second_row = context.create_component(document, TableRow::new()).unwrap();
        let first = context
            .create_component(document, TableCell::new("one"))
            .unwrap();
        let second = context
            .create_component(document, TableCell::new("two"))
            .unwrap();
        context.append_child(table, first_row).unwrap();
        context.append_child(table, second_row).unwrap();
        context.append_child(first_row, first).unwrap();
        context.append_child(second_row, second).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(first.stable_id()));
        context.commit_mutations(focus).unwrap();

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowDown".into(),
            text: None,
            code: "ArrowDown".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
    }

    #[test]
    fn segmented_keyboard_routing_precedes_generic_navigation_and_activation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let disabled = context
            .create_detached_component(document, SegmentedOption::new("Split").disabled(true))
            .unwrap();
        let last = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, disabled, last], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, event: &SegmentedSelectionRequested, _cx| {
                    observed.lock().unwrap().push(event.option);
                },
            )
            .unwrap();
        context.focus_node(document, first.stable_id()).unwrap();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("ArrowRight", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert_eq!(&*requests.lock().unwrap(), &[last.stable_id()]);
        for modifiers in [
            InputModifiers {
                alt: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        ] {
            assert!(
                !adapter
                    .dispatch(&mut context, document, &key("Home", true, modifiers))
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(requests.lock().unwrap().as_slice(), [last.stable_id()]);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Home", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [last.stable_id(), first.stable_id()]
        );
        let count = requests.lock().unwrap().len();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Space", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().len(), count);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Enter", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().last(), Some(&first.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(
            context
                .set_segmented_selection(control, Some(last))
                .unwrap()
        );
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(last.stable_id())
        );
    }

    #[test]
    fn range_field_quantizes_keyboard_and_cancels_captured_drag() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let range = context
            .create_component(document, RangeField::new(0.5, 0.0, 1.0, 0.1).unwrap())
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            range.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 300.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        assert!(context.focus_node(document, range.stable_id()).unwrap());

        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("ArrowRight"))
                .unwrap()
                .prevent_default
        );
        assert!((context.read(range, |range| range.value).unwrap() - 0.6).abs() < 1e-12);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Home"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);

        let track = match context.world().component_geometry(range.stable_id()) {
            Some(nana_ui_runtime::ComponentGeometry::Range { track, .. }) => track,
            _ => panic!("range geometry expected"),
        };
        let drag_x = track.x + track.width * 0.8;
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.8);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);
        assert_eq!(context.world().pointer_capture(document, 1), None);
    }

    #[test]
    fn overlay_pointer_sequence_never_activates_the_underlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let underlay = context
            .create_component(document, Button::new("Underlay"))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Dialog"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(underlay, move |_button, _event: &Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            underlay.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 300.0,
            },
        );
        layout.write_layout(
            dialog.stable_id(),
            LayoutBox {
                x: 100.0,
                y: 100.0,
                width: 100.0,
                height: 100.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*activations.lock().unwrap(), 0);
    }

    /// Lays a popover over a button so the two never overlap, and reports the
    /// shared activation counter of the button.
    fn popover_over_button(
        context: &mut AppContext,
        document: DocumentId,
    ) -> (Entity<ActionMenu>, Arc<Mutex<u32>>) {
        let underlay = context
            .create_component(document, Button::new("Underlay"))
            .unwrap();
        let menu = context
            .create_component(document, ActionMenu::new().open(true))
            .unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(underlay, move |_button, _event: &Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            underlay.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 60.0,
            },
        );
        layout.write_layout(
            menu.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 100.0,
                width: 200.0,
                height: 100.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        (menu, activations)
    }

    #[test]
    fn outside_press_closes_the_popover_without_reaching_the_underlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, activations) = popover_over_button(&mut context, document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Up, 20.0, 20.0),
            )
            .unwrap();

        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
        // An app-owned trigger button sits outside the popover too, so letting
        // this press through would toggle the menu straight back open.
        assert_eq!(*activations.lock().unwrap(), 0);
    }

    #[test]
    fn press_inside_the_popover_leaves_it_open() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        let item = context
            .create_component(document, ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            item.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 104.0,
                width: 192.0,
                height: 28.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 20.0, 110.0),
            )
            .unwrap();

        assert!(context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn escape_closes_focused_field_options_without_committing() {
        use nana_ui_core::DropdownEvent;
        use nana_ui_runtime::{
            Dropdown, DropdownOption, SearchDropdown, SearchDropdownEvent, SearchDropdownOption,
            Select, SelectOption,
        };

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let select = context
            .create_component(
                document,
                Select::new(Some("a"))
                    .options([
                        SelectOption::new("a", "Alpha"),
                        SelectOption::new("b", "Beta"),
                    ])
                    .opened(true),
            )
            .unwrap();
        let dropdown = context
            .create_component(
                document,
                Dropdown::single(Some("a"))
                    .options([
                        DropdownOption::new("a", "Alpha"),
                        DropdownOption::new("b", "Beta"),
                    ])
                    .opened(true),
            )
            .unwrap();
        let search = context
            .create_component(
                document,
                SearchDropdown::new(Some("a"))
                    .options([
                        SearchDropdownOption::new("a", "Alpha"),
                        SearchDropdownOption::new("b", "Beta"),
                    ])
                    .query("Beta")
                    .opened(true),
            )
            .unwrap();
        let dropdown_events = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&dropdown_events);
        context
            .on(dropdown, move |_, event: &DropdownEvent<Arc<str>>, _| {
                events.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let search_events = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&search_events);
        context
            .on(search, move |_, event: &SearchDropdownEvent, _| {
                events.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context
            .update_component(select, |field, _| field.highlighted = Some(1))
            .unwrap();
        context
            .update_component(dropdown, |field, _| field.highlighted = Some(1))
            .unwrap();
        let selection = context
            .read(dropdown, |field| field.selection.clone())
            .unwrap();
        let search_state = context.read(search, |field| field.state.clone()).unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        for target in [select.stable_id(), dropdown.stable_id(), search.stable_id()] {
            assert!(context.focus_node(document, target).unwrap());
            assert!(
                adapter
                    .dispatch(&mut context, document, &escape)
                    .unwrap()
                    .prevent_default
            );
            assert!(
                !adapter
                    .dispatch(&mut context, document, &escape)
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(
            context
                .read(select, |field| (field.opened, field.value.clone()))
                .unwrap(),
            (false, Some(Arc::from("a")))
        );
        assert_eq!(
            context
                .read(dropdown, |field| (field.opened, field.selection.clone()))
                .unwrap(),
            (false, selection)
        );
        assert_eq!(
            context
                .read(search, |field| (
                    field.opened,
                    field.value.clone(),
                    field.query.clone(),
                    field.state.clone()
                ))
                .unwrap(),
            (false, Some(Arc::from("a")), "Beta".into(), search_state)
        );
        assert_eq!(
            *dropdown_events.lock().unwrap(),
            vec![DropdownEvent::Closed]
        );
        assert_eq!(
            *search_events.lock().unwrap(),
            vec![SearchDropdownEvent::Closed]
        );
    }

    #[test]
    fn escape_closes_the_popover() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
        // The next Escape belongs to the application navigation layer. A
        // host must pass the per-event result rather than cache overlay state.
        assert!(
            !adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
    }

    #[test]
    fn a_popover_that_opts_out_ignores_outside_presses_and_escape() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        context
            .update_component(menu, |menu, _| {
                menu.popover.close_on_outside = false;
                menu.popover.close_on_escape = false;
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();

        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 20.0, 20.0),
            )
            .unwrap();
        adapter
            .dispatch(
                &mut context,
                document,
                &InputEvent::Keyboard {
                    pressed: true,
                    key: "Escape".into(),
                    text: None,
                    code: "Escape".into(),
                    repeat: false,
                    modifiers: InputModifiers::default(),
                },
            )
            .unwrap();

        assert!(context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn menu_item_can_close_during_activation_without_releasing_input_or_wheel_barrier() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let menu = context
            .create_component(document, ActionMenu::new().open(true))
            .unwrap();
        let item = context
            .create_component(document, ActionMenuItem::new("Build"))
            .unwrap();
        context.append_child(background, host).unwrap();
        context.append_child(host, menu).unwrap();
        context.append_child(menu, item).unwrap();
        let host_id = host.stable_id();
        context
            .on(item, move |_item, _event: &Activate, cx| {
                cx.mutations()
                    .set_overlay_host(host_id, OverlayHostState::default());
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        for (id, x, y, width, height) in [
            (background.stable_id(), 0.0, 0.0, 300.0, 300.0),
            (menu.stable_id(), 100.0, 100.0, 100.0, 100.0),
            (item.stable_id(), 110.0, 110.0, 80.0, 32.0),
        ] {
            layout.write_layout(
                id,
                LayoutBox {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        context.commit_mutations(layout).unwrap();
        context
            .set_scroll_metrics(
                background,
                ScrollMetrics {
                    viewport_width: 300.0,
                    viewport_height: 300.0,
                    content_width: 300.0,
                    content_height: 900.0,
                },
            )
            .unwrap();
        context.activate_overlay(host, menu).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(20.0, 20.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .scroll_offset(background.stable_id())
                .unwrap()
                .y,
            0.0
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
    }

    #[test]
    fn overlay_keyboard_ignores_primary_tab_and_repeated_escape() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        let button = context
            .create_detached_component(document, Button::new("Save"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context
            .set_modal_slots(
                dialog,
                ModalSlots {
                    actions: vec![button.stable_id()],
                    ..ModalSlots::default()
                },
            )
            .unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };

        let primary_tab = key(
            "Tab",
            false,
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
        let repeat_escape = key("Escape", true, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &repeat_escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            Some(dialog.stable_id())
        );
        let key_release = InputEvent::Keyboard {
            pressed: false,
            key: "a".into(),
            text: None,
            code: "KeyA".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            adapter
                .dispatch(&mut context, document, &key_release)
                .unwrap()
                .prevent_default
        );
        let escape = key("Escape", false, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(context.active_runtime_overlay(document).is_none());
        context.advance_animations(std::time::Duration::from_secs(1));
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
    }

    #[test]
    fn pointer_on_dock_handle_changes_split_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            handle,
            LayoutBox {
                x: 156.8,
                y: 0.0,
                width: 8.0,
                height: 200.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 160.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - (0.4_f32 + 40.0 / 392.0).clamp(0.05, 0.95)).abs() < 0.001);
    }

    #[test]
    fn keyboard_arrow_right_on_focused_dock_handle_changes_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        assert!(context.focus_node(document, handle).unwrap());

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowRight".into(),
            text: None,
            code: "ArrowRight".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - 0.45).abs() < 0.001);
    }

    #[test]
    #[cfg(feature = "calendar")]
    fn pointer_on_calendar_heatmap_sets_active_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let heatmap = context
            .create_component(
                document,
                CalendarHeatmap::new([
                    CalendarHeatmapDatum::<()>::new("2026-06-01", 2.0),
                    CalendarHeatmapDatum::<()>::new("2026-06-03", 8.0),
                ]),
            )
            .unwrap();
        let model = context.read(heatmap, CalendarHeatmap::model).unwrap();
        let cell = model
            .cells
            .iter()
            .find(|cell| cell.date == "2026-06-03")
            .expect("June 3");
        context
            .commit_mutations({
                let mut mutations = MutationQueue::new();
                mutations.write_layout(
                    heatmap.stable_id(),
                    LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: model.width,
                        height: model.height,
                    },
                );
                mutations
            })
            .unwrap();
        context.rebuild_hit_test(document);

        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, cell.x + 1.0, cell.y + 1.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.read(heatmap, |calendar| calendar.active).unwrap(),
            Some(
                model
                    .cells
                    .iter()
                    .position(|item| item.date == "2026-06-03")
                    .expect("index")
            )
        );
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 400.0, 400.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .read(heatmap, |calendar| calendar.active)
                .unwrap()
                .is_none()
        );
    }

    fn edit_key(key: &str, text: Option<&str>, modifiers: InputModifiers) -> InputEvent {
        InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: text.map(str::to_string),
            code: key.into(),
            repeat: false,
            modifiers,
        }
    }

    fn plain_key(key: &str) -> InputEvent {
        edit_key(key, None, InputModifiers::default())
    }

    fn shift_key(key: &str) -> InputEvent {
        edit_key(
            key,
            None,
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        )
    }

    fn meta_key(key: &str) -> InputEvent {
        edit_key(
            key,
            None,
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        )
    }

    fn textarea_selection(context: &AppContext, node: StableNodeId) -> (String, usize, usize) {
        let state = context.world().text_input(node).unwrap();
        (
            state.value.clone(),
            state.selection.anchor,
            state.selection.focus,
        )
    }

    #[test]
    fn arrow_keys_move_and_extend_the_focused_textarea_caret() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abcdef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // The caret starts at the value end; Left steps back one grapheme.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 5, 5));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Home"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 0, 0));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("End"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 6, 6));

        // Shift+Left extends the selection; typing replaces it.
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 6, 4));
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("x", Some("X"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdX".into(), 5, 5));
    }

    #[test]
    fn vertical_arrows_fall_back_to_logical_lines_without_a_shaper() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abc\ndefg\nhi"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 9, 9)
        );

        // Up keeps the grapheme column: column 0 lands on the line start.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 4, 4)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 9, 9)
        );

        // Cmd+Up / Cmd+Down jump to the document edges.
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 0, 0)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 11, 11)
        );
    }

    #[test]
    fn delete_keys_remove_selections_words_and_line_spans() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("one two three"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // Forward delete at the caret end is a no-op that still stays owned.
        let word_modifier = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Backspace", None, word_modifier)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "one two ");

        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("Backspace", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "");

        // Forward delete at the value end declines; typing still works.
        assert!(
            !adapter
                .dispatch(&mut context, document, &plain_key("Delete"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("h", Some("h"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "h");
    }

    #[test]
    fn code_editor_newline_copies_indent_and_completes_pairs() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("fn a() {\n  x").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // Enter after indented content copies the indentation.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn a() {\n  x\n  ".into(), 15, 15)
        );

        // Typing an open brace completes the pair and parks inside it.
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("{", Some("{"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn a() {\n  x\n  {}".into(), 16, 16)
        );

        // Enter between the pair opens a middle line at the deeper level.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node).0,
            "fn a() {\n  x\n  {\n  \t\n  }"
        );
        assert_eq!(textarea_selection(&context, node).2, 20);
    }

    #[test]
    fn code_editor_comment_toggle_and_tab_indent_the_line() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("  x").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  //x");
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  x");

        // Tab indents the caret line; Shift+Tab outdents again.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Home"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "\t  x");
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  x");
    }

    #[test]
    fn plain_textarea_enter_inserts_a_bare_newline_without_pairing() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab\n");
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("(", Some("("), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab\n(");
    }

    #[test]
    fn pointer_press_places_the_caret_and_multi_click_selects() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hello world"))
            .unwrap();
        let node = input.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 32.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down || phase == PointerPhase::Move),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        // A press past the line end parks the caret on the line end.
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(190.0, 16.0, PointerPhase::Down),
                    Duration::from_millis(1_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(190.0, 16.0, PointerPhase::Up),
                Duration::from_millis(1_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 11, 11)
        );

        // A quick second press selects the word under the caret.
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(190.0, 16.0, PointerPhase::Down),
                    Duration::from_millis(1_060),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 6, 11)
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(190.0, 16.0, PointerPhase::Up),
                Duration::from_millis(1_080),
                Some(&mut shaper),
            )
            .unwrap();
    }

    #[test]
    fn pointer_drag_extends_the_selection_from_the_press_anchor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hello world"))
            .unwrap();
        let node = input.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 32.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let pointer_event = |phase: PointerPhase, x: f32| InputEvent::Pointer {
            phase,
            pointer_id: 3,
            pointer_type: PointerType::Mouse,
            x,
            y: 16.0,
            screen_x: x,
            screen_y: 16.0,
            button: 0,
            buttons: u16::from(phase != PointerPhase::Up),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_event(PointerPhase::Down, 190.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &pointer_event(PointerPhase::Move, 0.0),
                    Duration::from_millis(2_010),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 11, 0)
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_event(PointerPhase::Up, 0.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
    }

    /// 挂一个收集 TextChanged 的观察者，供查找/替换命令断言事件发射。
    fn track_text_changed(
        context: &mut AppContext,
        area: Entity<TextArea>,
    ) -> Arc<Mutex<Vec<String>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(area, move |_area, event: &TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.clone());
            })
            .unwrap();
        events
    }

    /// 多行编辑器拖拽移动测试的公共装配：两行 `abc\ndef`，字符宽 10、
    /// 行高 12、零内边距（offset = 列×10 + 行×12 命中）。返回
    /// `(context, document, node, 事件收集器)`。
    fn drag_drop_editor() -> (
        AppContext,
        DocumentId,
        StableNodeId,
        Arc<Mutex<Vec<String>>>,
    ) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("abc\ndef").style(nana_ui_runtime::NodeStyle {
                    layout: std::sync::Arc::new(nana_ui_core::LayoutStyle {
                        padding: Some(nana_ui_core::LengthSpec::Px(0.0)),
                        font_size: Some(10.0),
                        line_height: Some(nana_ui_core::LineHeightSpec::Absolute(12.0)),
                        min_height: None,
                        ..nana_ui_core::LayoutStyle::default()
                    }),
                    ..nana_ui_runtime::NodeStyle::default()
                }),
            )
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 64.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        (context, document, node, events)
    }

    /// 带 x/y 与修饰键的指针事件构造器。
    fn drag_pointer_event(
        phase: PointerPhase,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
    ) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 3,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase != PointerPhase::Up),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers,
        }
    }

    /// 选中 `def`：End 到文档尾，Shift+Left 三次（偏移 7..4）。
    fn select_trailing_def(
        adapter: &mut RuntimeInputAdapter,
        context: &mut AppContext,
        document: DocumentId,
        shaper: &mut MeasureTextShaper,
    ) {
        adapter
            .dispatch_with_shaper(
                context,
                document,
                &plain_key("End"),
                Duration::from_millis(1_000),
                Some(shaper),
            )
            .unwrap();
        for _ in 0..3 {
            adapter
                .dispatch_with_shaper(
                    context,
                    document,
                    &shift_key("ArrowLeft"),
                    Duration::from_millis(1_000),
                    Some(shaper),
                )
                .unwrap();
        }
    }

    /// 无修饰键的拖拽指针事件（按下/移动/释放）。
    fn pointer_down(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Down, x, y, InputModifiers::default())
    }

    fn pointer_move(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Move, x, y, InputModifiers::default())
    }

    fn pointer_up(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Up, x, y, InputModifiers::default())
    }

    /// 拖拽移动主流程：选中 `def`（第二行 0..3 列 → 偏移 4..7），在选区
    /// 内按下并拖到第一行行首释放 = 移动文本，选区落在插入文本上，整
    /// 个移动只发一次变更（单步撤销的修订语义）。
    #[test]
    fn drag_selection_moves_text_in_one_revision() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        // 选中 "def"：End 到文档尾，Shift+Left 三次。
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );

        // 选区内按下（"e"，offset 5）→ 不塌缩选区。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        // 超过阈值拖到第一行行首（offset 0）。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(0.0, 6.0),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(0.0, 6.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        // "def" 移到文档头，选区落在插入文本上；单次变更（一步撤销）。
        assert_eq!(
            textarea_selection(&context, node),
            ("defabc\n".into(), 0, 3)
        );
        assert_eq!(*events.lock().unwrap(), vec!["defabc\n".to_owned()]);
    }

    /// 落点在源选区边界（target == start / target == end）是退化 no-op：
    /// 文本与选区都保持原状，不产生变更事件。
    #[test]
    fn drag_to_selection_boundary_is_a_no_op() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        // target == start（offset 4）：选区头。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(2.0, 18.0),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(2.0, 18.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        // target == end（offset 7）：选区尾。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(3_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(35.0, 18.0),
                Duration::from_millis(3_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(35.0, 18.0),
                Duration::from_millis(3_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    /// Alt 拖拽 = 复制：原文本保留，选区落在插入的副本上。
    #[test]
    fn alt_drag_selection_copies_text() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        let alt = InputModifiers {
            alt: true,
            ..Default::default()
        };
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, alt),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 0.0, 6.0, alt),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Up, 0.0, 6.0, alt),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("defabc\ndef".into(), 0, 3)
        );
        assert_eq!(*events.lock().unwrap(), vec!["defabc\ndef".to_owned()]);
    }

    /// 低于阈值：按下选区后小位移释放不移动文本，按原点击语义落 caret。
    #[test]
    fn selection_press_below_threshold_falls_back_to_click() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        // 2px 位移，未过阈值。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 13.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Up, 13.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 5, 5)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    /// Esc 取消拖拽：文本与选区保持原状，后续释放不落文本。
    #[test]
    fn escape_cancels_selection_drag() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 0.0, 6.0, InputModifiers::default()),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("Escape"),
                    Duration::from_millis(2_015),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(0.0, 6.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn find_next_and_previous_select_matches_without_text_changed() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab AB ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let sensitive = TextSearchOptions {
            case_sensitive: true,
            ..TextSearchOptions::default()
        };

        // 大小写敏感："ab" 只命中 0..2 与 6..8。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 6, 8)
        );
        // 越过末尾后环绕。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        // 大小写不敏感：从当前选区末端起下一个命中是 "AB"。
        assert!(
            context
                .find_next_focused_text_match(
                    document,
                    "ab",
                    TextSearchOptions::default(),
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 3, 5)
        );
        // 上一个回到第一个 "ab"。
        assert!(
            context
                .find_previous_focused_text_match(
                    document,
                    "ab",
                    sensitive,
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        // 纯移动：值不变、不发 TextChanged。
        assert!(events.lock().unwrap().is_empty());
        // 空 query 不命中。
        assert!(
            !context
                .find_next_focused_text_match(document, "", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert!(
            !context
                .find_previous_focused_text_match(
                    document,
                    "zz",
                    sensitive,
                    TextFindScope::Document
                )
                .unwrap()
        );
    }

    #[test]
    fn replace_focused_text_match_replaces_only_a_matching_selection() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let options = TextSearchOptions::default();

        // 选中第一个 "ab" 后替换，并选中替换后的文本。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: 0,
                    focus: 2,
                };
            })
            .unwrap();
        assert!(
            context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 0, 2));
        assert_eq!(*events.lock().unwrap(), vec!["XY ab".to_string()]);

        // 选区不再是匹配（现在是 "XY"），替换拒绝且不发射事件。
        assert!(
            !context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 0, 2));

        // 宿主先查找下一个再替换。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 3, 5));
        assert!(
            context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY XY".into(), 3, 5));
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    #[test]
    fn replace_all_focused_text_matches_reports_count_and_lands_on_first() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);

        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "ab",
                    TextSearchOptions {
                        whole_word: true,
                        ..TextSearchOptions::default()
                    },
                    "X",
                    TextFindScope::Document,
                    false,
                )
                .unwrap(),
            2
        );
        assert_eq!(textarea_selection(&context, node), ("X cd X".into(), 0, 1));
        assert_eq!(*events.lock().unwrap(), vec!["X cd X".to_string()]);

        // 没有匹配时不修改、不发射事件、计数为 0。
        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "ab",
                    TextSearchOptions::default(),
                    "X",
                    TextFindScope::Document,
                    false,
                )
                .unwrap(),
            0
        );
        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "",
                    TextSearchOptions::default(),
                    "X",
                    TextFindScope::Document,
                    false
                )
                .unwrap(),
            0
        );
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn alt_arrow_keys_move_and_duplicate_the_caret_line() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        let alt = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };
        let alt_shift = InputModifiers {
            alt: true,
            shift: true,
            ..InputModifiers::default()
        };
        // 光标停在 "cd" 行内（偏移 4）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(4);
            })
            .unwrap();

        // Alt+Up 把 "cd" 移到顶部，选区（光标）跟随移动后的文本。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("cd\nab\nef".into(), 1, 1)
        );
        assert_eq!(*events.lock().unwrap(), vec!["cd\nab\nef".to_string()]);

        // Alt+Down 移回原位。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowDown", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab\ncd\nef".into(), 4, 4)
        );

        // Alt+Shift+Down 在下方复制当前行，光标落在副本上。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_shift)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab\ncd\ncd\nef".into(), 7, 7)
        );

        // 文档边缘：手势仍被消费（不回落为普通光标移动），但没有编辑。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("top\nbottom");
            })
            .unwrap();
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "top\nbottom");
    }

    #[test]
    fn cmd_shift_k_ctrl_j_and_case_keys_transform_the_focused_editor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        let ctrl_shift = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::default()
        };
        let ctrl = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        // 光标停在 "cd" 行内。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(4);
            })
            .unwrap();

        // Ctrl+Shift+K 删除光标所在行 "cd"。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("k", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("ab\nef".into(), 3, 3));
        assert_eq!(*events.lock().unwrap(), vec!["ab\nef".to_string()]);

        // Ctrl+J 合并剩余两行（单空格接缝）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(1);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("j", None, ctrl))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab ef");
        assert_eq!(events.lock().unwrap().len(), 2);

        // Ctrl+Shift+U 转大写选区，Ctrl+U 转小写。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: 0,
                    focus: 5,
                };
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("u", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("AB EF".into(), 0, 5));
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("u", None, ctrl))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("ab ef".into(), 0, 5));
        assert_eq!(events.lock().unwrap().len(), 4);
    }

    #[test]
    fn line_transformation_keys_decline_on_single_line_fields() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("abc"))
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // 单行字段没有行块语义：这些键不消费，留给通用路由。
        let ctrl_shift = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::default()
        };
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("k", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "abc");
    }

    #[test]
    fn page_keys_page_by_logical_lines_without_a_shaper() {
        let value = (0..40)
            .map(|index| format!("l{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value.clone()))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(1);
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();

        // 无 shaper：固定 15 个逻辑行。第 15 行起点在 10 个 3 字节行 + 5 个
        // 4 字节行之后，保持第 1 列。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("PageDown"))
                .unwrap()
                .prevent_default
        );
        let line15 = 10 * 3 + 5 * 4;
        assert_eq!(
            textarea_selection(&context, node),
            (value.clone(), line15 + 1, line15 + 1)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("PageUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value.clone(), 1, 1));

        // Shift+PageDown 扩展选区（锚点保留在原列）。
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("PageDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value, 1, line15 + 1));
    }

    #[test]
    fn page_keys_with_a_shaper_move_one_viewport_and_clamp() {
        let value = (0..10)
            .map(|index| format!("l{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value.clone()))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 300.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();

        // 视口高于文档：一次 PageDown 钳制到文档末尾，PageUp 回到首行。
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("PageDown"),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        // 目标列保持 0：落在最后一行行首（而非文档末尾偏移）。
        assert_eq!(textarea_selection(&context, node), (value.clone(), 27, 27));
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("PageUp"),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value, 0, 0));
    }

    #[test]
    fn goto_focused_text_matching_bracket_jumps_to_the_partner() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("fn main() {}"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);

        // 光标停在 '{' 之前：跳到配对的 '}' 上（纯移动，不发事件）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(10);
            })
            .unwrap();
        assert!(
            context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn main() {}".into(), 11, 11)
        );
        // 再跳一次回到 '{'。
        assert!(
            context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn main() {}".into(), 10, 10)
        );
        assert!(events.lock().unwrap().is_empty());

        // 邻近没有括号时不消费。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(2);
            })
            .unwrap();
        assert!(
            !context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
    }

    #[test]
    fn sort_focused_text_lines_sorts_dedups_and_emits_one_change() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("pear\napple\npear"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: 0,
                    focus: "pear\napple\npear".len(),
                };
            })
            .unwrap();

        // 升序 + 去重，选区覆盖排序后的块。
        assert!(
            context
                .sort_focused_text_lines(document, false, true)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("apple\npear".into(), 0, 10)
        );
        assert_eq!(*events.lock().unwrap(), vec!["apple\npear".to_string()]);

        // 降序还原顺序差异。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: 0,
                    focus: "apple\npear".len(),
                };
            })
            .unwrap();
        assert!(
            context
                .sort_focused_text_lines(document, true, false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node).0, "pear\napple");

        // 单行无变化：拒绝且不发事件。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(2);
            })
            .unwrap();
        assert!(
            !context
                .sort_focused_text_lines(document, false, false)
                .unwrap()
        );
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    fn textarea_selections(
        context: &AppContext,
        node: StableNodeId,
    ) -> (String, (usize, usize), Vec<(usize, usize)>) {
        let state = context.world().text_input(node).unwrap();
        (
            state.value.clone(),
            (state.selection.anchor, state.selection.focus),
            state
                .additional_selections
                .iter()
                .map(|selection| (selection.anchor, selection.focus))
                .collect(),
        )
    }

    fn set_selections(
        context: &mut AppContext,
        area: Entity<TextArea>,
        primary: (usize, usize),
        additional: Vec<(usize, usize)>,
    ) {
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection {
                    anchor: primary.0,
                    focus: primary.1,
                };
                area.state.additional_selections = additional
                    .into_iter()
                    .map(|(anchor, focus)| TextSelection { anchor, focus })
                    .collect();
            })
            .unwrap();
    }

    #[test]
    fn editor_render_options_default_off_and_opt_in_drives_derived_presentation() {
        // 四个渲染选项默认关闭：未开启时不产生任何派生标记。
        let defaults = TextArea::new("alpha beta\nalpha");
        assert!(!defaults.occurrence_highlight);
        assert!(!defaults.relative_line_numbers);
        assert!(!defaults.show_whitespace);
        assert!(defaults.wrap_guides.is_empty());

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("alpha beta\nalpha")
                    .line_numbers(true)
                    .relative_line_numbers(true)
                    .occurrence_highlight(true)
                    .show_whitespace(true)
                    .wrap_guides(std::sync::Arc::from([4])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 40.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();

        let presentation = context
            .world()
            .text_input_presentation(node)
            .expect("presentation");
        // 出现高亮：光标（值末尾）停在第二行 "alpha" 上，该出现不画；
        // 全词匹配排除前缀，只剩第一行的 "alpha" 一条。
        assert_eq!(presentation.occurrence_marks.len(), 1);
        // 空白显示：行内一个空格（"alpha beta"），换行不标记。
        assert_eq!(presentation.whitespace_marks.len(), 1);
        // wrap guide：列 4 一个 x 位置。
        assert_eq!(presentation.wrap_guides.len(), 1);
        // 相对行号：光标（值末尾）在第 2 行，显示绝对 2；第 1 行距离 1。
        assert_eq!(presentation.line_numbers, vec![1, 2]);
    }

    #[test]
    fn pointer_press_on_fold_gutter_toggles_the_fold() {
        let value = "fn a() {\n    x();\n    y();\n}\nfn b() {}";
        let fold = nana_ui_runtime::TextCodeFold::new(7, 28);
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut style = nana_ui_runtime::NodeStyle::default();
        std::sync::Arc::make_mut(&mut style.layout).padding_left =
            Some(nana_ui_core::LengthSpec::Px(46.0));
        let area = context
            .create_component(
                document,
                TextArea::new(value)
                    .code_editor(true)
                    .line_numbers(true)
                    .code_folds(std::sync::Arc::from([fold]))
                    .style(style),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 40.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();

        // 光标移到折叠起始行，避免聚焦滚动把该行推出视口。
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = nana_ui_runtime::TextSelection::caret(3);
            })
            .unwrap();
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);

        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down || phase == PointerPhase::Move),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        // 折叠前：文档末行的 reveal 行距按 5 行计。
        let reveal_before = context
            .world()
            .text_input_reveal_scroll(node, value.len())
            .unwrap();
        let gutter = context
            .world()
            .component_geometry(node)
            .and_then(|geometry| match geometry {
                nana_ui_runtime::ComponentGeometry::TextInput { folds, .. } => {
                    folds.gutters.first().copied()
                }
                _ => None,
            })
            .expect("fold gutter geometry");
        let center = (
            gutter.bounds.x + gutter.bounds.width / 2.0,
            gutter.bounds.y + gutter.bounds.height / 2.0,
        );

        // 点击 gutter 箭头：折叠该区间（消费事件、不落光标）。
        assert!(
            context
                .pointer_target(document, center.0, center.1)
                .is_some(),
            "no hit target at {:?}",
            center
        );
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(center.0, center.1, PointerPhase::Down),
                    Duration::from_millis(1_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(center.0, center.1, PointerPhase::Up),
                Duration::from_millis(1_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(context.world().text_fold_collapsed(node), vec![fold]);
        let work = context.take_system_work();
        context.world_mut().resolve_styles(&work.style).unwrap();
        context
            .world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();

        // 折叠后渲染行数减少：同一偏移的 reveal 行距按显示视图换算变小。
        let reveal_after = context
            .world()
            .text_input_reveal_scroll(node, value.len())
            .unwrap();
        assert!(reveal_after.y < reveal_before.y);

        // 再次点击箭头：展开。
        let gutter = context
            .world()
            .component_geometry(node)
            .and_then(|geometry| match geometry {
                nana_ui_runtime::ComponentGeometry::TextInput { folds, .. } => {
                    folds.gutters.first().copied()
                }
                _ => None,
            })
            .expect("fold gutter geometry");
        let center = (
            gutter.bounds.x + gutter.bounds.width / 2.0,
            gutter.bounds.y + gutter.bounds.height / 2.0,
        );
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(center.0, center.1, PointerPhase::Down),
                    Duration::from_millis(2_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(center.0, center.1, PointerPhase::Up),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(context.world().text_fold_collapsed(node).is_empty());
    }

    #[test]
    fn escape_collapses_additional_cursors_and_single_cursor_escape_passes_through() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let escape = || InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 多光标：Esc 塌缩到主光标并消费事件。
        set_selections(&mut context, area, (1, 1), vec![(4, 4)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &escape())
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncd\nef".into(), (1, 1), vec![])
        );

        // 单光标：Esc 不消费（宿主继续处理），选区不变。
        assert!(
            !adapter
                .dispatch(&mut context, document, &escape())
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncd\nef".into(), (1, 1), vec![])
        );
    }

    #[test]
    fn escape_ends_snippet_session_before_collapsing_cursors() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("s", "[$1]$0"),
                )
                .unwrap()
        );
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 第一个 Esc：只结束 snippet 会话，多光标保留。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("[]ab\ncd".into(), (2, 2), vec![(7, 7)])
        );

        // 第二个 Esc：塌缩多光标。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("[]ab\ncd".into(), (2, 2), vec![])
        );
    }

    #[test]
    fn snippet_session_tab_routes_through_the_adapter_before_indent() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("if", "if $1 {$0"),
                )
                .unwrap()
        );
        let mut adapter = RuntimeInputAdapter::default();

        // 会话内 Tab 跳位（消费且不插入缩进）。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("if  {".into(), 3, 3));

        // 会话结束后 Tab 回到缩进行为。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_ne!(textarea_selection(&context, node).0, "if  {");
    }

    #[test]
    fn multi_cursor_typing_deleting_and_newline_edit_every_selection() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        set_selections(&mut context, area, (1, 1), vec![(4, 4)]);

        // 打字：每个光标各插入一个字符，一次事件。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("x", Some("x"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("axb\ncxd".into(), (2, 2), vec![(6, 6)])
        );
        assert_eq!(events.lock().unwrap().len(), 1);

        // 退格：每个光标各删除一个字符。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Backspace", None, InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            // 第二个光标删掉 c 后的 x，落在 c、d 之间（偏移 4）。
            ("ab\ncd".into(), (1, 1), vec![(4, 4)])
        );
        assert_eq!(events.lock().unwrap().len(), 2);

        // Enter：每个光标各换一行（无代码编辑，无自动缩进）。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Enter", Some("\n"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("a\nb\nc\nd".into(), (2, 2), vec![(6, 6)])
        );
        assert_eq!(events.lock().unwrap().len(), 3);
    }

    #[test]
    fn alt_cmd_arrows_add_cursors_by_column_skip_duplicates_and_stay_at_edges() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abcd\nef\nghij"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let alt_cmd = InputModifiers {
            alt: true,
            meta: true,
            ..InputModifiers::default()
        };
        set_selections(&mut context, area, (2, 2), vec![]);

        // Alt+Cmd+Down 在下一行按列加光标；列超出则贴到行尾。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("abcd\nef\nghij".into(), (2, 2), vec![(7, 7)])
        );

        // 再按一次：第二个光标下方按列对齐，第一行光标的候选与已有重复合。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("abcd\nef\nghij".into(), (2, 2), vec![(7, 7), (10, 10)])
        );

        // 文档边缘：手势仍被消费，但不再新增光标。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node).2,
            vec![(7, 7), (10, 10)]
        );

        // Alt+Cmd+Up 回程同样按列对齐（10 -> 7 -> 2 依次被已有光标去重）。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt_cmd))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node).2,
            vec![(7, 7), (10, 10)]
        );
    }

    #[test]
    fn cmd_d_selects_occurrences_wrapping_and_skipping_covered_spans() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };
        set_selections(&mut context, area, (0, 2), vec![]);

        // Cmd+D 选中下一个 "ab"（全词匹配）。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (0, 2), vec![(6, 8)])
        );

        // 全部出现都已有光标：不再新增（键不被消费）。
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );

        // 环形：只留末尾选区时，Cmd+D 绕回文档开头。
        set_selections(&mut context, area, (6, 8), vec![]);
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (6, 8), vec![(0, 2)])
        );

        // 全部选中：裸光标取光标下的词，选中所有出现。
        set_selections(&mut context, area, (1, 1), vec![]);
        assert!(
            context
                .select_all_focused_text_occurrences(document)
                .unwrap()
        );
        // 裸光标被它所在的词选区吸收（并集后主光标即该词）。
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (0, 2), vec![(6, 8)])
        );

        // 收回到主光标；再次收回是空操作。
        assert!(context.collapse_focused_text_selections(document).unwrap());
        assert_eq!(textarea_selections(&context, node).2, vec![]);
        assert!(!context.collapse_focused_text_selections(document).unwrap());
    }

    #[test]
    fn copy_joins_multi_cursor_selections_and_paste_hits_every_cursor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let meta = |key: &str| {
            edit_key(
                key,
                None,
                InputModifiers {
                    meta: true,
                    ..InputModifiers::default()
                },
            )
        };
        set_selections(&mut context, area, (0, 2), vec![(3, 5)]);

        // Cmd+C：多选区按序拼接（Zed 语义，换行连接）。
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("ab\ncd")
        );

        // Cmd+V：同一段文本插入到每个光标。
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncdab cdab\ncd".into(), (5, 5), vec![(15, 15)])
        );

        // Cmd+X：多选区剪切一并删除。
        set_selections(&mut context, area, (0, 2), vec![(8, 10)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).0, "\ncdab ab\ncd");
    }

    #[test]
    fn ime_commit_scopes_to_the_primary_cursor_and_remaps_others() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let adapter = RuntimeInputAdapter::default();
        set_selections(&mut context, area, (2, 2), vec![(4, 4)]);

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("X".into()))
                .unwrap()
                .prevent_default
        );
        // 只有主光标收到提交文本，附加光标随编辑平移。
        assert_eq!(
            textarea_selections(&context, node),
            ("abX\ncd".into(), (3, 3), vec![(5, 5)])
        );
    }

    #[test]
    fn single_line_fields_reject_multi_cursor_gestures() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hi"))
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let alt_cmd = InputModifiers {
            alt: true,
            meta: true,
            ..InputModifiers::default()
        };
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };

        // 命令层直接拒绝。
        assert!(
            !context
                .add_focused_text_cursor(document, false, None)
                .unwrap()
        );
        assert!(
            !context
                .select_focused_text_occurrence(document, false)
                .unwrap()
        );

        // Alt+Cmd+Down 回落到普通移动（meta=DocEnd），不加光标。
        // 先把光标挪到行首，DocEnd 才有位移。
        context
            .update_component(input, |input, _cx| {
                input.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).1, (2, 2));
        assert_eq!(textarea_selections(&context, node).2, vec![]);

        // Cmd+D 不消费、不产生附加光标。
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).2, vec![]);
    }

    #[test]
    fn alt_click_adds_and_removes_cursors_and_plain_click_collapses() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("first\nsecond"))
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 64.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let click = |x: f32, y: f32, alt: bool| InputEvent::Pointer {
            phase: PointerPhase::Down,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: 1,
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: if alt {
                InputModifiers {
                    alt: true,
                    ..InputModifiers::default()
                }
            } else {
                InputModifiers::default()
            },
        };
        struct Ctx<'a> {
            context: &'a mut AppContext,
            shaper: &'a mut MeasureTextShaper,
        }
        let mut ctx = Ctx {
            context: &mut context,
            shaper: &mut shaper,
        };
        let mut dispatch_down = |ctx: &mut Ctx, event: &InputEvent, at: u64| -> bool {
            adapter
                .dispatch_with_shaper(
                    ctx.context,
                    document,
                    event,
                    Duration::from_millis(at),
                    Some(&mut *ctx.shaper),
                )
                .unwrap()
                .prevent_default
        };

        // 先用普通点击探出 (2, 20) 落点的字符偏移（不依赖具体行高）。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, false), 1_000));
        let probe = textarea_selections(ctx.context, node).1;
        assert_eq!(probe.0, probe.1);
        // 把主光标挪到文档末尾，让目标点空出来。
        ctx.context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret("first\nsecond".len());
            })
            .unwrap();

        // Alt+点击同一点：新增一个光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, true), 2_000));
        assert_eq!(
            textarea_selections(ctx.context, node),
            ("first\nsecond".into(), (12, 12), vec![probe])
        );

        // 时间错开避免双击判定；再次 Alt+点击同一点：移除该光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, true), 3_000));
        assert_eq!(textarea_selections(ctx.context, node).2, vec![]);

        // Alt+点击第一行行首（主光标不在该处）：新增光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 4.0, true), 4_000));
        assert_eq!(textarea_selections(ctx.context, node).2, vec![(0, 0)]);

        // 普通点击：天然塌缩回单光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 4.0, false), 5_000));
        assert_eq!(
            textarea_selections(ctx.context, node),
            ("first\nsecond".into(), (0, 0), vec![])
        );
    }

    #[test]
    fn multi_cursor_code_editing_indents_comments_moves_lines_and_deletes_words() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("  a\n  b").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let control = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        let alt = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };

        // Enter 自动缩进：两个缩进行上的光标各起新行并继承缩进。
        set_selections(&mut context, area, (3, 3), vec![(7, 7)]);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Enter", Some("\n"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("  a\n  \n  b\n  ".into(), (6, 6), vec![(13, 13)])
        );

        // Ctrl+/ 注释切换：每个光标注释自己所在的行。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(4)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, control))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("//aa\n//bb".into(), (3, 3), vec![(8, 8)])
        );

        // Alt+Backspace 词删除：每个光标删到词首。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(4)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("Backspace", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("a\nb".into(), (0, 0), vec![(2, 2)])
        );

        // Alt+Down 行移动：首行光标把行下移；末行光标在边缘保持不动。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb\ncc");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(7)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowDown", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("bb\naa\ncc".into(), (4, 4), vec![(7, 7)])
        );
    }

    fn completion_items(labels: &[&str]) -> std::sync::Arc<[nana_ui_runtime::TextCompletion]> {
        labels
            .iter()
            .map(|label| nana_ui_runtime::TextCompletion::new(*label, "fn"))
            .collect::<Vec<_>>()
            .into()
    }

    /// 布局 + shape + 命中测试的完整几何环境（指针/滚轮路由需要）。
    fn shape_completion_editor(context: &mut AppContext, document: DocumentId, node: StableNodeId) {
        context.world_mut().resolve_styles(&[node]).unwrap();
        context
            .world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);
    }

    fn completion_popup_geometry(
        context: &AppContext,
        node: StableNodeId,
    ) -> nana_ui_runtime::TextCompletionPopup {
        match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput {
                completion_popup, ..
            }) => completion_popup.expect("completion popup geometry"),
            _ => panic!("text input geometry"),
        }
    }

    #[test]
    fn completion_popup_owns_navigation_and_accept_keys() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("let fo").completions(completion_items(&["food", "foobar"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (6, 6), vec![]);
        let mut adapter = RuntimeInputAdapter::default();
        let selected = |context: &AppContext, node| {
            context
                .world()
                .text_completion_snapshot(node)
                .map(|snapshot| (snapshot.selected, snapshot.dismissed))
        };

        // Down：弹层消费（选区不动），候选选中移到第二条。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("let fo".into(), 6, 6));
        assert_eq!(selected(&context, node), Some((1, false)));

        // Up：回到第一条。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(selected(&context, node), Some((0, false)));

        // Enter：接受选中项，一次 TextChanged，光标落在插入末尾。
        let changes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&changes);
        context
            .on(area, move |_view, event: &TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.clone());
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let food".into(), 8, 8)
        );
        assert_eq!(*changes.lock().unwrap(), vec!["let food".to_string()]);

        // 宿主重喂（组件重投影）：会话重新激活，Tab 同样接受。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["food"]);
            })
            .unwrap();
        assert_eq!(selected(&context, node), Some((0, false)));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let food".into(), 8, 8)
        );

        // 打字穿透正常编辑：committed value 直接更新（弹层保持，过滤
        // 由宿主重喂驱动）。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["food"]);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("s", Some("s"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let foods".into(), 9, 9)
        );
    }

    #[test]
    fn modified_keys_pass_through_while_completion_active() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("ab ab").completions(completion_items(&["ab"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (1, 1), vec![]);
        let mut adapter = RuntimeInputAdapter::default();

        // Cmd+D 穿透：选中下一出现（多光标 +1），弹层保持。
        let meta_d = edit_key(
            "d",
            None,
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_d)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab ab".into(), (1, 1), vec![(3, 5)])
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| !snapshot.dismissed)
        );
    }

    #[test]
    fn escape_closes_completion_after_snippet_and_before_collapse() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("ab\ncd").completions(completion_items(&["ab"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("s", "[$1]$0"),
                )
                .unwrap()
        );
        // snippet 插入后宿主重喂（组件重投影路径）：弹层重新激活。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["ab"]);
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 第一个 Esc：结束 snippet 会话（弹层与多光标保留）。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| !snapshot.dismissed)
        );
        assert_eq!(textarea_selections(&context, node).2.len(), 1);

        // 第二个 Esc：关闭补全弹层（多光标保留）。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| snapshot.dismissed)
        );
        assert_eq!(textarea_selections(&context, node).2.len(), 1);

        // 第三个 Esc：塌缩多光标。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).2, vec![]);
    }

    #[test]
    fn completion_click_accepts_row_and_wheel_scrolls_overlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("")
                    .completions(completion_items(&["alpha", "beta", "gamma", "delta"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 140.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        shape_completion_editor(&mut context, document, node);

        // 点击弹层第二行：接受该候选（beta），不落光标。
        let popup = completion_popup_geometry(&context, node);
        let row = &popup.rows[1];
        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(row.bounds.x + 2.0, row.bounds.y + 2.0, PointerPhase::Down),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("beta".into(), 4, 4));

        // 重喂十条候选并重建几何：滚轮落在弹层面板内滚动弹层（消费），
        // 不落到编辑器滚动。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&[
                    "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "a10",
                ]);
                view.state.selection = nana_ui_runtime::TextSelection::caret(4);
            })
            .unwrap();
        shape_completion_editor(&mut context, document, node);
        let popup = completion_popup_geometry(&context, node);
        let scroll = |context: &AppContext| {
            context
                .world()
                .text_completion_snapshot(node)
                .map(|snapshot| snapshot.scroll)
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(popup.panel.x + 3.0, popup.panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(scroll(&context), Some(1));
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(popup.panel.x + 3.0, popup.panel.y + 3.0, -3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(scroll(&context), Some(0));

        // hover 浮窗滚轮：正文按行滚动并被消费。
        context
            .update_component(area, |view, _| {
                view.hover = Some(nana_ui_runtime::TextHover::new(
                    0,
                    "beta",
                    "one\ntwo\nthree",
                ));
            })
            .unwrap();
        shape_completion_editor(&mut context, document, node);
        let hover_panel = match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput { hover_popup, .. }) => {
                hover_popup.expect("hover popup").panel
            }
            _ => panic!("text input geometry"),
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(hover_panel.x + 3.0, hover_panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);
    }

    #[test]
    fn hover_wheel_scrolls_without_editor_focus() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("alpha beta").hover(Some(nana_ui_runtime::TextHover::new(
                    6,
                    "beta",
                    "one\ntwo\nthree",
                ))),
            )
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 140.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        shape_completion_editor(&mut context, document, node);
        let hover_panel = match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput { hover_popup, .. }) => {
                hover_popup.expect("hover popup").panel
            }
            _ => panic!("text input geometry"),
        };
        let mut adapter = RuntimeInputAdapter::default();

        // 编辑器未聚焦：滚轮落在 hover 面板内仍滚动该面板（命中测试驱动，
        // hover 显示不要求焦点）。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(hover_panel.x + 3.0, hover_panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);

        // 面板外：不消费，落回编辑器/文档滚动。
        assert!(
            !adapter
                .dispatch(&mut context, document, &wheel(-50.0, -50.0, 3.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);
    }
}
