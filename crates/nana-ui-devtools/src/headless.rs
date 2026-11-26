//! A whole [`RuntimeProgram`] offscreen: [`nana_ui::headless::HeadlessRuntime`]
//! drives its hooks, this paints and reads back its windows.
//!
//! The program keeps its documents; nothing is moved into a session and back.
//! Application-specific commands reach the program through
//! [`HeadlessRuntime::program_mut`] (via `Deref`).

use std::borrow::Cow;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::time::{Duration, Instant};

use nana_gpu::{GpuContext, GpuTextureDescriptor, GpuTextureUsages};
use nana_ui::runtime::{AccessibilityAction, AccessibilityActionRequest, StableNodeId};
use nana_ui::{
    RuntimeProgram, ScenePaintViewport, SceneWgpuPainter, WindowDescriptor,
    commit_image_natural_sizes,
};
use nana_ui_platform::{
    InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, PointerInput,
    PointerPhase, WindowGeometry, WindowId,
};

pub use nana_ui::headless::{
    HeadlessColorSampler, HeadlessFileDialog, HeadlessOptions, HeadlessQuiesce, HeadlessReport,
    HeadlessRuntime, HeadlessTurn, HeadlessWindow, headless_geometry,
};

use crate::agent::pixels::{pixel_diff, pixel_stats};
use crate::agent::runtime::button_mask;
use crate::agent::{
    A11yFilter, AccessibilityDumpNode, AgentError, HitDump, InspectDump, PixelDiff, PixelStats,
    SceneProbeDump, Target, ThemeInspectDump, ThemeInvalidationDump, ThemePackageLoadDump,
    accessibility_dump, document_hit_test, document_inspect, document_load_theme_package,
    document_scene_probe, document_set_field, document_theme_inspect, document_theme_invalidation,
    filter_accessibility_nodes, node_center_in, resolve_target_in,
};
pub use crate::agent::{
    AccessibilityViolation, AccessibilityViolationKind, format_accessibility_violations,
    validate_accessibility_tree,
};
use crate::offscreen::{FORMAT, Size, read_png, readback, write_painter_png};

/// How long a screenshot waits for the images its scene shows to decode.
const IMAGE_DEADLINE: Duration = Duration::from_secs(35);

/// Monotonic clock floor tracker so returning from manual simulated time to
/// system wall time never moves time backward.
#[derive(Debug, Clone)]
struct MonotonicClockBridge {
    manual_requested: bool,
    clock_floor: Option<Instant>,
    last_wall_sample: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockSyncAction {
    /// Leave the underlying runtime clock untouched.
    Keep,
    /// Wall time has caught up to (or passed) the floor; restore system clock.
    RestoreSystem,
    /// Wall time is still behind the floor; keep manual clock at `target`.
    PinAt(Instant),
}

impl MonotonicClockBridge {
    fn new(now: Instant) -> Self {
        Self {
            manual_requested: false,
            clock_floor: None,
            last_wall_sample: now,
        }
    }

    fn record_floor(&mut self, observed: Instant) {
        self.clock_floor = Some(
            self.clock_floor
                .map_or(observed, |floor| floor.max(observed)),
        );
    }

    fn sync(&mut self, wall: Instant) -> ClockSyncAction {
        let dt = wall.saturating_duration_since(self.last_wall_sample);
        self.last_wall_sample = wall;
        if self.manual_requested {
            return ClockSyncAction::Keep;
        }
        let Some(floor) = self.clock_floor else {
            return ClockSyncAction::Keep;
        };
        let next = floor + dt;
        if wall >= next {
            self.clock_floor = None;
            ClockSyncAction::RestoreSystem
        } else {
            self.clock_floor = Some(next);
            ClockSyncAction::PinAt(next)
        }
    }

    fn request_manual(&mut self, wall: Instant, runtime_now: Instant) -> Instant {
        let target = match self.sync(wall) {
            ClockSyncAction::PinAt(pinned) => pinned.max(runtime_now),
            ClockSyncAction::Keep | ClockSyncAction::RestoreSystem => self
                .clock_floor
                .map_or(runtime_now, |floor| floor.max(runtime_now)),
        };
        self.manual_requested = true;
        self.record_floor(target);
        target
    }

