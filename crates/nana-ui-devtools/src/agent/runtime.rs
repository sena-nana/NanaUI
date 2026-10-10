//! L3 Runtime agent session. Vue-free: no JS engine, no Vue renderer.

use std::path::Path;
use std::time::Duration;

use nana_ui::runtime::{
    AccessibilityAction, AccessibilityActionRequest, LayoutViewport, RuntimeDocument,
    RuntimeFrameUpdate, StableNodeId,
};
use nana_ui::{HeadlessInput, HostTextureRegistry, InputCounters, NanaTextShaper, ThemeAppearance};
use nana_ui_core::SemanticColorRole;
use nana_ui_platform::{
    InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, PointerInput,
    PointerPhase, WheelInput, WheelUnit,
};
use std::borrow::Cow;

use super::protocol::{
    HitDump, KeyStroke, PixelStats, PointerGesture, SceneProbeDump, SessionInfo, ThemeName,
};
use super::session::AgentSession;
use super::{AccessibilityDumpNode, AgentError, scene_probe};
use crate::offscreen::{self, OffscreenSnapshots, Size};

/// L3 Runtime document driven without a winit window.
pub struct RuntimeAgentSession {
    document: RuntimeDocument,
    shaper: NanaTextShaper,
    gpu: Option<OffscreenSnapshots>,
    scale_factor: f32,
    width: u32,
    height: u32,
    /// `None` follows the document's active theme. A fixed light clear made
    /// every dark-theme screenshot lie about its background.
    clear: Option<[f32; 4]>,
    host_textures: HostTextureRegistry,
    /// The session's input source: the same routing a native window's input
    /// takes, bound to this document, with a headless host behind it.
    ///
    /// Its clock moves one frame per dispatched event. A zero timestamp would
    /// silently disable every time-gated path in the Runtime (tooltip delay,
    /// the split handle hover probe); a fixed step rather than the wall clock
    /// keeps a scripted session reproducible.
    input: HeadlessInput,
}

