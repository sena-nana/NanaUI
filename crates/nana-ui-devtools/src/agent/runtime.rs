//! L3 Runtime agent session. Vue-free: no JS engine, no Vue renderer.

use std::collections::BTreeMap;
use std::path::Path;

use nana_ui::runtime::{
    AccessibilityAction, AccessibilityActionRequest, LayoutViewport, RuntimeDocument, StableNodeId,
};
use nana_ui::{NanaTextShaper, RuntimeInputAdapter};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use super::{AccessibilityDumpNode, AgentError, DEFAULT_CLEAR, dump_accessibility_node};
use crate::offscreen::{self, OffscreenSnapshots, Size};

/// L3 Runtime document driven without a winit window.
pub struct RuntimeAgentSession {
    document: RuntimeDocument,
    shaper: NanaTextShaper,
    gpu: Option<OffscreenSnapshots>,
    scale_factor: f32,
    width: u32,
    height: u32,
    clear: [f32; 4],
}

impl RuntimeAgentSession {
    pub fn new(document: RuntimeDocument, width: u32, height: u32) -> Result<Self, AgentError> {
        Self::new_scaled(document, width, height, 1.0)
    }

    /// Width and height are logical pixels; PNG dimensions include the scale.
    pub fn new_scaled(
        document: RuntimeDocument,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<Self, AgentError> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(AgentError(
                "snapshot scale must be finite and positive".into(),
            ));
        }
        let mut session = Self {
            scale_factor,
            document,
            shaper: NanaTextShaper::default(),
            gpu: None,
            width,
            height,
            clear: DEFAULT_CLEAR,
        };
        session.flush()?;
        Ok(session)
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    pub fn flush(&mut self) -> Result<(), AgentError> {
        self.document
            .flush(
                LayoutViewport::new(self.width as f32, self.height as f32),
                &mut self.shaper,
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    pub fn accessibility_dump(&self) -> Vec<AccessibilityDumpNode> {
        self.document
            .context()
            .world()
            .project_accessibility(self.document.document())
            .into_iter()
            .map(|node| dump_accessibility_node(node, &BTreeMap::new()))
            .collect()
    }

    pub fn click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        dispatch_runtime_pointer(&mut self.document, PointerPhase::Down, x, y)?;
        dispatch_runtime_pointer(&mut self.document, PointerPhase::Up, x, y)?;
        self.flush()?;
        Ok(true)
    }

    /// Secondary-button click (button 2), the way a right-click reaches the
    /// tree. Context menus open on the press, so both phases are sent with the
    /// button held then released — a primary [`Self::click_xy`] never routes
    /// there, and hand-rolling the `InputEvent` is the same boilerplate in
    /// every consumer that wants to verify a context menu headlessly.
    pub fn secondary_click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        dispatch_runtime_button(
            &mut self.document,
            PointerPhase::Down,
            x,
            y,
            2,
            button_mask(2),
        )?;
        dispatch_runtime_button(&mut self.document, PointerPhase::Up, x, y, 2, 0)?;
        self.flush()?;
        Ok(true)
    }

