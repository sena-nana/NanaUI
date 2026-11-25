use super::*;
use crate::view_components::TextAreaResizeDrag;

static TEXT_AREA_RESIZE_DRAG_OPS: crate::framework::ActiveDragOps =
    crate::framework::ActiveDragOps {
        pre_gesture: true,
        cancel_on_touch_scroll: false,
        update: |cx, doc, _, ptr, x, y, _| cx.update_text_area_resize(doc, ptr, x, y),
        end: |cx, doc, _, ptr, _, _, _, cancel| cx.end_text_area_resize(doc, ptr, cancel),
    };

impl AppContext {
    pub(crate) fn text_area_cursor_at(
        &self,
        node: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<nana_ui_input::CursorIcon> {
        let entity = self.view_entity::<TextArea>(node)?;
        if !self
            .read(entity, |area| {
                area.supports_vertical_resize() && !area.disabled
            })
            .ok()?
        {
            return None;
        }
        let (lx, ly) = self.world.pointer_layout_position(node, x, y)?;
        let grip = self
            .world
            .text_input_geometry(node)
            .and_then(|g| g.resize_grip)?;
        grip.contains(lx, ly)
            .then_some(nana_ui_input::CursorIcon::NsResize)
    }

    /// Starts a height-only drag on the editor's projected resize grip.
    pub fn begin_text_area_resize(
        &mut self,
        pointer: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.view_entity::<TextArea>(target) else {
            return Ok(false);
        };
        let Some(document) = self.world.document_of(target) else {
            return Ok(false);
        };
        let mut restored_height = None;
        if let Some(drag) = self.read(entity, |area| area.resize_drag)?
            && (drag.pointer == pointer
                || self.world.pointer_capture(document, drag.pointer) != Some(target))
        {
            restored_height = Some(drag.start_height);
            self.component_lifecycle
                .text_area_resizes
                .retain(|_, value| *value != target);
            let owns_capture = self.world.pointer_capture(document, drag.pointer) == Some(target);
            self.update_component(entity, |area, cx| {
                if owns_capture {
                    cx.mutations().release_pointer(drag.pointer, target);
                }
                area.resized_height = drag.previous_height;
                area.resize_drag = None;
            })?;
        }
        if !self.read(entity, |area| {
            area.supports_vertical_resize() && !area.disabled && area.resize_drag.is_none()
        })? {
            return Ok(false);
        }
        let Some((x, y)) = self.world.pointer_layout_position(target, x, y) else {
            return Ok(false);
        };
        let Some(grip) = self
            .world
            .text_input_geometry(target)
            .and_then(|g| g.resize_grip)
        else {
            return Ok(false);
        };
        if !grip.contains(x, y) {
            return Ok(false);
        }
        let Some(bounds) = self.world.component_layout_box(target) else {
            return Ok(false);
        };
        let content_box = self.read(entity, |area| {
            area.style.layout.box_sizing == nana_ui_core::BoxSizing::ContentBox
        })?;
        let (min_height, mut max_height) = self
            .world
            .resolved_height_limits(target)
            .unwrap_or((0.0, None));
        if content_box {
            // A content-box height is dragged in content units. Layout holds
            // it to `min-height` before adding padding and border and to
            // `max-height` after (`layout_engine::measure::finish_intrinsic_size`), so the upper
            // limit loses the chrome here.
            let chrome = self
                .world
                .component_content_box(target)
                .map_or(0.0, |content| (bounds.height - content.height).max(0.0));
            max_height = max_height.map(|max| (max - chrome).max(0.0));
        }
        let start_height = self.read(entity, |area| {
            if content_box {
                area.resized_height
                    .or_else(|| match area.style.layout.height {
                        Some(nana_ui_core::LengthSpec::Px(height)) => Some(height),
                        _ => None,
                    })
                    .unwrap_or(bounds.height)
            } else {
                bounds.height
            }
        })?;
        // Start from the height the limits let the field show, so the first
        // move already moves the edge.
        let start_height = crate::view_components::clamp_resized_height(
            restored_height.unwrap_or(start_height),
            min_height,
            max_height,
        );
        self.update_component(entity, |area, cx| {
            area.resize_drag = Some(TextAreaResizeDrag {
                pointer,
                start_y: y,
                start_height,
                previous_height: area.resized_height,
                min_height,
                max_height,
            });
            cx.mutations().capture_pointer(pointer, target);
        })?;
        self.component_lifecycle
            .text_area_resizes
            .insert((document, pointer), target);
        self.begin_active_drag(document, pointer, target, &TEXT_AREA_RESIZE_DRAG_OPS);
        Ok(true)
    }

    pub fn update_text_area_resize(
        &mut self,
        document: DocumentId,
        pointer: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self
            .component_lifecycle
            .text_area_resizes
            .get(&(document, pointer))
            .copied()
        else {
            return Ok(false);
        };
        let Some(entity) = self.view_entity::<TextArea>(target) else {
            return Ok(false);
        };
        let Some(drag) = self.read(entity, |area| area.resize_drag)? else {
            return Ok(false);
        };
        if drag.pointer != pointer {
            return Ok(false);
        }
        if self.world.pointer_capture(document, pointer) != Some(target)
            || self.read(entity, |area| {
                area.disabled || !area.supports_vertical_resize()
            })?
        {
            return self.end_text_area_resize(document, pointer, true);
        }
        let Some((_, y)) = self.world.pointer_layout_position(target, x, y) else {
            return Ok(false);
        };
        self.update_component(entity, |area, _| {
            area.resized_height = Some(drag.height_at(y));
        })?;
        Ok(true)
    }

    pub fn end_text_area_resize(
        &mut self,
        document: DocumentId,
        pointer: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self
            .component_lifecycle
            .text_area_resizes
            .remove(&(document, pointer))
        else {
            return Ok(false);
        };
        self.clear_active_drag(document, pointer, target);
        let Some(entity) = self.view_entity::<TextArea>(target) else {
            return Ok(false);
        };
        let Some(drag) = self.read(entity, |area| area.resize_drag)? else {
            return Ok(false);
        };
        if drag.pointer != pointer {
            return Ok(false);
        }
        let owns_capture = self.world.pointer_capture(document, pointer) == Some(target);
        let cancel = cancel || !owns_capture;
        self.update_component(entity, |area, cx| {
            if cancel || area.disabled || !area.supports_vertical_resize() {
                area.resized_height = drag.previous_height;
            }
            area.resize_drag = None;
            if owns_capture {
                cx.mutations().release_pointer(pointer, target);
            }
        })?;
        Ok(true)
    }
}