    fn request_system(&mut self, wall: Instant, runtime_now: Instant) -> ClockSyncAction {
        self.record_floor(runtime_now);
        self.manual_requested = false;
        self.last_wall_sample = wall;
        self.sync(wall)
    }

    fn effective_now(&self, runtime_now: Instant) -> Instant {
        self.clock_floor
            .map_or(runtime_now, |floor| floor.max(runtime_now))
    }
}

/// [`HeadlessRuntime`] with pixels.
pub struct HeadlessHost<P: RuntimeProgram> {
    runtime: HeadlessRuntime<P>,
    painter: SceneWgpuPainter,
    theme_registry: nana_ui_core::ThemeRegistry,
    clock_bridge: MonotonicClockBridge,
}

impl<P: RuntimeProgram> Deref for HeadlessHost<P> {
    type Target = HeadlessRuntime<P>;
    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl<P: RuntimeProgram> DerefMut for HeadlessHost<P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}

impl<P: RuntimeProgram> HeadlessHost<P> {
    /// Initialize the program on `gpu`, the device its content producers
    /// share, with a primary window of `geometry`.
    pub fn new(
        gpu: GpuContext,
        primary: WindowDescriptor,
        geometry: WindowGeometry,
        options: HeadlessOptions,
    ) -> Result<Self, AgentError> {
        let painter = SceneWgpuPainter::try_new(&gpu, FORMAT)
            .map_err(|error| AgentError(error.to_string()))?;
        let runtime = HeadlessRuntime::new(gpu, primary, geometry, options).map_err(AgentError)?;
        let initial_now = runtime.now_instant();
        Ok(Self {
            runtime,
            painter,
            theme_registry: nana_ui_core::ThemeRegistry::default(),
            clock_bridge: MonotonicClockBridge::new(initial_now),
        })
    }

    pub fn runtime(&mut self) -> &mut HeadlessRuntime<P> {
        &mut self.runtime
    }

    fn apply_clock_sync(&mut self, action: ClockSyncAction) {
        match action {
            ClockSyncAction::Keep => {}
            ClockSyncAction::RestoreSystem => self.runtime.use_system_clock(),
            ClockSyncAction::PinAt(target) => {
                let current = self.runtime.use_manual_clock();
                let delta = target.saturating_duration_since(current);
                let advanced = self.runtime.advance_clock(delta);
                self.clock_bridge.record_floor(advanced);
            }
        }
    }

    fn sync_clock(&mut self) {
        let action = self.clock_bridge.sync(Instant::now());
        self.apply_clock_sync(action);
        self.clock_bridge.record_floor(self.runtime.now_instant());
    }

    /// Pin this host's clock to simulated time so subsequent turns no longer
    /// advance with wall time unless [`Self::advance_clock`] is called.
    pub fn use_manual_clock(&mut self) -> Instant {
        let target = self
            .clock_bridge
            .request_manual(Instant::now(), self.runtime.now_instant());
        let current = self.runtime.use_manual_clock();
        let delta = target.saturating_duration_since(current);
        let pinned = self.runtime.advance_clock(delta);
        self.clock_bridge.record_floor(pinned);
        pinned
    }

    /// Return this host to the system wall clock without ever moving time
    /// backward.
    ///
    /// If simulated time was previously advanced ahead of `Instant::now()`,
    /// the host keeps advancing smoothly by wall-clock deltas from that floor
    /// until system wall time catches up.
    pub fn use_system_clock(&mut self) {
        let action = self
            .clock_bridge
            .request_system(Instant::now(), self.runtime.now_instant());
        self.apply_clock_sync(action);
    }

    /// Whether the host is currently driven by a caller-requested manual clock.
    pub fn is_manual_clock(&self) -> bool {
        self.clock_bridge.manual_requested
    }

    /// Current monotonic timestamp seen by host turns, input, and presentation.
    pub fn now_instant(&self) -> Instant {
        self.clock_bridge.effective_now(self.runtime.now_instant())
    }

    /// Advance the simulated clock by `delta`, switching to manual clock mode
    /// if needed.
    pub fn advance_clock(&mut self, delta: Duration) -> Instant {
        let _ = self.use_manual_clock();
        let next = self.runtime.advance_clock(delta);
        self.clock_bridge.record_floor(next);
        next
    }

    /// Execute one host turn while keeping the clock monotonic.
    pub fn turn(&mut self) -> HeadlessTurn {
        self.sync_clock();
        let turn = self.runtime.turn();
        self.clock_bridge.record_floor(self.runtime.now_instant());
        turn
    }

