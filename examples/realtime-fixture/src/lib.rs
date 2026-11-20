//! Realtime application fixture (Issue #177).
//!
//! A game-shaped consumer of NanaUI's common mechanics, and nothing more:
//!
//! - **State and update are the application's.** [`sim::World`] integrates
//!   its bodies in [`sim::World::fixed_update`] at 60 Hz on an application
//!   [`sim::FixedClock`]; NanaUI only wakes the program at the deadline it
//!   reports through [`RuntimeProgram::next_wakeup`].
//! - **Presentation cadence is separate.** While running and visible the
//!   window asks for [`FrameDemand::Continuous`] at the display-bound rate;
//!   paused, hidden or minimised it asks for nothing.
//! - **Input mapping is the application's.** `MoveX`/`MoveY` come from raw
//!   physical keys in [`RuntimeProgram::input_event`], not from focus, a
//!   control or a framework action map.
//! - **The UI tree holds one anchor.** The playfield is a single `GpuView`;
//!   bodies never become nodes. [`render::PlayfieldRenderer`] draws them as
//!   one instanced draw from a published copy of the state, so a fixed step
//!   mutates nothing in the `UiWorld`.
//! - **The overlay is ordinary NanaUI**, event-driven: it changes only when
//!   the application pauses or resumes.
//!
//! [`Fixture::stats`] and [`render::RenderStats`] are the evidence counters;
//! `cargo run -p realtime-fixture -- --evidence` prints them for a scripted
//! run on a real window.

#![forbid(unsafe_code)]

pub mod render;
pub mod sim;

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{
    Activate, Button, DocumentId, Entity, FrameworkError, GpuTextureView, GpuView, LengthSpec,
    NodeStyle, RuntimeDocument, Stack, Text,
};
use nana_ui::{
    FrameDemand, HostTexture, HostTextureAlphaMode, HostTextureRegistry, RoutedInput,
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, SceneGpuRendererRegistry,
    WindowDescriptor, run_runtime,
};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{InputPayload, KeyState, WindowEvent, WindowId};

use render::{PLAYFIELD_SLOT, Playfield, PlayfieldRenderer, SPRITE_EXTENT, SpriteAsset};
use sim::{FIXED_HZ, FixedClock, InputAxes, Instance, World};

/// Host-texture slot of the sprite the overlay shows.
pub const SPRITE_SLOT: &str = "sprite";

/// How a run is configured. Read once by [`RuntimeProgram::initialize`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Drifting bodies, besides the player.
    pub bodies: usize,
    /// Presentation rate asked for while running. The display's refresh
    /// bounds it, so a high value means "display-driven".
    pub present_hz: u32,
    /// Run the scripted evidence sequence and exit.
    pub evidence: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bodies: 64,
            present_hz: 120,
            evidence: false,
        }
    }
}

static OPTIONS: OnceLock<Options> = OnceLock::new();
static REPORT: Mutex<Option<String>> = Mutex::new(None);

/// Application-side evidence counters. Everything here is counted by the
/// fixture itself; renderer counters live in [`render::RenderStats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppStats {
    /// Fixed steps run ([`World::fixed_update`] calls).
    pub app_fixed_updates: u64,
    /// Due fixed steps given up beyond the catch-up bound.
    pub dropped_fixed_steps: u64,
    /// [`RuntimeProgram::wake`] calls.
    pub runtime_wakeups: u64,
    /// [`RuntimeProgram::prepare_window_frame`] calls: every frame tick,
    /// presented or hidden.
    pub prepare_ticks: u64,
    /// [`RuntimeProgram::window_frame_presented`] calls.
    pub frames_presented: u64,
    /// Presented frames whose `UiWorld` generation moved since the previous
    /// presented frame: the frames that carried any UI mutation at all.
    pub ui_world_frames: u64,
    /// States published to the renderer.
    pub published_revisions: u64,
    /// Key events the application's own mapping consumed.
    pub mapped_key_events: u64,
    /// UiWorld edits the application made (overlay text on pause/resume).
    pub ui_mutations: u64,
    /// CPU loads of the sprite resource.
    pub resource_loads: u64,
    /// Wakeups while the window was hidden or minimised.
    pub hidden_wakeups: u64,
    /// Frame ticks while the window was hidden or minimised.
    pub hidden_prepare_ticks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    TogglePause,
}

/// One step of the scripted evidence run.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Minimize,
    Restore,
    Resize(f32, f32),
    Exit,
}

/// The evidence script: offsets from the first wake, and what happens then.
const SCRIPT: [(Duration, Step, &str); 4] = [
    (Duration::from_secs(3), Step::Minimize, "running"),
    (Duration::from_secs(6), Step::Restore, "minimized"),
    (
        Duration::from_secs(8),
        Step::Resize(560.0, 360.0),
        "restored",
    ),
    (Duration::from_secs(10), Step::Exit, "resized"),
];