/// One frame at 60Hz. What the input clock advances per dispatched event.
const INPUT_FRAME: Duration = Duration::from_millis(16);

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
        let document_id = document.document();
        let mut document = document;
        let input = HeadlessInput::bind(document.context_mut(), document_id);
        let mut session = Self {
            scale_factor,
            document,
            shaper: NanaTextShaper::default(),
            gpu: None,
            width,
            height,
            clear: None,
            host_textures: HostTextureRegistry::new(),
            input,
        };
        session.flush()?;
        Ok(session)
    }

    /// Host textures sampled by `nana.host-texture` nodes during a screenshot.
    /// Without a registry every Avatar, Thumbnail and video node paints its
    /// placeholder, which is indistinguishable from a binding that never landed.
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }

    /// Background actually used for the next screenshot.
    pub fn clear_color(&self) -> [f32; 4] {
        self.clear.unwrap_or_else(|| {
            let color = self
                .document
                .context()
                .world()
                .style_model()
                .color(SemanticColorRole::Background);
            [color.r, color.g, color.b, color.a]
        })
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    /// Input routing counters for adapter conformance and performance
    /// fixtures. Reading them does not schedule a frame or walk the document.
    pub fn input_counters(&self) -> InputCounters {
        self.document.context().input_counters()
    }

    /// Route one event at the next input frame.
    fn dispatch_input(&mut self, payload: InputPayload) -> Result<(), AgentError> {
        self.input.advance(INPUT_FRAME);
        self.input
            .route_shaped(self.document.context_mut(), payload, Some(&mut self.shaper))
            .map(|_| ())
            .map_err(|error| AgentError(error.to_string()))
    }

    fn dispatch_pointer(
        &mut self,
        phase: PointerPhase,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
    ) -> Result<(), AgentError> {
        self.dispatch_input(InputPayload::Pointer(PointerInput {
            button,
            buttons,
            pressure: 0.5,
            ..PointerInput::mouse(phase, x, y)
        }))
    }

    /// Deliver one headless pointer sample through the canonical input
    /// lowering and router. This is the reference adapter entry point for
    /// fixtures that need a precise drag rather than a click helper.
    pub fn pointer_event(
        &mut self,
        phase: PointerPhase,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
    ) -> Result<(), AgentError> {
        self.dispatch_pointer(phase, x, y, button, buttons)
    }

    /// Drains one frame. Work counters survive idle frames; per-frame
    /// accounting counts only a flush that is not [`RuntimeFrameUpdate::is_idle`].
    pub fn flush(&mut self) -> Result<RuntimeFrameUpdate, AgentError> {
        self.document
            .flush(
                LayoutViewport::new(self.width as f32, self.height as f32),
                &mut self.shaper,
            )
            .map_err(|error| AgentError(error.to_string()))
    }

    pub fn accessibility_dump(&self) -> Vec<AccessibilityDumpNode> {
        super::accessibility_dump(&self.document)
    }

    pub fn click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        self.dispatch_pointer(PointerPhase::Down, x, y, 0, button_mask(0))?;
        self.dispatch_pointer(PointerPhase::Up, x, y, 0, 0)?;
        self.flush()?;
        Ok(true)
    }

    /// Secondary-button click (button 2), the way a right-click reaches the
    /// tree. Context menus open on the press, so both phases are sent with the
    /// button held then released — a primary [`Self::click_xy`] never routes
    /// there, and hand-rolling the pointer events is the same boilerplate in
    /// every consumer that wants to verify a context menu headlessly.
    pub fn secondary_click_xy(&mut self, x: f32, y: f32) -> Result<bool, AgentError> {
        self.dispatch_pointer(PointerPhase::Down, x, y, 2, button_mask(2))?;
        self.dispatch_pointer(PointerPhase::Up, x, y, 2, 0)?;
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
        for character in text.chars() {
            let key = character.to_string();
            self.input.advance(INPUT_FRAME);
            self.input
                .press(
                    self.document.context_mut(),
                    KeyInput {
                        physical: PhysicalKey(Cow::Borrowed("Unidentified")),
                        logical: LogicalKey(Cow::Owned(key.clone())),
                        state: KeyState::Pressed,
                        repeat: false,
                        modifiers: InputModifiers::default(),
                    },
                    Some(&key),
                    Some(&mut self.shaper),
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
        self.dispatch_input(InputPayload::Wheel(WheelInput {
            pointer_id: nana_ui_platform::PointerId(1),
            x,
            y,
            delta_x,
            delta_y,
            unit: WheelUnit::Pixels,
            modifiers: InputModifiers::default(),
        }))?;
        self.flush()?;
        Ok(())
    }

    /// Move the pointer without pressing, so hover-only presentation (tooltips,
    /// hover cards, row affordances) can be captured.
    pub fn hover_xy(&mut self, x: f32, y: f32) -> Result<(), AgentError> {
        self.dispatch_pointer(PointerPhase::Move, x, y, 0, 0)?;
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
        for state in [KeyState::Pressed, KeyState::Released] {
            self.dispatch_input(InputPayload::Key(KeyInput {
                physical: PhysicalKey(Cow::Owned(code.to_owned())),
                logical: LogicalKey(Cow::Owned(key.to_owned())),
                state,
                repeat: false,
                modifiers,
            }))?;
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
        let clear = self.clear_color();
        let scene = self.document.scene().clone();
        let textures = self.host_textures.clone();
        let gpu = self.gpu_mut()?;
        let renderers = gpu.default_gpu_renderers();
        let pixels = gpu
            .paint_layers_scaled(
                &[(&scene, true)],
                size,
                scale,
                clear,
                Some(&textures),
                Some(&renderers),
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Ok((size, pixels))
    }

    /// Returns the frame's own verdict, so a caller never has to decide by eye
    /// whether anything painted.
    pub fn screenshot_png(&mut self, path: impl AsRef<Path>) -> Result<PixelStats, AgentError> {
        let clear = self.clear_color();
        let (size, pixels) = self.screenshot_rgba()?;
        offscreen::write_painter_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(super::pixels::pixel_stats(size, &pixels, clear))
    }

    fn gpu_mut(&mut self) -> Result<&mut OffscreenSnapshots, AgentError> {
        if self.gpu.is_none() {
            self.gpu =
                Some(OffscreenSnapshots::new().map_err(|error| AgentError(error.to_string()))?);
        }
        Ok(self.gpu.as_mut().expect("gpu initialized"))
    }
}

impl AgentSession for RuntimeAgentSession {
    fn describe(&self) -> SessionInfo {
        SessionInfo {
            kind: "runtime".into(),
            width: self.width,
            height: self.height,
            scale: self.scale_factor,
            clear: self.clear_color(),
        }
    }

    fn flush(&mut self) -> Result<(), AgentError> {
        Self::flush(self).map(drop)
    }

    fn accessibility_nodes(&self) -> Vec<AccessibilityDumpNode> {
        self.accessibility_dump()
    }

    fn set_viewport(&mut self, width: u32, height: u32, scale: f32) -> Result<(), AgentError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(AgentError("scale must be finite and positive".into()));
        }
        if width == 0 || height == 0 {
            return Err(AgentError("viewport must be non-zero".into()));
        }
        self.width = width;
        self.height = height;
        self.scale_factor = scale;
        Self::flush(self).map(drop)
    }

    fn set_theme(&mut self, mode: ThemeName) -> Result<(), AgentError> {
        let mode = match mode {
            ThemeName::Light => ThemeAppearance::Light,
            ThemeName::Dark => ThemeAppearance::Dark,
        };
        self.document
            .context_mut()
            .set_preset_theme(mode)
            .map_err(|error| AgentError(error.to_string()))?;
        Self::flush(self).map(drop)
    }

    fn set_clear(&mut self, clear: Option<[f32; 4]>) {
        self.clear = clear;
    }

    fn pointer(&mut self, gesture: PointerGesture) -> Result<bool, AgentError> {
        match gesture {
            PointerGesture::Click { x, y, button: 0 } => self.click_xy(x, y),
            PointerGesture::Click { x, y, .. } => self.secondary_click_xy(x, y),
            PointerGesture::Hover { x, y } => self.hover_xy(x, y).map(|()| true),
            PointerGesture::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => self.scroll_by(x, y, delta_x, delta_y).map(|()| true),
        }
    }

    fn activate(&mut self, node: u64) -> Result<bool, AgentError> {
        self.click_node(node)
    }

    fn keyboard(&mut self, stroke: KeyStroke) -> Result<(), AgentError> {
        self.key_press(&stroke.key, &stroke.code, modifiers(&stroke))
    }

    fn type_text(&mut self, text: &str) -> Result<(), AgentError> {
        Self::type_text(self, text)
    }

    fn inspect(&self, node: u64) -> Result<super::protocol::InspectDump, AgentError> {
        use super::protocol::{
            AppliedDump, CauseDump, DynamicDump, FieldDump, InspectDump, LayoutCauseDump,
            LayoutNodeDump, SegmentDump,
        };
        let target =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        let context = self.document.context();
        let inspection = context
            .inspect(target)
            .ok_or_else(|| AgentError(format!("node {node} does not exist")))?;
        #[cfg(feature = "reactive-trace")]
        let causes = context
            .why_updated(target)
            .map(|why| {
                why.causes
                    .iter()
                    .map(|cause| CauseDump {
                        written_at: cause.written_at.to_string(),
                        signal_created: cause.signal_created.map(|at| at.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        #[cfg(not(feature = "reactive-trace"))]
        let causes: Vec<CauseDump> = Vec::new();
        Ok(InspectDump {
            control: inspection.control.map(str::to_owned),
            element: inspection
                .source_element
                .map(|at| at.to_string())
                .or_else(|| inspection.element.map(|at| at.to_string())),
            fields: inspection
                .fields
                .into_iter()
                .map(|field| FieldDump {
                    name: field.name.to_owned(),
                    value: field.value,
                    bound_at: field
                        .source_bound_at
                        .map(|at| at.to_string())
                        .or_else(|| field.bound_at.map(|at| at.to_string())),
                })
                .collect(),
            causes,
            layout: inspection.layout.map(|cause| {
                let invalidation = cause.invalidation;
                LayoutCauseDump {
                    pending: cause.pending,
                    seed: cause.seed,
                    source: format!("{:?}", invalidation.source).to_ascii_lowercase(),
                    reasons: invalidation.reason.names().map(str::to_owned).collect(),
                    stages: invalidation.kind.names().map(str::to_owned).collect(),
                    changed: invalidation
                        .changed_inputs
                        .names()
                        .map(str::to_owned)
                        .collect(),
                    dependencies: invalidation
                        .affected_axes
                        .names()
                        .map(str::to_owned)
                        .collect(),
                }
            }),
            dynamic: inspection.dynamic.map(|dynamic| DynamicDump {
                generation: dynamic.generation,
                segments: dynamic
                    .segments
                    .iter()
                    .map(|segment| SegmentDump {
                        capacity: segment.capacity.0,
                        cost: segment.marginal_cost.finite(),
                        kind: format!("{:?}", segment.kind).to_ascii_lowercase(),
                        class: format!("{:?}", segment.execution_class).to_ascii_lowercase(),
                        sources: segment.sources,
                    })
                    .collect(),
                applied: dynamic.applied.map(|applied| AppliedDump {
                    axis: if applied.inline { "inline" } else { "block" }.to_owned(),
                    amount: applied.amount.0,
                    padding: applied.padding.0,
                }),
            }),
            layout_node: inspection.layout_node.map(|layout| {
                let view = layout.view;
                let lower = |value: &dyn std::fmt::Debug| format!("{value:?}").to_ascii_lowercase();
                LayoutNodeDump {
                    node: view.id.get(),
                    intent: format!(
                        "display={} direction={} position={}",
                        view.intent.display.map_or("auto".into(), |d| lower(&d)),
                        view.intent.direction.map_or("auto".into(), |d| lower(&d)),
                        lower(&view.intent.position),
                    ),
                    content: lower(&view.content),
                    established: view.established.map(|kind| lower(&kind)),
                    parent_context: view.parent_context.map(|kind| lower(&kind)),
                    participation: view.participation.map(|kind| lower(&kind)),
                    metrics_generation: layout.metrics_generation,
                    context_generation: view.context_generation,
                    result_generation: view.result_generation,
                }
            }),
        })
    }

    fn set_field(&mut self, node: u64, field: &str, value: &str) -> Result<(), AgentError> {
        let target =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        self.document
            .context_mut()
            .set_field(target, field, value)
            .map_err(AgentError)?;
        Self::flush(self).map(drop)
    }

    fn set_value(&mut self, node: u64, value: &str) -> Result<bool, AgentError> {
        let target =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        let document_id = self.document.document();
        let handled = self
            .document
            .context_mut()
            .apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetValue(value.to_owned()),
                },
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Self::flush(self)?;
        Ok(handled)
    }

    fn hit_test(&self, x: f32, y: f32) -> Vec<HitDump> {
        let candidates =
            self.document
                .context()
                .world()
                .hit_test_candidates(self.document.document(), x, y);
        scene_probe::hits(&self.accessibility_dump(), &candidates)
    }

    fn scene_probe(&self, node: u64) -> Option<SceneProbeDump> {
        let id = StableNodeId::new(node)?;
        scene_probe::probe(
            id,
            self.document.scene(),
            self.document.context().world().layout_box(id),
            self.width as f32,
            self.height as f32,
            |x, y| self.hit_test(x, y),
        )
    }

    fn screenshot_rgba(&mut self) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        Self::screenshot_rgba(self)
    }

    fn resolve_agent_id(&self, agent_id: &str) -> Option<u64> {
        self.accessibility_dump()
            .into_iter()
            .find(|node| node.agent_id.as_deref() == Some(agent_id))
            .map(|node| node.id)
    }
}

fn modifiers(stroke: &KeyStroke) -> InputModifiers {
    InputModifiers {
        alt: stroke.alt,
        control: stroke.ctrl,
        meta: stroke.meta,
        shift: stroke.shift,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::runtime::view::{entity_ref, widget, with_refs};
    use nana_ui::runtime::{Button, DocumentId, List, Text};

    #[test]
    fn runtime_session_click_node_and_optional_preview() {
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let (_, button) = document
            .context_mut()
            .mount_view_root(document_id, || {
                let button = entity_ref::<Button>();
                let root = widget(List::new().label("Agent")).key("root").children((
                    widget(Text::new("idle")).key("label"),
                    widget(Button::new("Go")).key("go").entity_ref(button),
                ));
                with_refs(root, button)
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
        if offscreen::optional().is_some() {
            let (size, pixels) = session.screenshot_rgba().expect("preview");
            assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
            assert!(pixels.iter().any(|channel| *channel != 0));
        }
    }

    #[test]
    fn headless_pointer_fixture_routes_canonically_and_idle_stays_idle() {
        let document_id = DocumentId::new(1).expect("document");
        let document = RuntimeDocument::new(document_id);
        let mut session = RuntimeAgentSession::new(document, 240, 160).expect("session");
        assert!(session.flush().expect("idle flush").is_idle());
        assert_eq!(session.input_counters().events_routed, 0);

        for index in 0..1_000 {
            session
                .pointer_event(
                    PointerPhase::Move,
                    index as f32 % 240.0,
                    (index / 240) as f32,
                    0,
                    0,
                )
                .expect("canonical pointer route");
        }
        assert_eq!(session.input_counters().events_routed, 1_000);
        assert!(session.flush().expect("pointer fixture flush").is_idle());
    }

    #[test]
    fn static_document_keeps_high_frequency_empty_hover_idle() {
        let document_id = DocumentId::new(2).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .mount_view_root(document_id, || widget(Text::new("static")).key("label"))
            .expect("root");
        let mut session = RuntimeAgentSession::new(document, 240, 160).expect("session");
        let _ = session.flush().expect("initial mount");
        for index in 0..240 {
            session
                .pointer_event(PointerPhase::Move, 239.0, 159.0 - (index % 2) as f32, 0, 0)
                .expect("canonical pointer route");
        }
        let update = session.flush().expect("steady pointer frame");
        assert!(
            update.is_idle(),
            "static empty hover must not schedule layout or scene work"
        );
    }

    /// A subtree taken out of paint and input settles in one frame.
    #[test]
    fn hidden_subtree_settles_to_idle_flushes() {
        use nana_ui::runtime::{PointerEventsSpec, Stack, VisibilitySpec};

        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let stack = document
            .context_mut()
            .mount_view_root(document_id, || {
                widget(Stack::column(8.0)).key("chrome").children((
                    widget(Text::new("Output")).key("label"),
                    widget(Button::new("Go")).key("go"),
                ))
            })
            .ok()
            .and_then(|view| view.root::<Stack>())
            .expect("root");
        let mut session = RuntimeAgentSession::new(document, 240, 160).expect("session");
        session
            .document_mut()
            .context_mut()
            .update_component(stack, |view, _| {
                *view = view.clone().with_layout(|layout| {
                    layout.pointer_events = Some(PointerEventsSpec::None);
                    layout.paint.visibility = Some(VisibilitySpec::Hidden);
                });
            })
            .expect("hide");
        assert!(!session.flush().expect("settle").is_idle());
        for _ in 0..3 {
            assert!(session.flush().expect("steady").is_idle());
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

        let Some(mut gpu) = offscreen::optional() else {
            return;
        };
        let renderers = default_scene_gpu_renderers();
        let document_id = DocumentId::new(1).expect("document");
        let mut document = RuntimeDocument::new(document_id);
        let view = document
            .context_mut()
            .mount_view_root(document_id, || {
                widget(GpuView::new(1).palette(GpuViewPalette {
                    background: [1.0, 0.0, 0.0, 1.0],
                    accent: [1.0, 0.0, 0.0, 1.0],
                }))
                .key("view")
            })
            .ok()
            .and_then(|view| view.root::<GpuView>())
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

        let Some(mut gpu) = offscreen::optional() else {
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
            .mount_view_root(document_id, || {
                let rows = (0..8)
                    .map(|index| {
                        let mut row = NodeStyle::default();
                        {
                            let layout = std::sync::Arc::make_mut(&mut row.layout);
                            layout.width = Some(LengthSpec::Fill);
                            layout.height = Some(LengthSpec::Px(40.0));
                        }
                        widget(Text::new(format!("Row {index}")).style(row))
                            .key(format!("row-{index}"))
                    })
                    .collect::<Vec<_>>();
                widget(
                    ScrollView::new(ScrollAxes::Vertical)
                        .scrollbars(ScrollbarVisibility::Always)
                        .style(viewport),
                )
                .key("scroll")
                .children(rows)
            })
            .ok()
            .and_then(|view| view.root::<ScrollView>())
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

        if offscreen::optional().is_none() {
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
        let (_, (radios, first, second, divider)) = document
            .context_mut()
            .mount_view_root(document_id, || {
                let refs = (
                    entity_ref::<SegmentedControl>(),
                    entity_ref::<SegmentedOption>(),
                    entity_ref::<SegmentedOption>(),
                    entity_ref::<Divider>(),
                );
                let root = widget(Card::new().style(column)).key("root").children((
                    widget(SegmentedControl::radio_group())
                        .key("radios")
                        .entity_ref(refs.0)
                        .children((
                            widget(SegmentedOption::new("Automatic"))
                                .key("auto")
                                .entity_ref(refs.1),
                            widget(SegmentedOption::new("Manual"))
                                .key("manual")
                                .entity_ref(refs.2),
                        )),
                    widget(Divider::horizontal())
                        .key("divider")
                        .entity_ref(refs.3),
                ));
                with_refs(root, refs)
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