    /// Advance the simulated clock by `delta` and execute one host turn.
    pub fn step(&mut self, delta: Duration) -> HeadlessTurn {
        self.advance_clock(delta);
        self.turn()
    }

    /// Run turns until queued messages, layout flushes, and active animations
    /// settle, preserving the caller's system-clock mode and monotonic floor
    /// even when `step` temporarily advances simulated time ahead of wall time.
    pub fn turn_until_quiescent(&mut self, step: Duration, max_steps: usize) -> HeadlessQuiesce {
        self.sync_clock();
        let was_manual = self.clock_bridge.manual_requested;
        let report = self.runtime.turn_until_quiescent(step, max_steps);
        self.clock_bridge.record_floor(self.runtime.now_instant());
        if !was_manual {
            self.use_system_clock();
        }
        report
    }

    /// `id`'s accessibility tree as an agent sees it.
    pub fn accessibility(&self, id: WindowId) -> Vec<AccessibilityDumpNode> {
        self.runtime
            .with_document(id, accessibility_dump)
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Fetch `id`'s accessibility tree and validate it against the generic
    /// NanaUI accessibility contract.
    pub fn validate_accessibility(
        &self,
        id: WindowId,
    ) -> Result<Vec<AccessibilityDumpNode>, AgentError> {
        let nodes = self.accessibility(id);
        validate_accessibility_tree(&nodes)
            .map_err(|violations| AgentError(format_accessibility_violations(&violations)))?;
        Ok(nodes)
    }

    /// `id`'s accessibility nodes filtered by `filter`. The boolean marks
    /// whether nodes were dropped by the filter.
    pub fn filtered_accessibility(
        &self,
        id: WindowId,
        filter: &A11yFilter,
    ) -> (Vec<AccessibilityDumpNode>, bool) {
        filter_accessibility_nodes(self.accessibility(id), filter)
    }

    /// Resolve `target` against `id`'s current accessibility tree.
    pub fn resolve_target(&self, id: WindowId, target: &Target) -> Result<u64, AgentError> {
        let nodes = self.accessibility(id);
        resolve_target_in(&nodes, target)
    }

    /// Logical center point of `target` in `id`'s viewport.
    pub fn target_point(&self, id: WindowId, target: &Target) -> Result<(f32, f32), AgentError> {
        if let Some(point) = target.point() {
            return Ok(point);
        }
        let nodes = self.accessibility(id);
        let node = resolve_target_in(&nodes, target)?;
        node_center_in(&nodes, node)
            .ok_or_else(|| AgentError(format!("node {node} has no painted geometry")))
    }

    /// Hit-test `(x, y)` in `id`'s retained world and accessibility tree.
    pub fn hit_test(&self, id: WindowId, x: f32, y: f32) -> Result<Vec<HitDump>, AgentError> {
        self.runtime
            .with_document(id, |document| document_hit_test(document, x, y))
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))
    }