#[derive(Debug)]
struct Script {
    start: Option<Instant>,
    next: usize,
    mark: (Instant, AppStats, render::RenderStats),
    lines: Vec<String>,
}

pub struct Fixture {
    document: RuntimeDocument,
    status: Entity<Text>,
    pause: Entity<Button>,
    world: World,
    clock: FixedClock,
    input: InputAxes,
    instances: Vec<Instance>,
    playfield: Arc<Playfield>,
    renderers: SceneGpuRendererRegistry,
    textures: HostTextureRegistry,
    options: Options,
    paused: bool,
    occluded: bool,
    zero_size: bool,
    /// The clock restarts from the host's `now` at the next wake: a program
    /// has no clock of its own to trust, and a resumed clock owes nothing.
    restart_clock: bool,
    last_wake: Instant,
    presented_generation: Option<u64>,
    stats: AppStats,
    script: Option<Script>,
}

impl Fixture {
    /// The document and application state, without a GPU. The sprite slot and
    /// renderer bind in [`RuntimeProgram::initialize`].
    pub fn mount(options: Options) -> Result<Self, FrameworkError> {
        let document_id = DocumentId::new(1).expect("fixture document id");
        let mut document = RuntimeDocument::new(document_id);
        let (_, (status, pause)) = document.context_mut().mount_view_root(document_id, || {
            let status = entity_ref::<Text>();
            let pause = entity_ref::<Button>();
            let header = widget(Stack::row(8.0)).children((
                widget(GpuTextureView::new(SPRITE_SLOT).style(fixed_square(20.0))),
                widget(Text::new("Realtime fixture")),
                widget(Text::new(status_text(false, options.bodies))).entity_ref(status),
                widget(Button::new(pause_label(false)))
                    .entity_ref(pause)
                    .on_cx(|_button, _event: &Activate, cx| {
                        cx.dispatch_program(Message::TogglePause);
                    }),
            ));
            // The playfield and the overlay share one area; the overlay is
            // later in document order, so it paints on top.
            let root = widget(
                Stack::layers()
                    .width(LengthSpec::Fill)
                    .height(LengthSpec::Fill),
            )
            .children((
                widget(GpuView::new(PLAYFIELD_SLOT)),
                widget(Stack::column(6.0).padding(12.0)).children((
                    header,
                    widget(Text::new("WASD or arrow keys move the player, P pauses")),
                )),
            ));
            with_refs(root, (status, pause))
        })?;
        let sprite = SpriteAsset::load();
        let playfield = Playfield::new(sprite);
        let mut renderers = SceneGpuRendererRegistry::new();
        renderers.insert(
            nana_ui::runtime::GPU_VIEW_RENDERER,
            Arc::new(PlayfieldRenderer::new(Arc::clone(&playfield))),
        );
        let mut fixture = Self {
            document,
            status,
            pause,
            world: World::new(options.bodies),
            clock: FixedClock::new(FIXED_HZ),
            input: InputAxes::default(),
            instances: Vec::new(),
            playfield,
            renderers,
            textures: HostTextureRegistry::new(),
            options,
            paused: false,
            occluded: false,
            zero_size: false,
            restart_clock: true,
            last_wake: Instant::now(),
            presented_generation: None,
            stats: AppStats {
                resource_loads: 1,
                ..AppStats::default()
            },
            script: None,
        };
        fixture.publish();
        Ok(fixture)
    }

    pub fn stats(&self) -> AppStats {
        self.stats
    }