    pub fn click_node(&mut self, id: u64) -> Result<bool, AgentError> {
        let target =
            StableNodeId::new(id).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        let document_id = self.document.document();
        let handled = self
            .document
            .context_mut()
            .apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::Click,
                },
            )
            .map_err(|error| AgentError(error.to_string()))?;
        self.flush()?;
        Ok(handled)
    }

    pub fn type_text(&mut self, text: &str) -> Result<(), AgentError> {
        let document_id = self.document.document();
        for character in text.chars() {
            let key = character.to_string();
            RuntimeInputAdapter::default()
                .dispatch(
                    self.document.context_mut(),
                    document_id,
                    &InputEvent::Keyboard {
                        pressed: true,
                        key: key.clone(),
                        text: Some(key.clone()),
                        code: "Unidentified".into(),
                        repeat: false,
                        modifiers: InputModifiers::default(),
                    },
                )
                .map_err(|error| AgentError(error.to_string()))?;
        }
        self.flush()?;
        Ok(())
    }

    /// Wheel-scroll at a point, in logical pixels. Positive `delta_y` scrolls
    /// content up (the same sign the platform reports). Retained scroll,
    /// nested-clip hit testing and virtual materialisation only misbehave once
    /// something has actually scrolled, so a headless session that cannot
    /// scroll cannot reproduce that whole class of defect.
    pub fn scroll_by(
        &mut self,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<(), AgentError> {
        let document_id = self.document.document();
        RuntimeInputAdapter::default()
            .dispatch(
                self.document.context_mut(),
                document_id,
                &InputEvent::Wheel {
                    x,
                    y,
                    delta_x,
                    delta_y,
                    line_delta: false,
                    modifiers: InputModifiers::default(),
                },
            )
            .map_err(|error| AgentError(error.to_string()))?;
        self.flush()?;
        Ok(())
    }

    /// Move the pointer without pressing, so hover-only presentation (tooltips,
    /// hover cards, row affordances) can be captured.
    pub fn hover_xy(&mut self, x: f32, y: f32) -> Result<(), AgentError> {
        dispatch_runtime_pointer(&mut self.document, PointerPhase::Move, x, y)?;
        self.flush()?;
        Ok(())
    }

    /// Press and release one named key. `key` and `code` follow the platform
    /// input contract (`"Escape"`, `"ArrowDown"`, `"Enter"`, …); no text is
    /// committed, which is what separates navigation from [`Self::type_text`].
    pub fn key_press(
        &mut self,
        key: &str,
        code: &str,
        modifiers: InputModifiers,
    ) -> Result<(), AgentError> {
        let document_id = self.document.document();
        for pressed in [true, false] {
            RuntimeInputAdapter::default()
                .dispatch(
                    self.document.context_mut(),
                    document_id,
                    &InputEvent::Keyboard {
                        pressed,
                        key: key.to_owned(),
                        text: None,
                        code: code.to_owned(),
                        repeat: false,
                        modifiers,
                    },
                )
                .map_err(|error| AgentError(error.to_string()))?;
        }
        self.flush()?;
        Ok(())
    }

    pub fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        self.flush()?;
        let size = Size::new(
            (self.width as f32 * self.scale_factor).round() as u32,
            (self.height as f32 * self.scale_factor).round() as u32,
        );
        let scale = self.scale_factor;
        let clear = self.clear;
        let scene = self.document.scene().clone();
        let gpu = self.gpu_mut()?;
        let pixels = gpu
            .paint_scaled(&scene, size, scale, clear)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok((size, pixels))
    }

    pub fn screenshot_png(&mut self, path: impl AsRef<Path>) -> Result<(), AgentError> {
        let (size, pixels) = self.screenshot_rgba()?;
        offscreen::write_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(())
    }

    fn gpu_mut(&mut self) -> Result<&mut OffscreenSnapshots, AgentError> {
        if self.gpu.is_none() {
            self.gpu =
                Some(OffscreenSnapshots::new().map_err(|error| AgentError(error.to_string()))?);
        }
        Ok(self.gpu.as_mut().expect("gpu initialized"))
    }
}

/// Platform button mask for a button index, matching the hosted adapter.
const fn button_mask(button: i16) -> u16 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        _ => 0,
    }
}

fn dispatch_runtime_pointer(
    document: &mut RuntimeDocument,
    phase: PointerPhase,
    x: f32,
    y: f32,
) -> Result<(), AgentError> {
    dispatch_runtime_button(document, phase, x, y, 0, 0)
}