    /// Probe how `target` painted into `id`'s scene and responded to hit testing.
    pub fn scene_probe(&self, id: WindowId, target: &Target) -> Result<SceneProbeDump, AgentError> {
        let node = self.resolve_target(id, target)?;
        let window = self
            .runtime
            .window(id)
            .ok_or_else(|| AgentError(format!("no window {}", id.0)))?;
        let viewport = window.geometry.content(window.content_zoom).logical_size;
        self.runtime
            .with_document(id, |document| {
                document_scene_probe(document, viewport, node)
            })
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))?
            .ok_or_else(|| AgentError(format!("node {node} is not in the scene")))
    }

    /// Inspect `target`'s retained control, fields, layout invalidation, and layout node in `id`.
    pub fn inspect(&self, id: WindowId, target: &Target) -> Result<InspectDump, AgentError> {
        let node = self.resolve_target(id, target)?;
        self.runtime
            .with_document(id, |document| document_inspect(document, node))
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))?
    }

    /// Inspect `target`'s resolved theme, active scope chain, color overrides,
    /// dependency mask, and package provenance in `id` (#109).
    pub fn theme_inspect(
        &self,
        id: WindowId,
        target: &Target,
    ) -> Result<ThemeInspectDump, AgentError> {
        let node = self.resolve_target(id, target)?;
        self.runtime
            .with_document(id, |document| {
                document_theme_inspect(document, node, Some(&self.theme_registry))
            })
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))?
    }

    /// Most recent theme or scope invalidation summary recorded on `id`'s document (#109).
    pub fn theme_invalidation(
        &self,
        id: WindowId,
    ) -> Result<Option<ThemeInvalidationDump>, AgentError> {
        self.runtime
            .with_document(id, document_theme_invalidation)
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))
    }

    /// Parse, validate, and register a JSON `ThemePackage` fail-closed; apply it
    /// to `id`'s document and re-present when `apply` is true (#109).
    pub fn load_theme_package(
        &mut self,
        id: WindowId,
        json: &str,
        apply: bool,
    ) -> Result<ThemePackageLoadDump, AgentError> {
        let registry = &mut self.theme_registry;
        let result = self
            .runtime
            .with_document_mut(id, |document| {
                document_load_theme_package(document, registry, json, apply)
            })
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))??;
        if apply {
            self.runtime.present(id);
        }
        Ok(result)
    }

    /// Present `id` once more, keeping the host clock monotonic.
    pub fn present(&mut self, id: WindowId) {
        self.sync_clock();
        self.runtime.present(id);
        self.clock_bridge.record_floor(self.runtime.now_instant());
    }

    /// Present `id` once more, then paint it over `clear` and read it back,
    /// waiting for the images it shows to decode. Image sizes the paint
    /// learned go to the document for its next frame, as after a native one.
    pub fn screenshot(
        &mut self,
        id: WindowId,
        clear: [f32; 4],
    ) -> Result<(Size<u32>, Vec<u8>), AgentError> {
        let error = |error: &dyn std::fmt::Display| AgentError(error.to_string());
        self.present(id);
        // Painted as the native host paints it: in the window's content
        // geometry, so a content zoom shows.
        let window = self
            .runtime
            .window(id)
            .ok_or_else(|| AgentError(format!("no window {}", id.0)))?;
        let geometry = window.geometry.content(window.content_zoom);
        let size = Size::new(geometry.physical_size.0, geometry.physical_size.1);
        let gpu = self.runtime.gpu().clone();
        let texture = gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("nana-ui headless screenshot"),
                width: size.width,
                height: size.height,
                format: FORMAT,
                usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
            })
            .map_err(|e| error(&e))?;
        let target = texture.render_target().map_err(|e| error(&e))?;
        let scene = self
            .runtime
            .with_document(id, |document| document.shared_scene())
            .ok()
            .flatten()
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))?;
        let textures = self.runtime.program().host_textures(id);
        let renderers = self.runtime.program().scene_gpu_renderers(id);
        let viewport = ScenePaintViewport {
            logical_size: [geometry.logical_size.0, geometry.logical_size.1],
            physical_size: [size.width, size.height],
            scale_factor: geometry.scale_factor,
            scene_origin: [0.0; 2],
            target_origin: [0.0; 2],
            clear_color: clear,
            clear: true,
        };
        let deadline = Instant::now() + IMAGE_DEADLINE;
        let pixels = loop {
            let revision = self.painter.image_revision();
            let mut frame = gpu.begin_frame("nana-ui headless screenshot");
            self.painter
                .paint(
                    &scene,
                    &mut frame,
                    &target,
                    viewport,
                    textures.as_ref(),
                    renderers.as_ref(),
                )
                .map_err(|e| AgentError(format!("scene paint failed: {e:?}")))?;
            frame.submit();
            let pixels = readback(&gpu, &texture, size).map_err(|e| error(&e))?;
            if revision == self.painter.image_revision() && !self.painter.has_pending_images() {
                break pixels;
            }
            if Instant::now() >= deadline {
                return Err(AgentError("screenshot timed out waiting for images".into()));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let sizes = self.painter.take_image_natural_sizes(None);
        if !sizes.is_empty() {
            let _ = self.runtime.with_document_mut(id, |document| {
                commit_image_natural_sizes(document.context_mut(), &sizes)
            });
        }
        Ok((size, pixels))
    }

    /// Click `node` through its accessibility action.
    pub fn click_node(&mut self, id: WindowId, node: u64) -> Result<(), AgentError> {
        let target =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        self.runtime
            .accessibility_action(
                id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::Click,
                },
            )
            .map_err(|error| AgentError(error.to_string()))
    }

    /// A mouse sample at `(x, y)`: `button` is the one that changed (-1 for
    /// none), `buttons` the ones held after it.
    pub fn mouse(
        &mut self,
        id: WindowId,
        phase: PointerPhase,
        (x, y): (f32, f32),
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    ) -> Result<(), AgentError> {
        let pointer = PointerInput {
            button,
            buttons,
            pressure: if buttons == 0 { 0.0 } else { 0.5 },
            modifiers,
            ..PointerInput::mouse(phase, x, y)
        };
        self.input(id, InputPayload::Pointer(pointer), None)
    }

    /// Move to `(x, y)`, then press and release the primary button there.
    pub fn click(&mut self, id: WindowId, point: (f32, f32)) -> Result<(), AgentError> {
        self.click_button(id, point, 0)
    }

    /// Move to `(x, y)`, then press and release `button` (`0` primary, `1` middle, `2` secondary).
    pub fn click_button(
        &mut self,
        id: WindowId,
        point: (f32, f32),
        button: i16,
    ) -> Result<(), AgentError> {
        let mask = button_mask(button);
        let none = InputModifiers::default();
        self.mouse(id, PointerPhase::Move, point, -1, 0, none)?;
        self.mouse(id, PointerPhase::Down, point, button, mask, none)?;
        self.mouse(id, PointerPhase::Up, point, button, 0, none)
    }

    /// Resolve `target` and move the pointer to its center without pressing.
    pub fn hover_target(&mut self, id: WindowId, target: &Target) -> Result<(), AgentError> {
        let point = self.target_point(id, target)?;
        self.mouse(
            id,
            PointerPhase::Move,
            point,
            -1,
            0,
            InputModifiers::default(),
        )
    }

    /// Resolve `target` to its viewport center and click it via pointer events.
    pub fn click_target(&mut self, id: WindowId, target: &Target) -> Result<(), AgentError> {
        let point = self.target_point(id, target)?;
        self.click(id, point)
    }

    /// Resolve `target` and activate it through its accessibility click action.
    pub fn activate_target(&mut self, id: WindowId, target: &Target) -> Result<u64, AgentError> {
        let node = self.resolve_target(id, target)?;
        self.click_node(id, node)?;
        Ok(node)
    }

    /// Resolve `target` and set its value through accessibility `SetValue`.
    pub fn set_value(
        &mut self,
        id: WindowId,
        target: &Target,
        value: &str,
    ) -> Result<u64, AgentError> {
        let node = self.resolve_target(id, target)?;
        let stable =
            StableNodeId::new(node).ok_or_else(|| AgentError("node id 0 is reserved".into()))?;
        self.runtime
            .accessibility_action(
                id,
                AccessibilityActionRequest {
                    target: stable,
                    action: AccessibilityAction::SetValue(value.to_owned()),
                },
            )
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(node)
    }

    /// Resolve `target`, set inspector `field` to `value`, and re-present `id`.
    pub fn set_field(
        &mut self,
        id: WindowId,
        target: &Target,
        field: &str,
        value: &str,
    ) -> Result<u64, AgentError> {
        let node = self.resolve_target(id, target)?;
        let result = self
            .runtime
            .with_document_mut(id, |document| {
                document_set_field(document, node, field, value)
            })
            .map_err(|error| AgentError(error.to_string()))?
            .ok_or_else(|| AgentError(format!("window {} has no document", id.0)))?;
        result?;
        self.runtime.present(id);
        Ok(node)
    }

    /// Paint `id` over `clear`, save an unpremultiplied RGBA8 PNG at `path`, and return pixel stats.
    pub fn screenshot_png(
        &mut self,
        id: WindowId,
        clear: [f32; 4],
        path: impl AsRef<Path>,
    ) -> Result<PixelStats, AgentError> {
        let (size, pixels) = self.screenshot(id, clear)?;
        write_painter_png(path.as_ref(), size, &pixels)
            .map_err(|error| AgentError(error.to_string()))?;
        Ok(pixel_stats(size, &pixels, clear))
    }

    /// Compare a fresh screenshot of `id` over `clear` against a PNG file at `baseline`.
    pub fn diff_screenshot(
        &mut self,
        id: WindowId,
        clear: [f32; 4],
        baseline: impl AsRef<Path>,
        tolerance: u8,
    ) -> Result<PixelDiff, AgentError> {
        let baseline = baseline.as_ref();
        let (base_size, base) = read_png(baseline)
            .ok_or_else(|| AgentError(format!("cannot read baseline {}", baseline.display())))?;
        let (size, current) = self.screenshot(id, clear)?;
        if size != base_size {
            return Err(AgentError(format!(
                "frame sizes differ: baseline {}x{}, candidate {}x{}",
                base_size.width, base_size.height, size.width, size.height
            )));
        }
        pixel_diff(size, &base, &current, tolerance).map_err(AgentError)
    }

    /// One key transition; a press may carry the `text` it types.
    pub fn key(
        &mut self,
        id: WindowId,
        (logical, physical): (&str, &str),
        state: KeyState,
        repeat: bool,
        modifiers: InputModifiers,
        text: Option<&str>,
    ) -> Result<(), AgentError> {
        self.input(
            id,
            InputPayload::Key(KeyInput {
                physical: PhysicalKey(Cow::Owned(physical.to_owned())),
                logical: LogicalKey(Cow::Owned(logical.to_owned())),
                state,
                repeat,
                modifiers,
            }),
            text,
        )
    }

    /// Type `text` one character per key press.
    pub fn type_text(&mut self, id: WindowId, text: &str) -> Result<(), AgentError> {
        for character in text.chars() {
            let character = character.to_string();
            self.key(
                id,
                (&character, "Unidentified"),
                KeyState::Pressed,
                false,
                InputModifiers::default(),
                Some(&character),
            )?;
        }
        Ok(())
    }

    /// Route `payload` into `id`; see [`HeadlessRuntime::input`].
    pub fn input(
        &mut self,
        id: WindowId,
        payload: InputPayload,
        text: Option<&str>,
    ) -> Result<(), AgentError> {
        self.sync_clock();
        self.runtime
            .input(id, payload, text)
            .map_err(|error| AgentError(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ClockSyncAction, MonotonicClockBridge};

    #[test]
    fn use_system_clock_never_moves_backward_after_manual_advance() {
        let t0 = Instant::now();
        let mut bridge = MonotonicClockBridge::new(t0);

        // Pin manual clock at t0 and advance simulated time 10s ahead of wall time.
        let pinned = bridge.request_manual(t0, t0);
        assert_eq!(pinned, t0);
        let simulated = t0 + Duration::from_secs(10);
        bridge.record_floor(simulated);

        // Switch back to system clock 50ms later in wall time: must stay pinned
        // at simulated (t0 + 10s) instead of jumping back to t0 + 0.05s.
        let wall1 = t0 + Duration::from_millis(50);
        let action1 = bridge.request_system(wall1, simulated);
        assert_eq!(action1, ClockSyncAction::PinAt(simulated));
        assert!(!bridge.manual_requested);
        assert!(bridge.effective_now(wall1) >= simulated);

        // Another 25ms of wall time elapses: advances smoothly without going backward.
        let wall2 = wall1 + Duration::from_millis(25);
        let action2 = bridge.sync(wall2);
        assert_eq!(
            action2,
            ClockSyncAction::PinAt(t0 + Duration::from_millis(10_025))
        );

        // Re-entering manual clock starts at least at the bridged timestamp.
        let wall3 = wall2 + Duration::from_millis(25);
        let re_pinned = bridge.request_manual(wall3, t0 + Duration::from_millis(10_025));
        assert_eq!(re_pinned, t0 + Duration::from_millis(10_050));
        assert!(bridge.manual_requested);
    }

    #[test]
    fn system_clock_restores_once_wall_time_reaches_floor() {
        let t0 = Instant::now();
        let mut bridge = MonotonicClockBridge::new(t0);
        let _ = bridge.request_manual(t0, t0);
        // Small simulated advance (10ms) already overtaken by wall time (50ms).
        let simulated = t0 + Duration::from_millis(10);
        bridge.record_floor(simulated);
        let wall1 = t0 + Duration::from_millis(50);
        assert_eq!(
            bridge.request_system(wall1, simulated),
            ClockSyncAction::RestoreSystem
        );
        assert_eq!(bridge.clock_floor, None);
        assert_eq!(
            bridge.sync(wall1 + Duration::from_millis(5)),
            ClockSyncAction::Keep
        );
    }
}