    pub fn render_stats(&self) -> render::RenderStats {
        self.playfield.stats()
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn input(&self) -> InputAxes {
        self.input
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// The renderers this program installs, for painting its scene offscreen.
    pub fn renderers(&self) -> &SceneGpuRendererRegistry {
        &self.renderers
    }

    pub fn textures(&self) -> &HostTextureRegistry {
        &self.textures
    }

    pub fn playfield_revision(&self) -> u64 {
        self.playfield.revision()
    }

    fn hidden(&self) -> bool {
        self.occluded || self.zero_size
    }

    fn simulating(&self) -> bool {
        !self.paused && !self.hidden()
    }

    fn publish(&mut self) {
        self.world.write_instances(&mut self.instances);
        self.playfield.publish(&self.instances);
        self.stats.published_revisions += 1;
    }

    /// Stop or restart the fixed clock after pause or visibility changed.
    fn sync_clock(&mut self) {
        if self.simulating() {
            if !self.clock.running() {
                self.restart_clock = true;
            }
        } else {
            self.clock.stop();
            self.restart_clock = false;
            // Key-up events will go to whatever has focus now.
            self.input.clear();
        }
    }

    fn toggle_pause(&mut self) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.paused = !self.paused;
        self.sync_clock();
        let paused = self.paused;
        let bodies = self.options.bodies;
        let context = self.document.context_mut();
        context.update_component(self.status, |text, _| {
            text.value = status_text(paused, bodies);
        })?;
        context.update_component(self.pause, |button, _| {
            button.label = pause_label(paused).to_owned();
        })?;
        self.stats.ui_mutations += 2;
        Ok(RuntimeProgramUpdate::redraw(WindowId::PRIMARY))
    }

    fn bind_gpu(&mut self, gpu: &nana_ui::GpuContext) {
        match self.playfield.sprite_texture(gpu) {
            Ok(texture) => {
                self.textures.register(
                    SPRITE_SLOT,
                    HostTexture::new(&texture),
                    SPRITE_EXTENT,
                    SPRITE_EXTENT,
                    HostTextureAlphaMode::Premultiplied,
                );
            }
            Err(error) => panic!("sprite texture: {error}"),
        }
    }

    /// Run the fixed steps due at `now`. Publishes once per wake that stepped.
    fn step(&mut self, now: Instant) {
        if self.restart_clock {
            self.restart_clock = false;
            self.clock.start(now);
        }
        let advance = self.clock.advance(now);
        self.stats.dropped_fixed_steps += advance.dropped;
        if advance.steps == 0 {
            return;
        }
        let dt = self.clock.step().as_secs_f32();
        for _ in 0..advance.steps {
            self.world.fixed_update(dt, &self.input);
        }
        self.stats.app_fixed_updates += u64::from(advance.steps);
        self.publish();
    }

    fn script_deadline(&self) -> Option<Instant> {
        let script = self.script.as_ref()?;
        let Some(start) = script.start else {
            return Some(self.last_wake);
        };
        SCRIPT
            .get(script.next)
            .map(|(offset, _, _)| start + *offset)
    }

    fn run_script(&mut self, now: Instant) -> RuntimeProgramUpdate {
        let stats = self.stats;
        let render = self.playfield.stats();
        let Some(script) = self.script.as_mut() else {
            return RuntimeProgramUpdate::default();
        };
        let start = *script.start.get_or_insert_with(|| {
            script.mark = (now, stats, render);
            now
        });
        let mut update = RuntimeProgramUpdate::default();
        while let Some((offset, step, phase)) = SCRIPT.get(script.next).copied() {
            if now < start + offset {
                break;
            }
            script.next += 1;
            script
                .lines
                .push(phase_line(phase, &script.mark, now, stats, render));
            script.mark = (now, stats, render);
            let command = match step {
                Step::Minimize => WindowCommand::SetMinimized {
                    id: WindowId::PRIMARY,
                    minimized: true,
                },
                Step::Restore => WindowCommand::SetMinimized {
                    id: WindowId::PRIMARY,
                    minimized: false,
                },
                Step::Resize(width, height) => WindowCommand::SetBounds {
                    id: WindowId::PRIMARY,
                    position: (120.0, 120.0),
                    size: (width, height),
                },
                Step::Exit => {
                    let mut report = String::from(REPORT_HEADER);
                    for line in &script.lines {
                        report.push_str(line);
                        report.push('\n');
                    }
                    report.push_str(&totals_line(stats, render));
                    *REPORT.lock().expect("report") = Some(report);
                    return RuntimeProgramUpdate::exit();
                }
            };
            update.window_commands.push(command);
        }
        update
    }
}

const REPORT_HEADER: &str = "phase      seconds  fixed/s  wakes/s  ticks/s  frames/s  uploads/s  draws/s  prepares  ui_frames  hidden_wakes  hidden_ticks\n";

fn phase_line(
    phase: &str,
    mark: &(Instant, AppStats, render::RenderStats),
    now: Instant,
    stats: AppStats,
    render: render::RenderStats,
) -> String {
    let (since, before, render_before) = *mark;
    let seconds = now.duration_since(since).as_secs_f64().max(1e-6);
    let rate = |value: u64| value as f64 / seconds;
    format!(
        "{phase:<10} {seconds:>7.2}  {:>7.1}  {:>7.1}  {:>7.1}  {:>8.1}  {:>9.1}  {:>7.1}  {:>8}  {:>9}  {:>12}  {:>12}",
        rate(stats.app_fixed_updates - before.app_fixed_updates),
        rate(stats.runtime_wakeups - before.runtime_wakeups),
        rate(stats.prepare_ticks - before.prepare_ticks),
        rate(stats.frames_presented - before.frames_presented),
        rate(render.instance_uploads - render_before.instance_uploads),
        rate(render.draws - render_before.draws),
        render.prepares - render_before.prepares,
        stats.ui_world_frames - before.ui_world_frames,
        stats.hidden_wakeups - before.hidden_wakeups,
        stats.hidden_prepare_ticks - before.hidden_prepare_ticks,
    )
}

fn totals_line(stats: AppStats, render: render::RenderStats) -> String {
    format!("totals: {stats:?}\n        {render:?}\n",)
}

fn status_text(paused: bool, bodies: usize) -> String {
    if paused {
        "Paused".to_owned()
    } else {
        format!("Running, {bodies} bodies")
    }
}

fn pause_label(paused: bool) -> &'static str {
    if paused { "Resume" } else { "Pause" }
}