fn dispatch_runtime_button(
    document: &mut RuntimeDocument,
    phase: PointerPhase,
    x: f32,
    y: f32,
    button: i16,
    buttons: u16,
) -> Result<(), AgentError> {
    let document_id = document.document();
    RuntimeInputAdapter::default()
        .dispatch(
            document.context_mut(),
            document_id,
            &InputEvent::Pointer {
                phase,
                pointer_id: 1,
                pointer_type: PointerType::Mouse,
                x,
                y,
                screen_x: x,
                screen_y: y,
                button,
                buttons,
                pressure: 0.5,
                tangential_pressure: 0.0,
                tilt_x: 0,
                tilt_y: 0,
                twist: 0,
                is_primary: true,
                activation_click: false,
                modifiers: InputModifiers::default(),
            },
        )
        .map_err(|error| AgentError(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::runtime::{Button, DocumentId, List, Text};

    /// GPU-less environments skip pixel evidence, and the skip must stay observable.
    fn offscreen_gpu() -> Option<OffscreenSnapshots> {
        match OffscreenSnapshots::new() {
            Ok(gpu) => Some(gpu),
            Err(error) => {
                eprintln!("skipping offscreen GPU evidence: {error}");
                None
            }
        }
    }
    #[test]
    fn runtime_session_click_node_and_optional_preview() {
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let button = document
            .context_mut()
            .build(document_id, |ui| {
                ui.with("root", List::new().label("Agent"), |ui| {
                    ui.child("label", Text::new("idle"));
                    ui.child("go", Button::new("Go"))
                })
            })
            .expect("root");
        let mut session = RuntimeAgentSession::new(document, 240, 160).expect("runtime session");
        assert!(
            session
                .accessibility_dump()
                .iter()
                .any(|node| node.role == "button")
        );
        let handled = session.click_node(button.stable_id().get()).expect("click");
        assert!(handled);
        if offscreen_gpu().is_some() {
            let (size, pixels) = session.screenshot_rgba().expect("preview");
            assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
            assert!(pixels.iter().any(|channel| *channel != 0));
        }
    }

    /// Real GPU evidence that `CustomRenderNode::params` reaches the painter:
    /// changing only a `GpuView` palette must change the painted pixels.
    #[test]
    fn gpu_view_palette_params_reach_the_default_painter() {
        use nana_ui::default_scene_gpu_renderers;
        use nana_ui::runtime::{GpuView, GpuViewPalette};

        const WIDTH: u32 = 96;
        const HEIGHT: u32 = 96;

        let Some(mut gpu) = offscreen_gpu() else {
            return;
        };
        let renderers = default_scene_gpu_renderers();
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let view = document
            .context_mut()
            .build(document_id, |ui| {
                ui.child(
                    "view",
                    GpuView::new(1).palette(GpuViewPalette {
                        background: [1.0, 0.0, 0.0, 1.0],
                        accent: [1.0, 0.0, 0.0, 1.0],
                    }),
                )
            })
            .expect("gpu view");
        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");

        // `readback` returns RGBA; sum each channel over the whole frame.
        let mut paint = |session: &mut RuntimeAgentSession| {
            session.flush().expect("flush");
            let scene = session.document().scene().clone();
            let pixels = gpu
                .paint(
                    &scene,
                    Size::new(WIDTH, HEIGHT),
                    [0.0, 0.0, 0.0, 1.0],
                    None,
                    Some(&renderers),
                )
                .expect("offscreen paint with the gpu-view renderer");
            let (rgba_pixels, _) = pixels.as_chunks::<4>();
            rgba_pixels.iter().fold([0u64; 3], |mut acc, rgba| {
                acc[0] += u64::from(rgba[0]);
                acc[1] += u64::from(rgba[1]);
                acc[2] += u64::from(rgba[2]);
                acc
            })
        };

        let red = paint(&mut session);
        assert!(
            red[0] > red[1] && red[0] > red[2],
            "the red palette must paint red-dominant, got {red:?}"
        );

        session
            .document_mut()
            .context_mut()
            .update_component(view, |view, _| {
                view.palette = GpuViewPalette {
                    background: [0.0, 1.0, 0.0, 1.0],
                    accent: [0.0, 1.0, 0.0, 1.0],
                };
                view.invalidate_content();
            })
            .expect("recolor");
        let green = paint(&mut session);
        assert!(
            green[1] > green[0] && green[1] > green[2],
            "the recolored palette must reach the painter, got {green:?}"
        );
    }

    /// Real GPU evidence that resident scrollbar chrome lands on the scrollport
    /// edge: the right-hand columns must brighten once the bar is drawn.
    #[test]
    fn resident_scrollbar_paints_pixels_on_the_scrollport_edge() {
        use nana_ui::runtime::{LengthSpec, NodeStyle, ScrollAxes, ScrollView, Text};
        use nana_ui_core::ScrollbarVisibility;

        const WIDTH: u32 = 160;
        const HEIGHT: u32 = 120;

        let Some(mut gpu) = offscreen_gpu() else {
            return;
        };
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let mut viewport = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut viewport.layout);
            layout.width = Some(LengthSpec::Px(WIDTH as f32));
            layout.height = Some(LengthSpec::Px(HEIGHT as f32));
        }
        let scroll = document
            .context_mut()
            .build(document_id, |ui| {
                let scroll = ui.child(
                    "scroll",
                    ScrollView::new(ScrollAxes::Vertical)
                        .scrollbars(ScrollbarVisibility::Always)
                        .style(viewport),
                );
                ui.nest(scroll, |ui| {
                    for index in 0..8 {
                        let mut row = NodeStyle::default();
                        {
                            let layout = std::sync::Arc::make_mut(&mut row.layout);
                            layout.width = Some(LengthSpec::Fill);
                            layout.height = Some(LengthSpec::Px(40.0));
                        }
                        ui.child(
                            format!("row-{index}"),
                            Text::new(format!("Row {index}")).style(row),
                        );
                    }
                });
                scroll
            })
            .expect("scroll view");
        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");

        // Brightness of the rightmost track-thick band versus the same band on
        // the opposite edge, which never carries chrome.
        let mut edges = |session: &mut RuntimeAgentSession| {
            session.flush().expect("flush");
            let scene = session.document().scene().clone();
            let pixels = gpu
                .paint(
                    &scene,
                    Size::new(WIDTH, HEIGHT),
                    [0.0, 0.0, 0.0, 1.0],
                    None,
                    None,
                )
                .expect("offscreen paint");
            let band = nana_ui_core::SCROLLBAR_METRICS.thickness as u32;
            let mut right = 0u64;
            let mut left = 0u64;
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let offset = ((y * WIDTH + x) * 4) as usize;
                    let luma = u64::from(pixels[offset])
                        + u64::from(pixels[offset + 1])
                        + u64::from(pixels[offset + 2]);
                    if x >= WIDTH - band {
                        right += luma;
                    } else if x < band {
                        left += luma;
                    }
                }
            }
            (left, right)
        };

        let (left, right) = edges(&mut session);
        assert!(
            right > left,
            "the resident bar must brighten the right edge: left {left}, right {right}"
        );

        session
            .document_mut()
            .context_mut()
            .update_component(scroll, |scroll, _| {
                scroll.scrollbars = ScrollbarVisibility::Hidden;
            })
            .expect("hide bars");
        let (_, hidden_right) = edges(&mut session);
        assert!(
            hidden_right < right,
            "hiding the bar must remove those pixels: {hidden_right} vs {right}"
        );
    }

    #[test]
    fn divider_and_radio_selection_reach_pixels() {
        use nana_ui::runtime::{
            Card, Divider, LengthSpec, NodeStyle, SegmentedControl, SegmentedOption,
        };
        use nana_ui_core::FlexDirection;

        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 360;

        if offscreen_gpu().is_none() {
            return;
        }
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let mut column = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut column.layout);
            layout.width = Some(LengthSpec::Px(WIDTH as f32));
            layout.height = Some(LengthSpec::Px(HEIGHT as f32));
            layout.direction = Some(FlexDirection::Column);
            layout.gap = Some(LengthSpec::Px(12.0));
            layout.padding = Some(LengthSpec::Px(16.0));
        }
        let (radios, first, second, divider) = document
            .context_mut()
            .build(document_id, |ui| {
                ui.with("root", Card::new().style(column), |ui| {
                    let radios = ui.child("radios", SegmentedControl::radio_group());
                    let (first, second) = ui.nest(radios, |ui| {
                        let first = ui.child("auto", SegmentedOption::new("Automatic"));
                        let second = ui.child("manual", SegmentedOption::new("Manual"));
                        (first, second)
                    });
                    let divider = ui.child("divider", Divider::horizontal());
                    (radios, first, second, divider)
                })
            })
            .expect("root");
        document
            .context_mut()
            .set_segmented_options(radios, vec![first, second], Some(second))
            .expect("select");

        let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT).expect("session");
        let boxes = |session: &RuntimeAgentSession, id| {
            session
                .document()
                .context()
                .world()
                .layout_box(id)
                .expect("layout")
        };
        let rule = boxes(&session, divider.stable_id());
        let unselected = boxes(&session, first.stable_id());
        let selected = boxes(&session, second.stable_id());
        let (_, pixels) = session.screenshot_rgba().expect("pixels");
        let luma = |x: f32, y: f32| {
            let offset = ((y.round() as u32 * WIDTH + x.round() as u32) * 4) as usize;
            u32::from(pixels[offset])
                + u32::from(pixels[offset + 1])
                + u32::from(pixels[offset + 2])
        };

        let row = luma(rule.x + rule.width * 0.5, rule.y);
        let above = luma(rule.x + rule.width * 0.5, rule.y - 4.0);
        assert!(
            row > above,
            "the hairline rule must paint its own row: {row} vs {above}"
        );

        // The ring sits a fixed inset in from the option's leading edge; only
        // the selected one carries a filled dot at its center.
        let dot_x = |option: nana_ui::runtime::LayoutBox| {
            option.x
                + nana_ui_core::RADIO_ROW_INSET
                + nana_ui_core::ControlSize::Medium.indicator_size() / 2.0
        };
        let selected_dot = luma(dot_x(selected), selected.y + selected.height * 0.5);
        let empty_ring = luma(dot_x(unselected), unselected.y + unselected.height * 0.5);
        assert!(
            selected_dot > empty_ring,
            "only the selected radio fills its ring: {selected_dot} vs {empty_ring}"
        );
    }
}