fn fixed_square(side: f32) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Px(side));
    layout.height = Some(LengthSpec::Px(side));
    style
}

impl RuntimeProgram for Fixture {
    type Message = Message;
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let options = OPTIONS.get().copied().unwrap_or_default();
        let mut fixture = Self::mount(options)?;
        fixture.bind_gpu(context.gpu());
        if options.evidence {
            fixture.script = Some(Script {
                start: None,
                next: 0,
                mark: (Instant::now(), AppStats::default(), Default::default()),
                lines: Vec::new(),
            });
        }
        Ok((fixture, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::TogglePause => self
                .toggle_pause()
                .unwrap_or_else(|error| panic!("pause toggle failed: {error}")),
        }
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn scene_gpu_renderers(&self, _id: WindowId) -> Option<SceneGpuRendererRegistry> {
        Some(self.renderers.clone())
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.bind_gpu(context.gpu());
    }

    fn frame_demand(&self, _id: WindowId) -> FrameDemand {
        match NonZeroU32::new(self.options.present_hz) {
            Some(rate) if self.simulating() => FrameDemand::Continuous(rate),
            _ => FrameDemand::OnDemand,
        }
    }

    fn next_wakeup(&self) -> Option<Instant> {
        let clock = if self.restart_clock {
            Some(self.last_wake)
        } else {
            self.clock.deadline()
        };
        [clock, self.script_deadline()].into_iter().flatten().min()
    }

    fn wake(
        &mut self,
        now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.last_wake = now;
        self.stats.runtime_wakeups += 1;
        if self.hidden() {
            self.stats.hidden_wakeups += 1;
        }
        if self.simulating() {
            self.step(now);
        }
        // No redraw: the fixed step changed no UI. The continuous frame
        // demand presents whatever was published last.
        self.run_script(now)
    }

    fn prepare_window_frame(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
        self.stats.prepare_ticks += 1;
        if self.hidden() {
            self.stats.hidden_prepare_ticks += 1;
        }
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.stats.frames_presented += 1;
        let generation = self.document.context().world().generation();
        if self.presented_generation.replace(generation) != Some(generation) {
            self.stats.ui_world_frames += 1;
        }
        RuntimeProgramUpdate::default()
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match &input.event.payload {
            InputPayload::Key(key) => {
                let pressed = key.state == KeyState::Pressed;
                let physical = key.physical.0.as_ref();
                if physical == "KeyP" && pressed && !key.repeat {
                    self.stats.mapped_key_events += 1;
                    if !input.disposition.prevent_default {
                        return self.toggle_pause();
                    }
                    return Ok(RuntimeProgramUpdate::default());
                }
                // Held direction keys only count while the simulation runs.
                if self.simulating() && self.input.apply_key(physical, pressed) {
                    self.stats.mapped_key_events += 1;
                    return Ok(RuntimeProgramUpdate::default());
                }
            }
            InputPayload::Focus { focused: false } => self.input.clear(),
            _ => {}
        }
        // Everything else was routed into the overlay already; let it repaint.
        Ok(RuntimeProgramUpdate::redraw(id))
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::CloseRequested { .. } => return RuntimeProgramUpdate::exit(),
            WindowEvent::VisibilityChanged { id, hidden } if id == WindowId::PRIMARY => {
                self.occluded = hidden;
                self.sync_clock();
            }
            WindowEvent::Ready { id, geometry } | WindowEvent::Resized { id, geometry }
                if id == WindowId::PRIMARY =>
            {
                self.zero_size = geometry.physical_size.0 == 0 || geometry.physical_size.1 == 0;
                self.sync_clock();
            }
            WindowEvent::FocusChanged { focused: false, .. } => self.input.clear(),
            _ => {}
        }
        RuntimeProgramUpdate::default()
    }
}

/// Run the fixture in a native window. Returns the evidence report when
/// [`Options::evidence`] is set.
pub fn run(options: Options) -> Result<Option<String>, nana_ui::HostedRunError> {
    let _ = OPTIONS.set(options);
    run_runtime::<Fixture>(
        WindowDescriptor::new("NanaUI realtime fixture")
            .initial_size(800.0, 520.0)
            .minimum_size(360.0, 240.0)
            .system_caption(true),
    )?;
    Ok(REPORT.lock().expect("report").take())
}

#[cfg(test)]
mod tests;
